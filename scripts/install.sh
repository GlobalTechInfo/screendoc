#!/usr/bin/env bash
# screendoc installer.
#
# Default: downloads the prebuilt binary for your OS/architecture from the
# GitHub releases, verifies the SHA-256, and installs it to ~/.local/bin
# (Termux: $PREFIX/bin). Prerequisites are checked and (when a supported
# package manager is present — apt/brew/dnf/pacman/Termux pkg) installed
# automatically:
#   - curl   (required for the download)
#   - ffmpeg (optional: video renders; cast/web/import still work without)
#   - bash   (the shell the recording wrapper launches)
#   - tmux   (optional: `record --tmux`)
#
# 32-bit Linux (i686/armv7) and Termux (aarch64, no glibc) automatically
# get the fully-static musl artifacts.
#
# --source        build from this checkout instead of downloading
# --version <tag> pin a release tag (default: latest)
# --dir <path>    install directory (default: ~/.local/bin, $PREFIX/bin on Termux)
set -euo pipefail

REPO="GlobalTechInfo/screendoc"
BIN_NAME="screendoc"
VERSION="latest"
SOURCE=0
DEST_SET=0
DEST_DIR="${SCREENDOC_INSTALL_DIR:-$HOME/.local/bin}"

usage() { grep -E '^# ' "$0" | sed 's/^# \{0,2\}//'; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --source)  SOURCE=1; shift ;;
    --dir)     DEST_DIR="$2"; DEST_SET=1; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown flag: $1" >&2; usage; exit 2 ;;
  esac
done

log()  { printf '==> %s\n' "$*"; }
warn() { printf '    !! %s\n' "$*" >&2; }

os="$(uname -s)"
arch="$(uname -m)"
# Termux (Android) detection: $PREFIX points at the Termux root
# (/data/data/com.termux/files). Termux has no glibc, so the static musl
# artifact is the only one that runs there.
TERMUX=0
case "${PREFIX:-}" in
  */com.termux*) TERMUX=1 ;;
esac

# 64-bit: native gnu/darwin artifacts; 32-bit (i686, armv7): the static
# musl release artifacts, which run on any Linux of that word size with
# no glibc requirements. Termux (aarch64): the static musl aarch64
# artifact for the same no-glibc reason.
case "$os:$arch" in
  Linux:x86_64|Linux:amd64)       artifact="$BIN_NAME-x86_64-unknown-linux-gnu.tar.gz" ;;
  Linux:aarch64|Linux:arm64)
    if [ "$TERMUX" -eq 1 ]; then
      artifact="$BIN_NAME-aarch64-linux-musl.tar.gz"
    else
      artifact="$BIN_NAME-aarch64-unknown-linux-gnu.tar.gz"
    fi ;;
  Linux:i686|Linux:i386|Linux:x86) artifact="$BIN_NAME-i686-linux-musl.tar.gz" ;;
  Linux:armv7l|Linux:armv7)       artifact="$BIN_NAME-armv7-linux-musleabihf.tar.gz" ;;
  Darwin:arm64)                   artifact="$BIN_NAME-aarch64-apple-darwin.tar.gz" ;;
  Darwin:x86_64|Darwin:amd64)     artifact="$BIN_NAME-x86_64-apple-darwin.tar.gz" ;;
  *)
    echo "unsupported platform: $os/$arch — clone the repo and build with 'cargo build --release' (see README)" >&2
    exit 1 ;;
esac

# Termux installs into $PREFIX/bin, which is already on PATH — unless the
# user chose a directory explicitly.
[ "$TERMUX" -eq 1 ] && [ "$DEST_SET" -eq 0 ] && DEST_DIR="$PREFIX/bin"

# ----------------------------------------------------------------------
# Prerequisites
# ----------------------------------------------------------------------
have() { command -v "$1" >/dev/null 2>&1; }

pkg_apt()   { have apt-get && apt-get install -y --no-install-recommends "$1"; }
pkg_brew()  { have brew && brew install --formula "$1"; }
pkg_dnf()   { have dnf && (dnf install -y "$1" || true); }
pkg_pacman(){ have pacman && (pacman -S --noconfirm --needed "$1" >/dev/null || true); }
pkg_termux(){ [ "$TERMUX" -eq 1 ] && have pkg && pkg install -y "$1"; }

ensure_curl() {
  have curl && return 0
  log "installing curl (required for the download)"
  { have sudo && sudo apt-get update && sudo apt-get install -y curl; } \
    || pkg_apt curl || pkg_brew curl || pkg_dnf curl || pkg_pacman curl \
    || pkg_termux curl \
    || { warn "curl not found and no supported package manager could install it."; exit 1; }
}

ensure_ffmpeg() {
  if have ffmpeg; then return 0; fi
  log "ffmpeg not found — video rendering (mp4/webm/gif/gif-av1) is optional;"
  log "installing ffmpeg now so all output formats work"
  local installed=0
  if [ "$os" = "Darwin" ] && pkg_brew ffmpeg; then installed=1
  elif [ "$TERMUX" -eq 1 ] && pkg_termux ffmpeg; then installed=1
  elif have sudo && have apt-get; then
    sudo apt-get update && { sudo apt-get install -y ffmpeg || apt-get install -y ffmpeg; } && installed=1
  elif pkg_pacman ffmpeg; then installed=1
  elif pkg_dnf ffmpeg; then installed=1; fi
  if [ $installed -eq 1 ] && have ffmpeg; then
    log "ffmpeg installed"
  else
    warn "could not install ffmpeg automatically — cast/web/import still work;"
    warn "video rendering is disabled. Install ffmpeg and re-run renders."
  fi
}

