use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::err::{CoreError, Result};
use crate::event::{Event, Meta};
use crate::markers::MarkerRecord;

/// The on-disk home of all screendoc data (`~/.screendoc`).
pub fn data_root() -> PathBuf {
    match dirs::home_dir() {
        Some(h) => h.join(".screendoc"),
        None => PathBuf::from(".screendoc"),
    }
}

/// Directory that holds all sessions.
pub fn sessions_dir() -> PathBuf {
    data_root().join("sessions")
}

/// Current wall-clock time in Unix milliseconds.
pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Process-local counter used to keep session ids unique within one process.
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a new opaque session id (timestamp-pid-counter, stable and sortable).
pub fn new_session_id() -> String {
    let n = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{ms}-{pid}-{n}", ms = now_unix_ms(), pid = std::process::id(), n = n)
}

/// A session and its on-disk layout.
#[derive(Debug, Clone)]
pub struct Session {
    id: String,
    dir: PathBuf,
    /// When true, the content files are encrypted record streams.
    encrypted: bool,
}

impl Session {
    /// Create a new session directory with empty logs and metadata.
    ///
    /// On Unix the directory is created `0700` and the log files `0600`:
    /// sessions capture terminal output, which may contain secrets.
    pub fn create(meta: &Meta) -> Result<Self> {
        Self::create_inner(meta, false)
    }

    /// Create a new session whose content files are encrypted at rest
    /// (AES-256-GCM under the user key file; created on demand).
    pub fn create_encrypted(meta: &Meta) -> Result<Self> {
        Self::create_inner(meta, true)
    }

    fn create_inner(meta: &Meta, encrypted: bool) -> Result<Self> {
        let dir = sessions_dir().join(&meta.id);
        restrict_dir(&dir)?;
        Self::save_meta_in(&dir, meta)?;
        create_private_file(dir.join("events.jsonl"))?;
        create_private_file(dir.join("markers.jsonl"))?;
        create_private_file(dir.join("raw.cast"))?;
        if encrypted {
            fs::File::create(dir.join(crate::encryption::ENCRYPTED_SIDECAR))?;
            // The key file is created on demand; the sidecar marks the
            // session so listing works without key material.
            crate::encryption::SessionKey::load_or_create()?;
        }
        Ok(Self { id: meta.id.clone(), dir, encrypted })
    }

    /// Open an existing session.
    ///
    /// The id is validated to be a single safe path component, so values
    /// like `../../etc` cannot escape the sessions directory.
    ///
    /// Opening never touches the encryption key: decryption happens
    /// lazily at the first content read, so listing and management work
    /// without key material.
    pub fn open(id: &str) -> Result<Self> {
        Self::validate_id(id)?;
        let dir = sessions_dir().join(id);
        if !dir.join("meta.json").is_file() {
            return Err(CoreError::SessionNotFound(id.to_string()));
        }
        let encrypted = Self::is_encrypted_in(&dir);
        Ok(Self { id: id.to_string(), dir, encrypted })
    }

    /// Whether this session's content files are encrypted at rest.
    ///
    /// Detected via the `encrypted` sidecar so it works without the key.
    pub fn is_encrypted(&self) -> bool {
        self.encrypted
    }

    /// The sidecar check, applied to a directory (used by `open`).
    fn is_encrypted_in(dir: &Path) -> bool {
        dir.join(crate::encryption::ENCRYPTED_SIDECAR).exists()
    }

    /// The encryption key for this session (only valid when encrypted).
    fn key(&self) -> Result<crate::encryption::SessionKey> {
        if !self.encrypted {
            return Err(CoreError::Other("session is not encrypted".to_string()));
        }
        crate::encryption::SessionKey::load_or_create()
    }

    /// A session id must be a single, non-traversal path component.
    pub fn validate_id(id: &str) -> Result<()> {
        if id.is_empty()
            || id == "."
            || id == ".."
            || id.contains('/')
            || id.contains('\\')
            || id.contains('\0')
        {
            return Err(CoreError::Other(format!("invalid session id `{id}`")));
        }
        Ok(())
    }

    /// Resolve a session from an explicit id, or fall back to the active one.
    pub fn resolve(id: Option<&str>) -> Result<Self> {
        match id {
            Some(i) => Self::open(i),
            None => {
                let active = Self::active_id().ok_or(CoreError::NoActiveSession)?;
                Self::open(&active)
            }
        }
    }

