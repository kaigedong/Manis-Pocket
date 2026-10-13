import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "check-macos-startup.py"
SPEC = importlib.util.spec_from_file_location("check_macos_startup", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class MacOSStartupTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.app = Path(self.temp.name) / "ManisPocket.app"
        self.executable = self.app / "Contents" / "MacOS" / "ManisPocket"
        self.executable.parent.mkdir(parents=True)
        self.executable.touch()

    def test_success_requires_diagnostic_entry_point(self):
        result = subprocess.CompletedProcess([], 0, MODULE.SUCCESS_MARKER + "\n", "")
        with patch.object(MODULE.subprocess, "run", return_value=result) as run:
            MODULE.check_startup(self.app)
        run.assert_called_once_with(
            [str(self.executable.resolve()), "--startup-smoke-test"],
            capture_output=True, text=True, timeout=20,
        )

    def test_dyld_abort_is_reported_even_with_success_marker(self):
        result = subprocess.CompletedProcess(
            [], -6, MODULE.SUCCESS_MARKER + "\n", "dyld: different Team IDs",
        )
        with patch.object(MODULE.subprocess, "run", return_value=result):
            with self.assertRaisesRegex(ValueError, "different Team IDs"):
                MODULE.check_startup(self.app)

    def test_zero_exit_without_marker_is_not_a_pass(self):
        result = subprocess.CompletedProcess([], 0, "", "")
        with patch.object(MODULE.subprocess, "run", return_value=result):
            with self.assertRaisesRegex(ValueError, "failed startup"):
                MODULE.check_startup(self.app)

    def test_missing_executable_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Missing packaged executable"):
            MODULE.check_startup(self.app / "missing")

    def test_timeout_is_reported(self):
        with patch.object(MODULE.subprocess, "run", side_effect=subprocess.TimeoutExpired([], 20)):
            with self.assertRaisesRegex(ValueError, "timed out"):
                MODULE.check_startup(self.app)

    def test_execution_permission_error_is_reported(self):
        with patch.object(MODULE.subprocess, "run", side_effect=PermissionError("not executable")):
            with self.assertRaisesRegex(ValueError, "Cannot execute"):
                MODULE.check_startup(self.app)


if __name__ == "__main__":
    unittest.main()
