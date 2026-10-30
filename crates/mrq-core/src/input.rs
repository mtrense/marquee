//! Resolving command-line inputs (files, directories, globs and `-`) into an ordered
//! list of [`Input`]s.
//!
//! Every argument is classified in this order:
//!
//! 1. `-` is stdin. It may appear at most once and is never writable, so commands that
//!    rewrite files print stdin results to stdout instead.
//! 2. An existing directory is walked recursively for `*.md` / `*.markdown` files
//!    (extension compared case-insensitively). The walk respects `.gitignore`,
//!    `.ignore`, `.git/info/exclude` and the global git excludes file, **even outside a
//!    git repository** (`require_git(false)`), skips hidden files and directories, and
//!    does not follow symlinked directories.
//! 3. Any other existing path is an explicit file. It is included whatever its extension
//!    and whether or not an ignore rule matches it: naming a file is an explicit choice.
//! 4. A non-existent path containing glob metacharacters (`*`, `?`, `[`, `{`) is a glob
//!    (`globset` syntax; `*` stays within one path component, `**` crosses them). It is
//!    expanded relative to the base directory by walking from its longest literal
//!    directory prefix with the same ignore rules as a directory walk. Matches are not
//!    limited to Markdown extensions, since the pattern already selects files.
//! 5. Anything else is [`InputError::NotFound`].
//!
//! Extra ignore patterns (the config `ignore` setting) use gitignore syntax relative to
//! the base directory and prune directory walks and glob expansions, not explicit files.
//!
//! Ordering is deterministic: arguments keep their order, each walk or glob expansion
//! is sorted by file name (byte order) depth-first, and a path seen before is dropped.
//! Paths are lexically normalised (`./` and interior `.` components removed) but not
//! canonicalised.
//!
//! Resolution fails fast: the first nonexistent path, glob without matches, invalid
//! pattern or walk error aborts it with an [`InputError`]. A directory that contains no
//! Markdown is not an error. An empty argument list resolves to no inputs; supplying a
//! default (the current directory) is up to the command.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use globset::GlobBuilder;
use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::source::SourceFile;

/// File extensions picked up by directory walks (compared case-insensitively).
const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown"];

/// What an [`Input`] reads from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// A file on disk.
    File,
    /// Standard input (`-`).
    Stdin,
}

/// One resolved input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    path: Option<PathBuf>,
}

impl Input {
    /// A file input.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
        }
    }

    /// The stdin input.
    pub fn stdin() -> Self {
        Self { path: None }
    }

    /// Whether this is a file or stdin.
    pub fn kind(&self) -> InputKind {
        if self.path.is_some() {
            InputKind::File
        } else {
            InputKind::Stdin
        }
    }

    /// The file path, or `None` for stdin.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Whether results may be written back to this input. Stdin never is.
    pub fn is_writable(&self) -> bool {
        self.path.is_some()
    }

    /// Reads the input, taking stdin from the process's standard input.
    pub fn read(&self) -> Result<SourceFile, InputError> {
        self.read_with(&mut std::io::stdin().lock())
    }

    /// Reads the input, taking stdin from `stdin` (injectable for tests). The text must
    /// be valid UTF-8.
    pub fn read_with(&self, stdin: &mut dyn Read) -> Result<SourceFile, InputError> {
        let text = if let Some(path) = &self.path {
            std::fs::read_to_string(path)
        } else {
            let mut text = String::new();
            stdin.read_to_string(&mut text).map(|_| text)
        }
        .map_err(|source| InputError::Read {
            name: self.name(),
            source,
        })?;
        Ok(SourceFile::new(self.path.clone(), text))
    }

    fn name(&self) -> String {
        self.path
            .as_ref()
            .map_or_else(|| "<stdin>".to_owned(), |p| p.display().to_string())
    }
}

/// Why inputs could not be resolved or read.
#[derive(Debug, thiserror::Error)]
pub enum InputError {
    /// A path argument that exists neither as a file nor a directory and is no glob.
    #[error("{}: no such file or directory", path.display())]
    NotFound { path: PathBuf },
    /// A glob argument that matched no file.
    #[error("no files match the glob pattern `{pattern}`")]
    NoMatches { pattern: String },
    /// A glob argument that is not a valid pattern.
    #[error("invalid glob pattern `{pattern}`: {source}")]
    InvalidGlob {
        pattern: String,
        source: globset::Error,
    },
    /// An extra ignore pattern that is not valid gitignore syntax.
    #[error("invalid ignore pattern `{pattern}`: {source}")]
    InvalidIgnorePattern {
        pattern: String,
        source: ignore::Error,
    },
    /// `-` given more than once.
    #[error("`-` (stdin) can be given only once")]
    DuplicateStdin,
    /// An error while walking a directory (e.g. an unreadable directory).
    #[error(transparent)]
    Walk(#[from] ignore::Error),
    /// An input that could not be read (missing, unreadable, or not UTF-8).
    #[error("cannot read {name}: {source}")]
    Read {
        name: String,
        source: std::io::Error,
    },
}

/// Resolves input arguments into [`Input`]s; see the [module docs](self) for the rules.
#[derive(Debug, Clone, Default)]
pub struct InputResolver {
    base_dir: PathBuf,
    ignore_patterns: Vec<String>,
}

impl InputResolver {
    /// A resolver relative to the process's current directory, without extra ignores.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolves relative arguments against `dir` instead of the current directory.
    /// Resulting paths are `dir` joined with the relative path. By default the base is
    /// empty, so paths come out as written.
    #[must_use]
    pub fn base_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.base_dir = dir.into();
        self
    }

