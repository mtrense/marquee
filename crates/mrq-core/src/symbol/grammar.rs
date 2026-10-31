//! [`TreeSitterLocator`]: symbols found with tree-sitter grammars and a query per language.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;
use std::sync::OnceLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor};

use super::{Symbol, SymbolKind, SymbolLocator};
use crate::source::ByteRange;

/// A language [`TreeSitterLocator`] knows.
struct Grammar {
    /// The language's name, the fence's info string if the include has no `lang`.
    name: &'static str,
    /// Other `lang` values naming it.
    aliases: &'static [&'static str],
    /// File extensions (lowercase) of code in it.
    extensions: &'static [&'static str],
    language: fn() -> Language,
    /// The `symbols.scm` queries, joined.
    queries: &'static [&'static str],
    /// What joins the parts of qualified names.
    separator: &'static str,
}

const JAVASCRIPT: &str = include_str!("../../queries/javascript/symbols.scm");
const TYPESCRIPT: &str = include_str!("../../queries/typescript/symbols.scm");

/// The languages, by name.
const GRAMMARS: &[Grammar] = &[
    Grammar {
        name: "go",
        aliases: &["golang"],
        extensions: &["go"],
        language: || tree_sitter_go::LANGUAGE.into(),
        queries: &[include_str!("../../queries/go/symbols.scm")],
        separator: ".",
    },
    Grammar {
        name: "javascript",
        aliases: &["js", "jsx", "mjs", "cjs"],
        extensions: &["js", "jsx", "mjs", "cjs"],
        language: || tree_sitter_javascript::LANGUAGE.into(),
        queries: &[JAVASCRIPT],
        separator: ".",
    },
    Grammar {
        name: "python",
        aliases: &["py", "python3"],
        extensions: &["py", "pyi"],
        language: || tree_sitter_python::LANGUAGE.into(),
        queries: &[include_str!("../../queries/python/symbols.scm")],
        separator: ".",
    },
    Grammar {
        name: "rust",
        aliases: &["rs"],
        extensions: &["rs"],
        language: || tree_sitter_rust::LANGUAGE.into(),
        queries: &[include_str!("../../queries/rust/symbols.scm")],
        separator: "::",
    },
    Grammar {
        name: "tsx",
        aliases: &[],
        extensions: &["tsx"],
        language: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        queries: &[JAVASCRIPT, TYPESCRIPT],
        separator: ".",
    },
    Grammar {
        name: "typescript",
        aliases: &["ts", "mts", "cts"],
        extensions: &["ts", "mts", "cts"],
        language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        queries: &[JAVASCRIPT, TYPESCRIPT],
        separator: ".",
    },
];

/// Finds symbols with tree-sitter in Go, JavaScript (with JSX), Python, Rust, TSX and
/// TypeScript.
///
/// The language is the first word of `lang` if it names one (`rust` or `rs`, `python`
/// or `py`, `javascript` or `js`, `typescript` or `ts`, `tsx`, `go` or `golang`), else
/// the file's extension. Each language has a query, `queries/<lang>/symbols.scm`, whose
/// captures say what the symbols are:
///
/// - `@definition.<kind>` captures a definition, the [kind](SymbolKind::name) of
///   symbol it is (`@definition.function`), and `@name` its name. Where several
///   patterns capture the same node, the first one counts.
/// - `@parent` (with a definition) names what qualifies it, for definitions not nested
///   in it, like the type of a Rust `impl` block or a Go method's receiver. Definitions
///   nested in other definitions are qualified by them too.
/// - `@doc` captures doc comments, attributes and decorators: those right before a
///   definition (without a blank line between) or at its start belong to it.
/// - `@wrapper` captures nodes that belong to the one definition in them, like
///   JavaScript's `export` statement.
///
/// Captures starting with `_` are left for predicates. Adding a language takes its
/// grammar crate, a query file, and an entry in the list of grammars.
pub struct TreeSitterLocator {
    /// The compiled query of each of [`GRAMMARS`], once needed.
    compiled: Vec<OnceLock<Compiled>>,
}

