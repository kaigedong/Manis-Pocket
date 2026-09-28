#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 PKGVER SHA256 OUTPUT_DIRECTORY" >&2
  exit 2
fi

version="$1"
checksum="$(printf '%s' "$2" | tr '[:upper:]' '[:lower:]')"
output_directory="$3"
script_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.r[0-9]+\.g[0-9a-f]{7,40}$ ]]; then
  echo "invalid Manis Pocket Wayland package version" >&2
  exit 1
fi
if [[ ! "$checksum" =~ ^[0-9a-f]{64}$ ]]; then
  echo "SHA-256 must contain exactly 64 hexadecimal characters" >&2
  exit 1
fi

mkdir -p "$output_directory"
sed \
  -e "s/@PKGVER@/$version/g" \
  -e "s/@SHA256@/$checksum/g" \
  "$script_directory/PKGBUILD.in" > "$output_directory/PKGBUILD"
