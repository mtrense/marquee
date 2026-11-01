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

### `mrq show` hands the terminal to the editor and reloads changed files

`crates/mrq-tui/tests/reload.rs` drives the reload state machine with a scripted editor and change events (`e`, `R`, changes with and without unsaved edits, deleted files, our own saves), and checks the notify watcher against the real file system (an atomic save is reported once; on macOS the test skips itself where a sandbox does not let FSEvents start). Leaving and restoring the terminal around a real editor, and the watcher inside the running viewer, can only be checked by hand:

1. Copy a few files to a scratch directory, e.g. `cp README.md PLAN.md /tmp/mrq-reload/`, and run `mrq show /tmp/mrq-reload/` (built with `cargo run --` or installed).
2. With `EDITOR=vi` (and `VISUAL` unset), select a heading far down and press `e`. Check that:
   - vi shows the file with the cursor on the selected heading's line, on the normal screen (not over the viewer);
   - after a change and `:wq`, the viewer comes back fully redrawn (no leftovers of vi), shows the change, says `reloaded …` and keeps the selection on the same heading;
   - `:q` without a change says `… is unchanged`, and `:cq` says `editor: \`vi\` exited with exit status: 1`;
   - `EDITOR='code --wait'` (or another editor with arguments) opens it, and the viewer waits until its tab is closed; `EDITOR=no-such-editor` says `editor: cannot run \`no-such-editor\`: …` and the viewer is usable afterwards. `VISUAL` wins over `EDITOR`.
3. Make an edit (`d`), then press `e`: the status line says to write or drop the edits first and no editor starts. `mrq show - < README.md` refuses `e` for `<stdin>`.
4. In another terminal, change the shown file with an editor that saves atomically (vim with `backupcopy=no`, or `mrq fmt`), and with one that writes in place (`echo >> file`). Check that each change shows up within a moment, once, with the selection kept where it was, and that a change to a file in the file list that is not shown is there when it is opened.
5. Press `w` after an edit (`d`): the file is written and no `reloaded` or `changed on disk` message follows.
6. Make an edit, then change the file in the other terminal. Check that the edit stays and the status line says `… changed on disk; your edits are kept: w writes them over it, R reloads it`. `R` shows the file on disk and `u` brings the edit back; alternatively `w` writes the edit over the change.
7. Delete the shown file in the other terminal (`rm`): the status line says it was deleted and its content is kept; `w` writes it back. Recreate it with other text and check that it reloads.
8. Follow a link to a file that was not open yet, change that file in the other terminal and check that it reloads too.
9. On Linux, repeat 4–7 once (inotify instead of FSEvents), and on a file reached through a symlinked directory.

### `mrq show` draws images and diagrams in kitty and WezTerm

`crates/mrq-tui/tests/images.rs` draws images and diagrams on a `TestBackend` with halfblocks and with faked kitty, iTerm2 and sixel pickers: the rows reserved for them, images partly scrolled off the top or bottom of the panel, the cache per size, the text fallbacks without a picker and for an image that does not decode, and the choice of protocol from the environment. Whether a real terminal answers the query and shows the escapes in the right cells can only be checked by hand:

1. In a scratch directory, put a PNG taller than the screen (`big.png`, e.g. a screenshot), a small PNG (`small.png`), an SVG (`logo.svg`) and a `doc.md` with text around them: a few paragraphs, `![big](big.png)`, a paragraph, `> ![small](small.png)`, `- ![logo](logo.svg)`, a ```` ```dot ```` block (`digraph { a -> b }`), a ```` ```mermaid ```` block if built with `mermaid`, a broken `![gone](nope.png)` and a few more paragraphs. Run `mrq show doc.md` (built with `cargo run --` or installed).
2. In kitty (and Ghostty, if at hand), check that:
   - every image and the DOT diagram show as pictures at their size or shrunk to the panel's width, never wider, with the right aspect ratio; `nope.png` shows `[image: gone]`;
   - the quoted image has the `│` bar on each of its rows, and the list item's image starts after its bullet;
   - `↓`/`↑` step over each image as one block (the gutter mark covers all its rows), `PgDn`/`PgUp` and `Home`/`End` scroll through `big.png` with the part above or below the panel cut off cleanly, nothing drawn over the border, the status line or the outline, and no leftovers when an image leaves the screen;
   - resizing the window (narrower than `big.png`, then wider again) re-lays the images out without leftovers, and scrolling back and forth stays smooth (each image is scaled once per size);
   - `?` covers the images with the help box, and closing it shows them again; so does `Tab` to the outline and back, and `o`;
   - `/` with a word after an image finds it on the right line, and `Enter` on an outline heading below `big.png` lands on it;
   - `e` and quitting the editor bring the images back.
