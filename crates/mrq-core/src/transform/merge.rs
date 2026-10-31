//! Merging documents into one: [`merge`].

use std::path::PathBuf;

use serde_json::{Map, Value};

use super::{LinkMap, LinkRewrite, Location, ShiftOptions, rewrite_links, shift_headings};
use crate::ast::{FrontMatterFormat, NodeKind};
use crate::diag::{Diagnostic, Severity};
use crate::document::Document;
use crate::frontmatter::Frontmatter;
use crate::source::{ByteRange, SourceFile, apply_edits};

/// Code of the warning for a file whose headings could not be matched with the merged
/// document's, so links to its anchors are not redirected.
pub const MERGE_ANCHORS_CODE: &str = "merge-anchors";

/// How [`merge`] combines the files' frontmatter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrontmatterMerge {
    /// The first frontmatter block, as written.
    KeepFirst,
    /// One mapping of all blocks' keys; on conflicts the first file wins.
    #[default]
    Merge,
    /// No frontmatter.
    Strip,
}

/// How [`merge`] merges.
#[derive(Debug, Clone)]
pub struct MergeOptions {
    title: Option<String>,
    wrap: bool,
    shift: Option<u8>,
    frontmatter: FrontmatterMerge,
    output: Option<PathBuf>,
    root: Option<PathBuf>,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            title: None,
            wrap: true,
            shift: None,
            frontmatter: FrontmatterMerge::default(),
            output: None,
            root: None,
        }
    }
}

impl MergeOptions {
    /// Wrapping each file in a section, no title, the default shift, merged frontmatter,
    /// output in the current directory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts the document with a level-1 heading of `title` (plain text).
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Whether each file becomes a section with a heading of its own (default `true`).
    #[must_use]
    pub const fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// Levels to push each file's headings down by. The default is one per level above
    /// them: the [title](Self::title) and the [section headings](Self::wrap).
    #[must_use]
    pub const fn shift(mut self, shift: u8) -> Self {
        self.shift = Some(shift);
        self
    }

    /// Sets the [`FrontmatterMerge`] strategy.
    #[must_use]
    pub const fn frontmatter(mut self, frontmatter: FrontmatterMerge) -> Self {
        self.frontmatter = frontmatter;
        self
    }

    /// The file the merged document is written to, which links are made relative to
    /// (default: a file in the current directory, as for stdout).
    #[must_use]
    pub fn output(mut self, output: impl Into<PathBuf>) -> Self {
        self.output = Some(output.into());
        self
    }

    /// The directory a leading `/` in links stands for (default: the current directory).
    #[must_use]
    pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }
}

/// The result of [`merge`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Merged {
    /// The merged document.
    pub text: String,
    /// Warnings about the input files: heading clamps
    /// ([`HEADING_CLAMP_CODE`](super::HEADING_CLAMP_CODE)) and [`MERGE_ANCHORS_CODE`].
    pub diagnostics: Vec<Diagnostic>,
}

impl MergeOptions {
    /// The shift: as set, else one per level above the files' headings.
    fn levels(&self) -> u8 {
        self.shift
            .unwrap_or_else(|| u8::from(self.wrap) + u8::from(self.title.is_some()))
    }
}

