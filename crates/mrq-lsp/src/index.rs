//! [`WorkspaceIndex`]: every Markdown file under the root folders, parsed, with its
//! anchors and links.
//!
//! Each root folder has its own configuration (loaded as the CLI loads it from that
//! directory), [`Workspace`] and walk of its Markdown files (honouring `.gitignore`,
//! hidden files and the config's `ignore`, as directory inputs of the CLI do). Open
//! documents are indexed with the client's text instead of the file's, also when they
//! are ignored. A file outside every root is indexed while it is open, in a workspace of
//! its own directory that is not walked.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use mrq_core::config::{
    CONFIG_FILE_NAMES, Config, ConfigError, ConfigLoader, LoadedConfig, MARKDOWNLINT_FILE_NAMES,
    Origin,
};
use mrq_core::document::{Document, DocumentOptions, Workspace};
use mrq_core::input::{InputResolver, has_markdown_extension};
use mrq_core::lint::{Linter, Registry};
use mrq_core::source::SourceFile;
use mrq_core::validate::{self, DocumentLink, FrontmatterSchemas, Target};
use serde::{Deserialize, Serialize};
use tower_lsp_server::ls_types::{FileChangeType, Uri};

use crate::ServerOptions;
use crate::convert::path_to_uri;
use crate::store::DocumentStore;

/// One root folder, or the directory of an open file outside every root.
#[derive(Debug)]
pub struct Root {
    path: PathBuf,
    /// `None` if the configuration could not be loaded: the defaults are used.
    config: Option<LoadedConfig>,
    /// The configured lint rules; `None` if the `lint` settings are invalid.
    linter: Option<Linter>,
    /// The configured frontmatter schemas, without those that fail to load.
    schemas: FrontmatterSchemas,
    workspace: Workspace,
    /// The files of the last walk; empty for a loose root, which is never walked.
    walked: HashSet<PathBuf>,
    /// The links of each document, by its path.
    links: Vec<(PathBuf, Vec<DocumentLink>)>,
}

impl Root {
    /// The root directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The configuration in effect, `None` if it could not be loaded (then the
    /// defaults apply).
    pub const fn config(&self) -> Option<&LoadedConfig> {
        self.config.as_ref()
    }

    /// The lint rules of the configuration, `None` if its `lint` settings are invalid.
    pub const fn linter(&self) -> Option<&Linter> {
        self.linter.as_ref()
    }

    /// The frontmatter schemas of the configuration (`validate.frontmatter`).
    pub const fn schemas(&self) -> &FrontmatterSchemas {
        &self.schemas
    }

    /// The documents under the root.
    pub const fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// The links of the document at `path`, as [`validate::links`] finds them.
    pub fn links(&self, path: &Path) -> &[DocumentLink] {
        self.links
            .iter()
            .find(|(p, _)| p == path)
            .map_or(&[], |(_, links)| links)
    }

    /// A root with its configuration loaded, and the problems with it.
    fn new(path: PathBuf, options: &ServerOptions) -> (Self, Vec<String>) {
        let (config, mut errors) = match load_config(&path, options) {
            Ok(config) => (Some(config), Vec::new()),
            Err(err) => (None, vec![err.to_string()]),
        };
        let mut root = Self {
            workspace: Workspace::new(&path),
            path,
            config: None,
            linter: None,
            schemas: FrontmatterSchemas::default(),
            walked: HashSet::new(),
            links: Vec::new(),
        };
        errors.extend(root.configure(config, options));
        (root, errors)
    }

    /// Applies `config` (the defaults if `None`): how documents are parsed and checked.
    /// The workspace starts empty; [`rebuild`](Self::rebuild) reads the files again.
    /// Returns the settings that fail to load, which are left out.
    fn configure(&mut self, config: Option<LoadedConfig>, options: &ServerOptions) -> Vec<String> {
        let mut errors = Vec::new();
        let defaults = Config::default();
        let lint = config.as_ref().map_or(&defaults.lint, |c| &c.config().lint);
        self.linter = Linter::new(&Registry::builtin(), lint)
            .inspect_err(|err| errors.push(err.to_string()))
            .ok();
        self.schemas = match &config {
            Some(config) => FrontmatterSchemas::from_config(config).unwrap_or_else(|err| {
                errors.push(err.to_string());
                FrontmatterSchemas::default()
            }),
            None => FrontmatterSchemas::default(),
        };
        let document_options = document_options(config.as_ref(), options);
        self.workspace = Workspace::with_options(&self.path, document_options);
        self.config = config;
        errors
    }

