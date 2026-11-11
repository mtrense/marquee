//! Structural checks on `.github/workflows/release.yml` (M13.1, M13.2). The workflow
//! itself only runs on a tag push, so these tests pin down what can be checked offline:
//! trigger, job graph, target list, archive naming, the files that go into each archive,
//! pinned actions, the `check-version` script (run against the real `Cargo.toml`), and the
//! Homebrew tap jobs (probe, checksum fetch with a stubbed `curl`, formula rendering).

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
    #[serde(rename = "if")]
    condition: Option<String>,
    needs: Option<OneOrMany>,
    permissions: Option<BTreeMap<String, String>>,
    outputs: Option<BTreeMap<String, String>>,
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
    id: Option<String>,
    name: Option<String>,
    uses: Option<String>,
    run: Option<String>,
    shell: Option<String>,
    env: Option<BTreeMap<String, String>>,
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

// ---------------------------------------------------------------------------------------
// M13.2: Homebrew tap pull request
// ---------------------------------------------------------------------------------------

const CHECKOUT: &str = "actions/checkout";
const CREATE_PR: &str = "peter-evans/create-pull-request";
const TAP_TOKEN: &str = "${{ secrets.HOMEBREW_TAP_TOKEN }}";
const STABLE_ONLY: &str = "!contains(github.ref_name, '-')";

/// Targets the release must not fail on: every matrix entry that isn't experimental.
fn required_targets(wf: &Workflow) -> BTreeSet<String> {
    let (_, job) = build_job(wf);
    job.strategy
        .as_ref()
        .unwrap()
        .matrix
        .include
        .iter()
        .filter(|e| e.get("experimental") != Some(&Scalar::Bool(true)))
        .map(|e| e["target"].as_str().to_owned())
        .collect()
}

fn step_by_id<'a>(job: &'a Job, id: &str) -> &'a Step {
    job.steps
        .iter()
        .find(|s| s.id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no step with id `{id}`"))
}

/// Parses `key=value` lines written to `$GITHUB_OUTPUT`.
fn read_outputs(path: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

#[test]
fn tap_update_runs_after_the_builds_and_the_token_probe() {
    let wf = workflow();
    let probe = wf.jobs.get("check-secret").expect("check-secret job");
    let tap = wf.jobs.get("tap-update").expect("tap-update job");
    let (build_name, _) = build_job(&wf);
    assert_eq!(
        needs(tap),
        BTreeSet::from([build_name.clone(), "check-secret".to_owned()])
    );
    assert_eq!(tap.runs_on, "ubuntu-latest");
    assert_eq!(probe.runs_on, "ubuntu-latest");
    // The tap job only reads this repository; it writes to the tap with its own token.
    assert_eq!(
        tap.permissions
            .as_ref()
            .and_then(|p| p.get("contents"))
            .map(String::as_str),
        Some("read")
    );
    // Only the experimental (Windows) build may fail without blocking the tap job, and
    // it is part of the same matrix job, so `continue-on-error` must stay per entry.
    assert!(!wf.jobs.keys().any(|k| k.contains("windows")));
}

#[test]
fn tap_jobs_run_only_for_stable_tags_with_a_token() {
    let wf = workflow();
    let probe = &wf.jobs["check-secret"];
    let tap = &wf.jobs["tap-update"];
    let probe_if = probe.condition.as_deref().expect("check-secret has an if");
    assert!(probe_if.contains(STABLE_ONLY), "{probe_if}");
    let tap_if = tap.condition.as_deref().expect("tap-update has an if");
    assert!(tap_if.contains(STABLE_ONLY), "{tap_if}");
    assert!(
        tap_if.contains("needs.check-secret.outputs.has_token == 'true'"),
        "{tap_if}"
    );
    // `secrets` isn't available in a job-level `if:`.
    assert!(!tap_if.contains("secrets."), "{tap_if}");
    assert!(!probe_if.contains("secrets."), "{probe_if}");
}

#[cfg(unix)]
#[test]
fn token_probe_reports_whether_the_secret_is_set() {
    let wf = workflow();
    let probe = &wf.jobs["check-secret"];
    assert_eq!(
        probe
            .outputs
            .as_ref()
            .and_then(|o| o.get("has_token"))
            .map(String::as_str),
        Some("${{ steps.probe.outputs.has_token }}")
    );
    let step = step_by_id(probe, "probe");
    let env = step.env.as_ref().expect("probe step env");
    let (var, _) = env
        .iter()
        .find(|(_, v)| v.as_str() == TAP_TOKEN)
        .expect("the probe reads HOMEBREW_TAP_TOKEN through env");
    let script = step.run.as_deref().unwrap();
    assert!(!script.contains("${{"), "no expressions inside the script");

    for (value, expected) in [(Some("tok"), "true"), (Some(""), "false"), (None, "false")] {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let mut cmd = Command::new("bash");
        cmd.args(["-eo", "pipefail", "-c", script])
            .env("GITHUB_OUTPUT", &out)
            .env_remove(var);
        if let Some(v) = value {
            cmd.env(var, v);
        }
        let status = cmd.status().unwrap();
        assert!(status.success());
        assert_eq!(
            read_outputs(&out).get("has_token").map(String::as_str),
            Some(expected),
            "token {value:?}"
        );
    }
}

/// Runs the `checksums` step of `tap-update` with a stub `curl` that serves files from
/// `assets` (by base name) and logs the requested URLs. Returns (success, outputs, urls).
#[cfg(unix)]
fn fetch_checksums(
    tag: &str,
    assets: &BTreeMap<String, String>,
) -> (bool, BTreeMap<String, String>, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let wf = workflow();
    let step = step_by_id(&wf.jobs["tap-update"], "checksums");
    let script = step.run.as_deref().expect("checksums step runs a script");
    assert!(!script.contains("${{"), "no expressions inside the script");

    let dir = tempfile::tempdir().unwrap();
    let fixtures = dir.path().join("assets");
    std::fs::create_dir(&fixtures).unwrap();
    for (name, content) in assets {
        std::fs::write(fixtures.join(name), content).unwrap();
    }
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let curl = bin.join("curl");
    std::fs::write(
        &curl,
        "#!/usr/bin/env bash\nurl=\"${@: -1}\"\necho \"$url\" >> \"$CURL_LOG\"\nf=\"$FIXTURES/${url##*/}\"\n[ -f \"$f\" ] || exit 22\ncat \"$f\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = dir.path().join("out");
    let log = dir.path().join("curl.log");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let status = Command::new("bash")
        .args(["-eo", "pipefail", "-c", script])
        .current_dir(dir.path())
        .env("PATH", path)
        .env("FIXTURES", &fixtures)
        .env("CURL_LOG", &log)
        .env("GITHUB_OUTPUT", &out)
        .env("GITHUB_REF_NAME", tag)
        .env("GITHUB_SERVER_URL", "https://github.com")
        .env("GITHUB_REPOSITORY", "mtrense/marquee")
        .envs(step.env.iter().flatten().map(|(k, v)| {
            assert_eq!(v, "${{ github.ref_name }}", "unexpected env {k}");
            (k, tag)
        }))
        .status()
        .unwrap();
    let urls = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    (status.success(), read_outputs(&out), urls)
}

#[cfg(unix)]
fn sample_assets(tag: &str, targets: &BTreeSet<String>) -> BTreeMap<String, String> {
    targets
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let sha = format!("{i}").repeat(64);
            (
                format!("mrq-{tag}-{t}.sha256"),
                format!("{sha}  mrq-{tag}-{t}.tar.gz\n"),
            )
        })
        .collect()
}

