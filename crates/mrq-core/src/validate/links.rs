//! Link classification and resolution: [`Target`], [`DocumentLink`] and [`links`].

use std::path::{Path, PathBuf};

use crate::ast::{Link, LinkForm, NodeId, NodeKind, NodeType, Span, WikiLink};
use crate::{Document, Workspace};

/// One link, image, wikilink or embed of a document, with its classified target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentLink {
    /// The link's node.
    pub node: NodeId,
    /// The node's type: [`Link`](NodeType::Link), [`Image`](NodeType::Image),
    /// [`WikiLink`](NodeType::WikiLink) or [`Embed`](NodeType::Embed).
    pub kind: NodeType,
    /// The node's span.
    pub span: Span,
    /// For a reference link (`[text][label]`, `[label][]`, `[label]`), the label as
    /// written; the target is that of the definition.
    pub reference: Option<String>,
    /// What the destination points at, with file paths
    /// [resolved](Target::resolve) from the document.
    pub target: Target,
}

/// Every link, image, wikilink and embed of `document` in document order, classified
/// and resolved against `workspace`.
///
/// Link reference definitions are not included: a reference link already carries its
/// definition's target, and checks of the definitions themselves need the
/// [`LinkDefinition`](NodeKind::LinkDefinition) nodes.
pub fn links(document: &Document, workspace: &Workspace) -> Vec<DocumentLink> {
    let ast = document.ast();
    ast.descendants(ast.root())
        .filter_map(|node| {
            let (target, reference) = match ast.kind(node) {
                NodeKind::Link(link) | NodeKind::Image(link) => {
                    (Target::parse(&link.url), reference_label(link))
                }
                NodeKind::WikiLink(link) | NodeKind::Embed(link) => (Target::wikilink(link), None),
                _ => return None,
            };
            Some(DocumentLink {
                node,
                kind: ast.kind(node).node_type(),
                span: ast.span(node),
                reference,
                target: target.resolve(workspace, document.path()),
            })
        })
        .collect()
}

/// The label of a reference link.
fn reference_label(link: &Link) -> Option<String> {
    match &link.form {
        LinkForm::Reference { label, .. } => Some(label.clone()),
        _ => None,
    }
}

/// What a link destination points at: the class of a link, with what checking it needs.
///
/// [`parse`](Self::parse) classifies a Markdown destination and
/// [`wikilink`](Self::wikilink) an Obsidian wikilink or embed. Paths and fragments are
/// percent-decoded; a query string is dropped. [`File`](Self::File) paths are as written
/// until [resolved](Self::resolve) against a [`Workspace`].
///
/// ```
/// use std::path::Path;
/// use mrq_core::Workspace;
/// use mrq_core::validate::Target;
///
/// let ws = Workspace::new("site");
/// let target = Target::parse("../img/my%20logo.png?v=2").resolve(&ws, Some(Path::new("site/docs/a.md")));
/// assert_eq!(target, Target::File { path: "site/img/my logo.png".into(), anchor: None });
/// assert_eq!(Target::parse("#setup"), Target::Anchor("setup".into()));
/// assert_eq!(Target::parse("mailto:me@x.org"), Target::Mailto("me@x.org".into()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Target {
    /// No destination (`[text]()`, `[[]]`), or only a query string: the document itself.
    Empty,
    /// `#anchor` (or `[[#anchor]]`) in the same document, decoded and without `#`. Empty
    /// for a bare `#`.
    Anchor(String),
    /// A URL with a scheme other than `mailto:` (`https:`, `ftp:`, `tel:`, …), or
    /// protocol-relative (`//host/…`); kept verbatim.
    External(String),
    /// `mailto:`: the decoded address, without `?subject=…` and the like.
    Mailto(String),
    /// A local file (relative, or from the workspace root if it starts with `/`), possibly
    /// with an anchor in it.
    File {
        /// The decoded path; normalized and joined to its base once resolved.
        path: PathBuf,
        /// The decoded fragment after `#`, if any (empty for a trailing `#`).
        anchor: Option<String>,
    },
    /// An Obsidian `[[name#anchor]]` or `![[name#anchor]]`, to be looked up by name.
    WikiLink {
        /// The page or file name as written.
        name: String,
        /// The text after `#`: a heading, or `^id` for a block reference.
        anchor: Option<String>,
    },
}

