//! screendoc core: event model, session storage, asciinema cast writing, marker demuxing.
//!
//! The source of truth for a recording is the event log (`events.jsonl`) plus
//! the raw asciinema byte stream (`raw.cast`). All derived outputs (GIF, MP4,
//! web player) are rendered from these two artifacts.

/// Asciinema 2.x cast-file writing.
pub mod cast;
/// Reading asciinema v2/v3 cast files into a unified event list.
pub mod cast_read;
/// User configuration loading.
pub mod config;
/// Optional at-rest encryption for session content files.
pub mod encryption;
/// Core error types.
pub mod err;
/// The structured event model.
pub mod event;
/// Inline marker encoding and byte-stream demultiplexing.
pub mod markers;
/// Secret redaction for recorded sessions.
pub mod redact;
/// On-disk session layout and lifecycle.
pub mod session;
/// Process-wide Ctrl-C flag (register-once handler).
pub mod signal;

pub use cast::CastWriter;
pub use cast_read::{CastEvent, ParsedCast, parse_cast_size, parse_cast_text};
pub use config::Config;
pub use encryption::{EncWriter, SessionKey};
pub use err::{CoreError, Result};
pub use event::{ChapterSource, Event, Meta, Mode, Size};
pub use markers::{Marker, MarkerReader, MarkerRecord, merge_markers, shorten};
pub use redact::{
    CompiledRule, RedactSummary, compile_pattern, mask_rules, parse_rules_file, redact_segments,
    redact_session, redact_session_rules, secret_rules,
};
pub use session::Session;
