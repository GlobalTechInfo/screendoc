use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek, SeekFrom};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use sdoc_core::{
    CastWriter, Event, Marker, MarkerReader, Meta, Mode, Session, Size,
    session::{new_session_id, now_unix_ms},
};

/// A running tmux pane recording.
///
/// Created by [`start`], fed by repeated [`poll`] calls, and finished with
/// [`stop`].
pub struct TmuxRecorder {
    session: Session,
    target: String,
    tmux: PathBuf,
    raw: File,
    raw_pos: u64,
    marker_pos: u64,
    t0_unix_ms: u64,
    demuxer: MarkerReader,
    pending: HashMap<u64, (u64, String)>,
    cast: CastWriter,
}

/// Whether a usable `tmux` binary is in `PATH`.
pub fn tmux_available() -> bool {
    find_in_path("tmux").is_some()
}

/// Locate a binary in `PATH`.
fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Parse `tmux display-message -p "#{pane_width} #{pane_height}"` output
/// into a `(cols, rows)` pair.
pub fn parse_pane_size(output: &str) -> Result<(u16, u16)> {
    output
        .trim()
        .split_once(' ')
        .and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)))
        .ok_or_else(|| anyhow::anyhow!("unexpected tmux pane size output"))
}

/// Index of the last byte in `data` that is a newline, if any.
fn last_newline(data: &[u8]) -> Option<usize> {
    data.iter().rposition(|&b| b == b'\n')
}

/// Summary of a stopped tmux recording.
pub struct TmuxSummary {
    /// The finalized session.
    pub session: Session,
    /// Number of command events captured.
    pub commands: usize,
}

/// Start recording the given tmux pane (default: the current pane `#P`).
pub fn start(target: Option<&str>, encrypt: bool) -> Result<TmuxRecorder> {
    let tmux = find_in_path("tmux").ok_or_else(|| {
        anyhow::anyhow!(
            "tmux not found in PATH; install tmux (e.g. `brew install tmux` / `apt install tmux`)"
        )
    })?;

    let resolved = match target {
        Some(t) if !t.is_empty() => t.to_string(),
        Some(_) => bail!(
            "--tmux requires a target, e.g. `screendoc record --tmux main:0` (or `#P` inside tmux)"
        ),
        None => match std::env::var("TMUX") {
            Ok(_) => "#P".to_string(),
            Err(_) => bail!(
                "no tmux target given and not running inside tmux; pass one, e.g. `--tmux main:0`"
            ),
        },
    };

    let probe = std::process::Command::new(&tmux)
        .arg("display-message")
        .arg("-p")
        .arg("-t")
        .arg(&resolved)
        .arg("#{pane_width} #{pane_height}")
        .output()
        .context("failed to run `tmux` — is a server running?")?;
    if !probe.status.success() {
        let stderr = String::from_utf8_lossy(&probe.stderr);
        bail!("tmux target `{resolved}` not found: {stderr}");
    }
    let (cols, rows) = parse_pane_size(&String::from_utf8_lossy(&probe.stdout))?;

    let meta = Meta {
        id: new_session_id(),
        started_unix_ms: now_unix_ms(),
        shell: None,
        cwd: None,
        mode: Mode::Tmux,
        video: true,
        pane_target: Some(resolved.clone()),
        duration_ms: None,
        title: Some(format!("tmux {resolved}")),
        group: None,
    };
    let session = if encrypt { Session::create_encrypted(&meta)? } else { Session::create(&meta)? };
    session.set_active()?;

    let raw_path = session.dir().join("raw.bin");
    File::create(&raw_path)?;
    let raw = OpenOptions::new().read(true).open(&raw_path)?;

    let pipe_cmd =
        format!("cat >> '{}'", raw_path.to_string_lossy().to_string().replace('\'', "'\\''"));
    let enable = std::process::Command::new(&tmux)
        .arg("pipe-pane")
        .arg("-t")
        .arg(&resolved)
        .arg("-o")
        .arg(&pipe_cmd)
        .status()
        .context("failed to enable tmux pipe-pane")?;
    if !enable.success() {
        bail!("tmux pipe-pane failed for target `{resolved}`");
    }

    let cast = session.cast_writer(&Size { cols, rows })?;
    session.append_event(&Event::SessionStart {
        at_ms: 0,
        shell: None,
        size: Size { cols, rows },
    })?;

    Ok(TmuxRecorder {
        session,
        target: resolved,
        tmux,
        raw,
        raw_pos: 0,
        marker_pos: 0,
        t0_unix_ms: meta.started_unix_ms,
        demuxer: MarkerReader::new(),
        pending: HashMap::new(),
        cast,
    })
}

