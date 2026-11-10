#!/usr/bin/env python3
"""Offline gh release double. State and call log live in FAKE_GH_STATE."""

import base64
import json
import os
from pathlib import Path
import sys


path = Path(os.environ["FAKE_GH_STATE"])
state = json.loads(path.read_text())
args = sys.argv[1:]
state["calls"].append(args)


def save():
    path.write_text(json.dumps(state))


def option(name):
    return args[args.index(name) + 1]


if len(args) < 3 or args[0] != "release":
    raise SystemExit("Only gh release is supported")
command, tag = args[1:3]
release = state["releases"].get(tag)
if command == "view":
    save()
    if release is None:
        raise SystemExit(1)
    print(json.dumps({key: release[key] for key in ("isDraft", "isPrerelease", "assets")}))
elif command == "create":
    if release is not None:
        raise SystemExit("Release already exists")
    state["releases"][tag] = {
        "isDraft": "--draft" in args,
        "isPrerelease": "--prerelease" in args,
        "assets": [],
        "contents": {},
    }
    save()
elif command == "upload":
    for filename in args[3:args.index("--clobber")]:
        asset = Path(filename)
        content = asset.read_bytes()
        release["assets"] = [entry for entry in release["assets"] if entry["name"] != asset.name]
        release["assets"].append({"name": asset.name, "size": len(content)})
        release["contents"][asset.name] = base64.b64encode(content).decode()
    save()
elif command == "edit":
    release["isDraft"] = "--draft=false" not in args
    release["isPrerelease"] = "--prerelease=true" in args
    save()
elif command == "download":
    name = option("--pattern")
    directory = Path(option("--dir"))
    directory.mkdir(parents=True, exist_ok=True)
    (directory / name).write_bytes(base64.b64decode(release["contents"][name]))
    save()
else:
    raise SystemExit(f"Unsupported release command: {command}")
