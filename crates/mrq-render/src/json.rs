//! The json backend: the document's [JSON projection](mrq_core::ast::json)
//! (`{"mrq_ast": 1, …}`, mdast-like, with source positions), pretty-printed with a
//! trailing newline like the other `-o json` outputs.

use mrq_core::Document;

/// `doc` as its pretty-printed JSON projection.
pub fn render(doc: &Document) -> String {
    doc.to_json_ast().to_json_string()
}
