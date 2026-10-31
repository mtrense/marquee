//! Navigation: following the links of the selected block, jumping to outline entries and
//! files, and going back.
//!
//! The links of a block are its links and wikilinks in document order (images and
//! embeds are shown, not followed); `Enter` follows the chosen one, the first unless
//! `←`/`→` chose another. Targets resolve as `mrq validate` resolves them
//! ([`validate::links`], [`anchor_offset`], [`Vault`]):
//!
//! - an anchor selects its heading (or `^block`) in the shown file;
//! - a Markdown file is shown, at the anchor if there is one, else at the top. A file
//!   that is not open yet is loaded and added to the file list;
//! - web links, `mailto:` addresses and other local files go to the [`Opener`](crate::Opener).
//!
//! Every jump to a heading or a file (also from the outline and the file list) remembers
//! where it came from, for `Backspace`. A target that is not there is a status message,
//! and nothing moves; a missing anchor in another file shows that file at the top.

use std::ffi::OsStr;
use std::path::Path;

use mrq_core::ast::{NodeKind, NodeType};
use mrq_core::document::normalize_path;
use mrq_core::input::has_markdown_extension;
use mrq_core::validate::{self, DocumentLink, Target, Vault, anchor_offset};

use super::{App, FileView, Location, Panel};

impl App {
    /// The index of the chosen link among the selected block's, `None` if it has none.
    pub fn link_index(&self) -> Option<usize> {
        let count = self.block_links().len();
        (count > 0).then(|| self.chosen_link().min(count - 1))
    }

    /// The links of the selected block that `Enter` can follow, in document order.
    pub fn block_links(&self) -> Vec<DocumentLink> {
        let file = self.file();
        let Some(block) = file.view.selected() else {
            return Vec::new();
        };
        let span = block.span;
        validate::links(&file.document, &self.workspace)
            .into_iter()
            .filter(|link| matches!(link.kind, NodeType::Link | NodeType::WikiLink))
            .filter(|link| span.start <= link.span.start && link.span.end <= span.end)
            .collect()
    }

    /// The chosen link's index for the selected block, 0 unless chosen for it.
    fn chosen_link(&self) -> usize {
        let node = self.file().view.selected().map(|block| block.node);
        match self.link {
            Some((file, chosen, index)) if file == self.current && Some(chosen) == node => index,
            _ => 0,
        }
    }

    /// Chooses the link `step` links on in the selected block, wrapping around, and
    /// shows it in the status line.
    pub(super) fn choose_link(&mut self, step: isize) {
        let links = self.block_links();
        let Some(node) = self.file().view.selected().map(|block| block.node) else {
            return;
        };
        if links.is_empty() {
            self.status = Some("no link in the selected block".to_owned());
            return;
        }
        let count = links.len().cast_signed();
        let at = self.chosen_link().min(links.len() - 1).cast_signed();
        let index = (at + step).rem_euclid(count).cast_unsigned();
        self.link = Some((self.current, node, index));
        let destination = destination(&self.file().document, &links[index]);
        self.status = Some(format!("link {}/{count}: {destination}", index + 1));
    }

    /// Follows the chosen link of the selected block.
    pub(super) fn follow(&mut self) {
        let mut links = self.block_links();
        if links.is_empty() {
            self.status = Some("no link in the selected block".to_owned());
            return;
        }
        let link = links.swap_remove(self.chosen_link().min(links.len() - 1));
        let wiki = link.kind == NodeType::WikiLink;
        match &link.target {
            Target::Empty => self.go_to(self.current, 0),
            Target::Anchor(anchor) => match anchor_offset(&self.file().document, anchor, wiki) {
                Some(offset) => self.go_to(self.current, offset),
                None => self.status = Some(format!("no anchor `#{anchor}` in this document")),
            },
            Target::File { path, anchor } => {
                let shown = written_path(&self.file().document, &link).to_owned();
                self.follow_file(path, anchor.as_deref(), wiki, &shown);
            }
            Target::WikiLink { name, anchor } => {
                let workspace = &self.workspace;
                let vault = self.vault.get_or_insert_with(|| Vault::list(workspace));
                match vault.find(name).map(Path::to_owned) {
                    Some(path) => self.follow_file(&path, anchor.as_deref(), wiki, name),
                    None => self.status = Some(format!("no file named `{name}` in the vault")),
                }
            }
            Target::External(_) | Target::Mailto(_) => {
                let url = destination(&self.file().document, &link);
                self.open_externally(OsStr::new(&url));
            }
            _ => self.status = Some("cannot follow this link".to_owned()),
        }
    }

