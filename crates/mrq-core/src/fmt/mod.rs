//! The Markdown printer behind `mrq fmt`: [`format()`] prints a [`Document`]'s tree back
//! to canonical Markdown that renders to the same HTML.
//!
//! ```
//! use mrq_core::Document;
//! use mrq_core::config::FmtConfig;
//! use mrq_core::fmt;
//!
//! let doc = Document::parse("Title\n=====\n* _one_\n* __two__\n");
//! assert_eq!(
//!     fmt::format(&doc, &FmtConfig::default()),
//!     "# Title\n\n- *one*\n- **two**\n"
//! );
//! ```
//!
//! # Conventions
//!
//! - Headings are ATX (`# …`, without a closing sequence), except a setext heading whose
//!   text spans lines, which stays setext because an ATX heading cannot.
//! - Blocks are separated by one blank line, except the items and the item contents of
//!   a tight list (where a paragraph after a block quote keeps an empty `>` line, so it
//!   does not continue the quote). Trailing whitespace goes (a hard break is printed as
//!   `\`, or as two spaces after a bare URL, which would take in the `\`; empty
//!   definition details keep the space after their `:`), and the output ends with
//!   exactly one newline; an empty document prints as nothing.
//! - Paragraphs are wrapped following `fmt.wrap`: `preserve` keeps their line breaks,
//!   `never` puts each on one line, `width` fills lines up to `fmt.width` columns
//!   (display width, with the prefixes of block quotes, list items and footnotes
//!   counted, and a task marker on the first line) by textwrap's optimal fit, and
//!   `sentence` puts each sentence on a line of its own. Lines break only at whitespace
//!   and soft breaks (a run of which becomes one space where it does not break), never
//!   inside code spans, link destinations and titles, autolinks, raw HTML, math or
//!   comments, nor right before raw HTML or a comment, nor after raw HTML that starts
//!   the paragraph; hard breaks always end a line. A sentence ends at a word ending in
//!   `.`, `!` or `?` (before closing quotes, brackets or emphasis), unless the next word
//!   starts in lowercase or the word is an abbreviation (`e.g.`, `Dr.`, `etc.`, …), an
//!   initial (`J.`) or dotted (`U.S.`). Headings, table cells and definition terms are
//!   never wrapped. Words wider than the room left get a line of their own, and an
//!   escape added at the start of a line may take a line one column past the width.
//! - Emphasis, strong emphasis, bullet markers and code fences use `fmt.emphasis`,
//!   `fmt.strong`, `fmt.list_marker` and `fmt.code_fence`. Where the configured marker
//!   would change the meaning, another one is used: `*` for intraword emphasis, the
//!   other emphasis character where delimiters would touch (`**_a_**`), another bullet
//!   (or `)` instead of `.`) for a list right after a list of the same kind, a tilde
//!   fence for an info string with a backtick, and a longer fence than any fence-like
//!   line of the code.
//! - Thematic breaks are `---`, or `***` where that would be read as a setext
//!   underline, a list item or (first in the document) frontmatter.
//! - Indented code blocks become fenced.
//! - Ordered lists keep their start number, which is part of the HTML; the other items
//!   follow `fmt.ordered_list`: counting up from it (`ordered`), repeating it (`one`), or
//!   as written, leading zeros included (`preserve`). A number past nine digits repeats
//!   the start number. All content of an item is indented by the width of its own
//!   marker (so `10.` indents by four where `9.` indents by three), tight lists stay
//!   tight and loose ones loose, and task markers are `[ ]` and `[x]`, on a line of
//!   their own unless the item starts with a paragraph.
//! - Tables are pipe tables with leading and trailing pipes. Every column is padded to
//!   its widest cell (at least three), measured in display columns so CJK text and emoji
//!   line up; cells are padded on the side their alignment leaves open, the delimiter row
//!   keeps each column's alignment marker, and every row has one cell per column.
//! - Links, images and autolinks keep their form (inline, reference, `<…>`, bare);
//!   destinations with spaces are written `<…>` and titles in double quotes.
//! - Reference definitions are recovered from the source (the parser drops them):
//!   each is printed where it was, normalized from the url and title its links resolve
//!   to. A label that links use but whose definition was not found gets a definition at
//!   the end of the document. A shortcut or collapsed reference whose printed text no
//!   longer equals its label (whitespace runs aside) becomes a full reference.
//!   In a tight list too, a blank line separates definitions from a block before them
//!   that would take them in (anything but a heading, a thematic break or a code
//!   block; the source had one there) and from a block after them that would be read
//!   as part of them (anything but a heading, a code block or a block quote; it does
//!   not make the list loose).
//! - Raw HTML and `mrq:` directives are printed verbatim, and so is frontmatter unless
//!   `fmt.frontmatter` is `normalize` (see [Frontmatter](#frontmatter)).
//! - Fenced code is printed as written, unless there is a [`CodeFormatter`] for its
//!   language (see [Code](#code)).
//! - Flavor syntax has one form in every flavor (only the escaping differs; see
//!   [`PrinterHooks`]):
//!   - callouts are `> [!kind]± title` (title as written), with an empty `>` line
//!     after the marker unless the body starts with a block that continues or
//!     interrupts a paragraph;
//!   - wikilinks and embeds are `[[target#anchor|alias]]` with their parts trimmed,
//!     tags `#name`;
//!   - `%%` comments, inline or block, and mkdocs' `\(…\)`/`\[…\]` math keep their
//!     source, with the container prefixes of their lines replaced by the printed
//!     ones;
//!   - admonitions are `!!! kind classes "title"` (`???`, `???+` when foldable) with
//!     the body indented by four spaces;
//!   - heading attributes are normalized to `{#id .class key=value}` (a bare word for
//!     `w=w`, values quoted only where needed), or kept as written where a value has
//!     both kinds of quotes;
//!   - definition details are `:` and three spaces, their content indented by four,
//!     after a blank line where the source had one (which makes them loose); terms
//!     are never wrapped (each line of a term is a term in Python-Markdown);
//!   - abbreviation definitions are `*[abbr]: expansion`, and stay on the lines of the
//!     paragraph or definitions they were written next to.
//!
//!   A flavor's hooks may also ask for any node kind to be printed from its source.
//!
//! # Escaping
//!
//! Text is escaped only where a character could start markup: always `\`, `` ` ``,
//! `[` and `]`; `*` and `_` where they could open emphasis, or close the emphasis they
//! are in (so `snake_case`, `2 * 3` and a trailing `x_` stay as they are); `<` when a
//! `>` may follow; `&` before a character reference; and the syntax of the enabled
//! extensions (`~`, `$`, `^`, `==`, `|` in table cells). At the start of a line block
//! markers are escaped too (`#`, `>`, `-`, `+`, `*`, `1.`, a line of `=` or `_`, a
//! table delimiter row, the `<` of an HTML block start, and `:` with definition
//! lists); with wrapping, they are escaped on each line once the lines are
//! known. Whitespace that the parser would strip (at the start or end of a line or of
//! emphasis) is written as a character reference such as `&#32;`.
//!
//! Flavors add their own rules ([`PrinterHooks`]):
//! `obsidian` escapes `#` where a tag could start and every `%%`; `mkdocs` escapes
//! `!!!` and `???` at the start of a line and a trailing `{…}` in heading text, and
//! writes an escaped `[` or `(` as `&#91;` or `&#40;`, since `\[` and `\(` open math.
//!
//! # Frontmatter
//!
//! With `fmt.frontmatter: normalize`, YAML frontmatter is written as serde-saphyr writes
//! its value (block style, strings quoted only where needed) and TOML as `toml_edit`
//! writes a document built from scratch (basic strings, decimal numbers, one space around
//! `=`, inline arrays and tables on one line, a blank line before each table header;
//! tables keep their kind). Frontmatter is left as written, with a
//! [`codes::FRONTMATTER_NORMALIZE`] warning, where it does not parse, where normalizing
//! would drop something (comments, YAML tags) or where its data could change: YAML keys
//! that are not strings, plain YAML scalars that YAML 1.1 and 1.2 read differently
//! (`yes`, `off`), and anything that does not read back to the same value.
//!
//! # Code
//!
//! [`format_with`] hands the code of every block with a language (the first word of its
//! info string; indented code has none) to a [`CodeFormatter`], in containers too.
//! [`format()`] uses a [`CommandFormatter`] for `fmt.code_formatters`, which pipes the code
//! through the command configured for the language (matched ignoring ASCII case) with a
//! timeout ([`CommandFormatter::DEFAULT_TIMEOUT`]). The formatted code gets a final
//! newline if it has none, and the fence is chosen for it. When a formatter fails (a
//! command that cannot start, exits non-zero, times out or writes something other than
//! UTF-8), the code is kept as written with a [`codes::CODE_FORMATTER`] warning. Fences
//! and the meaning of the Markdown are kept whatever the formatter does; idempotence then
//! depends on the formatter's.
//!
//! # Known limits
//!
//! Every `CommonMark` and GFM spec example keeps its HTML (`tests/fmt.rs`), with every
//! marker setting, in every wrap mode (where whitespace runs outside code blocks
//! compare as one space) and with CRLF line endings, except one:
//!
//! - GFM example 614, `<foo\+@bar.example.com>`: the escaped `+` is what keeps the
//!   e-mail address from being an extended autolink. The parser resolves the escape, and
//!   the printer has no rule for text that would become a bare autolink, so the address
//!   becomes a link.
//!
//! Beyond the spec suites (found by the wrapping proptest, in every mode):
//!
//! - Whitespace kept (as `&#32;`) right after a bare URL is taken into the URL; no
//!   markup can end a bare URL there.
//! - A paragraph right after a table in a tight list item becomes rows of the table,
//!   since the printed table has pipes.
//! - A paragraph inside raw HTML `<pre>` is wrapped like any other.
//!
//! Beyond those (found by the generated documents proptest; `tests/fmt.rs` skips them):
//!
//! - With strikethrough on, comrak opens or closes emphasis next to a `~` where the
//!   `~` would make that impossible (`*a~*a`); the printer escapes the `~`, and the
//!   emphasis is gone.
//! - An HTML block indented by one to three spaces right after a list keeps its
//!   indentation, which puts it into the last item once the list is printed with less.
//! - Whether comrak reads a list as loose can hang on a reference definition at the end
//!   of an item; for a list without paragraphs in its items, where it makes no
//!   difference to the HTML, a second run may then change the blank lines once more.
//!
//! The real-world READMEs in `tests/corpus/` keep their HTML and structure, in every
//! wrap mode, and format idempotently.
//!
//! Flavor syntax keeps its structure (the JSON projection without positions; fixtures
//! and a proptest in `tests/fmt.rs`), except:
//!
//! - A line of a `%%` comment or of math in a container that continued its paragraph
//!   lazily may start a block once it is indented, and its literal gains the indent.
//! - An admonition header in a definition term (the passes put it there) splits the
//!   term into blocks, which no term can hold. In a tight list item, an admonition
//!   header right before a definition list, a table or an empty item is read as a term,
//!   a table header or a setext heading.
//!
//! Reference definitions are recovered with single-line labels only; a definition with
//! a multi-line label, or one the parser reports at a drifted position (e.g. at the
//! start of a block quote's paragraph), moves to the end of the document. The title of
//! a definition no link uses is kept only when it is on the destination's line.

