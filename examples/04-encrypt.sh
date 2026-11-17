#!/usr/bin/env bash
# 04-encrypt: encrypt a session at rest (AES-256-GCM), then render from it.
# Usage: ./04-encrypt.sh /path/to/screendoc
set -euo pipefail
BIN="${1:-}"
[ -n "$BIN" ] || { echo "usage: $0 /path/to/screendoc" >&2; exit 2; }
command -v tmux >/dev/null || { echo "tmux not found" >&2; exit 1; }

TMPD="$(mktemp -d)"
export HOME="$TMPD"
trap 'rm -rf "$TMPD"' EXIT

"$BIN" record --encrypt --size 80x24 \
  bash -lc 'echo confidential step one; sleep 0.3; echo confidential step two'

ID="$(ls "$HOME/.screendoc/sessions" | head -n1)"
[ -n "$ID" ] || { echo "no session was recorded" >&2; exit 1; }

# The on-disk stream must NOT contain the plaintext; the key file must exist.
if grep -rq "confidential step one" "$HOME/.screendoc/sessions/$ID/" 2>/dev/null; then
  echo "plaintext leaked into the encrypted session dir!" >&2
  exit 1
fi
[ -f "$HOME/.screendoc/key" ] || { echo "missing key file" >&2; exit 1; }
echo "at-rest encryption OK (no plaintext on disk, key present)"

# The renderer must transparently decrypt and produce a still frame.
OUT="$(mktemp -d)"
"$BIN" render "$ID" --frames --out "$OUT"
[ -s "$OUT/$ID-final.png" ] || { echo "missing $OUT/$ID-final.png" >&2; exit 1; }
echo "still from encrypted session: $(wc -c < "$OUT/$ID-final.png") bytes"
echo "04-encrypt OK"
