//! Optional at-rest encryption for session files.
//!
//! Sessions capture terminal output, which may contain secrets. With
//! `--encrypt`, the content files (`events.jsonl`, `markers.jsonl`,
//! `raw.cast`) are stored as per-line AES-256-GCM records under a
//! user-held key file (`~/.screendoc/key`, mode 0600). `meta.json` and
//! the `encrypted` sidecar stay plaintext so session listing keeps
//! working without touching the key.
//!
//! File format (magic + record stream):
//!
//! ```text
//! "SDOC1ENC"                     ; 8-byte magic
//! repeat:
//!   [u32 LE total_payload_len]   ; payload = nonce (12) || ciphertext+tag
//!   nonce (12 bytes, random)
//!   ciphertext || GCM tag (16)
//! ```
//!
//! Every record is authenticated, so truncation or bit-flipping is
//! detected at read time; a missing/short record is a clean error, never
//! a panic (fuzzed: `fuzz/fuzz_targets/enc.rs`).

use std::fs;
use std::io::Write;

use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit},
};
use getrandom::fill;

use crate::err::{CoreError, Result};

/// The magic prefix of an encrypted session file.
pub const MAGIC: &[u8] = b"SDOC1ENC";

/// The key file name inside the data root.
const KEY_FILE: &str = "key";

/// The `encrypted` sidecar that marks a session without opening the key.
pub const ENCRYPTED_SIDECAR: &str = "encrypted";

/// A session encryption key (256-bit, kept in a 0600 file).
///
/// Display hides the material; the key is secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionKey([u8; 32]);

impl SessionKey {
    /// Construct a key from its 32 raw bytes (used by tests and fuzz
    /// targets; production code goes through [`Self::load_or_create`]).
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The data-root path of the key file.
    pub fn key_path() -> std::path::PathBuf {
        crate::session::data_root().join(KEY_FILE)
    }

    /// Load the key file, creating it (32 random bytes, mode 0600 on
    /// Unix) when absent.
    pub fn load_or_create() -> Result<Self> {
        let path = Self::key_path();
        if path.exists() {
            let bytes = fs::read(&path)
                .map_err(|e| CoreError::Other(format!("failed to read key file: {e}")))?;
            if bytes.len() != 32 {
                return Err(CoreError::Decryption(format!(
                    "key file {} has {} bytes; expected 32",
                    path.display(),
                    bytes.len()
                )));
            }
            let mut k = [0u8; 32];
            k.copy_from_slice(&bytes);
            return Ok(Self(k));
        }
        let mut k = [0u8; 32];
        fill(&mut k).map_err(|e| CoreError::Other(format!("CSPRNG unavailable: {e}")))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::File::create(&path)?;
        f.write_all(&k)?;
        restrict_key_file(&path);
        Ok(Self(k))
    }

    /// Encrypt `line` into one authenticated record appended to `file`.
    /// Creates the file with the magic when it is new.
    pub fn append_record(&self, file: &mut fs::File, line: &[u8]) -> Result<()> {
        if file.metadata().map(|m| m.len() == 0).unwrap_or(false) {
            file.write_all(MAGIC)?;
        }
        let mut nonce = [0u8; 16];
        fill(&mut nonce).map_err(|e| CoreError::Other(format!("CSPRNG unavailable: {e}")))?;
        let cipher = Aes256Gcm::new_from_slice(&self.0)
            .map_err(|e| CoreError::Other(format!("invalid key: {e}")))?;
        let nonce12 = &nonce[..12];
        let nonce = generic_array::GenericArray::from_slice(nonce12).to_owned();
        let ct = cipher
            .encrypt(&nonce, line)
            .map_err(|e| CoreError::Other(format!("encryption failed: {e}")))?;
        let payload_len = nonce12.len() + ct.len();
        let mut out = Vec::with_capacity(4 + payload_len);
        out.extend_from_slice(&(payload_len as u32).to_le_bytes());
        out.extend_from_slice(nonce12);
        out.extend_from_slice(&ct);
        file.write_all(&out)?;
        Ok(())
    }

    /// Decrypt a whole encrypted file into plaintext.
    ///
    /// A file without the magic is returned as an error (it is either a
    /// plaintext file or corrupt); a truncated record or a failed
    /// authentication is a [`CoreError::Decryption`].
    pub fn decrypt_stream(&self, path: &std::path::Path) -> Result<Vec<u8>> {
        let bytes = fs::read(path)
            .map_err(|e| CoreError::Other(format!("failed to read {}: {e}", path.display())))?;
        if !bytes.starts_with(MAGIC) {
            return Err(CoreError::Decryption(format!(
                "{} is not an encrypted session file",
                path.display()
            )));
        }
        let bytes = &bytes[MAGIC.len()..];
        let cipher = Aes256Gcm::new_from_slice(&self.0)
            .map_err(|e| CoreError::Other(format!("invalid key: {e}")))?;
        let mut out = Vec::new();
        let mut off = 0usize;
        while off < bytes.len() {
            let len_u32: [u8; 4] = bytes
                .get(off..off + 4)
                .ok_or_else(|| CoreError::Decryption("truncated record header".into()))?
                .try_into()
                .expect("4 bytes");
            let len = u32::from_le_bytes(len_u32) as usize;
            off += 4;
            let payload = bytes
                .get(off..off + len)
                .ok_or_else(|| CoreError::Decryption("truncated record".into()))?;
            if payload.len() < 12 {
                return Err(CoreError::Decryption("short record (missing nonce)".into()));
            }
            let (nonce, ct) = payload.split_at(12);
            let pt = cipher
                .decrypt(&generic_array::GenericArray::from_slice(nonce).to_owned(), ct)
                .map_err(|_| {
                    CoreError::Decryption(
                        "authentication failed (wrong key or corrupt file)".into(),
                    )
                })?;
            out.extend_from_slice(&pt);
            off += len;
        }
        Ok(out)
    }
}

