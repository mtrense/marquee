//! The clap command tree of `mrq`.
//!
//! Every subcommand documented in the README is declared here with its flags.
//! Commands that are not implemented yet are dispatched to a stub that
//! reports "not implemented" and exits with code 2 (see [`crate::run`]).

use std::path::PathBuf;

use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

/// A Markdown toolkit for the terminal: verify, lint, format, query,
/// transform, render and display Markdown.
#[derive(Debug, Parser)]
#[command(name = "mrq", version)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

/// Flags accepted by every subcommand.
#[derive(Debug, Args)]
#[command(next_help_heading = "Global options")]
pub struct GlobalArgs {
    /// Use this config file instead of discovering `.mrq.yaml`.
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Markdown flavor [default: frontmatter `mrq.flavor`, then config, then gfm].
    #[arg(long, global = true, value_name = "FLAVOR")]
    pub flavor: Option<Flavor>,

    /// When to use colors.
    #[arg(long, global = true, value_name = "WHEN", default_value = "auto")]
    pub color: ColorChoice,

    #[arg(
        long,
        global = true,
        value_name = "FORMAT",
        help = "Diagnostics output format [default: human, or github when GITHUB_ACTIONS=true]"
    )]
    pub format: Option<DiagnosticFormat>,

    /// Print less output; repeat for even less.
    #[arg(short, long, global = true, action = ArgAction::Count, conflicts_with = "verbose")]
    pub quiet: u8,

    /// Print more output; repeat for even more.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,
}

/// A built-in Markdown flavor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Flavor {
    #[value(help = "Strict CommonMark")]
    Commonmark,
    /// GitHub Flavored Markdown.
    Gfm,
    /// Obsidian: gfm + wikilinks, embeds, callouts, tags, math, comments.
    Obsidian,
    #[value(help = "MkDocs: gfm + admonitions, heading attributes, definition lists, math")]
    Mkdocs,
}

/// When to emit ANSI colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

/// Output format for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DiagnosticFormat {
    /// Human-readable, with source snippets.
    Human,
    Json,
    /// One JSON object per line.
    Ndjson,
    /// SARIF 2.1.0.
    Sarif,
    /// GitHub Actions workflow annotations.
    Github,
}

/// The `mrq` subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the heading outline of each file.
    Toc(TocArgs),
    /// Render files or stdin to HTML, ANSI, PDF, Markdown, JSON or man.
    Render(RenderArgs),
    /// Check links, anchors, assets, footnotes, includes and frontmatter.
    Validate(ValidateArgs),
    /// Apply markdownlint-compatible style rules.
    Lint(LintArgs),
    /// Format Markdown canonically.
    Fmt(FmtArgs),
    /// Select parts of documents.
    Query(QueryArgs),
    /// Combine several files into one document.
    Merge(MergeArgs),
    /// Expand `mrq:include` directives.
    Interpolate(InterpolateArgs),
    /// Split a file into one file per section.
    Split(SplitArgs),
    /// Count words, headings, links, code blocks, tasks and more.
    Stats(StatsArgs),
    /// Structural, semantic diff of two documents.
    Diff(DiffArgs),
    /// Browse files in an interactive viewer.
    Show(ShowArgs),
    /// Run the language server on stdio.
    Lsp,
    /// Inspect the configuration.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Print a shell completion script to stdout.
    Completions {
        /// Target shell.
        shell: Shell,
    },
}

impl Command {
    /// The subcommand name as typed on the command line.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Toc(_) => "toc",
            Self::Render(_) => "render",
            Self::Validate(_) => "validate",
            Self::Lint(_) => "lint",
            Self::Fmt(_) => "fmt",
            Self::Query(_) => "query",
            Self::Merge(_) => "merge",
            Self::Interpolate(_) => "interpolate",
            Self::Split(_) => "split",
            Self::Stats(_) => "stats",
            Self::Diff(_) => "diff",
            Self::Show(_) => "show",
            Self::Lsp => "lsp",
            Self::Config(ConfigCommand::Show { .. }) => "config show",
            Self::Config(ConfigCommand::Schema) => "config schema",
            Self::Completions { .. } => "completions",
        }
    }
}

