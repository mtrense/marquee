//! `mrq render` over `tests/fixtures/render`: a snapshot per format (PDF checked for
//! validity and size only), `--fragment`, `-s`, `--flavor` as the md target, `--width`,
//! multiple inputs concatenated or one per input with `--out-dir`, `--out`, stdin,
//! diagram warnings and the exit codes.
//!
//! Every test runs `mrq` in a temp directory holding a copy of the fixtures, with
//! `MRQ_CONFIG_HOME` and `MRQ_CACHE_HOME` pointing at empty temp directories, so no user
//! config or cache leaks in.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/render")
}

struct Mrq {
    user: tempfile::TempDir,
    cache: tempfile::TempDir,
    dir: tempfile::TempDir,
}

impl Mrq {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for entry in fs::read_dir(fixtures()).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
        }
        Self {
            user: tempfile::tempdir().unwrap(),
            cache: tempfile::tempdir().unwrap(),
            dir,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("mrq").unwrap();
        cmd.current_dir(self.dir.path())
            .env("MRQ_CONFIG_HOME", self.user.path())
            .env("MRQ_CACHE_HOME", self.cache.path())
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("GITHUB_ACTIONS")
            .env_remove("COLUMNS")
            .env_remove("COLORTERM")
            .arg("render")
            .args(args);
        cmd
    }

    fn output(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// Stdout of `mrq render`, asserting success and an empty stderr.
    fn render(&self, args: &[&str]) -> String {
        String::from_utf8(self.render_bytes(args)).unwrap()
    }

    fn render_bytes(&self, args: &[&str]) -> Vec<u8> {
        let out = self.output(args);
        assert_eq!(out.status.code(), Some(0), "mrq render {args:?}: {out:?}");
        assert!(out.stderr.is_empty(), "mrq render {args:?}: {out:?}");
        out.stdout
    }
}

/// `page` with the inline stylesheet replaced by `…`, asserting there is one.
fn without_stylesheet(page: &str) -> String {
    let (head, rest) = page.split_once("<style>\n").expect("a stylesheet");
    let (css, tail) = rest.split_once("</style>").expect("a closed stylesheet");
    assert!(css.contains("hl-"), "the code theme's rules are inlined");
    format!("{head}<style>…</style>{tail}")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// --- One snapshot per format --------------------------------------------------------------

#[test]
fn html_is_the_default_and_a_standalone_page() {
    let mrq = Mrq::new();
    let page = mrq.render(&["guide.md"]);
    assert_eq!(page, mrq.render(&["-o", "html", "guide.md"]));
    insta::assert_snapshot!("html_page", without_stylesheet(&page));
}

#[test]
fn html_fragment_is_the_body_only() {
    let mrq = Mrq::new();
    let fragment = mrq.render(&["-o", "html", "--fragment", "guide.md"]);
    assert!(!fragment.contains("<html") && !fragment.contains("<style"));
    insta::assert_snapshot!("html_fragment", fragment);
}

#[test]
fn style_changes_the_stylesheet() {
    let mrq = Mrq::new();
    let light = mrq.render(&["guide.md"]);
    let dark = mrq.render(&["-s", "dark", "guide.md"]);
    assert_ne!(light, dark);
    fs::write(mrq.path("mine.yaml"), "extends: dark\n").unwrap();
    assert_eq!(dark, mrq.render(&["--style", "./mine.yaml", "guide.md"]));
    fs::write(mrq.path(".mrq.yaml"), "render:\n  style: dark\n").unwrap();
    assert_eq!(
        dark,
        mrq.render(&["guide.md"]),
        "render.style from the config"
    );
    assert_eq!(light, mrq.render(&["-s", "light", "guide.md"]), "-s wins");
}

#[test]
fn ansi_at_a_given_width() {
    let mrq = Mrq::new();
    let text = mrq.render(&[
        "-o", "ansi", "--width", "60", "--color", "never", "guide.md",
    ]);
    assert!(!text.contains('\x1b'));
    insta::assert_snapshot!("ansi_60", text);
    let colored = mrq.render(&["-o", "ansi", "--color", "always", "guide.md"]);
    assert!(colored.contains("\x1b["), "{colored}");
}

#[test]
fn ansi_width_comes_from_columns_then_defaults_to_80() {
    let mrq = Mrq::new();
    let doc = format!("{}\n", "word ".repeat(30).trim_end());
    fs::write(mrq.path("long.md"), &doc).unwrap();
    let widest = |text: &str| text.lines().map(|l| l.chars().count()).max().unwrap();
    let default = mrq.render(&["-o", "ansi", "long.md"]);
    assert!(widest(&default) <= 80 && widest(&default) > 70, "{default}");
    let out = mrq
        .command(&["-o", "ansi", "long.md"])
        .env("COLUMNS", "40")
        .output()
        .unwrap();
    let narrow = String::from_utf8(out.stdout).unwrap();
    assert!(widest(&narrow) <= 40, "{narrow}");
}

#[test]
fn md_converts_to_the_flavor_given() {
    let mrq = Mrq::new();
    // The input flavor comes from the frontmatter, not from --flavor.
    let md = mrq.render(&["-o", "md", "--flavor", "commonmark", "note.md"]);
    assert!(!md.contains("[["), "{md}");
    insta::assert_snapshot!("md_commonmark", md);
    let same = mrq.render(&["-o", "md", "note.md"]);
    assert!(same.contains("[[guide#Setup|the setup]]"), "{same}");
}

#[test]
fn flavor_is_the_input_flavor_for_other_formats() {
    let mrq = Mrq::new();
    fs::write(mrq.path("wiki.md"), "[[page]]\n").unwrap();
    let gfm = mrq.render(&["-o", "html", "--fragment", "wiki.md"]);
    let obsidian = mrq.render(&[
        "-o",
        "html",
        "--fragment",
        "--flavor",
        "obsidian",
        "wiki.md",
    ]);
    assert!(!gfm.contains("wikilink") && obsidian.contains("wikilink"));
}

#[test]
fn json_is_the_ast() {
    let mrq = Mrq::new();
    let json = mrq.render(&["-o", "json", "other.md"]);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["mrq_ast"], 1);
    insta::assert_snapshot!("json", json);
}