/// Merges `docs`, in order, into one document: the merged frontmatter, a level-1
/// heading of the [title](MergeOptions::title) if set, then each file's body (without
/// leading and trailing blank lines), separated by blank lines.
///
/// With [wrapping](MergeOptions::wrap), each file is a section headed at level
/// `max(shift, 1)`, titled by its frontmatter `title`, else its first top-level H1
/// (with its inline markup), else its file stem; plain-text titles are escaped. A file
/// without a frontmatter `title` whose body starts with its only top-level H1 is a
/// section already: that H1 is the section heading, and the whole file is shifted to
/// put it at the section level. Every other file gets a section heading inserted and its
/// headings shifted by `shift`. Without wrapping, every file's headings are shifted by
/// `shift`. Levels are clamped to 1–6 ([`shift_headings`]).
///
/// Links are rewritten to work from the [output](MergeOptions::output)
/// ([`rewrite_links`]): a link to a merged file goes to its section heading (without
/// wrapping, to its first heading if the file starts with one, else to `#`), and one to
/// an anchor of a merged file goes to that heading in the merged document, whose
/// anchors the flavor's slugger makes unique (`setup`, `setup-1`). Other relative links
/// and assets are rebased. The merged document is parsed with the first document's
/// flavor.
///
/// Frontmatter follows [`FrontmatterMerge`]. Merged frontmatter is written as the first
/// block was, unchanged if merging adds nothing to it, else re-serialized in its format
/// (YAML, or TOML when that can hold the data); a [title](MergeOptions::title) replaces
/// its `title`. Malformed blocks are left out.
///
/// Known limits: reference definitions and footnotes keep their labels, so the same
/// label in two files collides; explicit `{#id}`s are not made unique; bare wikilinks
/// (`[[name]]`) are vault-wide names and stay as written; a file ending in an unclosed
/// code fence swallows the rest.
pub fn merge(docs: &[Document], options: &MergeOptions) -> Merged {
    let shift = options.levels();
    let level = shift.clamp(1, 6);
    let mut diagnostics = Vec::new();
    let parts: Vec<Part<'_>> = docs
        .iter()
        .map(|doc| Part::new(doc, options, shift, level, &mut diagnostics))
        .collect();
    let title = options
        .title
        .as_deref()
        .map(|title| format!("# {}", escape(title, docs.first())));
    let Some(first) = docs.first() else {
        let text = assemble(None, title.as_deref(), &[]).0;
        return Merged { text, diagnostics };
    };

    // The merged document before links are rewritten has the anchors they need.
    let draft: Vec<(Option<String>, &str)> = parts
        .iter()
        .map(|part| (part.heading(&part.shifted), part.body(&part.shifted)))
        .collect();
    let output = options.output.clone().unwrap_or_else(|| PathBuf::from("-"));
    let (draft, spans) = assemble(None, title.as_deref(), &draft);
    let draft = first.reparse(SourceFile::new(Some(output.clone()), draft));
    let anchors: Vec<(usize, &str)> = draft
        .anchors()
        .iter()
        .map(|(id, anchor)| (draft.ast().span(id).start, anchor))
        .collect();
    let mut map = LinkMap::new();
    for (part, span) in parts.iter().zip(&spans) {
        part.map(&anchors, span, &mut map, &mut diagnostics);
    }

    let root = options.root.clone().unwrap_or_default();
    let rewritten: Vec<Document> = parts
        .iter()
        .map(|part| {
            let rewrite = LinkRewrite::new(part.doc.name(), &output)
                .root(&root)
                .map(&map);
            let text = part.shifted.text();
            let edits = rewrite_links(
                &part.shifted,
                ByteRange::new(part.body_start, text.len()),
                &rewrite,
            );
            if edits.is_empty() {
                return part.shifted.clone();
            }
            let text = apply_edits(text, &edits).expect("link edits do not overlap");
            part.shifted
                .reparse(SourceFile::new(part.doc.path().map(Into::into), text))
        })
        .collect();
    let pieces: Vec<(Option<String>, &str)> = parts
        .iter()
        .zip(&rewritten)
        .map(|(part, doc)| (part.heading(doc), part.body(doc)))
        .collect();
    let front = frontmatter(docs, options);
    let text = assemble(front.as_deref(), title.as_deref(), &pieces).0;
    Merged { text, diagnostics }
}

/// A file being merged.
struct Part<'d> {
    doc: &'d Document,
    /// `doc` with its headings shifted.
    shifted: Document,
    /// Where the body starts, after the frontmatter (the same in `doc` and `shifted`).
    body_start: usize,
    /// The section heading to insert, if any.
    heading: Option<Title>,
    level: u8,
}

/// The title of an inserted section heading.
enum Title {
    /// Markdown text.
    Text(String),
    /// The text of the top-level heading at this index among the root's children.
    Heading(usize),
}

