# mrq

**mrq** is a Markdown toolkit for the terminal. It verifies, lints, formats, queries, transforms, renders and displays Markdown, from the command line, in an interactive TUI, or inside your editor through its language server. The engine behind it, `mrq-core`, is a standalone Rust library you can embed in your own tools.

```sh
mrq toc README.md                       # table of contents
mrq validate docs/                      # broken links, anchors, assets, footnotes, frontmatter schema
mrq lint --fix docs/                    # markdownlint-compatible style rules, auto-fixed
mrq fmt --check .                       # canonical formatting, CI-friendly
mrq query '# Install | ```sh' README.md # the shell snippets under "Install"
mrq render -o pdf -s paper book.md > book.pdf
mrq show docs/                          # browse in the TUI
```

## Features

- **One engine, many commands.** Every command shares one parser, one document model and one diagnostics system.
- **Pluggable flavors.** `gfm` (default), `commonmark`, `obsidian` and `mkdocs` ship built in. New flavors are a small, self-contained addition.
- **Frontmatter aware.** YAML and TOML frontmatter are parsed, preserved, merged and validated against JSON Schema.
- **Structure-aware transforms.** Merge, split and include files, with heading levels shifted and relative links and assets rewritten on the way.
- **Three query syntaxes over one engine.** Markdown-native selectors, CSS-style selectors, and full jq.
- **Rich rendering.** HTML, terminal (ANSI), PDF, Markdown, JSON AST and man pages, with syntax highlighting, math and diagrams (Mermaid, Graphviz, D2, PlantUML, …).
- **Interactive TUI.** Outline sidebar, search, queries, link following and structural editing.
- **Language server.** Diagnostics, go-to-heading, link completion, and heading renames that update every anchor.
- **CI-ready output.** Human-readable diagnostics plus JSON/NDJSON, SARIF and GitHub annotation formats.

## Installation

```sh
# Homebrew
brew install mtrense/tap/mrq

# Nix
nix profile install github:mtrense/mrq

# Arch Linux (AUR)
paru -S mrq

