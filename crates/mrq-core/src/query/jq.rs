//! The jq front-end: jq programs (jaq) over the [JSON projection](crate::ast::json), with
//! mrq's helper definitions.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use jaq_core::compile::{Filter, Undefined};
use jaq_core::data::HasLut;
use jaq_core::load::{self, Arena, File, Loader, lex, parse};
use jaq_core::native::{self, Fun};
use jaq_core::{Compiler, Ctx, Cv, DataT, Exn, Lut, Native, RunPtr, ValXs, Vars};
use jaq_json::{Map, Num, Val};
use serde_json::Value;

use super::error::{RuntimeError, SyntaxError};
use super::output::section_json;
use super::section::{Section, sections};
use crate::Document;
use crate::ast::NodeId;
use crate::ast::json::project;

/// mrq's helper definitions, on top of jq's standard library and the natives `text` and
/// `sections` ([`funs`]).
const DEFS: &str = r#"
def nodes: ., (if type == "object" and .type == "section" then .heading, .children[]
               else .children[]? end | nodes);
def links: nodes | select(.type == "link");
def code: nodes | select(.type == "code");
def code($lang): code | select((.lang // "" | ascii_downcase) == ($lang | ascii_downcase));
"#;

/// A compiled jq program.
#[derive(Clone)]
pub(super) struct Program {
    filter: Arc<Filter<Native<Data>>>,
}

impl fmt::Debug for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Program")
    }
}

impl PartialEq for Program {
    /// Programs are compared by their source (in [`Query`](super::Query)); the compiled
    /// filters of equal sources are equal.
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Program {}

/// Parses and compiles `source` with jq's standard library and mrq's definitions.
pub(super) fn compile(source: &str) -> Result<Program, SyntaxError> {
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs())
        .chain(load::parse(DEFS, |p| p.defs()).expect("the mrq definitions parse"));
    let arena = Arena::default();
    let modules = Loader::new(defs)
        .load(
            &arena,
            File {
                code: source,
                path: (),
            },
        )
        .map_err(|errors| load_error(source, errors))?;
    let filter = Compiler::default()
        .with_funs(funs())
        .compile(modules)
        .map_err(|errors| compile_error(source, errors))?;
    Ok(Program {
        filter: Arc::new(filter),
    })
}

impl Program {
    /// Runs the program on the projection of `doc`; `query` is its source, for errors.
    pub(super) fn run(&self, query: &str, doc: &Document) -> Result<Vec<Value>, RuntimeError> {
        let input = to_val(serde_json::to_value(project(doc)).expect("the projection serializes"));
        let env = Env {
            lut: &self.filter.lut,
            doc,
            nodes: OnceCell::new(),
            sections: OnceCell::new(),
        };
        let ctx = Ctx::<Data>::new(&env, Vars::new([]));
        self.filter
            .id
            .run((ctx, input))
            .map(|output| {
                output
                    .map(|value| to_json(&value))
                    .map_err(|exn| RuntimeError::new(query, exn_message(exn)))
            })
            .collect()
    }
}

/// The message of a runtime error: a string error value as is, other values as JSON.
fn exn_message(exn: Exn<Val>) -> String {
    match exn.get_err() {
        Ok(err) => match err.into_val() {
            Val::TStr(text) | Val::BStr(text) => String::from_utf8_lossy(&text).into_owned(),
            value => value.to_string(),
        },
        Err(exn) => match exn.get_halt() {
            Ok(code) => format!("halt with exit code {code}"),
            Err(_) => "unexpected break".to_owned(),
        },
    }
}

// --- Errors --------------------------------------------------------------------------

/// The byte offset of `part` in `whole`, or 0 if it is not a slice of it (e.g. a slice of
/// a prelude definition).
fn offset_in(whole: &str, part: &str) -> usize {
    let (start, end) = (
        whole.as_ptr() as usize,
        whole.as_ptr() as usize + whole.len(),
    );
    let at = part.as_ptr() as usize;
    if (start..=end).contains(&at) {
        load::span(whole, part).start
    } else {
        0
    }
}

