//! `fmt.frontmatter: normalize`: frontmatter re-serialized canonically, as long as that
//! keeps its data.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use serde_saphyr::granit_parser::{Event, Parser, ScalarStyle};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, RawString, Table};

use crate::ast::{FrontMatter, FrontMatterFormat};
use crate::frontmatter::Frontmatter;

/// The canonical content of `front` (the lines between its delimiters, each ending with
/// a newline), or why it is kept as written: it does not parse, or normalizing would
/// lose something (comments, YAML tags, keys that are not strings).
pub(super) fn normalize(front: &FrontMatter) -> Result<String, String> {
    let (fence, content) = match front.format {
        FrontMatterFormat::Toml => ("+++", toml(&front.literal)?),
        _ => ("---", yaml(&front.literal)?),
    };
    // The data must survive: the normalized block reads back to the same value.
    let value = |content: &str| {
        Frontmatter::extract(&format!("{fence}\n{content}{fence}\n"))
            .and_then(|front| front.value().cloned())
    };
    match value(&front.literal) {
        None => Err("it does not parse".to_owned()),
        Some(before) if value(&content).as_ref() == Some(&before) => Ok(content),
        Some(_) => Err("normalizing it would change its data".to_owned()),
    }
}

/// Canonical YAML: block style, as serde-saphyr writes it.
fn yaml(text: &str) -> Result<String, String> {
    check_yaml(text)?;
    let value: Value = serde_saphyr::from_str(text).map_err(|_| "it does not parse".to_owned())?;
    match value {
        Value::Null => Ok(String::new()),
        Value::Object(map) if map.is_empty() => Ok(String::new()),
        value => serde_saphyr::to_string(&value).map_err(|err| err.to_string()),
    }
}

/// A YAML collection being read: a mapping that expects a key next, or a sequence.
#[derive(Debug, Clone, Copy)]
enum Frame {
    Map { at_key: bool },
    Seq,
}

/// Rejects YAML whose data a JSON value does not hold, or that writing it back would
/// change for some parser: comments, tags, mapping keys that are not strings (`1:`,
/// `null:`, `[a]:`, aliases), and plain scalars that serde-saphyr (which reads YAML 1.1
/// booleans like `yes` and `off`) and YAML 1.2's core schema read differently.
fn check_yaml(text: &str) -> Result<(), String> {
    let mut stack: Vec<Frame> = Vec::new();
    for event in Parser::new_from_str(text) {
        let Ok((event, _)) = event else {
            return Err("it does not parse".to_owned());
        };
        let at_key = matches!(stack.last(), Some(Frame::Map { at_key: true }));
        match event {
            Event::Comment(..) => return Err("normalizing it would drop a comment".to_owned()),
            Event::Scalar(_, _, _, Some(_))
            | Event::MappingStart(_, _, Some(_))
            | Event::SequenceStart(_, _, Some(_)) => {
                return Err("normalizing it would drop a tag".to_owned());
            }
            Event::Scalar(text, ScalarStyle::Plain, ..) => {
                let core_string = is_core_string(&text);
                if at_key && !core_string {
                    return Err(format!("its key `{text}` is not a string"));
                }
                let saphyr_string =
                    matches!(serde_saphyr::from_str::<Value>(&text), Ok(Value::String(_)));
                if !at_key && core_string != saphyr_string {
                    return Err(format!("`{text}` reads differently in YAML 1.1 and 1.2"));
                }
                done(&mut stack);
            }
            Event::Alias(_) if at_key => {
                return Err("an alias as a key is not a string".to_owned());
            }
            Event::Scalar(..) | Event::Alias(_) => done(&mut stack),
            Event::MappingStart(..) | Event::SequenceStart(..) if at_key => {
                return Err("a collection as a key is not a string".to_owned());
            }
            Event::MappingStart(..) => stack.push(Frame::Map { at_key: true }),
            Event::SequenceStart(..) => stack.push(Frame::Seq),
            Event::MappingEnd | Event::SequenceEnd => {
                stack.pop();
                done(&mut stack);
            }
            _ => {}
        }
    }
    Ok(())
}

/// A node of the innermost collection is complete: a mapping expects the other half of
/// its entry next.
fn done(stack: &mut [Frame]) {
    if let Some(Frame::Map { at_key }) = stack.last_mut() {
        *at_key = !*at_key;
    }
}