# From source / crates.io (the crate is called mrq-cli; the binary is mrq)
cargo install mrq-cli
```

Prebuilt binaries for Linux, macOS and Windows are attached to every GitHub release.

### Cargo features

Release binaries enable everything. A minimal build is available with `cargo install mrq-cli --no-default-features`.

| Feature          | Enables                                                           |
| ---------------- | ----------------------------------------------------------------- |
| `pdf`            | `render -o pdf` (embedded Typst)                                  |
| `external-links` | checking http(s) links in `validate --external` (default; no request without `--external`) |
| `remote-include` | `mrq:include` from http(s) URLs and `sha256` pins (default; nothing fetched without `include.allow_remote`) |
| `tree-sitter`    | structural code includes (`function="…"`) and tree-sitter grammars |
| `mermaid`        | built-in Mermaid rendering (no Node.js needed)                    |
| `kroki`          | diagrams from a Kroki server in `render` (default; never contacted without `render.diagrams.kroki.enabled`) |
| `tui`            | `mrq show` (default)                                              |
| `lsp`            | `mrq lsp`                                                         |

## Input

Every command that reads Markdown accepts any mix of:

- a file: `mrq toc README.md`
- a directory, walked recursively for `*.md` / `*.markdown` while respecting `.gitignore` and the `ignore` setting: `mrq lint docs/`
- a glob: `mrq stats 'docs/**/*.md'`
- `-` for stdin: `cat notes.md | mrq toc -`

With no path, commands that make sense on a project (`lint`, `fmt`, `validate`, `stats`) use the current directory. Commands that rewrite files (`fmt`, `lint --fix`, `interpolate --in-place`, `split`) refuse to write stdin input back and print to stdout instead.

## Commands

### `mrq toc`

Prints the heading outline of each file.

```sh
mrq toc README.md
mrq toc --json docs/ | jq '.[].headings[].slug'
mrq toc --depth 3 --links README.md        # nested Markdown list with anchor links
mrq toc --insert README.md                 # refresh a <!-- mrq:toc --> … <!-- mrq:end --> block in place
mrq toc --insert --check docs/             # CI: exit 1 if a table of contents is out of date
```

Anchor slugs follow the active flavor's rules (GitHub-style for `gfm`). `--json` emits level, text, slug and source position for each heading, as an array with one `{file, flavor, headings}` object per input. With several inputs, the text and `--links` output puts a `==> file <==` header before each file.

`--insert` replaces the content between `<!-- mrq:toc -->` and `<!-- mrq:end -->` with the `--links` list of the document's headings, framed by blank lines, and leaves every other byte of the file as it was. A start marker without an `mrq:end` gets one inserted after the list. `depth=N` on the marker (`<!-- mrq:toc depth=2 -->`) limits the levels for that region and takes precedence over `--depth`. Headings inside the region itself are not listed, so running `--insert` again changes nothing. Files without an `mrq:toc` marker are left untouched, and stdin input is printed to stdout instead. `--check` writes nothing, prints `<file>: table of contents is out of date` for each stale input and exits 1 if there is one.

### `mrq render`

Renders files or stdin to another format.

```sh
mrq render -o html README.md > README.html
mrq render -o html --fragment README.md          # body only, no <html> wrapper
mrq render -o ansi README.md | less -R
mrq render -o pdf -s paper report.md > report.pdf
mrq render -o md --flavor commonmark notes.md    # down-convert flavor-specific syntax
mrq render -o json README.md                     # the full AST
mrq render -o man mrq.1.md > mrq.1
mrq render -o ansi --width 100 README.md         # lay out for 100 columns
mrq render -o pdf --out book.pdf chapters/       # all chapters as one PDF
mrq render -o html --out-dir site/ docs/         # one page per file: site/<name>.html
```

Several inputs are concatenated into one document the way `mrq merge --no-wrap` does it: anchors are made unique, links between the files point into the combined document, and other relative links are rebased to the output (`--out`, else the current directory). `-o json` instead gives an array of `{file, ast}` objects. With `--out-dir DIR`, each input is rendered on its own to `DIR/<stem>.<ext>` (`html`, `ansi`, `pdf`, `md`, `json`, and for man pages the section extension of the name, `mrq.1.md` → `mrq.1`, else `man`). Files are written atomically and the directory is created. With `-o md`, `--flavor` names the flavor to convert to, and the input flavor comes from frontmatter or the configuration. `-o ansi` lays out for the width of the terminal, else `$COLUMNS`, else 80 columns, unless `--width` is given. A PDF is never written to a terminal: redirect stdout or use `--out`. Warnings, such as a diagram shown as code, go to stderr; an unreadable input, an unknown style or a PDF that does not compile exits with 2 and writes nothing.

| Format | Notes                                                                              |
| ------ | ---------------------------------------------------------------------------------- |
| `html` | Standalone page or `--fragment`; heading ids, highlighted code, inline SVG diagrams, MathML |
| `ansi` | Terminal output with colors, highlighting, tables and (where the terminal supports it) images |
| `pdf`  | Produced via embedded Typst; no external tools needed                              |
| `md`   | Normalized Markdown, optionally converted to another flavor                        |
| `json` | Stable mdast-like AST with source positions                                        |
| `man`  | roff, for man pages                                                                |

#### Styles

`--style/-s` accepts a built-in theme name (`light`, `dark`, `github`, `paper`) or a path to a theme file. A theme describes colors, fonts, heading styles and the code-highlighting theme once, and every backend maps it as well as it can. Each backend also takes an optional native override for things the portable theme can't express:

```yaml
# mytheme.yaml
extends: github
colors:
  foreground: "#24292f"
  accent: "#0969da"
  code_background: "#f6f8fa"
fonts:
  body: "Inter"
  mono: "JetBrains Mono"
headings:
  h1: { size: 2.0em, weight: bold, underline: true }
  h2: { size: 1.5em, weight: bold }
code:
  theme: "Solarized (light)"   # any bundled syntect theme or a .tmTheme path
overrides:
  html: ./extra.css            # appended to the generated stylesheet
  pdf: ./template.typ          # Typst template wrapping the document
  ansi: ./ansi.yaml            # per-element ANSI styles
