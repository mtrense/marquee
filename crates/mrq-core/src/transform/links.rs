//! Link and asset rewriting: [`rewrite_links`].

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use crate::ast::{LinkForm, NodeKind};
use crate::document::{Document, normalize};
use crate::lint::rules::link_syntax;
use crate::source::{ByteRange, TextEdit};
use crate::validate::{percent_decode, reference_definitions, scheme};

/// Where a rewritten link points: a file and an anchor in it, or an anchor in the
/// output (the document being written, [`LinkRewrite`]'s `to`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Location {
    /// The file; `None` for the output. Relative paths are relative to the current
    /// directory.
    pub file: Option<PathBuf>,
    /// The anchor (a slug, not percent-encoded); `None` for the top of the file.
    pub anchor: Option<String>,
}

impl Location {
    /// The top of the output.
    pub const fn output() -> Self {
        Self {
            file: None,
            anchor: None,
        }
    }

    /// The top of the file at `path`.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            file: Some(path.into()),
            anchor: None,
        }
    }

    /// Sets the anchor.
    #[must_use]
    pub fn with_anchor(mut self, anchor: impl Into<String>) -> Self {
        self.anchor = Some(anchor.into());
        self
    }
}

/// Where the contents of files and their anchors went: the table [`rewrite_links`]
/// consults before rebasing a link.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkMap {
    files: HashMap<PathBuf, Location>,
    anchors: HashMap<(PathBuf, String), Location>,
}

impl LinkMap {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that the content of `file` went to `to`: a link to the file goes to
    /// `to`, and one to an anchor of it that has no [entry](Self::insert_anchor) of its
    /// own goes to the same anchor in `to`'s file.
    pub fn insert_file(&mut self, file: impl AsRef<Path>, to: Location) {
        self.files.insert(absolute(file.as_ref()), to);
    }

    /// Records that `anchor` (a slug, not percent-encoded) of `file` went to `to`.
    pub fn insert_anchor(
        &mut self,
        file: impl AsRef<Path>,
        anchor: impl Into<String>,
        to: Location,
    ) {
        self.anchors
            .insert((absolute(file.as_ref()), anchor.into()), to);
    }

    /// Where a link to `anchor` of `file` (its top for `None`) points now, if the map
    /// knows: the anchor's entry, else the file's (with the anchor kept).
    pub fn get(&self, file: impl AsRef<Path>, anchor: Option<&str>) -> Option<Location> {
        let file = absolute(file.as_ref());
        if let Some(location) = anchor.and_then(|anchor| self.anchor(&file, anchor)) {
            return Some(location.clone());
        }
        let location = self.files.get(&file)?;
        Some(match anchor {
            Some(anchor) => Location {
                file: location.file.clone(),
                anchor: Some(anchor.to_owned()),
            },
            None => location.clone(),
        })
    }

    /// The entry for `anchor` of the absolute, normalized `file`.
    fn anchor(&self, file: &Path, anchor: &str) -> Option<&Location> {
        // Borrowed tuple keys cannot be looked up without building one.
        self.anchors.get(&(file.to_owned(), anchor.to_owned()))
    }
}

/// How [`rewrite_links`] rewrites.
#[derive(Debug, Clone)]
pub struct LinkRewrite<'m> {
    from: PathBuf,
    to: PathBuf,
    root: PathBuf,
    map: Option<&'m LinkMap>,
    /// The URL of remote content, which makes relative links absolute URLs instead.
    url: Option<String>,
}

