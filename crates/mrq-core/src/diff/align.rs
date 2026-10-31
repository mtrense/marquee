//! Alignment of two section trees and the diff of their blocks; see
//! [Alignment](super#alignment).

use std::collections::{HashMap, VecDeque};

use serde::Serialize;
use similar::{Algorithm, ChangeTag, DiffOp, DiffTag, capture_diff_slices, diff_ratio};

use super::{Block, Section, normalize};
use crate::Document;
use crate::ast::Span;

/// How much the title similarity weighs in the score of two sections whose paths
/// differ; the similarity of their content makes up the rest.
pub const TITLE_WEIGHT: f32 = 0.4;

/// The lowest score at which two sections whose paths differ are taken to be the same
/// section, moved or renamed.
pub const SECTION_MATCH_THRESHOLD: f32 = 0.5;

/// The lowest similarity at which a deleted block and an inserted block of the same
/// kind are reported as one modified block.
pub const BLOCK_MODIFY_THRESHOLD: f32 = 0.4;

/// The diff algorithm for block sequences, word ops and similarities.
const ALGORITHM: Algorithm = Algorithm::Patience;

/// Options for [`diff`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffOptions {
    /// Diff modified blocks word by word ([`BlockOp::Modify::words`]).
    pub word: bool,
}

/// The differences between two documents, section by section.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocDiff {
    /// Every section of both documents: the new document's in its order, each removed
    /// one after the section that the one before it in the old document matched. The
    /// root sections come first.
    pub sections: Vec<SectionDiff>,
}

impl DocDiff {
    /// Whether the documents differ only in formatting.
    pub fn is_empty(&self) -> bool {
        self.sections.iter().all(SectionDiff::is_unchanged)
    }

    /// The sections that changed.
    pub fn changes(&self) -> impl Iterator<Item = &SectionDiff> {
        self.sections
            .iter()
            .filter(|section| !section.is_unchanged())
    }
}

/// Whether a section is in one document or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionStatus {
    /// Only in the new document.
    Added,
    /// Only in the old document.
    Removed,
    /// In both; see [`SectionDiff::moved`], [`renamed`](SectionDiff::renamed) and
    /// [`modified`](SectionDiff::modified).
    Matched,
}

/// One section of either document or both, and how it changed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SectionDiff {
    /// Which documents the section is in.
    pub status: SectionStatus,
    /// Matched under another parent, or out of order among its siblings.
    pub moved: bool,
    /// Matched with another title.
    pub renamed: bool,
    /// Some block op is not [`BlockOp::Equal`], the heading's aside if only the title
    /// changed (that is [`renamed`](Self::renamed)); always set for added and removed
    /// sections.
    pub modified: bool,
    /// The section in the old document; `None` if added.
    pub old: Option<SectionSide>,
    /// The section in the new document; `None` if removed.
    pub new: Option<SectionSide>,
    /// The heading (if any) and the blocks of the section, children excluded, in order.
    pub blocks: Vec<BlockOp>,
}

impl SectionDiff {
    /// Matched, neither moved nor renamed, and every block equal.
    pub fn is_unchanged(&self) -> bool {
        self.status == SectionStatus::Matched && !self.moved && !self.renamed && !self.modified
    }
}

/// Where a section is in one of the documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SectionSide {
    /// [`Section::path`]; empty for the root.
    pub path: Vec<String>,
    /// [`Section::title`].
    pub title: String,
    /// [`Section::level`].
    pub level: u8,
    /// [`Section::span`].
    pub span: Span,
    /// The heading's line, from 0; 0 for the root.
    pub line: usize,
}

impl SectionSide {
    fn of(section: &Section) -> Self {
        Self {
            path: section.path.clone(),
            title: section.title.clone(),
            level: section.level,
            span: section.span,
            line: section
                .heading
                .as_ref()
                .map_or(0, |heading| heading.lines.start),
        }
    }
}