#[cfg(unix)]
#[test]
fn checksums_are_fetched_for_the_four_required_targets_only() {
    let wf = workflow();
    let targets = required_targets(&wf);
    assert_eq!(targets.len(), 4, "{targets:?}");
    assert!(targets.iter().all(|t| !t.contains("windows")));
    let tag = "v0.1.0";
    let assets = sample_assets(tag, &targets);
    let (ok, outputs, urls) = fetch_checksums(tag, &assets);
    assert!(ok, "{outputs:?} {urls:?}");

    let expected_urls: BTreeSet<String> = targets
        .iter()
        .map(|t| {
            format!(
                "https://github.com/mtrense/marquee/releases/download/{tag}/mrq-{tag}-{t}.sha256"
            )
        })
        .collect();
    assert_eq!(urls.iter().cloned().collect::<BTreeSet<_>>(), expected_urls);

    // One output per target, holding the bare hex digest.
    let shas: BTreeSet<&str> = outputs.values().map(String::as_str).collect();
    let expected: BTreeSet<String> = (0..4).map(|i| format!("{i}").repeat(64)).collect();
    assert_eq!(
        shas,
        expected.iter().map(String::as_str).collect::<BTreeSet<_>>()
    );
}

#[cfg(unix)]
#[test]
fn checksum_fetch_fails_on_a_missing_empty_or_malformed_checksum() {
    let wf = workflow();
    let targets = required_targets(&wf);
    let tag = "v0.1.0";
    for target in &targets {
        let name = format!("mrq-{tag}-{target}.sha256");
        for broken in [None, Some(""), Some("\n"), Some("not-a-sha  x.tar.gz\n")] {
            let mut assets = sample_assets(tag, &targets);
            match broken {
                None => {
                    assets.remove(&name);
                }
                Some(content) => {
                    assets.insert(name.clone(), content.to_owned());
                }
            }
            let (ok, outputs, _) = fetch_checksums(tag, &assets);
            assert!(!ok, "{target} with {broken:?} must fail: {outputs:?}");
        }
    }
}

