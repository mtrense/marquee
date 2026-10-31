//! Loading themes: built-in themes, YAML files, `extends` chains and path resolution.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::model::Theme;

/// The names of the built-in themes, in the order they are documented.
pub const BUILTIN_THEMES: [&str; 4] = ["light", "dark", "github", "paper"];

/// The YAML sources of the built-in themes defined as files; `light` is
/// [`Theme::default`] and has none.
const SOURCES: [(&str, &str); 3] = [
    ("dark", include_str!("builtin/dark.yaml")),
    ("github", include_str!("builtin/github.yaml")),
    ("paper", include_str!("builtin/paper.yaml")),
];

/// The built-in themes that are written as YAML (all but `light`), with their source.
pub fn builtin_sources() -> impl Iterator<Item = (&'static str, &'static str)> {
    SOURCES.into_iter()
}

/// Loading a theme failed.
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    /// A theme file could not be read.
    #[error("cannot read theme file {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// A theme is not valid YAML, not a mapping, has an unknown key or a bad value.
    #[error("{}invalid theme: {message}", Prefix(path.as_deref(), *line, *column))]
    Invalid {
        /// The theme file; `None` for a theme given as text.
        path: Option<PathBuf>,
        /// 1-based line.
        line: usize,
        /// 1-based column.
        column: usize,
        message: String,
    },
    /// `extends` (or `--style`) names neither a built-in theme nor a file.
    #[error(
        "unknown theme `{name}`; the built-in themes are {}, and a theme file path needs a `/` or an extension",
        BUILTIN_THEMES.join(", ")
    )]
    Unknown { name: String },
    /// A theme file extends itself, directly or through other files.
    #[error("theme extends itself: {}", Chain(chain))]
    Cycle {
        /// The canonical paths of the chain, starting and ending with the same file.
        chain: Vec<PathBuf>,
    },
}

/// `path:line:column: ` or `line:column: `.
struct Prefix<'a>(Option<&'a Path>, usize, usize);

impl fmt::Display for Prefix<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = self.0 {
            write!(f, "{}:", path.display())?;
        }
        write!(f, "{}:{}: ", self.1, self.2)
    }
}

/// Paths joined by ` -> `.
struct Chain<'a>(&'a [PathBuf]);

impl fmt::Display for Chain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, path) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(" -> ")?;
            }
            write!(f, "{}", path.display())?;
        }
        Ok(())
    }
}

impl Theme {
    /// The built-in theme `name` (one of [`BUILTIN_THEMES`]).
    pub fn builtin(name: &str) -> Option<Self> {
        if name == "light" {
            return Some(Self::default());
        }
        let (_, text) = SOURCES.iter().find(|(builtin, _)| *builtin == name)?;
        let theme = Loader::default().text(text, None, Path::new(""));
        Some(theme.unwrap_or_else(|err| panic!("built-in theme {name} is invalid: {err}")))
    }

    /// A `--style` / `render.style` value: a built-in theme name, or else a theme file
    /// path relative to `dir`. A value without a path separator or a `.` is a name.
    pub fn resolve(style: &str, dir: &Path) -> Result<Self, ThemeError> {
        Loader::default().reference(style, dir)
    }

    /// Loads the theme file at `path`, with its `extends` chain.
    pub fn load(path: &Path) -> Result<Self, ThemeError> {
        Loader::default().file(path)
    }

    /// Loads a theme from YAML text; relative `extends` and other paths in it resolve
    /// against `dir`.
    pub fn from_yaml(text: &str, dir: &Path) -> Result<Self, ThemeError> {
        Loader::default().text(text, None, dir)
    }
}

/// Whether an `extends`/`--style` value is a theme name rather than a path.
fn is_name(reference: &str) -> bool {
    !reference.contains(['/', '\\', '.'])
}

/// Follows an `extends` chain, remembering the files on it.
#[derive(Default)]
struct Loader {
    chain: Vec<PathBuf>,
}

impl Loader {
    /// The theme a name or path refers to.
    fn reference(&mut self, reference: &str, dir: &Path) -> Result<Theme, ThemeError> {
        if is_name(reference) {
            Theme::builtin(reference).ok_or_else(|| ThemeError::Unknown {
                name: reference.to_owned(),
            })
        } else {
            self.file(&dir.join(reference))
        }
    }

