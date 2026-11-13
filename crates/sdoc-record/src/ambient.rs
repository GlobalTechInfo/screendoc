use sdoc_core::{
    CoreError, Marker, MarkerRecord, Meta, Mode, Session, merge_markers,
    session::{new_session_id, now_unix_ms},
};

use crate::hooks;

/// Options for starting an ambient (journal) recording.
pub struct OnOptions {
    /// Shell to generate hooks for. Detected from `$SHELL` when `None`.
    pub shell: Option<String>,
    /// Encrypt the session's content files at rest (AES-256-GCM).
    pub encrypt: bool,
}

/// Result of starting an ambient recording.
pub struct OnResult {
    /// The created session.
    pub session: Session,
    /// The shell the hooks were generated for.
    pub shell: String,
    /// The `eval` line the user must run in their running shell.
    pub eval: String,
}

/// Summary of a stopped ambient recording.
pub struct OffSummary {
    /// The finalized session.
    pub session: Session,
    /// Number of command events merged into the event log.
    pub commands: usize,
}

/// Start an ambient recording session.
///
/// Creates the session, marks it active, and returns the `eval` snippet the
/// user runs in their *running* shell to install the hooks.
pub fn on(opts: &OnOptions) -> Result<OnResult, CoreError> {
    let shell = opts.shell.clone().unwrap_or_else(|| hooks::detect_shell().to_string());
    if !hooks::SUPPORTED_SHELLS.contains(&shell.as_str()) {
        return Err(CoreError::Other(format!(
            "unsupported shell: {shell} (supported: {})",
            hooks::SUPPORTED_SHELLS.join(", ")
        )));
    }

    let meta = Meta {
        id: new_session_id(),
        started_unix_ms: now_unix_ms(),
        shell: Some(shell.clone()),
        cwd: None,
        mode: Mode::Journal,
        video: false,
        pane_target: None,
        duration_ms: None,
        title: None,
        group: None,
    };
    let session =
        if opts.encrypt { Session::create_encrypted(&meta)? } else { Session::create(&meta)? };
    session.set_active()?;

    let eval = format!("eval \"$(screendoc hook-script {shell})\"");
    Ok(OnResult { session, shell, eval })
}

/// Print the hook snippet for a running shell (reads the active session).
pub fn hook_script(shell: &str) -> Result<String, CoreError> {
    let id = Session::active_id()
        .ok_or_else(|| CoreError::Other("no active session; run `screendoc on` first".into()))?;
    hooks::ambient_hook_script(shell, &id)
        .ok_or_else(|| CoreError::Other(format!("unsupported shell: {shell}")))
}

/// Stop the ambient recording and merge markers into the event log.
pub fn off(id: Option<&str>) -> Result<OffSummary, CoreError> {
    let session = Session::resolve(id)?;
    let meta = session.read_meta()?;

    let records = session.read_marker_records()?;
    let events = session.read_events()?;

    let mut commands = 0usize;
    if !events.iter().any(|e| matches!(e, sdoc_core::Event::SessionEnd { .. })) {
        let merged = merge_markers(&records, meta.started_unix_ms);
        for ev in &merged {
            if matches!(ev, sdoc_core::Event::Command { .. }) {
                commands += 1;
            }
            session.append_event(ev)?;
        }
        session.append_event(&sdoc_core::Event::SessionEnd {
            at_ms: now_unix_ms().saturating_sub(meta.started_unix_ms),
            reason: "stopped".into(),
        })?;
    }

    let started = meta.started_unix_ms;
    let mut finalized = meta;
    if finalized.duration_ms.is_none() {
        finalized.duration_ms = Some(now_unix_ms().saturating_sub(started));
    }
    session.save_meta(&finalized)?;
    Session::clear_active(session.id())?;

    Ok(OffSummary { session, commands })
}

