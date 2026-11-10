#!/usr/bin/env bash
# Resolve the pkgver for the Arch Linux package.
#
# A release build reuses the product version that the Build and Release workflow
# published for the same commit (tag `build-X.Y.Z` for betas, `vX.Y.Z` for
# stable releases), so every artifact of one release carries the same number.
# Builds that are not attached to a release (pull requests, manual dispatches)
# fall back to a VCS-style version, because makepkg still needs a unique and
# monotonically increasing pkgver for those throwaway packages.
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version_pattern='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'

release_version=""
if [[ -n "${RELEASE_VERSION:-}" ]]; then
  if [[ ! "$RELEASE_VERSION" =~ $version_pattern ]]; then
    echo "RELEASE_VERSION must be MAJOR.MINOR.PATCH, got '$RELEASE_VERSION'" >&2
    exit 1
  fi
  release_version="$RELEASE_VERSION"
else
  # Only the checked-out commit counts: a release tag describes this exact
  # build, never a neighbour in history.
  while IFS= read -r tag; do
    case "$tag" in
      build-*) candidate="${tag#build-}" ;;
      v*) candidate="${tag#v}" ;;
      *) continue ;;
    esac
    if [[ "$candidate" =~ $version_pattern ]]; then
      release_version="$candidate"
      break
    fi
  done < <(git -C "$repository" tag --points-at HEAD)
fi

if [[ -n "$release_version" ]]; then
  printf '%s\n' "$release_version"
  exit 0
fi

revision="$(git -C "$repository" rev-list --count HEAD)"
commit="$(git -C "$repository" rev-parse --short=7 HEAD)"
printf '0.0.0.r%s.g%s\n' "$revision" "$commit"