impl<'m> LinkRewrite<'m> {
    /// Rewrites links written in the file `from` so they work from the file `to`
    /// (relative paths are relative to the current directory). For output that is not
    /// a file, such as stdout, `to` is any file in the directory links should be
    /// relative to.
    pub fn new(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Self {
        Self {
            from: absolute(from.as_ref()),
            to: absolute(to.as_ref()),
            root: absolute(Path::new("")),
            map: None,
            url: None,
        }
    }

    /// Rewrites links written in the document at the http(s) `url` (remote content) to
    /// absolute URLs, so they work from anywhere: relative paths, `/…` included, are
    /// resolved against `url` the way a browser does (RFC 3986). Same-document `#anchor`s
    /// and wikilinks stay; there is no [root](Self::root) or [map](Self::map).
    pub fn from_url(url: impl Into<String>) -> Self {
        Self {
            url: Some(url.into()),
            ..Self::new("", "")
        }
    }

    /// Sets the directory a leading `/` stands for, the workspace root (default: the
    /// current directory). Such links are never rebased, only [mapped](Self::map).
    #[must_use]
    pub fn root(mut self, root: impl AsRef<Path>) -> Self {
        self.root = absolute(root.as_ref());
        self
    }

    /// Sets the [`LinkMap`] to consult.
    #[must_use]
    pub const fn map(mut self, map: &'m LinkMap) -> Self {
        self.map = Some(map);
        self
    }
}

impl LinkRewrite<'_> {
    /// The destination `written` (as in the source) rewritten, if it changes.
    fn destination(&self, written: &str, syntax: Syntax) -> Option<String> {
        if written.is_empty() || scheme(written).is_some() || written.starts_with("//") {
            return None;
        }
        if let Some(url) = &self.url {
            let keep = syntax == Syntax::Wiki || written.starts_with('#');
            return (!keep).then(|| join_url(url, written));
        }
        if syntax == Syntax::Wiki {
            // Names are looked up in the whole vault; only `./` and `../` are paths.
            return if written.starts_with("./") || written.starts_with("../") {
                self.rebase(written, syntax)
            } else {
                None
            };
        }
        let (rest, fragment) = match written.split_once('#') {
            Some((rest, fragment)) => (rest, Some(fragment)),
            None => (written, None),
        };
        let (path, query) = rest.split_at(rest.find('?').unwrap_or(rest.len()));
        let anchor = fragment.map(|fragment| decode(fragment, syntax));
        if path.is_empty() {
            let anchor = anchor?;
            let location = self.map?.anchor(&self.from, &anchor)?;
            // The same anchor of the output: kept as written.
            let output = location
                .file
                .as_deref()
                .is_none_or(|file| absolute(file) == self.to);
            if output && location.anchor.as_deref() == Some(&anchor) {
                return None;
            }
            return Some(self.render(location, fragment.zip(Some(anchor.as_str()))));
        }
        let decoded = decode(path, syntax);
        let file = match decoded.strip_prefix('/') {
            Some(rest) => normalize(&self.root.join(rest)),
            None => normalize(&parent(&self.from).join(&decoded)),
        };
        if let Some(location) = self.map.and_then(|map| map.get(&file, anchor.as_deref())) {
            return Some(self.render(&location, fragment.zip(anchor.as_deref())));
        }
        if path.starts_with('/') {
            return None;
        }
        let mut out = self.rebase(path, syntax)?;
        out.push_str(query);
        if let Some(fragment) = fragment {
            out.push('#');
            out.push_str(fragment);
        }
        Some(out)
    }

    /// The relative `path` as written from `to`'s directory, if that differs from
    /// `from`'s: the way from there to `from`'s directory, then `path`, with `.` dropped
    /// and `..` folded into the name before it.
    fn rebase(&self, path: &str, syntax: Syntax) -> Option<String> {
        let (to, from) = (parent(&self.to), parent(&self.from));
        if to == from {
            return None;
        }
        let mut segments: Vec<String> = relative(&to, &from)?
            .into_iter()
            .map(|segment| encode(&segment, syntax))
            .collect();
        for segment in path.split('/') {
            match segment {
                "" | "." => {}
                ".." if segments.last().is_some_and(|last| last != "..") => {
                    segments.pop();
                }
                other => segments.push(other.to_owned()),
            }
        }
        let mut out = segments.join("/");
        if path.ends_with('/') || out.is_empty() {
            out.push('/');
        }
        // A wikilink that does not start with `./` or `../` is a name; a first segment
        // with a colon could read as a URL scheme.
        let dotted = out.starts_with("../") || out == "..";
        if out.starts_with('/') || (syntax == Syntax::Wiki && !dotted) || scheme(&out).is_some() {
            out.insert_str(0, if out.starts_with('/') { "." } else { "./" });
        }
        Some(out)
    }