/// The first error of a failed load (lexing or parsing).
fn load_error(source: &str, errors: load::Errors<&str, ()>) -> SyntaxError {
    let error =
        |at: &str, message: String| SyntaxError::new(source, offset_in(source, at), message);
    let Some((_, first)) = errors.into_iter().next() else {
        return SyntaxError::new(source, 0, "invalid jq program");
    };
    match first {
        load::Error::Io(errors) => errors.into_iter().next().map_or_else(
            || SyntaxError::new(source, 0, "invalid jq program"),
            |(path, message)| error(path, message),
        ),
        load::Error::Lex(errors) => {
            let Some((expect, found)) = errors.into_iter().next() else {
                return SyntaxError::new(source, 0, "invalid jq program");
            };
            match expect {
                lex::Expect::Delim(open) => error(
                    open,
                    format!("unclosed `{open}`, expected {}", expect.as_str()),
                ),
                lex::Expect::Token => {
                    let token = found.chars().next().map(String::from).unwrap_or_default();
                    error(found, format!("unexpected token `{token}`"))
                }
                expect => error(found, format!("expected {}", expect.as_str())),
            }
        }
        load::Error::Parse(errors) => {
            let Some((expect, found)) = errors.into_iter().next() else {
                return SyntaxError::new(source, 0, "invalid jq program");
            };
            let message = match (&expect, found) {
                (parse::Expect::Nothing, found) => format!("unexpected `{found}`"),
                (expect, "") => format!("expected {}", expect.as_str()),
                (expect, found) => format!("expected {}, found `{found}`", expect.as_str()),
            };
            error(found, message)
        }
    }
}

/// The first error of a failed compilation: an undefined name.
fn compile_error(source: &str, errors: jaq_core::compile::Errors<&str, ()>) -> SyntaxError {
    let Some((name, undefined)) = errors.into_iter().flat_map(|(_, errors)| errors).next() else {
        return SyntaxError::new(source, 0, "invalid jq program");
    };
    let message = match undefined {
        Undefined::Filter(arity) => format!("undefined filter `{name}/{arity}`"),
        undefined => format!("undefined {} `{name}`", undefined.as_str()),
    };
    SyntaxError::new(source, offset_in(source, name), message)
}

// --- Data and natives ----------------------------------------------------------------

/// The data the natives see: the document being queried.
struct Data;

impl DataT for Data {
    type V<'a> = Val;
    type Data<'a> = &'a Env<'a>;
}

/// The run-time environment of a program: its look-up table and the document.
struct Env<'a> {
    lut: &'a Lut<Data>,
    doc: &'a Document,
    /// Attached nodes by [`NodeId::index`].
    nodes: OnceCell<HashMap<usize, NodeId>>,
    /// The document's sections and their JSON values, in document order.
    sections: OnceCell<Vec<(Section<'a>, Val)>>,
}

impl<'a> HasLut<'a, Data> for &'a Env<'a> {
    fn lut(&self) -> &'a Lut<Data> {
        self.lut
    }
}

impl<'a> Env<'a> {
    fn node(&self, index: isize) -> Option<NodeId> {
        let nodes = self.nodes.get_or_init(|| {
            let ast = self.doc.ast();
            ast.descendants(ast.root())
                .map(|id| (id.index(), id))
                .collect()
        });
        nodes.get(&usize::try_from(index).ok()?).copied()
    }

    fn sections(&self) -> &[(Section<'a>, Val)] {
        self.sections.get_or_init(|| {
            sections(self.doc.ast())
                .into_iter()
                .map(|section| (section, to_val(section_json(self.doc, section))))
                .collect()
        })
    }

    /// The section that a `section` object stands for.
    fn section_of(&self, value: &Val) -> Option<Section<'a>> {
        let heading = self.node(as_isize(field(field(value, "heading")?, "id")?)?)?;
        self.sections()
            .iter()
            .map(|&(section, _)| section)
            .find(|section| section.heading().id() == heading)
    }

    /// The plain text of a node, a section or a string.
    fn text(&self, value: &Val) -> Option<String> {
        if let Val::TStr(text) = value {
            return Some(String::from_utf8_lossy(text).into_owned());
        }
        if is_section(value) {
            return self.section_of(value).map(Section::text);
        }
        let id = self.node(as_isize(field(value, "id")?)?)?;
        Some(self.doc.ast().text(id))
    }

