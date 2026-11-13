#!/usr/bin/env bash
# Valgrind memory/leak sweep across every subcommand and the edge-case
# inputs that matter. Each command runs in an isolated scratch HOME so
# sessions don't bleed.
#
# The sweep builds and tests the `valgrind` cargo profile: unoptimized with
# full debug info. Valgrind is designed for deterministic unoptimized
# builds — no suppressions are used, and nothing is silenced: every check
# must pass with a clean, fully-instrumented binary.
#
# Verdict rule: a check fails when (a) valgrind exits 99 (--error-exitcode:
# memory error or definite leak), (b) the child's exit code differs from
# the expected one, or (c) valgrind itself misbehaves ("valgrind: " lines).
#
# Usage: scripts/valgrind-check.sh
#        BIN=/path/to/binary scripts/valgrind-check.sh   (opt-in override)
set -u
cd "$(dirname "$0")/.." || exit 1

if [ -n "${BIN:-}" ]; then
  :
else
  command cargo build --profile valgrind 2>&1 | tail -1
  BIN=target/valgrind/screendoc
fi
[ -x "$BIN" ] || { echo "missing $BIN"; exit 2; }

VG="valgrind --leak-check=full --track-origins=yes --errors-for-leak-kinds=definite,possible,indirect --error-exitcode=99"
WORK=$(mktemp -d /tmp/sdoc-vgrind-XXXXXX)
HOME_ISOLATE="$WORK/home"; mkdir -p "$HOME_ISOLATE"

pass=0; fail=0; failed_cmds=()

# check NAME EXPECTED_RC CMD...
check() {
  local name=$1; shift
  local expected=$1; shift
  local out rc log
  log=$WORK/logs/$name.log
  mkdir -p "$WORK/logs"
  out=$(env HOME="$HOME_ISOLATE" $VG "$@" 2>&1)
  rc=$?
  printf '%s\n' "$out" > "$log"
  if [ $rc -eq 99 ]; then
    echo "FAIL  $name (valgrind error) — log: $log"
    echo "$out" | grep -E "Invalid|uninitialised|definitely lost|ERROR SUMMARY|Lost object|== *[0-9]+ ==" | head -14
    fail=$((fail + 1)); failed_cmds+=("$name")
  elif [ $rc -ne $expected ]; then
    echo "FAIL  $name (child rc $rc, expected $expected)"
    echo "$out" | tail -4
    fail=$((fail + 1)); failed_cmds+=("$name")
  elif echo "$out" | grep -qE "^valgrind:"; then
    echo "FAIL  $name (valgrind option/runtime complaint)"
    echo "$out" | grep -E "^valgrind:" | head -5
    fail=$((fail + 1)); failed_cmds+=("$name")
  else
    echo "PASS  $name"
    pass=$((pass + 1))
  fi
}

# --- fixtures ---------------------------------------------------------------
fix_v2=$WORK/fixture-v2.cast
printf '%s\n' \
  '{"version":2,"width":80,"height":24,"timestamp":0,"title":"vg"}' \
  '[0.5,"prompt $ "]' \
  '[0.9,"echo hello-world && exit 3"," "]' \
  '[2.1,""]' > "$fix_v2"

# Gzipped fixture: transparent gzip import path.
gzip -c "$fix_v2" > "$fix_v2.gz"

# A one-rule file for the `redact --rules` path.
printf 'grep\\s*-r\\t[hit]\\n' > "$WORK/rules.txt"

fix_v3=$WORK/fixture-v3.jsonl
printf '%s\n' \
  '{"timestamp":0,"type":"session","version":3,"width":80,"height":24,"env":{"SHELL":"/bin/bash","TERM":"xterm-256color"}}' \
  '{"timestamp":0.4,"type":"output","text":"root@vg:~# "}' \
  '{"timestamp":0.7,"type":"output","text":"grep -r secret .","tokens":[]}' \
  '{"timestamp":1.2,"type":"output","text":"src/a.rs:1:secret_x\n"}' \
  '{"timestamp":1.5,"type":"input","text":"exit 0","tokens":[]}' \
  '{"timestamp":9.0,"type":"end","status":0}' > "$fix_v3"

fix_junk=$WORK/junk.cast
head -c 4096 /dev/urandom > "$fix_junk"

fix_empty=$WORK/empty.cast
: > "$fix_empty"

# --- stateless commands -------------------------------------------------------
check "version"           0 "$BIN" --version
check "help"              0 "$BIN" --help
check "doctor"            0 "$BIN" doctor
check "list-empty"        0 "$BIN" list
check "completions-bash"  0 "$BIN" completions bash
check "completions-zsh"   0 "$BIN" completions zsh
# Without an active session the hook bind fails cleanly (rc 1) — the real
# hook path needs a running ambient session, covered by the e2e suite.
check "hook-script"       1 "$BIN" hook-script bash
check "status-none"       0 "$BIN" status --json
check "search-no-hit"     0 "$BIN" search no-such-query-zzz
check "play-missing"      1 "$BIN" play no-such-session
check "render-missing"    1 "$BIN" render no-such-session --cast
check "traversal-id"      1 "$BIN" play ../..