    /// The Markdown files under the root, by the configured ignore rules.
    fn walk(&self) -> Result<HashSet<PathBuf>, String> {
        let ignore = self
            .config
            .as_ref()
            .map(|config| config.config().ignore.clone())
            .unwrap_or_default();
        let inputs = InputResolver::new()
            .base_dir(&self.path)
            .ignore_patterns(ignore)
            .resolve(&["."])
            .map_err(|err| format!("{}: {err}", self.path.display()))?;
        Ok(inputs
            .iter()
            .filter_map(|input| input.path().map(Path::to_path_buf))
            .collect())
    }

    /// Re-reads every file: the walked ones and the open ones under the root.
    fn rebuild(&mut self, store: &DocumentStore, open: &[PathBuf]) -> Option<String> {
        let (walked, error) = match self.walk() {
            Ok(walked) => (walked, None),
            Err(err) => (HashSet::new(), Some(err)),
        };
        let options = self.workspace.options().clone();
        self.workspace = Workspace::with_options(&self.path, options);
        self.links.clear();
        let mut paths: Vec<&PathBuf> = walked.iter().chain(open).collect();
        paths.sort();
        paths.dedup();
        for path in paths {
            self.load(path, store);
        }
        self.walked = walked;
        error
    }

    /// Adds the files a new walk finds that are not indexed yet.
    fn add_new(&mut self, store: &DocumentStore) -> Option<String> {
        let walked = match self.walk() {
            Ok(walked) => walked,
            Err(err) => return Some(err),
        };
        let mut new: Vec<&PathBuf> = walked
            .iter()
            .filter(|path| self.workspace.get(path).is_none())
            .collect();
        new.sort();
        for path in new {
            self.load(path, store);
        }
        self.walked = walked;
        None
    }

    /// Indexes `path` with its open text, else the file's; skips unreadable files.
    fn load(&mut self, path: &Path, store: &DocumentStore) {
        let source = match store.get_path(path) {
            Some(open) => open.source.clone(),
            None => match std::fs::read_to_string(path) {
                Ok(text) => SourceFile::new(Some(path.to_path_buf()), text),
                Err(_) => return,
            },
        };
        self.insert(source);
    }

    /// Parses and indexes `source`, replacing the document with its path.
    fn insert(&mut self, source: SourceFile) {
        let Some(path) = source.path().map(Path::to_path_buf) else {
            return;
        };
        let document = parse(self.workspace.options(), source);
        let links = validate::links(&document, &self.workspace);
        self.workspace.insert(document);
        match self.links.iter_mut().find(|(p, _)| *p == path) {
            Some((_, old)) => *old = links,
            None => self.links.push((path, links)),
        }
    }

    fn remove(&mut self, path: &Path) {
        self.workspace.remove(path);
        self.links.retain(|(p, _)| p != path);
    }
}

/// The parsed Markdown files of the root folders, and of open files outside them.
///
/// The [server](crate::Backend) keeps it current: [`update`](Self::update) for opened
/// and changed documents, [`close`](Self::close), [`file_changed`](Self::file_changed)
/// for watched files and [`reload_config`](Self::reload_config) when a config file
/// changes. Methods that read files take the [`DocumentStore`], whose text wins over the
/// file's. Configuration problems come back as messages for the user.
#[derive(Debug, Default)]
pub struct WorkspaceIndex {
    options: ServerOptions,
    roots: Vec<Root>,
    /// Roots for open files outside every root folder, one per directory.
    loose: Vec<Root>,
}

impl WorkspaceIndex {
    pub fn new(options: ServerOptions) -> Self {
        Self {
            options,
            roots: Vec::new(),
            loose: Vec::new(),
        }
    }

    /// The root folders, in the order they were added.
    pub fn roots(&self) -> &[Root] {
        &self.roots
    }

    /// Adds the root folder `path` (unless present) and indexes its files.
    pub fn add_root(&mut self, path: PathBuf, store: &DocumentStore) -> Vec<String> {
        if self.roots.iter().any(|root| root.path == path) {
            return Vec::new();
        }
        let (root, mut errors) = Root::new(path, &self.options);
        self.roots.push(root);
        errors.extend(self.rebuild(self.roots.len() - 1, store));
        self.relocate_loose(store);
        errors
    }

    /// Removes the root folder `path`; its open documents stay indexed.
    pub fn remove_root(&mut self, path: &Path, store: &DocumentStore) {
        self.roots.retain(|root| root.path != path);
        self.relocate_loose(store);
    }

    /// The root that `path` belongs to: the innermost root folder containing it, else
    /// the loose root of its directory.
    pub fn root_of(&self, path: &Path) -> Option<&Root> {
        match self.folder_of(path) {
            Some(i) => Some(&self.roots[i]),
            None => self
                .loose
                .iter()
                .find(|root| Some(&*root.path) == path.parent()),
        }
    }

    /// The indexed document at `path`.
    pub fn document(&self, path: &Path) -> Option<&Document> {
        self.root_of(path)?.workspace.get(path)
    }

