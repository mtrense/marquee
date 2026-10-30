//! Post-parse AST passes and the context they run in.

use std::path::Path;

use super::Flavor;
use crate::ast::{Ast, Span};
use crate::diag::Diagnostic;
use crate::parse::Parser as _;

/// A rewrite of the parsed tree that recognizes flavor syntax the parser does not know
/// (admonitions, heading attributes, tags, …).
///
/// Passes run in the order of [`Flavor::passes`], each over the whole tree. A pass
/// typically collects the node ids it cares about, re-scans their source
/// (`&cx.src()[span]`) and replaces subtrees through the structural edits on [`Ast`];
/// nodes it creates should span the source they stand for. Problems in the source are
/// reported as diagnostics ([`PassContext::report`]); a pass never fails.
pub trait Pass: Send + Sync {
    /// A short identifier, e.g. `admonitions`.
    fn name(&self) -> &'static str;

    /// Rewrites `ast`, the tree of `cx.src()`.
    fn run(&self, ast: &mut Ast, cx: &mut PassContext<'_>);
}

/// What a [`Pass`] can see and do besides editing the tree: the full source, the file
/// name for diagnostics, the flavor, and nested parsing.
#[derive(Debug)]
pub struct PassContext<'a> {
    src: &'a str,
    file: &'a Path,
    flavor: &'a dyn Flavor,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> PassContext<'a> {
    pub(crate) fn new(src: &'a str, file: &'a Path, flavor: &'a dyn Flavor) -> Self {
        Self {
            src,
            file,
            flavor,
            diagnostics: Vec::new(),
        }
    }

    /// The full source text (frontmatter included); every span indexes into it.
    pub const fn src(&self) -> &'a str {
        self.src
    }

    /// The file being parsed, for [`Diagnostic::file`].
    pub const fn file(&self) -> &'a Path {
        self.file
    }

    /// The flavor whose passes are running.
    pub const fn flavor(&self) -> &'a dyn Flavor {
        self.flavor
    }

    /// Records a diagnostic about the source.
    pub fn report(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Parses `src[range]` as a document of its own, with the flavor's parser and then all
    /// of its passes (so nested flavor syntax is recognized too). The returned tree's
    /// spans index into the full [`src`](Self::src); its root spans `range`. Import its
    /// children into the main tree with [`Ast::import`] (keeping the spans as they are).
    ///
    /// The fragment must be contiguous source; for a body that first needs transforming
    /// (e.g. dedenting) use [`parse_mapped`](Self::parse_mapped).
    pub fn parse_fragment(&mut self, range: Span) -> Ast {
        let nested = self
            .flavor
            .parser()
            .parse(&self.src[range.start..range.end]);
        let mut ast = Ast::new(range);
        let root = ast.root();
        for &child in nested.children(nested.root()) {
            let copy = ast.import(&nested, child, |span| {
                Span::new(span.start + range.start, span.end + range.start)
            });
            ast.append_child(root, copy)
                .expect("an imported node is detached");
        }
        self.run_passes(&mut ast);
        ast
    }

    /// Parses `text`, derived from the source but not a contiguous slice of it (e.g. a
    /// dedented admonition body), as a document of its own with the flavor's parser and
    /// all of its passes. The passes see `text` as their [`src`](Self::src); afterwards
    /// `map` translates every span of `text` into the full source: the returned tree's
    /// spans, and the ranges of the diagnostics the passes reported (fix replacements
    /// are kept as they are). Its root spans `map(0..text.len())`. Import its children
    /// with [`Ast::import`], keeping the spans.
    pub fn parse_mapped(&mut self, text: &str, map: impl Fn(Span) -> Span) -> Ast {
        let mut nested = self.flavor.parser().parse(text);
        let mut inner = PassContext::new(text, self.file, self.flavor);
        inner.run_passes(&mut nested);
        for mut diagnostic in inner.into_diagnostics() {
            diagnostic.range = map(diagnostic.range);
            for label in &mut diagnostic.labels {
                label.range = map(label.range);
            }
            for edit in diagnostic.fix.iter_mut().flat_map(|fix| &mut fix.edits) {
                edit.range = map(edit.range);
            }
            self.report(diagnostic);
        }
        let mut ast = Ast::new(map(nested.span(nested.root())));
        let root = ast.root();
        for &child in nested.children(nested.root()) {
            let copy = ast.import(&nested, child, &map);
            ast.append_child(root, copy)
                .expect("an imported node is detached");
        }
        ast
    }

    /// Runs every pass of the flavor over `ast`.
    pub(crate) fn run_passes(&mut self, ast: &mut Ast) {
        for pass in self.flavor.passes() {
            pass.run(ast, self);
        }
    }

    pub(crate) fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::NodeKind;
    use crate::diag::{Fix, Label, Severity};
    use crate::flavor::{GithubSlugger, Slugger};
    use crate::parse::ParseOptions;
    use crate::source::TextEdit;

    /// Reports every paragraph, with a label and a fix on it.
    struct Report;

    impl Pass for Report {
        fn name(&self) -> &'static str {
            "report"
        }

        fn run(&self, ast: &mut Ast, cx: &mut PassContext<'_>) {
            let spans: Vec<Span> = ast
                .descendants(ast.root())
                .filter(|&id| ast.kind(id) == &NodeKind::Paragraph)
                .map(|id| ast.span(id))
                .collect();
            for span in spans {
                let edit = TextEdit {
                    range: span,
                    replacement: String::new(),
                };
                cx.report(
                    Diagnostic::new("p", Severity::Info, cx.file(), span, "paragraph")
                        .with_label(Label::new(span, "here"))
                        .with_fix(Fix::new("drop", vec![edit])),
                );
            }
        }
    }

    #[derive(Debug)]
    struct Reporting;

    impl Flavor for Reporting {
        fn name(&self) -> &'static str {
            "reporting"
        }
        fn parse_options(&self) -> ParseOptions {
            ParseOptions::commonmark()
        }
        fn slugger(&self) -> Box<dyn Slugger> {
            Box::new(GithubSlugger::new())
        }
        fn passes(&self) -> &[&dyn Pass] {
            &[&Report]
        }
    }

    #[test]
    fn mapped_parses_map_spans_and_diagnostics() {
        let src = "> a\n> b\n";
        let mut cx = PassContext::new(src, Path::new("q.md"), &Reporting);
        // "a\nb\n" is the quote without its markers: offsets 0..2 sit at 2..4, the
        // rest at 6..; an end offset of 2 stays on the first line.
        let shift = |offset: usize, end: bool| {
            if offset < 2 || (end && offset == 2) {
                offset + 2
            } else {
                offset + 4
            }
        };
        let ast = cx.parse_mapped("a\nb\n", |s| {
            Span::new(shift(s.start, false), shift(s.end, true))
        });
        let para = ast.children(ast.root())[0];
        assert_eq!(ast.span(ast.root()), Span::new(2, 8));
        assert_eq!(ast.node(para).source(src), "a\n> b");
        let diagnostics = cx.into_diagnostics();
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.range, Span::new(2, 7));
        assert_eq!(diagnostic.labels[0].range, Span::new(2, 7));
        assert_eq!(
            diagnostic.fix.as_ref().unwrap().edits[0].range,
            Span::new(2, 7)
        );
    }
}
