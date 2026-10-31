//! The ANSI backend: styled lines at a terminal width (snapshots at 60 and 100 columns
//! over fixtures with every node kind), terminal detection (`NO_COLOR`, `--color`,
//! graphics protocols), OSC 8 hyperlinks, images and diagrams.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use mrq_core::Document;
use mrq_core::document::DocumentOptions;
use mrq_render::Theme;
use mrq_render::ansi::{
    AnsiError, AnsiOptions, AnsiRenderer, Capabilities, ColorChoice, ColorLevel, GraphicsProtocol,
    StyledDocument,
};
use mrq_render::diagram::{DiagramError, DiagramFormat, DiagramOutput, DiagramRenderer, Registry};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ansi");

/// A diagram renderer that answers from memory.
struct Fixed {
    languages: Vec<String>,
    formats: Vec<DiagramFormat>,
    result: Result<DiagramOutput, DiagramError>,
}

impl Fixed {
    fn new(lang: &str, result: Result<DiagramOutput, DiagramError>) -> Self {
        let formats = match &result {
            Ok(output) => vec![output.format()],
            Err(_) => vec![DiagramFormat::Text, DiagramFormat::Png],
        };
        Self {
            languages: vec![lang.to_owned()],
            formats,
            result,
        }
    }
}

impl DiagramRenderer for Fixed {
    fn id(&self) -> &'static str {
        "fixed"
    }

    fn languages(&self) -> &[String] {
        &self.languages
    }

    fn formats(&self, _lang: &str) -> &[DiagramFormat] {
        &self.formats
    }

    fn render(
        &self,
        _lang: &str,
        _src: &str,
        _format: DiagramFormat,
    ) -> Result<DiagramOutput, DiagramError> {
        self.result.clone()
    }
}

fn bob_registry() -> Registry {
    Registry::new().with_renderer(Fixed::new(
        "bob",
        Ok(DiagramOutput::Text("┌────┐\n│ ok │\n└────┘\n".to_owned())),
    ))
}

fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let vars: HashMap<String, String> = vars
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |name| vars.get(name).cloned()
}

/// A true-colour terminal with hyperlinks and no graphics.
fn truecolor() -> Capabilities {
    Capabilities::detect(
        ColorChoice::Auto,
        true,
        env(&[("TERM", "xterm-256color"), ("COLORTERM", "truecolor")]),
    )
}

fn options(width: usize) -> AnsiOptions {
    AnsiOptions {
        width,
        ..AnsiOptions::default()
    }
}

fn render_with(doc: &Document, options: AnsiOptions, registry: Registry) -> StyledDocument {
    AnsiRenderer::new(&Theme::default(), options)
        .unwrap()
        .with_diagrams(registry)
        .render(doc)
}

fn render(flavor: &str, src: &str, width: usize) -> StyledDocument {
    let doc = DocumentOptions::new().flavor(flavor).parse(src).unwrap();
    render_with(&doc, options(width), Registry::new())
}

/// Escapes made visible, for snapshots.
fn visible(ansi: &str) -> String {
    ansi.replace('\x1b', "␛").replace('\x07', "␇")
}

fn fixture(name: &str) -> Document {
    let path = Path::new(FIXTURES).join(name);
    let flavor = name.trim_end_matches(".md");
    DocumentOptions::new().flavor(flavor).load(&path).unwrap()
}

/// The plain text and the visible ANSI of a fixture at `width`, for one snapshot.
fn snapshot(name: &str, width: usize) -> String {
    let doc = fixture(name);
    let styled = render_with(&doc, options(width), bob_registry());
    for line in &styled.lines {
        assert!(
            line.width() <= width,
            "{name} at {width}: line wider than the terminal: {:?}",
            line.text()
        );
    }
    format!(
        "{}\n----- ANSI -----\n{}",
        styled.plain_text(),
        visible(&styled.to_ansi(&truecolor()))
    )
}

// --- Snapshots --------------------------------------------------------------------------

#[test]
fn gfm_at_60() {
    insta::assert_snapshot!(snapshot("gfm.md", 60));
}

