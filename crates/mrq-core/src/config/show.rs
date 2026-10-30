//! YAML rendering of the effective configuration for `mrq config show [--origin]`.

use std::fmt::Write as _;

use figment::value::{Empty, Num, Tag, Value};

use super::load::{LoadedConfig, Origin};

impl LoadedConfig {
    /// The effective configuration as YAML, keys sorted. With `origins`, every value is
    /// followed by a comment naming its source: `path:line`, `default` or `cli`.
    pub fn to_yaml(&self, origins: bool) -> String {
        // Infallible for the config types: maps have string keys and there are no
        // unsupported values.
        let value =
            Value::serialize(self.config()).unwrap_or(Value::Empty(Tag::Default, Empty::Unit));
        let mut out = String::new();
        let mut path = Vec::new();
        let origin = |path: &[String]| {
            if !origins {
                return String::new();
            }
            match self.origin(path) {
                Some(Origin::Default) => "  # default".to_owned(),
                Some(Origin::Cli) => "  # cli".to_owned(),
                Some(Origin::File { path, line }) => format!("  # {}:{line}", path.display()),
                None => String::new(),
            }
        };
        match &value {
            Value::Dict(_, dict) if !dict.is_empty() => {
                write_entries(&value, 0, &mut path, &origin, &mut out);
            }
            _ => {
                let _ = writeln!(out, "{}", scalar(&value));
            }
        }
        out
    }
}

/// Writes the entries of a dict or the items of an array at `indent`.
fn write_entries(
    value: &Value,
    indent: usize,
    path: &mut Vec<String>,
    origin: &dyn Fn(&[String]) -> String,
    out: &mut String,
) {
    let pad = " ".repeat(indent);
    let children: Vec<(String, &Value)> = match value {
        Value::Dict(_, dict) => dict.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Value::Array(_, items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        _ => return,
    };
    let is_array = matches!(value, Value::Array(..));
    for (segment, child) in children {
        let lead = if is_array {
            "- ".to_owned()
        } else {
            format!("{}: ", key(&segment))
        };
        path.push(segment);
        if is_container(child) {
            if is_array {
                // Compact form: the child's first line goes after the dash.
                let mut nested = String::new();
                write_entries(child, indent + 2, path, origin, &mut nested);
                let first_indent = indent + 2;
                let _ = write!(out, "{pad}- {}", &nested[first_indent..]);
            } else {
                // Only leaves are annotated: a mapping may merge keys from several layers.
                let _ = writeln!(out, "{pad}{}:", key(&path[path.len() - 1]));
                write_entries(child, indent + 2, path, origin, out);
            }
        } else {
            let _ = writeln!(out, "{pad}{lead}{}{}", scalar(child), origin(path));
        }
        path.pop();
    }
}

/// A non-empty dict or array, written in block style.
fn is_container(value: &Value) -> bool {
    match value {
        Value::Dict(_, dict) => !dict.is_empty(),
        Value::Array(_, items) => !items.is_empty(),
        _ => false,
    }
}

/// A scalar (or empty container) in flow notation.
fn scalar(value: &Value) -> String {
    match value {
        Value::String(_, s) => string(s),
        Value::Char(_, c) => string(&c.to_string()),
        Value::Bool(_, b) => b.to_string(),
        Value::Num(_, num) => number(*num),
        Value::Empty(..) => "null".to_owned(),
        Value::Dict(..) => "{}".to_owned(),
        Value::Array(..) => "[]".to_owned(),
    }
}

fn number(num: Num) -> String {
    match num {
        Num::U8(n) => n.to_string(),
        Num::U16(n) => n.to_string(),
        Num::U32(n) => n.to_string(),
        Num::U64(n) => n.to_string(),
        Num::U128(n) => n.to_string(),
        Num::USize(n) => n.to_string(),
        Num::I8(n) => n.to_string(),
        Num::I16(n) => n.to_string(),
        Num::I32(n) => n.to_string(),
        Num::I64(n) => n.to_string(),
        Num::I128(n) => n.to_string(),
        Num::ISize(n) => n.to_string(),
        Num::F32(n) => n.to_string(),
        Num::F64(n) => n.to_string(),
    }
}

fn key(segment: &str) -> String {
    string(segment)
}

/// A string as a plain scalar when that is unambiguous, else double-quoted (JSON string
/// syntax is valid YAML).
fn string(s: &str) -> String {
    const RESERVED: &[&str] = &[
        "true", "false", "null", "yes", "no", "on", "off", "y", "n", "~",
    ];
    let plain = s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
        && !RESERVED.contains(&s.to_ascii_lowercase().as_str());
    if plain {
        s.to_owned()
    } else {
        serde_json::to_string(s).unwrap_or_else(|_| format!("{s:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_quoted_only_when_needed() {
        assert_eq!(string("gfm"), "gfm");
        assert_eq!(string("vendor/x.md"), "vendor/x.md");
        assert_eq!(string("off"), "\"off\"");
        assert_eq!(string("No"), "\"No\"");
        assert_eq!(string("*"), "\"*\"");
        assert_eq!(string("12"), "\"12\"");
        assert_eq!(string(""), "\"\"");
        assert_eq!(string("a: b"), "\"a: b\"");
        assert_eq!(string("say \"hi\""), "\"say \\\"hi\\\"\"");
    }

    #[test]
    fn scalars_render_in_flow_notation() {
        assert_eq!(scalar(&Value::from(true)), "true");
        assert_eq!(scalar(&Value::from(-3i64)), "-3");
        assert_eq!(scalar(&Value::from(1.5f64)), "1.5");
        assert_eq!(scalar(&Value::from('x')), "x");
        assert_eq!(scalar(&Value::Empty(Tag::Default, Empty::Unit)), "null");
        assert_eq!(scalar(&Value::from(Vec::<u8>::new())), "[]");
    }
}
