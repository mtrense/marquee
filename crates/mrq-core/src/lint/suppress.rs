//! Inline configuration comments: `<!-- markdownlint-disable … -->`, `<!-- mrq-disable … -->`
//! and their siblings, resolved per line the way markdownlint does.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

use super::rule::RuleMeta;
use crate::Document;
use crate::ast::{CommentSyntax, NodeKind};
use crate::source::PositionEncoding;

/// `<!--`, the prefix and an action followed by whitespace or the end of the comment;
/// longer actions first so `disable-line` is not read as `disable`.
static COMMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)<!--\s*(?:markdownlint|mrq)-(disable-next-line|disable-line|disable-file|enable-file|configure-file|disable|enable|capture|restore)(?:\s|-->)",
    )
    .expect("valid regex")
});

/// What an inline configuration comment does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Disable,
    Enable,
    DisableLine,
    DisableNextLine,
    DisableFile,
    EnableFile,
    Capture,
    Restore,
    ConfigureFile,
}

/// One inline configuration comment.
#[derive(Debug, Clone)]
pub(super) struct Directive<'a> {
    pub action: Action,
    /// The text between the action and `-->`.
    pub parameter: &'a str,
    /// The 0-based line of the comment's `<!--`.
    pub line: usize,
}

/// Every inline configuration comment of `doc` in source order. Only HTML (block,
/// inline and comment nodes) is searched, so text in code blocks, code spans and
/// frontmatter never counts.
pub(super) fn directives(doc: &Document) -> Vec<Directive<'_>> {
    let ast = doc.ast();
    let text = doc.text();
    let mut found = Vec::new();
    for id in ast.descendants(ast.root()) {
        let html = match ast.kind(id) {
            NodeKind::Comment(comment) => comment.syntax == CommentSyntax::Html,
            NodeKind::HtmlBlock(_) | NodeKind::HtmlInline(_) => true,
            _ => false,
        };
        if !html {
            continue;
        }
        let span = ast.span(id);
        let html = &text[span.start..span.end];
        for captures in COMMENT.captures_iter(html) {
            let (whole, action) = (captures.get(0).unwrap(), captures.get(1).unwrap());
            let rest = &html[action.end()..];
            let Some(end) = rest.find("-->") else {
                continue;
            };
            let line = doc
                .source()
                .offset_to_position(span.start + whole.start(), PositionEncoding::Utf8)
                .map_or(0, |position| position.line);
            found.push(Directive {
                action: match action.as_str().to_ascii_lowercase().as_str() {
                    "disable" => Action::Disable,
                    "enable" => Action::Enable,
                    "disable-line" => Action::DisableLine,
                    "disable-next-line" => Action::DisableNextLine,
                    "disable-file" => Action::DisableFile,
                    "enable-file" => Action::EnableFile,
                    "capture" => Action::Capture,
                    "restore" => Action::Restore,
                    _ => Action::ConfigureFile,
                },
                parameter: &rest[..end],
                line,
            });
        }
    }
    found.sort_by_key(|directive| directive.line);
    found
}

/// The rules (by index into the linter's rule list) switched on for each line.
pub(super) struct LineStates {
    lines: Vec<BTreeSet<usize>>,
}

impl LineStates {
    /// Resolves `directives` over `line_count` lines like markdownlint: file-wide comments
    /// first (on top of `initial`, the configured rules), then `enable`/`disable`/
    /// `capture`/`restore` line by line (a comment counts from its own line on), then
    /// `disable-line`/`disable-next-line` on single lines. `capture` keeps one state, and
    /// `restore` before any `capture` goes back to `initial`.
    pub(super) fn resolve(
        rules: &[RuleMeta],
        initial: &BTreeSet<usize>,
        directives: &[Directive<'_>],
        line_count: usize,
    ) -> Self {
        let mut state = initial.clone();
        let mut captured = initial.clone();
        for directive in directives {
            match directive.action {
                Action::DisableFile => switch(&mut state, rules, directive.parameter, false),
                Action::EnableFile => switch(&mut state, rules, directive.parameter, true),
                _ => {}
            }
        }

        let mut lines = Vec::with_capacity(line_count);
        let mut pending = directives.iter().peekable();
        for line in 0..line_count {
            while let Some(directive) = pending.next_if(|directive| directive.line <= line) {
                match directive.action {
                    Action::Disable => switch(&mut state, rules, directive.parameter, false),
                    Action::Enable => switch(&mut state, rules, directive.parameter, true),
                    Action::Capture => captured.clone_from(&state),
                    Action::Restore => state.clone_from(&captured),
                    _ => {}
                }
            }
            lines.push(state.clone());
        }

        for directive in directives {
            let target = match directive.action {
                Action::DisableLine => directive.line,
                Action::DisableNextLine => directive.line + 1,
                _ => continue,
            };
            if let Some(line) = lines.get_mut(target) {
                switch(line, rules, directive.parameter, false);
            }
        }
        Self { lines }
    }

    /// Whether rule `index` is on anywhere.
    pub(super) fn any(&self, index: usize) -> bool {
        self.lines.iter().any(|line| line.contains(&index))
    }

    /// Whether rule `index` is on at `line`; past the end, the last line's state counts.
    pub(super) fn is_on(&self, index: usize, line: usize) -> bool {
        self.lines
            .get(line)
            .or_else(|| self.lines.last())
            .is_some_and(|state| state.contains(&index))
    }
}

/// Switches the rules named in `parameter` (ids, aliases or tags, whitespace-separated,
/// ignoring ASCII case; unknown names are ignored), or every rule when it is blank.
fn switch(state: &mut BTreeSet<usize>, rules: &[RuleMeta], parameter: &str, on: bool) {
    let mut set = |index| {
        if on {
            state.insert(index);
        } else {
            state.remove(&index);
        }
    };
    let names: Vec<&str> = parameter.split_whitespace().collect();
    for (index, meta) in rules.iter().enumerate() {
        if names.is_empty()
            || names
                .iter()
                .any(|name| meta.is_named(name) || meta.has_tag(name))
        {
            set(index);
        }
    }
}
