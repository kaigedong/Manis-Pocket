#!/usr/bin/env bash
set -euo pipefail

# Only called by the release job after both platform artifacts are available.
: "${RELEASE_TAG:?}" "${RELEASE_VERSION:?}" "${RELEASE_CHANNEL:?}"
: "${GITHUB_SHA:?}" "${GITHUB_REPOSITORY:?}" "${RELEASE_BUILD_NUMBER:?}"

if [[ ! "$RELEASE_TAG" =~ ^(build-|v)[0-9]+\.[0-9]+\.[0-9]+$ ||
      ! "$RELEASE_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ||
      ! "$RELEASE_CHANNEL" =~ ^(beta|stable)$ ]]; then
  echo "Invalid release metadata" >&2
  exit 1
fi
if [[ "$RELEASE_CHANNEL" == beta ]]; then
  FEED_NAME=appcast-beta.xml
  PRERELEASE=true
  LATEST=false
  test "$RELEASE_TAG" == "build-$RELEASE_VERSION"
else
  FEED_NAME=appcast.xml
  PRERELEASE=false
  LATEST=true
  test "$RELEASE_TAG" == "v$RELEASE_VERSION"
fi

ASSETS=(ManisPocket.app.zip app-debug.apk "release-feed/$FEED_NAME")
for asset in "${ASSETS[@]}"; do
  test -s "$asset"
done

# Never move an existing version tag, including annotated stable tags.
if git ls-remote --exit-code origin "refs/tags/$RELEASE_TAG"; then
  :
else
  STATUS=$?
  if [[ "$STATUS" != 2 ]]; then
    exit "$STATUS"
  fi
  git push origin "$GITHUB_SHA:refs/tags/$RELEASE_TAG"
fi
git fetch --no-tags origin "refs/tags/$RELEASE_TAG"
test "$(git rev-parse 'FETCH_HEAD^{commit}')" == "$GITHUB_SHA"

RELEASE_JSON="$(mktemp)"
trap 'rm -f "$RELEASE_JSON"' EXIT
if ! gh release view "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" \
    --json isDraft,isPrerelease,assets > "$RELEASE_JSON"; then
  OPTIONS=(--draft --verify-tag)
  if [[ "$PRERELEASE" == true ]]; then
    OPTIONS+=(--prerelease)
  fi
  gh release create "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" \
    --title "Manis Pocket $RELEASE_VERSION" \
    --notes "Automated $RELEASE_CHANNEL build of $GITHUB_SHA." "${OPTIONS[@]}"
  gh release view "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" \
    --json isDraft,isPrerelease,assets > "$RELEASE_JSON"
fi

verify_release_assets() {
  python3 - "$RELEASE_JSON" "$PRERELEASE" "$FEED_NAME" "$1" <<'PY'
import json
import sys

with open(sys.argv[1]) as stream:
    release = json.load(stream)
if release["isDraft"] != (sys.argv[4] == "true") or release["isPrerelease"] != (sys.argv[2] == "true"):
    raise SystemExit("Existing release does not match the requested channel")
assets = {asset["name"]: asset for asset in release["assets"]}
for name in ("ManisPocket.app.zip", "app-debug.apk", sys.argv[3]):
    if name not in assets or assets[name]["size"] <= 0:
        raise SystemExit(f"Published release is incomplete: {name}")
PY
}

verify_release_feed() {
  python3 - "$RELEASE_JSON" "release-feed/$FEED_NAME" <<'PY'
import json
import os
import sys
import xml.etree.ElementTree as ET

sparkle = "{http://www.andymatuschak.org/xml-namespaces/sparkle}"
with open(sys.argv[1]) as stream:
    assets = {asset["name"]: asset for asset in json.load(stream)["assets"]}
enclosure = ET.parse(sys.argv[2]).find("./channel/item/enclosure")
expected_url = (f"https://github.com/{os.environ['GITHUB_REPOSITORY']}/releases/download/"
                f"{os.environ['RELEASE_TAG']}/ManisPocket.app.zip")
expected = {
    "url": expected_url,
    sparkle + "version": os.environ["RELEASE_BUILD_NUMBER"],
    sparkle + "shortVersionString": os.environ["RELEASE_VERSION"],
    "length": str(assets["ManisPocket.app.zip"]["size"]),
}
if enclosure is None or any(enclosure.get(key) != value for key, value in expected.items()):
    raise SystemExit("Published feed does not match its versioned release assets")
PY
}

IS_DRAFT="$(python3 -c 'import json,sys; print(str(json.load(open(sys.argv[1]))["isDraft"]).lower())' "$RELEASE_JSON")"
if [[ "$IS_DRAFT" == true ]]; then
  # Validate uploaded assets and metadata BEFORE exposing a draft to users.
  gh release upload "$RELEASE_TAG" "${ASSETS[@]}" --clobber --repo "$GITHUB_REPOSITORY"
  gh release view "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" \
    --json isDraft,isPrerelease,assets > "$RELEASE_JSON"
  verify_release_assets true
  verify_release_feed
  gh release edit "$RELEASE_TAG" --draft=false --prerelease="$PRERELEASE" \
    --latest="$LATEST" --repo "$GITHUB_REPOSITORY"
  gh release view "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" \
    --json isDraft,isPrerelease,assets > "$RELEASE_JSON"
fi

verify_release_assets false
# On rerun, use the ORIGINAL published feed, not a newly rebuilt archive's size.
gh release download "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" \
  --pattern "$FEED_NAME" --dir release-feed --clobber
verify_release_feed