    fn file(&mut self, path: &Path) -> Result<Theme, ThemeError> {
        let io = |source| ThemeError::Io {
            path: path.to_owned(),
            source,
        };
        let canonical = fs::canonicalize(path).map_err(io)?;
        if self.chain.contains(&canonical) {
            let mut chain = std::mem::take(&mut self.chain);
            chain.push(canonical);
            return Err(ThemeError::Cycle { chain });
        }
        let text = fs::read_to_string(&canonical).map_err(io)?;
        self.chain.push(canonical.clone());
        let dir = canonical.parent().unwrap_or(Path::new(""));
        let theme = self.text(&text, Some(&canonical), dir)?;
        self.chain.pop();
        Ok(theme)
    }

    /// Validates one theme file's text, then merges it over the theme it extends.
    fn text(&mut self, text: &str, path: Option<&Path>, dir: &Path) -> Result<Theme, ThemeError> {
        let mut layer = parse(text, path)?;
        let base = match layer.remove("extends") {
            Some(Value::String(reference)) => self.reference(&reference, dir)?,
            _ => Theme::default(),
        };
        rebase_paths(&mut layer, dir);
        let mut merged = serde_json::to_value(&base).unwrap_or_default();
        merge(&mut merged, Value::Object(layer));
        let mut theme: Theme = serde_json::from_value(merged).map_err(|err| invalid(path, err))?;
        theme.extends = None;
        Ok(theme)
    }
}

/// The keys a theme file sets, after checking it against [`Theme`] for an error with a
/// position.
fn parse(text: &str, path: Option<&Path>) -> Result<Map<String, Value>, ThemeError> {
    let error = |err: serde_saphyr::Error| {
        let (line, column) = err.location().map_or((1, 1), |loc| {
            let at = |n: u64| usize::try_from(n).unwrap_or(usize::MAX).max(1);
            (at(loc.line()), at(loc.column()))
        });
        let message = err
            .to_string()
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();
        ThemeError::Invalid {
            path: path.map(Path::to_owned),
            line,
            column,
            message,
        }
    };
    let layer = match serde_saphyr::from_str::<Option<Value>>(text).map_err(error)? {
        None | Some(Value::Null) => return Ok(Map::new()),
        Some(Value::Object(layer)) => layer,
        Some(_) => {
            return Err(ThemeError::Invalid {
                path: path.map(Path::to_owned),
                line: 1,
                column: 1,
                message: "the top level must be a mapping of keys to values".to_owned(),
            });
        }
    };
    serde_saphyr::from_str::<Theme>(text).map_err(error)?;
    Ok(layer)
}

/// An error without a position, from the merged theme.
fn invalid(path: Option<&Path>, err: impl fmt::Display) -> ThemeError {
    ThemeError::Invalid {
        path: path.map(Path::to_owned),
        line: 1,
        column: 1,
        message: err.to_string(),
    }
}

/// Makes the relative paths a theme file sets (overrides, a `.tmTheme` code theme)
/// relative to `dir`, so they keep pointing at the same file when another file extends
/// this one.
fn rebase_paths(layer: &mut Map<String, Value>, dir: &Path) {
    let rebase = |value: &mut Value| {
        if let Value::String(path) = value
            && !Path::new(path.as_str()).has_root()
        {
            *path = dir.join(path.as_str()).display().to_string();
        }
    };
    if let Some(Value::Object(overrides)) = layer.get_mut("overrides") {
        overrides.values_mut().for_each(rebase);
    }
    if let Some(Value::Object(code)) = layer.get_mut("code")
        && let Some(theme) = code.get_mut("theme")
        && theme.as_str().is_some_and(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("tmTheme"))
        })
    {
        rebase(theme);
    }
}

/// Merges `layer` over `base`: mappings key by key, recursively; anything else (strings,
/// numbers, lists, `null`) replaces the base value.
fn merge(base: &mut Value, layer: Value) {
    match (base, layer) {
        (Value::Object(base), Value::Object(layer)) => {
            for (key, value) in layer {
                match base.get_mut(&key) {
                    Some(slot) => merge(slot, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, layer) => *base = layer,
    }
}
