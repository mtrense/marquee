//! Splitting a document into one file per section: [`split`].

use std::path::{Path, PathBuf};

use super::{LinkMap, LinkRewrite, Location, ShiftOptions, rewrite_links, shift_headings};
use crate::ast::{NodeId, NodeKind};
use crate::diag::{Diagnostic, Severity};
use crate::document::Document;
use crate::flavor::FlavorOrigin;
use crate::source::{ByteRange, SourceFile, TextEdit, apply_edits};

/// Code of the warning for a section whose headings are not promoted, because shifting
/// them back would not give them as they were (setext headings become ATX).
pub const SPLIT_PROMOTE_CODE: &str = "split-promote";

/// How [`split`] splits.
#[derive(Debug, Clone)]
pub struct SplitOptions {
    depth: u8,
    dir: Option<PathBuf>,
    root: Option<PathBuf>,
}

impl Default for SplitOptions {
    fn default() -> Self {
        Self {
            depth: 1,
            dir: None,
            root: None,
        }
    }
}

impl SplitOptions {
    /// Splitting at level 1, into the default directory.
    pub fn new() -> Self {
        Self::default()
    }

    /// The heading level to split at, 1–6 (default 1).
    #[must_use]
    pub const fn depth(mut self, depth: u8) -> Self {
        self.depth = depth;
        self
    }

    /// The directory the files go to (default: the document's path without its
    /// extension, `guide/` for `guide.md`; the current directory for stdin).
    #[must_use]
    pub fn dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// The directory a leading `/` in links stands for (default: the current directory).
    #[must_use]
    pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }
}

/// A file [`split`] makes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SplitFile {
    /// Where it goes: in the [directory](SplitOptions::dir).
    pub path: PathBuf,
    pub text: String,
}

/// The result of [`split`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Split {
    /// `index.md`: everything outside the sections, with an include directive for each.
    pub index: SplitFile,
    /// One file per section, in order.
    pub parts: Vec<SplitFile>,
    /// [`SPLIT_PROMOTE_CODE`] warnings, in the document.
    pub diagnostics: Vec<Diagnostic>,
}

