#!/usr/bin/env python3
"""Resolve one version for all artifacts in a Build and Release workflow run."""

import argparse
import re


BASE_VERSION = "2.7.0"
VERSION_PATTERN = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")


def release_version(ref, run_number, event):
    if not re.fullmatch(r"[1-9][0-9]*", run_number):
        raise ValueError("run number must be a positive integer")
    if int(run_number) > 2_100_000_000:
        raise ValueError("run number exceeds Android's versionCode limit")
    if ref.startswith("refs/tags/v"):
        version = ref.removeprefix("refs/tags/v")
        if not VERSION_PATTERN.fullmatch(version):
            raise ValueError("stable tags must use vMAJOR.MINOR.PATCH")
        tag = f"v{version}"
        channel = "stable"
    else:
        major, minor, _ = BASE_VERSION.split(".")
        version = f"{major}.{minor}.{run_number}"
        tag = f"build-{version}"
        channel = "beta"
    publish = event == "push" and (
        ref == "refs/heads/master" or ref.startswith("refs/tags/v")
    )
    return {
        "version": version,
        "build_number": run_number,
        "tag": tag,
        "channel": channel,
        "publish": str(publish).lower(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ref", required=True)
    parser.add_argument("--run-number", required=True)
    parser.add_argument("--event", required=True)
    args = parser.parse_args()
    try:
        values = release_version(args.ref, args.run_number, args.event)
    except ValueError as error:
        parser.error(str(error))
    for name, value in values.items():
        print(f"{name}={value}")


if __name__ == "__main__":
    main()
