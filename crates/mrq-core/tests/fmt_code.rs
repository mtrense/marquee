//! Acceptance tests for formatting fenced code in `mrq_core::fmt`: the [`CodeFormatter`]
//! seam with in-process fakes, and [`CommandFormatter`] with a fake formatter script
//! (`tests/fixtures/formatters/fake.sh`, run through `sh`).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use mrq_core::Document;
use mrq_core::config::FmtConfig;
use mrq_core::diag::Severity;
use mrq_core::document::DocumentOptions;
use mrq_core::fmt::{CodeFormatter, CommandFormatter, Formatted, codes, format, format_with};

const FAKE: &str = "tests/fixtures/formatters/fake.sh";

/// Upper-cases `rust` code and records what it was called with.
#[derive(Default)]
struct Upper {
    calls: RefCell<Vec<(String, String)>>,
}

impl CodeFormatter for Upper {
    fn format(&self, lang: &str, code: &str) -> Option<Result<String, String>> {
        self.calls
            .borrow_mut()
            .push((lang.to_owned(), code.to_owned()));
        (lang == "rust").then(|| Ok(code.to_uppercase()))
    }
}

fn fmt_flavor(src: &str, flavor: &str, code: &dyn CodeFormatter) -> Formatted {
    let doc = DocumentOptions::new().flavor(flavor).parse(src).unwrap();
    format_with(&doc, &FmtConfig::default(), code)
}

#[test]
fn code_of_a_language_with_a_formatter_is_replaced() {
    let upper = Upper::default();
    let out = fmt_flavor(
        "```rust title=\"x\"\nfn main() {}\n```\n\n```sh\necho\n```\n",
        "gfm",
        &upper,
    );
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert_eq!(
        out.text,
        "```rust title=\"x\"\nFN MAIN() {}\n```\n\n```sh\necho\n```\n"
    );
    // The formatter gets the language (the first word of the info string) and the code
    // with its final newline.
    assert_eq!(
        upper.calls.borrow().as_slice(),
        [
            ("rust".to_owned(), "fn main() {}\n".to_owned()),
            ("sh".to_owned(), "echo\n".to_owned())
        ]
    );
}

#[test]
fn code_without_a_language_is_not_formatted() {
    let upper = Upper::default();
    let out = fmt_flavor("    indented\n\n```\nplain\n```\n", "gfm", &upper);
    assert_eq!(out.text, "```\nindented\n```\n\n```\nplain\n```\n");
    assert!(upper.calls.borrow().is_empty());
}

#[test]
fn code_in_containers_is_formatted() {
    let upper = Upper::default();
    let gfm = fmt_flavor(
        "> ```rust\n> a\n> ```\n\n- item\n\n  ```rust\n  b\n  ```\n\n[^n]:\n    ```rust\n    c\n    ```\n",
        "gfm",
        &upper,
    );
    assert_eq!(
        gfm.text,
        "> ```rust\n> A\n> ```\n\n- item\n\n  ```rust\n  B\n  ```\n\n[^n]: ```rust\n    C\n    ```\n"
    );
    let obsidian = fmt_flavor("> [!note]\n> ```rust\n> a\n> ```\n", "obsidian", &upper);
    assert_eq!(obsidian.text, "> [!note]\n> ```rust\n> A\n> ```\n");
    let mkdocs = fmt_flavor(
        "!!! note\n    ```rust\n    a\n    ```\n\nTerm\n:   ```rust\n    b\n    ```\n",
        "mkdocs",
        &upper,
    );
    assert_eq!(
        mkdocs.text,
        "!!! note\n    ```rust\n    A\n    ```\n\nTerm\n:   ```rust\n    B\n    ```\n"
    );
}

#[test]
fn the_fence_outgrows_fence_like_lines_of_the_formatted_code() {
    let fences = |_: &str, code: &str| Some(Ok(format!("{code}````\n")));
    let out = fmt_flavor("```rust\na\n```\n", "gfm", &fences);
    assert_eq!(out.text, "`````rust\na\n````\n`````\n");
    // Formatting the output again adds another fence line, so it is not compared with
    // itself; without a formatter it is stable.
    let again = format(&Document::parse(out.text.as_str()), &FmtConfig::default());
    assert_eq!(again, out.text);
}

#[test]
fn formatted_code_gets_a_final_newline() {
    let bare = |_: &str, _: &str| Some(Ok("x".to_owned()));
    assert_eq!(
        fmt_flavor("```rust\na\n```\n", "gfm", &bare).text,
        "```rust\nx\n```\n"
    );
    let crlf = |_: &str, _: &str| Some(Ok("x\r\ny\r\n".to_owned()));
    assert_eq!(
        fmt_flavor("```rust\na\n```\n", "gfm", &crlf).text,
        "```rust\nx\ny\n```\n"
    );
}

#[test]
fn a_failing_formatter_leaves_the_code_with_a_warning() {
    let failing = |_: &str, _: &str| Some(Err("it broke".to_owned()));
    let src = "# A\n\n```rust\nfn  main() {}\n```\n";
    let doc = Document::parse(src);
    let out = format_with(&doc, &FmtConfig::default(), &failing);
    assert_eq!(out.text, src);
    let [warning] = out.diagnostics.as_slice() else {
        panic!("{:?}", out.diagnostics)
    };
    assert_eq!(warning.code, codes::CODE_FORMATTER);
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(
        &src[warning.range.start..warning.range.end],
        "```rust\nfn  main() {}\n```"
    );
    assert!(warning.message.contains("rust"), "{}", warning.message);
    assert!(warning.message.contains("it broke"), "{}", warning.message);
}

