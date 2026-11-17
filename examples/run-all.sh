#!/usr/bin/env bash
# run-all: build the release binary and run every example against it.
# Usage: ./run-all.sh [extra cargo build args…]
set -euo pipefail
cd "$(dirname "$0")/.."

BIN_ARGS=(--release "$@")
echo "== building screendoc (cargo build ${BIN_ARGS[*]}) =="
cargo build "${BIN_ARGS[@]}"
BIN="$PWD/target/release/screendoc"

command -v tmux >/dev/null || { echo "SKIP: tmux not installed" >&2; exit 0; }
FFMPEG=0
command -v ffmpeg >/dev/null && FFMPEG=1
[ "$FFMPEG" = 1 ] || echo "note: ffmpeg not found — video examples still run, only stills/cast checked"

for ex in 01-record-render 02-chapters-redact 03-web-serve 04-encrypt; do
  echo
  echo "== $ex =="
  "./examples/$ex.sh" "$BIN"
done

echo
echo "all examples OK"
