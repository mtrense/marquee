# Testing

`just check` runs formatting, clippy and every test with all features on. Most behaviour is covered by unit tests, snapshot tests (`insta`) and CLI tests (`assert_cmd`) under `crates/*/tests/`. A few things can only be checked by hand; they are listed here.

## Lint parity with markdownlint

Each markdownlint rule has fixtures in `crates/mrq-core/tests/fixtures/lint/<rule>/`: `pass*.md` must be clean, `fail*.md` must report something, and rule options come from `<!-- markdownlint-configure-file … -->` comments, which both tools read. Next to each fixture, `<name>.markdownlint.txt` records what markdownlint-cli2 reports with only that rule on (`line[:column] RULE message`) and `<name>.markdownlint.fixed` what its `--fix` writes (absent when nothing changes). `crates/mrq-core/tests/lint_rules.rs` compares mrq with these recordings offline, and snapshots mrq's findings and fixed text. Fixtures are loaded from their path, so links and includes resolve against helper files in subdirectories (which are not fixtures themselves). mrq's own rules (`MRQ###`, documented in [rules/](rules/README.md)) have fixtures in the same layout but no recordings: markdownlint has no counterpart, so the test requires recordings for `MD###` fixtures only, `cargo xtask lint-parity` skips `mrq###` directories, and their snapshots are the reference.

MD052 and MD053 are recorded with MD043 on as well: markdownlint only parses the document when a rule needs its tokens, and these two read them without saying so, so on their own they never report anything (MD043 does nothing without `headings`, and only the rule's own findings are kept).

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

### Man pages from `mrq render -o man` read well in `man`

`crates/mrq-render/tests/man.rs` snapshots the roff of the fixtures in `crates/mrq-render/tests/fixtures/man/` (`mrq.1.md` for GFM, `mkdocs.7.md` for mkdocs syntax and frontmatter). How a pager shows them can only be judged by eye:

1. Write the roff of a fixture to a file, e.g. by stripping the insta header from its snapshot: `awk 'n>=2; /^---$/{n++}' crates/mrq-render/tests/snapshots/man__gfm_fixture.snap > /tmp/mrq.1` (or, once `mrq render` exists, `mrq render -o man crates/mrq-render/tests/fixtures/man/mrq.1.md > /tmp/mrq.1`).
2. Open it with `man -l /tmp/mrq.1` (man-db, Linux; it runs `tbl` because of the `'\" t` first line), `man /tmp/mrq.1` (macOS, mandoc) or `mandoc -T utf8 /tmp/mrq.1 | less -R`. Check that:
   - the header and footer show `MRQ(1)`, and `NAME` reads `mrq - a Markdown toolkit for the terminal`;
   - `SYNOPSIS` keeps one command per line, with commands bold and placeholders italic or underlined;
   - no line starts with a stray `.` request or shows a literal `\fB`, dashes and backslashes print as written, apostrophes as `'`;
   - lists, nested lists and definition lists are indented under their bullets, numbers and terms; code blocks, quotes and alerts are indented, with bold alert titles;
   - the table has a bold header with a rule under it, and long cells wrap within their column;
   - footnotes are listed under `NOTES` in the order of their references.
3. `mandoc -T lint -W warning /tmp/mrq.1` reports nothing but a missing date (the fixture has none). `cargo test -p mrq-render --test man -- --ignored` runs this lint for both fixtures, with a date.