# The recording wrapper launches `bash` in a pty. Termux ships mksh by
# default, so bash is installed when missing.
ensure_bash() {
  if have bash; then return 0; fi
  if [ "$TERMUX" -eq 1 ]; then
    log "bash not found (Termux ships mksh) — installing it; the recording wrapper needs a bash shell"
    pkg_termux bash && log "bash installed" \
      || warn "could not install bash — wrapper-mode recording will not work"
  else
    warn "bash not found — wrapper-mode recording needs it; install it via your package manager."
  fi
}

ensure_tmux() {
  if have tmux; then return 0; fi
  if [ "$TERMUX" -eq 1 ]; then
    warn "tmux not found — it is only needed for 'record --tmux' ('pkg install tmux' on Termux)."
    return 0
  fi
  warn "tmux not found — it is only needed for 'record --tmux' (skipping auto-install);"
  warn "install it via your package manager when you need it."
}

# ----------------------------------------------------------------------
# Fetch
# ----------------------------------------------------------------------
if [ "$SOURCE" -eq 1 ]; then
  log "building from source (release mode)"
  if ! have cargo; then
    warn "cargo not found. Install Rust (https://rustup.rs) or drop --source to"
    warn "download a prebuilt binary instead."
    exit 1
  fi
  ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  cargo build --release --manifest-path "$ROOT/Cargo.toml"
  BUILT="$ROOT/target/release/$BIN_NAME"
  mkdir -p "$DEST_DIR"
  install -m 0755 "$BUILT" "$DEST_DIR/$BIN_NAME"
  log "installed $DEST_DIR/$BIN_NAME (source build)"
else
  case "$VERSION" in
    latest)
      log "resolving the latest release tag"
      VERSION="$(curl -fsSL --max-time 15 \
        "https://api.github.com/repos/$REPO/releases/latest" \
        | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)"
      [ -n "$VERSION" ] || { warn "no releases published yet — use 'install.sh --source' from a checkout."; exit 1; } ;;
    v*) ;;
    *) VERSION="v$VERSION" ;;
  esac

  ensure_curl
  log "downloading $artifact (${VERSION})"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  if ! curl -fsSL --max-time 600 -o "$tmp/$artifact" \
       "https://github.com/$REPO/releases/download/$VERSION/$artifact"; then
    warn "artifact $artifact is not available for $VERSION yet."
    warn "clone the repo and run 'scripts/install.sh --source' to build instead."
    exit 1
  fi
  # Verify when the publisher shipped a checksum (the release workflow does).
  if curl -fsSL --max-time 30 -o "$tmp/$artifact.sha256" \
       "https://github.com/$REPO/releases/download/$VERSION/$artifact.sha256" 2>/dev/null; then
    (cd "$tmp" && sed 's/^[0-9a-fA-F] *//' "$artifact.sha256" >/dev/null 2>&1 || true)
    if cd "$tmp" && sha256sum -c "$artifact.sha256" >/dev/null 2>&1; then
      log "sha256 verified"
    else
      [ -s "$tmp/$artifact.sha256" ] && { warn "sha256 mismatch — aborting."; exit 1; }
    fi
  fi

  # GPG: when the release ships a detached signature, verify it against
  # the release key. A failed signature is fatal; a missing gpg binary
  # or unsigned release only downgrades to the checksum check.
  if curl -fsSL --max-time 30 -o "$tmp/$artifact.asc" \
       "https://github.com/$REPO/releases/download/$VERSION/$artifact.asc" 2>/dev/null; then
    if have gpg; then
      curl -fsSL --max-time 30 -o "$tmp/PGP-KEY.asc" \
        "https://github.com/$REPO/releases/download/$VERSION/PGP-KEY.asc" \
        || { warn "could not fetch the release PGP key."; exit 1; }
      gpg --batch --quiet --import "$tmp/PGP-KEY.asc" 2>/dev/null || true
      if ! gpg --batch --verify "$tmp/$artifact.asc" "$tmp/$artifact" >/dev/null 2>&1; then
        warn "gpg signature verification FAILED — aborting."
        exit 1
      fi
      log "gpg signature verified"
    else
      warn "release ships a .asc signature but gpg is not installed — skipping signature check."
    fi
  fi
  tar -xzf "$tmp/$artifact" -C "$tmp"
  mkdir -p "$DEST_DIR"
  install -m 0755 "$tmp/$BIN_NAME/$BIN_NAME" "$DEST_DIR/$BIN_NAME"
  log "installed $DEST_DIR/$BIN_NAME ($VERSION)"

  ensure_ffmpeg
  ensure_bash
  ensure_tmux
fi

# ----------------------------------------------------------------------
# PATH
# ----------------------------------------------------------------------
case ":$PATH:" in
  *":$DEST_DIR:"*) ;;
  *)
    log "NOTE: $DEST_DIR is not on your PATH. Add it to your shell profile, e.g.:"
    log "    echo 'export PATH=\"$HOME/.local/bin:\$PATH\"' >> ~/.bashrc" ;;
esac

log "done. verify with: $BIN_NAME doctor"
