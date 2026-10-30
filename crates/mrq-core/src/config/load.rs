//! Discovery and layered loading of config files via figment.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use figment::value::{Dict, Map, Value};
use figment::{Figment, Metadata, Profile, Provider, Source};

use super::error::{ConfigError, Location};
use super::locate::{KeyIndex, Pos};
use super::markdownlint;
use super::model::{Config, FlavorName};
use super::rules::RuleSettingError;
use crate::lint::Registry;
use crate::source::ByteRange;

/// File names of the per-directory config, in order of preference within one directory.
pub const CONFIG_FILE_NAMES: [&str; 2] = [".mrq.yaml", ".mrq.yml"];

const DEFAULT_NAME: &str = "default";
const CLI_NAME: &str = "cli";
const FILE_NAME: &str = "config file";

/// Where a configuration value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Built-in default.
    Default,
    /// A command-line flag.
    Cli,
    /// A config file, with the 1-based line of the key or item.
    File { path: PathBuf, line: usize },
}

/// The user config file: `$MRQ_CONFIG_HOME/config.yaml`, else
/// `$XDG_CONFIG_HOME/mrq/config.yaml`, else `~/.config/mrq/config.yaml` (home from `HOME`,
/// then `USERPROFILE`). Empty variables count as unset. `env` looks up environment
/// variables; `None` when no location can be determined.
pub fn user_config_path(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let var = |key: &str| {
        env(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = var("MRQ_CONFIG_HOME") {
        return Some(dir.join("config.yaml"));
    }
    if let Some(dir) = var("XDG_CONFIG_HOME") {
        return Some(dir.join("mrq").join("config.yaml"));
    }
    let home = var("HOME").or_else(|| var("USERPROFILE"))?;
    Some(home.join(".config").join("mrq").join("config.yaml"))
}

/// The user cache directory of mrq: `$MRQ_CACHE_HOME`, else `$XDG_CACHE_HOME/mrq`, else
/// `~/.cache/mrq` (home from `HOME`, then `USERPROFILE`), like [`user_config_path`].
/// Empty variables count as unset. `env` looks up environment variables; `None` when no
/// location can be determined.
pub fn user_cache_dir(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let var = |key: &str| {
        env(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = var("MRQ_CACHE_HOME") {
        return Some(dir);
    }
    if let Some(dir) = var("XDG_CACHE_HOME") {
        return Some(dir.join("mrq"));
    }
    let home = var("HOME").or_else(|| var("USERPROFILE"))?;
    Some(home.join(".cache").join("mrq"))
}

/// Builds the layered configuration.
///
/// Layers, lowest to highest priority; later layers override earlier ones key by key
/// (lists are replaced, not concatenated):
///
/// 1. built-in defaults,
/// 2. the user config (ignored when the file does not exist),
/// 3. either the explicit config file (`--config`, which must exist and replaces
///    discovery), or every `.mrq.yaml` (or `.mrq.yml`) from the filesystem root down to
///    the start directory, so the closest one wins,
/// 4. command-line overrides (`--flavor`).
#[derive(Debug, Clone)]
pub struct ConfigLoader {
    start_dir: PathBuf,
    user_config: Option<PathBuf>,
    explicit: Option<PathBuf>,
    flavor: Option<FlavorName>,
}

impl ConfigLoader {
    /// A loader discovering `.mrq.yaml` files from `start_dir` upward, without a user config.
    pub fn new(start_dir: impl Into<PathBuf>) -> Self {
        Self {
            start_dir: start_dir.into(),
            user_config: None,
            explicit: None,
            flavor: None,
        }
    }

    /// Sets the user config file, e.g. from [`user_config_path`].
    #[must_use]
    pub fn user_config(mut self, path: Option<PathBuf>) -> Self {
        self.user_config = path;
        self
    }

    /// Uses this file instead of discovering `.mrq.yaml` files (`--config`).
    #[must_use]
    pub fn explicit(mut self, path: Option<PathBuf>) -> Self {
        self.explicit = path;
        self
    }

    /// Overrides `flavor` (`--flavor`).
    #[must_use]
    pub fn flavor(mut self, flavor: Option<FlavorName>) -> Self {
        self.flavor = flavor;
        self
    }

    /// The user config file if it exists, and the `.mrq.yaml` files that apply, lowest
    /// priority first.
    fn files(&self) -> (Option<PathBuf>, Vec<PathBuf>) {
        let user = self.user_config.clone().filter(|user| user.is_file());
        let mut files = Vec::new();
        if let Some(explicit) = &self.explicit {
            files.push(explicit.clone());
        } else {
            let mut discovered: Vec<PathBuf> = self
                .start_dir
                .ancestors()
                .filter_map(|dir| {
                    CONFIG_FILE_NAMES
                        .iter()
                        .map(|name| dir.join(name))
                        .find(|path| path.is_file())
                })
                .collect();
            discovered.reverse();
            files.extend(discovered);
        }
        (user, files)
    }

    /// Reads, parses and merges every layer.
    pub fn load(&self) -> Result<LoadedConfig, ConfigError> {
        let (user, files) = self.files();
        let mut layers = Vec::new();
        if let Some(user) = user {
            layers.push(Layer::read(user)?);
        }
        let mut project = Vec::new();
        for path in files {
            project.push(Layer::read(path)?);
        }
        if !project.iter().any(|layer| layer.has("lint")) {
            layers.extend(markdownlint::layers(&self.start_dir)?);
        }
        layers.extend(project);

        let defaults = Value::serialize(Config::default()).map_err(|err| internal(&err))?;
        let mut figment = Figment::new().merge(DictProvider {
            metadata: Metadata::named(DEFAULT_NAME),
            value: defaults,
        });
        for layer in &layers {
            figment = figment.merge(DictProvider {
                metadata: Metadata::from(FILE_NAME, Source::File(layer.path.clone())),
                value: Value::from(layer.dict.clone()),
            });
        }
        if let Some(flavor) = self.flavor {
            let mut cli = Dict::new();
            cli.insert(
                "flavor".to_owned(),
                Value::serialize(flavor).map_err(|err| internal(&err))?,
            );
            figment = figment.merge(DictProvider {
                metadata: Metadata::named(CLI_NAME),
                value: Value::from(cli),
            });
        }

        let config: Config = figment.extract().map_err(|err| invalid(&err, &layers))?;
        config
            .lint
            .rules
            .validate(Registry::builtin_ref())
            .map_err(|err| invalid_rule(&err, &layers))?;
        let root = figment.find_value("").map_err(|err| internal(&err))?;
        Ok(LoadedConfig {
            config,
            figment,
            root,
            layers,
        })
    }
}

/// The merged configuration plus what is needed to explain where each value came from.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    config: Config,
    figment: Figment,
    /// The merged figment value; every leaf is tagged with its provider.
    root: Value,
    layers: Vec<Layer>,
}

impl LoadedConfig {
    /// The effective configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The config files that were merged, lowest priority first.
    pub fn files(&self) -> Vec<PathBuf> {
        self.layers.iter().map(|layer| layer.path.clone()).collect()
    }

    /// Where the value at `path` (map keys and decimal list indices) came from, or
    /// `None` if there is no such value.
    pub fn origin<S: AsRef<str>>(&self, path: &[S]) -> Option<Origin> {
        let mut value = &self.root;
        for segment in path {
            let segment = segment.as_ref();
            value = match value {
                Value::Dict(_, dict) => dict.get(segment)?,
                Value::Array(_, items) => items.get(segment.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        let metadata = self.figment.get_metadata(value.tag())?;
        Some(match &metadata.source {
            Some(Source::File(file)) => {
                let line = self
                    .layers
                    .iter()
                    .find(|layer| &layer.path == file)
                    .and_then(|layer| layer.index.get(path))
                    .map_or(1, |pos| pos.line);
                Origin::File {
                    path: file.clone(),
                    line,
                }
            }
            _ if metadata.name == CLI_NAME => Origin::Cli,
            _ => Origin::Default,
        })
    }
}

/// One parsed config file.
#[derive(Debug, Clone)]
pub(super) struct Layer {
    pub(super) path: PathBuf,
    pub(super) text: String,
    pub(super) dict: Dict,
    pub(super) index: KeyIndex,
}

impl Layer {
    fn read(path: PathBuf) -> Result<Self, ConfigError> {
        let text = read_text(&path)?;
        let dict = parse_yaml(&path, &text)?;
        let index = KeyIndex::build(&text);
        Ok(Self {
            path,
            text,
            dict,
            index,
        })
    }

    /// Whether the file sets key `key` at the top level.
    fn has(&self, key: &str) -> bool {
        self.dict.contains_key(key)
    }
}

/// The text of a config file.
pub(super) fn read_text(path: &Path) -> Result<String, ConfigError> {
    fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_owned(),
        source,
    })
}

/// Parses YAML text whose top level must be a mapping (or empty).
pub(super) fn parse_yaml(path: &Path, text: &str) -> Result<Dict, ConfigError> {
    let value: Option<Value> = match serde_saphyr::from_str(text) {
        Ok(value) => value,
        Err(err) => {
            let (line, column) = err
                .location()
                .map_or((1, 1), |loc| (to_usize(loc.line()), to_usize(loc.column())));
            let message = first_line(&err.to_string());
            let location = location_at(path, text, line.max(1), column.max(1));
            return Err(ConfigError::Syntax { location, message });
        }
    };
    match value {
        None | Some(Value::Empty(..)) => Ok(Dict::new()),
        Some(Value::Dict(_, dict)) => Ok(dict),
        Some(_) => Err(not_a_mapping(path, text)),
    }
}

/// The error for a config file whose top level is not a mapping.
pub(super) fn not_a_mapping(path: &Path, text: &str) -> ConfigError {
    ConfigError::Syntax {
        location: location_at(path, text, 1, 1),
        message: "the top level must be a mapping of keys to values".to_owned(),
    }
}

/// A figment provider over an already parsed value.
struct DictProvider {
    metadata: Metadata,
    value: Value,
}

impl Provider for DictProvider {
    fn metadata(&self) -> Metadata {
        self.metadata.clone()
    }

    fn data(&self) -> figment::Result<Map<Profile, Dict>> {
        let dict = self.value.clone().into_dict().unwrap_or_default();
        Ok(Profile::Default.collect(dict))
    }
}

/// A 1-based line or column from serde-saphyr; saturates on 32-bit targets.
fn to_usize(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// The first line of a (possibly multi-line, snippet-carrying) message.
fn first_line(message: &str) -> String {
    message.lines().next().unwrap_or_default().trim().to_owned()
}

/// A [`Location`] for a 1-based line and character column of `text`.
pub(super) fn location_at(path: &Path, text: &str, line: usize, column: usize) -> Location {
    let line_start: usize = text
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum();
    let rest = &text[line_start.min(text.len())..];
    let start = line_start
        + rest
            .char_indices()
            .nth(column - 1)
            .map_or(rest.len(), |(offset, _)| offset);
    let end = text[start..]
        .chars()
        .next()
        .filter(|c| *c != '\n')
        .map_or(start, |c| start + c.len_utf8());
    Location {
        path: path.to_owned(),
        line,
        column,
        range: ByteRange::new(start, end),
    }
}

pub(super) fn location_of(path: &Path, pos: Pos) -> Location {
    Location {
        path: path.to_owned(),
        line: pos.line,
        column: pos.column,
        range: pos.range,
    }
}

/// An error that cannot happen for well-formed built-in values.
fn internal(err: &figment::Error) -> ConfigError {
    ConfigError::Invalid {
        location: None,
        key: String::new(),
        message: format!("internal configuration error: {err}"),
    }
}

/// Maps an invalid `lint.rules` entry to the file and line that set it.
fn invalid_rule(err: &RuleSettingError, layers: &[Layer]) -> ConfigError {
    let path = ["lint", "rules", err.key.as_str()];
    let location = layers.iter().rev().find_map(|layer| {
        layer
            .index
            .get(&path)
            .map(|pos| location_of(&layer.path, pos))
    });
    ConfigError::Invalid {
        location,
        key: path.join("."),
        message: err.message.clone(),
    }
}

/// Maps a figment extraction error to the file and line that set the offending key.
fn invalid(err: &figment::Error, layers: &[Layer]) -> ConfigError {
    use figment::error::Kind;

    let mut path = err.path.clone();
    // figment's path may or may not already end in the unknown or missing field.
    let parent_of = |path: &mut Vec<String>, field: &str| {
        if path.last().is_some_and(|last| last == field) {
            path.pop();
        }
        let parent = path.join(".");
        path.push(field.to_owned());
        parent
    };
    let message = match &err.kind {
        Kind::UnknownField(field, expected) => {
            let parent = parent_of(&mut path, field);
            let mut message = if parent.is_empty() {
                format!("unknown key `{field}`")
            } else {
                format!("unknown key `{field}` in `{parent}`")
            };
            if !expected.is_empty() {
                let expected: Vec<String> = expected.iter().map(|k| format!("`{k}`")).collect();
                let _ = write!(message, "; expected one of {}", expected.join(", "));
            }
            message
        }
        Kind::MissingField(field) => {
            let parent = parent_of(&mut path, field);
            format!("missing key `{field}` in `{parent}`")
        }
        kind => format!("invalid value for `{}`: {kind}", path.join(".")),
    };
    let key = path.join(".");

    // The highest-priority file that has the longest prefix of the key path.
    let location = (1..=path.len()).rev().find_map(|len| {
        layers.iter().rev().find_map(|layer| {
            layer
                .index
                .get(&path[..len])
                .map(|pos| location_of(&layer.path, pos))
        })
    });
    let location = location.or_else(|| {
        let file = err.metadata.as_ref()?.source.as_ref()?.file_path()?;
        layers
            .iter()
            .find(|layer| layer.path == file)
            .map(|layer| location_at(&layer.path, &layer.text, 1, 1))
    });
    ConfigError::Invalid {
        location,
        key,
        message,
    }
}
