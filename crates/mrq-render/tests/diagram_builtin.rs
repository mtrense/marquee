//! The built-in diagram renderers: DOT through layout-rs (feature `dot`), Mermaid through
//! merman (feature `mermaid`), and PNG from any SVG through resvg (feature `png`).
//!
//! SVG snapshots are normalized (see [`normalize`]) so they stay stable across platforms;
//! PNG is only checked for its signature and dimensions, since anti-aliasing and the
//! system fonts change its pixels.

// Which helpers and imports are used depends on the features.
#![allow(dead_code, unused_imports)]

use mrq_render::diagram::{DiagramError, DiagramFormat, DiagramOutput, DiagramRenderer, Registry};

/// `svg` with what may differ between platforms or runs taken out: numbers rounded to one
/// decimal (layout arithmetic may differ in the last bits), generated ids replaced, the
/// stylesheet bodies dropped (they are long and say nothing about the layout), and one
/// tag per line.
fn normalize(svg: &str) -> String {
    let style = regex::Regex::new(r"(?s)<style>.*?</style>").unwrap();
    let svg = style.replace_all(svg, "<style>…</style>");
    let ids = regex::Regex::new(r"mrq-[0-9a-f]{8,}").unwrap();
    let svg = ids.replace_all(&svg, "mrq-ID");
    let number = regex::Regex::new(r"-?\d+\.\d+").unwrap();
    let svg = number.replace_all(&svg, |caps: &regex::Captures<'_>| {
        let value: f64 = caps[0].parse().unwrap();
        let rounded = format!("{value:.1}");
        if rounded == "-0.0" {
            "0.0".to_owned()
        } else {
            rounded
        }
    });
    let gaps = regex::Regex::new(r">\s*<").unwrap();
    let svg = gaps.replace_all(&svg, ">\n<");
    svg.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn svg(output: DiagramOutput) -> String {
    match output {
        DiagramOutput::Svg(svg) => svg,
        other => panic!("not SVG: {other:?}"),
    }
}

/// The width and height in the IHDR chunk of `png`, after checking the signature.
fn png_size(png: &[u8]) -> (u32, u32) {
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "not PNG");
    assert_eq!(&png[12..16], b"IHDR");
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    (width, height)
}

#[test]
fn normalize_rounds_numbers_and_puts_one_tag_per_line() {
    assert_eq!(
        normalize(
            r#"<svg width="10.04"><g id="mrq-0123abcd89"> <path d="M-0.01 2.25"/></g></svg>"#
        ),
        "<svg width=\"10.0\">\n<g id=\"mrq-ID\">\n<path d=\"M0.0 2.2\"/>\n</g>\n</svg>"
    );
}

// --- DOT ----------------------------------------------------------------------------------

#[cfg(feature = "dot")]
mod dot {
    use super::*;
    use mrq_render::diagram::DotRenderer;

    const SAMPLE: &str = r#"digraph build {
    node [shape=box];
    parse [label="Parse"];
    flavor [label="Flavor passes"];
    render [label="Render", shape=ellipse];
    parse -> flavor -> render;
    parse -> render [label="plain"];
}
"#;

    #[test]
    fn renders_dot_and_graphviz_to_svg() {
        let renderer = DotRenderer::new();
        assert_eq!(renderer.id(), "builtin:dot");
        assert_eq!(renderer.languages(), ["dot", "graphviz"]);
        assert_eq!(renderer.formats("dot"), [DiagramFormat::Svg]);
        let svg = svg(renderer.render("dot", SAMPLE, DiagramFormat::Svg).unwrap());
        assert!(svg.contains("<svg"), "{svg}");
        assert!(svg.contains("Flavor passes"), "{svg}");
        insta::assert_snapshot!("dot_sample", normalize(&svg));
        let graphviz = renderer
            .render("graphviz", SAMPLE, DiagramFormat::Svg)
            .unwrap();
        assert_eq!(super::svg(graphviz), svg);
    }

    #[test]
    fn output_is_the_same_on_every_run() {
        // Several font sizes: layout-rs keeps their styles in a hash map.
        let src = "digraph { a [fontsize=10]; b [fontsize=20]; c [fontsize=30]; a -> b -> c; }";
        let renderer = DotRenderer::new();
        let first = renderer.render("dot", src, DiagramFormat::Svg).unwrap();
        for _ in 0..10 {
            assert_eq!(
                DotRenderer::new()
                    .render("dot", src, DiagramFormat::Svg)
                    .unwrap(),
                first
            );
        }
    }

    #[test]
    fn syntax_errors_fail() {
        let error = DotRenderer::new()
            .render("dot", "digraph { a -> ; }", DiagramFormat::Svg)
            .unwrap_err();
        assert!(matches!(error, DiagramError::Failed(_)), "{error:?}");
    }

    #[test]
    fn other_formats_are_unavailable() {
        let error = DotRenderer::new()
            .render("dot", SAMPLE, DiagramFormat::Text)
            .unwrap_err();
        assert!(matches!(error, DiagramError::Unavailable(_)), "{error:?}");
    }

    #[test]
    fn the_registry_falls_back_to_it_without_graphviz() {
        let registry = Registry::new()
            .with_renderer(
                mrq_render::diagram::CommandRenderer::new("command:none", &["dot"])
                    .with_command(DiagramFormat::Svg, vec!["mrq-no-such-dot".to_owned()]),
            )
            .with_renderer(DotRenderer::new());
        let rendered = registry
            .render("dot", SAMPLE, &[DiagramFormat::Svg])
            .unwrap()
            .unwrap();
        assert_eq!(rendered.renderer, "builtin:dot");
    }
}

