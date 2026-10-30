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
//!
//! # Inline configuration
//!
//! HTML comments switch rules inside a document, with markdownlint's semantics.
//! Each takes the prefix `markdownlint-` or `mrq-` (same meaning) and optional
//! whitespace-separated rule ids, aliases or tags (ignoring ASCII case; unknown names
//! are ignored); without names it applies to every rule:
//!
//! | Comment | Effect |
//! | --- | --- |
//! | `<!-- mrq-disable MD012 -->` | off from the comment's line on |
//! | `<!-- mrq-enable MD012 -->` | on from the comment's line on, even if config left it off |
//! | `<!-- mrq-disable-line -->` | off on the comment's line |
//! | `<!-- mrq-disable-next-line -->` | off on the line after the comment |
//! | `<!-- mrq-disable-file -->` / `mrq-enable-file` | off/on for the whole file, wherever the comment is |
//! | `<!-- mrq-capture -->` | remembers the current state (one slot, not a stack) |
//! | `<!-- mrq-restore -->` | returns to the remembered state, or to the configured one before any capture |
//! | `<!-- mrq-configure-file { "MD012": { "maximum": 2 } } -->` | merges markdownlint-style JSON settings into the config for this file |
//!
//! Like markdownlint, file-wide comments apply first, then `enable`/`disable`/`capture`/
//! `restore` in order (several on one line apply in order, and the line gets the
//! resulting state), then `disable-line`/`disable-next-line`. A finding counts on the
//! line where its range starts. Comments are found only in HTML (block, inline and
//! comment nodes), never in code blocks, code spans or frontmatter.
//!
//! Divergences: a comment may span lines and counts on the line of its `<!--`
//! (markdownlint needs `-->` on that line, except for `configure-file`), and
//! `configure-file` silently drops invalid JSON and entries that are not valid
//! `lint.rules` settings; `default: true/false` sets `extends`.

mod linter;
mod registry;
mod rule;
pub mod rules;
mod suppress;

pub use linter::Linter;
pub use registry::Registry;
pub use rule::{NoOptions, Rule, RuleMeta, RuleOptions};
