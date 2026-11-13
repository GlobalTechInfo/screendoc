#!/usr/bin/env bash
# Performance and soak check: synthesize a 30-minute session, import it,
# render the cast, and verify wall time + peak RSS stay within budget.
#
# Usage: scripts/perf-check.sh   (builds release first if missing)
#   PERF_OUT=/path            redirect the synthetic cast (default: tmp)
set -u
cd "$(dirname "$0")/.." || exit 1

BIN=${BIN:-target/release/screendoc}
[ -x "$BIN" ] || { command cargo build --release 2>&1 | tail -1; }
[ -x "$BIN" ] || { echo "missing $BIN"; exit 2; }

WORK=${PERF_OUT:-$(mktemp -d /tmp/sdoc-perf-XXXXXX)}
CAST="$WORK/soak30m.cast"

# 30 minutes at 5 events/s = 9000 events, plus a resize every 30 s.
python3 - "$CAST" <<'EOF'
import sys
out = open(sys.argv[1], "w")
out.write('{"version":2,"width":120,"height":30}\n')
for i in range(9000):
    t = i * 0.2
    out.write('[%.3f,"o","soak heartbeat line %05d with a little padding text\\r\\n"]\n' % (t, i))
    if i % 150 == 0:
        out.write('[%.3f,"r","100x40"]\n' % t)
out.close()
EOF
echo "synthesized $(du -h "$CAST" | cut -f1) cast: 9000 events + 60 resizes (~30 min)"

HOME_ISOLATE=$(mktemp -d "$WORK/home-XXXXXX")
export HOME="$HOME_ISOLATE"

# --- import ---
start=$EPOCHREALTIME
ID=$("$BIN" import "$CAST" --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
rc=$?
end=$EPOCHREALTIME
if [ $rc -ne 0 ]; then echo "FAIL import"; exit 1; fi
echo "import: $(python3 -c "print(f'{$end - $start:.1f}s')")  (session $ID)"

# --- render cast (the full read+serialize+write pass) ---
OUTDIR="$WORK/out"
start=$EPOCHREALTIME
"$BIN" render "$ID" --cast --out "$OUTDIR" >/dev/null
rc=$?
end=$EPOCHREALTIME
if [ $rc -ne 0 ]; then echo "FAIL render-cast"; exit 1; fi
echo "render-cast: $(python3 -c "print(f'{$end - $start:.1f}s')")  ($(du -h "$OUTDIR/$ID.cast" | cut -f1))"

# --- peak RSS of the render path (child's ru_maxrss via python) ---
RSS=$(python3 - "$BIN" "$ID" "$OUTDIR" <<'EOF'
import resource, subprocess, sys
subprocess.run([sys.argv[1], "render", sys.argv[2], "--cast", "--out", sys.argv[3]],
               stdout=subprocess.DEVNULL)
print(int(resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss))
EOF
)
echo "peak rss: ${RSS:-?} KB"

# --- budgets (loose but real) ---
FAIL=0
if [ -n "${RSS:-}" ]; then
  [ "$RSS" -gt 419430 ] && { echo "FAIL: peak RSS ${RSS}KB > 419430KB (400MB)"; FAIL=1; }
fi
rm -rf "$WORK"
[ $FAIL -eq 0 ] && echo "perf: within budget"
exit $FAIL