/// Splits `doc` at its top-level headings of the [depth](SplitOptions::depth): each
/// section, from such a heading to the next top-level heading at that level or above,
/// becomes a file `NN-slug.md` (numbered from `01`, the slug made of the heading's text),
/// and the rest, the frontmatter, the preamble and the higher-level headings with what
/// follows them up to their first section, stays in `index.md`, with an
/// `<!-- mrq:include path="NN-slug.md" -->` in each section's place. Headings in
/// containers and in the content of directive regions do not start sections.
///
/// A file holds its section without trailing whitespace, plus a newline, after a
/// frontmatter block with `mrq.flavor` if the document's frontmatter set the flavor; the whitespace
/// between sections stays in `index.md`, after the directives, so expanding the index
/// ([`Includer::expand`](super::Includer::expand), `mrq interpolate`) gives the document
/// back. Links are made to work from the new files ([`rewrite_links`]): a link to an
/// anchor of the document goes to that heading in the file it went to (`02-usage.md#setup`
/// for `#setup-1`, or `#setup` in that file itself), and other relative links and assets
/// are rebased to the [directory](SplitOptions::dir). Expanding the index maps them back,
/// so the expansion is the document with its relative links rebased to `index.md`,
/// unchanged when the directory is the document's own.
///
/// Each file's headings are promoted so the shallowest of them is at level 1, and its
/// directive gets `shift=N` to push them back. A file whose headings would not shift
/// back as they were (setext headings become ATX when shifted) is not promoted, with a
/// [`SPLIT_PROMOTE_CODE`] warning.
///
/// Known limits: reference definitions and footnotes stay in the file of the section
/// they are in, so references to them from other files do not resolve until the index
/// is expanded; a link to the document by its own name (`guide.md#usage`) goes to the
/// new file too, and comes back as `#usage`; an anchor link whose anchor changes
/// (`#setup-1` to `02-usage.md#setup`) comes back written as the slug even if it was
/// percent-encoded; `mrq:include` directives in the document are expanded with the
/// index.
pub fn split(doc: &Document, options: &SplitOptions) -> Split {
    let depth = options.depth.clamp(1, 6);
    let dir = options.dir.clone().unwrap_or_else(|| default_dir(doc));
    let root = options.root.clone().unwrap_or_default();
    let text = doc.text();
    let sections = sections(doc, depth);
    let width = sections.len().to_string().len().max(2);
    let index_path = dir.join("index.md");
    let names: Vec<String> = sections
        .iter()
        .enumerate()
        .map(|(n, section)| {
            let slug = file_slug(&doc.ast().text(section.heading));
            let number = format!("{:0width$}", n + 1);
            if slug.is_empty() {
                format!("{number}.md")
            } else {
                format!("{number}-{slug}.md")
            }
        })
        .collect();
    let paths: Vec<PathBuf> = names.iter().map(|name| dir.join(name)).collect();
    let map = link_map(doc, &sections, &paths, &index_path);
    // A flavor the frontmatter chose goes with each file.
    let front = if doc.flavor_origin() == FlavorOrigin::Frontmatter {
        format!("---\nmrq:\n  flavor: {}\n---\n\n", doc.flavor().name())
    } else {
        String::new()
    };

    let mut diagnostics = Vec::new();
    let mut parts = Vec::new();
    let mut index_edits = Vec::new();
    for ((section, name), path) in sections.iter().zip(&names).zip(&paths) {
        let range = section.range;
        let rewrite = LinkRewrite::new(doc.name(), path).root(&root).map(&map);
        let edits: Vec<TextEdit> = rewrite_links(doc, range, &rewrite)
            .into_iter()
            .map(|edit| TextEdit {
                range: ByteRange::new(edit.range.start - range.start, edit.range.end - range.start),
                replacement: edit.replacement,
            })
            .collect();
        let linked =
            apply_edits(&text[range.start..range.end], &edits).expect("link edits do not overlap");
        let (promoted, by) = match promote(doc, path, linked) {
            Ok(promoted) => promoted,
            Err(linked) => {
                let span = doc.ast().span(section.heading);
                diagnostics.push(
                    Diagnostic::new(
                        SPLIT_PROMOTE_CODE,
                        Severity::Warning,
                        doc.name(),
                        ByteRange::new(span.start, span.end),
                        "the headings of this section are not promoted, since shifting them back would not restore them as written",
                    )
                    .with_help("setext headings become ATX headings when they are shifted"),
                );
                (linked, 0)
            }
        };
        parts.push(SplitFile {
            path: path.clone(),
            text: format!("{front}{promoted}\n"),
        });
        index_edits.push(TextEdit {
            range,
            replacement: marker(name, by),
        });
    }

    let rewrite = LinkRewrite::new(doc.name(), &index_path)
        .root(&root)
        .map(&map);
    let outside = |edit: &TextEdit| {
        !sections
            .iter()
            .any(|s| s.range.start <= edit.range.start && edit.range.start < s.range.end)
    };
    index_edits.extend(
        rewrite_links(doc, ByteRange::new(0, text.len()), &rewrite)
            .into_iter()
            .filter(outside),
    );
    index_edits.sort_by_key(|edit| edit.range.start);
    let index = SplitFile {
        path: index_path,
        text: apply_edits(text, &index_edits).expect("index edits do not overlap"),
    };
    Split {
        index,
        parts,
        diagnostics,
    }
}

/// A section [`split`] makes a file of.
struct SplitSection {
    heading: NodeId,
    /// From the start of the heading's line to the end of the section's last block.
    range: ByteRange,
}

/// The sections of `doc` at `depth`, in order.
fn sections(doc: &Document, depth: u8) -> Vec<SplitSection> {
    let ast = doc.ast();
    let text = doc.text();
    let in_region = |at: usize| {
        doc.directives()
            .iter()
            .any(|region| region.content.start <= at && at < region.content.end)
    };
    let line_start = |at: usize| text[..at].rfind('\n').map_or(0, |i| i + 1);
    // The top-level headings at `depth` or above: where sections end.
    let bounds: Vec<(NodeId, u8, usize)> = ast
        .children(ast.root())
        .iter()
        .filter_map(|&id| match ast.kind(id) {
            NodeKind::Heading(heading) if heading.level <= depth => {
                let at = ast.span(id).start;
                (!in_region(at)).then(|| (id, heading.level, line_start(at)))
            }
            _ => None,
        })
        .collect();
    bounds
        .iter()
        .enumerate()
        .filter(|&(_, &(_, level, _))| level == depth)
        .map(|(i, &(heading, _, start))| {
            let bound = bounds.get(i + 1).map_or(text.len(), |&(_, _, at)| at);
            let end = start + text[start..bound].trim_end().len();
            SplitSection {
                heading,
                range: ByteRange::new(start, end),
            }
        })
        .collect()
}

