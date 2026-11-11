//! Structural checks on `.github/workflows/release.yml` (M13.1). The workflow itself only
//! runs on a tag push, so these tests pin down what can be checked offline: trigger, job
//! graph, target list, archive naming, the files that go into each archive, pinned
//! actions, and the `check-version` script (run against the real `Cargo.toml`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Workflow {
    on: On,
    permissions: BTreeMap<String, String>,
    jobs: BTreeMap<String, Job>,
}

#[derive(Debug, Deserialize)]
struct On {
    push: Option<Push>,
    pull_request: Option<serde_json::Value>,
    workflow_dispatch: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Push {
    tags: Option<Vec<String>>,
    branches: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn to_vec(&self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s.clone()],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
enum Scalar {
    Bool(bool),
    Str(String),
}

impl Scalar {
    fn as_str(&self) -> &str {
        match self {
            Scalar::Str(s) => s,
            Scalar::Bool(true) => "true",
            Scalar::Bool(false) => "false",
        }
    }
}

#[derive(Debug, Deserialize)]
struct Job {
    needs: Option<OneOrMany>,
    #[serde(rename = "runs-on")]
    runs_on: String,
    #[serde(rename = "continue-on-error")]
    continue_on_error: Option<Scalar>,
    strategy: Option<Strategy>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
struct Strategy {
    #[serde(rename = "fail-fast")]
    fail_fast: Option<bool>,
    matrix: Matrix,
}

#[derive(Debug, Deserialize)]
struct Matrix {
    include: Vec<BTreeMap<String, Scalar>>,
}

#[derive(Debug, Deserialize)]
struct Step {
    name: Option<String>,
    uses: Option<String>,
    run: Option<String>,
    shell: Option<String>,
    with: Option<BTreeMap<String, Scalar>>,
}

impl Step {
    fn uses_action(&self, action: &str) -> bool {
        self.uses
            .as_deref()
            .is_some_and(|u| u.split('@').next() == Some(action))
    }

    fn input(&self, key: &str) -> Option<&str> {
        self.with.as_ref()?.get(key).map(Scalar::as_str)
    }
}

const UPLOAD: &str = "taiki-e/upload-rust-binary-action";
const CREATE_RELEASE: &str = "taiki-e/create-gh-release-action";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

fn workflow_text() -> String {
    let path = repo_root().join(".github/workflows/release.yml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn workflow() -> Workflow {
    serde_saphyr::from_str(&workflow_text()).expect("release.yml parses")
}

fn needs(job: &Job) -> BTreeSet<String> {
    job.needs
        .as_ref()
        .map(OneOrMany::to_vec)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

/// The single job that builds and uploads binaries.
fn build_job(wf: &Workflow) -> (&String, &Job) {
    let builds: Vec<_> = wf
        .jobs
        .iter()
        .filter(|(_, job)| job.steps.iter().any(|s| s.uses_action(UPLOAD)))
        .collect();
    assert_eq!(builds.len(), 1, "exactly one build job expected");
    builds[0]
}

fn upload_step(job: &Job) -> &Step {
    job.steps.iter().find(|s| s.uses_action(UPLOAD)).unwrap()
}

/// The default `out` directory of a `Justfile` recipe such as `man out="target/man":`.
fn justfile_default_out(recipe: &str) -> String {
    let justfile = std::fs::read_to_string(repo_root().join("Justfile")).unwrap();
    let prefix = format!("{recipe} out=\"");
    let line = justfile
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no `{recipe} out=…` recipe in the Justfile"));
    line[prefix.len()..].split('"').next().unwrap().to_owned()
}

#[test]
fn triggers_on_version_tags_only() {
    let wf = workflow();
    let push = wf.on.push.expect("push trigger");
    assert_eq!(push.tags.as_deref(), Some(&["v*".to_owned()][..]));
    assert!(push.branches.is_none());
    assert!(wf.on.pull_request.is_none());
    assert!(wf.on.workflow_dispatch.is_none());
    assert_eq!(
        wf.permissions.get("contents").map(String::as_str),
        Some("write")
    );
}

#[test]
fn release_is_created_once_after_the_version_check_and_before_the_builds() {
    let wf = workflow();
    let check = wf.jobs.get("check-version").expect("check-version job");
    assert!(needs(check).is_empty());

    let create = wf.jobs.get("create-release").expect("create-release job");
    assert_eq!(needs(create), BTreeSet::from(["check-version".to_owned()]));
    let creators: Vec<_> = wf
        .jobs
        .values()
        .flat_map(|j| &j.steps)
        .filter(|s| s.uses_action(CREATE_RELEASE))
        .collect();
    assert_eq!(creators.len(), 1, "the release is created exactly once");
    assert!(create.steps.iter().any(|s| s.uses_action(CREATE_RELEASE)));
    // A prerelease when the tag has a `-`: the action decides that from the tag, so
    // nothing may force `draft`/`prerelease` off.
    assert_ne!(creators[0].input("draft"), Some("true"));

    let (_, build) = build_job(&wf);
    assert!(needs(build).contains("create-release"));
    assert!(needs(build).contains("check-version"));
}

#[test]
fn builds_every_release_target_with_windows_experimental() {
    let wf = workflow();
    let (_, job) = build_job(&wf);
    let strategy = job.strategy.as_ref().expect("build matrix");
    assert_eq!(strategy.fail_fast, Some(false));
    assert_eq!(
        job.runs_on, "${{ matrix.os }}",
        "each target picks its runner"
    );

    let mut targets = BTreeMap::new();
    for entry in &strategy.matrix.include {
        let target = entry["target"].as_str().to_owned();
        let os = entry["os"].as_str().to_owned();
        let experimental = entry.get("experimental") == Some(&Scalar::Bool(true));
        targets.insert(target, (os, experimental));
    }
    let expected = BTreeMap::from([
        ("x86_64-unknown-linux-musl", ("ubuntu-latest", false)),
        ("aarch64-unknown-linux-musl", ("ubuntu-latest", false)),
        ("x86_64-apple-darwin", ("macos-latest", false)),
        ("aarch64-apple-darwin", ("macos-latest", false)),
        ("x86_64-pc-windows-msvc", ("windows-latest", true)),
    ]);
    let actual: BTreeMap<_, _> = targets
        .iter()
        .map(|(t, (os, exp))| (t.as_str(), (os.as_str(), *exp)))
        .collect();
    assert_eq!(actual, expected);

    // Only the experimental (Windows) entry may fail without failing the run.
    assert_eq!(
        job.continue_on_error.as_ref().map(Scalar::as_str),
        Some("${{ matrix.experimental || false }}")
    );
    assert_eq!(
        upload_step(job).input("target"),
        Some("${{ matrix.target }}")
    );
}

#[test]
fn archives_are_named_by_tag_and_target_with_a_checksum() {
    let wf = workflow();
    let (_, job) = build_job(&wf);
    let upload = upload_step(job);
    assert_eq!(upload.input("bin"), Some("mrq"));
    assert_eq!(upload.input("archive"), Some("mrq-$tag-$target"));
    assert_eq!(upload.input("checksum"), Some("sha256"));
    assert_eq!(upload.input("token"), Some("${{ secrets.GITHUB_TOKEN }}"));
    assert_eq!(upload.input("locked"), Some("true"));
    // Defaults give `.tar.gz` on unix and `.zip` on Windows; don't override them.
    assert!(upload.input("tar").is_none() && upload.input("zip").is_none());
}

#[test]
fn archives_include_docs_licences_man_pages_and_completions() {
    let wf = workflow();
    let (_, job) = build_job(&wf);
    let upload = upload_step(job);
    let include: BTreeSet<_> = upload
        .input("include")
        .expect("include input")
        .split([',', ' ', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();

    let man_dir = justfile_default_out("man");
    let completions_dir = justfile_default_out("completions");
    let expected: BTreeSet<_> = [
        "README.md",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        &man_dir,
        &completions_dir,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(include, expected);
    for file in ["README.md", "LICENSE-MIT", "LICENSE-APACHE"] {
        assert!(repo_root().join(file).is_file(), "{file} missing");
    }

    // The man pages and completions are generated before the upload, by the `just`
    // recipes whose default output directories the include list names.
    let upload_index = job
        .steps
        .iter()
        .position(|s| s.uses_action(UPLOAD))
        .unwrap();
    let before = &job.steps[..upload_index];
    let runs: Vec<&str> = before.iter().filter_map(|s| s.run.as_deref()).collect();
    let script = runs.join("\n");
    let commands: Vec<&str> = script.lines().map(str::trim).collect();
    assert!(commands.contains(&"just man"), "{script}");
    assert!(commands.contains(&"just completions"), "{script}");
    assert!(
        before
            .iter()
            .any(|s| s.uses_action("taiki-e/install-action") && s.input("tool") == Some("just")),
        "just must be installed before it is used"
    );
}

#[test]
fn man_and_completion_directories_hold_what_the_recipes_produce() {
    // Run the same xtask commands the recipes run, into a scratch directory.
    let dir = tempfile::tempdir().unwrap();
    let xtask = env!("CARGO_BIN_EXE_xtask");
    for (task, sub) in [("man", "man"), ("completions", "completions")] {
        let output = Command::new(xtask)
            .args([task, "--out"])
            .arg(dir.path().join(sub))
            .output()
            .unwrap();
        assert!(output.status.success(), "xtask {task} failed: {output:?}");
    }
    let names = |sub: &str| -> BTreeSet<String> {
        std::fs::read_dir(dir.path().join(sub))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect()
    };
    let man = names("man");
    assert!(man.contains("mrq.1"), "{man:?}");
    assert!(
        man.iter()
            .all(|n| n.starts_with("mrq") && n.ends_with(".1")),
        "{man:?}"
    );
    let completions = names("completions");
    let expected: BTreeSet<String> = [
        "_mrq", "_mrq.ps1", "mrq.bash", "mrq.elv", "mrq.fish", "mrq.nu", "mrq.yaml",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(completions, expected);

    // The archive keeps each directory under its base name.
    assert!(justfile_default_out("man").ends_with("/man"));
    assert!(justfile_default_out("completions").ends_with("/completions"));
}

#[test]
fn run_steps_use_bash_on_every_runner() {
    let wf = workflow();
    for (name, job) in &wf.jobs {
        for step in &job.steps {
            if step.run.is_some() {
                assert_eq!(
                    step.shell.as_deref(),
                    Some("bash"),
                    "{name}: step {:?} needs `shell: bash`",
                    step.name
                );
            }
        }
    }
}

#[test]
fn every_action_is_pinned_to_a_commit_sha_with_a_version_comment() {
    let text = workflow_text();
    let mut seen = 0;
    for line in text.lines() {
        let Some(rest) = line
            .trim_start()
            .trim_start_matches("- ")
            .strip_prefix("uses:")
        else {
            continue;
        };
        seen += 1;
        let (reference, comment) = rest.split_once('#').unwrap_or((rest, ""));
        let (_, sha) = reference.trim().split_once('@').expect("uses has @ref");
        assert!(
            sha.len() == 40
                && sha
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "not pinned to a commit sha: {line}"
        );
        let version = comment.trim();
        assert!(
            version.starts_with('v')
                && version[1..].split('.').count() == 3
                && version[1..].split('.').all(|p| p.parse::<u32>().is_ok()),
            "missing `# vX.Y.Z` comment: {line}"
        );
    }
    assert!(seen > 0);
}

/// Runs the `check-version` job's script with `GITHUB_REF_NAME=tag` in the repo root.
#[cfg(unix)]
fn check_version(tag: &str) -> bool {
    let wf = workflow();
    let job = &wf.jobs["check-version"];
    let script = job
        .steps
        .iter()
        .find_map(|s| s.run.as_deref())
        .expect("check-version has a run step");
    let output = Command::new("bash")
        .args(["-eo", "pipefail", "-c", script])
        .current_dir(repo_root())
        .env("GITHUB_REF_NAME", tag)
        .output()
        .unwrap();
    output.status.success()
}

#[cfg(unix)]
#[test]
fn check_version_accepts_only_the_workspace_version() {
    let version = env!("CARGO_PKG_VERSION");
    assert!(check_version(&format!("v{version}")));
    assert!(!check_version(version), "tag without the v prefix");
    assert!(!check_version(&format!("v{version}-rc.1")));
    assert!(!check_version("v999.0.0"));
    assert!(!check_version(&format!("v{version}.1")));
}
