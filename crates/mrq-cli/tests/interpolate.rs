//! `mrq interpolate`: expansion to stdout, `--keep-directives`, `--in-place`, `--check`
//! and stdin.
//!
//! Every test runs `mrq` in a fresh temp directory with `MRQ_CONFIG_HOME` pointing at an
//! empty one, so no user config leaks in.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;

const CHAPTER: &str = "---\ntitle: Install\n---\n# Install\n\n![shot](img/shot.png)\n";
const BOOK: &str = "# Book\n\n<!-- mrq:include path=\"chapters/install.md\" shift=1 -->\n<!-- mrq:end -->\n\nThe end.\n";
const EXPANDED: &str = "# Book\n\n## Install\n\n![shot](chapters/img/shot.png)\n\nThe end.\n";
const REFRESHED: &str = "# Book\n\n<!-- mrq:include path=\"chapters/install.md\" shift=1 -->\n\n## Install\n\n![shot](chapters/img/shot.png)\n\n<!-- mrq:end -->\n\nThe end.\n";

struct Mrq {
    user: tempfile::TempDir,
    dir: tempfile::TempDir,
}

impl Mrq {
    /// A directory with `chapters/install.md` and `book.md` including it.
    fn new() -> Self {
        let mrq = Self {
            user: tempfile::tempdir().unwrap(),
            dir: tempfile::tempdir().unwrap(),
        };
        mrq.write("chapters/install.md", CHAPTER);
        mrq.write("book.md", BOOK);
        mrq
    }

    fn write(&self, name: &str, text: &str) {
        let path = self.dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.dir.path().join(name)).unwrap()
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("mrq").unwrap();
        cmd.current_dir(self.dir.path())
            .env("MRQ_CONFIG_HOME", self.user.path())
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("GITHUB_ACTIONS")
            .args(args);
        cmd
    }

    fn entries(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

#[test]
fn stdout_gets_the_expansion_without_directives() {
    let mrq = Mrq::new();
    mrq.cmd(&["interpolate", "book.md"])
        .assert()
        .success()
        .stdout(EXPANDED)
        .stderr(predicate::str::is_empty());
    assert_eq!(mrq.read("book.md"), BOOK);
}

#[test]
fn keep_directives_prints_the_refreshed_regions() {
    let mrq = Mrq::new();
    mrq.cmd(&["interpolate", "--keep-directives", "book.md"])
        .assert()
        .success()
        .stdout(REFRESHED)
        .stderr(predicate::str::is_empty());
    assert_eq!(mrq.read("book.md"), BOOK);
}

#[test]
fn in_place_is_idempotent_and_check_passes_after_it() {
    let mrq = Mrq::new();
    mrq.cmd(&["interpolate", "--in-place", "book.md"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());
    assert_eq!(mrq.read("book.md"), REFRESHED);
    // No temp files are left behind.
    assert_eq!(mrq.entries(), ["book.md", "chapters"]);

    mrq.cmd(&["interpolate", "--in-place", "book.md"])
        .assert()
        .success();
    assert_eq!(mrq.read("book.md"), REFRESHED);
    mrq.cmd(&["interpolate", "--check", "book.md"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
    // The expansion is the same with the content in place.
    mrq.cmd(&["interpolate", "book.md"])
        .assert()
        .success()
        .stdout(EXPANDED);
}

#[test]
fn check_prints_a_diff_and_fails_on_stale_regions() {
    let mrq = Mrq::new();
    let assert = mrq
        .cmd(&["interpolate", "--check", "book.md"])
        .assert()
        .code(1)
        .stderr(predicate::str::is_empty());
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    insta::assert_snapshot!("check_diff", stdout);
    assert_eq!(mrq.read("book.md"), BOOK);

    // A bare start marker is stale too: --in-place would add its content and end marker.
    mrq.write("bare.md", "<!-- mrq:include path=chapters/install.md -->\n");
    mrq.cmd(&["interpolate", "--check", "bare.md"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("+<!-- mrq:end -->"));
}

#[test]
fn stdin_resolves_from_the_current_directory_and_is_never_written_back() {
    let mrq = Mrq::new();
    mrq.cmd(&["interpolate", "-"])
        .write_stdin(BOOK)
        .assert()
        .success()
        .stdout(EXPANDED);
    mrq.cmd(&["interpolate", "--in-place", "-"])
        .write_stdin(BOOK)
        .assert()
        .success()
        .stdout(REFRESHED);
    mrq.cmd(&["interpolate", "--check", "-"])
        .write_stdin(REFRESHED)
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn failed_includes_are_reported_left_alone_and_exit_2() {
    let mrq = Mrq::new();
    let host = "<!-- mrq:include path=gone.md -->\n<!-- mrq:end -->\n\n<!-- mrq:include path=chapters/install.md -->\n";
    mrq.write("host.md", host);
    mrq.cmd(&["interpolate", "--format", "json", "host.md"])
        .assert()
        .code(2)
        .stdout(
            "<!-- mrq:include path=gone.md -->\n<!-- mrq:end -->\n\n# Install\n\n![shot](chapters/img/shot.png)\n",
        )
        .stderr(predicate::str::contains("\"code\": \"missing-include\""));
    mrq.cmd(&["interpolate", "--in-place", "host.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "included file `gone.md` does not exist",
        ));
    assert_eq!(
        mrq.read("host.md"),
        "<!-- mrq:include path=gone.md -->\n<!-- mrq:end -->\n\n<!-- mrq:include path=chapters/install.md -->\n\n# Install\n\n![shot](chapters/img/shot.png)\n\n<!-- mrq:end -->\n"
    );
}

#[test]
fn several_inputs_are_printed_in_order() {
    let mrq = Mrq::new();
    mrq.write("other.md", "Other.\n");
    mrq.cmd(&["interpolate", "book.md", "other.md"])
        .assert()
        .success()
        .stdout(format!("{EXPANDED}Other.\n"));
}

#[test]
fn an_input_is_required_and_modes_conflict() {
    let mrq = Mrq::new();
    mrq.cmd(&["interpolate"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no input given"));
    for args in [
        &["interpolate", "--in-place", "--check", "book.md"][..],
        &["interpolate", "--keep-directives", "--in-place", "book.md"][..],
        &["interpolate", "--keep-directives", "--check", "book.md"][..],
    ] {
        mrq.cmd(args).assert().code(2);
    }
    assert_eq!(mrq.read("book.md"), BOOK);
}