/// Append a structured marker to the active session's marker log.
///
/// Called by shell hooks via `screendoc hook-append`.
pub fn append_marker(
    kind: &str,
    seq: Option<u64>,
    rc: Option<i32>,
    text: Option<String>,
    session_id: Option<&str>,
) -> Result<(), CoreError> {
    let id = resolve_session_id(session_id)?;
    let session = Session::open(&id)?;
    let marker = match kind {
        "cmd" => Marker::Cmd {
            seq: seq.ok_or(CoreError::Other("missing --seq".into()))?,
            exit_code: rc.unwrap_or(-1),
            command: text.unwrap_or_default(),
        },
        "cmd-start" => Marker::CmdStart {
            seq: seq.ok_or(CoreError::Other("missing --seq".into()))?,
            command: text.unwrap_or_default(),
        },
        "cmd-end" => Marker::CmdEnd {
            seq: seq.ok_or(CoreError::Other("missing --seq".into()))?,
            exit_code: rc.unwrap_or(-1),
        },
        "chapter" => {
            Marker::Chapter { title: text.ok_or(CoreError::Other("missing --text".into()))? }
        }
        _ => {
            return Err(CoreError::Other(format!(
                "unknown marker kind: {kind} (expected cmd, cmd-start, cmd-end, chapter)"
            )));
        }
    };
    session.append_marker_record(&MarkerRecord { ts_unix_ms: now_unix_ms(), marker })?;
    Ok(())
}

/// Resolve which session an operation applies to: explicit id, then the
/// `SCREENDOC_SESSION` environment variable, then the active pointer.
pub fn resolve_session_id(explicit: Option<&str>) -> Result<String, CoreError> {
    if let Some(e) = explicit {
        return Ok(e.to_string());
    }
    if let Ok(env) = std::env::var("SCREENDOC_SESSION")
        && !env.is_empty()
    {
        return Ok(env);
    }
    Session::active_id().ok_or(CoreError::NoActiveSession)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `on`/`off` mutate the global active-session pointer, so tests that
    /// create ambient sessions must run one at a time.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn on_produces_eval_snippet() -> Result<(), CoreError> {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let res = on(&OnOptions { shell: Some("bash".into()), encrypt: false })?;
        assert!(res.eval.contains("screendoc hook-script bash"));
        assert_eq!(Session::active_id().as_deref(), Some(res.session.id()));
        Session::clear_active(res.session.id())?;
        res.session.remove()?;
        Ok(())
    }

    #[test]
    fn unsupported_shell_rejected() {
        let res = on(&OnOptions { shell: Some("tcsh".into()), encrypt: false });
        assert!(res.is_err());
    }

    #[test]
    fn off_without_active_session_errors() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Ensure no active session, then `off` without an explicit id must
        // report NoActiveSession (not panic, not create anything).
        if let Some(active) = Session::active_id() {
            let _ = Session::clear_active(&active);
        }
        match off(None) {
            Err(e) => assert!(matches!(e, CoreError::NoActiveSession)),
            Ok(_) => panic!("expected NoActiveSession"),
        }
    }

    #[test]
    fn off_unknown_session_id_errors() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(off(Some("definitely-not-a-session")).is_err());
    }

    #[test]
    fn off_merges_markers() -> Result<(), CoreError> {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let res = on(&OnOptions { shell: Some("zsh".into()), encrypt: false })?;
        let id = res.session.id().to_string();
        res.session.append_marker_record(&MarkerRecord {
            ts_unix_ms: now_unix_ms(),
            marker: Marker::CmdStart { seq: 1, command: "make build".into() },
        })?;
        res.session.append_marker_record(&MarkerRecord {
            ts_unix_ms: now_unix_ms(),
            marker: Marker::CmdEnd { seq: 1, exit_code: 0 },
        })?;
        let summary = off(Some(&id))?;
        assert_eq!(summary.commands, 1);
        let events = summary.session.read_events()?;
        assert!(events.iter().any(
            |e| matches!(e, sdoc_core::Event::Command { command, .. } if command == "make build")
        ));
        summary.session.remove()?;
        Ok(())
    }
}
