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
//!   tight and loose ones loose, and task markers are `[ ]` and `[x]`.
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
//! - Frontmatter, raw HTML and `mrq:` directives are printed verbatim.
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
//! # Scope
//!
//! This is the printer core with tables, lists, wrapping and flavor syntax. Code
//! formatting (M6.6) builds on it.
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
mod inline;
mod refs;
mod table;
mod wrap;

use crate::Document;
use crate::ast::{Ast, NodeId};
use crate::config::FmtConfig;
use crate::flavor::{PrinterHooks, repeated_prefix};
use crate::parse::ParseOptions;

use self::refs::Definitions;

/// Prints `document` as canonical Markdown following `config`.
///
/// The result parses (with the document's flavor) into a tree that renders to the same
/// HTML as the document; see the [module docs](self) for the conventions.
pub fn format(document: &Document, config: &FmtConfig) -> String {
    let printer = Printer {
        ast: document.ast(),
        src: document.text(),
        syntax: document.flavor().parse_options(),
        hooks: document.flavor().printer_hooks(),
        config,
        definitions: Definitions::new(document),
    };
    printer.print()
}

/// Shared state of one [`format`] call; the block printer lives in `block`, tables in
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
}

impl Printer<'_> {
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
