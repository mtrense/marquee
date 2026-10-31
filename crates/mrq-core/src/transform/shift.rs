//! Heading shift: [`shift_headings`].

use crate::ast::{NodeId, NodeKind};
use crate::diag::{Diagnostic, Label, Severity};
use crate::document::Document;
use crate::source::{ByteRange, PositionEncoding, TextEdit};

/// Code of the warning for a clamped heading that now shares its level with a heading
/// that had a different one.
pub const HEADING_CLAMP_CODE: &str = "heading-clamp";

/// What happens to a heading shifted past level 6.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Overflow {
    /// It stays a heading at level 6.
    #[default]
    Clamp,
    /// It becomes a paragraph of its text in `**strong**` emphasis.
    Bold,
}

/// How [`shift_headings`] shifts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShiftOptions {
    /// Levels to add: positive pushes headings down (`#` to `##`), negative pulls them up.
    pub by: i32,
    /// What happens to headings shifted past level 6.
    pub overflow: Overflow,
}

impl ShiftOptions {
    /// A shift by `by` levels that clamps at level 6.
    pub const fn by(by: i32) -> Self {
        Self {
            by,
            overflow: Overflow::Clamp,
        }
    }

    /// Sets the [`Overflow`] handling.
    #[must_use]
    pub const fn overflow(mut self, overflow: Overflow) -> Self {
        self.overflow = overflow;
        self
    }
}

/// The result of [`shift_headings`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shifted {
    /// Edits against the document's source; empty if no heading changes.
    pub edits: Vec<TextEdit>,
    /// [`HEADING_CLAMP_CODE`] warnings.
    pub diagnostics: Vec<Diagnostic>,
}

/// Shifts the headings that start in `range` by `options.by` levels.
///
/// Levels are clamped to 1–6. A clamped heading that ends up at the level of a heading
/// in `range` that had another level gets a [`HEADING_CLAMP_CODE`] warning, labelled at
/// that heading; clamping that keeps the relative structure is silent. With
/// [`Overflow::Bold`], a heading shifted past level 6 becomes a paragraph (see
/// [`Overflow`]), except an empty one or one ending in a backslash, which are clamped.
///
/// Edits touch only what changes: the `#` run of an ATX heading, the whole of a setext
/// heading (which becomes ATX, with line breaks joined by spaces, since ATX headings
/// are one line), the whole of a heading made bold (plus a blank line before or after
/// it where an adjacent line would otherwise join the paragraph). A heading whose level
/// does not change is left alone, setext included.
pub fn shift_headings(doc: &Document, range: ByteRange, options: ShiftOptions) -> Shifted {
    if options.by == 0 {
        return Shifted::default();
    }
    let ast = doc.ast();
    let plans: Vec<Planned> = ast
        .descendants(ast.root())
        .filter_map(|id| {
            let NodeKind::Heading(heading) = ast.kind(id) else {
                return None;
            };
            let start = ast.span(id).start;
            if start < range.start || start >= range.end {
                return None;
            }
            let target = i32::from(heading.level) + options.by;
            let to =
                if target > 6 && options.overflow == Overflow::Bold && bold_text(doc, id).is_some()
                {
                    Target::Bold
                } else {
                    Target::Level(u8::try_from(target.clamp(1, 6)).expect("1–6 fits in u8"))
                };
            Some(Planned {
                id,
                from: heading.level,
                setext: heading.setext,
                target,
                to,
            })
        })
        .collect();
    Shifted {
        edits: plans
            .iter()
            .filter_map(|plan| edit(doc, plan, &plans))
            .collect(),
        diagnostics: (0..plans.len())
            .filter_map(|index| clamp_warning(doc, index, &plans, options))
            .collect(),
    }
}

/// A heading in the range and what it becomes.
#[derive(Debug, Clone, Copy)]
struct Planned {
    id: NodeId,
    /// The level it has.
    from: u8,
    setext: bool,
    /// `from + by`, possibly out of 1–6.
    target: i32,
    to: Target,
}

/// What a [`Planned`] heading becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// A heading of this level.
    Level(u8),
    /// A `**strong**` paragraph.
    Bold,
}

