//! Acceptance tests for `mrq_core::toc::refresh_edits`: regenerating the body of
//! `<!-- mrq:toc -->` regions with text edits that leave every other byte alone.

use mrq_core::Document;
use mrq_core::source::apply_edits;
use mrq_core::toc;
use proptest::prelude::*;

/// The document after one refresh with `depth`.
fn refresh(src: &str, depth: Option<u8>) -> String {
    let doc = Document::parse(src);
    let refresh = toc::refresh_edits(&doc, depth);
    assert!(
        refresh.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        refresh.diagnostics
    );
    apply_edits(src, &refresh.edits).unwrap()
}

/// Asserts that refreshing `src` gives `expected`, and that refreshing that again
/// produces no edits at all.
fn assert_refresh(src: &str, depth: Option<u8>, expected: &str) {
    let once = refresh(src, depth);
    assert_eq!(once, expected);
    let again = toc::refresh_edits(&Document::parse(&once), depth);
    assert_eq!(
        again.edits,
        vec![],
        "second refresh of {once:?} is not a no-op"
    );
}

#[test]
fn stale_region_is_replaced_by_the_link_list() {
    assert_refresh(
        "# Title\n\n<!-- mrq:toc -->\n- [Old](#old)\n<!-- mrq:end -->\n\n## Install\n\n## Use it\n",
        None,
        "# Title\n\n<!-- mrq:toc -->\n\n- [Title](#title)\n  - [Install](#install)\n  - [Use it](#use-it)\n\n<!-- mrq:end -->\n\n## Install\n\n## Use it\n",
    );
}

#[test]
fn up_to_date_region_produces_no_edits() {
    let src = "<!-- mrq:toc -->\n\n- [A](#a)\n\n<!-- mrq:end -->\n\n# A\n";
    assert_eq!(
        toc::refresh_edits(&Document::parse(src), None).edits,
        vec![]
    );
}

#[test]
fn bare_start_marker_gets_an_end_marker() {
    assert_refresh(
        "# A\n\n<!-- mrq:toc -->\n\n## B\n",
        None,
        "# A\n\n<!-- mrq:toc -->\n\n- [A](#a)\n  - [B](#b)\n\n<!-- mrq:end -->\n\n## B\n",
    );
}

#[test]
fn bare_start_marker_at_end_of_file_without_newline() {
    assert_refresh(
        "# A\n\n<!-- mrq:toc -->",
        None,
        "# A\n\n<!-- mrq:toc -->\n\n- [A](#a)\n\n<!-- mrq:end -->\n",
    );
}

#[test]
fn documents_without_toc_regions_get_no_edits() {
    let src = "# A\n\n<!-- mrq:include path=\"x.md\" -->\nold\n<!-- mrq:end -->\n\n<!-- just a comment -->\n";
    let refresh = toc::refresh_edits(&Document::parse(src), None);
    assert_eq!(refresh.edits, vec![]);
    assert_eq!(refresh.diagnostics, vec![]);
}

#[test]
fn empty_outline_leaves_only_the_markers() {
    assert_refresh(
        "text\n\n<!-- mrq:toc -->\n- [Gone](#gone)\n<!-- mrq:end -->\n",
        None,
        "text\n\n<!-- mrq:toc -->\n<!-- mrq:end -->\n",
    );
}

#[test]
fn headings_inside_the_old_region_body_are_not_listed() {
    assert_refresh(
        "<!-- mrq:toc -->\n# Stale\n<!-- mrq:end -->\n\n# Real\n",
        None,
        "<!-- mrq:toc -->\n\n- [Real](#real)\n\n<!-- mrq:end -->\n\n# Real\n",
    );
}

#[test]
fn headings_inside_the_old_region_body_do_not_shift_slugs() {
    assert_refresh(
        "<!-- mrq:toc -->\n# Real\n<!-- mrq:end -->\n\n# Real\n",
        None,
        "<!-- mrq:toc -->\n\n- [Real](#real)\n\n<!-- mrq:end -->\n\n# Real\n",
    );
}

#[test]
fn depth_argument_limits_levels() {
    assert_refresh(
        "<!-- mrq:toc -->\n<!-- mrq:end -->\n\n# A\n\n## B\n\n### C\n",
        Some(2),
        "<!-- mrq:toc -->\n\n- [A](#a)\n  - [B](#b)\n\n<!-- mrq:end -->\n\n# A\n\n## B\n\n### C\n",
    );
}

#[test]
fn depth_attribute_overrides_the_argument() {
    assert_refresh(
        "<!-- mrq:toc depth=1 -->\n<!-- mrq:end -->\n\n# A\n\n## B\n",
        Some(3),
        "<!-- mrq:toc depth=1 -->\n\n- [A](#a)\n\n<!-- mrq:end -->\n\n# A\n\n## B\n",
    );
}

