#!/usr/bin/env python3
"""Check dyld startup of the packaged app without opening UI or touching user data."""

import argparse
from pathlib import Path
import subprocess
import sys


SUCCESS_MARKER = "Manis Pocket startup smoke test passed"


def check_startup(app, timeout=20):
    executable = app / "Contents" / "MacOS" / "ManisPocket"
    if not executable.is_file():
        raise ValueError(f"Missing packaged executable: {executable}")
    try:
        result = subprocess.run(
            [str(executable.resolve()), "--startup-smoke-test"],
            capture_output=True, text=True, timeout=timeout,
        )
    except subprocess.TimeoutExpired as error:
        raise ValueError("Packaged app startup timed out") from error
    except OSError as error:
        raise ValueError(f"Cannot execute packaged app: {error}") from error
    if result.returncode != 0 or SUCCESS_MARKER not in result.stdout.splitlines():
        raise ValueError(
            f"Packaged app failed startup (exit {result.returncode})\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", required=True, type=Path)
    args = parser.parse_args()
    try:
        check_startup(args.app)
    except ValueError as error:
        print(str(error), file=sys.stderr)
        return 1
    print(SUCCESS_MARKER)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