/// The edit turning `plan`'s heading into its target, if it changes.
fn edit(doc: &Document, plan: &Planned, plans: &[Planned]) -> Option<TextEdit> {
    let text = doc.text();
    let span = doc.ast().span(plan.id);
    let level = match plan.to {
        Target::Level(level) if level == plan.from => return None,
        Target::Level(level) => level,
        Target::Bold => return Some(bold_edit(doc, plan, plans)),
    };
    let hashes = "#".repeat(usize::from(level));
    if !plan.setext {
        let open = text[span.start..].len() - text[span.start..].trim_start_matches('#').len();
        return Some(TextEdit {
            range: ByteRange::new(span.start, span.start + open),
            replacement: hashes,
        });
    }
    let (_, last) = lines(doc, span);
    let underline = line_range(doc, last);
    let content_end = span.start + text[span.start..underline.start].trim_end().len();
    let content = escape_closing_sequence(joined(doc, plan.id, span.start, content_end));
    Some(TextEdit {
        range: ByteRange::new(span.start, underline.end),
        replacement: format!("{hashes} {content}"),
    })
}

/// The edit turning `plan`'s heading into a bold paragraph, with a blank line (carrying
/// the `>` of enclosing quotes) before or after it where the line there would otherwise
/// continue the paragraph or underline it.
fn bold_edit(doc: &Document, plan: &Planned, plans: &[Planned]) -> TextEdit {
    let ast = doc.ast();
    let text = doc.text();
    let span = ast.span(plan.id);
    let (first, last) = lines(doc, span);
    let first_line = line_range(doc, first);
    let last_line = line_range(doc, last);
    let ending = if text[last_line.end..].starts_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let blank: String = text[first_line.start..span.start]
        .chars()
        .map(|c| if c == '>' { '>' } else { ' ' })
        .collect::<String>()
        .trim_end()
        .to_owned();
    let prev_is_bold = ast
        .prev_sibling(plan.id)
        .is_some_and(|prev| plans.iter().any(|p| p.id == prev && p.to == Target::Bold));
    let before = first > 0
        && !is_blank(doc, first - 1)
        && ast.prev_sibling(plan.id).is_some()
        && !prev_is_bold;
    let in_list_item = ast
        .parent(plan.id)
        .is_some_and(|parent| matches!(ast.kind(parent), NodeKind::ListItem(_)));
    let after = last + 1 < doc.source().line_count()
        && !is_blank(doc, last + 1)
        && (ast.next_sibling(plan.id).is_some() || !in_list_item);
    let mut replacement = String::new();
    let start = if before {
        replacement.push_str(&blank);
        replacement.push_str(ending);
        replacement.push_str(&text[first_line.start..span.start]);
        first_line.start
    } else {
        span.start
    };
    let content = bold_text(doc, plan.id).expect("planned bold only with bold text");
    replacement.push_str("**");
    replacement.push_str(&content);
    replacement.push_str("**");
    if after {
        replacement.push_str(ending);
        replacement.push_str(&blank);
    }
    TextEdit {
        range: ByteRange::new(start, last_line.end),
        replacement,
    }
}

/// The heading's inline content on one line, if it can be wrapped in `**…**`: not empty
/// and not ending in an escaping backslash. Heading attributes (`{#id}`) are left out.
fn bold_text(doc: &Document, heading: NodeId) -> Option<String> {
    let ast = doc.ast();
    let children = ast.children(heading);
    let (first, last) = (children.first()?, children.last()?);
    let content = joined(doc, heading, ast.span(*first).start, ast.span(*last).end);
    let backslashes = content.len() - content.trim_end_matches('\\').len();
    (!content.is_empty() && backslashes.is_multiple_of(2)).then_some(content)
}

/// `start..end` of the source, with each line break inside `heading` (and the
/// whitespace before it and the container prefixes after it) replaced by one space.
fn joined(doc: &Document, heading: NodeId, start: usize, end: usize) -> String {
    let ast = doc.ast();
    let text = doc.text();
    let is_break = |id| matches!(ast.kind(id), NodeKind::SoftBreak | NodeKind::LineBreak);
    let mut out = String::new();
    let mut pos = start;
    for id in ast.descendants(heading).filter(|&id| is_break(id)) {
        let span = ast.span(id);
        if span.start < pos || span.end > end {
            continue;
        }
        let resume = ast
            .descendants(heading)
            .filter(|&other| !is_break(other) && other != heading)
            .map(|other| ast.span(other).start)
            .filter(|&other| other >= span.end)
            .min()
            .unwrap_or(span.end)
            .min(end);
        out.push_str(text[pos..span.start].trim_end_matches([' ', '\t']));
        out.push(' ');
        pos = resume;
    }
    out.push_str(&text[pos..end]);
    out
}

