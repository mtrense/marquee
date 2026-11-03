//! `mrq diff`: the semantic diff of two documents ([`mrq_core::diff`]), as breadcrumbed
//! sections with `-`/`+` blocks or, with `--json`, a versioned object. Also usable as
//! `git difftool -x 'mrq diff'`, which passes two temp files (`/dev/null` for a side
//! that does not exist).

use std::env;
use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

use mrq_core::Document;
use mrq_core::Slashed as _;
use mrq_core::diag::HumanStyle;
use mrq_core::diff::{self, Block, BlockOp, DiffOptions, DocDiff, SectionDiff, SectionSide};
use mrq_core::diff::{SectionStatus, WordOp};
use mrq_core::input::{Input, InputError};
use mrq_core::source::SourceFile;
use serde::Serialize;

use crate::cli::{ColorChoice, DiffArgs, GlobalArgs};
use crate::{EXIT_USAGE, document_options, load_config, report_config_error};

/// Exit code for a non-empty diff.
const EXIT_CHANGED: u8 = 1;

/// The version of the `--json` shape.
const JSON_VERSION: u32 = 1;

/// ANSI styles of the human output.
const BOLD: &str = "\x1b[1m";
const CYAN: &str = "\x1b[36m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const RESET: &str = "\x1b[0m";

/// Diffs `OLD` against `NEW` and prints the changed sections; see the README for the
/// format. Exits 0 if the documents differ only in formatting, 1 if they differ, and 2
/// if the configuration or an input cannot be loaded (nothing is printed on stdout).
pub fn command(args: &DiffArgs, global: &GlobalArgs) -> ExitCode {
    let stdin = Path::new("-");
    if args.old == stdin && args.new == stdin {
        return fail("only one of OLD and NEW can be `-` (stdin)");
    }
    let options = match load_config(global) {
        Ok(loaded) => document_options(global, &loaded),
        Err(err) => {
            report_config_error(&err, global);
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let read = |path: &Path| -> Result<Document, String> {
        if path == stdin {
            return options.read(&Input::stdin()).map_err(|err| err.to_string());
        }
        if is_null_device(path) {
            let source = SourceFile::new(Some(path.to_path_buf()), "");
            return options.parse_source(source).map_err(|err| err.to_string());
        }
        // Checked first for a message that is the same on every OS.
        if !path.exists() {
            let path = path.to_path_buf();
            return Err(InputError::NotFound { path }.to_string());
        }
        options
            .read(&Input::file(path))
            .map_err(|err| err.to_string())
    };
    let (old, new) = match (read(&args.old), read(&args.new)) {
        (Ok(old), Ok(new)) => (old, new),
        (Err(err), _) | (_, Err(err)) => return fail(&err),
    };

    let diff = diff::diff(&old, &new, &DiffOptions { word: args.word });
    let names = (name(&args.old), name(&args.new));
    let text = if args.json {
        json(&diff, &names)
    } else {
        human(&diff, &names, color(global))
    };
    let _ = io::stdout().lock().write_all(text.as_bytes());
    if diff.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_CHANGED)
    }
}

fn fail(message: &str) -> ExitCode {
    let _ = writeln!(io::stderr().lock(), "mrq: {message}");
    ExitCode::from(EXIT_USAGE)
}

/// Whether `path` is the empty side `git difftool` passes for an added or deleted file
/// and that cannot be read as a file: on Windows, Git passes `nul` (or `/dev/null`).
fn is_null_device(path: &Path) -> bool {
    cfg!(windows)
        && (path.as_os_str().eq_ignore_ascii_case("nul") || path == Path::new("/dev/null"))
}

/// A side's name as given, `<stdin>` for `-`.
fn name(path: &Path) -> String {
    if path == Path::new("-") {
        "<stdin>".to_owned()
    } else {
        path.slashed()
    }
}

/// Whether to color stdout: `--color`, else only on a terminal without `NO_COLOR`.
fn color(global: &GlobalArgs) -> bool {
    match global.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            HumanStyle::detect(io::stdout().is_terminal(), |key| env::var(key).ok()).color
        }
    }
}