    /// `location` as a destination written from `to`. `written` is the fragment of the
    /// destination it replaces, as written and decoded: kept as written if it is the
    /// same anchor.
    fn render(&self, location: &Location, written: Option<(&str, &str)>) -> String {
        let anchor = location.anchor.as_deref().map_or(String::new(), |anchor| {
            match written.filter(|&(_, decoded)| decoded == anchor) {
                Some((fragment, _)) => format!("#{fragment}"),
                None => format!("#{}", encode(anchor, Syntax::Markdown)),
            }
        });
        let file = location.file.as_deref().map(absolute);
        match file {
            Some(file) if file != self.to => {
                let segments = relative(&parent(&self.to), &file)
                    .unwrap_or_else(|| vec![file.to_string_lossy().into_owned()]);
                let path: Vec<String> = segments
                    .iter()
                    .map(|segment| encode(segment, Syntax::Markdown))
                    .collect();
                format!("{}{anchor}", path.join("/"))
            }
            _ if anchor.is_empty() => "#".to_owned(),
            _ => anchor,
        }
    }
}

/// How a destination is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Syntax {
    /// A Markdown link, image or definition: backslash escapes and percent-encoding.
    Markdown,
    /// An HTML attribute value: percent-encoding (character references are kept).
    Html,
    /// A wikilink or embed target: literal.
    Wiki,
}

/// Rewrites the destinations of the links that start in `range` so they work from
/// another file, as `rewrite` says. Returns edits against the document's source,
/// sorted and without overlaps; empty if nothing changes.
///
/// Destinations are those of inline links and images, of link reference definitions
/// (reference links follow theirs), of `href` and `src` attributes in raw HTML (a light
/// tag scan that skips comments), and wikilink and embed targets written as relative
/// paths (`[[../Note]]`; other names are vault-wide lookups and stay). Autolinks,
/// external URLs, `mailto:` and protocol-relative `//host/…` URLs never change.
///
/// Each destination is resolved from `from` (`/…` from the root, after undoing
/// backslash escapes and percent-encoding), then:
///
/// 1. If the [`LinkMap`] knows the file and anchor, the whole destination (query
///    string included) is replaced by the [`Location`]: `#anchor` if it is in the output
///    (`to`), else a path relative to `to` with the anchor. An anchor that stays the
///    same keeps its fragment as written (`#%C3%BCber-uns`), and a same-document
///    `#anchor` that maps to itself in the output is left alone. A same-document
///    `#anchor` is looked up as an anchor of `from`, but only in the anchor entries, so
///    the file entry of `from` does not apply to it.
/// 2. Else a relative path is rebased: written from `to`'s directory, with the
///    segments, query string and fragment as written (`my%20file.md?raw=1#Set%20up`
///    keeps its encoding), and the new directory names percent-encoded where Markdown or
///    HTML needs it. `/…` and `#anchor` stay as they are.
///
/// When `from` and `to` are in the same directory, only mapped destinations change.
pub fn rewrite_links(doc: &Document, range: ByteRange, rewrite: &LinkRewrite<'_>) -> Vec<TextEdit> {
    let text = doc.text();
    let mut found = destinations(doc);
    found.retain(|(at, _)| at.start >= range.start && at.start < range.end);
    found.sort_by_key(|(at, _)| at.start);
    found
        .into_iter()
        .filter_map(|(at, syntax)| {
            let written = &text[at.start..at.end];
            let replacement = rewrite.destination(written, syntax)?;
            (replacement != written).then_some(TextEdit {
                range: at,
                replacement,
            })
        })
        .collect()
}

