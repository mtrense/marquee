//! Where links point: [`Resolver`] and [`Destination`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{DocumentLink, Target, Vault, anchor_offset};
use crate::ast::NodeType;
use crate::input::has_markdown_extension;
use crate::{Document, Workspace};

/// Where a link points, as [`Resolver::destination`] finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Destination {
    /// The linking document itself.
    Here {
        /// Where the anchor points ([`anchor_offset`]), 0 without one; `None` if the
        /// document lacks it.
        offset: Option<usize>,
    },
    /// A local file or directory that exists.
    File {
        path: PathBuf,
        /// Where the anchor points in a Markdown file, 0 without one; `None` if the file
        /// lacks it, or is not a readable Markdown file.
        offset: Option<usize>,
    },
}

/// Follows the links of a [`Workspace`]'s documents as the [`Checker`](super::Checker)
/// does: Markdown files are looked up in the workspace, else loaded from disk (with the
/// workspace's options) and cached, and wikilinks are looked up in the [`Vault`], listed
/// on first use. Reuse one resolver for many links to share both caches.
#[derive(Debug)]
pub struct Resolver<'w> {
    workspace: &'w Workspace,
    /// Markdown files outside the workspace by path; `None` if unreadable.
    loaded: HashMap<PathBuf, Option<Document>>,
    /// The vault's files, listed on first use.
    vault: Option<Vault>,
}

impl<'w> Resolver<'w> {
    /// A resolver for links of documents of `workspace` (or documents linking into it).
    pub fn new(workspace: &'w Workspace) -> Self {
        Self {
            workspace,
            loaded: HashMap::new(),
            vault: None,
        }
    }

    /// The workspace links are resolved against.
    pub const fn workspace(&self) -> &'w Workspace {
        self.workspace
    }

    /// The Markdown document at `path`: from the workspace, else loaded from disk.
    /// `None` if it is not a readable Markdown file.
    pub fn markdown(&mut self, path: &Path) -> Option<&Document> {
        if let Some(document) = self.workspace.get(path) {
            return Some(document);
        }
        if !has_markdown_extension(path) || !path.is_file() {
            return None;
        }
        let options = self.workspace.options();
        self.loaded
            .entry(path.to_owned())
            .or_insert_with(|| options.load(path).ok())
            .as_ref()
    }

    /// The file a wikilink `name` points at; see [`Vault::find`].
    pub fn wikilink(&mut self, name: &str) -> Option<PathBuf> {
        let workspace = self.workspace;
        let vault = self.vault.get_or_insert_with(|| Vault::list(workspace));
        vault.find(name).map(Path::to_owned)
    }

    /// Where `link` of `document` points: the document itself (an anchor, or no
    /// destination), or a file that exists (in the workspace or on disk). `None` for a
    /// missing file, a wikilink that names no file, and external links and `mailto:`.
    pub fn destination(&mut self, document: &Document, link: &DocumentLink) -> Option<Destination> {
        let wiki = matches!(link.kind, NodeType::WikiLink | NodeType::Embed);
        self.target_destination(document, &link.target, wiki)
    }

    /// Where a (resolved) `target` in `document` points, e.g. that of a
    /// [`ReferenceDefinition`](super::ReferenceDefinition); see
    /// [`destination`](Self::destination). `wiki` is whether it is a wikilink's, whose
    /// anchors are heading texts or block ids.
    pub fn target_destination(
        &mut self,
        document: &Document,
        target: &Target,
        wiki: bool,
    ) -> Option<Destination> {
        let (path, anchor) = match target {
            Target::Empty => return Some(Destination::Here { offset: Some(0) }),
            Target::Anchor(anchor) => {
                let offset = anchor_offset(document, anchor, wiki);
                return Some(Destination::Here { offset });
            }
            Target::File { path, anchor } => {
                if self.workspace.get(path).is_none() && !path.exists() {
                    return None;
                }
                (path.clone(), anchor.as_deref())
            }
            Target::WikiLink { name, anchor } => (self.wikilink(name)?, anchor.as_deref()),
            _ => return None,
        };
        let offset = self
            .markdown(&path)
            .and_then(|target| anchor_offset(target, anchor.unwrap_or(""), wiki));
        Some(Destination::File { path, offset })
    }
}
