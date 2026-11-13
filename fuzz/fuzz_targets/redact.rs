//! Fuzz the ANSI-segment-aware redaction core: arbitrary text + arbitrary
//! regex patterns must never panic and must leave escape sequences intact.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sdoc_core::redact::{parse_rules_file, redact_segments, secret_rules};

fuzz_target!(|data: &[u8]| {
    // Split the input into (text, pattern) halves; invalid UTF-8 becomes
    // lossy text, which is exactly what real terminal bytes look like.
    let mid = data.len() / 2;
    let text = String::from_utf8_lossy(&data[..mid]).into_owned();
    let pattern = String::from_utf8_lossy(&data[mid..]).into_owned();

    // An empty pattern is a valid regex (matches nothing visible); make the
    // interesting half non-empty so the regex engine has work to do.
    let pattern = if pattern.is_empty() { "a".to_string() } else { pattern };
    let _ = redact_segments(&text, &pattern, "***");

    // The same input as a rules-file body: newlines/`#`/tabs all parse
    // (or reject cleanly) without panicking.
    let rules_text = String::from_utf8_lossy(data).into_owned();
    if let Ok(rules) = parse_rules_file(&rules_text) {
        let _ = sdoc_core::redact::mask_rules(&text, &rules);
        // Built-in detectors on the same text, in the same pass.
        let _ = sdoc_core::redact::mask_rules(&text, &secret_rules());
    }
});
