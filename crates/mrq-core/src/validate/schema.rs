//! Frontmatter JSON Schema checks: [`FrontmatterSchemas`].

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use jsonschema::error::{TypeKind, ValidationErrorKind};
use jsonschema::{ValidationError, Validator};
use serde_json::{Map, Value};

use super::codes;
use crate::Document;
use crate::Slashed as _;
use crate::ast::Span;
use crate::config::{LoadedConfig, Origin};
use crate::diag::{Diagnostic, Severity};
use crate::document::normalize;
use crate::frontmatter::Frontmatter;

/// Why a frontmatter schema could not be added.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    /// The `files` glob does not parse.
    #[error("invalid glob `{glob}`: {source}")]
    Glob {
        glob: String,
        #[source]
        source: globset::Error,
    },
    /// The schema file cannot be read.
    #[error("cannot read schema `{}`: {source}", path.slashed())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The schema file is not valid JSON (or YAML, for `.yaml`/`.yml` files).
    #[error("cannot parse schema `{}`: {message}", path.slashed())]
    Parse { path: PathBuf, message: String },
    /// The schema (from the file at `path`, if any) is not a valid JSON Schema, or has a
    /// `$ref` that cannot be resolved.
    #[error("invalid schema{}: {message}", path.as_ref().map(|p| format!(" `{}`", p.slashed())).unwrap_or_default())]
    Invalid {
        path: Option<PathBuf>,
        message: String,
    },
}

/// JSON Schemas for frontmatter, each applied to the documents matching a glob.
///
/// A glob is matched against a document's path relative to the rule's base directory
/// (for config entries: the directory of the config file that declares them), with
/// `/` separators; `*` stays within one path component, `**` crosses them. Every
/// matching schema applies. Documents without a path (stdin) match no glob.
///
/// The draft comes from the schema's `$schema` (draft 4, 6, 7, 2019-09 or 2020-12;
/// 2020-12 without one), and so does whether `format` is checked (jsonschema's
/// per-draft default). `$ref`s can only point inside the schema: nothing is fetched from
/// files or the network. `idn-hostname` and `idn-email` formats are not checked.
///
/// ```
/// use mrq_core::Workspace;
/// use mrq_core::document::DocumentOptions;
/// use mrq_core::source::SourceFile;
/// use mrq_core::validate::{Checker, FrontmatterSchemas, codes};
/// use serde_json::json;
///
/// let mut schemas = FrontmatterSchemas::new();
/// schemas.add(".", "**/*.md", &json!({ "required": ["title"] }))?;
/// let source = SourceFile::new(Some("post.md".into()), "---\ndraft: true\n---\n");
/// let doc = DocumentOptions::new().parse_source(source)?;
/// let ws = Workspace::new(".");
/// let diagnostics = Checker::new(&ws).with_frontmatter_schemas(&schemas).check(&doc);
/// assert_eq!(diagnostics[0].code, codes::FRONTMATTER_SCHEMA);
/// assert_eq!(diagnostics[0].message, "missing required key `title`");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Default)]
pub struct FrontmatterSchemas {
    rules: Vec<Rule>,
}

#[derive(Debug)]
struct Rule {
    /// Absolute and normalized.
    base: PathBuf,
    glob: GlobMatcher,
    /// Where the schema came from, for the diagnostics' help.
    name: Option<PathBuf>,
    validator: Validator,
}

impl FrontmatterSchemas {
    /// No schemas.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether there are no schemas.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Applies `schema` to the documents whose path relative to `base` matches `files`.
    pub fn add(
        &mut self,
        base: impl AsRef<Path>,
        files: &str,
        schema: &Value,
    ) -> Result<(), SchemaError> {
        let validator = compile(schema, None)?;
        self.push(base.as_ref(), files, None, validator)
    }