/// Input files, directories, globs or `-` for stdin.
#[derive(Debug, Args)]
pub struct Inputs {
    /// Files, directories, globs, or `-` for stdin.
    #[arg(value_name = "PATH")]
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools, reason = "independent clap flags")]
pub struct TocArgs {
    /// Emit level, text, slug and source position per heading as JSON.
    #[arg(long)]
    pub json: bool,
    /// Only include headings up to this level.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=6))]
    pub depth: Option<u8>,
    /// Print a nested Markdown list with anchor links.
    #[arg(long, conflicts_with = "json")]
    pub links: bool,
    /// Refresh a `<!-- mrq:toc -->` … `<!-- mrq:end -->` block in place.
    #[arg(long, conflicts_with = "json")]
    pub insert: bool,
    /// With --insert: write nothing, list out-of-date files and exit 1 if there are any.
    #[arg(long, requires = "insert")]
    pub check: bool,
    #[command(flatten)]
    pub inputs: Inputs,
}

/// Output format of `mrq render`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RenderFormat {
    /// Standalone HTML page (or body only with --fragment).
    Html,
    /// Terminal output with colors and highlighting.
    Ansi,
    /// PDF via embedded Typst.
    Pdf,
    /// Normalized Markdown, optionally in another flavor.
    Md,
    /// Stable mdast-like AST with source positions.
    Json,
    /// roff, for man pages.
    Man,
}

#[derive(Debug, Args)]
pub struct RenderArgs {
    /// Output format.
    #[arg(short, long = "output", value_name = "FORMAT", default_value = "html")]
    pub output: RenderFormat,
    /// Built-in theme (light, dark, github, paper) or a theme file.
    #[arg(short, long, value_name = "STYLE")]
    pub style: Option<String>,
    /// HTML body only, without the `<html>` wrapper.
    #[arg(long)]
    pub fragment: bool,
    #[command(flatten)]
    pub inputs: Inputs,
}

#[derive(Debug, Args)]
pub struct ValidateArgs {
    /// Also check external http(s) links.
    #[arg(long)]
    pub external: bool,
    /// Exit 1 on warnings too, not only on errors.
    #[arg(long)]
    pub strict: bool,
    #[command(flatten)]
    pub inputs: Inputs,
}

#[derive(Debug, Args)]
pub struct LintArgs {
    /// Apply automatic fixes in place (stdin: print the fixed text), then report what is left.
    #[arg(long)]
    pub fix: bool,
    /// Like --fix, but report nothing and exit 0 even if findings without a fix remain.
    #[arg(long)]
    pub fix_only: bool,
    /// Do not write; print the fixes as a unified diff and exit 1 if there are any.
    #[arg(long, conflicts_with_all = ["fix", "fix_only"])]
    pub diff: bool,
    /// Only run this rule, alias or tag (repeatable), even if the config turns it off.
    #[arg(long, value_name = "RULE")]
    pub rule: Vec<String>,
    #[command(flatten)]
    pub inputs: Inputs,
}

#[derive(Debug, Args)]
pub struct FmtArgs {
    /// Do not write; exit 1 and show a diff if anything would change.
    #[arg(long)]
    pub check: bool,
    #[command(flatten)]
    pub inputs: Inputs,
}

/// Output format of `mrq query`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum QueryOutput {
    /// Markdown.
    Md,
    /// Nodes with source positions.
    Json,
    /// Plain text content.
    Text,
}