/// What happened to a block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BlockOp {
    /// The same canonical form in both documents.
    Equal {
        /// The block in the old document.
        old: Block,
        /// The block in the new document.
        new: Block,
    },
    /// Only in the new document.
    Insert {
        /// The block in the new document.
        new: Block,
    },
    /// Only in the old document.
    Delete {
        /// The block in the old document.
        old: Block,
    },
    /// A changed block: the headings of matched sections, and deleted and inserted
    /// blocks of the same kind that are similar enough ([`BLOCK_MODIFY_THRESHOLD`]).
    Modify {
        /// The block in the old document.
        old: Block,
        /// The block in the new document.
        new: Block,
        /// The canonical forms' word diff, with [`DiffOptions::word`].
        #[serde(skip_serializing_if = "Option::is_none")]
        words: Option<Vec<WordOp>>,
    },
}

/// A run of words (and the whitespace between them) of a modified block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "op", content = "text", rename_all = "snake_case")]
pub enum WordOp {
    /// In both canonical forms.
    Equal(String),
    /// Only in the new canonical form.
    Insert(String),
    /// Only in the old canonical form.
    Delete(String),
}

/// The differences between `old` and `new`; see [Alignment](super#alignment).
pub fn diff(old: &Document, new: &Document, options: &DiffOptions) -> DocDiff {
    let (old, new) = (normalize(old), normalize(new));
    let (old, new) = (flatten(&old), flatten(&new));
    let matching = Matching::new(&old, &new);
    let moved = moved(&old, &new, &matching);

    // Removed sections go after the new partner of the nearest matched section before
    // them in the old document (the root at least).
    let mut removed_after: Vec<Vec<usize>> = vec![Vec::new(); new.len()];
    let mut partner = 0;
    for (index, matched) in matching.old_to_new.iter().enumerate() {
        match matched {
            Some(n) => partner = *n,
            None => removed_after[partner].push(index),
        }
    }

    let mut sections = Vec::with_capacity(old.len().max(new.len()));
    for (n, flat) in new.iter().enumerate() {
        let section = flat.section;
        sections.push(match matching.new_to_old[n] {
            Some(o) => matched(old[o].section, section, moved[n], *options),
            None => SectionDiff {
                status: SectionStatus::Added,
                moved: false,
                renamed: false,
                modified: true,
                old: None,
                new: Some(SectionSide::of(section)),
                blocks: all(section)
                    .map(|block| BlockOp::Insert { new: block.clone() })
                    .collect(),
            },
        });
        for &o in &removed_after[n] {
            let section = old[o].section;
            sections.push(SectionDiff {
                status: SectionStatus::Removed,
                moved: false,
                renamed: false,
                modified: true,
                old: Some(SectionSide::of(section)),
                new: None,
                blocks: all(section)
                    .map(|block| BlockOp::Delete { old: block.clone() })
                    .collect(),
            });
        }
    }
    DocDiff { sections }
}

/// A section with the index of its parent in the flattened tree.
struct Flat<'a> {
    section: &'a Section,
    parent: Option<usize>,
}

/// The sections of `root` in [`Section::iter`] order.
fn flatten(root: &Section) -> Vec<Flat<'_>> {
    let mut out = Vec::new();
    let mut stack = vec![(root, None)];
    while let Some((section, parent)) = stack.pop() {
        let index = out.len();
        out.push(Flat { section, parent });
        stack.extend(
            section
                .children
                .iter()
                .rev()
                .map(|child| (child, Some(index))),
        );
    }
    out
}

/// The heading (if any) and the blocks of `section`.
fn all(section: &Section) -> impl Iterator<Item = &Block> {
    section.heading.iter().chain(&section.blocks)
}

/// Which sections of the old and new flattened trees are the same.
struct Matching {
    old_to_new: Vec<Option<usize>>,
    new_to_old: Vec<Option<usize>>,
}