#[test]
fn gfm_at_100() {
    insta::assert_snapshot!(snapshot("gfm.md", 100));
}

#[test]
fn obsidian_at_60() {
    insta::assert_snapshot!(snapshot("obsidian.md", 60));
}

#[test]
fn obsidian_at_100() {
    insta::assert_snapshot!(snapshot("obsidian.md", 100));
}

#[test]
fn mkdocs_at_60() {
    insta::assert_snapshot!(snapshot("mkdocs.md", 60));
}

#[test]
fn mkdocs_at_100() {
    insta::assert_snapshot!(snapshot("mkdocs.md", 100));
}

// --- Terminal detection -----------------------------------------------------------------

#[test]
fn no_color_turns_colour_off_unless_forced() {
    let no_color = env(&[("TERM", "xterm-256color"), ("NO_COLOR", "1")]);
    let caps = Capabilities::detect(ColorChoice::Auto, true, &no_color);
    assert_eq!(caps.color, ColorLevel::None);
    // `--color always` beats `NO_COLOR`, as no-color.org asks.
    let caps = Capabilities::detect(ColorChoice::Always, true, &no_color);
    assert_ne!(caps.color, ColorLevel::None);
    // An empty `NO_COLOR` is not set.
    let caps = Capabilities::detect(
        ColorChoice::Auto,
        true,
        env(&[("TERM", "xterm-256color"), ("NO_COLOR", "")]),
    );
    assert_eq!(caps.color, ColorLevel::Ansi256);
}

#[test]
fn no_color_output_has_no_sgr_sequences() {
    let doc = fixture("gfm.md");
    let styled = render_with(&doc, options(60), bob_registry());
    let caps = Capabilities::detect(
        ColorChoice::Auto,
        true,
        env(&[("TERM", "xterm-256color"), ("NO_COLOR", "1")]),
    );
    let ansi = styled.to_ansi(&caps);
    assert!(!ansi.contains("\x1b["), "{}", visible(&ansi));
    // Hyperlinks are not colour: they stay.
    assert!(ansi.contains("\x1b]8;;https://example.com/docs\x1b\\"));
}

#[test]
fn color_never_and_pipes_are_plain() {
    let terminal = env(&[
        ("TERM", "xterm-kitty"),
        ("COLORTERM", "truecolor"),
        ("KITTY_WINDOW_ID", "1"),
    ]);
    for caps in [
        Capabilities::detect(ColorChoice::Never, true, &terminal),
        Capabilities::detect(ColorChoice::Auto, false, &terminal),
    ] {
        assert_eq!(caps, Capabilities::plain());
    }
    let doc = fixture("gfm.md");
    let styled = render_with(&doc, options(60), bob_registry());
    assert_eq!(styled.to_ansi(&Capabilities::plain()), styled.plain_text());
}

#[test]
fn colour_levels_come_from_colorterm_and_term() {
    let level =
        |vars: &[(&str, &str)]| Capabilities::detect(ColorChoice::Auto, true, env(vars)).color;
    assert_eq!(
        level(&[("TERM", "xterm"), ("COLORTERM", "24bit")]),
        ColorLevel::TrueColor
    );
    assert_eq!(level(&[("TERM", "xterm-256color")]), ColorLevel::Ansi256);
    assert_eq!(level(&[("TERM", "xterm")]), ColorLevel::Ansi256);
    assert_eq!(level(&[("TERM", "dumb")]), ColorLevel::None);
    // Forced colour on a pipe still picks a level.
    let caps = Capabilities::detect(ColorChoice::Always, false, env(&[("TERM", "dumb")]));
    assert_eq!(caps.color, ColorLevel::Ansi256);
    assert!(!caps.hyperlinks);
    assert_eq!(caps.graphics, None);
}

