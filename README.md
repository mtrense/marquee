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
| `remote-include` | `mrq:include` from http(s) URLs                                   |
| `tree-sitter`    | structural code includes (`function="…"`) and tree-sitter grammars |
| `mermaid`        | built-in Mermaid rendering (no Node.js needed)                    |
| `tui`            | `mrq show`                                                        |
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
```

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

Diagnostics go to stdout in the `--format` (`human` by default, `github` when `GITHUB_ACTIONS=true`; also `json`, `ndjson` and `sarif`); the human format prints nothing when there is nothing to report. The inputs form one workspace rooted at the current directory, so a link starting with `/` resolves from there. Errors exit with 1, warnings (unused footnotes and references, duplicate anchors, unresolved wikilinks, stale tables of contents) only with `--strict`, and an unreadable input, a bad schema or an invalid `external_exclude` pattern with 2.

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
mrq lint --format github     # annotations for GitHub Actions (auto-detected in CI)
```

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
- prose wrapping: `preserve` (default), `never`, `width` (wrap at N columns) or `sentence` (one sentence per line)
- code inside fenced blocks, optionally, by calling configured external formatters (e.g. `rustfmt` for `rust`)

```sh
mrq fmt .                    # rewrite in place
mrq fmt --check .            # exit 1 and show a diff if anything would change
cat notes.md | mrq fmt -     # stdin → stdout
```

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

Combines several files into one document. By default each file becomes a section titled by its frontmatter `title` or first heading, with its own headings pushed down one level.

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
mrq interpolate book.md > book.full.md   # write the combined result to stdout
mrq interpolate --in-place README.md     # fill/refresh the content between the markers
mrq interpolate --check README.md        # exit 1 if any block is stale (CI)
```

In-place mode is idempotent: content between `mrq:include` and `mrq:end` is regenerated on every run, so the file stays both readable and current.

| Attribute       | Meaning                                                                       |
| --------------- | ----------------------------------------------------------------------------- |
| `path`          | file or http(s) URL (URLs need `remote-include` and `include.allow_remote`) |
| `shift`         | push headings down (or up, if negative) by N levels                           |
| `section`       | include only the section under this heading                                   |
| `select`        | include only what this query selects (`select="# Install \| ```sh"`)          |
| `frontmatter`   | `strip` (default), `keep` or `merge` into the host file                       |
| `lang`          | include as a fenced code block in this language (any non-Markdown file)       |
| `lines`         | line range for code includes: `10-20`, `10-`, `-20`                           |
| `function`, `struct`, `class`, `symbol` | structural selection from code files via tree-sitter  |
| `dedent`        | strip common indentation (default `true` for code includes)                   |

```markdown
<!-- mrq:include path="src/main.rs" function="main" -->
<!-- mrq:end -->
```

Includes are resolved recursively. Cycles are detected and reported with the full include chain. Links and assets in included content are rewritten relative to the including file. Remote includes are cached and can be pinned with `sha256="…"`.

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

Headings in each part are promoted so it starts at level 1. Links between sections are rewritten to point at the new files, and relative assets are adjusted. `mrq interpolate guide/index.md` reproduces the original document.

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

### `mrq show`

An interactive viewer for one or more files, or a directory, with colors, syntax highlighting, rendered tables, inline images and diagrams (on kitty, iTerm2, WezTerm or sixel terminals), and an outline sidebar.

| Key                     | Action                                                |
| ----------------------- | ----------------------------------------------------- |
| `↑` `↓` / `PgUp` `PgDn` | scroll / move selection                               |
| `Tab`                   | switch between panels (outline, document, file list)  |
| `o`                     | toggle the outline sidebar                            |
| `Enter`                 | follow link / jump to heading                         |
| `Backspace`             | go back                                               |
| `Shift-←` / `Shift-→`   | promote / demote the selected heading (with its section) |
| `Alt-↑` / `Alt-↓`       | move the selected section up / down                   |
| `/`, `n`, `N`           | search, next / previous match                         |
| `>`                     | run a query; matches are highlighted and navigable    |
| `u` / `Ctrl-r`          | undo / redo                                           |
| `w`                     | save                                                  |
| `e`                     | open `$EDITOR` at the current line                    |
| `?`                     | help                                                  |
| `q`                     | quit (asks if there are unsaved changes)              |

Files are reloaded when they change on disk.

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
  code_formatters:
    rust: ["rustfmt", "--emit=stdout"]

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
use mrq_core::{Document, query::Query};

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
