//! Acceptance tests for `mrq_core::stats`: per-document counters, totals, reading time,
//! the JSON report and the human table.

use mrq_core::Document;
use mrq_core::document::DocumentOptions;
use mrq_core::stats::{self, Counts, Report, StatsOptions};

fn counts(src: &str) -> Counts {
    Counts::of(&Document::parse(src), &StatsOptions::new())
}

#[test]
fn words_span_inline_markup_and_skip_punctuation() {
    let c = counts("# Hello *wor*ld\n\nIt's a [link](x) — and `code`, too.\n");
    // Hello, world, It's, a, link, and, code, too
    assert_eq!(c.words, 8);
}

#[test]
fn words_exclude_code_blocks_math_frontmatter_html_and_comments_by_default() {
    let src = "---\ntitle: Many words in here\n---\n\none two\n\n```rust\nfn main() {}\n```\n\n    indented code here\n\n<div>html block words</div>\n\n<!-- a comment -->\n\nthree\n";
    assert_eq!(counts(src).words, 3);
}

#[test]
fn include_code_counts_code_blocks_but_not_frontmatter() {
    let src = "---\ntitle: Many words\n---\n\none two\n\n```rust\nlet x = y;\n```\n";
    let doc = Document::parse(src);
    let c = Counts::of(&doc, &StatsOptions::new().include_code(true));
    // one, two, let, x, y
    assert_eq!(c.words, 5);
}

#[test]
fn obsidian_comments_and_math_are_not_words() {
    let doc = DocumentOptions::new()
        .flavor("obsidian")
        .parse("one %%hidden words%% two $x + y$\n\n$$\na = b\n$$\n")
        .unwrap();
    assert_eq!(Counts::of(&doc, &StatsOptions::new()).words, 2);
    let with_code = Counts::of(&doc, &StatsOptions::new().include_code(true));
    assert_eq!(with_code.words, 2 + 2 + 2);
}

#[test]
fn characters_are_non_whitespace_graphemes_of_the_counted_text() {
    // "Héllo" is 5 graphemes; the flag emoji is one; spaces don't count.
    let c = counts("Héllo 🇩🇪 *x*\n\n```\nignored\n```\n");
    assert_eq!(c.characters, 7);
}

#[test]
fn headings_are_counted_per_level() {
    let c = counts("# A\n\n## B\n\n## C\n\n###### F\n\nSetext\n======\n");
    assert_eq!(
        [
            c.headings.h1,
            c.headings.h2,
            c.headings.h3,
            c.headings.h4,
            c.headings.h5,
            c.headings.h6
        ],
        [2, 2, 0, 0, 0, 1]
    );
    assert_eq!(c.headings.total(), 5);
}

#[test]
fn links_are_internal_or_external() {
    let c = counts(
        "[a](other.md) [b](#anchor) [c](https://example.com) [d](mailto:x@y.z) <https://e.org> www.f.com [g](//cdn.x/y) [h](../up.md#x)\n",
    );
    assert_eq!(c.links.internal, 3);
    assert_eq!(c.links.external, 5);
}

#[test]
fn wikilinks_are_internal_and_image_embeds_are_images() {
    let doc = DocumentOptions::new()
        .flavor("obsidian")
        .parse("[[Note]] [[Other#Heading|alias]] ![[pic.PNG]] ![[Embedded note]] ![alt](i.svg)\n")
        .unwrap();
    let c = Counts::of(&doc, &StatsOptions::new());
    assert_eq!(c.links.internal, 3);
    assert_eq!(c.links.external, 0);
    assert_eq!(c.images, 2);
}

#[test]
fn code_blocks_are_counted_per_language() {
    let c = counts(
        "```rust\na\n```\n\n```Rust ignore\nb\n```\n\n```sh\nc\n```\n\n```\nd\n```\n\n    e\n",
    );
    assert_eq!(c.code_blocks.total, 5);
    let langs: Vec<(&str, u64)> = c
        .code_blocks
        .languages
        .iter()
        .map(|(lang, n)| (lang.as_str(), *n))
        .collect();
    // Languages are case-folded; unlabelled blocks only count towards the total.
    assert_eq!(langs, [("rust", 2), ("sh", 1)]);
}