#[test]
fn graphics_protocols_are_detected_from_the_environment() {
    let protocol =
        |vars: &[(&str, &str)]| Capabilities::detect(ColorChoice::Auto, true, env(vars)).graphics;
    assert_eq!(
        protocol(&[("TERM", "xterm-kitty")]),
        Some(GraphicsProtocol::Kitty)
    );
    assert_eq!(
        protocol(&[("KITTY_WINDOW_ID", "3")]),
        Some(GraphicsProtocol::Kitty)
    );
    assert_eq!(
        protocol(&[("TERM_PROGRAM", "ghostty")]),
        Some(GraphicsProtocol::Kitty)
    );
    assert_eq!(
        protocol(&[("TERM_PROGRAM", "iTerm.app")]),
        Some(GraphicsProtocol::Iterm2)
    );
    assert_eq!(
        protocol(&[("LC_TERMINAL", "iTerm2")]),
        Some(GraphicsProtocol::Iterm2)
    );
    assert_eq!(
        protocol(&[("TERM_PROGRAM", "WezTerm")]),
        Some(GraphicsProtocol::Iterm2)
    );
    assert_eq!(protocol(&[("TERM", "foot")]), Some(GraphicsProtocol::Sixel));
    assert_eq!(
        protocol(&[("TERM", "mlterm")]),
        Some(GraphicsProtocol::Sixel)
    );
    assert_eq!(protocol(&[("TERM", "xterm-256color")]), None);
    // tmux swallows the escapes unless configured for passthrough.
    assert_eq!(
        protocol(&[("TERM", "xterm-kitty"), ("TMUX", "/tmp/tmux")]),
        None
    );
    // `MRQ_GRAPHICS` overrides the guess.
    assert_eq!(
        protocol(&[("TERM", "xterm-256color"), ("MRQ_GRAPHICS", "sixel")]),
        Some(GraphicsProtocol::Sixel)
    );
    assert_eq!(
        protocol(&[("TERM", "xterm-kitty"), ("MRQ_GRAPHICS", "none")]),
        None
    );
}

#[test]
fn options_follow_the_capabilities() {
    let caps = Capabilities::detect(ColorChoice::Auto, true, env(&[("TERM", "xterm-kitty")]));
    let options = AnsiOptions::for_terminal(&caps, 72);
    assert_eq!(options.width, 72);
    assert!(!options.link_urls);
    assert!(options.graphics);
    let options = AnsiOptions::for_terminal(&Capabilities::plain(), 72);
    assert!(options.link_urls);
    assert!(!options.graphics);
}

// --- Content ----------------------------------------------------------------------------

#[test]
fn links_are_osc8_hyperlinks() {
    let styled = render(
        "gfm",
        "See [the docs](<https://example.com/a b>) and [here](#top).\n",
        80,
    );
    let link = styled.lines[0]
        .spans
        .iter()
        .find(|span| span.link.is_some())
        .unwrap();
    assert_eq!(link.text, "the docs");
    assert_eq!(link.link.as_deref(), Some("https://example.com/a b"));
    let caps = Capabilities {
        color: ColorLevel::None,
        hyperlinks: true,
        graphics: None,
    };
    let ansi = styled.to_ansi(&caps);
    // The URI is percent-encoded and the link closed before the line ends.
    assert!(
        ansi.contains("\x1b]8;;https://example.com/a%20b\x1b\\the docs\x1b]8;;\x1b\\"),
        "{}",
        visible(&ansi)
    );
    // A same-document anchor needs a file to point into; stdin has none.
    assert!(!ansi.contains("]8;;#top") && !ansi.contains("]8;;file:"));
}

#[test]
fn relative_links_become_file_uris() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.md");
    fs::write(&path, "[other](other.md#x) [self](#top)\n").unwrap();
    let doc = Document::load(&path).unwrap();
    let ansi = render_with(&doc, options(80), Registry::new()).to_ansi(&truecolor());
    let base = std::path::absolute(dir.path()).unwrap();
    let uri = |file: &str| format!("\x1b]8;;file://{}/{file}\x1b\\", base.display());
    assert!(ansi.contains(&uri("other.md#x")), "{}", visible(&ansi));
    assert!(ansi.contains(&uri("doc.md#top")), "{}", visible(&ansi));
}