impl Matching {
    /// The roots match, then sections with the same path (the k-th old one with the
    /// k-th new one), then the remaining pairs by descending [`score`] down to
    /// [`SECTION_MATCH_THRESHOLD`], ties in document order.
    fn new(old: &[Flat<'_>], new: &[Flat<'_>]) -> Self {
        let mut matching = Self {
            old_to_new: vec![None; old.len()],
            new_to_old: vec![None; new.len()],
        };
        matching.pair(0, 0);

        let mut by_path: HashMap<&[String], VecDeque<usize>> = HashMap::new();
        for (o, flat) in old.iter().enumerate().skip(1) {
            by_path.entry(&flat.section.path).or_default().push_back(o);
        }
        for (n, flat) in new.iter().enumerate().skip(1) {
            if let Some(o) = by_path
                .get_mut(flat.section.path.as_slice())
                .and_then(VecDeque::pop_front)
            {
                matching.pair(o, n);
            }
        }

        let unmatched_old: Vec<usize> = (0..old.len())
            .filter(|&o| matching.old_to_new[o].is_none())
            .collect();
        let unmatched_new: Vec<usize> = (0..new.len())
            .filter(|&n| matching.new_to_old[n].is_none())
            .collect();
        let old_profiles: Vec<_> = unmatched_old
            .iter()
            .map(|&o| profile(old[o].section))
            .collect();
        let new_profiles: Vec<_> = unmatched_new
            .iter()
            .map(|&n| profile(new[n].section))
            .collect();
        let mut candidates = Vec::new();
        for (i, &o) in unmatched_old.iter().enumerate() {
            for (j, &n) in unmatched_new.iter().enumerate() {
                let score = score(&old_profiles[i], &new_profiles[j]);
                if score >= SECTION_MATCH_THRESHOLD {
                    candidates.push((score, o, n));
                }
            }
        }
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then((a.1, a.2).cmp(&(b.1, b.2))));
        for (_, o, n) in candidates {
            if matching.old_to_new[o].is_none() && matching.new_to_old[n].is_none() {
                matching.pair(o, n);
            }
        }
        matching
    }

    fn pair(&mut self, old: usize, new: usize) {
        self.old_to_new[old] = Some(new);
        self.new_to_old[new] = Some(old);
    }
}

/// A section's title characters and the [`tokens`] of its own blocks.
type Profile<'a> = (Vec<char>, Vec<&'a str>);

fn profile(section: &Section) -> Profile<'_> {
    let body = section
        .blocks
        .iter()
        .flat_map(|block| tokens(&block.canonical))
        .collect();
    (section.title.chars().collect(), body)
}

/// How alike two sections are, from their title characters and content [`tokens`]:
/// [`TITLE_WEIGHT`] of the title similarity plus the rest of the content similarity,
/// or the title similarity alone if neither has content.
fn score((old_title, old_body): &Profile<'_>, (new_title, new_body): &Profile<'_>) -> f32 {
    let title = similarity(old_title, new_title);
    if old_body.is_empty() && new_body.is_empty() {
        return title;
    }
    TITLE_WEIGHT * title + (1.0 - TITLE_WEIGHT) * similarity(old_body, new_body)
}

/// The share of `old` and `new` that their diff keeps, 0 to 1 (1 for two empty
/// sequences).
fn similarity<T: Eq + std::hash::Hash>(old: &[T], new: &[T]) -> f32 {
    let ops = capture_diff_slices(ALGORITHM, old, new);
    diff_ratio(&ops, old.len(), new.len())
}

/// The words (runs of alphanumeric characters) and the other characters but
/// whitespace of `text`, one by one; what similarities compare.
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    pieces(text).filter(|piece| !piece.starts_with(char::is_whitespace))
}

/// `text` cut into words (runs of alphanumeric characters), runs of whitespace and
/// single other characters.
fn pieces(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let first = rest.chars().next()?;
        let run = |same: fn(char) -> bool| rest.find(|c: char| !same(c)).unwrap_or(rest.len());
        let len = if first.is_alphanumeric() {
            run(char::is_alphanumeric)
        } else if first.is_whitespace() {
            run(char::is_whitespace)
        } else {
            first.len_utf8()
        };
        let (piece, after) = rest.split_at(len);
        rest = after;
        Some(piece)
    })
}