    /// List all sessions, newest first.
    pub fn list() -> Result<Vec<Meta>> {
        let mut metas = Vec::new();
        let read_dir = match fs::read_dir(sessions_dir()) {
            Ok(rd) => rd,
            Err(_) => return Ok(metas),
        };
        for entry in read_dir {
            let entry = entry?;
            let meta_path = entry.path().join("meta.json");
            if !meta_path.is_file() {
                continue;
            }
            if let Some(meta) = fs::read_to_string(&meta_path)
                .ok()
                .and_then(|t| serde_json::from_str::<Meta>(&t).ok())
            {
                metas.push(meta);
            }
        }
        metas.sort_by_key(|m| std::cmp::Reverse(m.started_unix_ms));
        Ok(metas)
    }

    /// The session id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The session directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Path to the event log.
    pub fn events_path(&self) -> PathBuf {
        self.dir.join("events.jsonl")
    }

    /// Path to the marker log.
    pub fn markers_path(&self) -> PathBuf {
        self.dir.join("markers.jsonl")
    }

    /// Path to the asciinema byte stream.
    pub fn cast_path(&self) -> PathBuf {
        self.dir.join("raw.cast")
    }

    /// Directory for rendered frame images.
    pub fn frames_dir(&self) -> PathBuf {
        self.dir.join("frames")
    }

    /// Path to the metadata file.
    pub fn meta_path(&self) -> PathBuf {
        self.dir.join("meta.json")
    }

    /// Load the persisted metadata.
    pub fn read_meta(&self) -> Result<Meta> {
        let text = fs::read_to_string(self.meta_path())?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Persist updated metadata (call when finalizing a session).
    pub fn save_meta(&self, meta: &Meta) -> Result<()> {
        Self::save_meta_in(&self.dir, meta)
    }

    fn save_meta_in(dir: &Path, meta: &Meta) -> Result<()> {
        let text = serde_json::to_string_pretty(meta)?;
        fs::write(dir.join("meta.json"), text)?;
        Ok(())
    }

    /// Append one event to the event log (encrypted at rest when the
    /// session is).
    pub fn append_event(&self, ev: &Event) -> Result<()> {
        use std::io::Write as _;
        let line = serde_json::to_string(ev)?;
        if self.encrypted {
            let key = self.key()?;
            let mut f =
                fs::OpenOptions::new().create(true).append(true).open(self.events_path())?;
            key.append_record(&mut f, line.as_bytes())?;
        } else {
            let mut f =
                fs::OpenOptions::new().create(true).append(true).open(self.events_path())?;
            writeln!(f, "{line}")?;
        }
        Ok(())
    }

    /// Append a marker record to the marker log (used by shell hooks),
    /// encrypted at rest when the session is.
    pub fn append_marker_record(&self, rec: &MarkerRecord) -> Result<()> {
        use std::io::Write as _;
        let line = serde_json::to_string(rec)?;
        if self.encrypted {
            let key = self.key()?;
            let mut f =
                fs::OpenOptions::new().create(true).append(true).open(self.markers_path())?;
            key.append_record(&mut f, line.as_bytes())?;
        } else {
            let mut f =
                fs::OpenOptions::new().create(true).append(true).open(self.markers_path())?;
            writeln!(f, "{line}")?;
        }
        Ok(())
    }

    /// Open a cast writer for this session (an encrypted record stream
    /// when the session is).
    pub fn cast_writer(&self, size: &crate::event::Size) -> Result<crate::CastWriter> {
        if self.encrypted {
            let key = self.key()?;
            let w = crate::encryption::EncWriter::new(&self.cast_path(), &key)?;
            Ok(crate::CastWriter::new_w(Box::new(w), *size)?)
        } else {
            Ok(crate::CastWriter::new(&self.cast_path(), *size)?)
        }
    }

    /// The plaintext cast text (decrypts transparently for encrypted
    /// sessions).
    pub fn read_cast_text(&self) -> Result<String> {
        if self.encrypted {
            let key = self.key()?;
            let bytes = key.decrypt_stream(&self.cast_path())?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        } else {
            fs::read_to_string(self.cast_path())
                .map_err(|e| CoreError::Other(format!("failed to read cast: {e}")))
        }
    }

    /// The plaintext event-log text (decrypts transparently).
    pub fn read_events_text(&self) -> Result<String> {
        if self.encrypted {
            let key = self.key()?;
            let bytes = key.decrypt_stream(&self.events_path())?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        } else {
            fs::read_to_string(self.events_path())
                .map_err(|e| CoreError::Other(format!("failed to read events: {e}")))
        }
    }

    /// Replace the cast file with `text` (encrypted at rest when the
    /// session is). Used by `redact`.
    pub fn write_cast_text(&self, text: &str) -> Result<()> {
        if self.encrypted {
            let key = self.key()?;
            let mut f = fs::File::create(self.cast_path())?;
            key.append_record(&mut f, text.as_bytes())?;
        } else {
            fs::write(self.cast_path(), text)?;
        }
        Ok(())
    }

    /// Replace the event log with `text` (encrypted at rest when the
    /// session is). Used by `redact`.
    pub fn write_events_text(&self, text: &str) -> Result<()> {
        if self.encrypted {
            let key = self.key()?;
            let mut f = fs::File::create(self.events_path())?;
            key.append_record(&mut f, text.as_bytes())?;
        } else {
            fs::write(self.events_path(), text)?;
        }
        Ok(())
    }

    /// Read all events, in log order (decrypted transparently for
    /// encrypted sessions).
    pub fn read_events(&self) -> Result<Vec<Event>> {
        let text = self.read_events_text()?;
        let mut events = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            events.push(serde_json::from_str(line)?);
        }
        Ok(events)
    }

