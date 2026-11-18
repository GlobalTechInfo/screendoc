# CHANGELOG

All notable changes to this project are documented in this file, following
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.2] - 2026-11-18

### Fixed

- **Homebrew formula staleness**: `packaging/homebrew/screendoc.rb` had
  been frozen at v0.1.0 with `sha256 "TODO"` placeholders, so it was
  unusable via Homebrew. Backfilled to v0.4.1 with real checksums and a
  livecheck block.
- **Formula auto-maintenance**: the release workflow now regenerates
  the formula on `main` after every tagged release (version + checksums
  from the release's `.sha256` assets, committed via the contents API),
  so `brew install` always tracks the newest release.

### Changed

- README: Homebrew install instructions (from the in-repo formula) and
  note that the formula is auto-backfilled by the release workflow.

## [0.4.1] - 2026-11-18

### Fixed

- **deb/rpm package layout** (v0.4.0 shipped- bug): the distro job
  extracted the release tarball into the package root, so the .deb/.rpm
  installed the binary at `/screendoc` (filesystem root) with an empty
  `/usr/local/bin/` directory. Now extracted straight into
  `usr/local/bin/` → packages install `/usr/local/bin/screendoc` with
  no stray files.

## [0.4.0] - 2026-11-18

### Added

- **Supply-chain provenance**: every release artifact is attested with
  SLSA v1 provenance (Sigstore `actions/attest-build-provenance`), so
  downstream users can verify `which CI job built this binary` without
  trusting the release page.
- **Debian and RPM packages** (`screendoc-*-x86_64-linux.deb` /
  `.rpm`) ship alongside the tarballs; the release checksums, GPG
  signatures and the installer all cover them.
- **Published library crates**: `sdoc-core`, `sdoc-render` and
  `sdoc-record` are now publishable on crates.io (per-crate READMEs and
  LICENSE files included) for embedding the pipeline in other tools; the
  CLI crate stays private.
- **Rust API reference on GitHub Pages** (the docs site now builds
  `cargo doc` and serves it under `/api/`), plus a CI `doc` job that
  fails on any rustdoc warning or unresolved intra-doc link.
  (Fixed two broken intra-doc links found by that gate.)

## [0.3.0] - 2026-11-17

### Fixed

- **Encrypted session journals are now readable after decryption.**
  `record --encrypt` stored each event-log and marker-log record without a
  line terminator, so a session with more than one event decrypted to a
  single concatenated line and failed to parse (`json error: trailing
  characters`). Every record now carries its own newline; covered by a
  unit round-trip and the encrypted e2e test.

### Added

- **Runnable examples** (`examples/`, driven by CI): record-and-render,
  chapters + redaction, local web serving, and encrypted recording — each
  end-to-end verifiable in a sandboxed home directory.
- **Criterion micro-benchmarks** for the hot paths (rule masking, marker
  reading, cast parsing, idle compression) with a fast-mode CI job.
- **Documentation**: `docs/design.md` (architecture, data model,
  marker demux, render pipeline, encryption), `docs/user-guide.md`, and
  `docs/threat-model.md` (T1–T6 with mitigations and residual risk).
- **CI hardening**: MSRV job (Rust 1.99 `cargo check`), line-coverage
  gate (`cargo-llvm-cov`, `--fail-under-lines 70`), unused-dependency
  audit on nightly, and dedicated `examples` and `bench` jobs.

## [0.2.0] - 2026-11-17

### Fixed

- **Recorded terminal size now drives every raster render** (video, GIF,
  stills, chapters). Previously the cast header's `terminal_width`/
  `terminal_height` was dropped, so an 80x24 session rendered at the 120x30
  default. The session start size and the cast header are now honored as
  fallbacks, with explicit resize events still taking precedence.
- **Empty recordings render a minimum-length video instead of an
  unplayable single-frame artifact.** A session that captured no terminal
  output (e.g. `screendoc record true`) now yields a short, playable clip
  instead of a 1-frame 0.07 s GIF/MP4, and `record` reports it with a
  warning so the degenerate case is no longer silent.

### Added

- Release pipeline ships a **CycloneDX 1.6 SBOM** (`SBOM.json`) alongside
  the binary artifacts, signed and digested like the rest of the release.
- End-to-end regression coverage: render tests now assert GIF frame counts
  and output dimensions, including the recorded-size and empty-session
  cases that produced the defects above.

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
