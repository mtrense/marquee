//! TOML frontmatter: values and spans from one `toml_edit` parse.

use std::collections::HashMap;

use serde_json::{Map, Number, Value};
use toml_edit::{Document, Item, TableLike, Value as TomlValue};

use super::{EntrySpan, FrontmatterError, Index, Parsed, trimmed};
use crate::ast::Span;

/// Parses `text` (the frontmatter content) into a value and its span index. Spans and the
/// error range are relative to `text`.
pub(super) fn parse(text: &str) -> Result<Parsed, FrontmatterError> {
    let document = Document::parse(text).map_err(|err| {
        let range = err.span().map_or(Span::empty(0), Span::from);
        FrontmatterError {
            message: format!("invalid TOML frontmatter: {}", err.message().trim()),
            range,
        }
    })?;
    let mut entries = HashMap::new();
    let root = trimmed(text, Span::new(0, text.len()));
    entries.insert(Vec::new(), EntrySpan::new(None, root));
    let value = table(document.as_table(), &mut Vec::new(), &mut entries);
    Ok(Parsed { value, entries })
}

/// Converts `table` at `path`, indexing its entries.
fn table(table: &dyn TableLike, path: &mut Vec<String>, entries: &mut Index) -> Value {
    let mut object = Map::new();
    for (name, item) in table.iter() {
        let key = table
            .key(name)
            .and_then(toml_edit::Key::span)
            .map(Span::from);
        path.push(name.to_owned());
        if let Some(value) = self::item(item, key, path, entries) {
            object.insert(name.to_owned(), value);
        }
        path.pop();
    }
    Value::Object(object)
}

/// Converts the item at `path` whose key spans `key`, indexing it and its descendants.
fn item(
    item: &Item,
    key: Option<Span>,
    path: &mut Vec<String>,
    entries: &mut Index,
) -> Option<Value> {
    // Implicit tables (`a` in `a.b = 1` or `[a.b]`) have no span of their own.
    let span = item.span().map(Span::from).or(key);
    if let Some(value) = span {
        entries.insert(path.clone(), EntrySpan::new(key, value));
    }
    match item {
        Item::None => None,
        Item::Value(value) => Some(self::value(value, path, entries)),
        Item::Table(t) => Some(table(t, path, entries)),
        Item::ArrayOfTables(array) => {
            let mut items = Vec::new();
            for (index, t) in array.iter().enumerate() {
                path.push(index.to_string());
                if let Some(span) = t.span() {
                    entries.insert(path.clone(), EntrySpan::new(None, Span::from(span)));
                }
                items.push(table(t, path, entries));
                path.pop();
            }
            Some(Value::Array(items))
        }
    }
}

/// Converts an (inline) value, indexing the items and entries inside it.
fn value(value: &TomlValue, path: &mut Vec<String>, entries: &mut Index) -> Value {
    match value {
        TomlValue::String(s) => Value::String(s.value().clone()),
        TomlValue::Integer(i) => Value::from(*i.value()),
        // JSON has no NaN or infinities; keep them as their TOML spelling.
        TomlValue::Float(f) => Number::from_f64(*f.value())
            .map_or_else(|| Value::String(float_name(*f.value())), Value::Number),
        TomlValue::Boolean(b) => Value::Bool(*b.value()),
        TomlValue::Datetime(d) => Value::String(d.value().to_string()),
        TomlValue::Array(array) => {
            let mut items = Vec::new();
            for (index, element) in array.iter().enumerate() {
                path.push(index.to_string());
                if let Some(span) = element.span() {
                    entries.insert(path.clone(), EntrySpan::new(None, Span::from(span)));
                }
                items.push(self::value(element, path, entries));
                path.pop();
            }
            Value::Array(items)
        }
        TomlValue::InlineTable(t) => table(t, path, entries),
    }
}

fn float_name(f: f64) -> String {
    match f {
        f if f.is_nan() => "nan".to_owned(),
        f if f > 0.0 => "inf".to_owned(),
        _ => "-inf".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_convert_to_json() {
        let parsed =
            parse("i = 1\nf = 1.5\nb = true\nd = 1979-05-27T07:32:00Z\nn = nan\nm = -inf\n")
                .unwrap();
        assert_eq!(
            parsed.value,
            serde_json::json!({
                "i": 1, "f": 1.5, "b": true, "d": "1979-05-27T07:32:00Z", "n": "nan", "m": "-inf"
            })
        );
    }

    #[test]
    fn implicit_tables_fall_back_to_their_key_span() {
        let text = "[a.b]\nc = 1\n";
        let parsed = parse(text).unwrap();
        let entry = parsed.entries[&vec![String::from("a")]];
        assert_eq!(entry.key, Some(Span::new(1, 2)));
        assert_eq!(entry.value, Span::new(1, 2));
    }
}
