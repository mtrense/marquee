//! The diagram framework: the registry and its fallback order, the external-command
//! renderer (with a fake renderer script, `tests/fixtures/diagrams/fake.sh`, run through
//! `sh`), the content-hash cache, and diagrams in HTML.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mrq_core::Document;
use mrq_core::ast::CodeBlock;
use mrq_core::config::{DiagramsConfig, KrokiConfig};
use mrq_core::diag::Severity;
use mrq_render::Theme;
use mrq_render::diagram::{
    CommandRenderer, DiagramCache, DiagramError, DiagramFormat, DiagramHook, DiagramOutput,
    DiagramRenderer, Registry, codes,
};
use mrq_render::html::{CodeBlockContext, HtmlOptions, HtmlRenderer};

const FAKE: &str = "tests/fixtures/diagrams/fake.sh";

/// `sh fake.sh MODE [LOG]`.
fn fake(mode: &str, log: Option<&Path>) -> Vec<String> {
    let mut argv = vec!["sh".to_owned(), FAKE.to_owned(), mode.to_owned()];
    if let Some(log) = log {
        argv.push(log.to_string_lossy().into_owned());
    }
    argv
}

/// How often the fake renderer logging to `log` ran.
fn runs(log: &Path) -> usize {
    std::fs::read_to_string(log).map_or(0, |text| text.lines().count())
}

fn svg_renderer(lang: &str, mode: &str, log: Option<&Path>) -> CommandRenderer {
    CommandRenderer::new(format!("fake-{mode}"), &[lang])
        .with_command(DiagramFormat::Svg, fake(mode, log))
}

fn config(commands: &[(&str, Vec<String>)]) -> DiagramsConfig {
    DiagramsConfig {
        commands: commands
            .iter()
            .map(|(lang, argv)| ((*lang).to_owned(), argv.clone()))
            .collect::<BTreeMap<_, _>>(),
        ..DiagramsConfig::default()
    }
}

fn svg(output: &DiagramOutput) -> &str {
    match output {
        DiagramOutput::Svg(svg) => svg,
        other => panic!("not SVG: {other:?}"),
    }
}

/// A renderer that answers from memory, for registry tests that need no process.
struct Fixed {
    id: &'static str,
    languages: Vec<String>,
    formats: Vec<DiagramFormat>,
    result: Result<DiagramOutput, DiagramError>,
}

impl Fixed {
    fn new(id: &'static str, lang: &str, result: Result<DiagramOutput, DiagramError>) -> Self {
        let formats = match &result {
            Ok(output) => vec![output.format()],
            Err(_) => vec![DiagramFormat::Svg],
        };
        Self {
            id,
            languages: vec![lang.to_owned()],
            formats,
            result,
        }
    }
}

