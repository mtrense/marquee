//! Search and query results: what `/` and `>` found in the shown file.
//!
//! A search looks for its pattern in the text of the laid-out lines, as drawn, with
//! smart case ([`find_all`]); a match never spans lines. A query is a [`Query`] in one
//! of the three syntaxes; its matches are the blocks of the layout that show (part of)
//! a matched node or section. Either way the [`Hit`]s are in line order, and `n`/`N`
//! visit them in turn.
//!
//! A [`Find`] applies to whichever file is shown, laid out at whatever width: the app
//! refreshes it after every layout, which redoes the search or query
//! when the file or the width changed.

use std::ops::Range;

use mrq_core::query::{Match, Query, QueryError, Syntax};
use mrq_render::ansi::Line;

use crate::app::FileView;
use crate::ui;

/// One place a search or query found: a line, and for a search the matched bytes of the
/// line's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The index of the line in the layout; for a query, the first line of the first
    /// block of the match.
    pub line: usize,
    /// The matched bytes of the line's text as drawn (its [text](Line::text) unless it
    /// shows an image); empty for a query.
    pub columns: Range<usize>,
}

/// What is looked for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Wanted {
    /// A search for this pattern.
    Search(String),
    /// A query, typed as this text (with its `css:` or `jq:` prefix).
    Query(Query, String),
}

/// An active search or query and its hits in the shown file; see the [module
/// docs](self).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Find {
    wanted: Wanted,
    /// The file index and layout width the hits are for.
    key: Option<(usize, usize)>,
    hits: Vec<Hit>,
    /// For a query, whether each block of the layout shows part of a match.
    marked: Vec<bool>,
    current: Option<usize>,
}

impl Find {
    /// A search for `pattern`, not run yet.
    pub(crate) fn search(pattern: &str) -> Self {
        Self::new(Wanted::Search(pattern.to_owned()))
    }

    /// The query `query`, typed as `text`, not run yet.
    pub(crate) fn query(query: Query, text: &str) -> Self {
        Self::new(Wanted::Query(query, text.to_owned()))
    }

    fn new(wanted: Wanted) -> Self {
        Self {
            wanted,
            key: None,
            hits: Vec::new(),
            marked: Vec::new(),
            current: None,
        }
    }

    /// The search pattern, or the query as typed.
    pub fn pattern(&self) -> &str {
        match &self.wanted {
            Wanted::Search(pattern) => pattern,
            Wanted::Query(_, text) => text,
        }
    }

    pub fn is_query(&self) -> bool {
        matches!(self.wanted, Wanted::Query(..))
    }

    /// The hits in the shown file, in line order.
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// The index of the hit `n`/`N` went to last; `None` until they go to one in the
    /// shown file.
    pub fn current(&self) -> Option<usize> {
        self.current
    }

    /// The search hits on line `line`, left to right.
    pub fn hits_on(&self, line: usize) -> &[Hit] {
        if self.is_query() {
            return &[];
        }
        let start = self.hits.partition_point(|hit| hit.line < line);
        let end = self.hits.partition_point(|hit| hit.line <= line);
        &self.hits[start..end]
    }

    /// Whether block `index` of the view shows part of a query match.
    pub fn is_marked(&self, index: usize) -> bool {
        self.marked.get(index).copied().unwrap_or(false)
    }

    pub(crate) fn set_current(&mut self, index: usize) {
        self.current = Some(index);
    }

    /// Finds the hits in `file`, the file of index `index` laid out `width` columns wide,
    /// unless they are for it already.
    ///
    /// # Errors
    ///
    /// The error of a jq program that fails in `file`; it finds nothing there.
    pub(crate) fn refresh(
        &mut self,
        index: usize,
        width: usize,
        file: &FileView,
    ) -> Result<(), QueryError> {
        if self.key == Some((index, width)) {
            return Ok(());
        }
        self.key = Some((index, width));
        self.current = None;
        self.marked.clear();
        self.hits.clear();
        let query = match &self.wanted {
            Wanted::Search(pattern) => {
                self.hits = search(file.lines(), pattern);
                return Ok(());
            }
            Wanted::Query(query, _) => query.clone(),
        };
        self.hits = self.run_query(&query, file)?;
        Ok(())
    }

    /// The hits of `query` in `file`, marking the blocks of its matches: one hit per first
    /// line, in line order. jq outputs that are not nodes or sections are left out.
    fn run_query(&mut self, query: &Query, file: &FileView) -> Result<Vec<Hit>, QueryError> {
        let document = file.document();
        let spans: Vec<_> = if query.syntax() == Syntax::Jq {
            document
                .select_json(query)?
                .iter()
                .filter_map(|output| output.to_match(document))
                .map(|found| found.span())
                .collect()
        } else {
            document.select(query)?.iter().map(Match::span).collect()
        };
        let blocks = file.view().blocks();
        self.marked = vec![false; blocks.len()];
        let mut lines = Vec::new();
        for span in spans {
            // The blocks inside the match, or the one it is inside of.
            let mut first = None;
            for (i, block) in blocks.iter().enumerate() {
                if block.span.start < span.end && span.start < block.span.end {
                    self.marked[i] = true;
                    first = first.or(Some(block.lines.start));
                }
            }
            lines.extend(first);
        }
        lines.sort_unstable();
        lines.dedup();
        Ok(lines
            .into_iter()
            .map(|line| Hit {
                line,
                columns: 0..0,
            })
            .collect())
    }
}

/// The hits of the search for `pattern` in `lines`.
fn search(lines: &[Line], pattern: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (line, styled) in lines.iter().enumerate() {
        let text: String = ui::drawn(styled).iter().map(|s| s.text.as_str()).collect();
        hits.extend(
            find_all(&text, pattern)
                .into_iter()
                .map(|columns| Hit { line, columns }),
        );
    }
    hits
}

/// The byte ranges of `text` where `pattern` occurs, left to right and without overlaps.
///
/// The search is smart-case: a pattern without upper-case letters ignores case, one with
/// any matches it exactly. An empty pattern occurs nowhere.
pub fn find_all(text: &str, pattern: &str) -> Vec<Range<usize>> {
    let pattern: Vec<char> = pattern.chars().collect();
    if pattern.is_empty() {
        return Vec::new();
    }
    let fold = !pattern.iter().any(|c| c.is_uppercase());
    let same = |c: char, p: char| c == p || (fold && c.to_lowercase().eq(p.to_lowercase()));
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i + pattern.len() <= chars.len() {
        let window = &chars[i..i + pattern.len()];
        if window.iter().zip(&pattern).all(|(&(_, c), &p)| same(c, p)) {
            let end = chars
                .get(i + pattern.len())
                .map_or(text.len(), |&(at, _)| at);
            found.push(chars[i].0..end);
            i += pattern.len();
        } else {
            i += 1;
        }
    }
    found
}
