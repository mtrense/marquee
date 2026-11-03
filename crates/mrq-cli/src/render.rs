//! `mrq render`: documents to HTML, ANSI, PDF, Markdown, JSON or man with the mrq-render
//! backends, written to stdout, to `--out` or one file per input into `--out-dir`.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mrq_core::Document;
use mrq_core::Slashed;
use mrq_core::config::{LoadedConfig, Origin};
use mrq_core::diag::Diagnostic;
use mrq_core::document::DocumentOptions;
use mrq_core::input::Input;
use mrq_core::source::SourceFile;
use mrq_core::transform::{MergeOptions, merge};
use mrq_render::Theme;
use mrq_render::ansi::{self, AnsiOptions, AnsiRenderer, Capabilities};
use mrq_render::diagram::{DiagramCache, DiagramHook, Registry};
use mrq_render::html::{HtmlOptions, HtmlRenderer};
use mrq_render::man::{ManOptions, ManRenderer};
use mrq_render::md::MdRenderer;
use serde_json::json;

use crate::cli::{ColorChoice, GlobalArgs, RenderArgs, RenderFormat};

/// One output file (`None`: stdout or `--out`) and its bytes.
type Output = (Option<PathBuf>, Vec<u8>);

/// `mrq render`: loads the configuration and the inputs, then [`run`]s. With `-o md`,
/// `--flavor` is the target flavor, so it is not the input flavor.
pub fn command(args: &RenderArgs, global: &GlobalArgs) -> ExitCode {
    let input_flavor = match args.output {
        RenderFormat::Md => None,
        _ => global.flavor,
    };
    match crate::prepare_with(global, input_flavor, &args.inputs, false) {
        Some((loaded, options, inputs)) => run(args, &inputs, &options, &loaded, global),
        None => usage(),
    }
}

/// Renders the inputs and writes the result.
///
/// One input is rendered as it is. Several are concatenated into one document as
/// `mrq merge --no-wrap` would (anchors made unique, links between them redirected,
/// other links rebased to the output), except for `-o json`, which gives a list of
/// `{file, ast}` objects. With `--out-dir`, each input is rendered on its own into
/// `DIR/<stem>.<ext>` (`html`, `ansi`, `pdf`, `md`, `json`; man pages keep a section
/// extension such as `tool.1.md` → `tool.1`, else get `.man`).
///
/// Warnings (diagrams shown as code, fonts, …) go to stderr in the `--format` and never
/// change the exit code. The exit code is 2, and nothing is written, if an input cannot be
/// read, the theme cannot be loaded, a PDF does not compile, `-o pdf` was left out of the
/// build, or a PDF would go to a terminal; also if an output cannot be written.
fn run(
    args: &RenderArgs,
    inputs: &[Input],
    options: &DocumentOptions,
    loaded: &LoadedConfig,
    global: &GlobalArgs,
) -> ExitCode {
    let to_stdout = args.out.is_none() && args.out_dir.is_none();
    if args.output == RenderFormat::Pdf {
        if !cfg!(feature = "pdf") {
            report("`-o pdf` needs mrq built with the `pdf` feature");
            return usage();
        }
        if to_stdout && io::stdout().is_terminal() {
            report("refusing to write a PDF to the terminal; redirect stdout or use --out");
            return usage();
        }
    }
    let names = match &args.out_dir {
        Some(dir) => match output_names(inputs, dir, args.output) {
            Ok(names) => Some(names),
            Err(message) => {
                report(&message);
                return usage();
            }
        },
        None => None,
    };

    let mut docs = Vec::new();
    let mut failed = false;
    for input in inputs {
        match options.read(input) {
            Ok(doc) => docs.push(doc),
            Err(err) => {
                report(&err.to_string());
                failed = true;
            }
        }
    }
    if failed {
        return usage();
    }
    let backend = match Backend::new(args, loaded, global, to_stdout) {
        Ok(backend) => backend,
        Err(message) => {
            report(&message);
            return usage();
        }
    };

    let mut warnings = Warnings::default();
    let rendered = match names {
        Some(names) => render_each(&backend, &docs, names, &mut warnings),
        None => render_all(&backend, &docs, args, options, &mut warnings)
            .map(|bytes| vec![(None, bytes)]),
    };
    warnings.emit(global);
    let Ok(outputs) = rendered else {
        return usage();
    };

    for (path, bytes) in outputs {
        let path = path.or_else(|| args.out.clone());
        let written = match &path {
            Some(path) => write_file(path, &bytes),
            None => io::stdout().lock().write_all(&bytes),
        };
        if let Err(err) = written {
            let name = path.map_or_else(|| "<stdout>".to_owned(), |p| p.slashed());
            report(&format!("{name}: {err}"));
            return usage();
        }
    }
    ExitCode::SUCCESS
}