// --- Mermaid ------------------------------------------------------------------------------

#[cfg(feature = "mermaid")]
mod mermaid {
    use super::*;
    use mrq_render::diagram::MermaidRenderer;

    const FLOWCHART: &str = "flowchart LR\n    A[Markdown] --> B(mrq)\n    B --> C{Format?}\n    C -->|html| D[Page]\n    C -->|pdf| E[Document]\n";
    const SEQUENCE: &str = "sequenceDiagram\n    participant User\n    participant mrq\n    User->>mrq: render\n    mrq-->>User: HTML\n";

    fn text(output: DiagramOutput) -> String {
        match output {
            DiagramOutput::Text(text) => text,
            other => panic!("not text: {other:?}"),
        }
    }

    #[test]
    fn renders_mermaid_to_svg_and_text() {
        let renderer = MermaidRenderer::new();
        assert_eq!(renderer.id(), "builtin:mermaid");
        assert_eq!(renderer.languages(), ["mermaid"]);
        let formats = renderer.formats("mermaid");
        assert!(formats.contains(&DiagramFormat::Svg) && formats.contains(&DiagramFormat::Text));
        assert_eq!(
            formats.contains(&DiagramFormat::Png),
            cfg!(feature = "png"),
            "{formats:?}"
        );
    }

    #[test]
    fn flowchart_svg() {
        let svg = svg(MermaidRenderer::new()
            .render("mermaid", FLOWCHART, DiagramFormat::Svg)
            .unwrap());
        assert!(svg.contains("<svg"), "{svg}");
        assert!(svg.contains("Markdown"), "{svg}");
        insta::assert_snapshot!("mermaid_flowchart", normalize(&svg));
    }

    #[test]
    fn sequence_svg() {
        let svg = svg(MermaidRenderer::new()
            .render("mermaid", SEQUENCE, DiagramFormat::Svg)
            .unwrap());
        insta::assert_snapshot!("mermaid_sequence", normalize(&svg));
    }

    #[test]
    fn diagram_ids_differ_per_source_and_repeat_per_source() {
        let renderer = MermaidRenderer::new();
        let a = svg(renderer
            .render("mermaid", FLOWCHART, DiagramFormat::Svg)
            .unwrap());
        let b = svg(renderer
            .render("mermaid", SEQUENCE, DiagramFormat::Svg)
            .unwrap());
        let id = |svg: &str| {
            let start = svg.find("id=\"").unwrap() + 4;
            svg[start..start + svg[start..].find('"').unwrap()].to_owned()
        };
        assert!(id(&a).starts_with("mrq-"), "{}", id(&a));
        assert_ne!(id(&a), id(&b));
        let again = svg(renderer
            .render("mermaid", FLOWCHART, DiagramFormat::Svg)
            .unwrap());
        assert_eq!(again, a);
    }

    #[test]
    fn resvg_safe_svg_has_no_foreign_objects() {
        let svg = svg(MermaidRenderer::new()
            .with_resvg_safe_svg(true)
            .render("mermaid", FLOWCHART, DiagramFormat::Svg)
            .unwrap());
        assert!(!svg.contains("<foreignObject"), "{svg}");
        assert!(svg.contains("Markdown"), "{svg}");
    }