fn commands(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    entries
        .iter()
        .map(|(lang, argv)| {
            let argv = argv.iter().map(|arg| (*arg).to_owned()).collect();
            ((*lang).to_owned(), argv)
        })
        .collect()
}

fn fake(mode: &str) -> Vec<String> {
    ["sh", FAKE, mode].map(str::to_owned).to_vec()
}

#[cfg(unix)]
#[test]
fn commands_are_matched_by_language_ignoring_case() {
    let formatter = CommandFormatter::new(&commands(&[("Rust", &["sh", FAKE, "upper"])]));
    assert_eq!(formatter.format("rust", "a\n"), Some(Ok("A\n".to_owned())));
    assert_eq!(formatter.format("RUST", "a\n"), Some(Ok("A\n".to_owned())));
    assert_eq!(formatter.format("python", "a\n"), None);
}

#[cfg(unix)]
#[test]
fn commands_get_their_arguments_and_the_code_on_stdin() {
    let formatter = CommandFormatter::new(&commands(&[(
        "rust",
        &["sh", FAKE, "upper", "--edition", "2024"],
    )]));
    assert_eq!(
        formatter.format("rust", "fn main() {}\n"),
        Some(Ok("--edition\n2024\nFN MAIN() {}\n".to_owned()))
    );
    // Large code is streamed both ways without the pipes filling up.
    let cat = CommandFormatter::new(&commands(&[("text", &["cat"])]));
    let big = "0123456789abcdef\n".repeat(64 * 1024);
    assert_eq!(cat.format("text", &big), Some(Ok(big.clone())));
}

#[cfg(unix)]
#[test]
fn command_failures_are_errors() {
    let mut map = BTreeMap::new();
    for mode in ["fail", "slow", "binary"] {
        map.insert(mode.to_owned(), fake(mode));
    }
    map.insert(
        "missing".to_owned(),
        vec!["mrq-no-such-formatter".to_owned()],
    );
    map.insert("empty".to_owned(), Vec::new());
    let formatter = CommandFormatter::new(&map).with_timeout(Duration::from_millis(300));
    let error = |lang: &str| match formatter.format(lang, "code\n") {
        Some(Err(message)) => message,
        other => panic!("{lang}: {other:?}"),
    };
    let fail = error("fail");
    assert!(fail.contains("exit status: 3"), "{fail}");
    assert!(fail.contains("syntax error on line 1"), "{fail}");
    let started = Instant::now();
    let slow = error("slow");
    assert!(slow.contains("timed out"), "{slow}");
    assert!(started.elapsed() < Duration::from_secs(5), "{slow}");
    assert!(error("binary").contains("UTF-8"));
    assert!(error("missing").contains("mrq-no-such-formatter"));
    assert!(error("empty").contains("no command"));
}

#[cfg(unix)]
#[test]
fn configured_formatters_run_through_format() {
    let config = FmtConfig {
        code_formatters: commands(&[
            ("rust", &["sh", FAKE, "upper"]),
            ("bad", &["sh", FAKE, "fail"]),
        ]),
        ..FmtConfig::default()
    };
    let src = "```rust\nfn main() {}\n```\n\n```bad\nkeep\n```\n";
    let doc = Document::parse(src);
    let expected = "```rust\nFN MAIN() {}\n```\n\n```bad\nkeep\n```\n";
    assert_eq!(format(&doc, &config), expected);
    let out = format_with(&doc, &config, &CommandFormatter::from_config(&config));
    assert_eq!(out.text, expected);
    let [warning] = out.diagnostics.as_slice() else {
        panic!("{:?}", out.diagnostics)
    };
    assert_eq!(warning.code, codes::CODE_FORMATTER);
    assert!(
        warning.message.contains("syntax error"),
        "{}",
        warning.message
    );
}

#[cfg(unix)]
#[test]
fn formatted_fences_are_recomputed_from_command_output() {
    let config = FmtConfig {
        code_formatters: commands(&[("rust", &["sh", FAKE, "fence"])]),
        ..FmtConfig::default()
    };
    let out = format_with(
        &Document::parse("```rust\na\n```\n"),
        &config,
        &CommandFormatter::from_config(&config),
    );
    assert_eq!(out.text, "`````rust\na\n````\n~~~~~\n`````\n");
    let config = FmtConfig {
        code_formatters: commands(&[("rust", &["sh", FAKE, "no-newline"])]),
        ..FmtConfig::default()
    };
    assert_eq!(
        format(&Document::parse("```rust\na\n```\n"), &config),
        "```rust\nformatted\n```\n"
    );
}

/// A relative program path in a config file is relative to that file's directory, so a
/// formatter checked in next to `.mrq.yaml` runs from any directory; bare names are
/// looked up on `PATH` and arguments are passed as written.
#[cfg(unix)]
#[test]
fn relative_programs_resolve_against_their_config_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::create_dir_all(root.path().join("docs/deep")).unwrap();
    let script = tools.join("upper.sh");
    std::fs::write(&script, "#!/bin/sh\nprintf '%s\\n' \"$@\"\ntr a-z A-Z\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        root.path().join(".mrq.yaml"),
        "fmt:\n  code_formatters:\n    rust: [\"./tools/upper.sh\", \"./arg\"]\n    sh: [\"sh\", \"-c\", \"tr a-z A-Z\"]\n",
    )
    .unwrap();
    let loaded = mrq_core::config::ConfigLoader::new(root.path().join("docs/deep"))
        .load()
        .unwrap();
    let formatter = CommandFormatter::from_loaded(&loaded);
    assert_eq!(
        formatter.format("rust", "fn\n"),
        Some(Ok("./arg\nFN\n".to_owned()))
    );
    assert_eq!(
        formatter.format("sh", "echo\n"),
        Some(Ok("ECHO\n".to_owned()))
    );
}
