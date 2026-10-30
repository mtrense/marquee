//! Lint: markdownlint-compatible style rules (`MD###`) and mrq's own (`MRQ###`).
//!
//! A [`Rule`] is a type with [`RuleMeta`] (id, markdownlint aliases and tags,
//! description, whether it is on by default), an options type and a `check` that returns
//! [`Diagnostic`](crate::diag::Diagnostic)s, optionally with a
//! [`Fix`](crate::diag::Fix) made of [`TextEdit`](crate::source::TextEdit)s against the
//! document text. The options type derives serde and schemars, so the [`Registry`] reads
//! it from `lint.rules.<id>` and contributes its schema to `mrq config schema`:
//!
//! ```
//! use mrq_core::Document;
//! use mrq_core::config::{LintConfig, RuleSetting};
//! use mrq_core::diag::Diagnostic;
//! use mrq_core::lint::{Linter, Registry, Rule, RuleMeta};
//! use mrq_core::source::ByteRange;
//!
//! struct NoTodo;
//!
//! #[derive(Debug, Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
//! #[serde(default, deny_unknown_fields)]
//! struct NoTodoOptions {
//!     /// The marker to look for; `TODO` when empty.
//!     marker: String,
//! }
//!
//! impl Rule for NoTodo {
//!     type Options = NoTodoOptions;
//!     const META: RuleMeta = RuleMeta {
//!         id: "X001",
//!         aliases: &["no-todo"],
//!         tags: &["custom"],
//!         description: "No TODO markers",
//!         default_enabled: true,
//!     };
//!
//!     fn check(&self, doc: &Document, options: &NoTodoOptions) -> Vec<Diagnostic> {
//!         let marker = if options.marker.is_empty() { "TODO" } else { &options.marker };
//!         doc.text()
//!             .match_indices(marker)
//!             .map(|(at, m)| Self::META.diagnostic(doc, ByteRange::new(at, at + m.len()), "TODO left"))
//!             .collect()
//!     }
//! }
//!
//! let mut registry = Registry::new();
//! registry.register(NoTodo);
//! let mut config = LintConfig::default();
//! let options = serde_json::json!({ "marker": "FIXME" }).as_object().unwrap().clone();
//! config.rules.insert("no-todo", RuleSetting::Options(options));
//! let linter = Linter::new(&registry, &config)?;
//! let diagnostics = linter.check(&Document::parse("FIXME: lint\n"));
//! assert_eq!(diagnostics[0].code, "X001");
//! # Ok::<(), mrq_core::config::RuleSettingError>(())
//! ```
//!
//! # Configuration
//!
//! [`Linter::new`] selects rules from a [`LintConfig`](crate::config::LintConfig) like
//! markdownlint: `extends: default` (markdownlint's `default: true`) starts from the rules
//! that are on by default, `extends: none` from none. Then every `lint.rules` key that is
//! a tag switches the rules with that tag, and every key that is a rule id or alias
//! switches or configures that rule. Keys ignore ASCII case. A value is `true`/`on`,
//! `false`/`off` (YAML reads a bare `off` as `false`) or an options object, which
//! enables the rule.
//!
//! [`ConfigLoader`](crate::config::ConfigLoader) checks `lint.rules` against the built-in
//! rules, so mistakes are reported with the file and line, and falls back to
//! `.markdownlint.{jsonc,json,yaml,yml}` when no `.mrq.yaml` has a `lint` section.
//!
//! Every lint diagnostic is a [`Warning`](crate::diag::Severity::Warning) whose code is
//! the rule id.

mod linter;
mod registry;
mod rule;
pub mod rules;

pub use linter::Linter;
pub use registry::Registry;
pub use rule::{NoOptions, Rule, RuleMeta, RuleOptions};