mod block;
mod code;
mod frontmatter;
mod inline;
mod refs;
mod table;
mod wrap;

use std::cell::RefCell;
use std::path::Path;

use crate::Document;
use crate::ast::{Ast, NodeId, Span};
use crate::config::FmtConfig;
use crate::diag::{Diagnostic, Severity};
use crate::flavor::{PrinterHooks, repeated_prefix};
use crate::parse::ParseOptions;

pub use self::code::{CodeFormatter, CommandFormatter};
use self::refs::Definitions;

/// The [`Diagnostic`] codes of [`format_with`]: plain words, like the parser's and
/// validate's.
pub mod codes {
    /// A fenced code block whose formatter failed; the code is kept as written.
    pub const CODE_FORMATTER: &str = "code-formatter";
    /// Frontmatter that `fmt.frontmatter: normalize` could not normalize without losing
    /// something, or that does not parse; it is kept as written.
    pub const FRONTMATTER_NORMALIZE: &str = "frontmatter-normalize";
}

/// The result of [`format_with`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formatted {
    /// The formatted document.
    pub text: String,
    /// Warnings about the parts left as written, in document order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Prints `document` as canonical Markdown following `config`, with fenced code formatted
/// by the commands of `config.code_formatters` ([`CommandFormatter`]). Use
/// [`format_with`] for the warnings about what was left as written.
///
/// The result parses (with the document's flavor) into a tree that renders to the same
/// HTML as the document; see the [module docs](self) for the conventions.
pub fn format(document: &Document, config: &FmtConfig) -> String {
    format_with(document, config, &CommandFormatter::from_config(config)).text
}