/// For each new section, whether it moved: its parent is not its old parent's
/// partner, or it is not in the longest run of siblings that kept their old order.
fn moved(old: &[Flat<'_>], new: &[Flat<'_>], matching: &Matching) -> Vec<bool> {
    let mut moved = vec![false; new.len()];
    let mut siblings: Vec<Vec<usize>> = vec![Vec::new(); new.len()];
    for (n, flat) in new.iter().enumerate() {
        let (Some(parent), Some(o)) = (flat.parent, matching.new_to_old[n]) else {
            continue;
        };
        if old[o].parent.and_then(|p| matching.old_to_new[p]) == Some(parent) {
            siblings[parent].push(n);
        } else {
            moved[n] = true;
        }
    }
    for children in siblings {
        let order: Vec<usize> = children
            .iter()
            .map(|&n| matching.new_to_old[n].expect("matched"))
            .collect();
        let kept = longest_increasing(&order);
        for (i, &n) in children.iter().enumerate() {
            moved[n] = !kept[i];
        }
    }
    moved
}

/// Which elements of `seq` are in a longest strictly increasing subsequence (the one
/// that ends with the smallest values).
fn longest_increasing(seq: &[usize]) -> Vec<bool> {
    // `tails[k]`: the index of the smallest last element of an increasing run of k + 1.
    let mut tails: Vec<usize> = Vec::new();
    let mut previous = vec![None; seq.len()];
    for (i, &value) in seq.iter().enumerate() {
        let k = tails.partition_point(|&t| seq[t] < value);
        previous[i] = k.checked_sub(1).map(|k| tails[k]);
        if k == tails.len() {
            tails.push(i);
        } else {
            tails[k] = i;
        }
    }
    let mut kept = vec![false; seq.len()];
    let mut at = tails.last().copied();
    while let Some(i) = at {
        kept[i] = true;
        at = previous[i];
    }
    kept
}

/// The diff of two matched sections.
fn matched(old: &Section, new: &Section, moved: bool, options: DiffOptions) -> SectionDiff {
    let mut blocks = Vec::new();
    if let (Some(old), Some(new)) = (&old.heading, &new.heading) {
        blocks.push(if old.canonical == new.canonical {
            BlockOp::Equal {
                old: old.clone(),
                new: new.clone(),
            }
        } else {
            modify(old, new, options)
        });
    }
    let renamed = old.title != new.title;
    // A heading that changed beyond its title: another level or other markup.
    let heading = !blocks.is_empty()
        && !matches!(blocks[0], BlockOp::Equal { .. })
        && (old.level != new.level || !renamed);
    let body = block_ops(&old.blocks, &new.blocks, options);
    let modified = heading || body.iter().any(|op| !matches!(op, BlockOp::Equal { .. }));
    blocks.extend(body);
    SectionDiff {
        status: SectionStatus::Matched,
        moved,
        renamed,
        modified,
        old: Some(SectionSide::of(old)),
        new: Some(SectionSide::of(new)),
        blocks,
    }
}

/// The patience diff of two block sequences by canonical form, each run of deletions
/// and insertions paired up by [`pair_up`].
fn block_ops(old: &[Block], new: &[Block], options: DiffOptions) -> Vec<BlockOp> {
    let mut ops = Vec::new();
    // The deleted and inserted blocks since the last equal ones.
    let mut pending = (0..0, 0..0);
    for op in capture_diff_slices(ALGORITHM, &canonical(old), &canonical(new)) {
        let (_, old_range, new_range) = op.as_tag_tuple();
        if let DiffOp::Equal { .. } = op {
            pair_up(
                &old[pending.0.clone()],
                &new[pending.1.clone()],
                options,
                &mut ops,
            );
            ops.extend(
                old_range
                    .clone()
                    .zip(new_range.clone())
                    .map(|(o, n)| BlockOp::Equal {
                        old: old[o].clone(),
                        new: new[n].clone(),
                    }),
            );
            pending = (old_range.end..old_range.end, new_range.end..new_range.end);
        } else {
            if pending.0.is_empty() && pending.1.is_empty() {
                pending = (
                    old_range.start..old_range.start,
                    new_range.start..new_range.start,
                );
            }
            pending.0.end = old_range.end;
            pending.1.end = new_range.end;
        }
    }
    pair_up(
        &old[pending.0.clone()],
        &new[pending.1.clone()],
        options,
        &mut ops,
    );
    ops
}