    /// Adds gitignore-syntax patterns, relative to the base directory, that prune
    /// directory walks and glob expansions (the config `ignore` setting).
    #[must_use]
    pub fn ignore_patterns<I, S>(mut self, patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.ignore_patterns
            .extend(patterns.into_iter().map(Into::into));
        self
    }

    /// Resolves `args` in order into a deduplicated list of inputs.
    pub fn resolve<S: AsRef<Path>>(&self, args: &[S]) -> Result<Vec<Input>, InputError> {
        let ignore = Arc::new(self.ignore_matcher()?);
        let mut out = Inputs::default();
        for arg in args {
            let arg = arg.as_ref();
            if arg == Path::new("-") {
                if !out.push_stdin() {
                    return Err(InputError::DuplicateStdin);
                }
                continue;
            }
            let path = normalize(&self.base_dir.join(arg));
            if path.is_dir() {
                for file in walk(&path, &ignore)? {
                    if has_markdown_extension(&file) {
                        out.push(file);
                    }
                }
            } else if path.exists() {
                out.push(path);
            } else if let Some(pattern) = arg.to_str().filter(|s| is_glob(s)) {
                self.expand_glob(pattern, &ignore, &mut out)?;
            } else {
                return Err(InputError::NotFound { path });
            }
        }
        Ok(out.list)
    }

    fn ignore_matcher(&self) -> Result<Gitignore, InputError> {
        let root = if self.base_dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &self.base_dir
        };
        let mut builder = GitignoreBuilder::new(root);
        for pattern in &self.ignore_patterns {
            builder
                .add_line(None, pattern)
                .map_err(|source| InputError::InvalidIgnorePattern {
                    pattern: pattern.clone(),
                    source,
                })?;
        }
        Ok(builder.build()?)
    }

    fn expand_glob(
        &self,
        pattern: &str,
        ignore: &Arc<Gitignore>,
        out: &mut Inputs,
    ) -> Result<(), InputError> {
        let invalid = |source| InputError::InvalidGlob {
            pattern: pattern.to_owned(),
            source,
        };
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(invalid)?
            .compile_matcher();
        let absolute = Path::new(pattern).is_absolute();
        let root = normalize(&self.base_dir.join(literal_prefix(Path::new(pattern))));
        let root = if root.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            root
        };
        let mut matched = false;
        if root.is_dir() {
            for file in walk(&root, ignore)? {
                let candidate = if absolute {
                    &file
                } else {
                    file.strip_prefix(&self.base_dir).unwrap_or(&file)
                };
                if glob.is_match(candidate) {
                    matched = true;
                    out.push(file);
                }
            }
        }
        if matched {
            Ok(())
        } else {
            Err(InputError::NoMatches {
                pattern: pattern.to_owned(),
            })
        }
    }
}

/// The ordered, deduplicated result being built.
#[derive(Default)]
struct Inputs {
    list: Vec<Input>,
    seen: HashSet<PathBuf>,
    stdin: bool,
}

impl Inputs {
    fn push(&mut self, path: PathBuf) {
        if self.seen.insert(path.clone()) {
            self.list.push(Input::file(path));
        }
    }

    /// Adds stdin; `false` if it was already present.
    fn push_stdin(&mut self) -> bool {
        if self.stdin {
            return false;
        }
        self.stdin = true;
        self.list.push(Input::stdin());
        true
    }
}

/// All files under the directory `root`, sorted depth-first by file name, filtered by the standard
/// ignore files, hidden-file rules and `extra` patterns. Paths are normalised.
fn walk(root: &Path, extra: &Arc<Gitignore>) -> Result<Vec<PathBuf>, InputError> {
    let extra = Arc::clone(extra);
    let walker = WalkBuilder::new(root)
        .require_git(false)
        .sort_by_file_name(Ord::cmp)
        .filter_entry(move |entry| {
            entry.depth() == 0
                || !extra
                    .matched(entry.path(), entry.file_type().is_some_and(|t| t.is_dir()))
                    .is_ignore()
        })
        .build();
    let mut files = Vec::new();
    for entry in walker {
        let entry = entry?;
        let is_file = entry
            .file_type()
            .is_some_and(|t| t.is_file() || (t.is_symlink() && entry.path().is_file()));
        if is_file {
            files.push(normalize(entry.path()));
        }
    }
    Ok(files)
}

fn has_markdown_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            MARKDOWN_EXTENSIONS
                .iter()
                .any(|md| ext.eq_ignore_ascii_case(md))
        })
}

fn is_glob(arg: &str) -> bool {
    arg.contains(['*', '?', '[', '{'])
}

/// The leading components of a glob pattern that contain no glob metacharacters.
fn literal_prefix(pattern: &Path) -> PathBuf {
    pattern
        .components()
        .take_while(|c| !matches!(c, Component::Normal(s) if s.to_str().is_some_and(is_glob)))
        .collect()
}

/// Removes `.` components (keeping a lone `.`), without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let normalized: PathBuf = path
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    if normalized.as_os_str().is_empty() && !path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}