/// Like [`format()`], with fenced code formatted by `code`; code it fails on, and
/// frontmatter that cannot be normalized, are kept as written with a warning (codes in
/// [`codes`]).
pub fn format_with(document: &Document, config: &FmtConfig, code: &dyn CodeFormatter) -> Formatted {
    let printer = Printer {
        ast: document.ast(),
        src: document.text(),
        syntax: document.flavor().parse_options(),
        hooks: document.flavor().printer_hooks(),
        config,
        definitions: Definitions::new(document),
        code,
        file: document.name(),
        diagnostics: RefCell::new(Vec::new()),
    };
    let text = printer.print();
    let mut diagnostics = printer.diagnostics.into_inner();
    diagnostics.sort_by_key(|diagnostic| diagnostic.range.start);
    Formatted { text, diagnostics }
}

/// Shared state of one [`format_with`] call; the block printer lives in `block`, tables in
/// `table`, the inline printer and escaping in `inline`.
struct Printer<'a> {
    ast: &'a Ast,
    /// The text the tree's spans index, for verbatim parts.
    src: &'a str,
    /// The extensions in effect, which decide what needs escaping.
    syntax: ParseOptions,
    /// The flavor's rules on top of the generic ones.
    hooks: Option<&'a dyn PrinterHooks>,
    config: &'a FmtConfig,
    definitions: Definitions,
    /// Formats fenced code.
    code: &'a dyn CodeFormatter,
    /// The document's name, for diagnostics.
    file: &'a Path,
    /// Warnings about parts left as written.
    diagnostics: RefCell<Vec<Diagnostic>>,
}

