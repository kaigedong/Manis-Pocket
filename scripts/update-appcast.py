#!/usr/bin/env python3
"""Advance a channel feed only when its published build is newer."""

import argparse
from pathlib import Path
import re
import shutil
import xml.etree.ElementTree as ET


VERSION_ATTRIBUTE = "{http://www.andymatuschak.org/xml-namespaces/sparkle}version"


def build_number(path, allow_empty=False):
    enclosures = ET.parse(path).findall("./channel/item/enclosure")
    if not enclosures and allow_empty:
        return 0
    if len(enclosures) != 1:
        raise ValueError(f"{path}: expected a single-release feed")
    value = enclosures[0].get(VERSION_ATTRIBUTE, "")
    if not re.fullmatch(r"[1-9][0-9]*", value):
        raise ValueError(f"{path}: missing or invalid Sparkle build number")
    return int(value)


def update_feed(source, destination):
    incoming = build_number(source)
    current = build_number(destination, allow_empty=True) if destination.exists() else 0
    if incoming < current:
        return False
    if incoming == current:
        if source.read_bytes() != destination.read_bytes():
            raise ValueError("Same build number already has different published metadata")
        return False
    shutil.copyfile(source, destination)
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--destination", required=True, type=Path)
    args = parser.parse_args()
    try:
        changed = update_feed(args.source, args.destination)
    except (OSError, ValueError, ET.ParseError) as error:
        parser.error(str(error))
    print("Updated channel feed" if changed else "Channel feed is already current or newer")


if __name__ == "__main__":
    main()