#[test]
fn link_urls_are_shown_when_there_are_no_hyperlinks() {
    let text = render(
        "gfm",
        "A [link](https://a.example) and <https://b.example>.\n",
        80,
    )
    .plain_text();
    assert_eq!(text, "A link (https://a.example) and https://b.example.\n");
    let doc = Document::parse("A [link](https://a.example).\n");
    let options = AnsiOptions {
        link_urls: false,
        ..options(80)
    };
    let text = render_with(&doc, options, Registry::new()).plain_text();
    assert_eq!(text, "A link.\n");
}

#[test]
fn terminal_escapes_in_the_source_are_neutralized() {
    let styled = render(
        "gfm",
        "Evil \x1b]0;title\x07 text\n\n```\ncode \x1b[2J\n```\n",
        80,
    );
    let ansi = styled.to_ansi(&Capabilities::plain());
    assert_eq!(
        ansi,
        "Evil \u{fffd}]0;title\u{fffd} text\n\n  code \u{fffd}[2J\n"
    );
}

#[test]
fn headings_carry_their_level_and_anchor() {
    let styled = render("gfm", "# Hello World\n\ntext\n\n## Second *one*\n", 80);
    let headings: Vec<(u8, Option<&str>)> = styled
        .lines
        .iter()
        .filter_map(|line| line.block.as_ref()?.heading.as_ref())
        .map(|h| (h.level, h.anchor.as_deref()))
        .collect();
    assert_eq!(
        headings,
        // The H1's rule belongs to the heading too.
        [
            (1, Some("hello-world")),
            (1, Some("hello-world")),
            (2, Some("second-one")),
            (2, Some("second-one"))
        ]
    );
    // Every non-blank line points at the block it shows, with its source span.
    let doc = Document::parse("para\n\n- item\n");
    let styled = render_with(&doc, options(80), Registry::new());
    let spans: Vec<&str> = styled
        .lines
        .iter()
        .filter_map(|line| line.block.as_ref())
        .map(|block| &doc.text()[block.span.start..block.span.end])
        .collect();
    assert_eq!(spans, ["para", "item"]);
}

#[test]
fn paragraphs_wrap_at_the_width() {
    let text = render("gfm", "aaa bbb ccc ddd eee fff\n", 11).plain_text();
    assert_eq!(text, "aaa bbb ccc\nddd eee fff\n");
    // A word longer than the width is broken.
    let text = render("gfm", "abcdefghijklmnop\n", 10).plain_text();
    assert_eq!(text, "abcdefghij\nklmnop\n");
    // Wide characters count twice.
    let text = render("gfm", "漢字漢字漢字\n", 7).plain_text();
    assert_eq!(text, "漢字漢\n字漢字\n");
}

#[test]
fn a_renderer_can_change_its_width() {
    let doc = Document::parse("aaa bbb ccc ddd eee fff\n");
    let renderer = AnsiRenderer::new(&Theme::default(), options(80)).unwrap();
    let narrow = renderer.clone().with_width(11);
    assert_eq!(narrow.options().width, 11);
    assert_eq!(
        narrow.render(&doc).plain_text(),
        "aaa bbb ccc\nddd eee fff\n"
    );
    // At least one column.
    assert_eq!(renderer.with_width(0).options().width, 1);
}

