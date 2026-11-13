//! Fuzz the encrypted-session record format: any byte stream fed to the
//! decryptor must produce a clean Result, never a panic (out-of-bounds,
//! integer overflow, or allocation blowup on hostile length fields).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The decryptor works on files; materialize the fuzz input behind a
    // throwaway path (one per fuzz binary, single-threaded).
    let dir = std::env::temp_dir().join(format!(
        "sdoc-encfuzz-{}",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("fuzz.enc");
    let _ = std::fs::write(&path, data);

    // A key is required; use a fixed one so the fuzzer explores record
    // structure, not key derivation.
    let key = sdoc_core::encryption::SessionKey::from_bytes([0u8; 32]);
    let _ = key.decrypt_stream(&path);
    // Plain-text files (no magic) must be a clean error too.
    let plain = dir.join("plain.enc");
    let _ = std::fs::write(&plain, data);
    let _ = key.decrypt_stream(&plain);
});