impl Printer<'_> {
    /// Records a warning with `code` about `range`.
    fn warn(&self, code: &str, range: Span, message: String) {
        self.diagnostics.borrow_mut().push(Diagnostic::new(
            code,
            Severity::Warning,
            self.file,
            range,
            message,
        ));
    }

    /// The whole document. Lines never end with whitespace that is not content (code,
    /// raw HTML): prefixes of empty lines are trimmed where they are added.
    fn print(&self) -> String {
        let mut lines = self.document();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        let mut out = String::new();
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        out
    }

    /// The offset of the character to escape when a line starts with `line`: block
    /// markers, then the flavor's.
    fn line_start_marker(&self, line: &str) -> Option<usize> {
        inline::line_start_marker(line, self.syntax)
            .or_else(|| self.hooks.and_then(|hooks| hooks.line_start_marker(line)))
    }

    /// Whether the flavor wants `id` printed from its source.
    fn is_verbatim(&self, id: NodeId) -> bool {
        self.hooks
            .is_some_and(|hooks| hooks.verbatim(self.ast.kind(id)))
    }

    /// `text`, which is taken from the source of a block starting at `block` (or of the
    /// inlines of one), without the container prefixes of its continuation lines: the
    /// prefix of the line the block starts on (`> `, list indentation), as far as each
    /// line repeats it. The printer writes its own prefixes in their place.
    fn unprefixed(&self, text: &str, block: usize) -> String {
        let line_start = self.src[..block].rfind('\n').map_or(0, |nl| nl + 1);
        let prefix = &self.src[line_start..block];
        let mut out = String::with_capacity(text.len());
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                out.push('\n');
                out.push_str(&line[repeated_prefix(prefix, line)..]);
            } else {
                out.push_str(line);
            }
        }
        out.replace("\r\n", "\n")
    }
}