impl<'d> Part<'d> {
    /// `doc` shifted as a section at `level`, or by `shift` levels without wrapping.
    fn new(
        doc: &'d Document,
        options: &MergeOptions,
        shift: u8,
        level: u8,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Self {
        let ast = doc.ast();
        let blocks = ast.children(ast.root());
        let h1s: Vec<usize> = (0..blocks.len())
            .filter(|&i| matches!(ast.kind(blocks[i]), NodeKind::Heading(h) if h.level == 1))
            .collect();
        let first_block =
            (0..blocks.len()).find(|&i| !matches!(ast.kind(blocks[i]), NodeKind::FrontMatter(_)));
        let front_title = doc
            .frontmatter()
            .and_then(|front| front.get_str(&["title"]))
            .map(str::trim)
            .filter(|title| !title.is_empty());
        let sectioned =
            front_title.is_none() && h1s.len() == 1 && first_block == h1s.first().copied();
        let (by, heading) = if !options.wrap {
            (shift, None)
        } else if sectioned {
            (level - 1, None)
        } else {
            let title = match (front_title, h1s.first()) {
                (Some(title), _) => Title::Text(escape(title, Some(doc))),
                (None, Some(&index)) => Title::Heading(index),
                (None, None) => {
                    let stem = doc
                        .path()
                        .and_then(|path| path.file_stem())
                        .map_or_else(|| "stdin".into(), |stem| stem.to_string_lossy());
                    Title::Text(escape(&stem, Some(doc)))
                }
            };
            (shift, Some(title))
        };

        let body_start = doc.frontmatter().map_or(0, Frontmatter::body_start);
        let text = doc.text();
        let shifted = shift_headings(
            doc,
            ByteRange::new(body_start, text.len()),
            ShiftOptions::by(i32::from(by)),
        );
        diagnostics.extend(shifted.diagnostics);
        let shifted = if shifted.edits.is_empty() {
            doc.clone()
        } else {
            let text = apply_edits(text, &shifted.edits).expect("shift edits do not overlap");
            doc.reparse(SourceFile::new(doc.path().map(Into::into), text))
        };
        Self {
            doc,
            shifted,
            body_start,
            heading,
            level,
        }
    }

    /// The section heading line to insert, with its title as it reads in `version` (the
    /// shifted document, or that with its links rewritten).
    fn heading(&self, version: &Document) -> Option<String> {
        let title = match self.heading.as_ref()? {
            Title::Text(text) => text.clone(),
            Title::Heading(index) => heading_title(version, *index),
        };
        Some(format!("{} {title}", "#".repeat(usize::from(self.level))))
    }

    /// The body of `version` without leading blank lines and trailing whitespace.
    fn body<'v>(&self, version: &'v Document) -> &'v str {
        let mut body = &version.text()[self.body_start..];
        while let Some((line, rest)) = body.split_once('\n') {
            if !line.trim().is_empty() {
                break;
            }
            body = rest;
        }
        body.trim_end()
    }

    /// Records in `map` where this part's file and anchors went: `span` is where its
    /// heading and body are in the draft, whose headings start at the offsets of
    /// `anchors`.
    fn map(
        &self,
        anchors: &[(usize, &str)],
        span: &PartSpan,
        map: &mut LinkMap,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let in_body: Vec<(usize, &str)> = anchors
            .iter()
            .copied()
            .filter(|&(at, _)| span.body.start <= at && at < span.body.end)
            .collect();
        let section = match span.heading {
            Some(heading) => anchors.iter().find(|&&(at, _)| at == heading),
            None => in_body.first().filter(|&&(at, _)| at == span.body.start),
        };
        let file = self.doc.name();
        map.insert_file(
            file,
            section.map_or_else(Location::output, |&(_, anchor)| {
                Location::output().with_anchor(anchor)
            }),
        );
        let own = self.doc.anchors();
        if own.len() != in_body.len() {
            diagnostics.push(Diagnostic::new(
                MERGE_ANCHORS_CODE,
                Severity::Warning,
                file,
                ByteRange::new(self.body_start, self.body_start),
                format!(
                    "the merged document has {} headings from this file instead of {}; links to its anchors are left as they are",
                    in_body.len(),
                    own.len()
                ),
            ));
            return;
        }
        for ((_, from), &(_, to)) in own.iter().zip(&in_body) {
            map.insert_anchor(file, from, Location::output().with_anchor(to));
        }
    }
}

