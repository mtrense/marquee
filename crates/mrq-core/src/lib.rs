//! Core library of `mrq`: document model, parsing, query, validation, lint, formatting and transforms.
//!
//! Start with [`Document`] (one parsed file) or [`Workspace`] (several); the README's
//! library example is compiled as a doctest.

pub mod ast;
pub mod config;
pub mod diag;
pub mod directive;
pub mod document;
pub mod flavor;
pub mod fmt;
pub mod frontmatter;
pub mod input;
pub mod lint;
pub mod parse;
pub mod query;
pub mod source;
pub mod stats;
pub mod symbol;
pub mod toc;
pub mod transform;
pub mod validate;

pub use document::{Document, Workspace};

/// Compiles the Rust examples of `README.md` as doctests.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
pub struct ReadmeDoctests;