#[test]
fn tables_shrink_to_fit_the_width() {
    let src = "| a | b |\n| --- | --- |\n| one two three four five six | seven eight nine ten eleven twelve |\n";
    for width in [20, 30, 40] {
        let styled = render("gfm", src, width);
        for line in &styled.lines {
            assert!(line.width() <= width, "{width}: {:?}", line.text());
        }
        let text = styled.plain_text();
        assert!(text.starts_with('┌'), "{text}");
        assert!(text.contains("├"), "{text}");
        assert!(text.trim_end().ends_with('┘'), "{text}");
    }
    // A table that fits keeps its natural widths and alignment.
    let text = render("gfm", "| x | yy |\n| --: | :-: |\n| 1 | 2 |\n", 80).plain_text();
    insta::assert_snapshot!(text, @r"
    ┌───┬────┐
    │ x │ yy │
    ├───┼────┤
    │ 1 │ 2  │
    └───┴────┘
    ");
}

#[test]
fn footnotes_are_numbered_by_first_reference() {
    let src = "One[^b] two[^a] again[^b].\n\n[^a]: First.\n[^b]: Second.\n[^c]: Unused.\n";
    let text = render("gfm", src, 80).plain_text();
    assert_eq!(
        text,
        "One[1] two[2] again[1].\n\n────────────────────\n[1] Second.\n[2] First.\n"
    );
}

#[test]
fn diagrams_render_as_text_or_fall_back_to_code() {
    let doc = Document::parse("```bob\nsource\n```\n");
    let styled = render_with(&doc, options(40), bob_registry());
    assert_eq!(styled.plain_text(), "┌────┐\n│ ok │\n└────┘\n");
    assert_eq!(styled.diagnostics.len(), 0);

    let failing = Registry::new().with_renderer(Fixed::new(
        "bob",
        Err(DiagramError::Failed("syntax error".to_owned())),
    ));
    let styled = render_with(&doc, options(40), failing);
    assert_eq!(styled.plain_text(), "  source\n");
    assert_eq!(styled.diagnostics.len(), 1);
    assert_eq!(styled.diagnostics[0].code, "diagram");
    assert!(styled.diagnostics[0].message.contains("syntax error"));
}

#[test]
fn unknown_override_keys_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let overrides = dir.path().join("ansi.yaml");
    fs::write(&overrides, "link: { colour: \"#ff0000\" }\n").unwrap();
    let mut theme = Theme::default();
    theme.overrides.ansi = Some(overrides.clone());
    let error = AnsiRenderer::new(&theme, AnsiOptions::default()).unwrap_err();
    assert!(matches!(error, AnsiError::Override { .. }), "{error}");
    assert!(error.to_string().contains("ansi.yaml"), "{error}");
}

#[test]
fn overrides_set_element_styles() {
    let dir = tempfile::tempdir().unwrap();
    let overrides = dir.path().join("ansi.yaml");
    fs::write(
        &overrides,
        "link: { fg: \"#ff0000\", underline: false }\nstrong: { italic: true }\n",
    )
    .unwrap();
    let mut theme = Theme::default();
    theme.overrides.ansi = Some(overrides);
    let doc = Document::parse("[x](https://a.example) **b**\n");
    let ansi = AnsiRenderer::new(&theme, options(80))
        .unwrap()
        .render(&doc)
        .to_ansi(&truecolor());
    assert!(ansi.contains("\x1b[0;38;2;255;0;0mx"), "{}", visible(&ansi));
    assert!(ansi.contains("\x1b[0;1;3mb"), "{}", visible(&ansi));
}

#[test]
fn ansi256_maps_colours_to_the_palette() {
    let doc = Document::parse("[x](https://a.example)\n");
    let caps = Capabilities {
        color: ColorLevel::Ansi256,
        hyperlinks: false,
        graphics: None,
    };
    let ansi = render_with(&doc, options(80), Registry::new()).to_ansi(&caps);
    // The light theme's accent #1a5fb4 is closest to cube colour 25 (#005faf).
    assert!(ansi.contains("38;5;25"), "{}", visible(&ansi));
    assert!(!ansi.contains("38;2;"));
}

// --- Images -----------------------------------------------------------------------------

#[cfg(feature = "png")]
mod images {
    use super::*;

    /// A 4x2 PNG.
    fn png() -> Vec<u8> {
        mrq_render::diagram::rasterize(
            "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='2'><rect width='4' height='2' fill='#ff0000'/></svg>",
            1.0,
        )
        .unwrap()
    }

    fn caps(graphics: GraphicsProtocol) -> Capabilities {
        Capabilities {
            graphics: Some(graphics),
            ..truecolor()
        }
    }