#[test]
fn tap_update_checks_out_the_tap_and_this_repository() {
    let wf = workflow();
    let tap = &wf.jobs["tap-update"];
    let checkouts: Vec<&Step> = tap
        .steps
        .iter()
        .filter(|s| s.uses_action(CHECKOUT))
        .collect();
    let tap_checkout = checkouts
        .iter()
        .find(|s| s.input("repository") == Some("mtrense/homebrew-tap"))
        .expect("checkout of mtrense/homebrew-tap");
    assert_eq!(tap_checkout.input("token"), Some(TAP_TOKEN));
    assert_eq!(tap_checkout.input("path"), Some("homebrew-tap"));
    let own = checkouts
        .iter()
        .find(|s| s.input("repository").is_none())
        .expect("checkout of this repository (for the render script)");
    assert_eq!(own.input("path"), Some("marquee"));
    assert_eq!(own.input("persist-credentials"), Some("false"));
}

#[cfg(unix)]
#[test]
fn render_step_writes_the_formula_into_the_tap_checkout() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 not found; skipping");
        return;
    }
    let wf = workflow();
    let tap = &wf.jobs["tap-update"];
    let step = step_by_id(tap, "render");
    let script = step.run.as_deref().unwrap();
    assert!(!script.contains("${{"), "no expressions inside the script");
    assert!(
        script.contains("marquee/.github/scripts/render_formula.py"),
        "{script}"
    );

    // Feed the render step with what the checksum step produces.
    let tag = "v0.1.0";
    let targets = required_targets(&wf);
    let (ok, outputs, _) = fetch_checksums(tag, &sample_assets(tag, &targets));
    assert!(ok);
    let env = step.env.as_ref().expect("render step env");
    let resolved: BTreeMap<&String, String> = env
        .iter()
        .map(|(k, v)| {
            let value = if v == "${{ github.ref_name }}" {
                tag.to_owned()
            } else if let Some(key) = v
                .strip_prefix("${{ steps.checksums.outputs.")
                .and_then(|r| r.strip_suffix(" }}"))
            {
                outputs
                    .get(key)
                    .unwrap_or_else(|| panic!("checksums step has no output `{key}`"))
                    .clone()
            } else {
                v.clone()
            };
            (k, value)
        })
        .collect();

    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(repo_root(), dir.path().join("marquee")).unwrap();
    std::fs::create_dir(dir.path().join("homebrew-tap")).unwrap();
    let output = Command::new("bash")
        .args(["-eo", "pipefail", "-c", script])
        .current_dir(dir.path())
        .envs(resolved)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let formula = std::fs::read_to_string(dir.path().join("homebrew-tap/Formula/mrq.rb"))
        .expect("Formula/mrq.rb written");
    assert!(formula.contains("class Mrq < Formula"), "{formula}");
    for t in &targets {
        assert!(
            formula.contains(&format!("/{tag}/mrq-{tag}-{t}.tar.gz\"")),
            "{t}"
        );
    }
    for sha in outputs.values() {
        assert!(formula.contains(&format!("sha256 \"{sha}\"")), "{sha}");
    }
}

#[test]
fn tap_pull_request_is_opened_on_a_per_tag_branch() {
    let wf = workflow();
    let tap = &wf.jobs["tap-update"];
    let prs: Vec<&Step> = tap
        .steps
        .iter()
        .filter(|s| s.uses_action(CREATE_PR))
        .collect();
    assert_eq!(prs.len(), 1);
    let pr = prs[0];
    assert_eq!(pr.input("token"), Some(TAP_TOKEN));
    assert_eq!(pr.input("path"), Some("homebrew-tap"));
    assert_eq!(
        pr.input("branch"),
        Some("update-formula-${{ github.ref_name }}")
    );
    assert_eq!(pr.input("add-paths"), Some("Formula/mrq.rb"));
    // The PR comes last, after the formula is rendered.
    let render = tap
        .steps
        .iter()
        .position(|s| s.id.as_deref() == Some("render"))
        .unwrap();
    let pr_index = tap
        .steps
        .iter()
        .position(|s| s.uses_action(CREATE_PR))
        .unwrap();
    assert!(render < pr_index);
    // Nothing commits to the tap directly.
    for step in &tap.steps {
        if let Some(run) = &step.run {
            assert!(
                !run.contains("git push") && !run.contains("git commit"),
                "{run}"
            );
        }
    }
}

#[test]
fn no_run_step_interpolates_expressions() {
    // Values reach scripts through `env:`, never by `${{ }}` text substitution.
    let wf = workflow();
    for (name, job) in &wf.jobs {
        for step in &job.steps {
            if let Some(run) = &step.run {
                assert!(!run.contains("${{"), "{name}: {:?}", step.name);
            }
        }
    }
}

#[test]
fn render_formula_python_tests_pass() {
    let python = Command::new("python3").arg("--version").output();
    if python.is_err() {
        eprintln!("python3 not found; skipping");
        return;
    }
    let scripts = repo_root().join(".github/scripts");
    assert!(
        scripts.join("test_render_formula.py").is_file(),
        "missing .github/scripts/test_render_formula.py"
    );
    let output = Command::new("python3")
        .args(["-I", "-B", "-m", "unittest", "discover", "-s"])
        .arg(&scripts)
        .args(["-p", "test_*.py", "-v"])
        .current_dir(repo_root())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