fn canonical(blocks: &[Block]) -> Vec<&str> {
    blocks
        .iter()
        .map(|block| block.canonical.as_str())
        .collect()
}

/// Deleted `old` and inserted `new` blocks as ops, in order: each old block is
/// modified into the first new block after the last paired one that has its kind and
/// [`BLOCK_MODIFY_THRESHOLD`] similarity, or else deleted.
fn pair_up(old: &[Block], new: &[Block], options: DiffOptions, ops: &mut Vec<BlockOp>) {
    let mut next = 0;
    for old in old {
        let old_tokens: Vec<&str> = tokens(&old.canonical).collect();
        let partner = new[next..].iter().position(|new| {
            new.kind == old.kind
                && similarity(&old_tokens, &tokens(&new.canonical).collect::<Vec<_>>())
                    >= BLOCK_MODIFY_THRESHOLD
        });
        match partner {
            Some(at) => {
                let at = next + at;
                ops.extend(
                    new[next..at]
                        .iter()
                        .map(|new| BlockOp::Insert { new: new.clone() }),
                );
                ops.push(modify(old, &new[at], options));
                next = at + 1;
            }
            None => ops.push(BlockOp::Delete { old: old.clone() }),
        }
    }
    ops.extend(
        new[next..]
            .iter()
            .map(|new| BlockOp::Insert { new: new.clone() }),
    );
}

/// A [`BlockOp::Modify`], with word ops if `options` asks for them.
fn modify(old: &Block, new: &Block, options: DiffOptions) -> BlockOp {
    let words = options
        .word
        .then(|| word_ops(&old.canonical, &new.canonical));
    BlockOp::Modify {
        old: old.clone(),
        new: new.clone(),
        words,
    }
}

/// The patience diff of the [`pieces`] of `old` and `new`, runs of the same op merged.
fn word_ops(old: &str, new: &str) -> Vec<WordOp> {
    let (old, new): (Vec<&str>, Vec<&str>) = (pieces(old).collect(), pieces(new).collect());
    let mut words = Vec::new();
    for op in capture_diff_slices(ALGORITHM, &old, &new) {
        let (tag, old_range, new_range) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            push_words(&mut words, ChangeTag::Equal, &old[old_range]);
        } else {
            push_words(&mut words, ChangeTag::Delete, &old[old_range]);
            push_words(&mut words, ChangeTag::Insert, &new[new_range]);
        }
    }
    words
}

/// Appends `pieces` to the last of `words` if it has the op `tag`, or as a new one.
fn push_words(words: &mut Vec<WordOp>, tag: ChangeTag, pieces: &[&str]) {
    if pieces.is_empty() {
        return;
    }
    let text = pieces.concat();
    match (words.last_mut(), tag) {
        (Some(WordOp::Equal(run)), ChangeTag::Equal)
        | (Some(WordOp::Delete(run)), ChangeTag::Delete)
        | (Some(WordOp::Insert(run)), ChangeTag::Insert) => run.push_str(&text),
        (_, ChangeTag::Equal) => words.push(WordOp::Equal(text)),
        (_, ChangeTag::Delete) => words.push(WordOp::Delete(text)),
        (_, ChangeTag::Insert) => words.push(WordOp::Insert(text)),
    }
}
