#!/usr/bin/env bash
# Tag and push a release; the GitHub `release` workflow does the rest.
# Usage: scripts/release.sh 0.1.0 [--dry-run]
#
# `--dry-run` builds the local-target binary + SHA256SUMS into $OUT_DIR
# (default /tmp/screendoc-release-dry) WITHOUT touching git or the network.
set -euo pipefail
TAG="${1:?usage: scripts/release.sh <version> [--dry-run]}"
DRY=0
[ "${2:-}" = "--dry-run" ] && DRY=1

cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [ "$DRY" = "1" ]; then
  OUT_DIR="${OUT_DIR:-/tmp/screendoc-release-dry}"
  rm -rf "$OUT_DIR"; mkdir -p "$OUT_DIR/bin"
  echo "==> building local-target release binary (dry-run)"
  cargo build --release
  cp target/release/screendoc "$OUT_DIR/bin/screendoc-$(uname -m)"
  ( cd "$OUT_DIR/bin" && sha256sum * > ../SHA256SUMS )
  echo "==> dry-run artifacts in $OUT_DIR:"
  ls "$OUT_DIR" "$OUT_DIR/bin"
  exit 0
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "error: working tree is dirty; commit or stash first" >&2
  exit 1
fi

if git rev-parse -q --verify "v${TAG}" >/dev/null 2>&1; then
  echo "error: tag v${TAG} already exists" >&2
  exit 1
fi

grep -q "^version = \"${TAG}\"" Cargo.toml \
  && echo "==> version in Cargo.toml is ${TAG}" \
  || { echo "note: version in Cargo.toml is not ${TAG} (bump it before tagging)" >&2; }

git tag "v${TAG}"
git push origin "v${TAG}"
echo "==> pushed v${TAG}; the release workflow will build and publish the artifacts"