/// The human output: nothing for an empty diff, else `--- OLD` and `+++ NEW`, then each
/// changed section as a header line followed by its changed blocks.
fn human(diff: &DocDiff, (old, new): &(String, String), color: bool) -> String {
    let mut out = String::new();
    if diff.is_empty() {
        return out;
    }
    let paint = |style: &str, text: &str| {
        if color {
            format!("{style}{text}{RESET}")
        } else {
            text.to_owned()
        }
    };
    let _ = writeln!(out, "{}", paint(BOLD, &format!("--- {old}")));
    let _ = writeln!(out, "{}", paint(BOLD, &format!("+++ {new}")));
    for section in diff.changes() {
        out.push('\n');
        let _ = writeln!(out, "{}", paint(&format!("{BOLD}{CYAN}"), &header(section)));
        for op in &section.blocks {
            match op {
                BlockOp::Equal { .. } => {}
                BlockOp::Insert { new } => lines(&mut out, "+", new, color.then_some(GREEN)),
                BlockOp::Delete { old } => lines(&mut out, "-", old, color.then_some(RED)),
                BlockOp::Modify {
                    words: Some(words), ..
                } => word_lines(&mut out, words, color),
                BlockOp::Modify { old, new, .. } => {
                    lines(&mut out, "-", old, color.then_some(RED));
                    lines(&mut out, "+", new, color.then_some(GREEN));
                }
            }
        }
    }
    out
}

/// A section's header: its heading level and path (`## Guide › Usage`, `(preamble)` for
/// the root), what happened to it in brackets, then its 1-based line in each document
/// (`old:12 new:14`).
fn header(section: &SectionDiff) -> String {
    let side = section
        .new
        .as_ref()
        .or(section.old.as_ref())
        .expect("a section is in one document at least");
    let mut out = breadcrumb(side);
    let mut tags = Vec::new();
    match section.status {
        SectionStatus::Added => tags.push("added".to_owned()),
        SectionStatus::Removed => tags.push("removed".to_owned()),
        SectionStatus::Matched => {
            let old = section
                .old
                .as_ref()
                .expect("a matched section has an old side");
            if section.moved {
                tags.push(format!("moved from {}", path(&old.path)));
                if section.renamed {
                    tags.push("renamed".to_owned());
                }
            } else if section.renamed {
                tags.push(format!("renamed from {}", old.title));
            }
        }
    }
    if !tags.is_empty() {
        let _ = write!(out, "  [{}]", tags.join(", "));
    }
    out.push(' ');
    for (label, side) in [("old", &section.old), ("new", &section.new)] {
        if let Some(side) = side {
            let _ = write!(out, " {label}:{}", side.line + 1);
        }
    }
    out
}

fn breadcrumb(side: &SectionSide) -> String {
    if side.level == 0 {
        "(preamble)".to_owned()
    } else {
        format!("{} {}", "#".repeat(side.level.into()), path(&side.path))
    }
}

fn path(path: &[String]) -> String {
    if path.is_empty() {
        "(preamble)".to_owned()
    } else {
        path.join(" › ")
    }
}

/// Each line of `block`'s canonical form after `marker` and a space, in `style`.
fn lines(out: &mut String, marker: &str, block: &Block, style: Option<&str>) {
    for line in block.canonical.split('\n') {
        let text = format!("{marker} {line}");
        let text = text.trim_end();
        let _ = match style {
            Some(style) => writeln!(out, "{style}{text}{RESET}"),
            None => writeln!(out, "{text}"),
        };
    }
}

