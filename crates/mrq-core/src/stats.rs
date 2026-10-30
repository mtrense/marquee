//! Document statistics: [`Counts`] of one document (words, characters, headings per
//! level, links, images, code blocks per language, tables, footnotes, tasks), summed
//! into a total, plus the reading time at a given speed ([`reading_minutes`]). A
//! [`Report`] holds the counts of several documents and their total, and renders as JSON
//! (`mrq stats --json`, via serde) or as an aligned text table ([`Report::table`]).
//!
//! ```
//! use mrq_core::Document;
//! use mrq_core::stats::{Counts, StatsOptions};
//!
//! let doc = Document::parse("# Notes\n\n- [ ] Write *more* words\n\n```rust\nfn main() {}\n```\n");
//! let counts = Counts::of(&doc, &StatsOptions::new());
//! assert_eq!(counts.words, 4);
//! assert_eq!((counts.tasks.open, counts.code_blocks.languages["rust"]), (1, 1));
//! ```
//!
//! # What counts as a word
//!
//! Words and characters are counted over the text a reader sees: heading, paragraph,
//! table and list text including link text, image alt text, inline code, wikilink
//! aliases and admonition/callout titles. Frontmatter, raw HTML, comments (HTML and
//! Obsidian `%%…%%`), directives and link definitions never count. Code blocks and math
//! count only with [`StatsOptions::include_code`]. Words are Unicode word-boundary
//! segments (UAX #29) containing a letter or digit, so `it's` is one word and `—` none;
//! characters are the non-whitespace extended grapheme clusters of the same text.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::iter::Sum;
use std::ops::{Add, AddAssign};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::ast::{Admonition, Callout, Event, Link, NodeKind, TaskState, WikiLink};
use crate::document::Document;

/// What [`Counts::of`] counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatsOptions {
    include_code: bool,
}

impl StatsOptions {
    /// The defaults: code blocks and math are not words.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether code blocks and math count towards words and characters.
    #[must_use]
    pub const fn include_code(mut self, include: bool) -> Self {
        self.include_code = include;
        self
    }
}

/// The counters of one document, or the sum of several (`+`, `+=`, [`Sum`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Counts {
    /// Words of the prose text; see [what counts](self#what-counts-as-a-word).
    pub words: u64,
    /// Non-whitespace grapheme clusters of the same text.
    pub characters: u64,
    /// Headings per level.
    pub headings: HeadingCounts,
    /// Links, wikilinks and non-image embeds.
    pub links: LinkCounts,
    /// Images and image embeds (`![[pic.png]]`).
    pub images: u64,
    /// Fenced and indented code blocks.
    pub code_blocks: CodeBlockCounts,
    /// Tables.
    pub tables: u64,
    /// Footnote definitions.
    pub footnotes: u64,
    /// Task list items.
    pub tasks: TaskCounts,
}

/// Headings per level, in [`Counts::headings`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[allow(missing_docs, reason = "one field per heading level")]
pub struct HeadingCounts {
    pub h1: u64,
    pub h2: u64,
    pub h3: u64,
    pub h4: u64,
    pub h5: u64,
    pub h6: u64,
}

impl HeadingCounts {
    /// The counts of levels 1–6.
    pub const fn levels(&self) -> [u64; 6] {
        [self.h1, self.h2, self.h3, self.h4, self.h5, self.h6]
    }

    /// Headings of any level.
    pub fn total(&self) -> u64 {
        self.levels().iter().sum()
    }

    fn level_mut(&mut self, level: u8) -> Option<&mut u64> {
        match level {
            1 => Some(&mut self.h1),
            2 => Some(&mut self.h2),
            3 => Some(&mut self.h3),
            4 => Some(&mut self.h4),
            5 => Some(&mut self.h5),
            6 => Some(&mut self.h6),
            _ => None,
        }
    }
}

/// Links by destination, in [`Counts::links`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LinkCounts {
    /// Relative paths, `#anchors`, wikilinks and note embeds.
    pub internal: u64,
    /// Destinations with a URL scheme (`https:`, `mailto:`, …) or starting with `//`.
    pub external: u64,
}

impl LinkCounts {
    /// Links of either kind.
    pub const fn total(&self) -> u64 {
        self.internal + self.external
    }
}

/// Code blocks, in [`Counts::code_blocks`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CodeBlockCounts {
    /// All code blocks, with or without a language.
    pub total: u64,
    /// Blocks per language (the first word of the info string, lowercased), sorted by
    /// name. Blocks without a language only count towards [`total`](Self::total).
    pub languages: BTreeMap<String, u64>,
}

/// Task list items, in [`Counts::tasks`]. Custom states (`[-]`, `[?]`) are neither.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskCounts {
    /// `[ ]`
    pub open: u64,
    /// `[x]` / `[X]`
    pub done: u64,
}

impl TaskCounts {
    /// Open and done tasks.
    pub const fn total(&self) -> u64 {
        self.open + self.done
    }
}