    /// Applies the schema in the file at `schema` (relative to `base`) to the documents
    /// whose path relative to `base` matches `files`. The file is JSON, or YAML if its
    /// extension is `.yaml` or `.yml`.
    pub fn add_file(
        &mut self,
        base: impl AsRef<Path>,
        files: &str,
        schema: impl AsRef<Path>,
    ) -> Result<(), SchemaError> {
        let base = base.as_ref();
        let path = normalize(&base.join(schema));
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) => return Err(SchemaError::Read { path, source }),
        };
        let yaml = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"));
        let parsed = if yaml {
            serde_saphyr::from_str::<Value>(&text).map_err(|err| err.to_string())
        } else {
            serde_json::from_str::<Value>(&text).map_err(|err| err.to_string())
        };
        let value = match parsed {
            Ok(value) => value,
            Err(message) => return Err(SchemaError::Parse { path, message }),
        };
        let validator = compile(&value, Some(&path))?;
        self.push(base, files, Some(path), validator)
    }

    /// The schemas of `validate.frontmatter`. Each entry's `files` glob and `schema` path
    /// are relative to the directory of the config file that declares the entry.
    pub fn from_config(config: &LoadedConfig) -> Result<Self, SchemaError> {
        let mut schemas = Self::new();
        for (index, entry) in config.config().validate.frontmatter.iter().enumerate() {
            let origin = config.origin(&["validate", "frontmatter", &index.to_string()]);
            let base = match &origin {
                Some(Origin::File { path, .. }) => path.parent().unwrap_or(Path::new("")),
                _ => Path::new(""),
            };
            schemas.add_file(base, &entry.files, &entry.schema)?;
        }
        Ok(schemas)
    }

    fn push(
        &mut self,
        base: &Path,
        files: &str,
        name: Option<PathBuf>,
        validator: Validator,
    ) -> Result<(), SchemaError> {
        let glob = GlobBuilder::new(files)
            .literal_separator(true)
            .build()
            .map_err(|source| SchemaError::Glob {
                glob: files.to_owned(),
                source,
            })?
            .compile_matcher();
        self.rules.push(Rule {
            base: absolute(base),
            glob,
            name,
            validator,
        });
        Ok(())
    }

    /// The schema diagnostics for `document`, in schema order. Malformed frontmatter is
    /// skipped (the parser reports it); a document without frontmatter is checked as an
    /// empty mapping.
    pub fn check(&self, document: &Document) -> Vec<Diagnostic> {
        let Some(path) = document.path() else {
            return Vec::new();
        };
        let path = absolute(path);
        let frontmatter = document.frontmatter();
        let empty = Value::Object(Map::new());
        let instance = match frontmatter {
            Some(frontmatter) => match frontmatter.value() {
                Some(value) => value,
                None => return Vec::new(),
            },
            None => &empty,
        };
        let mut out = Vec::new();
        for rule in &self.rules {
            let Ok(relative) = path.strip_prefix(&rule.base) else {
                continue;
            };
            if !rule.glob.is_match(relative) {
                continue;
            }
            for error in rule.validator.iter_errors(instance) {
                report(document, frontmatter, rule, &error, &mut out);
            }
        }
        out
    }
}

/// Compiles `schema`, read from `path` if any.
fn compile(schema: &Value, path: Option<&Path>) -> Result<Validator, SchemaError> {
    jsonschema::validator_for(schema).map_err(|err| SchemaError::Invalid {
        path: path.map(Path::to_owned),
        message: err.to_string(),
    })
}

/// `path` made absolute against the current directory and normalized, lexically; the
/// empty path is the current directory.
fn absolute(path: &Path) -> PathBuf {
    let path = if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    };
    normalize(&std::path::absolute(path).unwrap_or_else(|_| path.to_owned()))
}