/// Where a part's inserted heading starts and its body is, in the merged text.
#[derive(Debug, Clone, Copy)]
struct PartSpan {
    heading: Option<usize>,
    body: ByteRange,
}

/// The merged text: `front`, `title`, then each part's heading and body, as blocks
/// separated by blank lines; empty pieces are left out. Also where each part's heading
/// and body are.
fn assemble(
    front: Option<&str>,
    title: Option<&str>,
    parts: &[(Option<String>, &str)],
) -> (String, Vec<PartSpan>) {
    let mut text = front.unwrap_or_default().to_owned();
    let push = |text: &mut String, block: &str| {
        if !text.is_empty() {
            text.push('\n');
        }
        let at = text.len();
        text.push_str(block);
        text.push('\n');
        at
    };
    if let Some(title) = title {
        push(&mut text, title);
    }
    let spans = parts
        .iter()
        .map(|(heading, body)| {
            let heading = heading.as_deref().map(|heading| push(&mut text, heading));
            let body = if body.is_empty() {
                ByteRange::new(text.len(), text.len())
            } else {
                let at = push(&mut text, body);
                ByteRange::new(at, at + body.len())
            };
            PartSpan { heading, body }
        })
        .collect();
    (text, spans)
}

/// The text of the heading at `index` among `doc`'s top-level blocks, as written if that
/// is one line of inline Markdown, else as escaped plain text.
fn heading_title(doc: &Document, index: usize) -> String {
    let ast = doc.ast();
    let id = ast.children(ast.root())[index];
    let children = ast.children(id);
    let plain = || escape(&ast.text(id), Some(doc));
    let NodeKind::Heading(heading) = ast.kind(id) else {
        return plain();
    };
    let (Some(&first), Some(&last)) = (children.first(), children.last()) else {
        return plain();
    };
    let written = doc.text()[ast.span(first).start..ast.span(last).end].trim();
    if heading.attrs.is_some() || written.contains(['\n', '\r']) || written.ends_with('#') {
        plain()
    } else {
        written.to_owned()
    }
}

/// `text` as heading content that reads as `text`: whitespace runs collapsed, and
/// characters that could start markup backslash-escaped (`[` as `&#91;` in mkdocs, which
/// reads `\[` as math).
fn escape(text: &str, doc: Option<&Document>) -> String {
    let mkdocs = doc.is_some_and(|doc| doc.flavor().name() == "mkdocs");
    let mut out = String::with_capacity(text.len());
    let mut chars = text
        .split_whitespace()
        .flat_map(|word| word.chars().chain([' ']))
        .peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' if chars.peek().is_none() => {}
            '[' if mkdocs => out.push_str("&#91;"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' | '~' | '$' | '%' | '{' | '}'
            | '#' | '=' => {
                out.push('\\');
                out.push(c);
            }
            '!' if chars.peek() == Some(&'[') => out.push_str("\\!"),
            '&' if chars
                .peek()
                .is_some_and(|next| next.is_alphanumeric() || *next == '#') =>
            {
                out.push_str("\\&");
            }
            c => out.push(c),
        }
    }
    out
}