impl fmt::Debug for TreeSitterLocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeSitterLocator")
            .field("languages", &self.languages())
            .finish()
    }
}

impl Default for TreeSitterLocator {
    fn default() -> Self {
        Self {
            compiled: GRAMMARS.iter().map(|_| OnceLock::new()).collect(),
        }
    }
}

impl TreeSitterLocator {
    /// A locator for every language it knows.
    pub fn new() -> Self {
        Self::default()
    }

    /// The grammar named `name` and its compiled query.
    fn grammar(&self, name: &str) -> Option<(&'static Grammar, &Compiled)> {
        let at = GRAMMARS.iter().position(|grammar| grammar.name == name)?;
        let grammar = &GRAMMARS[at];
        Some((
            grammar,
            self.compiled[at].get_or_init(|| Compiled::new(grammar)),
        ))
    }
}

impl SymbolLocator for TreeSitterLocator {
    fn language(&self, path: &Path, lang: Option<&str>) -> Option<String> {
        let word = lang
            .and_then(|lang| lang.split([' ', '\t', ',', '{']).next())
            .map(str::to_lowercase);
        let named = word.and_then(|word| {
            GRAMMARS
                .iter()
                .find(|grammar| grammar.name == word || grammar.aliases.contains(&&*word))
        });
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_lowercase);
        named
            .or_else(|| {
                let extension = extension?;
                GRAMMARS
                    .iter()
                    .find(|grammar| grammar.extensions.contains(&&*extension))
            })
            .map(|grammar| grammar.name.to_owned())
    }

    fn languages(&self) -> Vec<String> {
        GRAMMARS
            .iter()
            .map(|grammar| grammar.name.to_owned())
            .collect()
    }

    fn symbols(&self, language: &str, code: &str) -> Vec<Symbol> {
        let Some((grammar, compiled)) = self.grammar(language) else {
            return Vec::new();
        };
        let mut parser = Parser::new();
        parser
            .set_language(&compiled.language)
            .expect("the grammars are compatible with tree-sitter");
        let Some(tree) = parser.parse(code, None) else {
            return Vec::new();
        };
        compiled.symbols(grammar, tree.root_node(), code)
    }
}

/// What a capture of a `symbols.scm` query stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Definition(SymbolKind),
    Name,
    Parent,
    Doc,
    Wrapper,
    /// A capture for predicates (`@_name`).
    Other,
}

/// A grammar's language and query.
struct Compiled {
    language: Language,
    query: Query,
    /// The role of each capture, by index.
    roles: Vec<Role>,
}

impl Compiled {
    /// The language and query of `grammar`.
    ///
    /// # Panics
    ///
    /// If the query is invalid, or has a capture with no role.
    fn new(grammar: &Grammar) -> Self {
        let language = (grammar.language)();
        let source = grammar.queries.join("\n");
        let query = Query::new(&language, &source)
            .unwrap_or_else(|err| panic!("invalid {} symbols query: {err}", grammar.name));
        let roles = query
            .capture_names()
            .iter()
            .map(|&capture| match capture {
                "name" => Role::Name,
                "parent" => Role::Parent,
                "doc" => Role::Doc,
                "wrapper" => Role::Wrapper,
                other if other.starts_with('_') => Role::Other,
                other => other
                    .strip_prefix("definition.")
                    .and_then(SymbolKind::from_name)
                    .map_or_else(
                        || {
                            panic!(
                                "unknown capture @{other} in the {} symbols query",
                                grammar.name
                            )
                        },
                        Role::Definition,
                    ),
            })
            .collect();
        Self {
            language,
            query,
            roles,
        }
    }

