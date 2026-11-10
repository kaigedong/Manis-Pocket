"""Exercise publication offline with real local Git and a stateful gh double."""

import base64
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET


SCRIPT = Path(__file__).resolve().parents[1] / "publish-release.sh"
FAKE_GH = Path(__file__).resolve().parent / "fixtures" / "fake-gh.py"
SPARKLE = "{http://www.andymatuschak.org/xml-namespaces/sparkle}"


class PublishReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.checkout = self.root / "checkout"
        self.checkout.mkdir()
        self.env = os.environ.copy()
        self.env.update({
            "GIT_EDITOR": "true", "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": os.devnull, "GIT_TERMINAL_PROMPT": "0",
            "RELEASE_TAG": "build-1.2.3", "RELEASE_VERSION": "1.2.3",
            "RELEASE_CHANNEL": "beta", "RELEASE_BUILD_NUMBER": "123",
            "GITHUB_REPOSITORY": "offline/example",
        })
        self.git("init", "--bare", str(self.root / "origin.git"))
        self.git("init")
        self.git("config", "user.name", "Offline Test")
        self.git("config", "user.email", "test@example.invalid")
        self.git("remote", "add", "origin", str(self.root / "origin.git"))
        (self.checkout / "tracked").write_text("first")
        self.git("add", "tracked")
        self.git("commit", "-m", "first")
        self.sha = self.git("rev-parse", "HEAD").strip()
        self.env["GITHUB_SHA"] = self.sha
        self.bin = self.root / "bin"
        self.bin.mkdir()
        shutil.copyfile(FAKE_GH, self.bin / "gh")
        (self.bin / "gh").chmod(0o755)
        self.env["PATH"] = str(self.bin) + os.pathsep + self.env["PATH"]
        self.state_path = self.root / "gh.json"
        self.env["FAKE_GH_STATE"] = str(self.state_path)
        self.write_state({"calls": [], "releases": {}})
        (self.checkout / "ManisPocket.app.zip").write_bytes(b"original archive")
        (self.checkout / "app-debug.apk").write_bytes(b"android")
        (self.checkout / "release-feed").mkdir()
        self.write_feed()

    def git(self, *args):
        result = subprocess.run(["git", *args], cwd=self.checkout, env=self.env,
                                capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    @property
    def feed(self):
        name = "appcast-beta.xml" if self.env["RELEASE_CHANNEL"] == "beta" else "appcast.xml"
        return self.checkout / "release-feed" / name

    def write_feed(self, **overrides):
        attributes = {
            "url": f"https://github.com/offline/example/releases/download/{self.env['RELEASE_TAG']}/ManisPocket.app.zip",
            SPARKLE + "version": "123",
            SPARKLE + "shortVersionString": "1.2.3",
            "length": str((self.checkout / "ManisPocket.app.zip").stat().st_size),
        }
        attributes.update(overrides)
        rss = ET.Element("rss")
        item = ET.SubElement(ET.SubElement(rss, "channel"), "item")
        ET.SubElement(item, "enclosure", attributes)
        self.feed.write_bytes(ET.tostring(rss))

    def write_state(self, state):
        self.state_path.write_text(json.dumps(state))

    def state(self):
        return json.loads(self.state_path.read_text())

    def commands(self):
        return [call[1] for call in self.state()["calls"]]

    def run_publish(self, succeeds=True):
        result = subprocess.run(["bash", str(SCRIPT)], cwd=self.checkout, env=self.env,
                                capture_output=True, text=True, timeout=30)
        if succeeds:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def seed_release(self, draft=False, prerelease=True, omit=None):
        assets, contents = [], {}
        for path in (self.checkout / "ManisPocket.app.zip", self.checkout / "app-debug.apk", self.feed):
            if path.name == omit:
                continue
            content = path.read_bytes()
            assets.append({"name": path.name, "size": len(content)})
            contents[path.name] = base64.b64encode(content).decode()
        self.write_state({"calls": [], "releases": {self.env["RELEASE_TAG"]: {
            "isDraft": draft, "isPrerelease": prerelease,
            "assets": assets, "contents": contents,
        }}})

    def assert_remote_tag(self, sha):
        self.assertEqual(self.git("--git-dir", str(self.root / "origin.git"),
                                  "rev-parse", self.env["RELEASE_TAG"] + "^{commit}").strip(), sha)

    def test_first_beta_publication(self):
        self.run_publish()
        self.assert_remote_tag(self.sha)
        self.assertEqual(self.commands(), ["view", "create", "view", "upload", "view", "edit", "view", "download"])
        release = self.state()["releases"][self.env["RELEASE_TAG"]]
        self.assertFalse(release["isDraft"])
        self.assertTrue(release["isPrerelease"])
        edit = next(call for call in self.state()["calls"] if call[1] == "edit")
        self.assertIn("--latest=false", edit)

    def test_first_stable_publication_creates_tag(self):
        self.env.update(RELEASE_CHANNEL="stable", RELEASE_TAG="v1.2.3")
        self.write_feed()
        self.run_publish()
        self.assert_remote_tag(self.sha)
        self.assertEqual(self.commands(), ["view", "create", "view", "upload", "view", "edit", "view", "download"])
        self.assertFalse(self.state()["releases"]["v1.2.3"]["isPrerelease"])

    def test_new_stable_with_annotated_tag(self):
        self.env.update(RELEASE_CHANNEL="stable", RELEASE_TAG="v1.2.3")
        self.write_feed()
        self.git("tag", "-a", "v1.2.3", "-m", "stable")
        tag_object = self.git("rev-parse", "v1.2.3").strip()
        self.git("push", "origin", "refs/tags/v1.2.3")
        self.run_publish()
        self.assert_remote_tag(self.sha)
        self.assertEqual(self.git("ls-remote", "origin", "refs/tags/v1.2.3").split()[0], tag_object)
        release = self.state()["releases"]["v1.2.3"]
        self.assertFalse(release["isPrerelease"])
        edit = next(call for call in self.state()["calls"] if call[1] == "edit")
        self.assertIn("--latest=true", edit)

    def test_published_rerun_reuses_original_feed_without_mutation(self):
        self.run_publish()
        original_state = self.state()
        original_feed = self.feed.read_bytes()
        original_state["calls"] = []
        self.write_state(original_state)
        (self.checkout / "ManisPocket.app.zip").write_bytes(b"rebuilt archive of a different size")
        self.write_feed()
        self.run_publish()
        self.assertEqual(self.commands(), ["view", "download"])
        self.assertEqual(self.feed.read_bytes(), original_feed)
        self.assertEqual(self.state()["releases"], original_state["releases"])
        self.assert_remote_tag(self.sha)

    def test_incomplete_draft_can_be_completed(self):
        self.seed_release(draft=True, omit="app-debug.apk")
        self.run_publish()
        self.assertEqual(self.commands(), ["view", "upload", "view", "edit", "view", "download"])
        self.assertFalse(self.state()["releases"][self.env["RELEASE_TAG"]]["isDraft"])

    def test_bad_feed_is_rejected_before_draft_is_published(self):
        self.write_feed(length="999")
        result = self.run_publish(succeeds=False)
        self.assertIn("Published feed does not match", result.stderr)
        self.assertTrue(self.state()["releases"][self.env["RELEASE_TAG"]]["isDraft"])
        self.assertNotIn("edit", self.commands())

    def test_existing_tag_wrong_sha_is_rejected_without_mutation(self):
        self.git("tag", self.env["RELEASE_TAG"])
        self.git("push", "origin", "refs/tags/" + self.env["RELEASE_TAG"])
        (self.checkout / "tracked").write_text("second")
        self.git("commit", "-am", "second")
        self.env["GITHUB_SHA"] = self.git("rev-parse", "HEAD").strip()
        self.run_publish(succeeds=False)
        self.assert_remote_tag(self.sha)
        self.assertEqual(self.commands(), [])

    def test_published_missing_asset_is_rejected(self):
        self.seed_release(omit="app-debug.apk")
        result = self.run_publish(succeeds=False)
        self.assertIn("Published release is incomplete: app-debug.apk", result.stderr)
        self.assertEqual(self.commands(), ["view"])

    def test_published_wrong_channel_is_rejected(self):
        self.seed_release(prerelease=False)
        result = self.run_publish(succeeds=False)
        self.assertIn("does not match the requested channel", result.stderr)
        self.assertEqual(self.commands(), ["view"])

    def test_published_feed_version_or_length_mismatch_is_rejected(self):
        for attribute, value in ((SPARKLE + "version", "999"),
                                 (SPARKLE + "shortVersionString", "9.9.9"),
                                 ("length", "999")):
            with self.subTest(attribute=attribute):
                self.write_feed(**{attribute: value})
                self.seed_release()
                result = self.run_publish(succeeds=False)
                self.assertIn("Published feed does not match", result.stderr)
                self.assertEqual(self.commands(), ["view", "download"])


if __name__ == "__main__":
    unittest.main()
