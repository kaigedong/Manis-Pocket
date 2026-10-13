"""Standalone CLI regression tests; no project or third-party dependencies."""

from datetime import datetime, timezone
from email.utils import parsedate_to_datetime
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
import zipfile


SCRIPT = Path(__file__).resolve().parents[1] / "generate-appcast.py"
SPARKLE = "{http://www.andymatuschak.org/xml-namespaces/sparkle}"


class GenerateAppcastTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.archive = self.directory / "ManisPocket.app.zip"
        with zipfile.ZipFile(self.archive, "w") as archive:
            archive.writestr("ManisPocket.app/Contents/example", "actual archive bytes")
        self.output = self.directory / "appcast.xml"

    def run_cli(self, **overrides):
        options = {
            "version": "1.2.3-beta.4+build.5",
            "build-number": "1234",
            "tag": "v1.2.3-beta.4",
            "repository": "kaigedong/Manis-Pocket",
            "archive": str(self.archive),
            "output": str(self.output),
            "channel": "stable",
            "published-at": "2026-06-10T18:30:45+08:00",
        }
        options.update(overrides)
        command = [sys.executable, str(SCRIPT)]
        for name, value in options.items():
            if value is not None:
                command.extend([f"--{name}", value])
        return subprocess.run(command, capture_output=True, text=True)

    def test_stable_and_beta_metadata(self):
        for channel, feed, title in (
                ("stable", "appcast.xml", "Manis Pocket"),
                ("beta", "appcast-beta.xml", "Manis Pocket Beta")):
            with self.subTest(channel=channel):
                result = self.run_cli(channel=channel)
                self.assertEqual(result.returncode, 0, result.stderr)
                root = ET.parse(self.output).getroot()
                self.assertEqual(root.tag, "rss")
                self.assertEqual(root.attrib["version"], "2.0")
                self.assertEqual(root.findtext("channel/title"), title)
                self.assertEqual(root.findtext("channel/link"),
                                 f"https://raw.githubusercontent.com/kaigedong/Manis-Pocket/master/{feed}")
                item = root.find("channel/item")
                self.assertEqual(item.findtext("pubDate"), "Wed, 10 Jun 2026 10:30:45 GMT")
                self.assertEqual(item.findtext(SPARKLE + "minimumSystemVersion"), "14.0")
                enclosure = item.find("enclosure")
                self.assertEqual(enclosure.attrib[SPARKLE + "version"], "1234")
                self.assertEqual(enclosure.attrib[SPARKLE + "shortVersionString"],
                                 "1.2.3-beta.4+build.5")
                self.assertEqual(enclosure.attrib["length"], str(self.archive.stat().st_size))
                self.assertEqual(enclosure.attrib["url"],
                                 "https://github.com/kaigedong/Manis-Pocket/releases/download/"
                                 "v1.2.3-beta.4/ManisPocket.app.zip")
                self.assertEqual(enclosure.attrib["type"], "application/octet-stream")

    def test_default_date_and_custom_minimum(self):
        before = datetime.now(timezone.utc).replace(microsecond=0)
        result = self.run_cli(**{"published-at": None, "minimum-system-version": "15.1"})
        after = datetime.now(timezone.utc)
        self.assertEqual(result.returncode, 0, result.stderr)
        item = ET.parse(self.output).find("channel/item")
        timestamp = parsedate_to_datetime(item.findtext("pubDate"))
        self.assertLessEqual(before, timestamp)
        self.assertLessEqual(timestamp, after)
        self.assertEqual(item.findtext(SPARKLE + "minimumSystemVersion"), "15.1")

    def test_validation_failures(self):
        invalid = {
            "build-number": ["0", "-1", "1.5", "abc"],
            "repository": ["https://github.com/a/b", "a/b?x=1", "a/../b", "a/b&x", "a/b\n"],
            "version": ["", "1.2?<x>", "1.2/evil", "1.2\n"],
            "tag": ["../evil", "v1?x=1", "v1#fragment", 'v1"evil', "v1\n"],
            "archive": [str(self.directory / "missing.zip"), str(self.directory)],
            "published-at": ["yesterday", "2026-06-10T12:00:00"],
            "channel": ["nightly"],
            "minimum-system-version": ["14.0<x>"],
        }
        for name, values in invalid.items():
            for value in values:
                with self.subTest(argument=name, value=value):
                    result = self.run_cli(**{name: value})
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("error:", result.stderr)
                    self.assertFalse(self.output.exists())

    def test_required_arguments(self):
        for name in ("version", "build-number", "tag", "repository", "archive", "output", "channel"):
            with self.subTest(argument=name):
                result = self.run_cli(**{name: None})
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"--{name}", result.stderr)


if __name__ == "__main__":
    unittest.main()
