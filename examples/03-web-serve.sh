#!/usr/bin/env bash
# 03-web-serve: serve a session to a browser via the embedded xterm.js player.
# Usage: ./03-web-serve.sh /path/to/screendoc
set -euo pipefail
BIN="${1:-}"
[ -n "$BIN" ] || { echo "usage: $0 /path/to/screendoc" >&2; exit 2; }
command -v tmux >/dev/null || { echo "tmux not found" >&2; exit 1; }
command -v curl >/dev/null || { echo "curl not found" >&2; exit 1; }

TMPD="$(mktemp -d)"
export HOME="$TMPD"
trap 'rm -rf "$TMPD"' EXIT

"$BIN" record --size 80x24 \
  bash -lc 'echo hello from the web player; sleep 0.5; echo bye'
ID="$(ls "$HOME/.screendoc/sessions" | head -n1)"
[ -n "$ID" ] || { echo "no session was recorded" >&2; exit 1; }

PORT=4999
"$BIN" serve "$ID" --port "$PORT" &
SERVE_PID=$!

# Wait for the listener (up to 10s), then verify the player page responds.
ok=0
for _ in $(seq 1 100); do
  if curl -sf -o /dev/null "http://127.0.0.1:$PORT/"; then
    ok=1
    break
  fi
  sleep 0.1
done
"$BIN" stop || true
kill "$SERVE_PID" 2>/dev/null || true
[ "$ok" = 1 ] || { echo "web player never came up on :$PORT" >&2; exit 1; }

echo "player: http://127.0.0.1:$PORT/"
echo "03-web-serve OK"
