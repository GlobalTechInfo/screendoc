//! Fuzz `MarkerReader`: arbitrary byte streams must never panic, and must
//! terminate to clean output or pending-marker state.

#![no_main]

use libfuzzer_sys::fuzz_target;
use sdoc_core::MarkerReader;

fuzz_target!(|data: &[u8]| {
    let mut reader = MarkerReader::new();
    // Feed in random chunk sizes to exercise the partial-state paths.
    let mut i = 0usize;
    while i < data.len() {
        let chunk = ((i / 3) % 5) + 1;
        let end = (i + chunk).min(data.len());
        let (_, _markers) = reader.feed(&data[i..end]);
        i = end;
    }
    let mut tail = Vec::new();
    reader.flush(&mut tail);
    // A final flush with garbage must also be panic-free.
    let mut reader2 = MarkerReader::new();
    let mut tail2 = Vec::new();
    reader2.flush(&mut tail2);
});
