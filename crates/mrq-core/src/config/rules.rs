//! `lint.rules`: per-rule settings keyed by rule id, alias or tag.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::lint::Registry;

/// The setting of one `lint.rules` entry.
///
/// Written as `true`/`on` (enable with default options), `false`/`off` (disable) or an
/// options object (enable with these options; missing keys keep their defaults). YAML
/// reads a bare `off` as `false`, which is the same thing.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleSetting {
    /// `true`/`on` or `false`/`off`.
    Enabled(bool),
    /// An options object; the rule is enabled.
    Options(serde_json::Map<String, serde_json::Value>),
}

impl RuleSetting {
    /// Whether the setting enables the rule.
    pub const fn is_enabled(&self) -> bool {
        match self {
            Self::Enabled(enabled) => *enabled,
            Self::Options(_) => true,
        }
    }
}

impl Serialize for RuleSetting {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Enabled(enabled) => serializer.serialize_bool(*enabled),
            Self::Options(options) => options.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for RuleSetting {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SettingVisitor;

        impl<'de> Visitor<'de> for SettingVisitor {
            type Value = RuleSetting;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("`true`, `false`, `on`, `off` or a map of rule options")
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<RuleSetting, E> {
                Ok(RuleSetting::Enabled(value))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<RuleSetting, E> {
                match value.to_ascii_lowercase().as_str() {
                    "on" => Ok(RuleSetting::Enabled(true)),
                    "off" => Ok(RuleSetting::Enabled(false)),
                    _ => Err(E::invalid_value(de::Unexpected::Str(value), &self)),
                }
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<RuleSetting, A::Error> {
                let options =
                    serde_json::Map::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(RuleSetting::Options(options))
            }
        }

        deserializer.deserialize_any(SettingVisitor)
    }
}

/// `lint.rules`: settings keyed as written, by rule id (`MD013`), alias (`line-length`)
/// or tag (`whitespace`), all matched ignoring ASCII case.
///
/// Deserializing only checks the shape of each value; [`validate`](Self::validate)
/// checks names and options against a rule registry ([`ConfigLoader`](super::ConfigLoader)
/// does so with the built-in rules). When a [`Linter`](crate::lint::Linter) resolves the
/// settings, tags apply first and rules after them, so a rule's own setting wins over its
/// tags'.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuleSettings(BTreeMap<String, RuleSetting>);

/// A `lint.rules` entry that does not fit the registry.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct RuleSettingError {
    /// The key as written.
    pub key: String,
    pub message: String,
}

impl RuleSettings {
    /// The setting under `key`, exactly as written.
    pub fn get(&self, key: &str) -> Option<&RuleSetting> {
        self.0.get(key)
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every entry, sorted by key.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &RuleSetting)> {
        self.0.iter().map(|(key, setting)| (key.as_str(), setting))
    }

    /// Sets `key`, replacing a previous setting under the same key.
    pub fn insert(&mut self, key: impl Into<String>, setting: RuleSetting) {
        self.0.insert(key.into(), setting);
    }

    /// Checks that every key names a rule or tag of `registry`, that tags are only
    /// switched on or off, and that options parse as the rule's options type.
    pub fn validate(&self, registry: &Registry) -> Result<(), RuleSettingError> {
        for (key, setting) in &self.0 {
            let error = |message: String| RuleSettingError {
                key: key.clone(),
                message,
            };
            if let Some(rule) = registry.find(key) {
                if let RuleSetting::Options(options) = setting {
                    rule.parse_options(Some(options.clone())).map_err(|err| {
                        error(format!("invalid options for lint rule `{key}`: {err}"))
                    })?;
                }
            } else if registry.is_tag(key) {
                if let RuleSetting::Options(_) = setting {
                    return Err(error(format!(
                        "lint tag `{key}` can only be switched on or off, not given options"
                    )));
                }
            } else {
                return Err(error(format!("unknown lint rule `{key}`")));
            }
        }
        Ok(())
    }
}

impl FromIterator<(String, RuleSetting)> for RuleSettings {
    fn from_iter<I: IntoIterator<Item = (String, RuleSetting)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// The schema lists every built-in rule under its id and aliases (a switch or its
/// options) and every tag (a switch), and nothing else.
impl JsonSchema for RuleSettings {
    fn schema_name() -> Cow<'static, str> {
        "RuleSettings".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let switch = generator.subschema_for::<RuleSwitch>();
        let registry = Registry::builtin_ref();
        let mut properties = serde_json::Map::new();
        let mut tags = Vec::new();
        for rule in registry.iter() {
            let meta = rule.meta();
            let options = rule.options_schema(generator);
            let setting = json_schema!({
                "description": format!("{}: {}", meta.id, meta.description),
                "anyOf": [switch.clone(), options]
            });
            for key in std::iter::once(meta.id).chain(meta.aliases.iter().copied()) {
                properties.insert(key.to_owned(), setting.clone().to_value());
            }
            tags.extend(meta.tags.iter().copied());
        }
        tags.sort_unstable();
        tags.dedup();
        for tag in tags {
            if properties.contains_key(tag) {
                continue;
            }
            let schema = json_schema!({
                "description": format!("Every rule tagged `{tag}`"),
                "allOf": [switch.clone()]
            });
            properties.insert(tag.to_owned(), schema.to_value());
        }
        json_schema!({
            "description": "Per-rule settings keyed by rule id, alias or tag: `true`/`on`, `false`/`off`, or the rule's options.",
            "type": "object",
            "properties": properties,
            "additionalProperties": false
        })
    }
}

/// Schema of a rule or tag switched on or off without options.
struct RuleSwitch;

impl JsonSchema for RuleSwitch {
    fn schema_name() -> Cow<'static, str> {
        "RuleSwitch".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "`true`/`on` enables, `false`/`off` disables.",
            "anyOf": [
                { "type": "boolean" },
                { "type": "string", "enum": ["on", "off"] }
            ]
        })
    }
}
