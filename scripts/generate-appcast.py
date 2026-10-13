#!/usr/bin/env python3
"""Generate a single-release Sparkle appcast using only the Python standard library."""

import argparse
from datetime import datetime, timezone
from email.utils import format_datetime
from pathlib import Path
import re
from urllib.parse import quote
import xml.etree.ElementTree as ET


SPARKLE = "http://www.andymatuschak.org/xml-namespaces/sparkle"
ET.register_namespace("sparkle", SPARKLE)


def matching(pattern, description):
    def validate(value):
        if not re.fullmatch(pattern, value, flags=re.ASCII):
            raise argparse.ArgumentTypeError(f"invalid {description}: {value!r}")
        return value
    return validate


def build_number(value):
    if not re.fullmatch(r"[0-9]+", value) or int(value) <= 0:
        raise argparse.ArgumentTypeError("build number must be a positive integer")
    return str(int(value))


def repository(value):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9][A-Za-z0-9_.-]*", value):
        raise argparse.ArgumentTypeError("repository must be owner/name")
    if ".." in value or value.endswith("."):
        raise argparse.ArgumentTypeError("repository must not contain traversal segments")
    return value


def tag(value):
    if (not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", value)
            or ".." in value or value.endswith((".", ".lock"))):
        raise argparse.ArgumentTypeError("tag must be a safe release tag (letters, digits, ._-)")
    return value


def published_at(value):
    try:
        result = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise argparse.ArgumentTypeError("published-at must be an ISO8601 timestamp") from error
    if result.tzinfo is None:
        raise argparse.ArgumentTypeError("published-at must include a timezone")
    return result.astimezone(timezone.utc)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, type=matching(
        r"[0-9]+(?:\.[0-9]+)*(?:-[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*)?"
        r"(?:\+[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*)?", "version"))
    parser.add_argument("--build-number", required=True, type=build_number)
    parser.add_argument("--tag", required=True, type=tag)
    parser.add_argument("--repository", required=True, type=repository)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--channel", required=True, choices=("stable", "beta"))
    parser.add_argument("--published-at", type=published_at)
    parser.add_argument("--minimum-system-version", default="14.0", type=matching(
        r"[0-9]+(?:\.[0-9]+)*", "minimum system version"))
    args = parser.parse_args(argv)

    if not args.archive.is_file() or args.archive.suffix.lower() != ".zip":
        parser.error("--archive must be an existing .zip file")
    try:
        length = args.archive.stat().st_size
    except OSError as error:
        parser.error(f"cannot inspect archive: {error}")

    timestamp = args.published_at or datetime.now(timezone.utc)
    feed_name = "appcast.xml" if args.channel == "stable" else "appcast-beta.xml"
    brand = "Manis Pocket" if args.channel == "stable" else "Manis Pocket Beta"
    rss = ET.Element("rss", {"version": "2.0"})
    channel = ET.SubElement(rss, "channel")
    ET.SubElement(channel, "title").text = brand
    ET.SubElement(channel, "link").text = (
        f"https://raw.githubusercontent.com/{args.repository}/master/{feed_name}")
    ET.SubElement(channel, "description").text = f"Manis Pocket {args.channel} updates."
    ET.SubElement(channel, "language").text = "en"
    item = ET.SubElement(channel, "item")
    ET.SubElement(item, "title").text = f"{brand} {args.version}"
    ET.SubElement(item, "description").text = f"{brand} {args.version}."
    ET.SubElement(item, "pubDate").text = format_datetime(timestamp, usegmt=True)
    ET.SubElement(item, f"{{{SPARKLE}}}minimumSystemVersion").text = args.minimum_system_version
    ET.SubElement(item, "enclosure", {
        "url": (f"https://github.com/{args.repository}/releases/download/"
                f"{quote(args.tag, safe='')}/ManisPocket.app.zip"),
        f"{{{SPARKLE}}}version": args.build_number,
        f"{{{SPARKLE}}}shortVersionString": args.version,
        "length": str(length),
        "type": "application/octet-stream",
    })
    ET.indent(rss, space="  ")
    try:
        ET.ElementTree(rss).write(args.output, encoding="utf-8", xml_declaration=True)
    except OSError as error:
        parser.error(f"cannot write appcast: {error}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
