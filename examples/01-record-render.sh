#!/usr/bin/env bash
# 01-record-render: record a command in wrapper mode, render GIF + MP4.
# Usage: ./01-record-render.sh /path/to/screendoc
set -euo pipefail
BIN="${1:-}"
[ -n "$BIN" ] || { echo "usage: $0 /path/to/screendoc" >&2; exit 2; }
command -v tmux >/dev/null || { echo "tmux not found" >&2; exit 1; }

TMPD="$(mktemp -d)"
export HOME="$TMPD"
trap 'rm -rf "$TMPD"' EXIT

# Record a short session: 80x24 terminal, a few commands with output.
"$BIN" record --size 80x24 \
  bash -lc 'echo step one; sleep 0.4; echo step two; sleep 0.4; echo done'

# The store is isolated to this temp HOME, so exactly one session exists.
ID="$(ls "$HOME/.screendoc/sessions" | head -n1)"
[ -n "$ID" ] || { echo "no session was recorded" >&2; exit 1; }

OUT="$(mktemp -d)"
"$BIN" render "$ID" --gif --mp4 --out "$OUT"

[ -s "$OUT/$ID.gif" ] || { echo "missing $OUT/$ID.gif" >&2; exit 1; }
[ -s "$OUT/$ID.mp4" ] || { echo "missing $OUT/$ID.mp4" >&2; exit 1; }

echo "gif : $(wc -c < "$OUT/$ID.gif") bytes"
echo "mp4 : $(wc -c < "$OUT/$ID.mp4") bytes"
echo "01-record-render OK"
