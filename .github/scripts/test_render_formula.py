"""Tests for render_formula.py.

Run with `python3 -I -B -m unittest discover -s .github/scripts` (the xtask test
`render_formula_python_tests_pass` does this as part of `just test`).
"""

import importlib.util
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent.parent
CLI_MANIFEST = REPO / "crates" / "mrq-cli" / "Cargo.toml"

_spec = importlib.util.spec_from_file_location("render_formula", HERE / "render_formula.py")
rf = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(rf)

TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
)


def sha(i):
    return f"{i:x}" * 64


def good_env(tmp):
    env = {"TAG": "v1.2.3", "OUTPUT_PATH": str(pathlib.Path(tmp) / "Formula" / "marquee.rb")}
    for i, target in enumerate(TARGETS):
        env[rf.checksum_var(target)] = sha(i + 1)
    return env


class MetadataTest(unittest.TestCase):
    def test_mrq_cli_metadata_feeds_desc_and_homepage(self):
        meta = rf.load_metadata(CLI_MANIFEST)
        self.assertEqual(meta["homepage"], "https://github.com/mtrense/marquee")
        self.assertEqual(meta["repository"], "https://github.com/mtrense/marquee")
        self.assertEqual(meta["license"], "MIT OR Apache-2.0")
        desc = meta["description"]
        # `brew audit` rules for `desc`.
        self.assertTrue(desc)
        self.assertLessEqual(len(desc), 80, desc)
        self.assertFalse(desc.lower().startswith("mrq"), desc)
        self.assertIsNone(re.match(r"(?i)an?\s", desc), desc)
        self.assertFalse(desc.endswith("."), desc)

    def test_workspace_inherited_fields_are_resolved(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/x"]\n\n[workspace.package]\n'
                'license = "MIT"\nrepository = "https://example.com/r"\n'
                'homepage = "https://example.com/h"\n'
            )
            (root / "crates" / "x").mkdir(parents=True)
            manifest = root / "crates" / "x" / "Cargo.toml"
            manifest.write_text(
                '[package]\nname = "x"\ndescription = "Thing"\n'
                "license.workspace = true\nrepository.workspace = true\n"
                "homepage = { workspace = true }\n"
            )
            meta = rf.load_metadata(manifest)
        self.assertEqual(meta["description"], "Thing")
        self.assertEqual(meta["license"], "MIT")
        self.assertEqual(meta["repository"], "https://example.com/r")
        self.assertEqual(meta["homepage"], "https://example.com/h")

    def test_missing_metadata_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = pathlib.Path(tmp) / "Cargo.toml"
            manifest.write_text('[package]\nname = "x"\n')
            with self.assertRaises(rf.RenderError):
                rf.load_metadata(manifest)