    /// The symbols under `root`, a tree of `code` in `grammar`, in source order.
    fn symbols(&self, grammar: &Grammar, root: Node<'_>, code: &str) -> Vec<Symbol> {
        let text = |node: Node<'_>| &code[node.byte_range()];
        let mut docs = HashSet::new();
        let mut wrappers = HashSet::new();
        // By node id: the definition with the lowest pattern index.
        let mut found: HashMap<usize, (usize, Definition<'_>)> = HashMap::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, root, code.as_bytes());
        while let Some(found_match) = matches.next() {
            let mut definition = None;
            let mut name = None;
            let mut parent = None;
            for capture in found_match.captures() {
                match self.roles[capture.index as usize] {
                    Role::Definition(kind) => definition = Some((kind, capture.node)),
                    Role::Name => name = Some(text(capture.node)),
                    Role::Parent => parent = Some(text(capture.node)),
                    Role::Doc => {
                        docs.insert(capture.node.id());
                    }
                    Role::Wrapper => {
                        wrappers.insert(capture.node.id());
                    }
                    Role::Other => {}
                }
            }
            let (Some((kind, node)), Some(name)) = (definition, name) else {
                continue;
            };
            let pattern = found_match.pattern_index;
            let earlier = found.get(&node.id()).is_some_and(|(at, _)| *at < pattern);
            if !earlier {
                let definition = Definition {
                    kind,
                    node,
                    name,
                    parent,
                };
                found.insert(node.id(), (pattern, definition));
            }
        }

        let mut definitions: Vec<Definition<'_>> = found
            .into_values()
            .map(|(_, definition)| definition)
            .collect();
        for definition in &mut definitions {
            definition.node = unwrap(definition.node, &wrappers, &docs);
        }
        // Outer definitions first, so their qualified names are known for inner ones.
        definitions.sort_by_key(|definition| {
            (
                definition.node.start_byte(),
                std::cmp::Reverse(definition.node.end_byte()),
            )
        });
        let mut qualified: HashMap<usize, String> = HashMap::new();
        let mut symbols = Vec::with_capacity(definitions.len());
        for definition in definitions {
            let mut parts = Vec::new();
            let mut ancestor = definition.node.parent();
            while let Some(node) = ancestor {
                if let Some(name) = qualified.get(&node.id()) {
                    parts.push(name.as_str());
                    break;
                }
                ancestor = node.parent();
            }
            parts.extend(definition.parent);
            parts.push(definition.name);
            let name = parts.join(grammar.separator);
            qualified.insert(definition.node.id(), name.clone());
            let node = definition.node;
            symbols.push(Symbol {
                kind: definition.kind,
                name,
                range: ByteRange::new(start_after_docs(node, &docs), end(node)),
                docs_start: docs_start(node, &docs, code),
            });
        }
        symbols
    }
}

/// A definition a query found.
struct Definition<'t> {
    kind: SymbolKind,
    node: Node<'t>,
    name: &'t str,
    parent: Option<&'t str>,
}

/// The outermost [`@wrapper`](TreeSitterLocator) around `node` whose only named child,
/// other than [`@doc`](TreeSitterLocator) nodes, is `node` or such a wrapper; `node`
/// itself if there is none.
fn unwrap<'t>(node: Node<'t>, wrappers: &HashSet<usize>, docs: &HashSet<usize>) -> Node<'t> {
    let mut node = node;
    while let Some(parent) = node.parent()
        && wrappers.contains(&parent.id())
    {
        let mut walker = parent.walk();
        let only = parent
            .named_children(&mut walker)
            .filter(|child| !docs.contains(&child.id()))
            .all(|child| child.id() == node.id());
        if !only {
            break;
        }
        node = parent;
    }
    node
}

/// Where `node` starts after the [`@doc`](TreeSitterLocator) nodes at its start.
fn start_after_docs(node: Node<'_>, docs: &HashSet<usize>) -> usize {
    let mut walker = node.walk();
    node.children(&mut walker)
        .find(|child| !docs.contains(&child.id()))
        .map_or(node.start_byte(), |child| child.start_byte())
}