    /// Read all marker records, in log order (decrypted transparently for
    /// encrypted sessions).
    pub fn read_marker_records(&self) -> Result<Vec<MarkerRecord>> {
        if self.encrypted {
            let key = self.key()?;
            let bytes = key.decrypt_stream(&self.markers_path())?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let mut records = Vec::new();
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                records.push(serde_json::from_str(line)?);
            }
            return Ok(records);
        }
        let text = fs::read_to_string(self.markers_path())
            .map_err(|e| CoreError::Other(format!("failed to read markers: {e}")))?;
        let mut records = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            records.push(serde_json::from_str(line)?);
        }
        Ok(records)
    }

    /// Delete the session and all its files.
    pub fn remove(&self) -> Result<()> {
        fs::remove_dir_all(&self.dir)?;
        Ok(())
    }

    /// Rename the session directory to `new_id`, updating the metadata id
    /// and the active pointer when it points at this session.
    pub fn rename(&self, new_id: &str) -> Result<Self> {
        Self::validate_id(new_id)?;
        if new_id == self.id {
            return Ok(self.clone());
        }
        let new_dir = sessions_dir().join(new_id);
        if new_dir.exists() {
            return Err(CoreError::Other(format!(
                "session `{new_id}` already exists; cannot rename to it"
            )));
        }
        // Read the metadata *before* the move: the old path is gone.
        let mut meta = self.read_meta()?;
        fs::rename(&self.dir, &new_dir)?;
        meta.id = new_id.to_string();
        Self::save_meta_in(&new_dir, &meta)?;
        let renamed = Self { id: new_id.to_string(), dir: new_dir, encrypted: self.encrypted };
        if Self::active_id().as_deref() == Some(self.id.as_str()) {
            renamed.set_active()?;
        }
        Ok(renamed)
    }

    /// Mark this session as the active one.
    pub fn set_active(&self) -> Result<()> {
        fs::create_dir_all(data_root())?;
        fs::write(Self::active_path(), &self.id)?;
        Ok(())
    }

    /// The active session id, if any.
    pub fn active_id() -> Option<String> {
        let text = fs::read_to_string(Self::active_path()).ok()?;
        let id = text.trim().to_string();
        (!id.is_empty()).then_some(id)
    }

    /// Clear the active pointer, but only when it still points at `id`
    /// (so a stale pointer from an older run cannot be clobbered).
    pub fn clear_active(id: &str) -> Result<()> {
        let path = Self::active_path();
        if fs::read_to_string(&path).ok().as_deref() == Some(id) {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    fn active_path() -> PathBuf {
        data_root().join("current")
    }
}

/// Create `path` with owner-only permissions (0700 on Unix; default
/// elsewhere). Used for the session directory.
#[cfg(unix)]
fn restrict_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(dir.join("frames"))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    fs::set_permissions(dir.join("frames"), fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Same as [`restrict_dir`] without the Unix-specific parts.
#[cfg(not(unix))]
fn restrict_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir.join("frames"))?;
    Ok(())
}

