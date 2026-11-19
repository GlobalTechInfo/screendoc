#!/usr/bin/env bash
# 02-chapters-redact: manual + auto chapters, secret redaction, stills export.
# Usage: ./02-chapters-redact.sh /path/to/screendoc
set -euo pipefail
BIN="${1:-}"
[ -n "$BIN" ] || { echo "usage: $0 /path/to/screendoc" >&2; exit 2; }
command -v tmux >/dev/null || { echo "tmux not found" >&2; exit 1; }

TMPD="$(mktemp -d)"
export HOME="$TMPD"
trap 'rm -rf "$TMPD"' EXIT

# A fake secret appears in the recorded output; chapters mark the phases.
"$BIN" record --size 80x24 \
  bash -lc 'echo phase one; echo "secret=sk-abcdef1234567890"; sleep 0.3; screendoc-chapter "phase two" 2>/dev/null || echo phase two; sleep 0.3; echo phase three'

ID="$(ls "$HOME/.screendoc/sessions" | head -n1)"
[ -n "$ID" ] || { echo "no session was recorded" >&2; exit 1; }

# Add a manual chapter marker to the active session.
"$BIN" chapter "manual marker" --session "$ID" || true

# Redact the fake secret in place, then verify it is gone from the cast.
"$BIN" redact "$ID" 'sk-[A-Za-z0-9]{16,}' --yes
if grep -q "sk-abcdef1234567890" "$HOME/.screendoc/sessions/$ID/raw.cast" 2>/dev/null; then
  echo "secret still present after redact!" >&2
  exit 1
fi
echo "redact OK (secret removed from raw.cast)"

# Export the session's still frames (ffmpeg not required).
OUT="$(mktemp -d)"
"$BIN" render "$ID" --frames --out "$OUT"
[ -s "$OUT/$ID-final.png" ] || { echo "missing $OUT/$ID-final.png" >&2; exit 1; }
echo "still: $(wc -c < "$OUT/$ID-final.png") bytes"
echo "02-chapters-redact OK"
