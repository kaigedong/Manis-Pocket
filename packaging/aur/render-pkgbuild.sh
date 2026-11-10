#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 3 || $# -gt 4 ]]; then
  echo "usage: $0 PKGVER SHA256 OUTPUT_DIRECTORY [RELEASE_TAG]" >&2
  exit 2
fi

version="$1"
checksum="$(printf '%s' "$2" | tr '[:upper:]' '[:lower:]')"
output_directory="$3"
# The asset lives on the product release of this version; the tag therefore
# depends on the channel (`build-2.7.120` for betas, `v2.8.0` for stable).
release_tag="${4:-build-$version}"
script_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ ! "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "AUR pkgver must be a product release version (MAJOR.MINOR.PATCH), got '$version'" >&2
  exit 1
fi
if [[ ! "$release_tag" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "invalid release tag '$release_tag'" >&2
  exit 1
fi
if [[ ! "$checksum" =~ ^[0-9a-f]{64}$ ]]; then
  echo "SHA-256 must contain exactly 64 hexadecimal characters" >&2
  exit 1
fi

mkdir -p "$output_directory"
sed \
  -e "s/@PKGVER@/$version/g" \
  -e "s/@RELEASE_TAG@/$release_tag/g" \
  -e "s/@SHA256@/$checksum/g" \
  "$script_directory/PKGBUILD.in" > "$output_directory/PKGBUILD"
