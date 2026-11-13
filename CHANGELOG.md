# CHANGELOG

All notable changes to this project are documented in this file, following
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-11-11

First release.

### Added

- Three recording modes: one-shot wrapper (`record`), ambient shell hooks
  (`on`/`off`), and tmux pane or whole-session recording — each producing a
  portable session: structured event journal, asciinema byte stream, command
  boundaries with exit codes, and chapters.
- Rendering from one recording: GIF, MP4, WebM (VP9), AV1, per-chapter
  videos, still frames, asciinema cast export, a local web player, and a
  self-contained shareable `session.html` (`render --web`).
- Terminal replay with speed control and time windows (`play --from/--to`).
- Asciinema v2/v3 import, plain or gzip-compressed.
- Redaction: ad-hoc regex, built-in secret detectors (`redact --secret`),
  custom rule files, all ANSI-segment-aware, with per-rule JSON summaries.
- Session lifecycle: rename, prune, automatic retention (`retention_days`),
  search over commands and output, JSON/CSV machine output, config
  subcommand.
- Security: owner-only session files (0700/0600 on Unix) and optional
  AES-256-GCM encryption at rest.
- Quality tooling: four libFuzzer targets (marker, cast, redact, enc),
  a 46-check no-suppression valgrind sweep on a dedicated unoptimized
  profile, a 30-minute performance/soak check, and a dependency license
  audit with allowlist (`docs/LICENSES.md`).
- Packaging: pinned exact dependency versions, fully static musl builds,
  homebrew formula, and a tag-triggered release workflow with SHA-256
  checksums.