impl Target {
    /// Classifies a link or image destination as written (after Markdown unescaping).
    ///
    /// A Windows drive path (`C:/x.md`) looks like a URL scheme and is
    /// [`External`](Self::External).
    pub fn parse(url: &str) -> Self {
        if let Some(scheme) = scheme(url) {
            if scheme.eq_ignore_ascii_case("mailto") {
                let address = &url[scheme.len() + 1..];
                let address = address.split_once('?').map_or(address, |(a, _)| a);
                return Self::Mailto(percent_decode(address));
            }
            return Self::External(url.to_owned());
        }
        if url.starts_with("//") {
            return Self::External(url.to_owned());
        }
        let (rest, anchor) = match url.split_once('#') {
            Some((rest, fragment)) => (rest, Some(percent_decode(fragment))),
            None => (url, None),
        };
        let path = rest.split_once('?').map_or(rest, |(path, _)| path);
        match (path, anchor) {
            ("", Some(anchor)) => Self::Anchor(anchor),
            ("", None) => Self::Empty,
            (path, anchor) => Self::File {
                path: PathBuf::from(percent_decode(path)),
                anchor,
            },
        }
    }

    /// Classifies a wikilink or embed: a [`WikiLink`](Self::WikiLink) to look up by
    /// name, or an [`Anchor`](Self::Anchor) if it names no page (`[[#heading]]`). Names
    /// are not percent-decoded, as Obsidian writes them literally.
    pub fn wikilink(link: &WikiLink) -> Self {
        match (link.target.as_str(), &link.anchor) {
            ("", Some(anchor)) => Self::Anchor(anchor.clone()),
            ("", None) => Self::Empty,
            (name, anchor) => Self::WikiLink {
                name: name.to_owned(),
                anchor: anchor.clone(),
            },
        }
    }

    /// This target with a [`File`](Self::File) path resolved by
    /// [`Workspace::resolve`] from the document at `from`; other targets are unchanged.
    #[must_use]
    pub fn resolve(self, workspace: &Workspace, from: Option<&Path>) -> Self {
        match self {
            Self::File { path, anchor } => Self::File {
                path: workspace.resolve(from, path),
                anchor,
            },
            other => other,
        }
    }
}

/// The scheme of `url` (RFC 3986: a letter, then letters, digits, `+`, `-` or `.`,
/// before the first `:`), if it has one.
pub(crate) fn scheme(url: &str) -> Option<&str> {
    let (scheme, _) = url.split_once(':')?;
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then_some(scheme)
}

