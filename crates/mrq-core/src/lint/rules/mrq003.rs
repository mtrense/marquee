//! MRQ003 `frontmatter-key-order`.

use std::cmp::Ordering;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Document;
use crate::ast::{FrontMatterFormat, Span};
use crate::diag::{Diagnostic, Fix};
use crate::frontmatter::Frontmatter;
use crate::lint::{Rule, RuleMeta};
use crate::source::ByteRange;

use super::edit;

/// Frontmatter keys out of order.
///
/// Top-level keys listed in `order` come first, in that order; the others follow, sorted
/// by name (ignoring case first) with `alphabetical`, else in any order. Each key that
/// should come before the key above it is reported. In TOML, keys before the first table
/// header and tables declared with a header (`[extra]`), which must come after them, are
/// ordered separately. Nested keys are not checked. With neither option set the rule
/// does nothing, so it is off by default; configuring it turns it on.
///
/// The fix (on the first finding) sorts YAML block mappings whose keys start their lines:
/// each key moves with its value and the comment lines right above it, and comments
/// before the first key stay on top. TOML and flow mappings (`{b: 1, a: 2}`) are not
/// fixed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mrq003;

/// Options of MRQ003 `frontmatter-key-order`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Mrq003Options {
    /// Keys that come first, in this order.
    pub order: Vec<String>,
    /// Sort the keys not in `order` by name.
    pub alphabetical: bool,
}

/// A top-level key and where it sorts.
#[derive(Debug)]
struct Key<'a> {
    name: &'a str,
    /// Its key (or, without one, its value), the start of which orders keys in source.
    span: Span,
    /// TOML tables declared with a header sort after the other keys.
    table: bool,
    /// Index in `order`, or `order.len()` for unlisted keys.
    rank: usize,
}

impl Key<'_> {
    fn cmp(&self, other: &Self, alphabetical: bool) -> Ordering {
        let by_name = |a: &str, b: &str| {
            if alphabetical && self.rank == other.rank {
                a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b))
            } else {
                Ordering::Equal
            }
        };
        (self.table, self.rank)
            .cmp(&(other.table, other.rank))
            .then_with(|| by_name(self.name, other.name))
    }
}

impl Rule for Mrq003 {
    type Options = Mrq003Options;

    const META: RuleMeta = RuleMeta {
        id: "MRQ003",
        aliases: &["frontmatter-key-order"],
        tags: &["frontmatter", "mrq"],
        description: "Frontmatter keys out of order",
        default_enabled: false,
    };

    fn check(&self, doc: &Document, options: &Mrq003Options) -> Vec<Diagnostic> {
        let Some(frontmatter) = doc.frontmatter() else {
            return Vec::new();
        };
        if options.order.is_empty() && !options.alphabetical {
            return Vec::new();
        }
        let keys = keys(doc.text(), frontmatter, &options.order);
        let mut diagnostics = Vec::new();
        for pair in keys.windows(2) {
            let (before, key) = (&pair[0], &pair[1]);
            if key.cmp(before, options.alphabetical) == Ordering::Less {
                let message = format!(
                    "frontmatter key `{}` should come before `{}`",
                    key.name, before.name
                );
                diagnostics.push(Self::META.diagnostic(doc, key.span, message));
            }
        }
        if let Some(first) = diagnostics.first_mut()
            && let Some(fix) = sort_fix(doc.text(), frontmatter, &keys, options.alphabetical)
        {
            first.fix = Some(fix);
        }
        diagnostics
    }
}

/// The top-level keys of `frontmatter` in source order.
fn keys<'a>(text: &str, frontmatter: &'a Frontmatter, order: &[String]) -> Vec<Key<'a>> {
    let Some(map) = frontmatter.value().and_then(|value| value.as_object()) else {
        return Vec::new();
    };
    let toml = frontmatter.format() == FrontMatterFormat::Toml;
    let mut keys: Vec<Key<'a>> = map
        .keys()
        .filter_map(|name| {
            let entry = frontmatter.entry(&[name])?;
            let span = entry.key.unwrap_or(entry.value);
            let table = toml && text[line_start(text, span.start)..].starts_with('[');
            let rank = order
                .iter()
                .position(|listed| listed == name)
                .unwrap_or(order.len());
            Some(Key {
                name,
                span,
                table,
                rank,
            })
        })
        .collect();
    keys.sort_by_key(|key| key.span.start);
    keys
}

/// The edit that sorts the entries of a YAML block mapping, if `keys` all start lines.
fn sort_fix(
    text: &str,
    frontmatter: &Frontmatter,
    keys: &[Key<'_>],
    alphabetical: bool,
) -> Option<Fix> {
    if frontmatter.format() != FrontMatterFormat::Yaml
        || keys
            .iter()
            .any(|key| line_start(text, key.span.start) != key.span.start)
    {
        return None;
    }
    let content = frontmatter.content();
    // Each entry starts at its key's line, or at the comment lines right above it; those
    // above the first key stay where they are.
    let starts: Vec<usize> = keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let mut start = key.span.start;
            while i > 0 && start > content.start {
                let above = line_start(text, start - 1);
                if !text[above..].starts_with('#') {
                    break;
                }
                start = above;
            }
            start
        })
        .collect();
    let mut entries: Vec<(&Key<'_>, &str)> = keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let end = starts.get(i + 1).copied().unwrap_or(content.end);
            (key, &text[starts[i]..end])
        })
        .collect();
    entries.sort_by(|(a, _), (b, _)| a.cmp(b, alphabetical));
    let sorted: String = entries.iter().map(|(_, entry)| *entry).collect();
    let range = ByteRange::new(*starts.first()?, content.end);
    Some(Fix::new(
        "Sort the frontmatter keys",
        vec![edit(range, sorted)],
    ))
}

/// The start of the line holding `offset`.
fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |newline| newline + 1)
}
