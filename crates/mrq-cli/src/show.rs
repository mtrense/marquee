//! `mrq show`: the inputs in the terminal viewer of mrq-tui (feature `tui`).

use std::io::{self, Write};
use std::process::ExitCode;

use crate::cli::{GlobalArgs, ShowArgs};

/// Loads the configuration, the inputs and the `render.style` theme, then runs the viewer
/// until it quits. Exits 2, before the screen is touched, if an input cannot be read, a
/// directory holds no Markdown file, the theme cannot be loaded or stdout is not a
/// terminal; also if the terminal fails, or without the `tui` feature. The document's
/// colours follow `--color`, `NO_COLOR` and `COLORTERM` as for `render -o ansi`.
#[cfg(feature = "tui")]
pub fn command(args: &ShowArgs, global: &GlobalArgs) -> ExitCode {
    use std::io::IsTerminal;

    use mrq_render::ansi::{AnsiOptions, AnsiRenderer, Capabilities};

    let Some((loaded, options, inputs)) = crate::prepare_loaded(global, &args.inputs, false) else {
        return usage();
    };
    if inputs.is_empty() {
        report("no Markdown files to show");
        return usage();
    }
    let mut documents = Vec::with_capacity(inputs.len());
    for input in &inputs {
        match options.read(input) {
            Ok(doc) => documents.push(doc),
            Err(err) => {
                report(&err.to_string());
                return usage();
            }
        }
    }
    let renderer = crate::render::configured_theme(None, &loaded).and_then(|theme| {
        // Images come with M10.7; the width follows the document panel.
        AnsiRenderer::new(&theme, AnsiOptions::default()).map_err(|err| err.to_string())
    });
    let renderer = match renderer {
        Ok(renderer) => renderer.with_diagrams(crate::render::diagrams(&loaded)),
        Err(message) => {
            report(&message);
            return usage();
        }
    };
    if !io::stdout().is_terminal() {
        report("`show` needs a terminal on stdout");
        return usage();
    }
    // `--color` and `NO_COLOR` decide the document's colours, `COLORTERM` their depth.
    let colors = Capabilities::from_env(crate::render::color_choice(global.color), true).color;
    let mut app = mrq_tui::App::new(documents, renderer).with_color_level(colors);
    match mrq_tui::run(&mut app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            report(&format!("terminal: {err}"));
            usage()
        }
    }
}

/// Without the `tui` feature: an error, exit 2.
#[cfg(not(feature = "tui"))]
pub fn command(_args: &ShowArgs, _global: &GlobalArgs) -> ExitCode {
    report("`show` needs mrq built with the `tui` feature");
    usage()
}

fn usage() -> ExitCode {
    ExitCode::from(crate::EXIT_USAGE)
}

/// Writes `mrq: {message}` to stderr.
fn report(message: &str) {
    let _ = writeln!(io::stderr().lock(), "mrq: {message}");
}