/// Renders each document on its own, for `--out-dir`.
fn render_each(
    backend: &Backend,
    docs: &[Document],
    names: Vec<PathBuf>,
    warnings: &mut Warnings,
) -> Result<Vec<Output>, ()> {
    let mut outputs = Vec::new();
    for (doc, name) in docs.iter().zip(names) {
        let bytes = backend.render_one(doc, warnings)?;
        outputs.push((Some(name), bytes));
    }
    Ok(outputs)
}

/// Renders all documents as one output: see [`run`].
fn render_all(
    backend: &Backend,
    docs: &[Document],
    args: &RenderArgs,
    options: &DocumentOptions,
    warnings: &mut Warnings,
) -> Result<Vec<u8>, ()> {
    if let [doc] = docs {
        return backend.render_one(doc, warnings);
    }
    if let Backend::Json = backend {
        let files: Vec<_> = docs
            .iter()
            .map(|doc| {
                let file = doc.path().map(Slashed::slashed);
                json!({ "file": file, "ast": doc.to_json_ast() })
            })
            .collect();
        let mut json = serde_json::to_string_pretty(&files).expect("JSON values serialize");
        json.push('\n');
        return Ok(json.into_bytes());
    }

    let mut merge_options = MergeOptions::new().wrap(false);
    if let Some(out) = &args.out {
        merge_options = merge_options.output(out);
    }
    let merged = merge(docs, &merge_options);
    for doc in docs {
        warnings.files.push(doc.source().clone());
    }
    warnings.diagnostics.extend(merged.diagnostics);
    // Named after the output, or the first input in the current directory, which the
    // links were rebased to; images and the title fallback use the name.
    let name = args.out.clone().or_else(|| {
        let first = docs.first()?.path()?;
        first.file_name().map(PathBuf::from)
    });
    let doc = match options.parse_source(SourceFile::new(name, merged.text)) {
        Ok(doc) => doc,
        Err(err) => {
            report(&err.to_string());
            return Err(());
        }
    };
    backend.render_one(&doc, warnings)
}

/// The `--out-dir` file of each input; an error for stdin and for two inputs with the
/// same output name.
fn output_names(
    inputs: &[Input],
    dir: &Path,
    format: RenderFormat,
) -> Result<Vec<PathBuf>, String> {
    let mut seen: HashMap<PathBuf, &Path> = HashMap::new();
    let mut names = Vec::new();
    for input in inputs {
        let Some(path) = input.path() else {
            return Err("stdin cannot be rendered with --out-dir; use --out or stdout".to_owned());
        };
        let stem = path.file_stem().unwrap_or(path.as_os_str());
        let stem = Path::new(stem);
        let name = match format {
            RenderFormat::Man if man_section(stem) => stem.to_path_buf(),
            _ => stem.with_extension(extension(format)),
        };
        let name = dir.join(name);
        if let Some(other) = seen.insert(name.clone(), path) {
            return Err(format!(
                "{} and {} would both be written to {}",
                other.slashed(),
                path.slashed(),
                name.slashed()
            ));
        }
        names.push(name);
    }
    Ok(names)
}

/// Whether `stem` ends in a man section extension (`tool.1`, `mrq.1m`).
fn man_section(stem: &Path) -> bool {
    stem.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.starts_with(|c: char| c.is_ascii_digit()))
}