/// The merged frontmatter block, ending with a newline, if there is one.
fn frontmatter(docs: &[Document], options: &MergeOptions) -> Option<String> {
    let blocks: Vec<(&Document, &Frontmatter)> = docs
        .iter()
        .filter_map(|doc| Some((doc, doc.frontmatter()?)))
        .collect();
    let written = |(doc, front): (&Document, &Frontmatter)| {
        format!("{}\n", &doc.text()[front.span().start..front.span().end])
    };
    match options.frontmatter {
        FrontmatterMerge::Strip => None,
        FrontmatterMerge::KeepFirst => blocks.first().copied().map(written),
        FrontmatterMerge::Merge => {
            let blocks: Vec<_> = blocks
                .into_iter()
                .filter(|(_, front)| front.value().is_some_and(Value::is_object))
                .collect();
            let &(_, first) = blocks.first()?;
            let mut merged = Map::new();
            for (_, front) in &blocks {
                if let Some(Value::Object(value)) = front.value() {
                    merge_into(&mut merged, value);
                }
            }
            if let Some(title) = &options.title
                && let Some(value) = merged.get_mut("title")
            {
                *value = Value::String(title.clone());
            }
            if first.value() == Some(&Value::Object(merged.clone())) {
                return Some(written(blocks[0]));
            }
            if first.format() == FrontMatterFormat::Toml
                && let Some(toml) = toml(&merged)
            {
                return Some(format!("+++\n{toml}+++\n"));
            }
            let yaml = serde_saphyr::to_string(&Value::Object(merged)).ok()?;
            let newline = if yaml.ends_with('\n') { "" } else { "\n" };
            Some(format!("---\n{yaml}{newline}---\n"))
        }
    }
}

/// Adds the keys of `from` that `into` lacks, recursively in mappings both have.
fn merge_into(into: &mut Map<String, Value>, from: &Map<String, Value>) {
    for (key, value) in from {
        match (into.get_mut(key), value) {
            (Some(Value::Object(into)), Value::Object(from)) => merge_into(into, from),
            (Some(_), _) => {}
            (None, _) => {
                into.insert(key.clone(), value.clone());
            }
        }
    }
}

/// `value` as a TOML document (nested tables inline); `None` if it holds a null.
fn toml(value: &Map<String, Value>) -> Option<String> {
    let mut doc = toml_edit::DocumentMut::new();
    for (key, value) in value {
        doc.insert(key, toml_edit::Item::Value(toml_value(value)?));
    }
    Some(doc.to_string())
}