/// Where `node` ends, with the `;` right after it (that of a TypeScript abstract method,
/// say), if it is not a part of it.
fn end(node: Node<'_>) -> usize {
    match node.next_sibling() {
        Some(next) if next.kind() == ";" && !next.is_named() => next.end_byte(),
        _ => node.end_byte(),
    }
}

/// Where `node`, in `code`, starts with the [`@doc`](TreeSitterLocator) nodes right before
/// it: each one ends on the line before the next, or on its line.
fn docs_start(node: Node<'_>, docs: &HashSet<usize>, code: &str) -> usize {
    let mut start = node.start_byte();
    let mut previous = node.prev_sibling();
    while let Some(sibling) = previous
        && docs.contains(&sibling.id())
    {
        let gap = &code[sibling.end_byte()..start];
        let breaks = gap.matches('\n').count() + usize::from(text_ends_line(code, sibling));
        if !gap.trim().is_empty() || breaks > 1 {
            break;
        }
        start = sibling.start_byte();
        previous = sibling.prev_sibling();
    }
    start
}

/// Whether `node`'s text in `code` ends with its line's newline.
fn text_ends_line(code: &str, node: Node<'_>) -> bool {
    code[..node.end_byte()].ends_with('\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `kind name` of each symbol `TreeSitterLocator` finds in `code`, in `language`.
    fn symbols(language: &str, code: &str) -> Vec<String> {
        TreeSitterLocator::new()
            .symbols(language, code)
            .into_iter()
            .map(|symbol| format!("{} {}", symbol.kind, symbol.name))
            .collect()
    }

    #[test]
    fn every_query_compiles() {
        let locator = TreeSitterLocator::new();
        for grammar in GRAMMARS {
            assert!(locator.grammar(grammar.name).is_some(), "{}", grammar.name);
        }
    }

    #[test]
    fn the_language_comes_from_lang_then_the_extension() {
        let locator = TreeSitterLocator::new();
        let language = |path: &str, lang| locator.language(Path::new(path), lang);
        assert_eq!(language("a.rs", None).as_deref(), Some("rust"));
        assert_eq!(language("a.TSX", None).as_deref(), Some("tsx"));
        assert_eq!(
            language("a.txt", Some("Py title=x")).as_deref(),
            Some("python")
        );
        assert_eq!(language("a.go", Some("text")).as_deref(), Some("go"));
        assert_eq!(language("Makefile", Some("make")), None);
    }

    #[test]
    fn rust_symbols_are_qualified_by_modules_impls_and_traits() {
        let code = "mod a {\n    struct S;\n    impl<T> S<T> { fn new() {} }\n    trait T { fn f(); }\n}\nmacro_rules! m { () => {} }\nstatic X: u8 = 1;\n";
        assert_eq!(
            symbols("rust", code),
            [
                "module a",
                "struct a::S",
                "function a::S::new",
                "trait a::T",
                "function a::T::f",
                "macro m",
                "constant X",
            ]
        );
    }

    #[test]
    fn go_types_have_their_kind_and_methods_their_receiver() {
        let code = "package p\n\ntype (\n\tA struct{}\n\tB = int\n)\n\ntype C int\n\nfunc (s *Stack[T]) Push() {}\n\nconst K = 1\n";
        assert_eq!(
            symbols("go", code),
            [
                "struct A",
                "type B",
                "type C",
                "function Stack.Push",
                "constant K"
            ]
        );
    }

    #[test]
    fn typescript_namespaces_and_signatures_are_symbols() {
        let code =
            "namespace N {\n  export interface I { m(): void; }\n}\ndeclare function f(): void;\n";
        assert_eq!(
            symbols("typescript", code),
            ["module N", "interface N.I", "function N.I.m", "function f"]
        );
        assert_eq!(symbols("tsx", "const C = () => <div />;\n"), ["function C"]);
    }

    #[test]
    fn unknown_languages_have_no_symbols() {
        assert_eq!(
            symbols("cobol", "IDENTIFICATION DIVISION."),
            Vec::<String>::new()
        );
    }
}
