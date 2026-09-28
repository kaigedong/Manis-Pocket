#!/usr/bin/env bash
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
base_version="$(awk -F '"' '/^version = / { print $2; exit }' "$repository/crates/manis-pocket-wayland/Cargo.toml")"
revision="$(git -C "$repository" rev-list --count HEAD)"
commit="$(git -C "$repository" rev-parse --short=7 HEAD)"

printf '%s.r%s.g%s\n' "$base_version" "$revision" "$commit"