/// Every destination of `doc` as written, with its syntax.
fn destinations(doc: &Document) -> Vec<(ByteRange, Syntax)> {
    let ast = doc.ast();
    let text = doc.text();
    let mut found = Vec::new();
    for id in ast.descendants(ast.root()) {
        let span = ast.span(id);
        match ast.kind(id) {
            NodeKind::Link(link) | NodeKind::Image(link) if link.form == LinkForm::Inline => {
                if let Some(at) = link_syntax(doc, id).and_then(|syntax| syntax.destination) {
                    found.push((at, Syntax::Markdown));
                }
            }
            NodeKind::HtmlBlock(_) | NodeKind::HtmlInline(_) => found.extend(
                html_destinations(text, span)
                    .into_iter()
                    .map(|at| (at, Syntax::Html)),
            ),
            NodeKind::WikiLink(link) | NodeKind::Embed(link) => {
                let open = span.start
                    + if text[span.start..].starts_with('!') {
                        3
                    } else {
                        2
                    };
                if !link.target.is_empty() && text[open..span.end].starts_with(&link.target) {
                    found.push((ByteRange::new(open, open + link.target.len()), Syntax::Wiki));
                }
            }
            _ => {}
        }
    }
    for definition in reference_definitions(doc) {
        let end = definition.span.end - usize::from(text[..definition.span.end].ends_with('>'));
        if !definition.url.is_empty() {
            found.push((
                ByteRange::new(end - definition.url.len(), end),
                Syntax::Markdown,
            ));
        }
    }
    found
}

/// The values of the `href` and `src` attributes of the tags in `text[span]`, outside
/// comments.
fn html_destinations(text: &str, span: ByteRange) -> Vec<ByteRange> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut i = span.start;
    while i < span.end {
        // Bytes, since `i` steps through multi-byte characters.
        if bytes[i..span.end].starts_with(b"<!--") {
            i = text[i..span.end]
                .find("-->")
                .map_or(span.end, |close| i + close + 3);
        } else if bytes[i] == b'<' && bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
            i = tag_attributes(text, i + 1, span.end, &mut found);
        } else {
            i += 1;
        }
    }
    found
}

/// Scans the tag whose name starts at `i`, up to `end`, pushing the values of its
/// `href` and `src` attributes; returns the offset after the tag.
fn tag_attributes(text: &str, mut i: usize, end: usize, found: &mut Vec<ByteRange>) -> usize {
    let bytes = text.as_bytes();
    let blank = |i: usize| i < end && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/');
    let word =
        |i: usize| i < end && !bytes[i].is_ascii_whitespace() && !b"/>=\"'".contains(&bytes[i]);
    while word(i) {
        i += 1;
    }
    loop {
        while blank(i) {
            i += 1;
        }
        if !word(i) {
            // `>`, the end, or a stray quote or `=`.
            return (i + 1).min(end);
        }
        let name = i;
        while word(i) {
            i += 1;
        }
        let name = &text[name..i];
        let mut j = i;
        while j < end && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= end || bytes[j] != b'=' {
            continue;
        }
        j += 1;
        while j < end && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let value = match bytes.get(j) {
            Some(&quote @ (b'"' | b'\'')) if j < end => {
                let Some(close) = text[j + 1..end].find(char::from(quote)) else {
                    return end;
                };
                i = j + close + 2;
                ByteRange::new(j + 1, j + 1 + close)
            }
            _ => {
                let len = text[j..end]
                    .find(|c: char| c.is_ascii_whitespace() || c == '>')
                    .unwrap_or(end - j);
                i = j + len;
                ByteRange::new(j, j + len)
            }
        };
        if name.eq_ignore_ascii_case("href") || name.eq_ignore_ascii_case("src") {
            found.push(value);
        }
    }
}

/// `text` (a path or fragment as written) decoded: backslash escapes undone for
/// Markdown, then percent-decoded; wikilink targets are literal.
fn decode(text: &str, syntax: Syntax) -> String {
    match syntax {
        Syntax::Wiki => text.to_owned(),
        Syntax::Html => percent_decode(text),
        Syntax::Markdown => {
            let mut unescaped = String::with_capacity(text.len());
            let mut chars = text.chars().peekable();
            while let Some(c) = chars.next() {
                match chars.peek() {
                    Some(&next) if c == '\\' && next.is_ascii_punctuation() => {
                        unescaped.push(next);
                        chars.next();
                    }
                    _ => unescaped.push(c),
                }
            }
            percent_decode(&unescaped)
        }
    }
}

