//! Fuzz the unified cast reader: arbitrary text must parse without
//! panicking (bad lines are skipped, never rejected).

#![no_main]

use libfuzzer_sys::fuzz_target;
use sdoc_core::cast_read::parse_cast_text;

fuzz_target!(|data: &str| {
    let parsed = parse_cast_text(data);
    // The reader is total: it never errors, only skips unreadable lines.
    assert!(parsed.is_ok(), "cast reader must not fail on any input");
});
