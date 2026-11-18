# sdoc-core

screendoc core: event model, session storage, asciinema cast writing, marker
demuxing.

The source of truth for a recording is the event log (`events.jsonl`) plus
the raw asciinema byte stream (`raw.cast`). All derived outputs (GIF, MP4,
web player) are rendered from these two artifacts by `sdoc-render`.

This crate has no terminal I/O and no ffmpeg dependency; it is the safe,
portable foundation of the screendoc workspace.

- Documentation: <https://github.com/GlobalTechInfo/screendoc>
- License: MIT OR Apache-2.0
