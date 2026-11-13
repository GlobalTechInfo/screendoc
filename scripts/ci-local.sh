#!/usr/bin/env bash
# Run the CI pipeline locally: the same checks .github/workflows/ci.yml
# and fuzz.yml perform, plus the release dry-run, so the pipelines are
# proven to work without GitHub.
#
# Usage: scripts/ci-local.sh [--no-fuzz]
set -u
cd "$(dirname "$0")/.." || exit 1

DO_FUZZ=1
[ "${1:-}" = "--no-fuzz" ] && DO_FUZZ=0

pass=0; fail=0
step() {
  local name=$1; shift
  echo; echo "=== $name ==="
  if "$@"; then
    echo "PASS  $name"; pass=$((pass + 1))
  else
    echo "FAIL  $name"; fail=$((fail + 1))
  fi
}

run() { # run NAME CMD...
  local name=$1; shift
  echo "  \$ $*"
  "$@" || return 1
  echo "  ok"
}

step "fmt" bash -c "cargo fmt --all -- --check"

step "clippy -D warnings" cargo clippy --workspace --all-targets -- -D warnings

step "test (all)" cargo test --workspace

step "script lint (shellcheck)" bash -c '
  if command -v shellcheck >/dev/null 2>&1; then
    shellcheck -S warning scripts/*.sh
  else
    echo "  (skipped: shellcheck not installed — CI enforces it)"
  fi'

step "supply-chain audit (cargo audit + cargo deny + license check)" bash -c '
  set -e
  command -v cargo-audit >/dev/null 2>&1 || cargo install cargo-audit --locked
  cargo audit
  command -v cargo-deny >/dev/null 2>&1 || cargo install cargo-deny --locked
  cargo deny check bans licenses sources
  ./scripts/license-audit.sh --check'

if [ "$DO_FUZZ" = "1" ]; then
  # `cargo +nightly` only works through the rustup launcher.
  if [ -x "$HOME/.cargo/bin/cargo" ]; then RUSTCARGO="$HOME/.cargo/bin/cargo"; else RUSTCARGO=cargo; fi
  step "fuzz smoke (30s x 4 targets)" env RUSTCARGO="$RUSTCARGO" bash -c '
    set -e
    command -v rustup >/dev/null 2>&1 || { echo "  (skipped: rustup/nightly unavailable)"; exit 0; }
    # Route every tool (cargo AND rustc, spawned by cargo-fuzz) through the
    # rustup proxy so +nightly applies to all of them.
    export PATH="$HOME/.cargo/bin:$PATH"
    for t in marker cast redact enc; do
      echo "  :: $t"
      "$RUSTCARGO" +nightly fuzz run "$t" -- -max_total_time=30 -runs=50000
    done'
fi

step "release build" cargo build --release

step "valgrind sweep (46 checks, no suppressions)" scripts/valgrind-check.sh

step "license audit" scripts/license-audit.sh

step "perf check (30-min soak)" scripts/perf-check.sh

step "release dry-run (artifacts + SHA256SUMS)" bash -c '
  set -e
  rm -rf /tmp/sdoc-release-dry
  OUT_DIR=/tmp/sdoc-release-dry ./scripts/release.sh 0.1.0 --dry-run
  ls /tmp/sdoc-release-dry | head
  test -f /tmp/sdoc-release-dry/SHA256SUMS
'

echo
echo "==============================================="
echo "ci-local: $pass passed, $fail failed"
[ $fail -eq 0 ]