    /// The links of the document at `path`.
    pub fn links(&self, path: &Path) -> &[DocumentLink] {
        self.root_of(path).map_or(&[], |root| root.links(path))
    }

    /// Every indexed document with its root, root folders first.
    pub fn documents(&self) -> impl Iterator<Item = (&Root, &Document)> {
        self.roots
            .iter()
            .chain(&self.loose)
            .flat_map(|root| root.workspace.documents().map(move |doc| (root, doc)))
    }

    /// Indexes `source` (an opened or changed document) under its root.
    pub fn update(&mut self, source: SourceFile) {
        let Some(path) = source.path().map(Path::to_path_buf) else {
            return;
        };
        if let Some(i) = self.folder_of(&path) {
            self.roots[i].insert(source);
            return;
        }
        let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
        let i = self
            .loose
            .iter()
            .position(|root| root.path == dir)
            .unwrap_or_else(|| {
                // A problem with the configuration of a loose file is not reported: the
                // defaults are good enough for it.
                self.loose.push(Root::new(dir, &self.options).0);
                self.loose.len() - 1
            });
        self.loose[i].insert(source);
    }

    /// The document at `path` was closed: it is indexed from the file again if the root
    /// walks it, else dropped.
    pub fn close(&mut self, path: &Path, store: &DocumentStore) {
        if let Some(i) = self.folder_of(path) {
            let root = &mut self.roots[i];
            if root.walked.contains(path) {
                root.remove(path);
                root.load(path, store);
            } else {
                root.remove(path);
            }
        } else {
            for root in &mut self.loose {
                root.remove(path);
            }
            self.loose.retain(|root| !root.workspace.is_empty());
        }
    }

    /// A watched file changed on disk. Open documents are left alone: their text is the
    /// client's. A created file is picked up by walking its root again, so ignore rules
    /// apply; a deleted path may be a directory.
    pub fn file_changed(
        &mut self,
        path: &Path,
        change: FileChangeType,
        store: &DocumentStore,
    ) -> Vec<String> {
        let Some(i) = self.folder_of(path) else {
            if change == FileChangeType::DELETED {
                return Vec::new();
            }
            for root in &mut self.loose {
                if root.workspace.get(path).is_some() && store.get_path(path).is_none() {
                    root.load(path, store);
                }
            }
            return Vec::new();
        };
        let root = &mut self.roots[i];
        if change == FileChangeType::CREATED {
            return root.add_new(store).into_iter().collect();
        }
        if change == FileChangeType::DELETED {
            let gone: Vec<PathBuf> = root
                .workspace
                .documents()
                .filter_map(Document::path)
                .filter(|p| p.starts_with(path) && store.get_path(p).is_none())
                .map(Path::to_path_buf)
                .collect();
            for p in gone {
                root.remove(&p);
                root.walked.remove(&p);
            }
        } else if root.workspace.get(path).is_some() && store.get_path(path).is_none() {
            root.load(path, store);
        }
        Vec::new()
    }

    /// Reloads the configuration of every root folder and re-indexes them. A root whose
    /// configuration fails to load keeps the previous one; the errors are returned.
    pub fn reload_config(&mut self, store: &DocumentStore) -> Vec<String> {
        let mut errors = Vec::new();
        for i in 0..self.roots.len() {
            let root = &mut self.roots[i];
            match load_config(&root.path, &self.options) {
                Ok(config) => errors.extend(root.configure(Some(config), &self.options)),
                Err(err) => {
                    errors.push(err.to_string());
                    continue;
                }
            }
            errors.extend(self.rebuild(i, store));
        }
        errors
    }

    /// Re-reads root folder `i`, with the open documents under it.
    fn rebuild(&mut self, i: usize, store: &DocumentStore) -> Option<String> {
        let open: Vec<PathBuf> = store
            .iter()
            .filter_map(|(_, doc)| doc.source.path())
            .filter(|path| self.folder_of(path) == Some(i))
            .map(Path::to_path_buf)
            .collect();
        self.roots[i].rebuild(store, &open)
    }

    /// Puts every open document outside the root folders into a loose root again.
    fn relocate_loose(&mut self, store: &DocumentStore) {
        self.loose.clear();
        let outside: Vec<SourceFile> = store
            .iter()
            .map(|(_, doc)| &doc.source)
            .filter(|source| source.path().is_some_and(|p| self.folder_of(p).is_none()))
            .cloned()
            .collect();
        for source in outside {
            self.update(source);
        }
    }

    /// The innermost root folder containing `path`.
    fn folder_of(&self, path: &Path) -> Option<usize> {
        self.roots
            .iter()
            .enumerate()
            .filter(|(_, root)| path.starts_with(&root.path))
            .max_by_key(|(_, root)| root.path.components().count())
            .map(|(i, _)| i)
    }

