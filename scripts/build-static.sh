#!/usr/bin/env bash
# Build a fully-static screendoc binary for aarch64-unknown-linux-musl.
# Requires: musl-tools (apt) and the aarch64-unknown-linux-musl rust target.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc
CC=musl-gcc RUSTFLAGS="-C target-feature=+crt-static" \
  cargo build --release --target aarch64-unknown-linux-musl

BIN=target/aarch64-unknown-linux-musl/release/screendoc
mkdir -p dist
cp "$BIN" "dist/screendoc-$(uname -m)-linux-musl"
echo "==> dist/screendoc-$(uname -m)-linux-musl ($(du -h "$BIN" | cut -f1), fully static)"
