#!/usr/bin/env python3
"""Render the marquee Homebrew formula (Formula/marquee.rb, installs `mrq`) for a stable release.

Used by the `tap-update` job of .github/workflows/release.yml. Inputs come from the
environment:

  TAG                                  stable release tag, e.g. v0.1.0
  SHA_AARCH64_APPLE_DARWIN             sha256 of mrq-$TAG-aarch64-apple-darwin.tar.gz
  SHA_X86_64_APPLE_DARWIN              sha256 of mrq-$TAG-x86_64-apple-darwin.tar.gz
  SHA_AARCH64_UNKNOWN_LINUX_MUSL       sha256 of mrq-$TAG-aarch64-unknown-linux-musl.tar.gz
  SHA_X86_64_UNKNOWN_LINUX_MUSL        sha256 of mrq-$TAG-x86_64-unknown-linux-musl.tar.gz
  OUTPUT_PATH                          where to write (default: Formula/marquee.rb)
  MANIFEST_PATH                        mrq-cli Cargo.toml (default: crates/mrq-cli/Cargo.toml
                                       relative to this repository)

`desc`, `homepage`, `license` and the download base URL (`repository`) come from the
mrq-cli crate metadata. Nothing is written unless every input is valid: a blank or
malformed checksum would make `brew install` fail for every user.

Archives from taiki-e/upload-rust-binary-action have no leading directory: `mrq`,
`man/*.1` and `completions/*` sit at the archive root.
"""

import os
import pathlib
import re
import sys
import tomllib

REPO = pathlib.Path(__file__).resolve().parent.parent.parent
DEFAULT_MANIFEST = REPO / "crates" / "mrq-cli" / "Cargo.toml"

# (target, on_macos/on_linux, on_arm/on_intel)
TARGETS = (
    ("aarch64-apple-darwin", "on_macos", "on_arm"),
    ("x86_64-apple-darwin", "on_macos", "on_intel"),
    ("aarch64-unknown-linux-musl", "on_linux", "on_arm"),
    ("x86_64-unknown-linux-musl", "on_linux", "on_intel"),
)

STABLE_TAG = re.compile(r"v(\d+\.\d+\.\d+)")
SHA256 = re.compile(r"[0-9a-f]{64}")


class RenderError(Exception):
    pass


def checksum_var(target):
    return "SHA_" + target.upper().replace("-", "_")


def load_metadata(manifest_path):
    """description, homepage, repository and license of a crate, resolving
    `field.workspace = true` against the nearest ancestor workspace manifest."""
    manifest_path = pathlib.Path(manifest_path)
    package = tomllib.loads(manifest_path.read_text(encoding="utf-8")).get("package", {})
    workspace = None
    meta = {}
    for field in ("description", "homepage", "repository", "license"):
        value = package.get(field)
        if isinstance(value, dict) and value.get("workspace") is True:
            if workspace is None:
                workspace = _workspace_package(manifest_path)
            value = workspace.get(field)
        if not isinstance(value, str) or not value.strip():
            raise RenderError(f"{manifest_path}: package.{field} is missing")
        meta[field] = value.strip()
    return meta


def _workspace_package(manifest_path):
    for directory in manifest_path.resolve().parent.parents:
        candidate = directory / "Cargo.toml"
        if candidate.is_file():
            data = tomllib.loads(candidate.read_text(encoding="utf-8"))
            if "workspace" in data:
                return data["workspace"].get("package", {})
    raise RenderError(f"{manifest_path}: no workspace manifest found")


def ruby_string(value):
    escaped = value.replace("\\", "\\\\").replace('"', '\\"').replace("#{", "\\#{")
    return f'"{escaped}"'


def ruby_license(spdx):
    parts = [p.strip() for p in spdx.split(" OR ")]
    if not all(re.fullmatch(r"[A-Za-z0-9.+-]+", p) for p in parts):
        raise RenderError(f"unsupported license expression: {spdx!r}")
    if len(parts) == 1:
        return ruby_string(parts[0])
    return "any_of: [" + ", ".join(ruby_string(p) for p in parts) + "]"


def render(tag, checksums, meta):
    match = STABLE_TAG.fullmatch(tag)
    if not match:
        raise RenderError(f"TAG {tag!r} is not a stable release tag (vX.Y.Z)")
    version = match.group(1)
    for target, _, _ in TARGETS:
        value = checksums.get(target)
        if value is None or not value.strip():
            raise RenderError(f"{checksum_var(target)} is empty: refusing to render a formula without a checksum")
        if not SHA256.fullmatch(value):
            raise RenderError(f"{checksum_var(target)} is not a sha256 hex digest: {value!r}")

    base = f"{meta['repository'].rstrip('/')}/releases/download/{tag}"
    blocks = []
    for os_block in ("on_macos", "on_linux"):
        arches = []
        for target, os_name, arch in TARGETS:
            if os_name != os_block:
                continue
            arches.append(
                f"    {arch} do\n"
                f"      url {ruby_string(f'{base}/mrq-{tag}-{target}.tar.gz')}\n"
                f"      sha256 {ruby_string(checksums[target])}\n"
                f"    end\n"
            )
        blocks.append(f"  {os_block} do\n" + "\n".join(arches) + "  end\n")

    return (
        "# Rendered by the marquee release workflow (.github/scripts/render_formula.py).\n"
        "class Marquee < Formula\n"
        f"  desc {ruby_string(meta['description'])}\n"
        f"  homepage {ruby_string(meta['homepage'])}\n"
        f"  version {ruby_string(version)}\n"
        f"  license {ruby_license(meta['license'])}\n"
        "\n" + "\n".join(blocks) + "\n"
        "  def install\n"
        '    bin.install "mrq"\n'
        '    man1.install Dir["man/*.1"]\n'
        '    bash_completion.install "completions/mrq.bash"\n'
        '    zsh_completion.install "completions/_mrq"\n'
        '    fish_completion.install "completions/mrq.fish"\n'
        "  end\n"
        "\n"
        "  test do\n"
        '    assert_match version.to_s, shell_output("#{bin}/mrq --version")\n'
        "  end\n"
        "end\n"
    )


def main(env):
    try:
        tag = env.get("TAG", "")
        checksums = {t: env.get(checksum_var(t)) for t, _, _ in TARGETS}
        meta = load_metadata(env.get("MANIFEST_PATH") or DEFAULT_MANIFEST)
        formula = render(tag, checksums, meta)
    except (RenderError, OSError, tomllib.TOMLDecodeError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    output = pathlib.Path(env.get("OUTPUT_PATH") or "Formula/marquee.rb")
    try:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(formula, encoding="utf-8")
    except OSError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    print(f"formula written to {output}")
    return 0


if __name__ == "__main__":
    sys.exit(main(os.environ))