impl Counts {
    /// Counts the contents of `doc`.
    pub fn of(doc: &Document, options: &StatsOptions) -> Self {
        let ast = doc.ast();
        let mut counts = Self::default();
        let mut text = String::new();
        let mut events = ast.traverse(ast.root());
        while let Some(event) = events.next() {
            let Event::Enter(id) = event else { continue };
            let kind = ast.kind(id);
            if kind.is_block() {
                // Words never run across block boundaries.
                text.push('\n');
            }
            match kind {
                NodeKind::Text(literal) | NodeKind::Code(literal) => text.push_str(literal),
                NodeKind::SoftBreak | NodeKind::LineBreak => text.push(' '),
                NodeKind::Tag(tag) => text.push_str(&tag.name),
                NodeKind::Heading(heading) => {
                    if let Some(count) = counts.headings.level_mut(heading.level) {
                        *count += 1;
                    }
                }
                NodeKind::ListItem(item) => match item.task {
                    Some(TaskState::Open) => counts.tasks.open += 1,
                    Some(TaskState::Done) => counts.tasks.done += 1,
                    Some(TaskState::Other(_)) | None => {}
                },
                NodeKind::CodeBlock(code) => {
                    counts.code_blocks.total += 1;
                    if let Some(lang) = code.lang() {
                        *counts
                            .code_blocks
                            .languages
                            .entry(lang.to_lowercase())
                            .or_default() += 1;
                    }
                    if options.include_code {
                        text.push_str(&code.literal);
                    }
                }
                NodeKind::Math(math) => {
                    if options.include_code {
                        text.push(' ');
                        text.push_str(&math.literal);
                        text.push(' ');
                    }
                }
                NodeKind::Table(_) => counts.tables += 1,
                NodeKind::FootnoteDefinition(_) => counts.footnotes += 1,
                NodeKind::Link(Link { url, .. }) => {
                    if is_external(url) {
                        counts.links.external += 1;
                    } else {
                        counts.links.internal += 1;
                    }
                }
                NodeKind::Image(_) => counts.images += 1,
                NodeKind::WikiLink(link) => {
                    counts.links.internal += 1;
                    text.push_str(&wikilink_text(link));
                }
                NodeKind::Embed(embed) => {
                    if is_image(&embed.target) {
                        counts.images += 1;
                    } else {
                        counts.links.internal += 1;
                    }
                    text.push_str(embed.alias.as_deref().unwrap_or_default());
                }
                NodeKind::Admonition(Admonition { title, .. })
                | NodeKind::Callout(Callout { title, .. }) => {
                    text.push_str(title.as_deref().unwrap_or_default());
                }
                NodeKind::FrontMatter(_)
                | NodeKind::HtmlBlock(_)
                | NodeKind::HtmlInline(_)
                | NodeKind::Comment(_)
                | NodeKind::LinkDefinition(_)
                | NodeKind::AbbreviationDefinition(_)
                | NodeKind::IncludeDirective(_)
                | NodeKind::TocDirective(_)
                | NodeKind::DirectiveEnd => events.skip_children(),
                _ => {}
            }
        }
        counts.words = text.unicode_words().count() as u64;
        counts.characters = text
            .graphemes(true)
            .filter(|g| !g.chars().all(char::is_whitespace))
            .count() as u64;
        counts
    }
}

/// What a wikilink shows: its alias, else its target and anchor.
fn wikilink_text(link: &WikiLink) -> String {
    match (&link.alias, &link.anchor) {
        (Some(alias), _) => alias.clone(),
        (None, Some(anchor)) => format!("{} {anchor}", link.target),
        (None, None) => link.target.clone(),
    }
}

/// Whether a destination has a URL scheme (RFC 3986: a letter, then letters, digits,
/// `+`, `-` or `.`, then `:`) or is protocol-relative (`//host/…`).
fn is_external(url: &str) -> bool {
    if url.starts_with("//") {
        return true;
    }
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Whether an embed target names an image file, by extension.
fn is_image(target: &str) -> bool {
    const EXTENSIONS: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "avif", "ico", "tif", "tiff",
    ];
    target.rsplit_once('.').is_some_and(|(_, ext)| {
        EXTENSIONS
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known))
    })
}

impl AddAssign<&Self> for Counts {
    fn add_assign(&mut self, other: &Self) {
        self.words += other.words;
        self.characters += other.characters;
        for (level, n) in (1..).zip(other.headings.levels()) {
            if let Some(count) = self.headings.level_mut(level) {
                *count += n;
            }
        }
        self.links.internal += other.links.internal;
        self.links.external += other.links.external;
        self.images += other.images;
        self.code_blocks.total += other.code_blocks.total;
        for (lang, n) in &other.code_blocks.languages {
            *self.code_blocks.languages.entry(lang.clone()).or_default() += n;
        }
        self.tables += other.tables;
        self.footnotes += other.footnotes;
        self.tasks.open += other.tasks.open;
        self.tasks.done += other.tasks.done;
    }
}

impl AddAssign for Counts {
    fn add_assign(&mut self, other: Self) {
        *self += &other;
    }
}

impl Add for Counts {
    type Output = Self;

    fn add(mut self, other: Self) -> Self {
        self += &other;
        self
    }
}