/// `text` with `%XX` escapes decoded. Malformed escapes stay as written; if the decoded
/// bytes are not UTF-8, `text` is returned unchanged.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|&b| (b as char).to_digit(16));
        match (bytes[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push(u8::try_from(high * 16 + low).expect("two hex digits fit a byte"));
                i += 3;
            }
            (byte, ..) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_only_is_a_same_document_anchor() {
        assert_eq!(Target::parse("#intro"), Target::Anchor("intro".into()));
    }

    #[test]
    fn urls_with_a_scheme_are_external() {
        for url in [
            "https://example.com/a#b",
            "HTTP://x",
            "ftp://host/file.md",
            "//cdn.example.com/x.js",
            "tel:+123",
            "git+ssh://host/repo",
        ] {
            assert_eq!(Target::parse(url), Target::External(url.into()), "{url}");
        }
    }

    #[test]
    fn mailto_gives_the_decoded_address_without_query() {
        let mailto = |address: &str| Target::Mailto(address.into());
        assert_eq!(Target::parse("mailto:a@b.org"), mailto("a@b.org"));
        assert_eq!(
            Target::parse("MailTo:a@b.org?subject=Hi"),
            mailto("a@b.org")
        );
        assert_eq!(
            Target::parse("mailto:J%C3%BCrgen@b.org"),
            mailto("Jürgen@b.org")
        );
    }

    fn file(path: &str, anchor: Option<&str>) -> Target {
        Target::File {
            path: PathBuf::from(path),
            anchor: anchor.map(Into::into),
        }
    }

    #[test]
    fn paths_are_files_with_an_optional_anchor() {
        assert_eq!(Target::parse("docs/a.md"), file("docs/a.md", None));
        assert_eq!(Target::parse("../img/x.png"), file("../img/x.png", None));
        assert_eq!(Target::parse("/index.md"), file("/index.md", None));
        assert_eq!(Target::parse("a.md#setup"), file("a.md", Some("setup")));
        assert_eq!(Target::parse("a.md#"), file("a.md", Some("")));
    }

    #[test]
    fn query_strings_are_dropped() {
        assert_eq!(Target::parse("a.md?raw=1"), file("a.md", None));
        assert_eq!(Target::parse("a.md?raw=1#x"), file("a.md", Some("x")));
        assert_eq!(Target::parse("?raw=1#x"), Target::Anchor("x".into()));
        assert_eq!(Target::parse("?raw=1"), Target::Empty);
    }

    #[test]
    fn paths_and_fragments_are_percent_decoded() {
        assert_eq!(
            Target::parse("my%20notes/%C3%BCber.md#%C3%BCber-uns"),
            file("my notes/über.md", Some("über-uns"))
        );
        assert_eq!(Target::parse("#a%2Fb"), Target::Anchor("a/b".into()));
        // Malformed or non-UTF-8 escapes are kept as written.
        assert_eq!(Target::parse("100%.md"), file("100%.md", None));
        assert_eq!(Target::parse("a%zz%4.md"), file("a%zz%4.md", None));
        assert_eq!(Target::parse("%FF.md"), file("%FF.md", None));
    }

    fn wiki(target: &str, anchor: Option<&str>) -> WikiLink {
        WikiLink {
            target: target.into(),
            anchor: anchor.map(Into::into),
            alias: Some("shown".into()),
        }
    }

    #[test]
    fn wikilinks_keep_their_name_undecoded() {
        assert_eq!(
            Target::wikilink(&wiki("My%20Note", Some("Part 1"))),
            Target::WikiLink {
                name: "My%20Note".into(),
                anchor: Some("Part 1".into()),
            }
        );
        assert_eq!(
            Target::wikilink(&wiki("folder/x.png", None)),
            Target::WikiLink {
                name: "folder/x.png".into(),
                anchor: None,
            }
        );
    }

    #[test]
    fn wikilink_without_target_is_a_same_document_anchor() {
        assert_eq!(
            Target::wikilink(&wiki("", Some("Setup"))),
            Target::Anchor("Setup".into())
        );
        assert_eq!(
            Target::wikilink(&wiki("", Some("^block"))),
            Target::Anchor("^block".into())
        );
        assert_eq!(Target::wikilink(&wiki("", None)), Target::Empty);
    }

    #[test]
    fn file_paths_resolve_relative_to_the_document_or_the_root() {
        let ws = Workspace::new("site");
        let from = Some(Path::new("site/docs/guide/a.md"));
        let resolved = |url: &str, from| Target::parse(url).resolve(&ws, from);
        assert_eq!(
            resolved("../img/x%20y.png#frag", from),
            file("site/docs/img/x y.png", Some("frag"))
        );
        assert_eq!(resolved("./b.md", from), file("site/docs/guide/b.md", None));
        assert_eq!(resolved("/index.md", from), file("site/index.md", None));
        assert_eq!(resolved("b.md", None), file("site/b.md", None));
        // Everything else is left alone.
        for url in ["#x", "https://a/b.md", "mailto:a@b", ""] {
            assert_eq!(resolved(url, from), Target::parse(url), "{url}");
        }
    }

    #[test]
    fn empty_destination() {
        assert_eq!(Target::parse(""), Target::Empty);
        assert_eq!(Target::parse("#"), Target::Anchor(String::new()));
    }
}