const fn extension(format: RenderFormat) -> &'static str {
    match format {
        RenderFormat::Html => "html",
        RenderFormat::Ansi => "ansi",
        RenderFormat::Pdf => "pdf",
        RenderFormat::Md => "md",
        RenderFormat::Json => "json",
        RenderFormat::Man => "man",
    }
}

/// The warnings of all renders and the files they are about, emitted once.
#[derive(Default)]
struct Warnings {
    diagnostics: Vec<Diagnostic>,
    files: Vec<SourceFile>,
}

impl Warnings {
    fn add(&mut self, doc: &Document, diagnostics: Vec<Diagnostic>) {
        if !diagnostics.is_empty() {
            self.files.push(doc.source().clone());
            self.diagnostics.extend(diagnostics);
        }
    }

    fn emit(&self, global: &GlobalArgs) {
        if !self.diagnostics.is_empty() {
            crate::emit_diagnostics(&self.diagnostics, &self.files, global);
        }
    }
}

/// A backend set up for the run.
enum Backend {
    Html(HtmlRenderer),
    Ansi(AnsiRenderer, Capabilities),
    Md(MdRenderer),
    Json,
    Man(ManRenderer),
    #[cfg(feature = "pdf")]
    Pdf(mrq_render::pdf::PdfRenderer),
}

impl Backend {
    /// The backend for `-o`, with the theme of `-s` and the diagram renderers of the
    /// configuration; an error message if they cannot be set up.
    fn new(
        args: &RenderArgs,
        loaded: &LoadedConfig,
        global: &GlobalArgs,
        to_stdout: bool,
    ) -> Result<Self, String> {
        Ok(match args.output {
            RenderFormat::Html => {
                let options = HtmlOptions {
                    fragment: args.fragment,
                    ..HtmlOptions::default()
                };
                let renderer = HtmlRenderer::new(&theme(args, loaded)?, options)
                    .map_err(|e| e.to_string())?
                    .with_code_hook(DiagramHook::new(diagrams(loaded)));
                Self::Html(renderer)
            }
            RenderFormat::Ansi => {
                let is_terminal = to_stdout && io::stdout().is_terminal();
                let caps = Capabilities::from_env(color_choice(global.color), is_terminal);
                let width = args.width.map_or_else(terminal_width, usize::from);
                let renderer = AnsiRenderer::new(
                    &theme(args, loaded)?,
                    AnsiOptions::for_terminal(&caps, width),
                )
                .map_err(|e| e.to_string())?
                .with_diagrams(diagrams(loaded));
                Self::Ansi(renderer, caps)
            }
            RenderFormat::Md => {
                let mut renderer = MdRenderer::new(loaded.config().fmt.clone());
                if let Some(flavor) = global.flavor {
                    let name = crate::flavor_name(flavor);
                    let target = mrq_core::flavor::Registry::builtin()
                        .get(name.as_str())
                        .ok_or_else(|| format!("unknown flavor `{}`", name.as_str()))?;
                    renderer = renderer.with_target(target);
                }
                Self::Md(renderer)
            }
            RenderFormat::Json => Self::Json,
            RenderFormat::Man => Self::Man(ManRenderer::new(ManOptions {
                source: Some(format!("mrq {}", env!("CARGO_PKG_VERSION"))),
                ..ManOptions::default()
            })),
            #[cfg(feature = "pdf")]
            RenderFormat::Pdf => {
                use mrq_render::pdf::{PdfOptions, PdfRenderer};
                // Typst cannot draw Mermaid's `<foreignObject>` labels.
                #[cfg(feature = "mermaid")]
                let registry = diagrams(loaded).replacing(
                    "builtin:mermaid",
                    mrq_render::diagram::MermaidRenderer::new().with_resvg_safe_svg(true),
                );
                #[cfg(not(feature = "mermaid"))]
                let registry = diagrams(loaded);
                let renderer = PdfRenderer::new(&theme(args, loaded)?, PdfOptions::default())
                    .map_err(|e| e.to_string())?
                    .with_diagrams(registry);
                Self::Pdf(renderer)
            }
            #[cfg(not(feature = "pdf"))]
            RenderFormat::Pdf => unreachable!("checked in `run`"),
        })
    }

