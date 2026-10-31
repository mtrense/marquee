//! Down-conversion with `mrq_core::fmt::format_as`: flavor syntax the target flavor
//! cannot express is printed as the closest Markdown it can, and everything else as
//! `fmt` prints it.

use mrq_core::Document;
use mrq_core::config::FmtConfig;
use mrq_core::document::DocumentOptions;
use mrq_core::flavor::{Flavor, Registry};
use mrq_core::fmt::{format, format_as};

fn parse(src: &str, flavor: &str) -> Document {
    DocumentOptions::new().flavor(flavor).parse(src).unwrap()
}

fn target(name: &str) -> std::sync::Arc<dyn Flavor> {
    Registry::builtin().get(name).unwrap()
}

/// `src` parsed as `from` and printed for `to`.
fn convert(src: &str, from: &str, to: &str) -> String {
    format_as(&parse(src, from), &*target(to), &FmtConfig::default())
}

#[test]
fn the_same_flavor_prints_as_fmt_does() {
    let src = "# Note\n\n[[Other]] #tag %%hidden%%\n\n> [!tip]- Title\n> body\n";
    let doc = parse(src, "obsidian");
    assert_eq!(
        format_as(&doc, &*target("obsidian"), &FmtConfig::default()),
        format(&doc, &FmtConfig::default())
    );
}

#[test]
fn wikilinks_become_links_with_target_slugs() {
    let src = "# My Title\n\n[[Other Note]], [[Other Note#A Section|alias]], [[Note#Part]], \
               [[#My Title]], [[#Nowhere]], [[file.pdf]] and [[Note#^block]].\n";
    assert_eq!(
        convert(src, "obsidian", "commonmark"),
        "# My Title\n\n[Other Note](Other%20Note.md), [alias](Other%20Note.md#a-section), \
         [Note > Part](Note.md#part), [My Title](#my-title), [Nowhere](#nowhere), \
         [file.pdf](file.pdf) and [Note > ^block](Note.md).\n"
    );
}

#[test]
fn same_page_wikilinks_use_the_anchor_of_the_heading() {
    let src = "# Setup\n\n## Setup\n\nSee [[#setup]] and [[#Setup#Setup]].\n";
    // Obsidian anchors match headings ignoring case; the second `Setup` heading is the
    // anchor the target slugger dedupes.
    let out = convert(src, "obsidian", "gfm");
    assert!(out.contains("[setup](#setup)"), "{out}");
}

#[test]
fn embeds_become_images_or_links() {
    let src = "![[cat.png]] ![[cat.png|A cat]] ![[cat.png|100x200]] ![[Some Note]] \
               ![[Note#Part]]\n";
    assert_eq!(
        convert(src, "obsidian", "gfm"),
        "![cat.png](cat.png) ![A cat](cat.png) ![cat.png](cat.png) \
         [Some Note](Some%20Note.md) [Note > Part](Note.md#part)\n"
    );
}

#[test]
fn tags_become_text_and_percent_comments_html_comments() {
    let src = "A #tag and %%a note%% here.\n\n%%\nblock\n%%\n\nUnsafe %%a --> b%% gone.\n";
    assert_eq!(
        convert(src, "obsidian", "gfm"),
        "A #tag and <!--a note--> here.\n\n<!--\nblock\n-->\n\nUnsafe  gone.\n"
    );
}

#[test]
fn callouts_become_alerts_on_gfm() {
    let src = "> [!note]\n> Plain.\n\n> [!WARNING] Careful *now*\n> Body.\n\n\
               > [!faq]- Closed\n> Hidden.\n\n> [!bug]\n> Unknown kind.\n";
    assert_eq!(
        convert(src, "obsidian", "gfm"),
        "> [!NOTE]\n> Plain.\n\n> [!WARNING]\n> **Careful _now_**\n>\n> Body.\n\n\
         > **Closed**\n>\n> Hidden.\n\n> **Bug**\n>\n> Unknown kind.\n"
    );
}

#[test]
fn callouts_become_quotes_with_a_bold_title_on_commonmark() {
    let src = "> [!tip] See [[Other]]\n> Body.\n\n> [!note]\n> Plain.\n";
    assert_eq!(
        convert(src, "obsidian", "commonmark"),
        "> **See [Other](Other.md)**\n>\n> Body.\n\n> **Note**\n>\n> Plain.\n"
    );
}