```

#### Diagrams

Fenced blocks with a diagram language become images in HTML, PDF and capable terminals. Anywhere else they fall back to the original code block.

| Language        | Renderer (first available wins)                          |
| --------------- | -------------------------------------------------------- |
| `mermaid`       | built in (pure Rust) → `mmdc` → Kroki                    |
| `dot`/`graphviz`| `dot` → built in (pure Rust, basic layouts) → Kroki      |
| `d2`            | `d2` → Kroki                                             |
| `plantuml`      | `plantuml` → Kroki                                       |
| anything else   | any command you configure, or Kroki                      |

Rendered output is cached by content hash in the user cache directory. Kroki is never contacted unless it is enabled in the configuration.

### `mrq validate`

Checks syntax and consistency:

- internal links to other files, and anchors within and across files
- referenced assets (images, linked files)
- footnotes: undefined references and unused definitions
- link reference definitions: undefined and unused
- duplicate heading anchors
- frontmatter: parse errors, and JSON Schema validation (required keys, types, enums, …)
- `mrq:include` directives: missing targets, cycles and stale content
- external http(s) links, with `--external` (retries, rate limiting, caching)

```sh
mrq validate                            # the current directory
mrq validate docs/
mrq validate --strict docs/             # warnings fail too
mrq validate --external --format sarif . > results.sarif
```

Diagnostics go to stdout in the `--format` (`human` by default, `github` when `GITHUB_ACTIONS=true`; also `json`, `ndjson` and `sarif`); the human format prints nothing when there is nothing to report. The inputs form one workspace rooted at the current directory, so a link starting with `/` resolves from there. Errors exit with 1, warnings (unused footnotes and references, duplicate anchors, unresolved wikilinks, stale tables of contents and include regions) only with `--strict`, and an unreadable input, a bad schema or an invalid `external_exclude` pattern with 2.

External links are only checked with `--external` or `validate.external: true`, skipping URLs that match a `validate.external_exclude` regular expression. URLs found working are cached for a day in `external-links.json` in the user cache directory: `$MRQ_CACHE_HOME`, else `$XDG_CACHE_HOME/mrq`, else `~/.cache/mrq`. `mrq-cli` builds with the `external-links` feature by default; without it, `--external` fails with exit code 2.

The frontmatter schema is configured per glob:

```yaml
# .mrq.yaml
validate:
  frontmatter:
    - files: "blog/**/*.md"
      schema: ./schemas/post.schema.json
```

`files` and `schema` are relative to the config file that declares them, and every matching schema applies. The draft (7, 2020-12, …) comes from the schema's `$schema`, defaulting to 2020-12. Schemas are self-contained: `$ref`s to other files or URLs are not followed, so validation never touches the network. Errors point at the offending value, an unexpected key, or the object missing a required key.

### `mrq lint`

Applies style rules. The rule set is compatible with markdownlint (`MD001`–`MD059`, same IDs and semantics), extended with mrq-specific rules (`MRQ001`…). Existing `.markdownlint.{json,jsonc,yaml}` files are honored when no `.mrq.yaml` has a `lint` section. Rules are configured under `lint.rules` by ID, markdownlint alias (`line-length`) or tag (`whitespace`), with `true`/`on`, `false`/`off` or an options object.

```sh
mrq lint docs/
mrq lint --fix docs/
mrq lint --rule MD013 --rule MD022 README.md
mrq lint --diff docs/        # show what --fix would change, write nothing
mrq lint --format github     # annotations for GitHub Actions (auto-detected in CI)
```

`--fix` applies every fix that does not overlap another, re-parses and re-lints, and repeats until nothing changes (at most 10 passes). Changed files are rewritten atomically, and the findings left are reported. `--fix-only` fixes without reporting what is left, and `--diff` prints the fixes as a unified diff instead of writing them. With stdin, `--fix` prints the fixed text to stdout and the findings to stderr. `--rule` takes an ID, alias or tag, runs only those rules, and turns them on even if the configuration turns them off. `lint` exits 1 when findings are left (never with `--fix-only`) and `--diff` exits 1 when there is something to fix.

mrq's own rules are documented in [docs/rules](docs/rules/README.md):

| Rule | Alias | Default | Description |
| --- | --- | --- | --- |
| [MRQ001](docs/rules/MRQ001.md) | `valid-anchors` | off | Link to a missing anchor, flavor-aware and across files |
| [MRQ002](docs/rules/MRQ002.md) | `heading-punctuation` | off | Heading text ends with punctuation |
| [MRQ003](docs/rules/MRQ003.md) | `frontmatter-key-order` | off | Frontmatter keys out of order |
| [MRQ004](docs/rules/MRQ004.md) | `stale-include` | on | `mrq:include` region out of date |
| [MRQ005](docs/rules/MRQ005.md) | `no-url-wikilinks` | on | Bare URL in a wikilink |

Rules can be suppressed inline:

```markdown
<!-- mrq-disable MD033 -->
<div align="center">…</div>
<!-- mrq-enable MD033 -->

