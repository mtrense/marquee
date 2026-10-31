//! [`CommandRenderer`]: diagrams from external commands, the source on stdin and the
//! diagram on stdout.

use std::path::Path;
use std::time::Duration;

use mrq_core::process::{self, RunError};

use super::{DiagramError, DiagramFormat, DiagramOutput, DiagramRenderer};

/// A [`DiagramRenderer`] that runs an external command per format: each gets the diagram
/// source on stdin and writes the diagram to stdout. A command that is not installed is
/// [unavailable](DiagramError::Unavailable); one that exits with a non-zero status (its
/// stderr goes into the error), runs longer than the timeout (it is killed) or writes
/// something that is not the format [fails](DiagramError::Failed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRenderer {
    id: String,
    languages: Vec<String>,
    /// In the order added.
    formats: Vec<DiagramFormat>,
    /// Per entry of `formats`: program and arguments.
    commands: Vec<Vec<String>>,
    timeout: Duration,
}

impl CommandRenderer {
    /// How long a command may run by default; generous, since some (`mmdc`) start a
    /// browser.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    /// A renderer named `id` for `languages` (matched ignoring ASCII case), without
    /// commands yet.
    pub fn new(id: impl Into<String>, languages: &[&str]) -> Self {
        Self {
            id: id.into(),
            languages: languages
                .iter()
                .map(|lang| lang.to_ascii_lowercase())
                .collect(),
            formats: Vec::new(),
            commands: Vec::new(),
            timeout: Self::DEFAULT_TIMEOUT,
        }
    }

    /// Renders `format` by running `argv` (program and arguments), replacing an earlier
    /// command for it.
    #[must_use]
    pub fn with_command(mut self, format: DiagramFormat, argv: Vec<String>) -> Self {
        if let Some(i) = self.formats.iter().position(|&f| f == format) {
            self.commands[i] = argv;
        } else {
            self.formats.push(format);
            self.commands.push(argv);
        }
        self
    }

    /// Kills commands that run longer than `timeout`.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// A `render.diagrams.commands` entry: `argv` renders `lang` to SVG. Its id is
    /// `command:` and the program's file name.
    pub fn configured(lang: &str, argv: Vec<String>) -> Self {
        let name = argv.first().map_or(lang, |program| {
            Path::new(program)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(program)
        });
        Self::new(format!("command:{name}"), &[lang]).with_command(DiagramFormat::Svg, argv)
    }

    /// mermaid-cli: `mmdc` for `mermaid`, to SVG and PNG.
    pub fn mmdc() -> Self {
        let mmdc = |format: &str| argv(&["mmdc", "-i", "-", "-o", "-", "-e", format, "-q"]);
        Self::new("command:mmdc", &["mermaid"])
            .with_command(DiagramFormat::Svg, mmdc("svg"))
            .with_command(DiagramFormat::Png, mmdc("png"))
    }

    /// Graphviz: `dot` for `dot` and `graphviz`, to SVG and PNG.
    pub fn dot() -> Self {
        Self::new("command:dot", &["dot", "graphviz"])
            .with_command(DiagramFormat::Svg, argv(&["dot", "-Tsvg"]))
            .with_command(DiagramFormat::Png, argv(&["dot", "-Tpng"]))
    }

    /// D2: `d2` for `d2`, to SVG.
    pub fn d2() -> Self {
        Self::new("command:d2", &["d2"]).with_command(DiagramFormat::Svg, argv(&["d2", "-", "-"]))
    }

    /// `PlantUML`: `plantuml` for `plantuml`, to SVG, PNG and ASCII art.
    pub fn plantuml() -> Self {
        let plantuml = |format: &str| argv(&["plantuml", format, "-pipe"]);
        Self::new("command:plantuml", &["plantuml"])
            .with_command(DiagramFormat::Svg, plantuml("-tsvg"))
            .with_command(DiagramFormat::Png, plantuml("-tpng"))
            .with_command(DiagramFormat::Text, plantuml("-ttxt"))
    }
}

/// `args` as owned strings.
fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|&arg| arg.to_owned()).collect()
}

impl DiagramRenderer for CommandRenderer {
    fn id(&self) -> &str {
        &self.id
    }

    fn languages(&self) -> &[String] {
        &self.languages
    }

    fn formats(&self, _lang: &str) -> &[DiagramFormat] {
        &self.formats
    }

    /// The id and every command line.
    fn fingerprint(&self) -> String {
        let mut fingerprint = format!("command\0{}", self.id);
        for (format, argv) in self.formats.iter().zip(&self.commands) {
            fingerprint.push('\0');
            fingerprint.push_str(format.as_str());
            for arg in argv {
                fingerprint.push('\x1f');
                fingerprint.push_str(arg);
            }
        }
        fingerprint
    }

    fn render(
        &self,
        _lang: &str,
        src: &str,
        format: DiagramFormat,
    ) -> Result<DiagramOutput, DiagramError> {
        let Some(i) = self.formats.iter().position(|&f| f == format) else {
            return Err(DiagramError::Unavailable(format!("renders no {format}")));
        };
        let Some((program, args)) = self.commands[i].split_first() else {
            return Err(DiagramError::Failed("no command is configured".to_owned()));
        };
        let stdout =
            process::run(program, args, src.as_bytes(), self.timeout).map_err(|err| match err {
                RunError::NotStarted {
                    not_found: true, ..
                } => DiagramError::Unavailable(err.to_string()),
                _ => DiagramError::Failed(err.to_string()),
            })?;
        DiagramOutput::from_bytes(format, stdout)
            .map_err(|reason| DiagramError::Failed(format!("`{program}`: {reason}")))
    }
}
