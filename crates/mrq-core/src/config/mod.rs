//! Configuration: the `.mrq.yaml` model, layered loading and provenance.
//!
//! # Layers
//!
//! [`ConfigLoader`] merges, lowest to highest priority and key by key (lists are
//! replaced, not concatenated):
//!
//! 1. built-in defaults ([`Config::default`]),
//! 2. the user config, by default at [`user_config_path`]: `$MRQ_CONFIG_HOME/config.yaml`,
//!    else `$XDG_CONFIG_HOME/mrq/config.yaml`, else `~/.config/mrq/config.yaml`.
//!    `MRQ_CONFIG_HOME` exists mainly so tests can isolate the user config,
//! 3. `.mrq.yaml` files in the start directory and its ancestors, the closest winning
//!    (`.mrq.yml` is accepted where a directory has no `.mrq.yaml`). An explicit
//!    `--config <file>` replaces this discovery; the user config still applies below it,
//! 4. command-line overrides (`--flavor`).
//!
//! Merging is done by figment, which tags every value with its provider; YAML is parsed
//! with serde-saphyr (figment's own `yaml` feature uses the deprecated `serde_yaml`).
//! Line numbers come from a second, span-aware pass over each file, so errors
//! ([`ConfigError::location`]) and [`LoadedConfig::origin`] name the file and line.
//!
//! # Schema
//!
//! Every struct derives [`schemars::JsonSchema`]; [`schema_json`] is the published
//! JSON Schema (`mrq config schema`). `lint.rules` ([`RuleSettings`]) lists every
//! built-in lint rule with the schema of its options.
//!
//! # markdownlint fallback
//!
//! When no `.mrq.yaml` (discovered or `--config`) has a `lint` key, the closest
//! `.markdownlint.jsonc`, `.markdownlint.json`, `.markdownlint.yaml` or
//! `.markdownlint.yml` (in that order of preference within a directory) from the start
//! directory upward is read as a layer right above the user config: `default` becomes
//! `lint.extends` (`true` when missing), `extends` names a base file (relative to the
//! extending one) whose keys the extending file overrides one by one, `$schema` is
//! ignored, and every other key that names a built-in rule, alias or tag becomes a
//! `lint.rules` entry. Other keys (rules mrq does not have, markdownlint plugins) are
//! skipped. JSONC comments and trailing commas are allowed.

mod error;
mod load;
mod locate;
mod markdownlint;
mod model;
mod rules;
mod show;

pub use error::{CONFIG_ERROR_CODE, ConfigError, Location};
pub use load::{
    CONFIG_FILE_NAMES, ConfigLoader, LoadedConfig, Origin, user_cache_dir, user_config_path,
};
pub use markdownlint::MARKDOWNLINT_FILE_NAMES;
pub use model::{
    CodeFence, Config, DiagramsConfig, Emphasis, FlavorName, FmtConfig, FrontmatterFormat,
    FrontmatterSchema, IncludeConfig, KrokiConfig, LintConfig, LintExtends, ListMarker,
    OrderedList, RenderConfig, Strong, ValidateConfig, Wrap,
};
pub use rules::{RuleSetting, RuleSettingError, RuleSettings};

/// The JSON Schema of `.mrq.yaml`.
pub fn schema() -> schemars::Schema {
    schemars::schema_for!(Config)
}

/// [`schema`] as pretty-printed JSON with a trailing newline.
pub fn schema_json() -> String {
    let mut json = serde_json::to_string_pretty(&schema()).unwrap_or_default();
    json.push('\n');
    json
}