class RenderTest(unittest.TestCase):
    def setUp(self):
        self.meta = rf.load_metadata(CLI_MANIFEST)
        self.checksums = {t: sha(i + 1) for i, t in enumerate(TARGETS)}
        self.formula = rf.render("v1.2.3", self.checksums, self.meta)

    def test_header(self):
        f = self.formula
        self.assertIn("class Marquee < Formula\n", f)
        self.assertIn(f'  desc "{self.meta["description"]}"\n', f)
        self.assertIn('  homepage "https://github.com/mtrense/marquee"\n', f)
        self.assertIn('  version "1.2.3"\n', f)
        self.assertIn('  license any_of: ["MIT", "Apache-2.0"]\n', f)

    def test_each_platform_has_its_url_and_checksum(self):
        base = "https://github.com/mtrense/marquee/releases/download/v1.2.3"
        for target, checksum in self.checksums.items():
            block = f'url "{base}/mrq-v1.2.3-{target}.tar.gz"\n      sha256 "{checksum}"\n'
            self.assertIn(block, self.formula, target)
        self.assertNotIn("windows", self.formula)

    def test_platform_blocks(self):
        mac = self.formula.index("on_macos do")
        linux = self.formula.index("on_linux do")
        for target in TARGETS:
            pos = self.formula.index(f"-{target}.tar.gz")
            self.assertEqual("darwin" in target, mac < pos < linux, target)
            arm = self.formula.rindex("on_arm do", 0, pos)
            intel = self.formula.rfind("on_intel do", 0, pos)
            self.assertEqual(target.startswith("aarch64"), arm > intel, target)

    def test_installs_binary_man_pages_and_completions(self):
        install = self.formula[self.formula.index("def install") :]
        for line in (
            'bin.install "mrq"',
            'man1.install Dir["man/*.1"]',
            'bash_completion.install "completions/mrq.bash"',
            'zsh_completion.install "completions/_mrq"',
            'fish_completion.install "completions/mrq.fish"',
        ):
            self.assertIn(line, install)
        for archive_only in ("mrq.nu", "mrq.yaml", "mrq.elv", "_mrq.ps1"):
            self.assertNotIn(archive_only, self.formula)

    def test_test_block_checks_the_version(self):
        test = self.formula[self.formula.index("test do") :]
        self.assertIn('assert_match version.to_s, shell_output("#{bin}/mrq --version")', test)

    def test_ruby_string_escaping(self):
        meta = dict(self.meta, description='Say "hi" #{x} \\ ok')
        f = rf.render("v1.2.3", self.checksums, meta)
        self.assertIn('desc "Say \\"hi\\" \\#{x} \\\\ ok"', f)

    @unittest.skipUnless(shutil.which("ruby"), "ruby not installed")
    def test_formula_is_valid_ruby(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "marquee.rb"
            path.write_text(self.formula)
            result = subprocess.run(["ruby", "-c", str(path)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


class MainTest(unittest.TestCase):
    def run_main(self, env):
        return rf.main(env)

    def test_writes_the_formula(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = good_env(tmp)
            self.assertEqual(self.run_main(env), 0)
            text = pathlib.Path(env["OUTPUT_PATH"]).read_text()
        self.assertIn("class Marquee < Formula", text)
        self.assertIn('version "1.2.3"', text)

    def test_refuses_blank_checksums(self):
        for target in TARGETS:
            for blank in ("", "  ", "\n"):
                with tempfile.TemporaryDirectory() as tmp:
                    env = good_env(tmp)
                    env[rf.checksum_var(target)] = blank
                    self.assertNotEqual(self.run_main(env), 0, (target, blank))
                    self.assertFalse(os.path.exists(env["OUTPUT_PATH"]), target)

    def test_refuses_missing_or_malformed_checksums(self):
        for target in TARGETS:
            for bad in (None, "abc", sha(10).upper(), sha(1) + "0", "g" * 64):
                with tempfile.TemporaryDirectory() as tmp:
                    env = good_env(tmp)
                    if bad is None:
                        del env[rf.checksum_var(target)]
                    else:
                        env[rf.checksum_var(target)] = bad
                    self.assertNotEqual(self.run_main(env), 0, (target, bad))
                    self.assertFalse(os.path.exists(env["OUTPUT_PATH"]), target)

    def test_does_not_overwrite_an_existing_formula_on_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = good_env(tmp)
            out = pathlib.Path(env["OUTPUT_PATH"])
            out.parent.mkdir(parents=True)
            out.write_text("old")
            env[rf.checksum_var(TARGETS[0])] = ""
            self.assertNotEqual(self.run_main(env), 0)
            self.assertEqual(out.read_text(), "old")

    def test_refuses_prerelease_or_malformed_tags(self):
        for tag in ("v1.2.3-rc.1", "1.2.3", "v1.2", "", "v1.2.3\n"):
            with tempfile.TemporaryDirectory() as tmp:
                env = good_env(tmp)
                env["TAG"] = tag
                self.assertNotEqual(self.run_main(env), 0, tag)
                self.assertFalse(os.path.exists(env["OUTPUT_PATH"]), tag)

    def test_command_line_entry_point(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = dict(good_env(tmp), PATH=os.environ.get("PATH", ""))
            ok = subprocess.run(
                [sys.executable, "-I", str(HERE / "render_formula.py")],
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertEqual(ok.returncode, 0, ok.stderr)
            self.assertTrue(os.path.exists(env["OUTPUT_PATH"]))
            env[rf.checksum_var(TARGETS[2])] = ""
            os.remove(env["OUTPUT_PATH"])
            bad = subprocess.run(
                [sys.executable, "-I", str(HERE / "render_formula.py")],
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(bad.returncode, 0)
            self.assertIn(rf.checksum_var(TARGETS[2]), bad.stderr)
            self.assertFalse(os.path.exists(env["OUTPUT_PATH"]))


if __name__ == "__main__":
    unittest.main()
