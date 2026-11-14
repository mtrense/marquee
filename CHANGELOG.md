# Changelog

All notable changes to this project are documented in this file. The format is based on [Keep a Changelog](https://keepachangelog.com/) and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.3.0] - 2026-11-14

### Added

- `--open` flag for `render` opens the result in the default app. Without `--out` or `--out-dir`, it renders to a temp file named after the inputs, with links rebased to work from there. `$MRQ_OPENER` overrides the platform opener (open, start, xdg-open).
- `render` now reports `INPUT -> OUTPUT` on stderr for each file written, including `--out`, `--out-dir` and the `--open` temp file. Stdin shows as `<stdin>`. Nothing is printed when rendering to stdout or with `--quiet`.

## [0.2.1] - 2026-11-12

### Fixed

- Emoji in PDF output now render correctly instead of as missing-glyph boxes, by bundling the Noto Color Emoji font (SIL OFL 1.1) as a fallback font.

## [0.2.0] - 2026-11-11

### Added

- Release binaries are now built with every optional feature (`pdf` and `mermaid`) enabled.

### Changed

- Homebrew formula renamed to `marquee`; install with `brew install mtrense/tap/marquee` (installs the `mrq` binary).

### Fixed

- `mrq fmt` is now idempotent for raw HTML (e.g. `</div>`) on paragraph continuation lines; such HTML is indented four columns in all wrap modes so it no longer re-parses as an HTML block.
- Release tooling (xtask) no longer fails clippy on Windows, and script output no longer surfaces as spurious GitHub annotations.
