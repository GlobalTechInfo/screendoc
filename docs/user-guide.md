# User guide

screendoc records terminal sessions and turns them into shareable artifacts
(GIF / MP4 / WebM / AV1 video, still frames, a live web player, and
searchable text) without leaving your terminal.

## Install

```sh
cargo install screendoc
# or grab a signed binary from the GitHub releases (verify with
# `gpg --verify` against the released PGP-KEY.asc and SHA256SUMS)
```

Requirements: `tmux`. Video rendering additionally requires `ffmpeg`.

## Quick start

```sh
# Record a command (wrapper mode) and render it:
screendoc record echo "hello from screendoc"
screendoc list                      # shows the session id
screendoc render <id> --gif --mp4   # writes <id>.gif / <id>.mp4 to ./

# Record your whole shell (ambient mode):
screendoc on
# ... use your shell as usual; each command becomes a chapter ...
screendoc off
```

## Recording modes

| Mode | Command | When to use |
|---|---|---|
| Wrapper | `screendoc record <command…>` | One-shot: run a command in a fresh, recorded pane |
| Tmux pane | `screendoc record --tmux [target]` | Record an existing pane from outside, or the current one |
| Tmux group | `screendoc record --tmux-group <session>` | Record every pane of a session (linked sessions) |
| Ambient | `screendoc on` / `off` | Record an interactive shell session end to end |

Use `--size COLSXROWS` in wrapper mode to force the terminal dimensions —
the renderer honors the recorded size, so an 80×24 recording renders at
80×24. Use `--encrypt` to store the session's files encrypted at rest.

## Chapters and markers

- Commands are auto-chaptered: each command becomes a chapter entry in the
  session's event log.
- `screendoc chapter "my step" --session <id>` adds a manual chapter.
- `screendoc render <id> --chapters` splits the video at chapter marks.

## Rendering

`render` produces derived artifacts from the recorded session:

```sh
screendoc render <id> --gif --mp4 --webm --av1   # video (needs ffmpeg)
screendoc render <id> --frames                    # still PNGs (no ffmpeg)
screendoc render <id> --cast                      # portable asciinema cast
screendoc render <id> --web                       # self-contained HTML player
screendoc render <id> --chapters                  # split video at chapters
screendoc render <id> --out ./artifacts            # where to write
```

Options: `--fps` / `--gif-fps`, `--speed`, `--idle-limit` (idle gaps
longer than this collapse), `--theme` (color scheme), `--no-full` (skip the
full-video render when producing chapters).

If a session captured no terminal output (e.g. a command that exited
immediately), the renderer prints a warning: the video will show a blank
terminal with a minimum playable duration rather than a broken one-frame
artifact.

## Playback and sharing

```sh
screendoc play <id>        # replay in your terminal (no ffmpeg needed)
screendoc serve <id>       # live web player on :4999 (Ctrl-C to stop)
screendoc import <file>    # import a stock asciinema cast as a session
```

The `--web` export and `serve` both use an embedded xterm.js player that
reads the cast dimensions from the file header, so they are correct for any
recorded size.

## Searching and inspecting

```sh
screendoc search "cargo test"          # grep across all sessions
screendoc search "segfault" --json     # machine-readable
screendoc status                       # what is the active session
screendoc list --csv                   # tabular listing
screendoc doctor                       # check tmux/ffmpeg/config
screendoc prune                        # drop sessions older than retention_days
```

## Redaction

`redact` is **in-place and irreversible**: it rewrites the session's stored
files so the pattern never appears on disk again.

```sh
screendoc redact <id> 'sk-[A-Za-z0-9]{20,}'   # one pattern
screendoc redact <id> --secret                 # built-in secret detector
screendoc redact <id> --rules rules.json       # named rules, in order
```

Rules apply to the printable text of the terminal stream only — ANSI
escape sequences pass through untouched, so the stream stays well-formed.
Rederive any rendered artifacts after redacting.

## Configuration

`~/.config/screendoc/config.toml`:

```toml
fps = 20
gif_fps = 15
speed = 1.0
cols = 120
rows = 30
cell_width = 12
cell_height = 24
idle_limit = 2.0
theme = "xterm"
encrypt = false
retention_days = 0
port = 4999
```

Inspect with `screendoc config show`, change a value with
`screendoc config set <key> <value>`.

## Data layout

```
~/.screendoc/
├── key                    # AES-256-GCM key (0600), only if you used --encrypt
└── sessions/<id>/
    ├── meta.json
    ├── raw.cast
    ├── events.jsonl
    └── markers.jsonl
```

Sessions capture terminal output, which is assumed to contain secrets:
directories are 0700 and files 0600 on Unix.