/// Where the anchors of `doc` go when its `sections` go to the files `paths` and the
/// rest to `index`: each to the heading's anchor in its file, which may differ where
/// the document had several headings with the same text. Headings are matched in order,
/// and only if the file has as many as the document gives it.
fn link_map(doc: &Document, sections: &[SplitSection], paths: &[PathBuf], index: &Path) -> LinkMap {
    let text = doc.text();
    let ast = doc.ast();
    let section_of = |at: usize| {
        sections
            .iter()
            .position(|s| s.range.start <= at && at < s.range.end)
    };
    let mut own: Vec<Vec<&str>> = vec![Vec::new(); sections.len() + 1];
    for (id, anchor) in doc.anchors().iter() {
        let file = section_of(ast.span(id).start).unwrap_or(sections.len());
        own[file].push(anchor);
    }
    let mut index_text = String::new();
    let mut at = 0;
    for section in sections {
        index_text.push_str(&text[at..section.range.start]);
        index_text.push_str(&marker("", 0));
        at = section.range.end;
    }
    index_text.push_str(&text[at..]);
    let files = sections
        .iter()
        .zip(paths)
        .map(|(section, path)| {
            (
                path.as_path(),
                text[section.range.start..section.range.end].to_owned(),
            )
        })
        .chain([(index, index_text)]);

    let mut map = LinkMap::new();
    for ((path, text), own) in files.zip(own) {
        let file = doc.reparse(SourceFile::new(Some(path.to_path_buf()), text));
        if file.anchors().len() != own.len() {
            continue;
        }
        for ((_, to), from) in file.anchors().iter().zip(own) {
            map.insert_anchor(doc.name(), from, Location::file(path).with_anchor(to));
        }
    }
    map
}

/// `text`, the section in the file `path` of `doc`, with its headings promoted so the
/// shallowest is at level 1, and the levels they were pushed up by; `Err(text)` if
/// shifting them back would not give `text`.
fn promote(doc: &Document, path: &Path, text: String) -> Result<(String, u8), String> {
    let part = doc.reparse(SourceFile::new(Some(path.to_path_buf()), text));
    let ast = part.ast();
    let shallowest = ast
        .descendants(ast.root())
        .filter_map(|id| match ast.kind(id) {
            NodeKind::Heading(heading) => Some(heading.level),
            _ => None,
        })
        .min()
        .unwrap_or(1);
    let by = shallowest - 1;
    let whole = ByteRange::new(0, part.text().len());
    let shifted = shift_headings(&part, whole, ShiftOptions::by(-i32::from(by)));
    if shifted.edits.is_empty() {
        return Ok((part.text().to_owned(), by));
    }
    let promoted = apply_edits(part.text(), &shifted.edits).expect("shift edits do not overlap");
    let back = part.reparse(SourceFile::new(Some(path.to_path_buf()), promoted.clone()));
    let restored = shift_headings(
        &back,
        ByteRange::new(0, promoted.len()),
        ShiftOptions::by(i32::from(by)),
    );
    let restored = apply_edits(&promoted, &restored.edits).expect("shift edits do not overlap");
    if shifted.diagnostics.is_empty() && restored == part.text() {
        Ok((promoted, by))
    } else {
        Err(part.text().to_owned())
    }
}

/// The include directive of the file `name`, with headings pushed down by `shift`.
fn marker(name: &str, shift: u8) -> String {
    if shift == 0 {
        format!("<!-- mrq:include path=\"{name}\" -->")
    } else {
        format!("<!-- mrq:include path=\"{name}\" shift={shift} -->")
    }
}

/// `text` as a file name: lowercase letters and digits, the runs of other characters
/// between them as one `-`, at most 60 characters.
fn file_slug(text: &str) -> String {
    let mut slug = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.chars().take(60).collect();
    slug.trim_end_matches('-').to_owned()
}