fn toml_value(value: &Value) -> Option<toml_edit::Value> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(value) => (*value).into(),
        Value::Number(number) => match number.as_i64() {
            Some(int) => int.into(),
            None => number.as_f64()?.into(),
        },
        Value::String(value) => value.as_str().into(),
        Value::Array(values) => {
            let mut array = toml_edit::Array::new();
            for value in values {
                array.push(toml_value(value)?);
            }
            array.into()
        }
        Value::Object(map) => {
            let mut table = toml_edit::InlineTable::new();
            for (key, value) in map {
                table.insert(key, toml_value(value)?);
            }
            table.into()
        }
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::document::DocumentOptions;
    use crate::frontmatter::Frontmatter;
    use crate::source::SourceFile;
    use crate::transform::HEADING_CLAMP_CODE;

    /// Documents at the paths with the texts of `files`.
    fn docs(files: &[(&str, &str)]) -> Vec<Document> {
        files
            .iter()
            .map(|&(path, text)| {
                DocumentOptions::new()
                    .parse_source(SourceFile::new(Some(path.into()), text))
                    .unwrap()
            })
            .collect()
    }

    fn merged(files: &[(&str, &str)], options: &MergeOptions) -> String {
        let merged = merge(&docs(files), options);
        assert_eq!(merged.diagnostics, []);
        merged.text
    }

    fn stripped() -> MergeOptions {
        MergeOptions::new().frontmatter(FrontmatterMerge::Strip)
    }

    #[test]
    fn sections_are_titled_by_frontmatter_then_first_h1_then_stem() {
        let files = [
            ("a.md", "---\ntitle: Alpha\n---\n\nText a.\n"),
            ("b.md", "Intro b.\n\n# Beta\n\n## Sub\n"),
            ("c.md", "## Only sub"),
        ];
        assert_eq!(
            merged(&files, &stripped()),
            "# Alpha\n\nText a.\n\n# Beta\n\nIntro b.\n\n## Beta\n\n### Sub\n\n# c\n\n### Only sub\n"
        );
    }

    #[test]
    fn a_leading_only_h1_is_the_section_heading() {
        let files = [
            ("a.md", "# Install\n\n## Linux\n"),
            ("b.md", "# Use\n\nText\n\n# More\n"),
        ];
        assert_eq!(
            merged(&files, &stripped()),
            "# Install\n\n## Linux\n\n# Use\n\n## Use\n\nText\n\n## More\n"
        );
    }

    #[test]
    fn a_title_pushes_sections_down_by_default() {
        let files = [("a.md", "# Install\n\n## Linux\n"), ("b.md", "Plain.\n")];
        assert_eq!(
            merged(&files, &stripped().title("User Guide")),
            "# User Guide\n\n## Install\n\n### Linux\n\n## b\n\nPlain.\n"
        );
        assert_eq!(
            merged(&files, &stripped().title("User Guide").shift(1)),
            "# User Guide\n\n# Install\n\n## Linux\n\n# b\n\nPlain.\n"
        );
    }

    #[test]
    fn without_wrapping_files_are_concatenated() {
        let files = [("a.md", "# A\n\ntext\n"), ("b.md", "\n\n# B")];
        let options = stripped().wrap(false);
        assert_eq!(merged(&files, &options), "# A\n\ntext\n\n# B\n");
        assert_eq!(
            merged(&files, &options.clone().title("T")),
            "# T\n\n## A\n\ntext\n\n## B\n"
        );
        assert_eq!(
            merged(&files, &options.shift(2)),
            "### A\n\ntext\n\n### B\n"
        );
    }

    #[test]
    fn titles_are_escaped() {
        let files = [("my_notes.md", "Text\n")];
        assert_eq!(
            merged(&files, &stripped().title("A *b* [c] <d> #1")),
            "# A \\*b\\* \\[c\\] \\<d\\> \\#1\n\n## my\\_notes\n\nText\n"
        );
    }

    #[test]
    fn a_title_from_an_h1_keeps_its_markup_and_rebased_links() {
        let files = [("docs/a.md", "Intro\n\n# Using `mrq` with [x](x.md)\n")];
        assert_eq!(
            merged(&files, &stripped().output("guide.md")),
            "# Using `mrq` with [x](docs/x.md)\n\nIntro\n\n## Using `mrq` with [x](docs/x.md)\n"
        );
    }

    #[test]
    fn cross_file_links_become_anchors_and_colliding_anchors_are_unique() {
        let files = [
            (
                "docs/a.md",
                "# Setup\n\nSee [b](b.md), [b setup](b.md#setup), [here](#setup) and ![x](img/x.png).\n",
            ),
            (
                "docs/b.md",
                "# Guide B\n\n## Setup\n\nBack to [a](./a.md#setup), [self](#setup), [c](../c.md).\n",
            ),
        ];
        assert_eq!(
            merged(&files, &stripped().output("guide.md")),
            "# Setup\n\nSee [b](#guide-b), [b setup](#setup-1), [here](#setup) and ![x](docs/img/x.png).\n\n\
             # Guide B\n\n## Setup\n\nBack to [a](#setup), [self](#setup-1), [c](c.md).\n"
        );
    }

    #[test]
    fn links_are_relative_to_the_output() {
        let files = [
            (
                "a.md",
                "# A\n\n[b](b.md) [c](c.md) ![i](img/i.png) [r]\n\n[r]: /r.md\n",
            ),
            ("b.md", "# B\n"),
        ];
        assert_eq!(
            merged(&files, &stripped().output("out/guide.md")),
            "# A\n\n[b](#b) [c](../c.md) ![i](../img/i.png) [r]\n\n[r]: /r.md\n\n# B\n"
        );
    }

    #[test]
    fn links_to_unwrapped_files_go_to_their_first_heading_or_the_top() {
        let files = [
            ("a.md", "[b](b.md) [c](c.md)\n"),
            ("b.md", "# Bee\n"),
            ("c.md", "Sea\n"),
        ];
        assert_eq!(
            merged(&files, &stripped().wrap(false)),
            "[b](#bee) [c](#)\n\n# Bee\n\nSea\n"
        );
    }

    #[test]
    fn shifted_setext_headings_keep_their_rebased_links() {
        let files = [("docs/a.md", "Title [x](img/x.png)\n===\n\nSub\n---\n")];
        assert_eq!(
            merged(&files, &stripped().title("T").output("guide.md")),
            "# T\n\n## Title [x](docs/img/x.png)\n\n### Sub\n"
        );
    }

    #[test]
    fn clamped_headings_are_reported() {
        let merged = merge(
            &docs(&[("a.md", "##### Five\n\n###### Six\n")]),
            &stripped().wrap(false).shift(1),
        );
        assert_eq!(merged.text, "###### Five\n\n###### Six\n");
        let codes: Vec<(&str, &str)> = merged
            .diagnostics
            .iter()
            .map(|d| (d.code.as_str(), d.file.to_str().unwrap()))
            .collect();
        assert_eq!(codes, [(HEADING_CLAMP_CODE, "a.md")]);
    }

    fn front(text: &str) -> serde_json::Value {
        Frontmatter::extract(text).unwrap().value().unwrap().clone()
    }

    #[test]
    fn frontmatter_is_merged_with_the_first_file_winning() {
        let files = [
            (
                "a.md",
                "---\ntitle: A\ntags: [x]\nmeta:\n  k: 1\n---\n# A\n",
            ),
            (
                "b.md",
                "---\ntags: [y]\nauthor: Bob\nmeta:\n  k: 2\n  j: 3\n---\n# B\n",
            ),
        ];
        let options = MergeOptions::new().wrap(false);
        let text = merged(&files, &options);
        assert_eq!(
            front(&text),
            json!({"title": "A", "tags": ["x"], "meta": {"k": 1, "j": 3}, "author": "Bob"})
        );
        assert!(text.ends_with("---\n\n# A\n\n# B\n"), "{text}");
        assert_eq!(
            front(&merged(&files, &options.title("Guide"))),
            json!({"title": "Guide", "tags": ["x"], "meta": {"k": 1, "j": 3}, "author": "Bob"})
        );
    }

    #[test]
    fn frontmatter_that_merging_does_not_change_is_kept_as_written() {
        let files = [
            ("a.md", "---\n# comment\ntitle: A\n---\n# A\n"),
            ("b.md", "---\ntitle: B\n---\n# B\n"),
            ("c.md", "# C\n"),
        ];
        assert_eq!(
            merged(&files, &MergeOptions::new().wrap(false)),
            "---\n# comment\ntitle: A\n---\n\n# A\n\n# B\n\n# C\n"
        );
    }

    #[test]
    fn toml_frontmatter_stays_toml() {
        let files = [
            ("a.md", "+++\ntitle = \"A\"\n+++\n# A\n"),
            ("b.md", "---\nauthor: Bob\nn: [1, 2.5]\n---\n# B\n"),
        ];
        let text = merged(&files, &MergeOptions::new().wrap(false));
        assert!(text.starts_with("+++\n"), "{text}");
        assert_eq!(
            front(&text),
            json!({"title": "A", "author": "Bob", "n": [1, 2.5]})
        );
    }

    #[test]
    fn frontmatter_can_be_kept_from_the_first_file_or_stripped() {
        let files = [
            ("a.md", "# A\n"),
            ("b.md", "---\ntitle: B # b\n---\n# B\n"),
            ("c.md", "---\nauthor: C\n---\n# C\n"),
        ];
        let options = MergeOptions::new().wrap(false);
        assert_eq!(
            merged(
                &files,
                &options.clone().frontmatter(FrontmatterMerge::KeepFirst)
            ),
            "---\ntitle: B # b\n---\n\n# A\n\n# B\n\n# C\n"
        );
        assert_eq!(
            merged(&files, &options.frontmatter(FrontmatterMerge::Strip)),
            "# A\n\n# B\n\n# C\n"
        );
    }

    #[test]
    fn nothing_merges_to_the_title_only() {
        assert_eq!(merged(&[], &MergeOptions::new()), "");
        assert_eq!(merged(&[], &MergeOptions::new().title("T")), "# T\n");
    }
}
