# List available recipes.
default:
    @just --list

# Format the whole workspace.
fmt:
    cargo fmt --all

# Verify formatting without modifying files.
fmt-check:
    cargo fmt --all --check

# Lint with clippy, every feature on; warnings are errors.
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run all tests, every feature on (external-link tests start a mock HTTP server on 127.0.0.1).
test:
    cargo test --workspace --all-features --no-fail-fast

# Advisories, licences, bans, duplicates and sources from deny.toml; needs `cargo install cargo-deny`.
deny:
    cargo deny check --hide-inclusion-graph

# Check mrq-core, mrq-render and mrq-cli with each feature on its own; needs `cargo install cargo-hack`.
features:
    cargo hack check --each-feature --no-dev-deps -p mrq-core -p mrq-render -p mrq-cli

# Check the workspace on the MSRV toolchain (rust-version in Cargo.toml); needs `rustup toolchain install 1.99`.
msrv:
    cargo +1.99 check --workspace --all-targets

# fmt-check + lint + test.
check: fmt-check lint test

# Debug build of the whole workspace.
build-debug:
    cargo build --workspace

# Release build of the whole workspace.
build-release:
    cargo build --workspace --release

# Release build of the whole workspace with all available features.
build-release-all-features:
    cargo build --workspace --release --all-features

# Install the `mrq` binary into ~/.cargo/bin.
install:
    cargo install --path crates/mrq-cli --locked

# Regenerate the checked-in JSON Schemas: the AST (crates/mrq-core/schema/) and themes (crates/mrq-render/schema/).
schema:
    MRQ_UPDATE_SCHEMA=1 cargo test -p mrq-core --test json_ast schema_file_is_up_to_date
    MRQ_UPDATE_SCHEMA=1 cargo test -p mrq-render --test theme schema_file_is_up_to_date

# Generate man pages for mrq and every subcommand into `out` (default target/man).
man out="target/man":
    cargo xtask man --out {{out}}

# Record markdownlint-cli2's findings on the lint fixtures, then compare mrq with them; needs Node.js (npx) and network.
lint-parity *args:
    cargo xtask lint-parity {{args}}

# Coverage via tarpaulin: HTML report + lcov.info in target/tarpaulin/, fails below 80% line coverage.
coverage:
    cargo tarpaulin --workspace --out Html --out Lcov --output-dir target/tarpaulin --fail-under 80

# --- Cross compilation ---------------------------------------------------------
# `build-native-*` recipes use the host's own cargo toolchain; they only need
# `rustup target add <triple>`. `build-cross-*` recipes build inside Docker via `cross`
# (https://github.com/cross-rs/cross), which ships toolchain images with the right linkers.
# Use them for targets this host cannot build natively.
# Binaries land in target/<triple>/release/mrq[.exe].

# Linux x86_64 (glibc), natively on an x86_64 Linux host.
build-native-linux-x86_64: (_native "x86_64-unknown-linux-gnu")

# Linux x86_64 (glibc), via cross + Docker; same triple as above, for building it from a Mac.
build-cross-linux-x86_64: (_cross "x86_64-unknown-linux-gnu")

# Linux aarch64 (glibc), via cross + Docker.
build-cross-linux-aarch64: (_cross "aarch64-unknown-linux-gnu")

# Linux x86_64 fully static (musl), via cross + Docker.
build-cross-linux-x86_64-musl: (_cross "x86_64-unknown-linux-musl")

# Linux aarch64 fully static (musl), via cross + Docker.
build-cross-linux-aarch64-musl: (_cross "aarch64-unknown-linux-musl")

# Windows x86_64 (MSVC), natively on a Windows host; no shebang so it runs on Windows runners.
build-native-windows-x86_64:
    rustup target list --installed | grep -qx 'x86_64-pc-windows-msvc' || { echo "error: target x86_64-pc-windows-msvc not installed; run: rustup target add x86_64-pc-windows-msvc" >&2; exit 1; }
    cargo build --workspace --release --target x86_64-pc-windows-msvc

# Windows x86_64 (MinGW), via cross + Docker; the way to get a Windows binary from a Mac or Linux.
build-cross-windows-x86_64: (_cross "x86_64-pc-windows-gnu")

# macOS Intel, natively on a Mac.
build-native-macos-x86_64: (_native "x86_64-apple-darwin")

# macOS Apple Silicon, natively on a Mac.
build-native-macos-aarch64: (_native "aarch64-apple-darwin")

# Every release target the current OS can build; one call per CI runner.
build-release-targets:
    @just {{ if os() == "windows" { "build-native-windows-x86_64" } else if os() == "macos" { "build-native-macos-x86_64 build-native-macos-aarch64" } else { "build-native-linux-x86_64 build-cross-linux-aarch64 build-cross-linux-x86_64-musl build-cross-linux-aarch64-musl build-cross-windows-x86_64" } }}

# Every target buildable from a Mac (all but Windows MSVC): cross for Linux/Windows, native for macOS.
build-cross-all: build-cross-linux-x86_64 build-cross-linux-aarch64 build-cross-linux-x86_64-musl build-cross-linux-aarch64-musl build-cross-windows-x86_64 build-native-macos-x86_64 build-native-macos-aarch64

# Release build of a target through `cross`; verifies cross and a Docker daemon are available.
_cross target:
    #!/usr/bin/env sh
    set -eu
    command -v cross >/dev/null 2>&1 || { echo "error: 'cross' not found; run: cargo install cross --git https://github.com/cross-rs/cross" >&2; exit 1; }
    docker info >/dev/null 2>&1 || { echo "error: no Docker daemon reachable; start Docker (e.g. Docker Desktop, colima start), cross needs it to run the {{target}} toolchain image" >&2; exit 1; }
    cross build --workspace --release --target {{target}}

# Release build of a target with plain cargo; verifies the rustup target is installed.
_native target:
    #!/usr/bin/env sh
    set -eu
    rustup target list --installed | grep -qx '{{target}}' || { echo "error: target {{target}} not installed; run: rustup target add {{target}}" >&2; exit 1; }
    cargo build --workspace --release --target {{target}}