/// `content` with a backslash before a trailing `#` run that an ATX heading would read
/// as its closing sequence.
fn escape_closing_sequence(mut content: String) -> String {
    let run = content.len() - content.trim_end_matches('#').len();
    let before = &content[..content.len() - run];
    if run > 0 && (before.is_empty() || before.ends_with([' ', '\t'])) {
        content.insert(content.len() - run, '\\');
    }
    content
}

/// The [`HEADING_CLAMP_CODE`] warning for `plans[index]`, if clamping put it at the
/// level of a heading that had another level (the closest one before it, else after).
fn clamp_warning(
    doc: &Document,
    index: usize,
    plans: &[Planned],
    options: ShiftOptions,
) -> Option<Diagnostic> {
    let plan = plans[index];
    let Target::Level(level) = plan.to else {
        return None;
    };
    if i32::from(level) == plan.target {
        return None;
    }
    let flattened = |other: &&Planned| other.to == plan.to && other.from != plan.from;
    let other = plans[..index]
        .iter()
        .rev()
        .find(flattened)
        .or_else(|| plans[index + 1..].iter().find(flattened))?;
    let ast = doc.ast();
    let help = if plan.target > 6 && options.overflow == Overflow::Clamp {
        "shift by less, or turn headings past level 6 into bold paragraphs"
    } else {
        "shift by less"
    };
    Some(
        Diagnostic::new(
            HEADING_CLAMP_CODE,
            Severity::Warning,
            doc.name(),
            ast.span(plan.id),
            format!(
                "heading shifted by {:+} to level {} is clamped to level {level}, the level of a heading that was level {}",
                options.by, plan.target, other.from
            ),
        )
        .with_label(Label::new(
            ast.span(other.id),
            format!("was level {}, now level {level}", other.from),
        ))
        .with_help(help),
    )
}

/// The first and last line `span` touches.
fn lines(doc: &Document, span: ByteRange) -> (usize, usize) {
    let line = |offset| {
        doc.source()
            .offset_to_position(offset, PositionEncoding::Utf8)
            .map_or(0, |position| position.line)
    };
    (line(span.start), line(span.end.max(span.start + 1) - 1))
}

/// Line `line` without its line ending.
fn line_range(doc: &Document, line: usize) -> ByteRange {
    doc.source()
        .line_range(line)
        .unwrap_or(ByteRange::empty(doc.text().len()))
}