/// A modified block word by word, each line after `~ `: deleted runs as `[-…-]` and
/// inserted runs as `{+…+}`, or in red and green with `color`.
fn word_lines(out: &mut String, words: &[WordOp], color: bool) {
    let mut line = String::from("~ ");
    for word in words {
        let (text, (open, close)) = match word {
            WordOp::Equal(text) => (text, ("", "")),
            WordOp::Delete(text) if color => (text, (RED, RESET)),
            WordOp::Insert(text) if color => (text, (GREEN, RESET)),
            WordOp::Delete(text) => (text, ("[-", "-]")),
            WordOp::Insert(text) => (text, ("{+", "+}")),
        };
        for (index, part) in text.split('\n').enumerate() {
            if index > 0 {
                let _ = writeln!(out, "{}", line.trim_end());
                line = String::from("~ ");
            }
            if !part.is_empty() {
                let _ = write!(line, "{open}{part}{close}");
            }
        }
    }
    let _ = writeln!(out, "{}", line.trim_end());
}

/// The `--json` output: `{"mrq_diff": 1, "old", "new", "sections"}` with the changed
/// sections only, and their blocks without the equal ones.
fn json(diff: &DocDiff, (old, new): &(String, String)) -> String {
    let doc = JsonDiff {
        mrq_diff: JSON_VERSION,
        old,
        new,
        sections: diff.changes().map(JsonSection::of).collect(),
    };
    let mut json = serde_json::to_string_pretty(&doc).expect("a diff serializes");
    json.push('\n');
    json
}

#[derive(Serialize)]
struct JsonDiff<'a> {
    mrq_diff: u32,
    old: &'a str,
    new: &'a str,
    sections: Vec<JsonSection<'a>>,
}

#[derive(Serialize)]
struct JsonSection<'a> {
    status: SectionStatus,
    moved: bool,
    renamed: bool,
    modified: bool,
    old: Option<JsonSide<'a>>,
    new: Option<JsonSide<'a>>,
    blocks: Vec<JsonOp<'a>>,
}

impl<'a> JsonSection<'a> {
    fn of(section: &'a SectionDiff) -> Self {
        Self {
            status: section.status,
            moved: section.moved,
            renamed: section.renamed,
            modified: section.modified,
            old: section.old.as_ref().map(JsonSide::of),
            new: section.new.as_ref().map(JsonSide::of),
            blocks: section.blocks.iter().filter_map(JsonOp::of).collect(),
        }
    }
}

/// Where a section is: its path of titles (empty for the root), title, level (0 for the
/// root) and the 1-based line of its heading.
#[derive(Serialize)]
struct JsonSide<'a> {
    path: &'a [String],
    title: &'a str,
    level: u8,
    line: usize,
}

impl<'a> JsonSide<'a> {
    fn of(side: &'a SectionSide) -> Self {
        Self {
            path: &side.path,
            title: &side.title,
            level: side.level,
            line: side.line + 1,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum JsonOp<'a> {
    Insert {
        kind: &'static str,
        new: JsonBlock<'a>,
    },
    Delete {
        kind: &'static str,
        old: JsonBlock<'a>,
    },
    Modify {
        kind: &'static str,
        old: JsonBlock<'a>,
        new: JsonBlock<'a>,
        #[serde(skip_serializing_if = "Option::is_none")]
        words: Option<&'a [WordOp]>,
    },
}

impl<'a> JsonOp<'a> {
    fn of(op: &'a BlockOp) -> Option<Self> {
        Some(match op {
            BlockOp::Equal { .. } => return None,
            BlockOp::Insert { new } => Self::Insert {
                kind: new.kind.as_str(),
                new: JsonBlock::of(new),
            },
            BlockOp::Delete { old } => Self::Delete {
                kind: old.kind.as_str(),
                old: JsonBlock::of(old),
            },
            BlockOp::Modify { old, new, words } => Self::Modify {
                kind: new.kind.as_str(),
                old: JsonBlock::of(old),
                new: JsonBlock::of(new),
                words: words.as_deref(),
            },
        })
    }
}

/// A block's canonical text and its first and last 1-based source lines.
#[derive(Serialize)]
struct JsonBlock<'a> {
    text: &'a str,
    lines: [usize; 2],
}

impl<'a> JsonBlock<'a> {
    fn of(block: &'a Block) -> Self {
        Self {
            text: &block.canonical,
            lines: [block.lines.start + 1, block.lines.end],
        }
    }
}