#[test]
fn tables_footnotes_and_tasks() {
    let src = "| a | b |\n| - | - |\n| 1 | 2 |\n\nText[^1] and[^2].\n\n[^1]: One.\n[^2]: Two.\n\n- [ ] open\n- [x] done\n- [X] also done\n  - [ ] nested open\n- plain\n";
    let c = counts(src);
    assert_eq!(c.tables, 1);
    assert_eq!(c.footnotes, 2);
    assert_eq!((c.tasks.open, c.tasks.done), (2, 2));
    assert_eq!(c.tasks.total(), 4);
}

#[test]
fn counts_add_up() {
    let a = counts("# One two\n\n- [ ] x\n\n```rust\n```\n");
    let b = counts("three [l](https://x.y)\n\n```rust\n```\n\n```go\n```\n");
    let total: Counts = [a.clone(), b.clone()].into_iter().sum();
    assert_eq!(total.words, a.words + b.words);
    assert_eq!(total.headings.h1, 1);
    assert_eq!(total.links.external, 1);
    assert_eq!(total.tasks.open, 1);
    assert_eq!(total.code_blocks.total, 3);
    assert_eq!(total.code_blocks.languages["rust"], 2);
    assert_eq!(total.code_blocks.languages["go"], 1);
}

#[test]
fn reading_minutes_round_up() {
    assert_eq!(stats::reading_minutes(0, 200), 0);
    assert_eq!(stats::reading_minutes(1, 200), 1);
    assert_eq!(stats::reading_minutes(200, 200), 1);
    assert_eq!(stats::reading_minutes(201, 200), 2);
    assert_eq!(stats::reading_minutes(360, 180), 2);
}

fn report() -> Report {
    let mut report = Report::new(200);
    let options = StatsOptions::new();
    let a = DocumentOptions::new()
        .parse_source(mrq_core::source::SourceFile::new(
            Some("docs/a.md".into()),
            "# A\n\nSome words here.\n\n- [ ] todo\n".to_owned(),
        ))
        .unwrap();
    report.push(&a, Counts::of(&a, &options));
    let b = Document::parse("## B\n\n```rust\nx\n```\n\n[l](https://x.y)\n");
    report.push(&b, Counts::of(&b, &options));
    report
}

#[test]
fn report_has_files_and_total() {
    let report = report();
    assert_eq!(report.files.len(), 2);
    assert_eq!(report.files[0].file.as_deref(), Some("docs/a.md"));
    assert_eq!(report.files[1].file, None);
    assert_eq!(report.total.files, 2);
    // A, Some, words, here, todo + B, l
    assert_eq!(report.total.counts.words, 7);
    assert_eq!(report.total.reading_minutes, 1);
    assert_eq!(report.files[0].reading_minutes, 1);
}

#[test]
fn report_json_is_flat_and_round_trips() {
    let report = report();
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["wpm"], 200);
    assert_eq!(json["total"]["words"], 7);
    assert_eq!(json["total"]["files"], 2);
    assert_eq!(json["files"][0]["file"], "docs/a.md");
    assert_eq!(json["files"][0]["headings"]["h1"], 1);
    assert_eq!(json["files"][0]["tasks"]["open"], 1);
    assert_eq!(json["files"][1]["code_blocks"]["languages"]["rust"], 1);
    assert_eq!(json["files"][1]["links"]["external"], 1);
    let back: Report = serde_json::from_value(json).unwrap();
    assert_eq!(back, report);
}

#[test]
fn table_aligns_columns_and_adds_a_total_row_for_several_files() {
    insta::assert_snapshot!(report().table());
}

#[test]
fn table_of_one_file_has_no_total_row() {
    let mut report = Report::new(200);
    let doc = Document::parse("one two\n");
    report.push(&doc, Counts::of(&doc, &StatsOptions::new()));
    let table = report.table();
    assert_eq!(table.lines().count(), 2, "{table}");
    assert!(table.lines().nth(1).unwrap().starts_with("<stdin>"));
}