    /// `doc` rendered, its warnings added to `warnings`; `Err` once reported.
    fn render_one(&self, doc: &Document, warnings: &mut Warnings) -> Result<Vec<u8>, ()> {
        let (bytes, diagnostics) = match self {
            Self::Html(renderer) => {
                let out = renderer.render_with_diagnostics(doc);
                (out.html.into_bytes(), out.diagnostics)
            }
            Self::Ansi(renderer, caps) => {
                let styled = renderer.render(doc);
                (styled.to_ansi(caps).into_bytes(), styled.diagnostics)
            }
            Self::Md(renderer) => {
                let out = renderer.render_with_diagnostics(doc);
                (out.markdown.into_bytes(), out.diagnostics)
            }
            Self::Json => (mrq_render::json::render(doc).into_bytes(), Vec::new()),
            Self::Man(renderer) => {
                let out = renderer.render_with_diagnostics(doc);
                (out.roff.into_bytes(), out.diagnostics)
            }
            #[cfg(feature = "pdf")]
            Self::Pdf(renderer) => match renderer.render_with_diagnostics(doc) {
                Ok(out) => (out.pdf, out.diagnostics),
                Err(err) => {
                    report(&format!("{}: {err}", doc.name().slashed()));
                    return Err(());
                }
            },
        };
        warnings.add(doc, diagnostics);
        Ok(bytes)
    }
}

/// The theme of `-s`, else of `render.style` (relative to its config file), else
/// `light`.
fn theme(args: &RenderArgs, loaded: &LoadedConfig) -> Result<Theme, String> {
    configured_theme(args.style.as_deref(), loaded)
}

/// The theme `style` (relative to the current directory), else of `render.style`
/// (relative to its config file), else `light`.
pub(crate) fn configured_theme(
    style: Option<&str>,
    loaded: &LoadedConfig,
) -> Result<Theme, String> {
    let (style, dir) = if let Some(style) = style {
        (style, PathBuf::from("."))
    } else if let Some(style) = &loaded.config().render.style {
        let dir = match loaded.origin(&["render", "style"]) {
            Some(Origin::File { path, .. }) => {
                path.parent().map_or_else(PathBuf::new, Path::to_path_buf)
            }
            _ => PathBuf::from("."),
        };
        (style.as_str(), dir)
    } else {
        return Ok(Theme::default());
    };
    Theme::resolve(style, &dir).map_err(|err| err.to_string())
}

/// The diagram renderers of `render.diagrams`, cached in the user cache directory when
/// there is one.
pub(crate) fn diagrams(loaded: &LoadedConfig) -> Registry {
    let registry = Registry::from_loaded(loaded);
    match DiagramCache::in_user_cache(|key| env::var(key).ok()) {
        Some(cache) => registry.with_cache(cache),
        None => registry,
    }
}

pub(crate) const fn color_choice(color: ColorChoice) -> ansi::ColorChoice {
    match color {
        ColorChoice::Auto => ansi::ColorChoice::Auto,
        ColorChoice::Always => ansi::ColorChoice::Always,
        ColorChoice::Never => ansi::ColorChoice::Never,
    }
}

/// The width of the terminal on stdout, else `$COLUMNS`, else 80.
fn terminal_width() -> usize {
    terminal_size::terminal_size()
        .map(|(width, _)| usize::from(width.0))
        .or_else(|| env::var("COLUMNS").ok()?.trim().parse().ok())
        .filter(|&width| width > 0)
        .unwrap_or(80)
}

/// Writes `bytes` to `path` atomically, creating its directory.
fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => fs::create_dir_all(dir)?,
        _ => {}
    }
    crate::write_atomic(path, bytes)
}

fn usage() -> ExitCode {
    ExitCode::from(crate::EXIT_USAGE)
}

/// Writes `mrq: {message}` to stderr.
fn report(message: &str) {
    let _ = writeln!(io::stderr().lock(), "mrq: {message}");
}