#[test]
fn admonitions_become_alerts_quotes_or_callouts() {
    let src = "!!! note\n    Default title.\n\n!!! danger \"Watch out\"\n    Titled.\n\n\
               !!! tip \"\"\n    No title.\n\n??? info \"Collapsed\"\n    Folded.\n";
    assert_eq!(
        convert(src, "mkdocs", "gfm"),
        "> [!NOTE]\n> Default title.\n\n> **Watch out**\n>\n> Titled.\n\n\
         > [!TIP]\n> No title.\n\n> **Collapsed**\n>\n> Folded.\n"
    );
    assert_eq!(
        convert(src, "mkdocs", "obsidian"),
        "> [!note]\n> Default title.\n\n> [!danger] Watch out\n> Titled.\n\n\
         > [!tip]\n> No title.\n\n> [!info]- Collapsed\n> Folded.\n"
    );
}

#[test]
fn callouts_become_admonitions_on_mkdocs() {
    let src = "> [!tip]+ Open one\n> Body.\n\n> [!NOTE]\n> Alert.\n";
    assert_eq!(
        convert(src, "obsidian", "mkdocs"),
        "???+ tip \"Open one\"\n    Body.\n\n!!! note\n    Alert.\n"
    );
}

#[test]
fn heading_attributes_go_and_links_follow_the_anchors() {
    let src = "# Start {#top .big}\n\n## Install it {#setup}\n\n## Not_here\n\n\
               [a](#top), [b](#setup), [c](#not_here) and [d](other.md#setup).\n";
    assert_eq!(
        convert(src, "mkdocs", "gfm"),
        "# Start\n\n## Install it\n\n## Not_here\n\n\
         [a](#start), [b](#install-it), [c](#not_here) and [d](other.md#setup).\n"
    );
}

#[test]
fn definition_lists_become_bold_terms() {
    let src = "HTML\n:   Hyper Text Markup Language.\n\nTerm\n:   One.\n:   Two.\n";
    assert_eq!(
        convert(src, "mkdocs", "gfm"),
        "**HTML**\n\nHyper Text Markup Language.\n\n**Term**\n\nOne.\n\nTwo.\n"
    );
}

#[test]
fn abbreviation_definitions_are_dropped() {
    let src = "The HTML spec.\n\n*[HTML]: Hyper Text Markup Language\n";
    assert_eq!(convert(src, "mkdocs", "gfm"), "The HTML spec.\n");
}

#[test]
fn math_keeps_dollars_or_becomes_text_and_math_code() {
    let src = "Inline $a^2$ and\n\n$$\n\\frac{1}{2}\n$$\n";
    assert_eq!(
        convert(src, "obsidian", "commonmark"),
        "Inline $a^2$ and\n\n```math\n\\frac{1}{2}\n```\n"
    );
    assert_eq!(
        convert("Math \\(x_1\\) here.\n", "mkdocs", "gfm"),
        "Math $x_1$ here.\n"
    );
}

#[test]
fn unsupported_inline_extensions_become_html() {
    assert_eq!(
        convert("~~gone~~ and ==marked==\n", "obsidian", "commonmark"),
        "<del>gone</del> and <mark>marked</mark>\n"
    );
    assert_eq!(
        convert("==marked== ~~gone~~\n", "obsidian", "gfm"),
        "<mark>marked</mark> ~~gone~~\n"
    );
}

#[test]
fn bare_urls_become_autolinks_on_commonmark() {
    assert_eq!(
        convert(
            "See https://example.com and www.example.org.\n",
            "gfm",
            "commonmark"
        ),
        "See <https://example.com> and [www.example.org](http://www.example.org).\n"
    );
}

#[test]
fn footnotes_become_superscripts_on_commonmark() {
    let src = "Text[^1].\n\n[^1]: The note.\n";
    assert_eq!(
        convert(src, "gfm", "commonmark"),
        "Text<sup>1</sup>.\n\n<sup>1</sup> The note.\n"
    );
}

#[test]
fn tables_become_html_on_commonmark() {
    let src = "| a | b |\n| :-- | --: |\n| *x* | |\n";
    assert_eq!(
        convert(src, "gfm", "commonmark"),
        "<table>\n<thead>\n<tr>\n<th align=\"left\">\n\na\n\n</th>\n<th align=\"right\">\n\nb\n\n\
         </th>\n</tr>\n</thead>\n<tbody>\n<tr>\n<td align=\"left\">\n\n*x*\n\n</td>\n\
         <td align=\"right\"></td>\n</tr>\n</tbody>\n</table>\n"
    );
}

#[test]
fn the_frontmatter_flavor_names_the_target() {
    let src = "---\ntitle: T\nmrq:\n  flavor: obsidian\n---\n\n[[Other]]\n";
    let doc = Document::parse(src);
    assert_eq!(doc.flavor().name(), "obsidian");
    assert_eq!(
        format_as(&doc, &*target("gfm"), &FmtConfig::default()),
        "---\ntitle: T\nmrq:\n  flavor: gfm\n---\n\n[Other](Other.md)\n"
    );
}