Very long line … <!-- mrq-disable-line MD013 -->
```

`markdownlint-disable*` comments are honored too.

### `mrq fmt`

A canonical formatter for Markdown, in the spirit of rustfmt and prettier. It normalizes:

- emphasis and strong markers, list markers, code fence style, heading style (ATX)
- table column alignment, with Unicode width taken into account
- ordered-list numbering (`one`, `ordered` or `preserve`)
- blank lines between blocks, trailing whitespace and the final newline
- flavor syntax: callouts, wikilinks and tags (`obsidian`), admonitions with their bodies re-indented, heading attributes, definition lists and abbreviations (`mkdocs`)
- prose wrapping: `preserve` (default), `never`, `width` (wrap at N columns) or `sentence` (one sentence per line)
- code inside fenced blocks, optionally, by calling configured external formatters (e.g. `rustfmt` for `rust`); code a formatter fails on (non-zero exit, timeout, output that isn't UTF-8) is left unchanged with a warning
- frontmatter is kept byte for byte, unless `fmt.frontmatter: normalize` asks for canonical YAML or TOML (it stays as written, with a warning, where normalizing would drop comments or change its data)

```sh
mrq fmt .                    # rewrite in place
mrq fmt --check .            # exit 1 and show a diff if anything would change
cat notes.md | mrq fmt -     # stdin → stdout
```

With no path, `fmt` formats the current directory. Files are rewritten atomically, and only when their formatting changes; stdin is never written back, its formatted text goes to stdout. `--check` writes nothing, prints a unified diff for each input that is not formatted and exits 1 if there is one. The `fmt` settings come from the configuration (see [Configuration](#configuration)); a relative program path in `code_formatters` (`./tools/fmt.sh`) is relative to the config file that sets it. Code left unformatted and frontmatter left as written are reported as warnings on stderr in the `--format`, and never change the exit code.

Formatting is idempotent and never changes the rendered meaning of a document.

### `mrq query`

Selects parts of a document. There are three syntaxes, all evaluated by the same engine:

```sh
# Markdown-native selectors (default)
mrq query '# Install | ```rust' README.md         # Rust code blocks in the "Install" section
mrq query '[]()' docs/                            # all links
mrq query '- [ ]' TODO.md                         # open tasks
mrq query '# /^API/ | :-:' api.md                 # tables under headings matching /^API/

# CSS-style selectors
mrq query --css 'section[heading="Install"] code[lang=rust]' README.md
mrq query --css 'link[url^="http"]' docs/

