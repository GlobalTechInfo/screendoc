# Contributing to screendoc

Thanks for helping. This repo is deliberately boring: pinned dependencies,
zero-warning lints, and a hard gate. Match that.

## Development setup

```sh
# Requirements: Rust ≥ 1.99 (stable), ffmpeg on PATH for video tests,
# tmux for the tmux e2e tests (tests skip cleanly without it), and a
# modern valgrind for the memory sweep (optional).
cargo build --release
./scripts/install.sh

# Full local gate (what CI runs, plus the heavy checks):
scripts/ci-local.sh            # add --no-fuzz if you don't have nightly

# The individual pieces:
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/valgrind-check.sh      # 46 checks, no suppressions
scripts/perf-check.sh          # 30-minute soak, RSS budget
scripts/license-audit.sh       # regenerates docs/LICENSES.md
```

Fuzzing (optional, nightly + cargo-fuzz):

```sh
rustup toolchain install nightly
cargo +nightly install cargo-fuzz
cargo +nightly fuzz run marker|cast|redact|enc
```

## Rules of the house

- **Pinned dependencies, always.** New deps get an exact `=x.y.z` pin in
  the crate manifest; the lockfile is the source of truth.
- **No new lints, no new `#[allow]`.** Clippy is `-D warnings`; if a
  lint is wrong, fix the code (or argue it in the PR, not a pragma).
- **Every behavior ships with a test.** Unit tests in-crate; CLI surface
  in `crates/screendoc/tests/e2e.rs` (the suite serializes itself and
  cleans up after itself).
- **Security-sensitive code** (session storage, redaction, encryption,
  key handling) gets adversarial tests: hostile inputs, wrong keys,
  truncated files — and fuzz targets where the input is a byte stream.
- **No silent fallbacks.** Failures surface as clean errors; nothing is
  swallowed to keep a command from exiting non-zero.
- **Documentation follows the change:** README for user-facing changes,
  CHANGELOG for releases, `docs/LICENSES.md` regenerated when the
  dependency tree moves.

## Workflow

1. Fork → branch → PR. Small, reviewable diffs beat grand gestures.
2. Fill the PR template; the checklist is the contract.
3. CI must be green before review: fmt, clippy, tests (Linux + macOS),
   fuzz smoke, and the tag-triggered release job only on tags.
4. Maintainers merge with a green gate; release tags (`v*`) cut the
   artifacts automatically.

Questions that aren't "how do I use X" belong in a discussion or a
labeled issue, not in code review.