    #[test]
    fn flowchart_text() {
        let text = text(
            MermaidRenderer::new()
                .render("mermaid", FLOWCHART, DiagramFormat::Text)
                .unwrap(),
        );
        assert!(text.contains("Markdown"), "{text}");
        insta::assert_snapshot!("mermaid_flowchart_text", text);
    }

    #[test]
    fn ascii_text_is_ascii_only() {
        let text = text(
            MermaidRenderer::new()
                .with_ascii(true)
                .render("mermaid", SEQUENCE, DiagramFormat::Text)
                .unwrap(),
        );
        assert!(text.is_ascii(), "{text}");
        assert!(text.contains("User"), "{text}");
    }

    #[test]
    fn options_are_part_of_the_fingerprint() {
        let plain = MermaidRenderer::new().fingerprint();
        assert_ne!(plain, MermaidRenderer::new().with_ascii(true).fingerprint());
        assert_ne!(
            plain,
            MermaidRenderer::new()
                .with_resvg_safe_svg(true)
                .fingerprint()
        );
    }

    #[test]
    fn syntax_errors_fail() {
        for format in [DiagramFormat::Svg, DiagramFormat::Text] {
            let error = MermaidRenderer::new()
                .render("mermaid", "flowchart LR\n    A -->", format)
                .unwrap_err();
            assert!(
                matches!(error, DiagramError::Failed(_)),
                "{format}: {error:?}"
            );
        }
        let error = MermaidRenderer::new()
            .render("mermaid", "no diagram here", DiagramFormat::Svg)
            .unwrap_err();
        assert!(matches!(error, DiagramError::Failed(_)), "{error:?}");
    }

    #[cfg(feature = "png")]
    #[test]
    fn png_is_rasterized_from_resvg_safe_svg() {
        let renderer = MermaidRenderer::new().with_resvg_safe_svg(true);
        let svg = svg(renderer
            .render("mermaid", FLOWCHART, DiagramFormat::Svg)
            .unwrap());
        let DiagramOutput::Png(png) = MermaidRenderer::new()
            .render("mermaid", FLOWCHART, DiagramFormat::Png)
            .unwrap()
        else {
            panic!("not PNG");
        };
        let expected = mrq_render::diagram::rasterize(&svg, 1.0).unwrap();
        assert_eq!(png_size(&png), png_size(&expected));
    }
}

// --- PNG ----------------------------------------------------------------------------------

#[cfg(feature = "png")]
mod png {
    use super::*;
    use mrq_render::diagram::{DiagramCache, rasterize};

    const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30" viewBox="0 0 40 30"><rect width="40" height="30" fill="#c00"/><text x="2" y="20" font-size="12">mrq</text></svg>"##;

    #[test]
    fn rasterizes_at_the_svg_size_times_the_scale() {
        assert_eq!(png_size(&rasterize(SQUARE, 1.0).unwrap()), (40, 30));
        assert_eq!(png_size(&rasterize(SQUARE, 2.0).unwrap()), (80, 60));
    }

    #[test]
    fn invalid_svg_and_scales_fail() {
        assert!(matches!(
            rasterize("<svg", 1.0),
            Err(DiagramError::Failed(_))
        ));
        assert!(matches!(
            rasterize(SQUARE, 0.0),
            Err(DiagramError::Failed(_))
        ));
        let empty = r#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"/>"#;
        assert!(matches!(
            rasterize(empty, 1.0),
            Err(DiagramError::Failed(_))
        ));
    }

    /// An SVG-only renderer that counts its renders.
    struct SvgOnly {
        languages: Vec<String>,
        svg: Result<String, DiagramError>,
        runs: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl SvgOnly {
        fn new(svg: Result<&str, DiagramError>) -> Self {
            Self {
                languages: vec!["box".to_owned()],
                svg: svg.map(str::to_owned),
                runs: std::sync::Arc::default(),
            }
        }

        fn runs(&self) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
            self.runs.clone()
        }
    }

    impl DiagramRenderer for SvgOnly {
        fn id(&self) -> &'static str {
            "svg-only"
        }

        fn languages(&self) -> &[String] {
            &self.languages
        }

        fn formats(&self, _lang: &str) -> &[DiagramFormat] {
            &[DiagramFormat::Svg]
        }

