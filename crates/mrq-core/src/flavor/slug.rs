//! Heading slugs: [`Slugger`] with the GitHub and Obsidian implementations.

use std::collections::HashMap;
use std::fmt;

use unicode_general_category::{GeneralCategory, get_general_category};

/// Turns heading text into anchor ids, unique within one document.
///
/// A slugger is stateful: it remembers every slug it returned and suffixes repeats. Use
/// one per document, feeding it the headings in document order
/// (`slugger.slug(&ast.text(heading))`), and [`reset`](Self::reset) it before reuse.
pub trait Slugger: fmt::Debug + Send {
    /// The anchor for `text`, unique among the slugs returned since the last reset.
    fn slug(&mut self, text: &str) -> String;

    /// Forgets every slug returned so far.
    fn reset(&mut self);
}

/// GitHub's heading anchors, as implemented by
/// [github-slugger](https://github.com/Flet/github-slugger): see [`github_slug`].
///
/// A slug that was already returned gets the first free suffix `-1`, `-2`, … of its base
/// slug; a suffixed slug counts as taken too, so `a`, `a`, `a-1` give `a`, `a-1`, `a-1-1`.
#[derive(Debug, Clone, Default)]
pub struct GithubSlugger {
    taken: Taken,
}

impl GithubSlugger {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Slugger for GithubSlugger {
    fn slug(&mut self, text: &str) -> String {
        self.taken.claim(&github_slug(text))
    }

    fn reset(&mut self) {
        self.taken.clear();
    }
}

/// Obsidian's heading anchors: see [`obsidian_anchor`].
///
/// Obsidian links to a heading by its text (`[[Note#My Heading]]`, compared
/// case-insensitively) rather than by a slug, so the anchor is the heading text itself,
/// lightly normalized. Obsidian has no anchor for a repeated heading (links go to the
/// first one); to keep anchors unique this slugger suffixes repeats like
/// [`GithubSlugger`] does: `My Heading`, `My Heading-1`, …
#[derive(Debug, Clone, Default)]
pub struct ObsidianSlugger {
    taken: Taken,
}

impl ObsidianSlugger {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Slugger for ObsidianSlugger {
    fn slug(&mut self, text: &str) -> String {
        self.taken.claim(&obsidian_anchor(text))
    }

    fn reset(&mut self) {
        self.taken.clear();
    }
}

/// The Obsidian anchor of a heading with plain text `text`, without duplicate tracking.
///
/// The characters Obsidian does not allow in a link target (`#`, `|`, `^`, `:`, `%`,
/// `[`, `]` and `\`) become spaces, then runs of whitespace collapse to one space and
/// the ends are trimmed. Case and everything else are kept, so `Setup: Linux` gives
/// `Setup Linux`, as in `[[Note#Setup Linux]]`.
pub fn obsidian_anchor(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| match c {
            '#' | '|' | '^' | ':' | '%' | '[' | ']' | '\\' => ' ',
            c => c,
        })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The anchors a slugger has returned, for duplicate suffixes.
#[derive(Debug, Clone, Default)]
struct Taken {
    /// Every anchor returned so far → how many times it has been suffixed as a base.
    occurrences: HashMap<String, usize>,
}

impl Taken {
    /// `base`, or its first free suffixed form `base-1`, `base-2`, …; marks it taken.
    fn claim(&mut self, base: &str) -> String {
        let mut slug = base.to_owned();
        while self.occurrences.contains_key(&slug) {
            let count = self
                .occurrences
                .get_mut(base)
                .expect("the base slug is taken before any suffix is tried");
            *count += 1;
            slug = format!("{base}-{count}");
        }
        self.occurrences.insert(slug.clone(), 0);
        slug
    }

    fn clear(&mut self) {
        self.occurrences.clear();
    }
}

/// The GitHub slug of `text`, without duplicate tracking.
///
/// The text is lowercased, every space becomes `-`, and everything is removed except
/// letters, marks, decimal and letter numbers, connector punctuation (`_`), `-`, and
/// characters with the Unicode `Alphabetic` property (such as circled letters `ⓐ`).
/// So other punctuation, symbols (emoji included), other separators and control
/// characters are dropped; leading or repeated spaces and dashes are kept as they are.
///
/// github-slugger uses Unicode 13 tables; this uses the newer tables of
/// `unicode-general-category` and `std`, so characters assigned since Unicode 13 are
/// kept when they are letters or numbers (github-slugger drops them as unassigned).
pub fn github_slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' | '-' => Some('-'),
            c if keeps(c) => Some(c),
            _ => None,
        })
        .collect()
}

/// Whether a (non-space, non-dash) character survives in a GitHub slug.
fn keeps(c: char) -> bool {
    use GeneralCategory as G;
    c.is_alphabetic()
        || matches!(
            get_general_category(c),
            G::UppercaseLetter
                | G::LowercaseLetter
                | G::TitlecaseLetter
                | G::ModifierLetter
                | G::OtherLetter
                | G::NonspacingMark
                | G::SpacingMark
                | G::EnclosingMark
                | G::DecimalNumber
                | G::LetterNumber
                | G::ConnectorPunctuation
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_rules() {
        assert_eq!(github_slug("Hello, World!"), "hello-world");
        assert_eq!(github_slug(" a  b "), "-a--b-");
        assert_eq!(github_slug("a\tb\nc"), "abc");
        assert_eq!(github_slug("ⓐ ½ ²"), "ⓐ--");
        assert_eq!(github_slug("snake_case – done"), "snake_case--done");
    }

    #[test]
    fn obsidian_anchor_rules() {
        assert_eq!(obsidian_anchor("  Setup:\tLinux  "), "Setup Linux");
        assert_eq!(obsidian_anchor("a#b|c^d%e[f]g\\h"), "a b c d e f g h");
        assert_eq!(obsidian_anchor("Ünïcödé — “q”!"), "Ünïcödé — “q”!");
    }

    #[test]
    fn suffixed_slugs_are_taken_too() {
        let mut slugger = GithubSlugger::new();
        let slugs: Vec<_> = ["a", "a-1", "a", "a"]
            .iter()
            .map(|t| slugger.slug(t))
            .collect();
        assert_eq!(slugs, ["a", "a-1", "a-2", "a-3"]);
    }
}
