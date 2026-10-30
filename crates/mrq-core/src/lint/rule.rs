//! The [`Rule`] trait, rule metadata and the type-erased form the registry stores.

use std::any::Any;
use std::fmt::Debug;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Document;
use crate::diag::{Diagnostic, Severity};
use crate::source::ByteRange;

/// Static description of a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleMeta {
    /// Stable id and diagnostic code: `MD013`, `MRQ001`, ….
    pub id: &'static str,
    /// markdownlint's names for the rule (`line-length`); accepted in config like the id.
    pub aliases: &'static [&'static str],
    /// markdownlint's tags (`headings`, `whitespace`, …); a tag in config toggles every
    /// rule that has it.
    pub tags: &'static [&'static str],
    /// One line, as in markdownlint's rule list.
    pub description: &'static str,
    /// Whether `lint.extends: default` enables the rule.
    pub default_enabled: bool,
}

impl RuleMeta {
    /// Whether `name` is the id or an alias, ignoring ASCII case.
    pub fn is_named(&self, name: &str) -> bool {
        self.id.eq_ignore_ascii_case(name)
            || self
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
    }

    /// Whether the rule has tag `tag`, ignoring ASCII case.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t.eq_ignore_ascii_case(tag))
    }

    /// A warning with this rule's id as code, about `range` of `doc`.
    pub fn diagnostic(
        &self,
        doc: &Document,
        range: ByteRange,
        message: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic::new(self.id, Severity::Warning, doc.name(), range, message)
    }
}

/// What a rule's options type needs: config deserialization with defaults for missing
/// keys (`#[serde(default, deny_unknown_fields)]` on a struct), serialization, and a
/// JSON Schema for `mrq config schema`. Implemented for every such type.
pub trait RuleOptions:
    DeserializeOwned + Serialize + JsonSchema + Default + Debug + Send + Sync + 'static
{
}

impl<T> RuleOptions for T where
    T: DeserializeOwned + Serialize + JsonSchema + Default + Debug + Send + Sync + 'static
{
}

/// Options of a rule that has none: only `{}` is accepted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoOptions {}

/// A lint rule: checks a document with typed options.
///
/// Rules are usually unit structs; [`Registry::register`](super::Registry::register)
/// takes care of reading the options from config and of the schema.
pub trait Rule: Send + Sync + 'static {
    /// The rule's options, read from `lint.rules.<id>` (missing keys take the defaults).
    type Options: RuleOptions;

    /// Id, aliases, tags, description and default.
    const META: RuleMeta;

    /// The findings in `doc`, each with an optional [`Fix`](crate::diag::Fix) whose edits
    /// apply to [`Document::text`]. [`RuleMeta::diagnostic`] makes a diagnostic with the
    /// right code and severity. Order does not matter.
    fn check(&self, doc: &Document, options: &Self::Options) -> Vec<Diagnostic>;
}

/// Options after [`DynRule::parse_options`], downcast again by the same rule.
pub(crate) type AnyOptions = Box<dyn Any + Send + Sync>;

/// Object-safe form of [`Rule`], implemented for every rule.
pub(crate) trait DynRule: Send + Sync {
    fn meta(&self) -> RuleMeta;

    /// The schema of the options; definitions are added to `generator`.
    fn options_schema(&self, generator: &mut SchemaGenerator) -> Schema;

    /// Typed options from a JSON object (or the defaults for `None`).
    fn parse_options(
        &self,
        value: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<AnyOptions, serde_json::Error>;

    /// Runs the rule with options made by [`parse_options`](Self::parse_options).
    fn check(&self, doc: &Document, options: &(dyn Any + Send + Sync)) -> Vec<Diagnostic>;
}

impl<R: Rule> DynRule for R {
    fn meta(&self) -> RuleMeta {
        R::META
    }

    fn options_schema(&self, generator: &mut SchemaGenerator) -> Schema {
        generator.subschema_for::<R::Options>()
    }

    fn parse_options(
        &self,
        value: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<AnyOptions, serde_json::Error> {
        let options: R::Options = match value {
            Some(map) => serde_json::from_value(serde_json::Value::Object(map))?,
            None => R::Options::default(),
        };
        Ok(Box::new(options))
    }

    fn check(&self, doc: &Document, options: &(dyn Any + Send + Sync)) -> Vec<Diagnostic> {
        let options = options
            .downcast_ref::<R::Options>()
            .expect("options were parsed by the same rule");
        Rule::check(self, doc, options)
    }
}
