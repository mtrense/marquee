# Testing

`just check` runs formatting, clippy and every test with all features on. Most behaviour is covered by unit tests, snapshot tests (`insta`) and CLI tests (`assert_cmd`) under `crates/*/tests/`. A few things can only be checked by hand; they are listed here.

## Lint parity with markdownlint

Each markdownlint rule has fixtures in `crates/mrq-core/tests/fixtures/lint/<rule>/`: `pass*.md` must be clean, `fail*.md` must report something, and rule options come from `<!-- markdownlint-configure-file … -->` comments, which both tools read. Next to each fixture, `<name>.markdownlint.txt` records what markdownlint-cli2 reports with only that rule on (`line[:column] RULE message`) and `<name>.markdownlint.fixed` what its `--fix` writes (absent when nothing changes). `crates/mrq-core/tests/lint_rules.rs` compares mrq with these recordings offline, and snapshots mrq's findings and fixed text.

`just lint-parity` (`cargo xtask lint-parity`) re-records them with `npx --yes markdownlint-cli2@0.24.0`, then runs that test; it needs Node.js and network access for npx. `--check` only reports stale recordings, and `--command` runs another markdownlint-cli2 (e.g. a local `node_modules/.bin/markdownlint-cli2`). Inside a sandbox where `~/.npm` is read-only, set `npm_config_cache` to a writable directory.

## Manual checks

### SARIF from `mrq validate` is accepted by GitHub code scanning

The SARIF emitter is validated against the official SARIF 2.1.0 schema in `crates/mrq-core/tests/diag_emitters.rs`, and `crates/mrq-cli/tests/validate.rs` snapshots a full `mrq validate --format sarif` log. Whether GitHub's `github/codeql-action/upload-sarif` accepts it (its checks are stricter than the schema, e.g. about artifact URIs) can only be checked on GitHub:

1. In a scratch repository with code scanning available (public, or private with GitHub Advanced Security), add a Markdown file with a few problems, e.g. `docs/broken.md` containing `[gone](missing.md) [nowhere](#nowhere)`.
2. Add a workflow that builds or installs `mrq` and uploads the log, with the actions pinned to commit SHAs as in `.github/workflows/ci.yml`:

   ```yaml
   name: validate
   on: [push]
   permissions:
     contents: read
     security-events: write
   jobs:
     validate:
       runs-on: ubuntu-latest
       steps:
         - uses: actions/checkout@<sha>
         - run: cargo install --git https://github.com/mtrense/marquee mrq-cli --locked
         - run: mrq validate --format sarif . > results.sarif || test $? -eq 1
         - uses: github/codeql-action/upload-sarif@<sha>
           with:
             sarif_file: results.sarif
             category: mrq-validate
   ```

   `|| test $? -eq 1` keeps the job going when findings make `mrq` exit with 1, but still fails on usage or I/O errors (exit 2).
3. Push and check that:
   - the `upload-sarif` step succeeds without warnings about the file (rejected logs fail the step and name the offending property);
   - the repository's *Security → Code scanning* page lists one alert per finding, under the tool `mrq`, with the rule id (`missing-file`, `missing-anchor`, …) and the message;
   - each alert links to the right file, line and highlighted columns of `docs/broken.md`.
4. Fix the file, push again, and check that the alerts are closed.

Paths in the log are relative to the current directory (`docs/broken.md`), which GitHub resolves against the repository root, so run `mrq validate` from the checkout's root.