3. In WezTerm, repeat 2: ratatui-image uses iTerm2's protocol there (its kitty support has no placeholders); check the same, in particular that partly scrolled images are cut at the panel.
4. Check the fallbacks: `MRQ_GRAPHICS=halfblocks mrq show doc.md` shows colored `▀` blocks in place of the images; `MRQ_GRAPHICS=none` and `--color never` show the alt texts and the DOT block as code; `NO_COLOR=1` keeps the pictures in kitty. Inside tmux, the images are half blocks. In Terminal.app (no protocol) they are half blocks, without a noticeable pause at start. `mrq show - < doc.md` does not query the terminal (its answer would come on stdin): it starts at once, shows the relative images' alt texts (stdin has no directory) and the DOT diagram as half blocks or, with `MRQ_GRAPHICS=kitty`, as a picture.
5. If at hand, check sixel in foot or `xterm -ti vt340`, with `MRQ_GRAPHICS=sixel` where the terminal is not detected, scrolling through `big.png` as in 2.

### `mrq lsp` works in Neovim and Helix as docs/editors.md sets it up

The tests in `crates/mrq-lsp/tests/` drive the server over in-memory pipes with a scripted client (`tests/support/mod.rs`). Whether real clients accept its answers, and whether the setup in [editors.md](editors.md) is right, can only be checked by hand:

1. Install the binary (`just install`) and make a scratch workspace: a `.mrq.yaml` with `fmt:\n  list_marker: "*"`, a `part.md` (`Part.`), and an `a.md` with a heading, a line with one trailing space, three blank lines, a `- one` / `- two` list, a link `[b](b.md#setup)`, an outdated region `<!-- mrq:include path="part.md" -->` / `old` / `<!-- mrq:end -->`, and a `b.md` with `## Setup`.
2. In Neovim 0.11+, with only the `vim.lsp.config` / `vim.lsp.enable` block and the optional autocommand and key from editors.md in an otherwise empty config (`nvim -u mini.lua a.md`), check that:
   - `:checkhealth vim.lsp` lists `mrq` attached to the buffer, with the workspace as its root;
   - diagnostics appear for the trailing space (MD009) and the blank lines (MD012), and the include region (MRQ004 if enabled);
   - `gra` on the trailing-space line (cursor anywhere on it) offers "Delete the trailing spaces" first, and applying it removes only the space; in the include region it offers "Refresh include region", which writes `Part.` between the markers; `<leader>mf` fixes everything at once, and `u` undoes each in one step;
   - typing a few characters and invoking `gra` at once still gives fixes that land where they should (no edits against an older version);
   - `:w` formats the buffer (`*` bullets, from `.mrq.yaml`) without moving the cursor off its line, and `:lua vim.lsp.buf.format()` on a formatted buffer changes nothing (`:undolist` shows no new entry);
   - `<C-]>` on the link jumps to `b.md`'s `## Setup`, `K` shows its preview, and `grn` on `## Setup` renames it and the link.
3. Repeat with the Neovim 0.10 `FileType` snippet, if a 0.10 is at hand.
4. In Helix, with the `languages.toml` from editors.md, check that:
   - `hx --health markdown` lists `mrq` as found;
   - the same diagnostics show in the gutter, and `space a` on the trailing space, in the include region and elsewhere offers the same actions as in Neovim, which apply as there;
   - `:w` with `auto-format = true` formats the document with `*` bullets, and `:format` does the same;
   - `gd`, `space k` and `space r` work as `<C-]>`, `K` and `grn` do in Neovim.
5. Check that formatting and the actions also work on a file outside the workspace (opened by its absolute path), with the default settings (`-` bullets).
