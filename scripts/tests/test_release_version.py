import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "release-version.py"
SPEC = importlib.util.spec_from_file_location("release_version", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ReleaseVersionTests(unittest.TestCase):
    def test_master_uses_workflow_run_number(self):
        result = MODULE.release_version("refs/heads/master", "153", "push")
        self.assertEqual(result, {
            "version": "2.7.153",
            "build_number": "153",
            "tag": "build-2.7.153",
            "channel": "beta",
            "publish": "true",
        })

    def test_new_run_increments_version(self):
        first = MODULE.release_version("refs/heads/master", "153", "push")
        second = MODULE.release_version("refs/heads/master", "154", "push")
        self.assertEqual(second["version"], "2.7.154")
        self.assertNotEqual(first["tag"], second["tag"])

    def test_rerun_is_deterministic(self):
        first = MODULE.release_version("refs/heads/master", "153", "push")
        self.assertEqual(first, MODULE.release_version("refs/heads/master", "153", "push"))

    def test_stable_tag_controls_display_version(self):
        result = MODULE.release_version("refs/tags/v2.8.0", "154", "push")
        self.assertEqual(result["version"], "2.8.0")
        self.assertEqual(result["tag"], "v2.8.0")
        self.assertEqual(result["channel"], "stable")
        self.assertEqual(result["build_number"], "154")
        self.assertEqual(result["publish"], "true")

    def test_branch_pr_and_dispatch_do_not_publish(self):
        cases = [
            ("refs/heads/feat/p2p-sync", "push"),
            ("refs/pull/12/merge", "pull_request"),
            ("refs/heads/master", "workflow_dispatch"),
            ("refs/tags/v2.8.0", "workflow_dispatch"),
        ]
        for ref, event in cases:
            with self.subTest(ref=ref, event=event):
                self.assertEqual(MODULE.release_version(ref, "154", event)["publish"], "false")

    def test_invalid_or_prerelease_stable_tags_fail(self):
        for ref in ["refs/tags/v2.8", "refs/tags/v2.8.0-beta", "refs/tags/v02.8.0",
                    "refs/tags/v2.8.0\npublish=true"]:
            with self.subTest(ref=ref), self.assertRaises(ValueError):
                MODULE.release_version(ref, "154", "push")

    def test_invalid_build_numbers_fail(self):
        for number in ["0", "-1", "001", "1.5", "154\npublish=true", "2100000001"]:
            with self.subTest(number=number), self.assertRaises(ValueError):
                MODULE.release_version("refs/heads/master", number, "push")

    def test_cli_emits_github_output_format(self):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--ref", "refs/heads/master",
             "--run-number", "153", "--event", "push"],
            check=True, capture_output=True, text=True,
        )
        self.assertEqual(result.stdout.splitlines(), [
            "version=2.7.153", "build_number=153", "tag=build-2.7.153",
            "channel=beta", "publish=true",
        ])

    def test_invalid_cli_exits_without_outputs(self):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--ref", "refs/tags/vinvalid",
             "--run-number", "153", "--event", "push"],
            capture_output=True, text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