# jq over the JSON AST (embedded jaq, no jq install needed)
mrq query --jq '.. | select(.type? == "link") | .url' docs/
```

Output is Markdown by default (`-o md`). It can also be `json` (nodes with source positions) or `text` (plain text content). jq results that aren't nodes of the document print as plain strings or compact JSON. `-e`/`--exit-status` exits with 1 when nothing matches.

Markdown selector cheat sheet:

| Selector                 | Matches                           |
| ------------------------ | --------------------------------- |
| `# text`                 | section whose heading matches     |
| `-` / `1.`               | unordered / ordered list items    |
| `- [ ]` / `- [x]` / `- [?]` | open / done / any tasks        |
| `[text](url)` / `![]()`  | links / images                    |
| `>`                      | blockquotes                       |
| ```` ```lang ````        | code blocks                       |
| `:-:`                    | tables                            |
| `</>`                    | HTML                              |
| `+++`                    | frontmatter                       |
| `P:`                     | paragraphs                        |
| `a \| b`                 | pipe: select `b` within results of `a` |

Text matchers: `word` (case-insensitive substring), `"exact"`, `/regex/`, `^anchored$`, `*` (any).

### `mrq merge`

Combines several files into one document. By default each file becomes a section titled by its frontmatter `title`, its first H1 or its file name, with its own headings pushed down one level. A file that starts with its only H1 keeps that heading as its section heading. `--title` adds a level-1 heading on top, and pushes everything below it down one more level unless `--shift` says otherwise.

```sh
mrq merge intro.md install.md usage.md > guide.md
mrq merge --shift 2 --title "User Guide" chapters/ -o guide.md
mrq merge --no-wrap a.md b.md            # concatenate with no extra section heading
```

Relative links and asset paths are rewritten to stay valid from the output location. Cross-file links become in-document anchors, and colliding anchors are made unique. Frontmatter is merged; the first file wins on conflicts, or use `--frontmatter keep-first|merge|strip`.

### `mrq interpolate`

Expands include directives written as HTML comments, so the source stays valid Markdown:

```markdown
<!-- mrq:include path="chapters/install.md" shift=1 -->
<!-- mrq:end -->
```

```sh
mrq interpolate book.md > book.full.md    # write the combined result to stdout
mrq interpolate --keep-directives book.md # stdout, the markers kept around the content
mrq interpolate --in-place README.md      # fill/refresh the content between the markers
mrq interpolate --check README.md         # exit 1 and print a diff if any block is stale (CI)
```

In-place mode is idempotent: content between `mrq:include` and `mrq:end` is regenerated on every run, framed by blank lines, so the file stays both readable and current. A start marker without `mrq:end` gets one. Markers inside the included content (of nested includes that failed, or `mrq:toc` regions) are dropped, so each region keeps its own end marker. `frontmatter=merge` applies only to the stdout expansion; in-place mode leaves the host's frontmatter as written.

| Attribute       | Meaning                                                                       |
| --------------- | ----------------------------------------------------------------------------- |
| `path`          | file or http(s) URL (URLs need `remote-include` and `include.allow_remote`) |
| `shift`         | push headings down (or up, if negative) by N levels                           |
| `section`       | include only the section under this heading                                   |
| `select`        | include only what this query selects (`select="# Install \| ```sh"`)          |
| `frontmatter`   | `strip` (default), `keep` or `merge` into the host file                       |
| `lang`          | include as a fenced code block in this language (any non-Markdown file)       |
| `lines`         | line range for code includes: `10-20`, `10-`, `-20`                           |
| `function`, `struct`, `class`, `symbol` | structural selection from code files via tree-sitter (Rust, Python, JavaScript, TypeScript, Go); qualified names like `Type::method`; `lang` defaults to the file's language |
| `with_docs`     | keep the doc comments, attributes and decorators of a selected symbol (default `false`) |
| `dedent`        | strip common indentation (default `true` for code includes)                   |
| `sha256`        | pin the content to this SHA-256 (64 hex digits); different content is an error |

```markdown
<!-- mrq:include path="src/main.rs" function="main" -->
<!-- mrq:end -->
```

Includes are resolved recursively. Cycles are detected and reported with the full include chain. Links and assets in included content are rewritten relative to the including file, and links to the anchors of included files (`chapter.md#setup`) go to those headings in the expanded document. Remote includes are fetched only with `include.allow_remote: true`; relative links in remote content become absolute URLs, and its own includes resolve against its URL. Fetched content is cached for an hour in `remote-includes/` in the user cache directory. `sha256="…"` pins the content of an include: different content is an error, and pinned remote content is served from the cache whenever it is there.

### `mrq split`

The inverse of `merge`: splits a file at a heading level into one file per section.

```sh
mrq split -d 2 guide.md
```

```text
guide/
├── index.md            # preamble + mrq:include directives for each part
├── 01-introduction.md
├── 02-installation.md
└── 03-usage.md
```