    /// The sections inside a node or section.
    fn sections_in(&self, value: &Val) -> Option<Vec<Val>> {
        let inside: Box<dyn Fn(Section<'_>) -> bool> = if is_section(value) {
            let outer = self.section_of(value)?;
            let span = outer.span();
            Box::new(move |s| {
                let start = s.heading().span().start;
                s != outer && span.start <= start && start < span.end
            })
        } else {
            let ast = self.doc.ast();
            let node = self.node(as_isize(field(value, "id")?)?)?;
            Box::new(move |s| {
                let heading = s.heading().id();
                heading == node || ast.ancestors(heading).any(|id| id == node)
            })
        };
        Some(
            self.sections()
                .iter()
                .filter(|&&(section, _)| inside(section))
                .map(|(_, value)| value.clone())
                .collect(),
        )
    }
}

/// `value.name` of an object.
fn field<'v>(value: &'v Val, name: &str) -> Option<&'v Val> {
    let Val::Obj(map) = value else { return None };
    map.get(&Val::from(name.to_owned()))
}

fn is_section(value: &Val) -> bool {
    matches!(field(value, "type"), Some(Val::TStr(ty)) if &***ty == b"section")
}

fn as_isize(value: &Val) -> Option<isize> {
    match value {
        Val::Num(num) => num.as_isize(),
        _ => None,
    }
}

/// The natives: `text` and `sections`.
fn funs() -> impl Iterator<Item = Fun<Data>> {
    jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs())
        .chain([
            native::run::<Data>(("text", native::v(0), text as RunPtr<Data>)),
            native::run::<Data>(("sections", native::v(0), sections_native as RunPtr<Data>)),
        ])
}

/// `text`: the plain text of a node or section, or a string as is.
fn text((ctx, value): Cv<'_, Data>) -> ValXs<'_, Val> {
    native::bome(ctx.data().text(&value).map(Val::from).ok_or_else(|| {
        jaq_core::Error::str(format!(
            "text: expected a node, a section or a string, got {value}"
        ))
    }))
}

/// `sections`: the sections inside a node (the whole document for the root) or inside a
/// section, in document order.
fn sections_native((ctx, value): Cv<'_, Data>) -> ValXs<'_, Val> {
    match ctx.data().sections_in(&value) {
        Some(found) => Box::new(found.into_iter().map(Ok)),
        None => native::bome(Err(jaq_core::Error::str(format!(
            "sections: expected a node or a section, got {value}"
        )))),
    }
}

// --- Values --------------------------------------------------------------------------

/// A JSON value as a jq value.
fn to_val(value: Value) -> Val {
    match value {
        Value::Null => Val::Null,
        Value::Bool(b) => Val::Bool(b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Val::Num(Num::from_integral(i))
            } else if let Some(u) = n.as_u64() {
                Val::Num(Num::from_integral(u))
            } else {
                Val::Num(Num::Float(n.as_f64().unwrap_or(f64::NAN)))
            }
        }
        Value::String(s) => Val::from(s),
        Value::Array(items) => items.into_iter().map(to_val).collect(),
        Value::Object(map) => Val::obj(
            map.into_iter()
                .map(|(key, value)| (Val::from(key), to_val(value)))
                .collect::<Map>(),
        ),
    }
}

/// A jq value as JSON, as jq prints it: `nan` is `null`, infinities are the largest
/// finite numbers, byte strings are (lossy) strings and non-string keys are printed.
fn to_json(value: &Val) -> Value {
    match value {
        Val::Null => Value::Null,
        Val::Bool(b) => Value::Bool(*b),
        Val::Num(num) => num_json(num),
        Val::TStr(text) | Val::BStr(text) => {
            Value::String(String::from_utf8_lossy(text).into_owned())
        }
        Val::Arr(items) => items.iter().map(to_json).collect(),
        Val::Obj(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let key = match key {
                        Val::TStr(text) | Val::BStr(text) => {
                            String::from_utf8_lossy(text).into_owned()
                        }
                        key => key.to_string(),
                    };
                    (key, to_json(value))
                })
                .collect(),
        ),
    }
}

fn num_json(num: &Num) -> Value {
    let float = |f: f64| {
        if f.is_nan() {
            Value::Null
        } else {
            serde_json::Number::from_f64(f.clamp(f64::MIN, f64::MAX))
                .map_or(Value::Null, Value::Number)
        }
    };
    match num {
        Num::Int(i) => Value::from(*i as i64),
        Num::Float(f) => float(*f),
        num => {
            let text = num.to_string();
            serde_json::from_str::<serde_json::Number>(&text)
                .map_or_else(|_| float(text.parse().unwrap_or(f64::NAN)), Value::Number)
        }
    }
}