#[test]
fn invalid_depth_attribute_is_reported_and_the_region_left_alone() {
    let src = "<!-- mrq:toc depth=nine -->\nold\n<!-- mrq:end -->\n\n# A\n";
    let doc = Document::parse(src);
    let refresh = toc::refresh_edits(&doc, None);
    assert_eq!(refresh.edits, vec![]);
    assert_eq!(refresh.diagnostics.len(), 1);
    let diagnostic = &refresh.diagnostics[0];
    assert_eq!(diagnostic.code, "directive");
    assert_eq!(
        &src[diagnostic.range.start..diagnostic.range.end],
        "depth=nine"
    );
}

#[test]
fn several_regions_are_all_refreshed() {
    assert_refresh(
        "<!-- mrq:toc -->\n<!-- mrq:end -->\n\n# A\n\n## B\n\n<!-- mrq:toc depth=1 -->\nx\n<!-- mrq:end -->\n",
        None,
        "<!-- mrq:toc -->\n\n- [A](#a)\n  - [B](#b)\n\n<!-- mrq:end -->\n\n# A\n\n## B\n\n<!-- mrq:toc depth=1 -->\n\n- [A](#a)\n\n<!-- mrq:end -->\n",
    );
}

#[test]
fn crlf_line_endings_are_kept() {
    assert_refresh(
        "# A\r\n\r\n<!-- mrq:toc -->\r\nold\r\n<!-- mrq:end -->\r\n",
        None,
        "# A\r\n\r\n<!-- mrq:toc -->\r\n\r\n- [A](#a)\r\n\r\n<!-- mrq:end -->\r\n",
    );
    assert_refresh(
        "# A\r\n\r\n<!-- mrq:toc -->",
        None,
        "# A\r\n\r\n<!-- mrq:toc -->\r\n\r\n- [A](#a)\r\n\r\n<!-- mrq:end -->\r\n",
    );
}

#[test]
fn regions_in_blockquotes_keep_the_quote_prefix() {
    assert_refresh(
        "# A\n\n> <!-- mrq:toc -->\n> old\n> <!-- mrq:end -->\n",
        None,
        "# A\n\n> <!-- mrq:toc -->\n>\n> - [A](#a)\n>\n> <!-- mrq:end -->\n",
    );
    assert_refresh(
        "# A\n\n> <!-- mrq:toc -->\n",
        None,
        "# A\n\n> <!-- mrq:toc -->\n>\n> - [A](#a)\n>\n> <!-- mrq:end -->\n",
    );
}

#[test]
fn regions_in_list_items_are_indented_to_the_item_content() {
    assert_refresh(
        "# A\n\n- <!-- mrq:toc -->\n",
        None,
        "# A\n\n- <!-- mrq:toc -->\n\n  - [A](#a)\n\n  <!-- mrq:end -->\n",
    );
}

#[test]
fn frontmatter_is_untouched() {
    assert_refresh(
        "---\ntitle: x\n---\n<!-- mrq:toc -->\n<!-- mrq:end -->\n# A\n",
        None,
        "---\ntitle: x\n---\n<!-- mrq:toc -->\n\n- [A](#a)\n\n<!-- mrq:end -->\n# A\n",
    );
}

/// A line of Markdown for the documents around a region.
fn line() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        "#{1,4} [A-Za-z][a-z ]{0,8}",
        "[a-z ]{1,12}",
        "- [a-z]{1,6}",
        "> [a-z]{0,6}",
        Just("<!-- note -->".to_owned()),
        Just("```\ncode\n```".to_owned()),
    ]
}

fn lines(max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(line(), 0..max).prop_map(|lines| {
        lines
            .into_iter()
            .map(|line| line + "\n")
            .collect::<String>()
    })
}

proptest! {
    /// Bytes before the end of the start marker and from the end marker on survive the
    /// refresh, and a second refresh changes nothing.
    #[test]
    fn only_the_region_body_changes_and_refresh_is_idempotent(
        before in lines(6),
        body in lines(4),
        after in lines(6),
        crlf in any::<bool>(),
        bare in any::<bool>(),
    ) {
        let eol = if crlf { "\r\n" } else { "\n" };
        let start = "<!-- mrq:toc -->";
        let end = "<!-- mrq:end -->";
        let head = format!("{before}\n{start}");
        let tail = if bare {
            format!("\n\n{after}")
        } else {
            format!("{end}\n\n{after}")
        };
        let src = format!("{head}\n{body}{tail}").replace('\n', eol);
        let (head, tail) = (head.replace('\n', eol), tail.replace('\n', eol));

        let doc = Document::parse(src.as_str());
        let refresh = toc::refresh_edits(&doc, None);
        prop_assert!(refresh.diagnostics.is_empty());
        let once = apply_edits(&src, &refresh.edits).unwrap();
        prop_assert!(once.starts_with(&head), "prefix changed: {once:?}");
        prop_assert!(once.ends_with(&tail), "suffix changed: {once:?}");
        if bare {
            prop_assert!(once[head.len()..once.len() - tail.len()].contains(end));
        }
        let again = toc::refresh_edits(&Document::parse(once.as_str()), None);
        prop_assert_eq!(again.edits, vec![], "not idempotent: {:?}", once);
    }
}