/// `segment` (a file name or slug) percent-encoded where it could not stand verbatim
/// in a destination: whitespace, controls, `()<>\"'`, `#`, `?` and `%`. Wikilink
/// targets are literal.
fn encode(segment: &str, syntax: Syntax) -> String {
    if syntax == Syntax::Wiki {
        return segment.to_owned();
    }
    let mut out = String::with_capacity(segment.len());
    for c in segment.chars() {
        if c.is_whitespace() || c.is_control() || "()<>\\\"'#?%".contains(c) {
            let mut buf = [0; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                let _ = write!(out, "%{byte:02X}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The URL reference `reference` (as written: a relative path with an optional query
/// and fragment, or a URL) resolved against the absolute URL `base`, as RFC 3986
/// (section 5.2) says: `..` and `.` segments are removed, and the fragment of `base` is
/// dropped. Nothing is decoded or encoded.
pub(crate) fn join_url(base: &str, reference: &str) -> String {
    if scheme(reference).is_some() {
        return reference.to_owned();
    }
    let base = base.split_once('#').map_or(base, |(base, _)| base);
    let (scheme, rest) = base.split_once(':').unwrap_or(("", base));
    if reference.starts_with("//") {
        return format!("{scheme}:{reference}");
    }
    // `//authority`, then the path, then `?query`.
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, rest) = rest.split_at(authority_end);
    let (path, query) = rest.split_at(rest.find('?').unwrap_or(rest.len()));
    let origin = format!("{scheme}://{authority}");
    if reference.is_empty() || reference.starts_with('#') {
        return format!("{origin}{path}{query}{reference}");
    }
    if reference.starts_with('?') {
        return format!("{origin}{path}{reference}");
    }
    let end = reference.find(['?', '#']).unwrap_or(reference.len());
    let (target, tail) = reference.split_at(end);
    let merged = if target.starts_with('/') {
        target.to_owned()
    } else {
        // The base path up to its last `/` (just `/` if it has no path), then the target.
        let dir = path.rfind('/').map_or("/", |slash| &path[..=slash]);
        format!("{dir}{target}")
    };
    format!("{origin}{}{tail}", remove_dot_segments(&merged))
}

/// The absolute URL path `path` without `.` and `..` segments; `..` above the top stays
/// at the top. A path ending in such a segment keeps a trailing `/`.
fn remove_dot_segments(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').skip(1).collect();
    let mut out: Vec<&str> = Vec::with_capacity(segments.len());
    for (i, segment) in segments.iter().enumerate() {
        let last = i + 1 == segments.len();
        match *segment {
            "." => {}
            ".." => {
                out.pop();
            }
            other => {
                out.push(other);
                continue;
            }
        }
        if last {
            out.push("");
        }
    }
    format!("/{}", out.join("/"))
}

/// The path components leading from the directory `base` to `target` (both absolute
/// and normalized): `..` for each of `base`'s that `target` does not share, then
/// `target`'s own. `None` if they have different roots or a name is not UTF-8.
fn relative(base: &Path, target: &Path) -> Option<Vec<String>> {
    let base: Vec<Component<'_>> = base.components().collect();
    let target: Vec<Component<'_>> = target.components().collect();
    let shared = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    if shared == 0 {
        return None;
    }
    let mut segments = vec!["..".to_owned(); base.len() - shared];
    for component in &target[shared..] {
        match component {
            Component::Normal(name) => segments.push(name.to_str()?.to_owned()),
            _ => return None,
        }
    }
    Some(segments)
}

/// The directory of the absolute, normalized `file`.
fn parent(file: &Path) -> PathBuf {
    file.parent().unwrap_or(file).to_owned()
}

/// `path` made absolute against the current directory, and normalized.
fn absolute(path: &Path) -> PathBuf {
    let path = if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    };
    normalize(&std::path::absolute(path).unwrap_or_else(|_| path.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentOptions;
    use crate::source::apply_edits;

    fn rewrite_doc(doc: &Document, rewrite: &LinkRewrite<'_>) -> String {
        let src = doc.text();
        let edits = rewrite_links(doc, ByteRange::new(0, src.len()), rewrite);
        apply_edits(src, &edits).unwrap()
    }

    /// `src`, written in `from`, rewritten to work from `to`.
    fn moved(src: &str, from: &str, to: &str) -> String {
        rewrite_doc(&Document::parse(src), &LinkRewrite::new(from, to))
    }

    fn mapped(src: &str, from: &str, to: &str, map: &LinkMap) -> String {
        rewrite_doc(&Document::parse(src), &LinkRewrite::new(from, to).map(map))
    }

    #[test]
    fn relative_links_are_rebased_up_and_down() {
        let src = "[a](img/a.png) [b](../b.md) [c](./c.md) ![d](d.png)\n";
        assert_eq!(
            moved(src, "docs/x.md", "x.md"),
            "[a](docs/img/a.png) [b](b.md) [c](docs/c.md) ![d](docs/d.png)\n"
        );
        assert_eq!(
            moved(src, "docs/x.md", "docs/guide/x.md"),
            "[a](../img/a.png) [b](../../b.md) [c](../c.md) ![d](../d.png)\n"
        );
        assert_eq!(
            moved(src, "docs/x.md", "other/x.md"),
            "[a](../docs/img/a.png) [b](../b.md) [c](../docs/c.md) ![d](../docs/d.png)\n"
        );
    }

    #[test]
    fn nothing_changes_within_one_directory() {
        let src = "[a](./a.md) [b](../b.md?x#y) <img src=\"./i.png\">\n";
        let doc = Document::parse(src);
        let rewrite = LinkRewrite::new("docs/x.md", "docs/y.md");
        assert_eq!(
            rewrite_links(&doc, ByteRange::new(0, src.len()), &rewrite),
            []
        );
    }

    #[test]
    fn query_strings_and_fragments_are_kept_as_written() {
        assert_eq!(
            moved(
                "[a](../a.md?raw=1&v=2#Set%20up) [b](b.md#x?y)\n",
                "docs/x.md",
                "x.md"
            ),
            "[a](a.md?raw=1&v=2#Set%20up) [b](docs/b.md#x?y)\n"
        );
    }

    #[test]
    fn written_segments_keep_their_encoding_and_new_ones_are_encoded() {
        assert_eq!(
            moved(
                "[a](my%20file.md) [b](<a b.md>) [c](a\\(1\\).md)\n",
                "x.md",
                "sub/x.md"
            ),
            "[a](../my%20file.md) [b](<../a b.md>) [c](../a\\(1\\).md)\n"
        );
        assert_eq!(
            moved("[a](a.md) [b](../b%20c.md)\n", "my dir/(1)/x.md", "x.md"),
            "[a](my%20dir/%281%29/a.md) [b](my%20dir/b%20c.md)\n"
        );
        assert_eq!(
            moved("[a](a.md)\n", "über/x.md", "x.md"),
            "[a](über/a.md)\n"
        );
    }

    #[test]
    fn directories_and_dots_stay_directories() {
        assert_eq!(
            moved(
                "[a](./) [b](../) [c](sub/) [d](.) [e](..)\n",
                "docs/x.md",
                "docs/sub/x.md"
            ),
            "[a](../) [b](../../) [c](../sub/) [d](..) [e](../..)\n"
        );
    }

    #[test]
    fn a_first_segment_that_reads_as_a_scheme_gets_a_dot() {
        assert_eq!(
            moved("[a](../c:d.md)\n", "x/y/a.md", "x/a.md"),
            "[a](./c:d.md)\n"
        );
    }

    #[test]
    fn non_relative_destinations_are_left_alone() {
        let src = "[a](https://x.org/a.md) [b](mailto:a@b.org) [c](//cdn/x.js) [d](#top) \
                   [e](/root.md) [f]() [g](?q) <https://x.org> www.x.org\n";
        assert_eq!(moved(src, "docs/x.md", "x.md"), src);
    }

    #[test]
    fn reference_definitions_are_rebased() {
        let src = "[a][r] [b][s]\n\n[r]: ../r.md \"Title\"\n[s]: <s s.md>\n";
        assert_eq!(
            moved(src, "docs/x.md", "x.md"),
            "[a][r] [b][s]\n\n[r]: r.md \"Title\"\n[s]: <docs/s s.md>\n"
        );
    }

    #[test]
    fn html_href_and_src_are_rebased() {
        let src = "<p align=\"center\">\n<img src=\"logo.png\" alt=\"x\"><a HREF='a.md'>a</a>\n</p>\n\n\
                   Text <a href=b.md>b</a> <img src=\"https://x.org/i.png\" data-src=\"no.png\">\n\n\
                   <!-- <img src=\"commented.png\"> -->\n";
        assert_eq!(
            moved(src, "docs/x.md", "x.md"),
            "<p align=\"center\">\n<img src=\"docs/logo.png\" alt=\"x\"><a HREF='docs/a.md'>a</a>\n</p>\n\n\
             Text <a href=docs/b.md>b</a> <img src=\"https://x.org/i.png\" data-src=\"no.png\">\n\n\
             <!-- <img src=\"commented.png\"> -->\n"
        );
    }

    #[test]
    fn explicitly_relative_wikilinks_are_rebased() {
        let obsidian = |src: &str| {
            DocumentOptions::new()
                .flavor("obsidian")
                .parse(src)
                .unwrap()
        };
        let doc = obsidian("[[../Note#A|alias]] ![[./pic.png]] [[Note]] [[folder/Note]]\n");
        assert_eq!(
            rewrite_doc(&doc, &LinkRewrite::new("vault/notes/x.md", "vault/x.md")),
            "[[./Note#A|alias]] ![[./notes/pic.png]] [[Note]] [[folder/Note]]\n"
        );
    }

    #[test]
    fn only_links_starting_in_the_range_change() {
        let src = "[a](a.md)\n\n[b](b.md)\n";
        let doc = Document::parse(src);
        let rewrite = LinkRewrite::new("docs/x.md", "x.md");
        let edits = rewrite_links(&doc, ByteRange::new(5, src.len()), &rewrite);
        assert_eq!(
            apply_edits(src, &edits).unwrap(),
            "[a](a.md)\n\n[b](docs/b.md)\n"
        );
    }

    #[test]
    fn root_relative_links_resolve_from_the_root_for_the_map() {
        let mut map = LinkMap::new();
        map.insert_file("site/b.md", Location::output().with_anchor("b"));
        let doc = Document::parse("[b](/b.md) [c](/c.md)\n");
        let rewrite = LinkRewrite::new("site/docs/a.md", "site/out.md")
            .root("site")
            .map(&map);
        assert_eq!(rewrite_doc(&doc, &rewrite), "[b](#b) [c](/c.md)\n");
    }

    #[test]
    fn merged_files_become_anchors_in_the_output() {
        let mut map = LinkMap::new();
        map.insert_file("a.md", Location::output().with_anchor("a"));
        map.insert_file("docs/b.md", Location::output().with_anchor("b"));
        map.insert_anchor(
            "docs/b.md",
            "setup",
            Location::output().with_anchor("setup-1"),
        );
        map.insert_anchor("a.md", "intro", Location::output().with_anchor("intro-2"));
        let src = "[1](docs/b.md) [2](docs/b.md#setup) [3](./docs/b.md?x#other) [4](#intro) \
                   [5](a.md#intro) [6](#none) [7](c.md)\n";
        assert_eq!(
            mapped(src, "a.md", "out/merged.md", &map),
            "[1](#b) [2](#setup-1) [3](#other) [4](#intro-2) [5](#intro-2) [6](#none) [7](../c.md)\n"
        );
    }

    #[test]
    fn split_anchors_become_file_links_except_into_the_output() {
        let mut map = LinkMap::new();
        map.insert_anchor(
            "guide.md",
            "install",
            Location::file("guide/02-install.md").with_anchor("install"),
        );
        map.insert_anchor(
            "guide.md",
            "intro",
            Location::file("guide/01-intro.md").with_anchor("intro"),
        );
        map.insert_anchor(
            "guide.md",
            "über uns",
            Location::file("guide/my part.md").with_anchor("über uns"),
        );
        let src = "[a](#install) [b](#intro) [c](guide.md#install) [d](#%C3%BCber%20uns) [e](img/x.png)\n";
        assert_eq!(
            mapped(src, "guide.md", "guide/01-intro.md", &map),
            "[a](02-install.md#install) [b](#intro) [c](02-install.md#install) \
             [d](my%20part.md#%C3%BCber%20uns) [e](../img/x.png)\n"
        );
    }

    #[test]
    fn link_map_lookup() {
        let mut map = LinkMap::new();
        map.insert_file("./docs/../b.md", Location::output().with_anchor("b"));
        map.insert_anchor("b.md", "x", Location::output().with_anchor("x-1"));
        assert_eq!(
            map.get("b.md", None),
            Some(Location::output().with_anchor("b"))
        );
        assert_eq!(
            map.get("b.md", Some("x")),
            Some(Location::output().with_anchor("x-1"))
        );
        assert_eq!(
            map.get("b.md", Some("y")),
            Some(Location::output().with_anchor("y"))
        );
        assert_eq!(map.get("c.md", None), None);
    }

    #[test]
    fn links_in_remote_content_become_absolute_urls() {
        let src = "[a](a.md) [b](../b.md?x=1#y) [c](/c.md) [d](#d) [e](https://e.org/) \
                   [f](//f.org/x) [g](mailto:g@h.org) <img src=\"img/i.png\">\n\n[r]: ./r.md\n";
        let rewrite = LinkRewrite::from_url("https://example.com/docs/guide.md?raw=1");
        assert_eq!(
            rewrite_doc(&Document::parse(src), &rewrite),
            "[a](https://example.com/docs/a.md) [b](https://example.com/b.md?x=1#y) \
             [c](https://example.com/c.md) [d](#d) [e](https://e.org/) [f](//f.org/x) \
             [g](mailto:g@h.org) <img src=\"https://example.com/docs/img/i.png\">\n\n\
             [r]: https://example.com/docs/r.md\n"
        );
    }

    #[test]
    fn urls_join_as_rfc_3986_says() {
        // The examples of RFC 3986, section 5.4.
        let base = "http://a/b/c/d;p?q";
        for (reference, joined) in [
            ("g:h", "g:h"),
            ("g", "http://a/b/c/g"),
            ("./g", "http://a/b/c/g"),
            ("g/", "http://a/b/c/g/"),
            ("/g", "http://a/g"),
            ("//g", "http://g"),
            ("?y", "http://a/b/c/d;p?y"),
            ("g?y", "http://a/b/c/g?y"),
            ("#s", "http://a/b/c/d;p?q#s"),
            ("g#s", "http://a/b/c/g#s"),
            ("g?y#s", "http://a/b/c/g?y#s"),
            (";x", "http://a/b/c/;x"),
            ("", "http://a/b/c/d;p?q"),
            (".", "http://a/b/c/"),
            ("./", "http://a/b/c/"),
            ("..", "http://a/b/"),
            ("../g", "http://a/b/g"),
            ("../..", "http://a/"),
            ("../../g", "http://a/g"),
            ("../../../g", "http://a/g"),
            ("/./g", "http://a/g"),
            ("/../g", "http://a/g"),
            ("g.", "http://a/b/c/g."),
            ("..g", "http://a/b/c/..g"),
            ("./../g", "http://a/b/g"),
            ("./g/.", "http://a/b/c/g/"),
            ("g/./h", "http://a/b/c/g/h"),
            ("g/../h", "http://a/b/c/h"),
        ] {
            assert_eq!(join_url(base, reference), joined, "{reference}");
        }
        assert_eq!(join_url("https://a", "g"), "https://a/g");
        assert_eq!(join_url("https://a#f", "#g"), "https://a#g");
        assert_eq!(join_url("https://a/b#f", ""), "https://a/b");
    }
}
