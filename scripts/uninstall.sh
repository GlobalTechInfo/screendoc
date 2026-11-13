#!/usr/bin/env bash
# Uninstall the screendoc binary installed by scripts/install.sh.
#
# Recorded sessions, the (encryption) key file, and config live in
# ~/.screendoc and are LEFT IN PLACE by default.
#
# --purge   also delete ~/.screendoc (sessions, config, key). Destructive:
#           an encrypted session becomes unreadable once the key is gone.
# --dir <p> remove from a specific install dir (default: ~/.local/bin and
#           /usr/local/bin, whichever holds the binary)
set -euo pipefail

BIN_NAME="screendoc"
PURGE=0
DEST=""

while [ $# -gt 0 ]; do
  case "$1" in
    --purge) PURGE=1; shift ;;
    --dir)   DEST="$2"; shift 2 ;;
    -h|--help) grep -E '^# ' "$0" | sed 's/^# \{0,2\}//'; exit 0 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

candidates=("$HOME/.local/bin/$BIN_NAME" "/usr/local/bin/$BIN_NAME" "$HOME/.cargo/bin/$BIN_NAME")
if [ -n "$DEST" ]; then candidates=("$DEST"); fi

removed=0
for c in "${candidates[@]}"; do
  if [ -e "$c" ] || [ -L "$c" ]; then
    rm -f "$c"
    echo "removed $c"
    removed=1
  fi
done
[ "$removed" -eq 1 ] || echo "nothing to remove: no $BIN_NAME binary found in the usual locations"

DATA_DIR="${SCREENDOC_DATA_DIR:-$HOME/.screendoc}"
if [ "$PURGE" -eq 1 ]; then
  if [ -e "$DATA_DIR" ]; then
    read -r -p "delete $DATA_DIR (sessions, config, encryption key)? 'yes' to confirm: " ans
    if [ "$ans" = "yes" ]; then
      rm -rf "$DATA_DIR"
      echo "purged $DATA_DIR"
    else
      echo "aborted: $DATA_DIR kept"
    fi
  fi
else
  echo "recorded sessions remain in $DATA_DIR — run '$0 --purge' to delete them too"
fi