    /// What the index holds, for [`INDEX_METHOD`](crate::INDEX_METHOD).
    pub fn report(&self, store: &DocumentStore) -> IndexReport {
        let roots = self
            .roots
            .iter()
            .filter_map(|root| {
                Some(RootReport {
                    uri: path_to_uri(&root.path)?,
                    config_files: root
                        .config
                        .as_ref()
                        .map(LoadedConfig::files)
                        .unwrap_or_default(),
                    flavor: root.workspace.options().flavor_name().map(str::to_owned),
                })
            })
            .collect();
        let mut documents: Vec<DocumentReport> = self
            .documents()
            .filter_map(|(root, doc)| {
                let path = doc.path()?;
                Some(DocumentReport {
                    uri: path_to_uri(path)?,
                    version: store.get_path(path).map(|open| open.version),
                    flavor: doc.flavor().name().to_owned(),
                    anchors: doc.anchors().iter().map(|(_, a)| a.to_owned()).collect(),
                    links: root
                        .links(path)
                        .iter()
                        .map(|link| describe(&link.target, &root.path))
                        .collect(),
                })
            })
            .collect();
        documents.sort_by(|a, b| a.uri.cmp(&b.uri));
        IndexReport { roots, documents }
    }
}

/// Whether `path` is a file that configures mrq: `.mrq.yaml` or a markdownlint file.
pub fn is_config_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            CONFIG_FILE_NAMES.contains(&name) || MARKDOWNLINT_FILE_NAMES.contains(&name)
        })
}

/// Whether a watched change of `path` concerns the index: a Markdown file, or a
/// directory (which may have held some).
pub fn is_indexed_kind(path: &Path) -> bool {
    has_markdown_extension(path) || path.extension().is_none()
}

/// The configuration for `root`, layered as the CLI does from the current directory.
fn load_config(root: &Path, options: &ServerOptions) -> Result<LoadedConfig, ConfigError> {
    ConfigLoader::new(root)
        .user_config(options.user_config.clone())
        .explicit(options.config.clone())
        .flavor(options.flavor)
        .load()
}

/// How documents of a root are parsed: the server's flavor, else the configured one
/// (if a file sets it), else the frontmatter's or the default.
fn document_options(config: Option<&LoadedConfig>, options: &ServerOptions) -> DocumentOptions {
    let document_options = DocumentOptions::new();
    if let Some(flavor) = options.flavor {
        return document_options.flavor(flavor.as_str());
    }
    match config {
        Some(config) if matches!(config.origin(&["flavor"]), Some(Origin::File { .. })) => {
            document_options.config_flavor(config.config().flavor.as_str())
        }
        _ => document_options,
    }
}

/// `source` parsed with `options`, or with the defaults if its flavor is unknown.
fn parse(options: &DocumentOptions, source: SourceFile) -> Document {
    options.parse_source(source.clone()).unwrap_or_else(|_| {
        DocumentOptions::new()
            .parse_source(source)
            .expect("the default flavor")
    })
}

/// A link target as text: a file relative to `root`, `#anchor`, the URL, `[[wikilink]]`.
fn describe(target: &Target, root: &Path) -> String {
    match target {
        Target::Anchor(anchor) => format!("#{anchor}"),
        Target::External(url) => url.clone(),
        Target::Mailto(address) => format!("mailto:{address}"),
        Target::File { path, anchor } => {
            let path = path.strip_prefix(root).unwrap_or(path);
            let path = path.to_string_lossy().replace('\\', "/");
            match anchor {
                Some(anchor) => format!("{path}#{anchor}"),
                None => path,
            }
        }
        Target::WikiLink { name, anchor } => match anchor {
            Some(anchor) => format!("[[{name}#{anchor}]]"),
            None => format!("[[{name}]]"),
        },
        _ => String::new(),
    }
}

/// The answer to [`INDEX_METHOD`](crate::INDEX_METHOD): the root folders and every
/// indexed document. Meant for tests and debugging, not a stable interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexReport {
    pub roots: Vec<RootReport>,
    /// Sorted by URI.
    pub documents: Vec<DocumentReport>,
}

/// A root folder in an [`IndexReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootReport {
    pub uri: Uri,
    /// The config files in effect, lowest priority first.
    pub config_files: Vec<PathBuf>,
    /// The flavor set by the server or the configuration, if any.
    pub flavor: Option<String>,
}

/// An indexed document in an [`IndexReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentReport {
    pub uri: Uri,
    /// The client's version if the document is open.
    pub version: Option<i32>,
    /// The flavor it was parsed with.
    pub flavor: String,
    /// Its heading anchors, in document order.
    pub anchors: Vec<String>,
    /// Its link targets in document order, as text: a file relative to the root
    /// (`sub/b.md#anchor`), `#anchor`, the URL, `mailto:…` or `[[name#anchor]]`.
    pub links: Vec<String>,
}