/// Whether the plain scalar `text` is a string in YAML 1.2's core schema (not `1`,
/// `true`, `null`, …). serde-saphyr reads every key as the string it is written as, and
/// would write `1:` back as `"1":`, which other parsers read differently.
fn is_core_string(text: &str) -> bool {
    static NOT_STRING: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(concat!(
            r"^(?:|~|null|Null|NULL|true|True|TRUE|false|False|FALSE",
            r"|[-+]?[0-9]+|0o[0-7]+|0x[0-9a-fA-F]+",
            r"|[-+]?(?:\.[0-9]+|[0-9]+(?:\.[0-9]*)?)(?:[eE][-+]?[0-9]+)?",
            r"|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$",
        ))
        .expect("valid regex")
    });
    !NOT_STRING.is_match(text)
}

/// Canonical TOML, as `toml_edit` writes a document built from scratch: basic strings,
/// decimal numbers, one space around `=`, inline arrays and tables on one line, a blank
/// line before each table header. Tables keep their kind (header, inline, dotted keys).
fn toml(text: &str) -> Result<String, String> {
    let document: DocumentMut = text.parse().map_err(|_| "it does not parse".to_owned())?;
    if has_comment(document.as_table()) || raw_comment(Some(document.trailing())) {
        return Err("normalizing it would drop a comment".to_owned());
    }
    let mut out = DocumentMut::new();
    *out.as_table_mut() = table(document.as_table());
    let mut text = out.to_string();
    // The first table header gets no blank line before it.
    if text.starts_with('\n') {
        text.remove(0);
    }
    Ok(text)
}

/// `source` rebuilt without its formatting.
fn table(source: &Table) -> Table {
    let mut out = Table::new();
    out.set_implicit(source.is_implicit());
    out.set_dotted(source.is_dotted());
    for (key, item) in source {
        out.insert(key, self::item(item));
    }
    out
}

fn item(item: &Item) -> Item {
    match item {
        Item::None => Item::None,
        Item::Value(value) => Item::Value(self::value(value)),
        Item::Table(t) => Item::Table(table(t)),
        Item::ArrayOfTables(tables) => {
            let mut out = ArrayOfTables::new();
            for t in tables {
                out.push(table(t));
            }
            Item::ArrayOfTables(out)
        }
    }
}

fn value(value: &toml_edit::Value) -> toml_edit::Value {
    use toml_edit::Value as V;
    match value {
        V::String(s) => V::from(s.value().as_str()),
        V::Integer(i) => V::from(*i.value()),
        V::Float(f) => V::from(*f.value()),
        V::Boolean(b) => V::from(*b.value()),
        V::Datetime(d) => V::from(*d.value()),
        V::Array(array) => {
            let mut out: Array = array.iter().map(self::value).collect();
            out.fmt();
            V::Array(out)
        }
        V::InlineTable(t) => {
            let mut out = InlineTable::new();
            out.set_dotted(t.is_dotted());
            for (key, v) in t {
                out.insert(key, self::value(v));
            }
            out.fmt();
            V::InlineTable(out)
        }
    }
}

/// Whether a comment is anywhere in `table`: whitespace and comments are all its decor
/// holds, so a `#` there starts one.
fn has_comment(table: &Table) -> bool {
    let decor = table.decor();
    if raw_comment(decor.prefix()) || raw_comment(decor.suffix()) {
        return true;
    }
    table.iter().any(|(key, item)| {
        table.key(key).is_some_and(|key| {
            let leaf = key.leaf_decor();
            raw_comment(leaf.prefix()) || raw_comment(leaf.suffix())
        }) || item_has_comment(item)
    })
}

fn item_has_comment(item: &Item) -> bool {
    match item {
        Item::None => false,
        Item::Value(value) => value_has_comment(value),
        Item::Table(t) => has_comment(t),
        Item::ArrayOfTables(tables) => tables.iter().any(has_comment),
    }
}

fn value_has_comment(value: &toml_edit::Value) -> bool {
    let decor = value.decor();
    if raw_comment(decor.prefix()) || raw_comment(decor.suffix()) {
        return true;
    }
    match value {
        toml_edit::Value::Array(array) => {
            raw_comment(Some(array.trailing())) || array.iter().any(value_has_comment)
        }
        toml_edit::Value::InlineTable(t) => {
            raw_comment(Some(t.trailing()))
                || t.iter().any(|(key, v)| {
                    t.key(key).is_some_and(|key| {
                        let leaf = key.leaf_decor();
                        raw_comment(leaf.prefix()) || raw_comment(leaf.suffix())
                    }) || value_has_comment(v)
                })
        }
        _ => false,
    }
}

fn raw_comment(raw: Option<&RawString>) -> bool {
    raw.and_then(RawString::as_str)
        .is_some_and(|text| text.contains('#'))
}
