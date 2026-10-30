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
    /// The fragment must be contiguous source. A body that first needs transforming
    /// (e.g. dedenting) can be parsed with [`Flavor::parser`] instead, mapping the spans
    /// back by hand.
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
