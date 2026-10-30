//! `mrq query`: select parts of every input with a selector, CSS or jq query and print
//! them as Markdown, plain text or JSON.

use std::io::{self, Write};
use std::process::ExitCode;

use mrq_core::document::{Document, DocumentOptions};
use mrq_core::input::Input;
use mrq_core::query::{JsonOutput, Match, Query, QueryError, Syntax};
use serde_json::{Value, json};

use crate::cli::{QueryArgs, QueryOutput};

/// Exit code of `--exit-status` when nothing matched.
const EXIT_NO_MATCH: u8 = 1;

/// Compiles the query in the syntax picked by `--css`/`--jq` (the selector DSL
/// otherwise). A syntax error is printed on stderr as the query with a caret under the
/// problem (`None`, exit 2).
pub fn compile(args: &QueryArgs) -> Option<Query> {
    let compiled = if args.css {
        Query::css(&args.query)
    } else if args.jq {
        Query::jq(&args.query)
    } else {
        Query::selector(&args.query)
    };
    match compiled {
        Ok(query) => Some(query),
        Err(QueryError::Syntax(err)) => {
            let _ = writeln!(io::stderr().lock(), "mrq: invalid query\n{}", err.render());
            None
        }
        Err(err) => {
            let _ = writeln!(io::stderr().lock(), "mrq: {err}");
            None
        }
    }
}

/// Prints the matches of `query` in every input.
///
/// `md` prints the source of each match (sections in full, from the heading through
/// their content) with a blank line between matches; `text` prints the plain text of
/// each match on its own line(s), without blank lines. jq outputs that are not unchanged
/// nodes or sections of the document have no source: strings print as is, other values
/// as compact JSON, one per line in either mode (in `md`, a blank line still separates
/// them from adjacent matches). With several inputs, files with matches get a `==> name <==` header
/// and a blank line before the next one. `json` is always an array with one
/// `{ "file", "matches" }` object per input (`file` is `null` for stdin), whose matches
/// are the [JSON projection](mrq_core::query::JsonOutput) or the jq outputs.
///
/// Inputs that cannot be read and jq programs that fail are reported on stderr, skip the
/// input and make the exit code 2. Otherwise it is 1 with `--exit-status` if nothing
/// matched, else 0.
pub fn run(
    args: &QueryArgs,
    query: &Query,
    inputs: &[Input],
    options: &DocumentOptions,
) -> ExitCode {
    let (mut failed, mut any_match) = (false, false);
    let mut files = Vec::new();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut first = true;
    for input in inputs {
        let doc = match options.read(input) {
            Ok(doc) => doc,
            Err(err) => {
                let _ = writeln!(io::stderr().lock(), "mrq: {err}");
                failed = true;
                continue;
            }
        };
        let report = |err: QueryError| {
            let name = doc.name().display();
            let _ = writeln!(io::stderr().lock(), "mrq: {name}: {err}");
        };
        if args.output == QueryOutput::Json {
            let Ok(outputs) = doc.select_json(query).map_err(report) else {
                failed = true;
                continue;
            };
            any_match |= !outputs.is_empty();
            let matches: Vec<Value> = outputs.into_iter().map(JsonOutput::into_value).collect();
            let file = doc.path().map(|path| path.display().to_string());
            files.push(json!({ "file": file, "matches": matches }));
            continue;
        }
        let Ok(results) = results(&doc, query).map_err(report) else {
            failed = true;
            continue;
        };
        any_match |= !results.is_empty();
        if results.is_empty() {
            continue;
        }
        if inputs.len() > 1 {
            let separator = if first { "" } else { "\n" };
            let _ = writeln!(out, "{separator}==> {} <==", doc.name().display());
        }
        first = false;
        let mut previous: Option<&Found<'_>> = None;
        for found in &results {
            let text = found.render(&doc, args.output);
            let blank = args.output == QueryOutput::Md
                && previous.is_some_and(|previous| found.is_match() || previous.is_match());
            let separator = if blank { "\n" } else { "" };
            previous = Some(found);
            let _ = writeln!(out, "{separator}{}", text.trim_end_matches(['\n', '\r']));
        }
    }
    if args.output == QueryOutput::Json {
        let mut json = serde_json::to_string_pretty(&files).expect("JSON values serialize");
        json.push('\n');
        let _ = out.write_all(json.as_bytes());
    }
    if failed {
        ExitCode::from(crate::EXIT_USAGE)
    } else if args.exit_status && !any_match {
        ExitCode::from(EXIT_NO_MATCH)
    } else {
        ExitCode::SUCCESS
    }
}

/// One result of the query in a document.
enum Found<'a> {
    /// A node or section of the document.
    Match(Match<'a>),
    /// A jq output that is not an unchanged node or section of the document.
    Value(Value),
}

/// The results of `query` in `doc` for `md` and `text`: the matches of a plan, or the
/// jq outputs, turned back into matches where possible.
fn results<'a>(doc: &'a Document, query: &Query) -> Result<Vec<Found<'a>>, QueryError> {
    if query.syntax() != Syntax::Jq {
        return Ok(doc.select(query)?.into_iter().map(Found::Match).collect());
    }
    Ok(doc
        .select_json(query)?
        .into_iter()
        .map(|output| match output.to_match(doc) {
            Some(found) => Found::Match(found),
            None => Found::Value(output.into_value()),
        })
        .collect())
}

impl Found<'_> {
    const fn is_match(&self) -> bool {
        matches!(self, Self::Match(_))
    }

    /// The source (`md`) or plain text (`text`) of a match; a string value as is and
    /// other values as compact JSON.
    fn render(&self, doc: &Document, output: QueryOutput) -> String {
        match self {
            Self::Match(found) if output == QueryOutput::Text => found.text(),
            Self::Match(found) => found.source(doc.text()).to_owned(),
            Self::Value(Value::String(text)) => text.clone(),
            Self::Value(value) => value.to_string(),
        }
    }
}
