# screendoc

Record your terminal session and publish it anywhere: **GIF, MP4, WebM, AV1,
asciinema `.cast`, a local web player, a self-contained shareable HTML page,
and per-chapter videos/stills** — all derived from one structured event
journal, with command boundaries, exit codes, and chapters.

[![CI](https://github.com/GlobalTechInfo/screendoc/actions/workflows/ci.yml/badge.svg)](https://github.com/GlobalTechInfo/screendoc/actions/workflows/ci.yml)
[![Fuzz](https://github.com/GlobalTechInfo/screendoc/actions/workflows/fuzz.yml/badge.svg)](https://github.com/GlobalTechInfo/screendoc/actions/workflows/fuzz.yml)
[![Release](https://img.shields.io/github/v/release/GlobalTechInfo/screendoc)](https://github.com/GlobalTechInfo/screendoc/releases)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache-2.0-blue.svg)](https://github.com/GlobalTechInfo/screendoc/blob/main/LICENSE)

```
$ screendoc record cargo test
● recording `cargo test`…
✓ session 1794016110407-394426-0 finalized: 1 command(s), 3s.
  render: screendoc render 1794016110407-394426-0 --gif --mp4
```

## Three ways to record, four targets

| Mode | Captures | Command |
|------|----------|---------|
| **Wrapper** | A command in a fresh pty (bash + inline OSC 90 markers) | `screendoc record … cmd` |
| **Ambient** | Your running interactive shell (PROMPT_COMMAND hooks) | `screendoc on` / `off` (bash/zsh/fish) |
| **tmux pane** | One live tmux pane (`pipe-pane`) | `screendoc record --tmux [pane]` |
| **tmux group** | *Every* pane of a tmux session, as linked sessions | `screendoc record --tmux-group <session>` |

Every run produces a portable session:

```
~/.screendoc/sessions/<id>/
├── meta.json      # shell, size, mode, group, duration, title
├── events.jsonl   # structured event journal — the source of truth
├── markers.jsonl  # command boundaries + chapter markers
├── raw.cast       # asciinema-compatible byte stream
└── frames/        # rendered stills
```

All video is **derived**: re-render any time, as many formats as you like,
without re-recording.

## Installation

`ffmpeg` on `PATH` for video (optional — cast import/export and the web
player work without it). `tmux` is optional (only for `record --tmux`,
Linux/macOS; use WSL on Windows). The installers check and, where a
supported package manager is present, install missing prerequisites
automatically.

**Linux / macOS** — one-liner (downloads the prebuilt binary, verifies
SHA-256, installs ffmpeg if missing):

```sh
curl -fsSL https://raw.githubusercontent.com/GlobalTechInfo/screendoc/main/scripts/install.sh | bash
# pin a version / custom dir / build from source:
curl -fsSL .../install.sh | bash -s -- --version v0.1.0 --dir /usr/local/bin
```

**Termux (Android)** — the same one-liner works; Termux is detected
automatically (static aarch64 musl binary, `$PREFIX/bin` install, `pkg`
for missing prerequisites):

```sh
curl -fsSL https://raw.githubusercontent.com/GlobalTechInfo/screendoc/main/scripts/install.sh | sh
```

**Windows** (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install.ps1
# ffmpeg (video) and Git Bash (the recorded shell) are installed
# automatically when winget or choco is available.
```

**From source** (any platform, requires Rust ≥ 1.99):

```sh
git clone https://github.com/GlobalTechInfo/screendoc && cd screendoc
./scripts/install.sh --source   # cargo release build → ~/.local/bin
screendoc doctor               # verify ffmpeg/tmux/shell/data dir
```

No glibc? Build a fully-static Linux binary: `./scripts/build-static.sh`
(musl). Uninstall: `scripts/uninstall.sh [--purge]` /
`scripts\uninstall.ps1 [-Purge]` (data in `~/.screendoc` is kept unless
you purge). Packages: homebrew formula in `packaging/`, GitHub releases
(Linux gnu x86_64/aarch64 + static musl x86_64/i686/armv7/aarch64 (Termux),
macOS, Windows; SHA-256 checksums, GPG-signed) cut by
`scripts/release.sh <version>` / the tag-triggered workflow. Verify a
release with [`PGP-KEY.asc`](PGP-KEY.asc) (fingerprint
`ADE1CFE02922679A2DE57563AE472669ED78D899` — see
[SECURITY.md](SECURITY.md#verifying-release-artifacts)).

### Platform support

| Platform | Status | Notes |
|----------|--------|-------|
| Linux x86_64 / aarch64 | full | CI-tested; static musl binary for glibc-less hosts |
| Linux i686 / armv7 (32-bit) | full | static musl release artifacts (`screendoc-i686-linux-musl`, `screendoc-armv7-linux-musleabihf`) — run without glibc; compile-gated in CI |
| Termux (Android, aarch64) | full | static musl aarch64 artifact (`screendoc-aarch64-linux-musl`); `install.sh` detects Termux, installs into `$PREFIX/bin`, and pulls `ffmpeg`/`bash` via `pkg` when missing |
| macOS x86_64 / arm64 | full | CI-tested; homebrew formula |
| Windows x86_64 | full | Git Bash is the recorded shell; `record --tmux` → use WSL; file-permission hardening (0600) is a no-op on Windows — the key file lives under `%USERPROFILE%\.screendoc` |

## Quick start

```sh
screendoc record --size 100x30 cargo test
screendoc render <id>                    # GIF + MP4 + cast by default
screendoc render <id> --chapters --gif   # one GIF per chapter
screendoc render <id> --frames           # per-chapter PNG stills
screendoc serve <id> --open              # web player at 127.0.0.1:4999
screendoc play <id>                      # terminal replay
screendoc search "make"                  # find that build across sessions
```

## Commands

| Command | Description |
|---------|-------------|
| `record` | Wrapper / tmux pane / tmux group (`--tmux`, `--tmux-group`, `--encrypt`) |
| `on` / `off` | Ambient shell recording (`on --encrypt`) |
| `hook-script` | Hook snippet to `eval` for bash/zsh/fish |
| `chapter` | Manual chapter marker |
| `render` | `--gif --mp4 --webm --av1 --cast --web --chapters --no-full --frames --idle-limit --speed --fps --gif-fps --out` (`--web` = self-contained HTML) |
| `serve` | Web player: chapter nav, speed, seek, keyboard shortcuts, `.cast` download |
| `play` | In-terminal replay (`--speed`, `--fast`, `--from`, `--to`) |
| `import` | Ingest asciinema v2/v3 casts — plain or gzip — as sessions (`--json` machine output) |
| `search` | Case-insensitive search over command text **and** captured output |
| `redact` | Mask secrets in place: ad-hoc regex, `--secret` (built-in credential detectors), `--rules FILE` (ANSI-segment-aware, irreversible, `--json` summary) |
| `list` / `status` / `rm` / `rename` / `prune` | Lifecycle & retention (`--older-than`, `--keep`, `--all`, `--json`, `list --csv`) |
| `config` | `show` / `get` / `set` the user config file |
| `stop` | Finalize a background tmux recording |
| `doctor` / `completions` | Environment check / shell completions |

## Configuration

`~/.config/screendoc/config.toml` — all keys optional; edit it directly or
via `screendoc config set <key> <value>` / `config show` / `config get`.

Keys: `fps`, `gif_fps`, `speed`, `cols`, `rows`, `port`, `cell_width`,
`cell_height`, `idle_limit` (compress idle gaps in video, 0 = off),
`theme` (`xterm` | `dracula` | `nord` | `tokyo` | `solarized`),
`encrypt` (default false), `retention_days` (0 = off).

## Security

- **Encryption at rest (opt-in):** `record --encrypt` / `on --encrypt`
  (or the `encrypt` config key) stores the session's content files
  (`events.jsonl`, `markers.jsonl`, `raw.cast`) as per-line AES-256-GCM
  records under a 32-byte key at `~/.screendoc/key` (mode 0600). Every
  read path (play, render, redact, search, the web players) decrypts
  transparently; `render --cast` is the explicit decryption point. A
  wrong key or tampered bytes fail with a clean error, never a panic
  (fuzzed: `fuzz/fuzz_targets/enc.rs`, 630k+ runs clean).
- Session files are created **owner-only** (`0700` directory, `0600` logs)
  on Unix even when encryption is off.
- `redact --secret` runs built-in high-precision detectors (AWS/GitHub/
  Google/Slack/Stripe/OpenAI/HuggingFace tokens, PEM private-key blocks,
  bearer tokens, credential assignments like `DB_PASSWORD=…`);
  `redact --rules file` adds your own regexes (`regex` or
  `regex<TAB>replacement` per line, `#` comments).
- `render --web` and the exported `.cast` are shareable only after
  redaction or export — there is no secret-stripping on render.
- **Dependency license audit:** `scripts/license-audit.sh` regenerates
  `docs/LICENSES.md` from the pinned lockfile and fails on any license
  outside the allowlist (permissive + font-data + one audited
  file-scoped-MPL transitive). The former `colored` dependency
  (relicensed to MPL-2.0 in v3) was replaced by an internal ANSI module,
  so no copyleft direct dependency remains.

## Operations

- **Retention:** `prune --older-than/--keep/--all` for manual cleanup;
  set `retention_days` (0 = off) in the config for automatic cleanup
  after each recording finalizes.
- **Machine output:** `--json` on `import`, `prune`, `redact`, `list`,
  `search`, `status`, `doctor`; `list --csv` for spreadsheets.
- **Performance evidence:** `scripts/perf-check.sh` synthesizes a
  30-minute session (9k events) and asserts wall time + peak RSS budgets
  (last run: import/render < 0.1 s, 12 MB peak RSS).
- **Compliance gate:** `scripts/ci-local.sh` runs the entire CI
  pipeline locally (fmt, clippy `-D warnings`, all tests, fuzz smoke on
  all four targets, release build, the 46-check no-suppression valgrind
  sweep, license audit, soak check, release dry-run) so the GitHub
  workflows are proven, not assumed.

## Formats & interop

- **Export:** asciinema v2 `.cast` (plays in `asciinema play` and on
  asciinema.org), GIF/MP4/WebM/AV1 (ffmpeg), local web player, and a
  **self-contained `session.html`** (`render --web`) that inlines xterm.js
  plus the recording — open it from disk or embed it in docs, no server.
- **Import:** `screendoc import file.cast` accepts v2 (JSON-array, absolute
  times) and v3 (JSON-lines, relative times) casts, plain or
  gzip-compressed (`.cast.gz` / magic-byte detected), including v3
  markers → chapters.

## How it works

- **Wrapper** spawns bash in a portable pty with a generated `--rcfile`
  that emits inline markers (`ESC]90;cmd;seq;rc;text BEL`), demuxed into
  structured `Command`/`Chapter` events alongside the raw bytes.
- **Ambient/tmux** append markers to `markers.jsonl` (hooks / `pipe-pane`).
- **Rendering** replays the cast through a VT100 grid, rasterizes with an
  embedded DejaVu font, compresses idle gaps, then encodes via ffmpeg;
  chapter titles + a time readout fade into the overlay bar. Per-chapter
  videos seed screen state up to the chapter start.

## Limitations

- Wrapper mode records through `bash` (on Windows: Git Bash); other shells
  via ambient hooks/tmux.
- GIF/MP4/WebM/AV1 need `ffmpeg`; cast/web player exports don't.
- The `serve` web player is local-only (127.0.0.1); share via `render --web`
  or the exported `.cast` instead.
- `record --tmux` needs tmux (Linux/macOS; on Windows use WSL).
- `redact` is best-effort: secrets split across events or inside escape
  sequences may not match. Run it *before* exporting/sharing.

## Development

Cargo workspace: `sdoc-core` (events/store/cast/markers/config/redaction/
encryption), `sdoc-record` (pty wrapper, hooks, tmux), `sdoc-render`
(rasterizer, ffmpeg pipeline, web players), `screendoc` (CLI + tests).

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
scripts/ci-local.sh        # the whole CI pipeline, locally
```

Contributors: read [CONTRIBUTING.md](CONTRIBUTING.md) for the house rules
(pinned deps, zero `#[allow]`, every behavior tested). Reports:
[issues](https://github.com/GlobalTechInfo/screendoc/issues), security:
[SECURITY.md](SECURITY.md).

`fuzz/` holds libFuzzer targets for the marker demuxer, the cast reader,
the redactor, and the encrypted record format (`cargo +nightly fuzz run
marker|cast|redact|enc`). A CI fuzz smoke job
(`.github/workflows/fuzz.yml`) builds and briefly runs every target on
each push and weekly; crash artifacts are uploaded on failure. UBSAN is
not available from rustc yet (see rust-lang/rust#34755), so
undefined-behavior coverage comes from the checks below plus
overflow-checked hot paths (`parse_duration`, `civil_from_days`).

Memory rigour: `scripts/valgrind-check.sh` runs the whole CLI surface
(every subcommand, edge inputs, the HTTP server, encrypted sessions)
under valgrind with definite/possible/indirect leaks as errors — 46
checks, all passing. The sweep builds a dedicated unoptimized `valgrind`
cargo profile (valgrind is only reliable on deterministic, unoptimized
builds) and uses **no suppressions**: nothing is silenced; every check
passes on a clean, fully-instrumented binary.

## License

MIT OR Apache-2.0 — see [LICENSE](LICENSE). The dependency tree is
audited by `scripts/license-audit.sh` against an allowlist
([docs/LICENSES.md](docs/LICENSES.md)).