#[derive(Debug, Args)]
pub struct QueryArgs {
    /// Interpret QUERY as a CSS-style selector.
    #[arg(long, conflicts_with = "jq")]
    pub css: bool,
    /// Interpret QUERY as a jq program over the JSON AST.
    #[arg(long)]
    pub jq: bool,
    /// Output format.
    #[arg(short, long = "output", value_name = "FORMAT", default_value = "md")]
    pub output: QueryOutput,
    /// Exit 1 if nothing matches in any input.
    #[arg(short, long)]
    pub exit_status: bool,
    /// The query (Markdown selector by default).
    #[arg(allow_hyphen_values = true)]
    pub query: String,
    #[command(flatten)]
    pub inputs: Inputs,
}

/// How `mrq merge` combines frontmatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FrontmatterMode {
    /// Keep the first file's frontmatter only.
    KeepFirst,
    /// Merge all frontmatter; the first file wins on conflicts.
    Merge,
    /// Drop all frontmatter.
    Strip,
}

#[derive(Debug, Args)]
pub struct MergeArgs {
    /// Push each file's headings down by N levels (default: one per heading level above
    /// them, the section headings and the title).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=5))]
    pub shift: Option<u8>,
    /// Title of the merged document.
    #[arg(long, value_name = "TITLE")]
    pub title: Option<String>,
    /// Concatenate without wrapping each file in a section heading.
    #[arg(long)]
    pub no_wrap: bool,
    /// How to combine frontmatter.
    #[arg(long, value_name = "MODE", default_value = "merge")]
    pub frontmatter: FrontmatterMode,
    /// Write to this file instead of stdout.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,
    #[command(flatten)]
    pub inputs: Inputs,
}

#[derive(Debug, Args)]
pub struct InterpolateArgs {
    /// Refresh the content between the markers in place (stdin: print the result).
    #[arg(long, conflicts_with = "check")]
    pub in_place: bool,
    /// Do not write; exit 1 and show a diff if --in-place would change anything.
    #[arg(long)]
    pub check: bool,
    /// Print the content between the markers instead of replacing the markers with it.
    #[arg(long, conflicts_with_all = ["in_place", "check"])]
    pub keep_directives: bool,
    #[command(flatten)]
    pub inputs: Inputs,
}

#[derive(Debug, Args)]
pub struct SplitArgs {
    /// Heading level to split at.
    #[arg(short, long, value_name = "LEVEL", default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=6))]
    pub depth: u8,
    /// Directory to write the files to [default: the file's path without its extension].
    #[arg(short, long, value_name = "DIR")]
    pub out: Option<PathBuf>,
    /// Do not write; list the files that would be written.
    #[arg(long)]
    pub dry_run: bool,
    /// The file to split, or `-` for stdin (needs --out).
    #[arg(value_name = "FILE")]
    pub file: PathBuf,
}

#[derive(Debug, Args)]
pub struct StatsArgs {
    /// Emit per-file and total counts as JSON.
    #[arg(long)]
    pub json: bool,
    /// Reading speed for the reading-time estimate, in words per minute.
    #[arg(long, value_name = "N", default_value_t = 200,
          value_parser = clap::value_parser!(u32).range(1..))]
    pub wpm: u32,
    /// Count words and characters in code blocks and math too.
    #[arg(long)]
    pub include_code: bool,
    #[command(flatten)]
    pub inputs: Inputs,
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    /// Emit the diff as JSON.
    #[arg(long)]
    pub json: bool,
    /// Show word-level changes inside modified paragraphs.
    #[arg(long)]
    pub word: bool,
    /// The old document.
    #[arg(value_name = "OLD")]
    pub old: PathBuf,
    /// The new document.
    #[arg(value_name = "NEW")]
    pub new: PathBuf,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    #[command(flatten)]
    pub inputs: Inputs,
}

/// `mrq config` subcommands.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the effective configuration.
    Show {
        /// Show where each value came from.
        #[arg(long)]
        origin: bool,
    },
    /// Print the JSON Schema of `.mrq.yaml`.
    Schema,
}

/// The clap `Command` for `mrq`, e.g. for man pages and completions.
pub fn command() -> clap::Command {
    Cli::command()
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_tree_is_consistent() {
        super::command().debug_assert();
    }
}
