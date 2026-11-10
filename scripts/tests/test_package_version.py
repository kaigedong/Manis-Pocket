"""Regression tests for the Arch pkgver resolver; no project dependencies."""

import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[2] / "packaging" / "package-version.sh"
FALLBACK_PATTERN = re.compile(r"^0\.0\.0\.r[0-9]+\.g[0-9a-f]{7}$")


class PackageVersionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name) / "repo"
        (self.repo / "packaging").mkdir(parents=True)
        shutil.copy(SCRIPT, self.repo / "packaging" / "package-version.sh")
        self.git("init", "-q")
        self.git("checkout", "-q", "-b", "master")
        self.commit("initial")

    def git(self, *arguments):
        return subprocess.run(
            ["git", "-C", str(self.repo),
             "-c", "user.name=Manis Pocket tests",
             "-c", "user.email=tests@example.com",
             "-c", "commit.gpgsign=false",
             "-c", "tag.gpgsign=false",
             *arguments],
            check=True, capture_output=True, text=True,
        ).stdout

    def commit(self, message):
        (self.repo / "file").write_text(message + "\n")
        self.git("add", "file")
        self.git("commit", "-q", "-m", message)

    def resolve(self, **environment):
        env = os.environ.copy()
        env.pop("RELEASE_VERSION", None)
        env.update(environment)
        return subprocess.run(
            ["bash", str(self.repo / "packaging" / "package-version.sh")],
            capture_output=True, text=True, env=env, cwd=str(self.repo),
        )

    def assert_version(self, expected):
        result = self.resolve()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, expected + "\n")

    def test_beta_release_tag_supplies_the_product_version(self):
        self.git("tag", "build-2.7.119")
        self.assert_version("2.7.119")

    def test_stable_release_tag_supplies_the_product_version(self):
        self.git("tag", "v2.8.0")
        self.assert_version("2.8.0")

    def test_release_version_drops_the_vcs_suffix(self):
        self.git("tag", "build-2.7.119")
        self.assertNotIn(".r", self.resolve().stdout)

    def test_tag_on_an_older_commit_is_ignored(self):
        self.git("tag", "build-2.7.119")
        self.commit("later change")
        result = self.resolve()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertRegex(result.stdout.strip(), FALLBACK_PATTERN)

    def test_stable_tag_on_an_older_commit_is_ignored(self):
        self.git("tag", "v2.8.0")
        self.commit("later change")
        result = self.resolve()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertRegex(result.stdout.strip(), FALLBACK_PATTERN)

    def test_untagged_build_falls_back_to_a_vcs_version(self):
        result = self.resolve()
        self.assertEqual(result.returncode, 0, result.stderr)
        fallback = result.stdout.strip()
        self.assertRegex(fallback, FALLBACK_PATTERN)
        revision = self.git("rev-list", "--count", "HEAD").strip()
        commit = self.git("rev-parse", "--short=7", "HEAD").strip()
        self.assertEqual(fallback, f"0.0.0.r{revision}.g{commit}")

    def test_explicit_release_version_wins_over_the_tag(self):
        self.git("tag", "build-2.7.119")
        result = self.resolve(RELEASE_VERSION="2.9.4")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "2.9.4\n")

    def test_invalid_explicit_release_version_fails(self):
        for value in ["2.7", "0.1.0.r1146.g0cbea45", "2.7.119-beta", "v2.7.1"]:
            with self.subTest(value=value):
                self.assertNotEqual(self.resolve(RELEASE_VERSION=value).returncode, 0)

    def test_ignores_unrelated_tags(self):
        self.git("tag", "latest-beta")
        self.git("tag", "wayland-0.1.0.r1146.g0cbea45")
        self.assertRegex(self.resolve().stdout.strip(), FALLBACK_PATTERN)


if __name__ == "__main__":
    unittest.main()
