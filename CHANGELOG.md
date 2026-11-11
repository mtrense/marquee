# Changelog

All notable changes to this project are documented in this file. The format is based on [Keep a Changelog](https://keepachangelog.com/) and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-11-11

### Added

- Release binaries are now built with every optional feature (`pdf` and `mermaid`) enabled.

### Changed

- Homebrew formula renamed to `marquee`; install with `brew install mtrense/tap/marquee` (installs the `mrq` binary).

### Fixed

- `mrq fmt` is now idempotent for raw HTML (e.g. `</div>`) on paragraph continuation lines; such HTML is indented four columns in all wrap modes so it no longer re-parses as an HTML block.
- Release tooling (xtask) no longer fails clippy on Windows, and script output no longer surfaces as spurious GitHub annotations.