#[test]
fn man_is_roff() {
    let mrq = Mrq::new();
    let roff = mrq.render(&["-o", "man", "tool.1.md"]);
    assert!(roff.contains(".TH"), "{roff}");
    let version = format!("\"mrq {}\"", env!("CARGO_PKG_VERSION"));
    assert!(roff.contains(&version), "{roff}");
    insta::assert_snapshot!("man", roff.replace(&version, "\"mrq VERSION\""));
}

#[cfg(feature = "pdf")]
fn assert_pdf(pdf: &[u8]) {
    assert!(pdf.starts_with(b"%PDF-"), "no PDF header");
    assert!(pdf.trim_ascii_end().ends_with(b"%%EOF"), "no %%EOF");
    assert!(
        (5_000..2_000_000).contains(&pdf.len()),
        "{} bytes",
        pdf.len()
    );
}

#[cfg(feature = "pdf")]
#[test]
fn pdf_is_a_valid_and_reproducible_file() {
    let mrq = Mrq::new();
    let pdf = mrq.render_bytes(&["-o", "pdf", "-s", "paper", "guide.md"]);
    assert_pdf(&pdf);
    assert_eq!(
        pdf,
        mrq.render_bytes(&["-o", "pdf", "-s", "paper", "guide.md"])
    );
    let both = mrq.render_bytes(&["-o", "pdf", "guide.md", "other.md"]);
    assert_pdf(&both);
    assert!(both.len() > pdf.len() / 2);
}