# --- import (v2, v3, junk, empty) ---------------------------------------------
check "import-v2"         0 "$BIN" import "$fix_v2"
check "import-v2-gz"      0 "$BIN" import "$fix_v2.gz"
check "import-v3"         0 "$BIN" import "$fix_v3"
check "import-junk"       0 "$BIN" import "$fix_junk"
check "import-empty"      0 "$BIN" import "$fix_empty"

imported_v2=$(ls "$HOME_ISOLATE/.screendoc/sessions" 2>/dev/null | head -1)
imported_v3=$(ls "$HOME_ISOLATE/.screendoc/sessions" 2>/dev/null | sed -n '3p')
[ -n "$imported_v2" ] || { echo "FATAL: no session created by import"; exit 1; }

# --- session operations ---------------------------------------------------------
check "search-hit"        0 "$BIN" search "grep"
check "search-json"       0 "$BIN" search "secret" --json
check "play-fast"         0 "$BIN" play "$imported_v2" --fast
check "play-range"        0 "$BIN" play "$imported_v2" --fast --from 0.2 --to 0.9
check "play-range-neg"    0 "$BIN" play "$imported_v2" --fast --from -1
check "render-cast"       0 "$BIN" render "$imported_v2" --cast
check "render-gif"        0 "$BIN" render "$imported_v2" --gif
check "render-frames"     0 "$BIN" render "$imported_v2" --cast --frames
check "render-web"        0 "$BIN" render "$imported_v2" --web
check "redact"            0 "$BIN" redact "$imported_v3" "secret[_a-z]*" --replace "***" --yes
check "redact-invalid"    1 "$BIN" redact "$imported_v3" "(" --yes
check "redact-secret"     0 "$BIN" redact "$imported_v3" --secret --yes
check "redact-rules"      0 "$BIN" redact "$imported_v3" --rules "$WORK/rules.txt" --yes
check "redact-rules-empty" 1 "$BIN" redact "$imported_v3" --rules /dev/null --yes
check "redact-no-source"  1 "$BIN" redact "$imported_v3" --yes
check "rename"            0 "$BIN" rename "$imported_v2" "renamed-vg"
check "rename-self"       0 "$BIN" rename "renamed-vg" "renamed-vg"
check "rename-onto-existing" 1 "$BIN" rename "renamed-vg" "$imported_v3"
imported_v2=renamed-vg   # the session was renamed; later checks use the new id
# --- serve smoke (needs a session — before prune-all) -------------------------
serve_home="$WORK/serve-home"; mkdir -p "$serve_home"
env HOME="$HOME_ISOLATE" $VG "$BIN" serve "$imported_v2" --port 4781 >"$WORK/serve.log" 2>&1 &
vpid=$!
# Give the (valgrind-instrumented, debug-build) server time to bind before
# exercising the HTTP handler, then stop it.
sleep 10
curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:4781/ >/dev/null 2>&1
curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:4781/no-such-thing >/dev/null 2>&1
pkill -f "$(basename "$BIN") serve" 2>/dev/null
wait "$vpid" 2>/dev/null
rc=$?
# valgrind exits 99 only on its own findings; a SIGTERM-killed child is fine.
if [ $rc -eq 99 ]; then
  echo "FAIL  serve-smoke (valgrind error)"
  grep -E "Invalid|definitely lost|ERROR SUMMARY" "$WORK/serve.log" | head -14
  fail=$((fail + 1)); failed_cmds+=("serve-smoke")
else
  echo "PASS  serve-smoke"
  pass=$((pass + 1))
fi

# --- encrypted session path (record --encrypt + transparent ops) -------------
mkdir -p "$HOME_ISOLATE/.screendoc"
check "record-encrypted" 0 "$BIN" record --size "40x5" --encrypt sh -c "echo enc-sweep; exit 0"
enc_id=$(ls "$HOME_ISOLATE/.screendoc/sessions" 2>/dev/null | tail -1)
[ -n "$enc_id" ] && check "play-encrypted" 0 "$BIN" play "$enc_id" --fast
[ -n "$enc_id" ] && check "redact-encrypted" 0 "$BIN" redact "$enc_id" "enc-sweep" --replace "x" --yes
[ -n "$enc_id" ] && check "render-encrypted-web" 0 "$BIN" render "$enc_id" --web

check "prune-all"         0 "$BIN" prune --all --yes

# --- config subcommand --------------------------------------------------------
check "config-show"       0 "$BIN" config show
check "config-get"        0 "$BIN" config get theme
check "config-set"        0 "$BIN" config set theme xterm
check "config-set-bad"    1 "$BIN" config set fps not-a-number
check "config-unknown"    1 "$BIN" config get definitely-not-a-key

# --- cleanup: remove surviving sessions ---------------------------------------
for d in "$HOME_ISOLATE/.screendoc/sessions"/*/; do
  [ -d "$d" ] || continue
  s="$(basename "$d")"
  check "rm-$s" 0 "$BIN" rm "$s" --yes
done

echo
echo "==============================================="
echo "valgrind sweep: $pass passed, $fail failed"
[ $fail -eq 0 ] && echo "memory: clean; no definite leaks"
if [ $fail -eq 0 ]; then
  rm -rf "$WORK"
else
  echo "failing-check logs kept in: $WORK/logs"
fi
if [ $fail -eq 0 ]; then exit 0; else exit 1; fi
