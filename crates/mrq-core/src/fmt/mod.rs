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
//!   a tight list. Trailing whitespace goes (a hard break is printed as `\`), and the
//!   output ends with exactly one newline; an empty document prints as nothing.
//! - Emphasis, strong emphasis, bullet markers and code fences use `fmt.emphasis`,
//!   `fmt.strong`, `fmt.list_marker` and `fmt.code_fence`. Where the configured marker
//!   would change the meaning, another one is used: `*` for intraword emphasis, the
//!   other emphasis character where delimiters would touch (`**_a_**`), another bullet
//!   (or `)` instead of `.`) for a list right after a list of the same kind, a tilde
//!   fence for an info string with a backtick, and a longer fence than any fence-like
//!   line of the code.
//! - Indented code blocks become fenced. Ordered lists are numbered consecutively from
//!   their start number (`fmt.ordered_list`), and item content is indented by the width
//!   of its marker.
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
//!   longer equals its label becomes a full reference.
//! - Frontmatter, raw HTML and `mrq:` directives are printed verbatim.
//!
//! # Escaping
//!
//! Text is escaped only where a character could start markup: always `\`, `` ` ``,
//! `[` and `]`; `*` and `_` where they could open emphasis, or close the emphasis they
//! are in (so `snake_case`, `2 * 3` and a trailing `x_` stay as they are); `<` when a
//! `>` may follow; `&` before a character reference; and the syntax of the enabled
//! extensions (`~`, `$`, `^`, `==`, `|` in table cells). At the start of a line block
//! markers are escaped too (`#`, `>`, `-`, `+`, `*`, `=`, `1.`); whitespace that the
//! parser would strip (at the start or end of a line or of emphasis) is written as a
//! character reference such as `&#32;`.
//!
//! # Scope
//!
//! This is the printer core and the table printer. List numbering modes and indentation
//! rules (M6.3), wrapping (M6.4), flavor-specific printing (M6.5) and code formatting
//! (M6.6) build on it; until then `fmt.wrap` is treated as `preserve`, and flavor syntax
//! is printed with simple generic rules (mkdocs heading attributes as written).
//!
//! # Known limits
//!
//! Every `CommonMark` and GFM spec example keeps its HTML (`tests/fmt.rs`), with every
//! marker setting and with CRLF line endings, except one:
//!
//! - GFM example 614, `<foo\+@bar.example.com>`: the escaped `+` is what keeps the
//!   e-mail address from being an extended autolink. The parser resolves the escape, and
//!   the printer has no rule for text that would become a bare autolink, so the address
//!   becomes a link.
//!
//! Reference definitions are recovered with single-line labels only; a definition with
//! a multi-line label, or one the parser reports at a drifted position (e.g. at the
//! start of a block quote's paragraph), moves to the end of the document. The title of
//! a definition no link uses is kept only when it is on the destination's line.

mod block;
mod inline;
mod refs;
mod table;

use crate::Document;
use crate::ast::Ast;
use crate::config::FmtConfig;
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
}