#[cfg(not(feature = "pdf"))]
#[test]
fn pdf_without_the_feature_is_a_usage_error() {
    let out = Mrq::new().output(&["-o", "pdf", "guide.md"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(stderr(&out).contains("`pdf` feature"), "{}", stderr(&out));
}

// --- Inputs and outputs ---------------------------------------------------------------

#[test]
fn several_inputs_are_concatenated() {
    let mrq = Mrq::new();
    let md = mrq.render(&["-o", "md", "guide.md", "other.md"]);
    insta::assert_snapshot!("concatenated_md", md);
    let html = mrq.render(&["-o", "html", "--fragment", "guide.md", "other.md"]);
    // Anchors are unique across the documents, and links between them follow.
    assert!(
        html.contains("id=\"setup\"") && html.contains("id=\"setup-1\""),
        "{html}"
    );
    assert!(html.contains("href=\"#setup-1\""), "{html}");
    let page = mrq.render(&["guide.md", "other.md"]);
    assert_eq!(page.matches("<html").count(), 1, "one page");
}

#[test]
fn several_inputs_as_json_are_a_list_of_files() {
    let mrq = Mrq::new();
    let json = mrq.render(&["-o", "json", "guide.md", "other.md"]);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let files = value.as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["file"], "guide.md");
    assert_eq!(files[1]["file"], "other.md");
    assert_eq!(files[1]["ast"]["mrq_ast"], 1);
}

#[test]
fn out_dir_writes_one_file_per_input() {
    let mrq = Mrq::new();
    let out = mrq.output(&[
        "-o",
        "html",
        "--out-dir",
        "site/html",
        "guide.md",
        "other.md",
    ]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(out.stdout, b"");
    assert_eq!(
        stderr(&out),
        "guide.md -> site/html/guide.html\nother.md -> site/html/other.html\n"
    );
    let guide = fs::read_to_string(mrq.path("site/html/guide.html")).unwrap();
    assert_eq!(guide, mrq.render(&["guide.md"]));
    assert!(mrq.path("site/html/other.html").is_file());
    mrq.render(&["-q", "-o", "man", "--out-dir", "man", "tool.1.md"]);
    assert!(
        mrq.path("man/tool.1").is_file(),
        "the man extension is the section's"
    );
    mrq.render(&["-q", "-o", "md", "--out-dir", "md", "note.md"]);
    assert!(mrq.path("md/note.md").is_file());
}

#[test]
fn out_dir_refuses_inputs_with_the_same_name() {
    let mrq = Mrq::new();
    fs::create_dir(mrq.path("sub")).unwrap();
    fs::copy(mrq.path("guide.md"), mrq.path("sub/guide.md")).unwrap();
    let out = mrq.output(&["--out-dir", "site", "guide.md", "sub/guide.md"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("guide.html"), "{}", stderr(&out));
    assert!(!mrq.path("site").exists());
}

#[test]
fn out_writes_a_file() {
    let mrq = Mrq::new();
    let out = mrq.output(&["-o", "md", "--out", "build/all.md", "guide.md", "other.md"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(out.stdout, b"");
    assert_eq!(
        stderr(&out),
        "guide.md -> build/all.md\nother.md -> build/all.md\n",
        "one line per input"
    );
    let written = fs::read_to_string(mrq.path("build/all.md")).unwrap();
    // Links are relative to the output file.
    assert!(written.contains("](../tool.1.md)"), "{written}");
    // `--quiet` leaves them out.
    mrq.render(&["-q", "-o", "md", "--out", "build/one.md", "guide.md"]);
}

#[test]
fn stdin_is_read_with_a_dash() {
    let mrq = Mrq::new();
    let out = mrq
        .command(&["-o", "html", "--fragment", "-"])
        .write_stdin("# Hi\n")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "<h1 id=\"hi\">Hi</h1>\n"
    );
    assert_eq!(stderr(&out), "", "nothing reported for stdout");
    let out = mrq
        .command(&["--out", "hi.html", "-"])
        .write_stdin("# Hi\n")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(stderr(&out), "<stdin> -> hi.html\n");
}

// --- Diagnostics and errors -------------------------------------------------------------

#[test]
fn failed_diagrams_are_warnings() {
    let mrq = Mrq::new();
    for format in ["html", "ansi"] {
        let out = mrq.output(&["--config", "diagrams.yaml", "-o", format, "diagram.md"]);
        assert_eq!(out.status.code(), Some(0), "{format}: {out:?}");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("o--o"),
            "{format}"
        );
        assert!(
            stderr(&out).contains("diagram"),
            "{format}: {}",
            stderr(&out)
        );
        assert!(
            stderr(&out).contains("diagram.md"),
            "{format}: {}",
            stderr(&out)
        );
    }
}

#[test]
fn errors_exit_2() {
    let mrq = Mrq::new();
    let cases: &[(&[&str], &str)] = &[
        (&["missing.md"], "missing.md"),
        (&["-s", "neon", "guide.md"], "neon"),
        (&["-s", "./none.yaml", "guide.md"], "none.yaml"),
        (&[], "no input"),
    ];
    for (args, message) in cases {
        let out = mrq.output(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        assert!(stderr(&out).contains(message), "{args:?}: {}", stderr(&out));
    }
}

#[test]
fn out_and_out_dir_conflict() {
    let out = Mrq::new().output(&["--out", "a.html", "--out-dir", "b", "guide.md"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn nothing_is_written_when_an_input_fails() {
    let mrq = Mrq::new();
    let out = mrq.output(&["--out", "all.html", "guide.md", "missing.md"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!Path::new(&mrq.path("all.html")).exists());
}

// --- --open ------------------------------------------------------------------------------

/// `mrq render --open …` with `$MRQ_OPENER` a script logging the files it is given and
/// `$TMPDIR` an empty directory: the output and the opened files.
#[cfg(unix)]
struct Opened {
    out: Output,
    files: Vec<PathBuf>,
    tmp: tempfile::TempDir,
}

#[cfg(unix)]
fn open(mrq: &Mrq, opener: Option<&str>, args: &[&str]) -> Opened {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let log = bin.path().join("opened.log");
    let script = bin.path().join("opener");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$MRQ_TEST_LOG\"\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let out = mrq
        .command(&[&["--open"], args].concat())
        .env("TMPDIR", tmp.path())
        .env("MRQ_TEST_LOG", &log)
        .env("MRQ_OPENER", opener.unwrap_or(script.to_str().unwrap()))
        .output()
        .unwrap();
    let files = fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(PathBuf::from)
        .collect();
    Opened { out, files, tmp }
}

#[cfg(unix)]
fn file_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

#[cfg(unix)]
#[test]
fn open_renders_to_a_temp_file_named_after_the_input() {
    let mrq = Mrq::new();
    let opened = open(&mrq, None, &["guide.md"]);
    assert_eq!(opened.out.status.code(), Some(0), "{:?}", opened.out);
    assert_eq!(opened.out.stdout, b"");
    let [file] = opened.files.as_slice() else {
        panic!("one file opened: {:?}", opened.files);
    };
    assert_eq!(
        stderr(&opened.out),
        format!("guide.md -> {}\n", file.display())
    );
    assert_eq!(
        file.parent().map(Path::to_path_buf),
        Some(opened.tmp.path().canonicalize().unwrap())
    );
    let name = file_name(file);
    assert!(
        name.starts_with("guide-") && file.extension().is_some_and(|ext| ext == "html"),
        "{name}"
    );
    let page = fs::read_to_string(file).unwrap();
    assert!(page.contains("<title>Guide</title>"));
    // Links are rebased from the temp directory back to the input's.
    let other = mrq.path("other.md");
    let rebased = page
        .split("href=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .find(|href| href.ends_with("other.md#setup"))
        .expect("the link to other.md");
    assert_ne!(rebased, "other.md#setup");
    let target = file.with_file_name(rebased.trim_end_matches("#setup"));
    assert_eq!(
        target.canonicalize().unwrap(),
        other.canonicalize().unwrap()
    );
}

#[cfg(unix)]
#[test]
fn open_keeps_the_input_name_as_the_title_fallback() {
    let mrq = Mrq::new();
    fs::write(mrq.path("plain.md"), "Just text.\n").unwrap();
    let opened = open(&mrq, None, &["plain.md"]);
    let page = fs::read_to_string(&opened.files[0]).unwrap();
    assert!(page.contains("<title>plain</title>"), "{page}");
}

#[cfg(unix)]
#[test]
fn open_names_the_temp_file_after_all_inputs_and_the_format() {
    let mrq = Mrq::new();
    let opened = open(&mrq, None, &["-o", "md", "guide.md", "other.md"]);
    assert_eq!(opened.out.status.code(), Some(0), "{:?}", opened.out);
    let name = file_name(&opened.files[0]);
    assert!(
        name.starts_with("guide+other-")
            && opened.files[0].extension().is_some_and(|ext| ext == "md"),
        "{name}"
    );
    let opened = open(&mrq, None, &["-o", "json", "guide.md"]);
    let json = fs::read_to_string(&opened.files[0]).unwrap();
    assert_eq!(
        json,
        mrq.render(&["-o", "json", "guide.md"]),
        "one AST, not a list"
    );
}

#[cfg(unix)]
#[test]
fn open_opens_out_and_out_dir_files_in_place() {
    let mrq = Mrq::new();
    let opened = open(&mrq, None, &["--out", "guide.html", "guide.md"]);
    assert_eq!(opened.out.status.code(), Some(0), "{:?}", opened.out);
    assert_eq!(opened.files, [PathBuf::from("guide.html")]);
    assert_eq!(
        fs::read_to_string(mrq.path("guide.html")).unwrap(),
        mrq.render(&["guide.md"])
    );
    assert_eq!(fs::read_dir(opened.tmp.path()).unwrap().count(), 0);

    let opened = open(&mrq, None, &["--out-dir", "site", "guide.md", "other.md"]);
    assert_eq!(
        opened.files,
        [
            PathBuf::from("site/guide.html"),
            PathBuf::from("site/other.html")
        ]
    );
    assert_eq!(fs::read_dir(opened.tmp.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn open_failures_exit_2() {
    let mrq = Mrq::new();
    let opened = open(&mrq, Some("false"), &["guide.md"]);
    assert_eq!(opened.out.status.code(), Some(2));
    assert!(
        stderr(&opened.out).contains("cannot open"),
        "{}",
        stderr(&opened.out)
    );
    assert_eq!(
        fs::read_dir(opened.tmp.path()).unwrap().count(),
        1,
        "the rendered file is kept"
    );

    let opened = open(&mrq, None, &["missing.md"]);
    assert_eq!(opened.out.status.code(), Some(2));
    assert_eq!(opened.files, Vec::<PathBuf>::new());
    assert_eq!(
        fs::read_dir(opened.tmp.path()).unwrap().count(),
        0,
        "the empty temp file is removed"
    );
}