        fn render(
            &self,
            _lang: &str,
            _src: &str,
            format: DiagramFormat,
        ) -> Result<DiagramOutput, DiagramError> {
            assert_eq!(format, DiagramFormat::Svg);
            self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.svg.clone().map(DiagramOutput::Svg)
        }
    }

    #[test]
    fn any_svg_renderer_can_satisfy_png() {
        let registry = Registry::new().with_renderer(SvgOnly::new(Ok(SQUARE)));
        let rendered = registry
            .render("box", "src", &[DiagramFormat::Png])
            .unwrap()
            .unwrap();
        assert_eq!(rendered.renderer, "svg-only+resvg");
        let DiagramOutput::Png(png) = rendered.output else {
            panic!("not PNG: {:?}", rendered.output);
        };
        assert_eq!(png_size(&png), (40, 30));
    }

    #[test]
    fn svg_is_still_preferred_when_asked_first() {
        let registry = Registry::new().with_renderer(SvgOnly::new(Ok(SQUARE)));
        let rendered = registry
            .render("box", "src", &[DiagramFormat::Svg, DiagramFormat::Png])
            .unwrap()
            .unwrap();
        assert_eq!(rendered.output.format(), DiagramFormat::Svg);
        assert_eq!(rendered.renderer, "svg-only");
    }

    #[test]
    fn a_failed_svg_is_not_rendered_again_for_png() {
        let renderer = SvgOnly::new(Err(DiagramError::Failed("syntax".to_owned())));
        let runs = renderer.runs();
        let registry = Registry::new().with_renderer(renderer);
        let failure = registry
            .render("box", "src", &[DiagramFormat::Svg, DiagramFormat::Png])
            .unwrap()
            .unwrap_err();
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(failure.attempts.len(), 1, "{failure}");
    }

    #[test]
    fn unrasterizable_svg_fails_like_a_render() {
        let registry = Registry::new().with_renderer(SvgOnly::new(Ok("<svg broken")));
        let failure = registry
            .render("box", "src", &[DiagramFormat::Png])
            .unwrap()
            .unwrap_err();
        assert_eq!(failure.attempts[0].renderer, "svg-only+resvg");
        assert!(matches!(failure.attempts[0].error, DiagramError::Failed(_)));
    }

    #[test]
    fn rasterized_png_and_its_svg_are_cached() {
        let dir = tempfile::tempdir().unwrap();
        let renderer = SvgOnly::new(Ok(SQUARE));
        let runs = renderer.runs();
        let registry = Registry::new()
            .with_renderer(renderer)
            .with_cache(DiagramCache::new(dir.path()));
        let first = registry
            .render("box", "src", &[DiagramFormat::Png])
            .unwrap()
            .unwrap();
        assert!(!first.cached);
        let second = registry
            .render("box", "src", &[DiagramFormat::Png])
            .unwrap()
            .unwrap();
        assert!(second.cached);
        assert_eq!(second.output, first.output);
        let svg = registry
            .render("box", "src", &[DiagramFormat::Svg])
            .unwrap()
            .unwrap();
        assert!(svg.cached, "the SVG behind the PNG was cached too");
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[cfg(feature = "dot")]
    #[test]
    fn dot_renders_to_png() {
        let registry = Registry::new().with_renderer(mrq_render::diagram::DotRenderer::new());
        let rendered = registry
            .render("dot", "digraph { a -> b; }", &[DiagramFormat::Png])
            .unwrap()
            .unwrap();
        let DiagramOutput::Png(png) = rendered.output else {
            panic!("not PNG");
        };
        let (width, height) = png_size(&png);
        assert!(width > 10 && height > 10, "{width}x{height}");
    }
}

#[cfg(not(feature = "png"))]
#[test]
fn without_png_svg_renderers_offer_no_png() {
    struct SvgOnly(Vec<String>);
    impl DiagramRenderer for SvgOnly {
        fn id(&self) -> &'static str {
            "svg-only"
        }
        fn languages(&self) -> &[String] {
            &self.0
        }
        fn formats(&self, _lang: &str) -> &[DiagramFormat] {
            &[DiagramFormat::Svg]
        }
        fn render(
            &self,
            _: &str,
            _: &str,
            _: DiagramFormat,
        ) -> Result<DiagramOutput, DiagramError> {
            Ok(DiagramOutput::Svg("<svg/>".to_owned()))
        }
    }
    let registry = Registry::new().with_renderer(SvgOnly(vec!["box".to_owned()]));
    let failure = registry
        .render("box", "src", &[DiagramFormat::Png])
        .unwrap()
        .unwrap_err();
    assert!(matches!(
        failure.attempts[0].error,
        DiagramError::Unavailable(_)
    ));
}