/// A `Write` that appends each `write` call as one encrypted record.
///
/// Used as the cast-writer sink for encrypted sessions: the cast writer
/// formats one JSON line per `write`, so the record boundaries stay
/// meaningful even if multiple writes arrive.
pub struct EncWriter {
    file: fs::File,
    key: SessionKey,
}

impl EncWriter {
    /// Open (create) the encrypted file for `path` with the key.
    pub fn new(path: &std::path::Path, key: &SessionKey) -> Result<Self> {
        let file = fs::OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { file, key: key.clone() })
    }

    /// The underlying file, for length checks.
    pub fn file(&self) -> &fs::File {
        &self.file
    }
}

impl Write for EncWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.key.append_record(&mut self.file, buf).map_err(std::io::Error::other)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Set the key file to owner-only.
#[cfg(unix)]
fn restrict_key_file(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mut perm = meta.permissions();
        perm.set_mode(0o600);
        let _ = fs::set_permissions(path, perm);
    }
}

#[cfg(not(unix))]
fn restrict_key_file(_path: &std::path::Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn key_roundtrip_and_modes() {
        // Uses the real data root; the key file must be 0600 on Unix.
        let key = SessionKey::load_or_create().expect("key");
        let path = SessionKey::key_path();
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "key file must be 0600");
        }
        // Second load returns the same key.
        assert_eq!(key, SessionKey::load_or_create().unwrap());
        let _ = key;
    }

    #[test]
    fn record_stream_roundtrip() {
        let dir = temp_dir();
        let p = dir.path().join("data.enc");
        let key = SessionKey::load_or_create().unwrap();
        let mut f = fs::File::create(&p).unwrap();
        key.append_record(&mut f, b"line one\n").unwrap();
        key.append_record(&mut f, b"line two\n").unwrap();
        let pt = key.decrypt_stream(&p).unwrap();
        assert_eq!(String::from_utf8(pt).unwrap(), "line one\nline two\n");
    }

    #[test]
    fn wrong_key_fails_cleanly() {
        let dir = temp_dir();
        let p = dir.path().join("data.enc");
        let key = SessionKey::load_or_create().unwrap();
        let mut f = fs::File::create(&p).unwrap();
        key.append_record(&mut f, b"secret\n").unwrap();

        // A key that differs in one byte must fail authentication,
        // not panic.
        let mut wrong_bytes = key.0;
        wrong_bytes[0] ^= 0xff;
        let wrong = SessionKey(wrong_bytes);
        let err = wrong.decrypt_stream(&p).unwrap_err();
        assert!(matches!(err, CoreError::Decryption(_)));
    }

    #[test]
    fn tampered_ciphertext_is_detected() {
        let dir = temp_dir();
        let p = dir.path().join("data.enc");
        let key = SessionKey::load_or_create().unwrap();
        let mut f = fs::File::create(&p).unwrap();
        key.append_record(&mut f, b"secret\n").unwrap();
        // Flip a byte inside the ciphertext region (after the magic +
        // 4-byte length + 12-byte nonce).
        let mut raw = fs::read(&p).unwrap();
        raw[MAGIC.len() + 4 + 12 + 2] ^= 0x01;
        fs::write(&p, &raw).unwrap();
        let err = key.decrypt_stream(&p).unwrap_err();
        assert!(matches!(err, CoreError::Decryption(_)));
    }

    #[test]
    fn truncated_stream_fails_cleanly() {
        let dir = temp_dir();
        let p = dir.path().join("data.enc");
        let key = SessionKey::load_or_create().unwrap();
        let mut f = fs::File::create(&p).unwrap();
        key.append_record(&mut f, b"line\n").unwrap();
        let mut raw = fs::read(&p).unwrap();
        raw.truncate(raw.len() - 3); // cut inside the record
        fs::write(&p, &raw).unwrap();
        assert!(matches!(key.decrypt_stream(&p).unwrap_err(), CoreError::Decryption(_)));
    }

    #[test]
    fn plaintext_file_is_rejected() {
        let dir = temp_dir();
        let p = dir.path().join("plain.enc");
        fs::write(&p, b"no magic here").unwrap();
        let key = SessionKey::load_or_create().unwrap();
        assert!(matches!(key.decrypt_stream(&p).unwrap_err(), CoreError::Decryption(_)));
    }

    #[test]
    fn hostile_lengths_do_not_panic() {
        // A forged length field pointing past EOF must error, never
        // panic (the fuzz target `enc` drives this path with garbage).
        let dir = temp_dir();
        let p = dir.path().join("forged.enc");
        let mut raw = Vec::new();
        raw.extend_from_slice(MAGIC);
        raw.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        raw.extend_from_slice(b"short");
        fs::write(&p, &raw).unwrap();
        let key = SessionKey::load_or_create().unwrap();
        assert!(matches!(key.decrypt_stream(&p).unwrap_err(), CoreError::Decryption(_)));
    }

    #[test]
    fn enc_writer_buffers_multiple_writes_as_records() {
        let dir = temp_dir();
        let p = dir.path().join("w.enc");
        let key = SessionKey::load_or_create().unwrap();
        let mut w = EncWriter::new(&p, &key).unwrap();
        w.write_all(b"abc").unwrap();
        w.write_all(b"def\n").unwrap();
        let pt = key.decrypt_stream(&p).unwrap();
        assert_eq!(String::from_utf8(pt).unwrap(), "abcdef\n");
    }
}