impl DiagramRenderer for Fixed {
    fn id(&self) -> &str {
        self.id
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

// --- Registry ---------------------------------------------------------------------------

/// `ids` without the built-in renderers whose feature is off.
fn built(ids: &[&'static str]) -> Vec<&'static str> {
    ids.iter()
        .copied()
        .filter(|&id| {
            (id != "builtin:mermaid" || cfg!(feature = "mermaid"))
                && (id != "builtin:dot" || cfg!(feature = "dot"))
        })
        .collect()
}

#[test]
fn default_order_follows_the_readme() {
    let registry = Registry::from_config(&DiagramsConfig::default());
    assert_eq!(
        registry.renderers_for("mermaid"),
        built(&["builtin:mermaid", "command:mmdc"])
    );
    let dot = built(&["command:dot", "builtin:dot"]);
    assert_eq!(registry.renderers_for("dot"), dot);
    assert_eq!(registry.renderers_for("Graphviz"), dot);
    assert_eq!(registry.renderers_for("d2"), ["command:d2"]);
    assert_eq!(registry.renderers_for("plantuml"), ["command:plantuml"]);
    assert_eq!(registry.renderers_for("bob"), Vec::<&str>::new());
    assert!(!registry.handles("rust"));
    assert!(registry.handles("MERMAID"));
}

#[test]
fn a_replaced_renderer_keeps_its_place() {
    let svg = Ok(DiagramOutput::Svg("<svg/>".to_owned()));
    let registry = Registry::from_config(&config(&[("mermaid", vec!["mine".to_owned()])]))
        .replacing("command:mmdc", Fixed::new("fixed", "mermaid", svg.clone()))
        .replacing("missing", Fixed::new("unused", "mermaid", svg));
    assert_eq!(
        registry.renderers_for("mermaid"),
        built(&["command:mine", "builtin:mermaid", "fixed"])
    );
}

#[test]
fn configured_commands_come_first_and_kroki_last() {
    let mut config = config(&[
        ("bob", vec!["svgbob".to_owned()]),
        ("mermaid", vec!["my-mermaid".to_owned(), "--svg".to_owned()]),
    ]);
    config.kroki = KrokiConfig {
        enabled: true,
        ..KrokiConfig::default()
    };
    let registry = Registry::from_config(&config);
    assert_eq!(registry.renderers_for("bob"), ["command:svgbob"]);
    assert_eq!(registry.renderers_for("svgbob"), ["kroki"]);
    assert_eq!(
        registry.renderers_for("mermaid"),
        built(&[
            "command:my-mermaid",
            "builtin:mermaid",
            "command:mmdc",
            "kroki"
        ])
    );
    assert_eq!(
        registry.renderers_for("dot"),
        built(&["command:dot", "builtin:dot", "kroki"]),
        "dot is graphviz on Kroki"
    );
    // Kroki knows its diagram types; a language it does not know is not a diagram.
    assert_eq!(registry.renderers_for("rust"), Vec::<&str>::new());
}

#[test]
fn kroki_is_never_registered_unless_enabled() {
    let config = config(&[("bob", vec!["svgbob".to_owned()])]);
    let registry = Registry::from_config(&config);
    for lang in ["bob", "mermaid", "plantuml", "svgbob", "ditaa"] {
        assert!(
            !registry.renderers_for(lang).contains(&"kroki"),
            "{lang}: {:?}",
            registry.renderers_for(lang)
        );
    }
}

#[cfg(not(feature = "kroki"))]
#[test]
fn enabled_kroki_without_the_feature_explains_itself() {
    let config = DiagramsConfig {
        kroki: KrokiConfig {
            enabled: true,
            ..KrokiConfig::default()
        },
        ..DiagramsConfig::default()
    };
    let registry = Registry::from_config(&config);
    let failure = registry
        .render("svgbob", "-->\n", &[DiagramFormat::Svg])
        .unwrap()
        .unwrap_err();
    assert!(failure.to_string().contains("`kroki` feature"), "{failure}");
}

#[test]
fn the_first_renderer_that_succeeds_wins() {
    let ok = |svg: &str| Ok(DiagramOutput::Svg(svg.to_owned()));
    let registry = Registry::new()
        .with_renderer(Fixed::new(
            "missing",
            "x",
            Err(DiagramError::Unavailable("not installed".to_owned())),
        ))
        .with_renderer(Fixed::new(
            "broken",
            "x",
            Err(DiagramError::Failed("bad input".to_owned())),
        ))
        .with_renderer(Fixed::new("good", "x", ok("<svg>good</svg>")))
        .with_renderer(Fixed::new("later", "x", ok("<svg>later</svg>")));
    let rendered = registry
        .render("x", "src", &[DiagramFormat::Svg])
        .unwrap()
        .unwrap();
    assert_eq!(rendered.renderer, "good");
    assert_eq!(svg(&rendered.output), "<svg>good</svg>");
    assert!(!rendered.cached);
    assert!(registry.render("y", "src", &[DiagramFormat::Svg]).is_none());
}

#[test]
fn failures_list_every_attempt() {
    let registry = Registry::new()
        .with_renderer(Fixed::new(
            "missing",
            "x",
            Err(DiagramError::Unavailable("not installed".to_owned())),
        ))
        .with_renderer(Fixed::new(
            "broken",
            "x",
            Err(DiagramError::Failed("bad input".to_owned())),
        ));
    let failure = registry
        .render("x", "src", &[DiagramFormat::Svg])
        .unwrap()
        .unwrap_err();
    assert_eq!(failure.attempts.len(), 2);
    let message = failure.to_string();
    assert!(message.contains("missing: not installed"), "{message}");
    assert!(message.contains("broken: bad input"), "{message}");
}

#[test]
fn accepted_formats_are_tried_in_order() {
    let registry = Registry::new()
        .with_renderer(Fixed::new(
            "svg-only",
            "x",
            Ok(DiagramOutput::Svg("<svg/>".to_owned())),
        ))
        .with_renderer(Fixed::new(
            "text-only",
            "x",
            Ok(DiagramOutput::Text("+--+\n".to_owned())),
        ));
    let text = registry
        .render("x", "src", &[DiagramFormat::Text])
        .unwrap()
        .unwrap();
    assert_eq!(text.renderer, "text-only");
    // The first renderer wins with its best format, before a later one's better format.
    let first = registry
        .render("x", "src", &[DiagramFormat::Text, DiagramFormat::Svg])
        .unwrap()
        .unwrap();
    assert_eq!(first.renderer, "svg-only");
    // With the `png` feature, SVG renderers rasterize for PNG (tests/diagram_builtin.rs).
    #[cfg(not(feature = "png"))]
    {
        let failure = registry
            .render("x", "src", &[DiagramFormat::Png])
            .unwrap()
            .unwrap_err();
        assert!(failure.to_string().contains("png"), "{failure}");
    }
}

// --- External commands ------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn commands_read_the_source_on_stdin_and_write_svg() {
    let renderer = svg_renderer("Bob", "svg", None);
    assert_eq!(renderer.languages(), ["bob"]);
    let output = renderer
        .render("BOB", "a --> b\n", DiagramFormat::Svg)
        .unwrap();
    assert_eq!(
        svg(&output),
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><text>a --> b</text></svg>\n"
    );
    // A format without a command is not offered.
    assert_eq!(renderer.formats("bob"), [DiagramFormat::Svg]);
    assert!(matches!(
        renderer.render("bob", "x", DiagramFormat::Png),
        Err(DiagramError::Unavailable(_))
    ));
}

#[cfg(unix)]
#[test]
fn commands_can_have_one_command_per_format() {
    let renderer = CommandRenderer::new("fake", &["x"])
        .with_command(DiagramFormat::Png, fake("png", None))
        .with_command(DiagramFormat::Text, fake("text", None));
    assert_eq!(
        renderer.formats("x"),
        [DiagramFormat::Png, DiagramFormat::Text]
    );
    assert_eq!(
        renderer.render("x", "ab\n", DiagramFormat::Png).unwrap(),
        DiagramOutput::Png(b"\x89PNG\r\n\x1a\nfake".to_vec())
    );
    assert_eq!(
        renderer.render("x", "ab\n", DiagramFormat::Text).unwrap(),
        DiagramOutput::Text("+--+\nab\n+--+\n".to_owned())
    );
}

#[cfg(unix)]
#[test]
fn command_failures_are_errors() {
    let failed = |mode: &str| {
        let renderer = svg_renderer("x", mode, None).with_timeout(Duration::from_millis(300));
        match renderer.render("x", "src\n", DiagramFormat::Svg) {
            Err(DiagramError::Failed(message)) => message,
            other => panic!("{mode}: {other:?}"),
        }
    };
    let fail = failed("fail");
    assert!(fail.contains("parse error on line 1"), "{fail}");
    let started = Instant::now();
    assert!(failed("slow").contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(failed("garbage").contains("SVG"));
    let missing = CommandRenderer::new("missing", &["x"])
        .with_command(DiagramFormat::Svg, vec!["mrq-no-such-renderer".to_owned()]);
    match missing.render("x", "src\n", DiagramFormat::Svg) {
        Err(DiagramError::Unavailable(message)) => {
            assert!(message.contains("mrq-no-such-renderer"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn a_missing_command_falls_through_to_the_next_renderer() {
    let config = config(&[("x", vec!["mrq-no-such-renderer".to_owned()])]);
    let registry = Registry::from_config(&config).with_renderer(svg_renderer("x", "svg", None));
    let rendered = registry
        .render("x", "hi\n", &[DiagramFormat::Svg])
        .unwrap()
        .unwrap();
    assert_eq!(rendered.renderer, "fake-svg");
}

#[cfg(unix)]
#[test]
fn relative_programs_resolve_against_their_config_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::create_dir_all(root.path().join("docs")).unwrap();
    let script = tools.join("bob.sh");
    std::fs::write(&script, "#!/bin/sh\nprintf '<svg>'\ncat\nprintf '</svg>'\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        root.path().join(".mrq.yaml"),
        "render:\n  diagrams:\n    commands:\n      bob: [\"./tools/bob.sh\"]\n",
    )
    .unwrap();
    let loaded = mrq_core::config::ConfigLoader::new(root.path().join("docs"))
        .load()
        .unwrap();
    let rendered = Registry::from_loaded(&loaded)
        .render("bob", "-->", &[DiagramFormat::Svg])
        .unwrap()
        .unwrap();
    assert_eq!(svg(&rendered.output), "<svg>--></svg>");
}

// --- Cache ------------------------------------------------------------------------------

#[test]
fn the_cache_lives_in_the_user_cache_directory() {
    let env = |key: &str| (key == "MRQ_CACHE_HOME").then(|| "/cache/mrq".to_owned());
    let cache = DiagramCache::in_user_cache(env).unwrap();
    assert_eq!(cache.dir(), Path::new("/cache/mrq/diagrams"));
    assert!(DiagramCache::in_user_cache(|_| None).is_none());
}

#[cfg(unix)]
fn cached_registry(cache: &Path, log: &Path, mode: &str) -> Registry {
    Registry::new()
        .with_renderer(svg_renderer("x", mode, Some(log)))
        .with_cache(DiagramCache::new(cache))
}

#[cfg(unix)]
#[test]
fn a_second_render_is_a_cache_hit() {
    let dir = tempfile::tempdir().unwrap();
    let (cache, log) = (dir.path().join("cache"), dir.path().join("log"));
    let registry = cached_registry(&cache, &log, "svg");
    let first = registry
        .render("x", "a\n", &[DiagramFormat::Svg])
        .unwrap()
        .unwrap();
    assert!(!first.cached);
    let second = registry
        .render("x", "a\n", &[DiagramFormat::Svg])
        .unwrap()
        .unwrap();
    assert!(second.cached);
    assert_eq!(second.renderer, "fake-svg");
    assert_eq!(first.output, second.output);
    assert_eq!(runs(&log), 1);
    // Another registry over the same directory (a later run) hits too.
    let later = cached_registry(&cache, &log, "svg");
    assert!(
        later
            .render("x", "a\n", &[DiagramFormat::Svg])
            .unwrap()
            .unwrap()
            .cached
    );
    assert_eq!(runs(&log), 1);
    let entries: Vec<PathBuf> = std::fs::read_dir(&cache)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    let [entry] = entries.as_slice() else {
        panic!("{entries:?}")
    };
    assert_eq!(entry.extension().unwrap(), "svg");
}

#[cfg(unix)]
#[test]
fn source_language_format_and_renderer_options_are_part_of_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let (cache, log) = (dir.path().join("cache"), dir.path().join("log"));
    let registry = cached_registry(&cache, &log, "svg");
    let render = |registry: &Registry, lang: &str, src: &str| {
        registry
            .render(lang, src, &[DiagramFormat::Svg])
            .unwrap()
            .unwrap()
            .cached
    };
    assert!(!render(&registry, "x", "a\n"));
    assert!(!render(&registry, "x", "b\n"), "other source");
    assert_eq!(runs(&log), 2);
    // Another command (here: another argument) is another key.
    let other_log = dir.path().join("other-log");
    let other = cached_registry(&cache, &other_log, "svg");
    assert!(!render(&other, "x", "a\n"), "other options");
    // Another language for the same renderer.
    let both = Registry::new()
        .with_renderer(
            CommandRenderer::new("fake-svg", &["x", "y"])
                .with_command(DiagramFormat::Svg, fake("svg", Some(&log))),
        )
        .with_cache(DiagramCache::new(&cache));
    assert!(render(&both, "x", "a\n"), "same command, same key");
    assert!(!render(&both, "y", "a\n"), "other language");
    assert!(render(&both, "y", "a\n"));
    // Another format.
    let formats = Registry::new()
        .with_renderer(
            CommandRenderer::new("fake", &["x"])
                .with_command(DiagramFormat::Svg, fake("svg", Some(&log)))
                .with_command(DiagramFormat::Text, fake("text", Some(&log))),
        )
        .with_cache(DiagramCache::new(&cache));
    let text = formats
        .render("x", "a\n", &[DiagramFormat::Text])
        .unwrap()
        .unwrap();
    assert!(!text.cached, "other format");
    assert_eq!(
        text.output,
        DiagramOutput::Text("+--+\na\n+--+\n".to_owned())
    );
    assert!(
        formats
            .render("x", "a\n", &[DiagramFormat::Text])
            .unwrap()
            .unwrap()
            .cached
    );
}

#[cfg(unix)]
#[test]
fn failures_are_not_cached() {
    let dir = tempfile::tempdir().unwrap();
    let (cache, log) = (dir.path().join("cache"), dir.path().join("log"));
    let registry = cached_registry(&cache, &log, "fail");
    assert!(
        registry
            .render("x", "a\n", &[DiagramFormat::Svg])
            .unwrap()
            .is_err()
    );
    assert!(
        registry
            .render("x", "a\n", &[DiagramFormat::Svg])
            .unwrap()
            .is_err()
    );
    assert_eq!(runs(&log), 2);
}

#[cfg(unix)]
#[test]
fn an_unreadable_cache_entry_is_a_miss() {
    let dir = tempfile::tempdir().unwrap();
    let (cache, log) = (dir.path().join("cache"), dir.path().join("log"));
    let registry = cached_registry(&cache, &log, "svg");
    registry
        .render("x", "a\n", &[DiagramFormat::Svg])
        .unwrap()
        .unwrap();
    for entry in std::fs::read_dir(&cache).unwrap() {
        std::fs::write(entry.unwrap().path(), b"\xff\xfe").unwrap();
    }
    let again = registry
        .render("x", "a\n", &[DiagramFormat::Svg])
        .unwrap()
        .unwrap();
    assert!(!again.cached);
    assert_eq!(runs(&log), 2);
    assert!(
        registry
            .render("x", "a\n", &[DiagramFormat::Svg])
            .unwrap()
            .unwrap()
            .cached
    );
}

#[cfg(unix)]
#[test]
fn without_a_cache_every_render_runs() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log");
    let registry = Registry::new().with_renderer(svg_renderer("x", "svg", Some(&log)));
    for _ in 0..2 {
        let rendered = registry
            .render("x", "a\n", &[DiagramFormat::Svg])
            .unwrap()
            .unwrap();
        assert!(!rendered.cached);
    }
    assert_eq!(runs(&log), 2);
}

// --- HTML -------------------------------------------------------------------------------

fn html(registry: Registry, src: &str) -> (String, Vec<mrq_core::diag::Diagnostic>) {
    let options = HtmlOptions {
        fragment: true,
        ..HtmlOptions::default()
    };
    let out = HtmlRenderer::new(&Theme::default(), options)
        .unwrap()
        .with_code_hook(DiagramHook::new(registry))
        .render_with_diagnostics(&Document::parse(src));
    (out.html, out.diagnostics)
}

#[cfg(unix)]
#[test]
fn diagrams_render_as_inline_svg_in_html() {
    let registry = Registry::new()
        .with_renderer(svg_renderer("bob", "svg", None))
        .with_renderer(svg_renderer("dot", "prolog", None));
    let (html, diagnostics) = html(
        registry,
        "```bob\na <b>\n```\n\n```dot\ndigraph {}\n```\n\n```rust\nlet x = 1;\n```\n",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    insta::assert_snapshot!(html);
}

#[test]
fn png_and_text_diagrams_in_html() {
    let registry = Registry::new()
        .with_renderer(Fixed::new(
            "png",
            "p",
            Ok(DiagramOutput::Png(b"\x89PNG\r\n\x1a\n".to_vec())),
        ))
        .with_renderer(Fixed::new(
            "text",
            "t",
            Ok(DiagramOutput::Text("<a> -> b\n".to_owned())),
        ));
    let (html, diagnostics) = html(registry, "```p\nx\n```\n\n```t\nx\n```\n");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    insta::assert_snapshot!(html);
}

#[test]
fn failed_diagrams_fall_back_to_the_code_block_with_a_warning() {
    let registry = Registry::new().with_renderer(Fixed::new(
        "broken",
        "mermaid",
        Err(DiagramError::Failed("syntax error".to_owned())),
    ));
    let src = "# Title\n\n```mermaid\ngraph TD\n```\n";
    let (html, diagnostics) = html(registry, src);
    assert!(
        html.contains("<code class=\"language-mermaid\">graph TD\n</code></pre>"),
        "{html}"
    );
    let [warning] = diagnostics.as_slice() else {
        panic!("{diagnostics:?}")
    };
    assert_eq!(warning.code, codes::DIAGRAM);
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(warning.file, Path::new("<stdin>"));
    assert_eq!(
        &src[warning.range.start..warning.range.end],
        "```mermaid\ngraph TD\n```"
    );
    assert!(warning.message.contains("mermaid"), "{}", warning.message);
    assert!(
        warning.message.contains("broken: syntax error"),
        "{}",
        warning.message
    );
}

#[test]
fn code_that_is_no_diagram_is_left_alone() {
    let registry = Registry::new().with_renderer(Fixed::new(
        "broken",
        "mermaid",
        Err(DiagramError::Failed("syntax error".to_owned())),
    ));
    let (html, diagnostics) = html(registry, "```rust\nfn main() {}\n```\n\n    mermaid\n");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(html.contains("language-rust"), "{html}");
}

#[test]
fn code_block_hooks_can_warn() {
    let options = HtmlOptions {
        fragment: true,
        ..HtmlOptions::default()
    };
    let out = HtmlRenderer::new(&Theme::default(), options)
        .unwrap()
        .with_code_hook(|block: &CodeBlock, cx: &mut CodeBlockContext<'_>| {
            cx.warn("custom", format!("saw {}", block.literal.trim()));
            None
        })
        .render_with_diagnostics(&Document::parse("text\n\n```\ncode\n```\n"));
    let [warning] = out.diagnostics.as_slice() else {
        panic!("{:?}", out.diagnostics)
    };
    assert_eq!(warning.code, "custom");
    assert_eq!(warning.message, "saw code");
    assert_eq!(warning.range.start, 6);
}
