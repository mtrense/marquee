//! Symbols in source code: [`SymbolLocator`].
//!
//! A symbol include (`<!-- mrq:include path="src/lib.rs" function="Point::new" -->`, see
//! [`Includer`](crate::transform::Includer)) asks a locator for the symbols a file defines
//! and picks one by kind and name. With the `tree-sitter` feature, `TreeSitterLocator`
//! finds them in Rust, Python, JavaScript, TypeScript and Go; other locators can add
//! languages or other ways of finding symbols.

#[cfg(feature = "tree-sitter")]
mod grammar;

use std::fmt;
use std::path::Path;

#[cfg(feature = "tree-sitter")]
pub use self::grammar::TreeSitterLocator;
use crate::source::ByteRange;

/// What kind of thing a [`Symbol`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SymbolKind {
    /// A function or method (`function="…"`).
    Function,
    /// A struct (`struct="…"`), or a Go struct type.
    Struct,
    /// A class (`class="…"`).
    Class,
    /// An enum.
    Enum,
    /// An interface, or a Go interface type.
    Interface,
    /// A Rust trait.
    Trait,
    /// A type alias, or another type definition.
    Type,
    /// A module or namespace.
    Module,
    /// A constant or static.
    Constant,
    /// A macro.
    Macro,
}

impl SymbolKind {
    /// Every kind.
    pub const ALL: [Self; 10] = [
        Self::Function,
        Self::Struct,
        Self::Class,
        Self::Enum,
        Self::Interface,
        Self::Trait,
        Self::Type,
        Self::Module,
        Self::Constant,
        Self::Macro,
    ];

    /// The kind's name, as in `function`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Class => "class",
            Self::Enum => "enum",
            Self::Interface => "interface",
            Self::Trait => "trait",
            Self::Type => "type",
            Self::Module => "module",
            Self::Constant => "constant",
            Self::Macro => "macro",
        }
    }

    /// The kind named `name` ([`SymbolKind::name`]).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A symbol defined in source code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub kind: SymbolKind,
    /// The qualified name, its parts joined the language's way: `Point::new` in Rust,
    /// `Greeter.greet` in Python. Methods are qualified by their type, items by the
    /// modules, types and functions they are nested in.
    pub name: String,
    /// The definition, without the doc comments, attributes or decorators before it.
    pub range: ByteRange,
    /// Where the definition starts with its doc comments, attributes and decorators;
    /// `range.start` if it has none.
    pub docs_start: usize,
}

impl Symbol {
    /// The parts of [`name`](Self::name): `["Point", "new"]` for `Point::new` or
    /// `Point.new`.
    pub fn path(&self) -> Vec<&str> {
        split_path(&self.name)
    }
}

/// The parts of the qualified name `name`, separated by `::` or `.`.
pub fn split_path(name: &str) -> Vec<&str> {
    name.split("::")
        .flat_map(|part| part.split('.'))
        .map(str::trim)
        .collect()
}

/// Finds the symbols defined in source code, for symbol includes.
pub trait SymbolLocator: Send + Sync {
    /// The language of the file at `path`, included with `lang` as the fence's info
    /// string if given, if the locator can find symbols in it. It is used as the info
    /// string if there is no `lang`.
    fn language(&self, path: &Path, lang: Option<&str>) -> Option<String>;

    /// The names of the languages it can find symbols in, for diagnostics.
    fn languages(&self) -> Vec<String>;

    /// The symbols defined in `code`, written in `language` (as given by
    /// [`language`](Self::language)), in source order.
    fn symbols(&self, language: &str, code: &str) -> Vec<Symbol>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_through_their_names() {
        for kind in SymbolKind::ALL {
            assert_eq!(SymbolKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(SymbolKind::from_name("method"), None);
    }

    #[test]
    fn paths_split_on_both_separators() {
        assert_eq!(split_path("a::B.c"), ["a", "B", "c"]);
        assert_eq!(split_path("main"), ["main"]);
    }
}