Headings in each part are promoted so it starts at level 1 (the include directive's `shift=` pushes them back). Links between sections are rewritten to point at the new files, and relative assets are adjusted. `mrq interpolate guide/index.md` reproduces the original document, with its relative links rebased to `guide/`. `--out DIR` picks another directory (`--out .` puts the files next to `guide.md`, and then the expansion is byte for byte the original), and `--dry-run` lists the files without writing them.

### `mrq stats`

Per-file and total counts: words, characters, estimated reading time, headings per level, links (internal/external), images, code blocks per language, tables, footnotes and tasks (open/done).

```sh
mrq stats docs/
mrq stats --json docs/ | jq '.total.words'
mrq stats --wpm 180 book.md
```

### `mrq diff`

A structural, semantic diff. Sections are matched by heading, even when they have moved. Blocks are compared by meaning, so reflowed paragraphs, a different list marker or a re-aligned table are not reported as changes.

```sh
mrq diff old.md new.md
mrq diff --json old.md new.md
mrq diff --word old.md new.md     # word-level changes inside modified paragraphs
git difftool -x 'mrq diff' -- README.md
```

`OLD` or `NEW` (not both) can be `-` for stdin, and `/dev/null` or an empty file stands for a missing side, as `git difftool` passes for added and deleted files. The exit code is 0 if the documents differ only in formatting, 1 if they differ, and 2 if an input cannot be read. The flavor comes from `--flavor`, the frontmatter or the config, as for every command.

The output starts with `--- OLD` and `+++ NEW` (nothing at all if there is no change), then lists each changed section in the new document's order: a header with its heading level and breadcrumb (`## Guide › Install`, `(preamble)` for what precedes the first heading), the tags `added`, `removed`, `moved from <old path>` or `renamed from <old title>`, and its line in each document (`old:9 new:10`), followed by its changed blocks in their canonical form. Deleted blocks are prefixed with `-`, inserted ones with `+`, and a modified block is shown as both; with `--word`, a modified block is one `~` line per line with `[-deleted-]` and `{+inserted+}` runs. Colors follow `--color` (by default only on a terminal, and not if `NO_COLOR` is set): headers in bold cyan, deletions in red and insertions in green, without the `[-…-]`/`{+…+}` markers.

```text
## Reference › Usage  [moved from Guide › Usage]  old:15 new:20
- Call `mrq` with a file.
+ Call `mrq` with one or more files.
```

`--json` prints `{"mrq_diff": 1, "old", "new", "sections"}` with the changed sections only. Each has `status` (`added`, `removed` or `matched`), `moved`, `renamed`, `modified`, `old` and `new` (`{path, title, level, line}` or `null`) and `blocks`: the ops `insert`, `delete` and `modify`, each with the block `kind` and `old`/`new` as `{text, lines: [first, last]}`, and with `--word` the modified block's `words` (`{op: equal|insert|delete, text}`). Lines are 1-based.

### `mrq show`

An interactive viewer for one or more files, or a directory, with colors, syntax highlighting, rendered tables, inline images and diagrams (on kitty, iTerm2, WezTerm or sixel terminals), and an outline sidebar.

| Key                     | Action                                                |
| ----------------------- | ----------------------------------------------------- |
| `↑` `↓` / `PgUp` `PgDn` / `Home` `End` | move the selection / scroll a page / go to the start or end |
| `Tab`                   | switch between panels (outline, document, file list)  |
| `o`                     | toggle the outline sidebar                            |
| `Enter`                 | follow the link in the selected block / jump to the outline entry's heading / open the file |
| `←` / `→`               | choose another link in the selected block (shown in the status line) |
| `Backspace`             | go back                                               |
| `Shift-←` / `Shift-→`   | promote / demote the selected heading (with its section) |
| `Alt-↑` / `Alt-↓`       | move the selected section up / down                   |
| `d` / `Delete`          | delete the selected section                           |
| `/`, `n`, `N`           | search, next / previous match                         |
| `>`                     | run a query; matches are highlighted and navigable    |
| `Esc`                   | clear the search or query highlighting                |
| `u` / `Ctrl-r`          | undo / redo                                           |
| `w`                     | save                                                  |
| `e`                     | edit the file in `$VISUAL` / `$EDITOR` at the selected line, then reload it |
| `R`                     | reload the file from disk                             |
| `?`                     | help                                                  |
| `q`                     | quit (asks if there are unsaved changes; `Ctrl-c` does not ask) |

`Enter` follows the first link (or wikilink) of the selected block, or the one chosen with `←`/`→`. Links resolve as for `mrq validate`: `#anchor` jumps to the heading in the same file, another Markdown file opens in the viewer (added to the file list, at its `#anchor` if given), and web links, `mailto:` addresses and other local files open in the system's opener (`open`, `xdg-open` or `explorer`). `Backspace` goes back to where each jump came from, including jumps from the outline and the file list. A link whose target is missing says so in the status line.

`/` opens a search prompt in the status line. The search runs as you type over the text as it is shown (not the Markdown source), highlighting every match and selecting the first one at or after the selection. It is smart-case: a pattern in lower case ignores case, and one with an upper-case letter matches it exactly. A match never spans two lines. `Enter` keeps the search, and `Esc` cancels it and goes back to where it started. `>` opens a query prompt for the [selector syntax](#mrq-query), or CSS after `css:` and jq after `jq:` (`>css:link[url^="http"]`). It runs on `Enter`; the blocks of its matches are marked in the gutter, and the first match at or after the selection is selected. A query that does not parse keeps the prompt open, with the error and its column in the status line and the cursor on the problem. jq outputs that aren't nodes of the document are skipped. Either way, `n` and `N` go to the next and previous match, wrapping around, with `match i/n` in the status line, and `Backspace` goes back to where the search or query started. The search or query applies to whichever file is shown; `Esc` clears it.

The editing keys act on a section: the selected heading with everything up to the next heading of the same or a higher level, or, if the selection is not on a heading, the innermost section it is in (with the outline focused, the section of the outline entry). `Shift-←`/`Shift-→` move the heading and all its subheadings one level up or down, the way `mrq merge --shift` does (a setext heading becomes ATX); they refuse, with a message in the status line, when a heading would go above level 1 or below level 6. `Alt-↑`/`Alt-↓` swap the section with the previous or next section that has the same parent, and `d` deletes it with the blank lines after it (before it, for the last section), without asking, since `u` brings it back. Every edit changes only the bytes of the section, and `u` and `Ctrl-r` undo and redo them byte for byte; after an edit the selection stays on the edited section. A file with unsaved edits has `[+]` after its name in the status line and the file list. `w` writes the shown file atomically (a temporary file renamed over it, keeping its permissions); stdin input cannot be written. `q` asks before quitting while any file has unsaved edits: `y` quits, any other key goes back.

`e` hands the terminal to `$VISUAL`, else `$EDITOR`, else `vi`, as `$EDITOR +line file` with the line of the selected block (the command is split at spaces, so `code --wait` works), and reloads the file when the editor exits. It refuses stdin input, and a file with unsaved edits: write them with `w` or drop them with `R` first, so that neither the editor nor the reload loses them.

Open files are watched (through their directories, so saves that rename a temporary file over the original are seen) and reloaded when they change on disk, with the selection kept on the same line of text. Our own `w` and changes that leave the text as it was are ignored. A file with unsaved edits is not reloaded: the status line says it changed, `w` writes the edits over it, and `R` reloads it, which `u` takes back to the edits. A file deleted on disk keeps its content in the viewer, and `w` writes it back. `R` also reloads a file without edits by hand, e.g. where the file system cannot be watched.

### `mrq lsp`

A Language Server Protocol server on stdio, built on the same engine as `validate` and `lint`:

- diagnostics from `validate` and `lint` as you type
- go to definition for links, anchors, footnotes and link references
- find references to a heading from every file in the workspace
- completion of file paths, heading anchors (including in other files) and footnote labels
- rename a heading, and every link to its anchor is updated across the workspace
- document symbols (outline) and workspace symbols
- formatting (`mrq fmt`) and code actions (`lint --fix` per diagnostic)
- hover previews for links and footnotes

Neovim:

```lua
vim.lsp.config('mrq', { cmd = { 'mrq', 'lsp' }, filetypes = { 'markdown' }, root_markers = { '.mrq.yaml', '.git' } })
vim.lsp.enable('mrq')
```

Helix (`languages.toml`):

```toml
[language-server.mrq]
command = "mrq"
args = ["lsp"]

[[language]]
name = "markdown"
language-servers = ["mrq"]
```

## Flavors

The flavor decides which syntax is recognized, how anchors are slugged, and what `fmt` and `render -o md` emit.

| Flavor       | Adds                                                                                         |
| ------------ | -------------------------------------------------------------------------------------------- |
| `commonmark` | nothing; strict CommonMark                                                                   |
| `gfm`        | tables, task lists, strikethrough, autolinks, footnotes, alerts (`> [!NOTE]`); GitHub slugs |
| `obsidian`   | `gfm` + wikilinks `[[page#heading\|alias]]`, embeds `![[file]]`, callouts, `#tags`, `$math$`, `%%comments%%` |
| `mkdocs`     | `gfm` + admonitions (`!!! note`), heading attributes `{#id .class}`, definition lists, `$math$`, abbreviations |

The flavor comes from `--flavor`, then a frontmatter `mrq.flavor` key, then the closest `.mrq.yaml`, then `gfm`.

## Configuration

mrq reads `.mrq.yaml` from the current directory and its ancestors (the closest one wins, key by key), then the user config at `~/.config/mrq/config.yaml`. Command-line flags override everything. `mrq config show` prints the effective configuration and where each value came from. A JSON Schema for the config file is published for editor completion.

```yaml
# .mrq.yaml
flavor: gfm
ignore: ["vendor/**", "CHANGELOG.md"]

lint:
  extends: default            # or "none"
  rules:
    MD013: { line_length: 120, code_blocks: false }
    MD033: { allowed_elements: [details, summary, kbd] }
    MD041: off

fmt:
  wrap: preserve              # preserve | never | width | sentence
  width: 80
  emphasis: "*"
  strong: "**"
  list_marker: "-"
  ordered_list: ordered       # one | ordered | preserve
  code_fence: "```"
  code_formatters:           # code on stdin, formatted code on stdout; kept on failure
    rust: ["rustfmt", "--emit=stdout"]
  frontmatter: preserve       # preserve | normalize

validate:
  external: false
  external_exclude: ["^https://localhost"]
  frontmatter:
    - files: "blog/**/*.md"
      schema: ./schemas/post.schema.json

include:
  allow_remote: false

render:
  style: github
  diagrams:
    kroki: { enabled: false, url: "https://kroki.io" }
    commands:
      bob: ["svgbob"]         # custom renderer: reads source on stdin, writes SVG to stdout
```

## Using `mrq-core` as a library

```toml
[dependencies]
mrq-core = "1"
```

```rust no_run
use mrq_core::{Document, config::LintConfig, lint::{Linter, Registry}, query::Query};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Flavor from frontmatter `mrq.flavor`, else gfm. `DocumentOptions` adds
    // `--flavor`, the configured flavor and custom flavor registries.
    let doc = Document::load("README.md")?;

    for heading in doc.toc() {
        println!("{}{} (#{})", "  ".repeat(heading.level as usize - 1), heading.text, heading.slug);
    }

    let snippets = doc.select(&Query::selector("# Install | ```sh")?)?;
    for node in snippets {
        println!("{}", node.text());
    }

    let linter = Linter::new(&Registry::builtin(), &LintConfig::default())?;
    for finding in linter.check(&doc) {
        println!("{}: {}", finding.code, finding.message);
    }

    Ok(())
}
```

`mrq-core` covers parsing, the document model, frontmatter, query, validate, lint, fmt and transforms (merge, split, interpolate, diff). Rendering, the TUI and the language server live in separate crates. Heavy or network-bound capabilities are behind cargo features.

## Exit codes

| Code | Meaning                                                                   |
| ---- | ------------------------------------------------------------------------- |
| 0    | success, nothing to report                                                |
| 1    | findings: validation/lint errors, `--check` mismatches, a non-empty `diff` |
| 2    | usage or I/O error                                                        |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