/// A multi-pane tmux recording: one linked session per pane of a tmux
/// session, all streamed concurrently.
pub struct TmuxGroup {
    recs: Vec<TmuxRecorder>,
}

impl TmuxGroup {
    /// The session ids, one per recorded pane.
    pub fn ids(&self) -> Vec<String> {
        self.recs.iter().map(|r| r.id().to_string()).collect()
    }

    /// Feed all panes.
    pub fn poll(&mut self) -> Result<usize> {
        self.recs.iter_mut().map(|r| r.poll()).sum()
    }

    /// Stop every pane recording and finalize each session.
    pub fn stop(mut self) -> Result<Vec<TmuxSummary>> {
        self.recs.drain(..).map(|r| r.stop()).collect::<Result<Vec<_>>>()
    }
}

/// Start recording **every pane** of a tmux session as linked sessions.
///
/// Each pane becomes its own session (with `group` set to the tmux session
/// name and `pane_target` to the pane id), so the whole layout is captured
/// and each pane can be rendered individually.
pub fn start_group(tmux_session: &str, encrypt: bool) -> Result<TmuxGroup> {
    let tmux = find_in_path("tmux").ok_or_else(|| {
        anyhow::anyhow!(
            "tmux not found in PATH; install tmux (e.g. `brew install tmux` / `apt install tmux`)"
        )
    })?;

    let out = std::process::Command::new(&tmux)
        .args(["list-panes", "-t", tmux_session, "-F", "#{pane_id} #{pane_width} #{pane_height}"])
        .output()
        .context("failed to run `tmux` — is a server running?")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("tmux session `{tmux_session}` not found: {stderr}");
    }
    let panes: Vec<(String, u16, u16)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, ' ');
            Some((it.next()?.to_string(), it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect();
    if panes.is_empty() {
        bail!("tmux session `{tmux_session}` has no panes");
    }

    let mut recs = Vec::new();
    for (pane_id, _, _) in &panes {
        let rec = start(Some(pane_id), encrypt)?;
        // Link this session to the group and label it with its pane.
        let meta = rec.session.read_meta()?;
        let mut linked = meta;
        linked.group = Some(tmux_session.to_string());
        linked.title = Some(format!("tmux {tmux_session} {pane_id}"));
        rec.session.save_meta(&linked)?;
        recs.push(rec);
    }
    Ok(TmuxGroup { recs })
}

impl TmuxRecorder {
    /// The recorded session id.
    pub fn id(&self) -> &str {
        self.session.id()
    }

    /// Process all new raw bytes and marker records since the last poll.
    ///
    /// Returns the number of events appended.
    pub fn poll(&mut self) -> Result<usize> {
        let mut n = 0usize;

        self.raw.seek(SeekFrom::Start(self.raw_pos))?;
        let mut chunk = Vec::new();
        let read = self.raw.read_to_end(&mut chunk)?;
        self.raw_pos += read as u64;
        if !chunk.is_empty() {
            let at_ms = now_unix_ms().saturating_sub(self.t0_unix_ms);
            let (clean, markers) = self.demuxer.feed(&chunk);
            if !clean.is_empty() {
                self.cast.output(at_ms, &String::from_utf8_lossy(&clean))?;
            }
            for m in markers {
                n += self.append_marker_events(&m, at_ms)?;
            }
        }

        n += self.poll_markers()?;
        Ok(n)
    }

    /// The target pane id this recorder streams from.
    pub fn target(&self) -> &str {
        &self.target
    }

    fn append_marker_events(&mut self, m: &Marker, at_ms: u64) -> Result<usize> {
        match m {
            Marker::CmdStart { seq, command } => {
                self.pending.insert(*seq, (at_ms, command.clone()));
                Ok(0)
            }
            Marker::CmdEnd { seq, exit_code } => {
                let (command_start_ms, command) =
                    self.pending.remove(seq).unwrap_or((at_ms, String::new()));
                let title = sdoc_core::shorten(&command);
                let n = 1;
                if !command.is_empty() {
                    self.session.append_event(&Event::Chapter {
                        at_ms: command_start_ms,
                        title,
                        source: sdoc_core::ChapterSource::Auto,
                    })?;
                }
                self.session.append_event(&Event::Command {
                    at_ms,
                    seq: *seq,
                    exit_code: *exit_code,
                    command,
                })?;
                Ok(n)
            }
            _ => {
                let mut count = 0;
                for ev in m.to_events(at_ms) {
                    if matches!(ev, Event::Command { .. }) {
                        count += 1;
                    }
                    self.session.append_event(&ev)?;
                }
                Ok(count)
            }
        }
    }

    fn poll_markers(&mut self) -> Result<usize> {
        let path = self.session.markers_path();
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(_) => return Ok(0),
        };
        let complete_end = last_newline(&data).map(|i| i + 1).unwrap_or(0);
        if complete_end as u64 <= self.marker_pos {
            return Ok(0);
        }
        let new = &data[self.marker_pos as usize..complete_end];
        self.marker_pos = complete_end as u64;

        let mut n = 0;
        for line in new.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            let rec: sdoc_core::MarkerRecord =
                serde_json::from_slice(line).context("bad marker record")?;
            let at_ms = rec.ts_unix_ms.saturating_sub(self.t0_unix_ms);
            n += self.append_marker_events(&rec.marker, at_ms)?;
        }
        Ok(n)
    }

    /// Stop streaming, flush pending markers, and finalize the session.
    pub fn stop(mut self) -> Result<TmuxSummary> {
        let clear = std::process::Command::new(&self.tmux)
            .arg("pipe-pane")
            .arg("-t")
            .arg(&self.target)
            .arg("-o")
            .status();
        let _ = clear; // a failed clear is not fatal (pane may be gone)

        let mut tail = Vec::new();
        self.demuxer.flush(&mut tail);
        if !tail.is_empty() {
            let at_ms = now_unix_ms().saturating_sub(self.t0_unix_ms);
            self.cast.output(at_ms, &String::from_utf8_lossy(&tail))?;
        }

        let mut commands = 0usize;
        for (seq, (start_ms, command)) in self.pending.drain() {
            commands += 1;
            let title = sdoc_core::shorten(&command);
            if !command.is_empty() {
                self.session.append_event(&Event::Chapter {
                    at_ms: start_ms,
                    title,
                    source: sdoc_core::ChapterSource::Auto,
                })?;
            }
            self.session.append_event(&Event::Command {
                at_ms: start_ms,
                seq,
                exit_code: -1,
                command,
            })?;
        }

        let end_ms = now_unix_ms().saturating_sub(self.t0_unix_ms);
        self.session
            .append_event(&Event::SessionEnd { at_ms: end_ms, reason: "stopped".into() })?;
        let meta = self.session.read_meta()?;
        let mut finalized = meta;
        finalized.duration_ms = Some(end_ms);
        self.session.save_meta(&finalized)?;
        self.cast.finish()?;
        Session::clear_active(self.session.id())?;

        Ok(TmuxSummary { session: self.session, commands })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_size_parses() {
        assert_eq!(parse_pane_size("100 30\n").unwrap(), (100, 30));
        assert_eq!(parse_pane_size("  120 40 ").unwrap(), (120, 40));
    }

    #[test]
    fn pane_size_rejects_garbage() {
        assert!(parse_pane_size("").is_err());
        assert!(parse_pane_size("no-spaces").is_err());
        assert!(parse_pane_size("abc def").is_err());
        assert!(parse_pane_size("100").is_err());
        assert!(parse_pane_size("100 99999999").is_err()); // overflows u16
    }
}
