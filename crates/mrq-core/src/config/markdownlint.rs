//! The `.markdownlint.*` fallback for `lint`: markdownlint config files read as layers.

use std::path::{Path, PathBuf};

use figment::value::{Dict, Value};

use super::error::ConfigError;
use super::load::{Layer, location_at, location_of, not_a_mapping, parse_yaml, read_text};
use super::locate::KeyIndex;
use super::model::LintExtends;
use crate::lint::Registry;

/// markdownlint config file names, in order of preference within one directory.
pub const MARKDOWNLINT_FILE_NAMES: [&str; 4] = [
    ".markdownlint.jsonc",
    ".markdownlint.json",
    ".markdownlint.yaml",
    ".markdownlint.yml",
];

/// The layers of the closest markdownlint config from `start_dir` upward: the file and
/// the files it `extends`, base first. Empty when there is none.
pub(super) fn layers(start_dir: &Path) -> Result<Vec<Layer>, ConfigError> {
    let Some(path) = start_dir.ancestors().find_map(|dir| {
        MARKDOWNLINT_FILE_NAMES
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
    }) else {
        return Ok(Vec::new());
    };

    // Follow `extends` to the base, then convert base first.
    let mut chain: Vec<(PathBuf, String, Dict)> = Vec::new();
    let mut next = Some(path);
    while let Some(path) = next.take() {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        if chain
            .iter()
            .any(|(seen, ..)| seen.canonicalize().unwrap_or_else(|_| seen.clone()) == canonical)
        {
            let (from, text, _) = chain.last().expect("a cycle needs an extending file");
            return Err(extends_error(
                from,
                text,
                format!("`extends` cycle: {} is extended again", path.display()),
            ));
        }
        if let Some((from, text, _)) = chain.last()
            && !path.is_file()
        {
            return Err(extends_error(
                from,
                text,
                format!("`extends` file {} does not exist", path.display()),
            ));
        }
        let text = read_text(&path)?;
        let dict = parse(&path, &text)?;
        if let Some(base) = dict.get("extends") {
            let Some(base) = base.as_str() else {
                return Err(extends_error(
                    &path,
                    &text,
                    "`extends` must be a file path".to_owned(),
                ));
            };
            let dir = path.parent().unwrap_or_else(|| Path::new(""));
            next = Some(dir.join(base));
        }
        chain.push((path, text, dict));
    }

    let registry = Registry::builtin_ref();
    Ok(chain
        .into_iter()
        .rev()
        .enumerate()
        .map(|(i, (path, text, dict))| convert(path, text, &dict, registry, i == 0))
        .collect())
}

/// Parses a markdownlint file: JSON (with JSONC comments and trailing commas) unless the
/// extension is `.yaml`/`.yml`.
fn parse(path: &Path, text: &str) -> Result<Dict, ConfigError> {
    let yaml = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"));
    if yaml {
        return parse_yaml(path, text);
    }
    let json: serde_json::Value = serde_json::from_str(&strip_jsonc(text)).map_err(|err| {
        let message = err.to_string();
        let message = message
            .split(" at line ")
            .next()
            .unwrap_or(&message)
            .to_owned();
        ConfigError::Syntax {
            location: location_at(path, text, err.line().max(1), err.column().max(1)),
            message,
        }
    })?;
    match Value::serialize(json) {
        Ok(Value::Dict(_, dict)) => Ok(dict),
        _ => Err(not_a_mapping(path, text)),
    }
}

/// One markdownlint file as an mrq layer: `{ lint: { extends, rules } }`. `extends` is
/// set from `default`, or to `default` for the base file when it has none.
fn convert(path: PathBuf, text: String, dict: &Dict, registry: &Registry, base: bool) -> Layer {
    let mut lint = Dict::new();
    let mut rules = Dict::new();
    let mut extends = base.then_some(LintExtends::Default);
    let mut keys: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in dict {
        if key.eq_ignore_ascii_case("default") {
            if let Some(default) = value.to_bool() {
                extends = Some(if default {
                    LintExtends::Default
                } else {
                    LintExtends::None
                });
                keys.push((key.clone(), vec!["lint".into(), "extends".into()]));
            }
        } else if registry.find(key).is_some() {
            rules.insert(key.clone(), value.clone());
            keys.push((
                key.clone(),
                vec!["lint".into(), "rules".into(), key.clone()],
            ));
        } else if registry.is_tag(key) {
            // markdownlint switches a tag on for any truthy value.
            let on = value.to_bool().unwrap_or(true);
            rules.insert(key.clone(), Value::from(on));
            keys.push((
                key.clone(),
                vec!["lint".into(), "rules".into(), key.clone()],
            ));
        }
    }
    if let Some(extends) = extends {
        let name = match extends {
            LintExtends::Default => "default",
            LintExtends::None => "none",
        };
        lint.insert("extends".to_owned(), Value::from(name));
    }
    lint.insert("rules".to_owned(), Value::from(rules));
    let mut root = Dict::new();
    root.insert("lint".to_owned(), Value::from(lint));

    let index = KeyIndex::build(&text).remap(|path| {
        let (_, target) = keys.iter().find(|(key, _)| *key == path[0])?;
        let mut target = target.clone();
        if target.len() > 2 {
            target.extend(path[1..].iter().cloned());
        }
        Some(target)
    });
    Layer {
        path,
        text,
        dict: root,
        index,
    }
}

/// An error about the `extends` key of `path`.
fn extends_error(path: &Path, text: &str, message: String) -> ConfigError {
    let location = KeyIndex::build(text).get(&["extends"]).map_or_else(
        || location_at(path, text, 1, 1),
        |pos| location_of(path, pos),
    );
    ConfigError::Invalid {
        location: Some(location),
        key: "extends".to_owned(),
        message,
    }
}

/// JSONC to JSON: comments and trailing commas become spaces, so byte offsets, lines and
/// columns stay the same.
fn strip_jsonc(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0;
    // Index of the last comma outside strings and comments, until a non-space follows.
    let mut comma: Option<usize> = None;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                comma = None;
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    out[i] = b' ';
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = text[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |e| i + 2 + e + 2);
                for byte in &mut out[i..end] {
                    if *byte != b'\n' {
                        *byte = b' ';
                    }
                }
                i = end;
            }
            b',' => {
                comma = Some(i);
                i += 1;
            }
            b'}' | b']' => {
                if let Some(at) = comma.take() {
                    out[at] = b' ';
                }
                i += 1;
            }
            byte => {
                if !byte.is_ascii_whitespace() {
                    comma = None;
                }
                i += 1;
            }
        }
    }
    // Only ASCII bytes outside strings were replaced by spaces; multi-byte characters
    // inside comments became several spaces, which is still valid UTF-8.
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_jsonc_keeps_offsets_and_strings() {
        let text = "{ // c\n  \"a//b\": \"/*x*/\", /* d\n */ \"e\": [1, 2,],\n}";
        let stripped = strip_jsonc(text);
        assert_eq!(stripped.len(), text.len());
        let value: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(value, serde_json::json!({ "a//b": "/*x*/", "e": [1, 2] }));
        assert_eq!(stripped.lines().count(), text.lines().count());
    }

    #[test]
    fn strip_jsonc_handles_escaped_quotes_and_unicode_comments() {
        let text = "{\"a\\\"\": 1 /* ü */}";
        let value: serde_json::Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
        assert_eq!(value, serde_json::json!({ "a\"": 1 }));
    }
}