impl Sum for Counts {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), Add::add)
    }
}

impl<'a> Sum<&'a Self> for Counts {
    fn sum<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
        iter.fold(Self::default(), |mut total, counts| {
            total += counts;
            total
        })
    }
}

/// Minutes to read `words` at `wpm` words per minute, rounded up; 0 for no words or a
/// speed of 0.
pub const fn reading_minutes(words: u64, wpm: u32) -> u64 {
    if wpm == 0 {
        return 0;
    }
    words.div_ceil(wpm as u64)
}

/// The statistics of several documents and their total, as printed by `mrq stats`
/// (`--json` serializes this; [`table`](Self::table) is the human form).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Report {
    /// Reading speed of the reading-time estimates, in words per minute.
    pub wpm: u32,
    /// One entry per document, in the order pushed.
    pub files: Vec<FileStats>,
    /// The sum over all documents.
    pub total: TotalStats,
}

/// One document in a [`Report`]. In JSON, the [`Counts`] fields sit next to `file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct FileStats {
    /// The document's path as given; `null` for stdin and in-memory text.
    pub file: Option<String>,
    /// [`reading_minutes`] of the document's words.
    pub reading_minutes: u64,
    /// The counters.
    #[serde(flatten)]
    pub counts: Counts,
}

/// The total of a [`Report`]. In JSON, the [`Counts`] fields sit next to `files`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TotalStats {
    /// Number of documents.
    pub files: u64,
    /// [`reading_minutes`] of all words together (not the sum of per-file minutes).
    pub reading_minutes: u64,
    /// The sum of the documents' counters.
    #[serde(flatten)]
    pub counts: Counts,
}

/// Name of a document without a path in [`Report::table`].
const STDIN_NAME: &str = "<stdin>";

impl Report {
    /// An empty report estimating reading time at `wpm` words per minute.
    pub fn new(wpm: u32) -> Self {
        Self {
            wpm,
            files: Vec::new(),
            total: TotalStats::default(),
        }
    }

    /// Adds the `counts` of `doc` (usually [`Counts::of`]) and updates the total.
    pub fn push(&mut self, doc: &Document, counts: Counts) {
        self.total.files += 1;
        self.total.counts += &counts;
        self.total.reading_minutes = reading_minutes(self.total.counts.words, self.wpm);
        self.files.push(FileStats {
            file: doc.path().map(|path| path.display().to_string()),
            reading_minutes: reading_minutes(counts.words, self.wpm),
            counts,
        });
    }

    /// An aligned text table: a header row, one row per file, and a `total` row when
    /// there is more than one file. Headings are shown per level (`h1/h2/…/h6`), links
    /// as `internal/external`, tasks as `open/done`, and code blocks as the total with
    /// the per-language counts in parentheses.
    pub fn table(&self) -> String {
        let mut rows = vec![HEADER.map(str::to_owned).to_vec()];
        for file in &self.files {
            let name = file.file.as_deref().unwrap_or(STDIN_NAME);
            rows.push(row(name, file.reading_minutes, &file.counts));
        }
        if self.files.len() > 1 {
            rows.push(row("total", self.total.reading_minutes, &self.total.counts));
        }
        render_table(&rows)
    }
}

/// Column titles of [`Report::table`].
const HEADER: [&str; 11] = [
    "file",
    "words",
    "chars",
    "reading",
    "headings h1-h6",
    "links int/ext",
    "images",
    "tables",
    "footnotes",
    "tasks open/done",
    "code blocks",
];

fn row(name: &str, minutes: u64, counts: &Counts) -> Vec<String> {
    let levels = counts.headings.levels().map(|n| n.to_string()).join("/");
    let mut code = counts.code_blocks.total.to_string();
    if !counts.code_blocks.languages.is_empty() {
        let langs: Vec<String> = counts
            .code_blocks
            .languages
            .iter()
            .map(|(lang, n)| format!("{lang} {n}"))
            .collect();
        let _ = write!(code, " ({})", langs.join(", "));
    }
    vec![
        name.to_owned(),
        counts.words.to_string(),
        counts.characters.to_string(),
        format!("{minutes} min"),
        levels,
        format!("{}/{}", counts.links.internal, counts.links.external),
        counts.images.to_string(),
        counts.tables.to_string(),
        counts.footnotes.to_string(),
        format!("{}/{}", counts.tasks.open, counts.tasks.done),
        code,
    ]
}

/// Lays out `rows` with two spaces between columns: the first and last column
/// left-aligned, the others right-aligned; no trailing whitespace.
fn render_table(rows: &[Vec<String>]) -> String {
    let columns = rows.first().map_or(0, Vec::len);
    let widths: Vec<usize> = (0..columns)
        .map(|col| {
            rows.iter()
                .map(|row| row[col].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for row in rows {
        let mut line = String::new();
        for (col, cell) in row.iter().enumerate() {
            let width = widths[col];
            if col > 0 {
                line.push_str("  ");
            }
            if col == 0 || col + 1 == columns {
                let _ = write!(line, "{cell:<width$}");
            } else {
                let _ = write!(line, "{cell:>width$}");
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}