/// Turns one validation error into diagnostics: missing keys at the parent's key (the
/// opening delimiter for the root), unexpected keys at each key, anything else at the
/// offending value.
fn report(
    document: &Document,
    frontmatter: Option<&Frontmatter>,
    rule: &Rule,
    error: &ValidationError<'_>,
    out: &mut Vec<Diagnostic>,
) {
    let pointer = error.instance_path().as_str();
    let prefix = if pointer.is_empty() {
        String::new()
    } else {
        format!("`{}`: ", key_path(pointer))
    };
    let found = || compact(error.instance());
    let mut findings = Vec::new();
    match error.kind() {
        ValidationErrorKind::Required { property } => {
            let key = property
                .as_str()
                .map_or_else(|| compact(property), str::to_owned);
            findings.push((
                key_span(frontmatter, pointer),
                format!("{prefix}missing required key `{key}`"),
            ));
        }
        ValidationErrorKind::AdditionalProperties { unexpected }
        | ValidationErrorKind::UnevaluatedProperties { unexpected } => {
            for key in unexpected {
                let at = format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1"));
                findings.push((
                    key_span(frontmatter, &at),
                    format!("{prefix}unexpected key `{key}`"),
                ));
            }
        }
        ValidationErrorKind::Type { kind } => findings.push((
            value_span(frontmatter, pointer),
            format!(
                "{prefix}expected {}, found {}",
                expected_types(kind),
                type_name(error.instance())
            ),
        )),
        ValidationErrorKind::Enum { options } => {
            let options = match &**options {
                Value::Array(items) => items.iter().map(compact).collect::<Vec<_>>().join(", "),
                other => compact(other),
            };
            findings.push((
                value_span(frontmatter, pointer),
                format!("{prefix}expected one of {options}, found {}", found()),
            ));
        }
        ValidationErrorKind::Constant { expected_value } => findings.push((
            value_span(frontmatter, pointer),
            format!(
                "{prefix}expected {}, found {}",
                compact(expected_value),
                found()
            ),
        )),
        _ => findings.push((value_span(frontmatter, pointer), format!("{prefix}{error}"))),
    }
    for (span, message) in findings {
        let mut diagnostic = Diagnostic::new(
            codes::FRONTMATTER_SCHEMA,
            Severity::Error,
            document.name(),
            span,
            message,
        );
        if let Some(name) = &rule.name {
            diagnostic = diagnostic.with_help(format!("schema `{}`", name.slashed()));
        }
        out.push(diagnostic);
    }
}

/// `value` as compact JSON.
fn compact(value: &Value) -> String {
    value.to_string()
}

/// The span of the key of the entry at `pointer`, or of its value if it has no key (an
/// array item); see [`value_span`] for entries without spans.
fn key_span(frontmatter: Option<&Frontmatter>, pointer: &str) -> Span {
    if pointer.is_empty() {
        return value_span(frontmatter, pointer);
    }
    frontmatter
        .and_then(|fm| fm.entry_at_pointer(pointer))
        .map_or_else(
            || value_span(frontmatter, pointer),
            |entry| entry.key.unwrap_or(entry.value),
        )
}

/// The span of the value at `pointer`, or of its closest ancestor that has one; the
/// opening delimiter for the root, or the start of the document without frontmatter.
fn value_span(frontmatter: Option<&Frontmatter>, pointer: &str) -> Span {
    let Some(frontmatter) = frontmatter else {
        return Span::new(0, 0);
    };
    let mut pointer = pointer;
    while !pointer.is_empty() {
        if let Some(entry) = frontmatter.entry_at_pointer(pointer) {
            return entry.value;
        }
        pointer = &pointer[..pointer.rfind('/').unwrap_or(0)];
    }
    let start = frontmatter.span().start;
    Span::new(start, start + 3)
}

/// A JSON Pointer as dotted keys: `/authors/0/name` → `authors.0.name`.
fn key_path(pointer: &str) -> String {
    pointer
        .split('/')
        .skip(1)
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
        .collect::<Vec<_>>()
        .join(".")
}

/// `string`, or `string or array`, or `null, string or array`.
fn expected_types(kind: &TypeKind) -> String {
    let names: Vec<String> = match kind {
        TypeKind::Single(ty) => vec![ty.to_string()],
        TypeKind::Multiple(set) => set.iter().map(|ty| ty.to_string()).collect(),
    };
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => {
            let mut out = rest.join(", ");
            let _ = write!(out, " or {last}");
            out
        }
        _ => names.concat(),
    }
}

/// The JSON type of `value`, integers apart from other numbers.
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
