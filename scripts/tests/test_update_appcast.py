import importlib.util
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET


SCRIPT = Path(__file__).resolve().parents[1] / "update-appcast.py"
SPEC = importlib.util.spec_from_file_location("update_appcast", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class UpdateAppcastTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.source = Path(self.temp.name) / "incoming.xml"
        self.destination = Path(self.temp.name) / "appcast-beta.xml"

    def feed(self, path, number=None, url="https://example.com/archive.zip"):
        root = ET.Element("rss", {"version": "2.0"})
        channel = ET.SubElement(root, "channel")
        if number is not None:
            item = ET.SubElement(channel, "item")
            ET.SubElement(item, "enclosure", {
                MODULE.VERSION_ATTRIBUTE: str(number), "url": url,
            })
        ET.ElementTree(root).write(path, encoding="utf-8", xml_declaration=True)

    def test_first_release_initializes_empty_feed(self):
        self.feed(self.source, 153)
        self.feed(self.destination)
        self.assertTrue(MODULE.update_feed(self.source, self.destination))
        self.assertEqual(self.source.read_bytes(), self.destination.read_bytes())

    def test_first_release_initializes_missing_feed(self):
        self.feed(self.source, 153)
        self.assertTrue(MODULE.update_feed(self.source, self.destination))

    def test_newer_release_advances_feed(self):
        self.feed(self.source, 154)
        self.feed(self.destination, 153)
        self.assertTrue(MODULE.update_feed(self.source, self.destination))
        self.assertEqual(MODULE.build_number(self.destination), 154)

    def test_old_rerun_does_not_downgrade(self):
        self.feed(self.source, 153)
        self.feed(self.destination, 154)
        before = self.destination.read_bytes()
        self.assertFalse(MODULE.update_feed(self.source, self.destination))
        self.assertEqual(self.destination.read_bytes(), before)

    def test_same_published_feed_is_idempotent(self):
        self.feed(self.source, 153)
        self.feed(self.destination, 153)
        self.assertFalse(MODULE.update_feed(self.source, self.destination))

    def test_same_build_different_metadata_is_rejected(self):
        self.feed(self.source, 153, "https://example.com/new.zip")
        self.feed(self.destination, 153)
        before = self.destination.read_bytes()
        with self.assertRaises(ValueError):
            MODULE.update_feed(self.source, self.destination)
        self.assertEqual(self.destination.read_bytes(), before)

    def test_invalid_source_does_not_change_existing_feed(self):
        self.feed(self.destination, 153)
        before = self.destination.read_bytes()
        for number in [None, 0, -1, "001", "bad"]:
            with self.subTest(number=number):
                self.feed(self.source, number)
                with self.assertRaises(ValueError):
                    MODULE.update_feed(self.source, self.destination)
                self.assertEqual(self.destination.read_bytes(), before)

    def test_corrupt_destination_is_not_silently_replaced(self):
        self.feed(self.source, 153)
        self.destination.write_text("not XML")
        with self.assertRaises(ET.ParseError):
            MODULE.update_feed(self.source, self.destination)
        self.assertEqual(self.destination.read_text(), "not XML")


if __name__ == "__main__":
    unittest.main()
