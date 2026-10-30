//! The `.mrq.yaml` data model: one struct per README section.
//!
//! Every struct rejects unknown keys (`deny_unknown_fields`) and fills missing keys
//! from its [`Default`], so a config file only needs the keys it changes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The effective configuration of mrq (`.mrq.yaml`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(title = "mrq configuration (.mrq.yaml)")]
pub struct Config {
    /// Markdown flavor used when neither `--flavor` nor frontmatter `mrq.flavor` picks one.
    pub flavor: FlavorName,
    /// Globs of files and directories to skip when walking directories.
    pub ignore: Vec<String>,
    /// `mrq lint` settings.
    pub lint: LintConfig,
    /// `mrq fmt` settings.
    pub fmt: FmtConfig,
    /// `mrq validate` settings.
    pub validate: ValidateConfig,
    /// `mrq:include` settings.
    pub include: IncludeConfig,
    /// `mrq render` settings.
    pub render: RenderConfig,
}

/// A built-in Markdown flavor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FlavorName {
    /// Strict `CommonMark`.
    Commonmark,
    /// GitHub Flavored Markdown.
    #[default]
    Gfm,
    /// gfm + wikilinks, embeds, callouts, tags, math, comments.
    Obsidian,
    /// gfm + admonitions, heading attributes, definition lists, math, abbreviations.
    Mkdocs,
}

/// `lint` section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LintConfig {
    /// Base rule set: `default` enables every rule that is on by default, `none` starts empty.
    pub extends: LintExtends,
    /// Per-rule settings keyed by rule id (`MD013`, `MRQ001`, …): an options object,
    /// `off`/`false` to disable, or `on`/`true` to enable with defaults.
    ///
    /// Open for now: rule ids and their option types are validated once the rule
    /// registry exists (M5), which will also extend this schema.
    pub rules: BTreeMap<String, serde_json::Value>,
}

/// Base rule set of `lint.extends`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LintExtends {
    /// Every rule that is enabled by default.
    #[default]
    Default,
    /// No rules; only those listed under `rules` run.
    None,
}

/// `fmt` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct FmtConfig {
    /// Prose wrapping mode.
    pub wrap: Wrap,
    /// Line width for `wrap: width`.
    pub width: u16,
    /// Emphasis marker.
    pub emphasis: Emphasis,
    /// Strong emphasis marker.
    pub strong: Strong,
    /// Bullet list marker.
    pub list_marker: ListMarker,
    /// Ordered list numbering.
    pub ordered_list: OrderedList,
    /// Code fence.
    pub code_fence: CodeFence,
    /// Frontmatter handling.
    pub frontmatter: FrontmatterFormat,
    /// External formatters for fenced code, keyed by language: a command and its
    /// arguments, reading the code on stdin and writing the result to stdout.
    pub code_formatters: BTreeMap<String, Vec<String>>,
}

impl Default for FmtConfig {
    fn default() -> Self {
        Self {
            wrap: Wrap::Preserve,
            width: 80,
            emphasis: Emphasis::Star,
            strong: Strong::Star,
            list_marker: ListMarker::Dash,
            ordered_list: OrderedList::Ordered,
            code_fence: CodeFence::Backtick,
            frontmatter: FrontmatterFormat::Preserve,
            code_formatters: BTreeMap::new(),
        }
    }
}

/// Prose wrapping mode of `fmt.wrap`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Wrap {
    /// Keep line breaks as written.
    #[default]
    Preserve,
    /// One line per paragraph.
    Never,
    /// Wrap at `fmt.width` columns.
    Width,
    /// One sentence per line.
    Sentence,
}

/// Emphasis marker of `fmt.emphasis`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Emphasis {
    #[default]
    #[serde(rename = "*")]
    Star,
    #[serde(rename = "_")]
    Underscore,
}

impl Emphasis {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Star => "*",
            Self::Underscore => "_",
        }
    }
}

/// Strong emphasis marker of `fmt.strong`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Strong {
    #[default]
    #[serde(rename = "**")]
    Star,
    #[serde(rename = "__")]
    Underscore,
}

impl Strong {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Star => "**",
            Self::Underscore => "__",
        }
    }
}

/// Bullet list marker of `fmt.list_marker`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ListMarker {
    #[default]
    #[serde(rename = "-")]
    Dash,
    #[serde(rename = "*")]
    Star,
    #[serde(rename = "+")]
    Plus,
}

impl ListMarker {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dash => "-",
            Self::Star => "*",
            Self::Plus => "+",
        }
    }
}

/// Ordered list numbering of `fmt.ordered_list`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OrderedList {
    /// Every item is numbered `1.`.
    One,
    /// Items are numbered consecutively from the first number.
    #[default]
    Ordered,
    /// Numbers are kept as written.
    Preserve,
}

/// Code fence of `fmt.code_fence`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum CodeFence {
    #[default]
    #[serde(rename = "```")]
    Backtick,
    #[serde(rename = "~~~")]
    Tilde,
}

impl CodeFence {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backtick => "```",
            Self::Tilde => "~~~",
        }
    }
}

/// Frontmatter handling of `fmt.frontmatter`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FrontmatterFormat {
    /// Keep frontmatter bytes verbatim.
    #[default]
    Preserve,
    /// Re-serialize frontmatter canonically.
    Normalize,
}

/// `validate` section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ValidateConfig {
    /// Check external http(s) links (also enabled by `--external`).
    pub external: bool,
    /// Regular expressions of external URLs never to check.
    pub external_exclude: Vec<String>,
    /// JSON Schemas for frontmatter, per glob.
    pub frontmatter: Vec<FrontmatterSchema>,
}

/// One `validate.frontmatter` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrontmatterSchema {
    /// Glob of the files the schema applies to.
    pub files: String,
    /// Path of the JSON Schema, relative to the config file that sets it.
    pub schema: PathBuf,
}

/// `include` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct IncludeConfig {
    /// Allow `mrq:include` from http(s) URLs (needs the `remote-include` feature).
    pub allow_remote: bool,
}

/// `render` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RenderConfig {
    /// Built-in theme name (`light`, `dark`, `github`, `paper`) or a theme file path.
    pub style: Option<String>,
    /// Diagram rendering.
    pub diagrams: DiagramsConfig,
}

/// `render.diagrams` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DiagramsConfig {
    /// The Kroki diagram service, tried after built-in renderers and commands.
    pub kroki: KrokiConfig,
    /// Custom renderers keyed by fence language: a command and its arguments, reading
    /// the diagram source on stdin and writing SVG to stdout.
    pub commands: BTreeMap<String, Vec<String>>,
}

/// `render.diagrams.kroki` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct KrokiConfig {
    /// Kroki is never contacted unless this is `true`.
    pub enabled: bool,
    /// Base URL of the Kroki service.
    pub url: String,
}

impl Default for KrokiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: "https://kroki.io".to_owned(),
        }
    }
}
