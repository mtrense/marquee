//! Link reference definitions for the printer: recovered from the source (the parser
//! drops them), normalized from what their links resolve to, and handed out in source
//! order as the block printer passes their positions.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use super::inline::{destination, title_string};
use crate::Document;
use crate::ast::{LinkForm, NodeKind};
use crate::validate::reference_definitions;

/// The definitions of one document, in printing order.
#[derive(Debug)]
pub(super) struct Definitions {
    /// Recovered definitions: their source offset and printed line, in source order.
    placed: Vec<(usize, String)>,
    /// How many of `placed` have been handed out.
    next: Cell<usize>,
    /// Definitions for labels that links use but no recovered definition covers.
    missing: RefCell<Vec<String>>,
}

impl Definitions {
    pub(super) fn new(document: &Document) -> Self {
        let ast = document.ast();
        // What each label resolves to, from the links that use it, in first-use order.
        let mut used: Vec<(String, String, String, Option<String>)> = Vec::new();
        let mut index = HashMap::new();
        for id in ast.descendants(ast.root()) {
            if let NodeKind::Link(link) | NodeKind::Image(link) = ast.kind(id)
                && let LinkForm::Reference { label, .. } = &link.form
            {
                let key = normalize_label(label);
                if !index.contains_key(&key) {
                    index.insert(key.clone(), used.len());
                    used.push((key, label.clone(), link.url.clone(), link.title.clone()));
                }
            }
        }
        let mut defined = vec![false; used.len()];
        let text = document.text();
        let placed = reference_definitions(document)
            .into_iter()
            .map(|def| {
                let line = match index.get(&normalize_label(&def.label)) {
                    Some(&i) if !defined[i] => {
                        defined[i] = true;
                        let (_, _, url, title) = &used[i];
                        definition(&def.label, url, title.as_deref())
                    }
                    // Unused (or a duplicate, which the first one shadows): as written,
                    // up to the end of the line its destination ends on.
                    _ => {
                        let rest = &text[def.span.end..];
                        let end = def.span.end + rest.find(['\n', '\r']).unwrap_or(rest.len());
                        let raw = &text[def.span.start..end];
                        collapse_whitespace(raw)
                    }
                };
                (def.span.start, line)
            })
            .collect();
        let missing = used
            .iter()
            .zip(defined)
            .filter(|&(_, defined)| !defined)
            .map(|((_, label, url, title), _)| definition(label, url, title.as_deref()))
            .collect();
        Self {
            placed,
            next: Cell::new(0),
            missing: RefCell::new(missing),
        }
    }

    /// The lines of the definitions not handed out yet that start before `offset`.
    pub(super) fn take_before(&self, offset: usize) -> Vec<String> {
        let from = self.next.get();
        let count = self.placed[from..]
            .iter()
            .take_while(|(start, _)| *start < offset)
            .count();
        self.next.set(from + count);
        self.placed[from..from + count]
            .iter()
            .map(|(_, line)| line.clone())
            .collect()
    }

    /// Every definition not handed out yet.
    pub(super) fn rest(&self) -> Vec<String> {
        let mut lines = self.take_before(usize::MAX);
        lines.append(&mut self.missing.borrow_mut());
        lines
    }
}

/// `[label]: destination "title"`.
pub(super) fn definition(label: &str, url: &str, title: Option<&str>) -> String {
    let destination = match destination(url) {
        empty if empty.is_empty() => "<>".to_owned(),
        destination => destination,
    };
    let label = collapse_whitespace(label);
    match title {
        Some(title) => format!("[{label}]: {destination} {}", title_string(title)),
        None => format!("[{label}]: {destination}"),
    }
}

/// A label as `CommonMark` matches it: whitespace runs collapsed, case folded (Unicode
/// lowercase, plus `ß`/`ẞ` folding to `ss`).
fn normalize_label(label: &str) -> String {
    collapse_whitespace(label).to_lowercase().replace('ß', "ss")
}

/// `text` with each whitespace run as one space and none at the ends; a label means the
/// same either way, and stays on one line.
pub(super) fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