    fn with_image(src: &str) -> (tempfile::TempDir, Document) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("pic.png"), png()).unwrap();
        let path = dir.path().join("doc.md");
        fs::write(&path, src).unwrap();
        let doc = DocumentOptions::new()
            .flavor("obsidian")
            .load(&path)
            .unwrap();
        (dir, doc)
    }

    fn graphics(width: usize) -> AnsiOptions {
        AnsiOptions {
            graphics: true,
            ..options(width)
        }
    }

    #[test]
    fn images_are_sized_in_cells() {
        let (_dir, doc) = with_image("![alt](pic.png)\n");
        let styled = render_with(
            &doc,
            AnsiOptions {
                cell_size: (2, 2),
                ..graphics(80)
            },
            Registry::new(),
        );
        let graphic = styled.lines[0].graphic.as_ref().unwrap();
        assert_eq!((graphic.columns, graphic.rows), (2, 1));
        assert_eq!((graphic.image.width, graphic.image.height), (4, 2));
        // Shrunk to the width, keeping the aspect ratio.
        let styled = render_with(
            &doc,
            AnsiOptions {
                cell_size: (1, 1),
                ..graphics(2)
            },
            Registry::new(),
        );
        let graphic = styled.lines[0].graphic.as_ref().unwrap();
        assert_eq!((graphic.columns, graphic.rows), (2, 1));
    }

    #[test]
    fn images_use_the_terminal_protocol() {
        let (_dir, doc) = with_image("![alt](pic.png)\n\n![[pic.png]]\n");
        let styled = render_with(&doc, graphics(80), Registry::new());
        assert_eq!(
            styled.lines.iter().filter(|l| l.graphic.is_some()).count(),
            2
        );

        let kitty = styled.to_ansi(&caps(GraphicsProtocol::Kitty));
        assert!(
            kitty.contains("\x1b_Ga=T,f=100,t=d,q=2,c=1,r=1,m=0;iVBOR"),
            "{}",
            visible(&kitty)
        );
        let iterm = styled.to_ansi(&caps(GraphicsProtocol::Iterm2));
        assert!(
            iterm.contains("\x1b]1337;File=inline=1;size="),
            "{}",
            visible(&iterm)
        );
        assert!(iterm.contains(";width=1;height=1;preserveAspectRatio=1:iVBOR"));
        let sixel = styled.to_ansi(&caps(GraphicsProtocol::Sixel));
        assert!(sixel.starts_with("\x1bPq\"1;1;4;2#"), "{}", visible(&sixel));
        assert!(sixel.contains("\x1b\\"));
        // Without a protocol, the alt text.
        assert_eq!(
            styled.to_ansi(&truecolor()).matches("[image: alt]").count(),
            1
        );
        assert_eq!(styled.plain_text(), "[image: alt]\n\n[image: pic.png]\n");
    }

    #[test]
    fn images_without_graphics_or_file_show_their_alt_text() {
        let (_dir, doc) = with_image(
            "![alt](pic.png) inline, ![gone](nope.png)\n\n![remote](https://x.example/a.png)\n",
        );
        let styled = render_with(&doc, options(80), Registry::new());
        assert!(styled.lines.iter().all(|line| line.graphic.is_none()));
        let styled = render_with(&doc, graphics(80), Registry::new());
        assert!(styled.lines.iter().all(|line| line.graphic.is_none()));
        assert_eq!(
            styled.plain_text(),
            "[image: alt] inline, [image: gone]\n\n[image: remote]\n"
        );
    }

    #[test]
    fn png_diagrams_need_graphics() {
        let doc = Document::parse("```bob\nsource\n```\n");
        let registry = || {
            Registry::new()
                .with_renderer(Fixed::new("bob", Ok(DiagramOutput::Png(png()))))
                .with_renderer(Fixed::new(
                    "bob",
                    Ok(DiagramOutput::Text("text\n".to_owned())),
                ))
        };
        let styled = render_with(&doc, graphics(40), registry());
        let graphic = styled.lines[0].graphic.as_ref().unwrap();
        assert_eq!(graphic.image.width, 4);
        assert_eq!(styled.plain_text(), "[bob diagram]\n");
        // Without graphics the PNG renderer is skipped for the text one.
        let styled = render_with(&doc, options(40), registry());
        assert_eq!(styled.plain_text(), "text\n");
    }
}