/// Create a new file with owner-only read/write (0600 on Unix).
#[cfg(unix)]
fn create_private_file(path: PathBuf) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let f =
        fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
    drop(f);
    Ok(())
}

/// Same as [`create_private_file`] without the Unix-specific parts.
#[cfg(not(unix))]
fn create_private_file(path: PathBuf) -> Result<()> {
    fs::File::create(&path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Mode;
    use crate::markers::Marker;

    fn test_meta() -> Meta {
        Meta {
            id: new_session_id(),
            started_unix_ms: now_unix_ms(),
            shell: Some("bash".into()),
            cwd: None,
            mode: Mode::Wrapper,
            video: true,
            pane_target: None,
            duration_ms: None,
            title: None,
            group: None,
        }
    }

    #[test]
    fn create_open_and_list() -> Result<()> {
        let meta = test_meta();
        let s = Session::create(&meta)?;
        let listed = Session::list()?;
        assert!(listed.iter().any(|m| m.id == meta.id));
        assert_eq!(s.id(), meta.id.as_str());
        s.remove()?;
        assert!(Session::open(&meta.id).is_err());
        Ok(())
    }

    #[test]
    fn unknown_session_fails() {
        assert!(Session::open("no-such-session").is_err());
    }

    #[test]
    fn session_ids_cannot_traverse_paths() {
        // Defense in depth: every public API that takes an id routes through
        // `open`, which must reject traversal and unsafe components.
        for bad in ["..", ".", "../etc", "..\\windows", "a/b", "a\\b", "a\0b", ""] {
            assert!(Session::open(bad).is_err(), "expected `{bad}` to be rejected");
            assert!(Session::validate_id(bad).is_err());
        }
        // A normal generated id still validates.
        assert!(Session::validate_id(&new_session_id()).is_ok());
    }

    #[test]
    fn event_and_marker_roundtrip() -> Result<()> {
        let meta = test_meta();
        let s = Session::create(&meta)?;
        let ev = Event::Chapter {
            at_ms: 42,
            title: "x".into(),
            source: crate::event::ChapterSource::Manual,
        };
        s.append_event(&ev)?;
        s.append_marker_record(&MarkerRecord {
            ts_unix_ms: now_unix_ms(),
            marker: Marker::Chapter { title: "y".into() },
        })?;

        assert_eq!(s.read_events()?, vec![ev]);
        assert_eq!(s.read_marker_records()?.len(), 1);
        s.remove()?;
        Ok(())
    }

    #[test]
    fn active_pointer_roundtrip() -> Result<()> {
        let meta = test_meta();
        let s = Session::create(&meta)?;
        s.set_active()?;
        assert_eq!(Session::active_id().as_deref(), Some(meta.id.as_str()));
        Session::clear_active(&meta.id)?;
        assert_eq!(Session::active_id(), None);
        s.remove()?;
        Ok(())
    }

    #[test]
    fn rename_updates_meta_and_refuses_existing() -> Result<()> {
        let m = test_meta();
        let s = Session::create(&m)?;
        let new_id = format!("renamed-{}", s.id());
        let renamed = s.rename(&new_id)?;
        assert_eq!(renamed.id(), &new_id);
        assert!(Session::open(&new_id).is_ok());
        assert!(Session::open(s.id()).is_err());
        // The renamed session's metadata carries the new id.
        assert_eq!(renamed.read_meta()?.id, new_id);
        // Refuse to rename onto an existing session.
        let m2 = test_meta();
        let other = Session::create(&m2)?;
        assert!(renamed.rename(other.id()).is_err());
        // A noop rename returns the same session.
        assert!(renamed.rename(renamed.id()).is_ok());
        renamed.remove()?;
        other.remove()?;
        Ok(())
    }

    /// Freshly created sessions get restrictive permissions on Unix:
    /// directory 0700, log files 0600 (sessions capture secrets).
    #[test]
    #[cfg(unix)]
    fn new_sessions_are_private_on_unix() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let m = test_meta();
        let s = Session::create(&m)?;
        let dir_mode = s.dir().symlink_metadata()?.permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700, "session dir must be 0700");
        for f in ["events.jsonl", "markers.jsonl", "raw.cast"] {
            let p = s.dir().join(f);
            let mode = p.symlink_metadata()?.permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "file {f} must be 0600");
        }
        s.remove()?;
        Ok(())
    }
}