/// The directory a document is split into by default: its path without the extension
/// (with `-split` added if it has none), or the current directory for stdin.
fn default_dir(doc: &Document) -> PathBuf {
    let Some(path) = doc.path() else {
        return PathBuf::new();
    };
    let dir = path.with_extension("");
    if dir == path {
        let mut name = dir.into_os_string();
        name.push("-split");
        return name.into();
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentOptions;
    use crate::source::SourceFile;

    fn doc(path: &str, text: &str) -> Document {
        DocumentOptions::new()
            .parse_source(SourceFile::new(Some(path.into()), text))
            .unwrap()
    }

    /// The files of `text` at `path` split by `options`, index first, as (path, text),
    /// asserting there are no diagnostics.
    fn files(path: &str, text: &str, options: &SplitOptions) -> Vec<(String, String)> {
        let split = split(&doc(path, text), options);
        assert_eq!(split.diagnostics, []);
        std::iter::once(split.index)
            .chain(split.parts)
            .map(|file| (file.path.to_string_lossy().into_owned(), file.text))
            .collect()
    }

    fn pairs(files: &[(&str, &str)]) -> Vec<(String, String)> {
        files
            .iter()
            .map(|&(path, text)| (path.to_owned(), text.to_owned()))
            .collect()
    }

    #[test]
    fn sections_become_numbered_files_that_the_index_includes() {
        let text = "---\ntitle: G\n---\n# Guide\n\nIntro.\n\n## Introduction\n\nHello.\n\n\
                    ## Installation\n\n### Linux\n\nRun it.\n\n## Usage\n\nUse it.\n";
        assert_eq!(
            files("guide.md", text, &SplitOptions::new().depth(2)),
            pairs(&[
                (
                    "guide/index.md",
                    "---\ntitle: G\n---\n# Guide\n\nIntro.\n\n\
                     <!-- mrq:include path=\"01-introduction.md\" shift=1 -->\n\n\
                     <!-- mrq:include path=\"02-installation.md\" shift=1 -->\n\n\
                     <!-- mrq:include path=\"03-usage.md\" shift=1 -->\n"
                ),
                ("guide/01-introduction.md", "# Introduction\n\nHello.\n"),
                (
                    "guide/02-installation.md",
                    "# Installation\n\n## Linux\n\nRun it.\n"
                ),
                ("guide/03-usage.md", "# Usage\n\nUse it.\n"),
            ])
        );
    }

    #[test]
    fn headings_above_the_depth_stay_in_the_index() {
        let text = "Pre\n\n## A\n\na\n\n# Part two\n\nIntro two\n\n## B\n\nb";
        assert_eq!(
            files("p.md", text, &SplitOptions::new().depth(2).dir("out")),
            pairs(&[
                (
                    "out/index.md",
                    "Pre\n\n<!-- mrq:include path=\"01-a.md\" shift=1 -->\n\n# Part two\n\n\
                     Intro two\n\n<!-- mrq:include path=\"02-b.md\" shift=1 -->"
                ),
                ("out/01-a.md", "# A\n\na\n"),
                ("out/02-b.md", "# B\n\nb\n"),
            ])
        );
        assert_eq!(
            files("p.md", "# A\n\n## Sub\n# B\n", &SplitOptions::new()),
            pairs(&[
                (
                    "p/index.md",
                    "<!-- mrq:include path=\"01-a.md\" -->\n<!-- mrq:include path=\"02-b.md\" -->\n"
                ),
                ("p/01-a.md", "# A\n\n## Sub\n"),
                ("p/02-b.md", "# B\n"),
            ])
        );
    }

    #[test]
    fn links_between_sections_become_file_links() {
        let text = "[Start](#setup)\n\n# Setup\n\nSee [usage](#usage), [its setup](#setup-1), \
                    [here](#setup), [nowhere](#nowhere) and [self](guide.md#usage).\n\n\
                    # Usage\n\n## Setup\n\nBack to [setup](#setup), [mine](#setup-1).\n";
        assert_eq!(
            files("guide.md", text, &SplitOptions::new()),
            pairs(&[
                (
                    "guide/index.md",
                    "[Start](01-setup.md#setup)\n\n<!-- mrq:include path=\"01-setup.md\" -->\n\n\
                     <!-- mrq:include path=\"02-usage.md\" -->\n"
                ),
                (
                    "guide/01-setup.md",
                    "# Setup\n\nSee [usage](02-usage.md#usage), [its setup](02-usage.md#setup), \
                     [here](#setup), [nowhere](#nowhere) and [self](02-usage.md#usage).\n"
                ),
                (
                    "guide/02-usage.md",
                    "# Usage\n\n## Setup\n\nBack to [setup](01-setup.md#setup), [mine](#setup).\n"
                ),
            ])
        );
        let text = "# Guide\n\n## A\n\nUp to [the guide](#guide).\n";
        assert_eq!(
            files("guide.md", text, &SplitOptions::new().depth(2))[1],
            (
                "guide/01-a.md".to_owned(),
                "# A\n\nUp to [the guide](index.md#guide).\n".to_owned()
            )
        );
    }

    #[test]
    fn assets_are_rebased_to_the_directory() {
        let text = "![logo](img/logo.png)\n\n# A\n\n[b](other.md) ![x](img/x.png) [abs](/r.md)\n";
        assert_eq!(
            files("docs/guide.md", text, &SplitOptions::new()),
            pairs(&[
                (
                    "docs/guide/index.md",
                    "![logo](../img/logo.png)\n\n<!-- mrq:include path=\"01-a.md\" -->\n"
                ),
                (
                    "docs/guide/01-a.md",
                    "# A\n\n[b](../other.md) ![x](../img/x.png) [abs](/r.md)\n"
                ),
            ])
        );
    }

    #[test]
    fn file_names_are_slugified_and_numbered() {
        let parts = (5..=10)
            .map(|n| format!("\n# Part {n}\n"))
            .collect::<Vec<_>>()
            .concat();
        let text = format!("# Hello, *World*!\n\n# C++ & Rust\n\n#\n\n# Über uns\n{parts}");
        let names: Vec<String> = split(&doc("x.md", &text), &SplitOptions::new().dir(""))
            .parts
            .into_iter()
            .map(|part| part.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "01-hello-world.md",
                "02-c-rust.md",
                "03.md",
                "04-über-uns.md",
                "05-part-5.md",
                "06-part-6.md",
                "07-part-7.md",
                "08-part-8.md",
                "09-part-9.md",
                "10-part-10.md",
            ]
        );
    }

    #[test]
    fn promotion_stops_at_shallower_headings_and_skips_setext_headings() {
        assert_eq!(
            files("q.md", "### A\n\n> ## Q\n", &SplitOptions::new().depth(3)),
            pairs(&[
                (
                    "q/index.md",
                    "<!-- mrq:include path=\"01-a.md\" shift=1 -->\n"
                ),
                ("q/01-a.md", "## A\n\n> # Q\n"),
            ])
        );
        let split = split(
            &doc("s.md", "# T\n\nA\n---\n\ntext\n"),
            &SplitOptions::new().depth(2),
        );
        assert_eq!(
            split.index.text,
            "# T\n\n<!-- mrq:include path=\"01-a.md\" -->\n"
        );
        assert_eq!(split.parts[0].text, "A\n---\n\ntext\n");
        let codes: Vec<&str> = split.diagnostics.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, [SPLIT_PROMOTE_CODE]);
    }

    #[test]
    fn a_flavor_from_the_frontmatter_goes_with_each_file() {
        let text = "---\nmrq:\n  flavor: mkdocs\n---\n\n# A {#a}\n";
        assert_eq!(
            files("m.md", text, &SplitOptions::new()),
            pairs(&[
                (
                    "m/index.md",
                    "---\nmrq:\n  flavor: mkdocs\n---\n\n<!-- mrq:include path=\"01-a.md\" -->\n"
                ),
                (
                    "m/01-a.md",
                    "---\nmrq:\n  flavor: mkdocs\n---\n\n# A {#a}\n"
                ),
            ])
        );
    }

    #[test]
    fn a_document_without_sections_is_all_index() {
        let split = split(
            &doc("docs/a.md", "# Title\n\n![x](x.png)\n"),
            &SplitOptions::new().depth(2),
        );
        assert_eq!(split.parts, []);
        assert_eq!(split.index.path, Path::new("docs/a/index.md"));
        assert_eq!(split.index.text, "# Title\n\n![x](../x.png)\n");
    }
}