    /// Follows a link to the local file `path` (written as `shown`), with `anchor` in it:
    /// shows a Markdown file, or hands any other file to the opener.
    fn follow_file(&mut self, path: &Path, anchor: Option<&str>, wiki: bool, shown: &str) {
        let open = self.open_index(path);
        if open.is_none() && !path.exists() {
            self.status = Some(format!("no file `{shown}`"));
            return;
        }
        if open.is_none() && (!has_markdown_extension(path) || !path.is_file()) {
            return self.open_externally(path.as_os_str());
        }
        let index = match open {
            Some(index) => index,
            None => match self.workspace.options().load(normalize_path(path)) {
                Ok(document) => {
                    self.files.push(FileView::new(document));
                    self.files.len() - 1
                }
                Err(err) => {
                    self.status = Some(format!("cannot open `{shown}`: {err}"));
                    return;
                }
            },
        };
        let anchor = anchor.unwrap_or("");
        let offset = anchor_offset(&self.files[index].document, anchor, wiki);
        if offset.is_none() {
            self.status = Some(format!("no anchor `#{anchor}` in `{shown}`"));
            if index == self.current {
                return;
            }
        }
        self.go_to(index, offset.unwrap_or(0));
    }

    /// The index of the open file at `path`.
    fn open_index(&self, path: &Path) -> Option<usize> {
        let path = normalize_path(path);
        self.files.iter().position(|file| {
            file.document
                .path()
                .is_some_and(|own| normalize_path(own) == path)
        })
    }

    /// Hands `target` to the opener, with the outcome in the status line.
    fn open_externally(&mut self, target: &OsStr) {
        let shown = target.to_string_lossy();
        self.status = Some(match self.opener.open(target) {
            Ok(()) => format!("opened {shown}"),
            Err(err) => format!("cannot open {shown}: {err}"),
        });
    }

    /// Jumps to the heading of the outline entry under the outline cursor.
    pub(super) fn open_outline_cursor(&mut self) {
        let file = self.file();
        if let Some(entry) = file.outline.get(file.outline_cursor) {
            let offset = file.document.ast().span(entry.node).start;
            self.go_to(self.current, offset);
        }
    }

    /// Shows the file under the file list's cursor, with the selection it had.
    pub(super) fn open_file_cursor(&mut self) {
        if self.file_cursor != self.current {
            self.history.push(self.location());
            self.show(self.file_cursor);
        }
    }

    /// Goes back to where the last jump came from.
    pub(super) fn back(&mut self) {
        let Some(location) = self.history.pop() else {
            self.status = Some("nothing to go back to".to_owned());
            return;
        };
        self.show(location.file);
        let file = &mut self.files[self.current];
        file.view.select_offset(location.offset);
        file.sync_outline();
        self.focus = Panel::Document;
    }

    /// Jumps to the block at the source offset `offset` of file `index`, remembering
    /// where it came from, and focuses the document.
    fn go_to(&mut self, index: usize, offset: usize) {
        self.history.push(self.location());
        self.show(index);
        let file = &mut self.files[self.current];
        file.view.jump(offset);
        file.sync_outline();
        self.focus = Panel::Document;
    }

    /// Shows file `index`, with the file list's cursor on it.
    fn show(&mut self, index: usize) {
        self.current = index;
        self.file_cursor = index;
        self.relayout();
    }

    /// The shown file and its selected block.
    pub(super) fn location(&self) -> Location {
        let offset = self
            .file()
            .view
            .selected()
            .map_or(0, |block| block.span.start);
        Location {
            file: self.current,
            offset,
        }
    }
}

/// A link's destination as written: the URL of a link, `name#anchor` of a wikilink.
fn destination(document: &mrq_core::Document, link: &DocumentLink) -> String {
    match document.ast().kind(link.node) {
        NodeKind::Link(link) => link.url.clone(),
        NodeKind::WikiLink(link) => match &link.anchor {
            Some(anchor) => format!("{}#{anchor}", link.target),
            None => link.target.clone(),
        },
        _ => String::new(),
    }
}

/// The path of a file link as written: its URL up to any `?` or `#`.
fn written_path<'d>(document: &'d mrq_core::Document, link: &DocumentLink) -> &'d str {
    match document.ast().kind(link.node) {
        NodeKind::Link(link) => {
            let end = link.url.find(['?', '#']).unwrap_or(link.url.len());
            &link.url[..end]
        }
        _ => "",
    }
}