/// Whether line `line` is empty but for whitespace and `>` markers.
fn is_blank(doc: &Document, line: usize) -> bool {
    let range = line_range(doc, line);
    doc.text()[range.start..range.end]
        .replace('>', "")
        .trim()
        .is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Severity;
    use crate::document::DocumentOptions;
    use crate::source::apply_edits;

    fn all(src: &str) -> ByteRange {
        ByteRange::new(0, src.len())
    }

    fn shift_doc(doc: &Document, options: ShiftOptions) -> (String, Shifted) {
        let src = doc.text();
        let shifted = shift_headings(doc, all(src), options);
        (apply_edits(src, &shifted.edits).unwrap(), shifted)
    }

    fn shift(src: &str, options: ShiftOptions) -> (String, Shifted) {
        shift_doc(&Document::parse(src), options)
    }

    fn shifted(src: &str, by: i32) -> String {
        shift(src, ShiftOptions::by(by)).0
    }

    fn bold(src: &str, by: i32) -> String {
        shift(src, ShiftOptions::by(by).overflow(Overflow::Bold)).0
    }

    #[test]
    fn a_zero_shift_changes_nothing() {
        let (out, shifted) = shift("Title\n===\n\n# A\n", ShiftOptions::by(0));
        assert_eq!(out, "Title\n===\n\n# A\n");
        assert_eq!(shifted, Shifted::default());
    }

    #[test]
    fn only_headings_starting_in_the_range_shift() {
        let src = "# A\n\n## B\n\n### C\n";
        let doc = Document::parse(src);
        let b = src.find("##").unwrap();
        let shifted = shift_headings(&doc, ByteRange::new(b, src.len() - 6), ShiftOptions::by(1));
        assert_eq!(
            apply_edits(src, &shifted.edits).unwrap(),
            "# A\n\n### B\n\n### C\n"
        );
    }

    #[test]
    fn atx_shifts_rewrite_only_the_opening_sequence() {
        let src = "## A ##\n";
        let shifted = shift(src, ShiftOptions::by(-1)).1;
        assert_eq!(
            shifted.edits,
            vec![TextEdit {
                range: ByteRange::new(0, 2),
                replacement: "#".to_owned(),
            }]
        );
    }

    #[test]
    fn clamping_without_flattening_is_silent() {
        let (out, shifted) = shift("###### A\n\n###### B\n", ShiftOptions::by(1));
        assert_eq!(out, "###### A\n\n###### B\n");
        assert_eq!(shifted, Shifted::default());
        let (out, shifted) = shift("# A\n\nText\n", ShiftOptions::by(-3));
        assert_eq!(out, "# A\n\nText\n");
        assert_eq!(shifted.diagnostics, vec![]);
    }

    #[test]
    fn flattening_warns_with_a_label_on_the_other_heading() {
        let src = "##### A\n\n###### B\n";
        let (_, shifted) = shift(src, ShiftOptions::by(1));
        let [diagnostic] = shifted.diagnostics.as_slice() else {
            panic!("expected one diagnostic: {:?}", shifted.diagnostics);
        };
        assert_eq!(diagnostic.code, HEADING_CLAMP_CODE);
        assert_eq!(diagnostic.severity, Severity::Warning);
        assert_eq!(diagnostic.range, ByteRange::new(9, 17));
        assert_eq!(diagnostic.labels.len(), 1);
        assert_eq!(diagnostic.labels[0].range, ByteRange::new(0, 7));
        assert_eq!(diagnostic.labels[0].message, "was level 5, now level 6");
        assert!(diagnostic.help.as_deref().unwrap().contains("bold"));
    }

    #[test]
    fn setext_headings_become_atx() {
        assert_eq!(shifted("Title\n=====\n\nText\n", 1), "## Title\n\nText\n");
        assert_eq!(shifted("Title\n---\n", -1), "# Title\n");
        assert_eq!(shifted("Title  \n---  \n", 2), "#### Title\n");
    }

    #[test]
    fn setext_headings_that_keep_their_level_stay_setext() {
        assert_eq!(shifted("Title\n===\n", -1), "Title\n===\n");
    }

    #[test]
    fn crlf_is_kept() {
        assert_eq!(
            shifted("A\r\nb\r\n===\r\n\r\nc\r\n", 1),
            "## A b\r\n\r\nc\r\n"
        );
        assert_eq!(bold("###### A\r\ntext\r\n", 1), "**A**\r\n\r\ntext\r\n");
    }

    #[test]
    fn mkdocs_attributes_stay_on_atx_and_go_with_bold() {
        let mkdocs = |src: &str| DocumentOptions::new().flavor("mkdocs").parse(src).unwrap();
        let doc = mkdocs("Title {#id}\n---\n");
        assert_eq!(shift_doc(&doc, ShiftOptions::by(1)).0, "### Title {#id}\n");
        let doc = mkdocs("###### Title {#id}\n");
        let options = ShiftOptions::by(1).overflow(Overflow::Bold);
        assert_eq!(shift_doc(&doc, options).0, "**Title**\n");
    }

    #[test]
    fn bold_converts_setext_and_closed_headings() {
        assert_eq!(bold("Title\n===\n", 6), "**Title**\n");
        assert_eq!(bold("##### A *b* #####\n", 2), "**A *b***\n");
    }

    #[test]
    fn bold_falls_back_to_clamping_where_it_cannot_apply() {
        // Empty: `****` would be a thematic break; a trailing backslash would escape `*`.
        let (out, shifted) = shift(
            "###### A\n\n#####\n\n##### B\\\n",
            ShiftOptions::by(2).overflow(Overflow::Bold),
        );
        assert_eq!(out, "**A**\n\n######\n\n###### B\\\n");
        assert_eq!(shifted.diagnostics, vec![]);
    }

    #[test]
    fn bold_keeps_tight_lists_tight() {
        assert_eq!(bold("- ###### A\n- b\n", 1), "- **A**\n- b\n");
    }

    #[test]
    fn bold_separates_itself_from_lazy_and_setext_lines() {
        assert_eq!(bold("> ###### A\nlazy\n", 1), "> **A**\n>\nlazy\n");
        assert_eq!(bold("###### A\n---\n", 1), "**A**\n\n---\n");
    }
}
