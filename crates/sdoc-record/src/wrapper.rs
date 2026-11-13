use std::io::Read;
#[cfg(windows)]
use std::os::windows::process::ExitStatusExt;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
#[cfg(not(windows))]
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use sdoc_core::{
    Event, MarkerReader, Meta, Mode, Session, Size,
    session::{new_session_id, now_unix_ms},
};

use crate::hooks;

/// Options for pty wrapper recording (Mode 1).
pub struct WrapperOptions {
    /// argv of the command to record.
    pub command: Vec<String>,
    /// Terminal size for the pseudo-terminal.
    pub size: Size,
    /// Working directory for the child process.
    pub cwd: Option<PathBuf>,
    /// Encrypt the session's content files at rest (AES-256-GCM).
    pub encrypt: bool,
}

/// Summary of a completed wrapper recording.
pub struct WrapperSummary {
    /// The recorded session.
    pub session: Session,
    /// Number of command markers captured.
    pub commands: usize,
}

/// Run a command and capture a full cast.
///
/// This is the primary recording mode: the command runs, its byte stream is
/// captured into a clean cast, and command boundaries become chapters. The
/// capture mechanism is platform-specific: a pseudo-terminal on Unix
/// (highest fidelity) and OS pipes on Windows, where a ConPTY child never
/// reports exit on headless runners.
pub fn run_wrapper(opts: &WrapperOptions) -> Result<WrapperSummary> {
    #[cfg(windows)]
    {
        run_wrapper_pipes(opts)
    }
    #[cfg(not(windows))]
    {
        run_wrapper_pty(opts)
    }
}

/// Unix: record a command in a real pseudo-terminal.
///
/// The command runs in bash with the marker rcfile; its byte stream is
/// demuxed into a clean cast and command boundaries become chapters.
#[cfg(not(windows))]
fn run_wrapper_pty(opts: &WrapperOptions) -> Result<WrapperSummary> {
    let shell = "bash";

    let meta = Meta {
        id: new_session_id(),
        started_unix_ms: now_unix_ms(),
        shell: Some(shell.into()),
        cwd: opts.cwd.as_ref().map(|p| p.to_string_lossy().to_string()),
        mode: Mode::Wrapper,
        video: true,
        pane_target: None,
        duration_ms: None,
        title: opts.command.first().cloned(),
        group: None,
    };
    let session =
        if opts.encrypt { Session::create_encrypted(&meta)? } else { Session::create(&meta)? };
    session.set_active()?;

    let pty_system = native_pty_system();
    let pty_size =
        PtySize { rows: opts.size.rows, cols: opts.size.cols, pixel_width: 0, pixel_height: 0 };
    let pair = pty_system.openpty(pty_size).context("failed to open pty")?;
    let portable_pty::PtyPair { master, slave } = pair;

    // Build the child command: bash with the marker rcfile.
    let rcpath = session.dir().join("hook.rc");
    std::fs::write(&rcpath, hooks::bash_wrapper_rcfile())?;
    let joined = shell_join(&opts.command);
    // Interactive mode drives the per-command markers and the user's
    // prompt; the rcfile's EXIT trap still records the final command and
    // status.
    let mut builder = CommandBuilder::new("bash");
    builder.arg("--rcfile");
    builder.arg(&rcpath);
    builder.arg("-i");
    builder.arg("-c");
    // Terminate explicitly after the recorded command so a shell that
    // would otherwise drop into a prompt loop cannot keep the pty open.
    // The EXIT trap captures the status before the plain `exit` runs.
    builder.arg(format!("{joined}; exit"));
    builder.env("TERM", "xterm-256color");
    if let Some(cwd) = &opts.cwd {
        builder.cwd(cwd);
    }
    builder.set_controlling_tty(true);

    let mut child = slave.spawn_command(builder).context("failed to spawn child")?;
    // Drop our slave handle so the child is the sole owner; the pty master
    // only sees end-of-stream once the child (and any forks) have exited.
    drop(slave);

    // Ctrl-C in the user's terminal stops the recording and terminates the
    // recorded command cleanly. The handler is installed once per process
    // (the CLI may dispatch several subcommands in one process).
    let stop = sdoc_core::signal::flag();

    session.append_event(&Event::SessionStart {
        at_ms: 0,
        shell: Some(shell.into()),
        size: opts.size,
    })?;

    let mut cast = session.cast_writer(&opts.size)?;

    let t0 = Instant::now();
    let mut demuxer = MarkerReader::new();
    let mut commands = 0usize;
    let mut interrupted = false;

    let mut reader = master.try_clone_reader().context("failed to clone pty reader")?;
    let mut buf = [0u8; 8192];
    loop {
        if stop.load(std::sync::atomic::Ordering::SeqCst) {
            interrupted = true;
            let _ = child.kill();
            break;
        }
        // A read error here means the pty was torn down (EIO on Linux once
        // every slave is closed), which we treat as end-of-stream.
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let now_ms = t0.elapsed().as_millis() as u64;
                let (clean, markers) = demuxer.feed(&buf[..n]);
                let clean_str = String::from_utf8_lossy(&clean);
                cast.output(now_ms, &clean_str)?;
                for m in markers {
                    for ev in m.to_events(now_ms) {
                        if matches!(ev, Event::Command { .. }) {
                            commands += 1;
                        }
                        session.append_event(&ev)?;
                    }
                }
            }
        }
    }

    let exit_status = if interrupted {
        let _ = child.kill();
        child.wait().unwrap_or_else(|_| portable_pty::ExitStatus::with_signal("SIGTERM"))
    } else {
        child.wait().context("failed to wait on child")?
    };
    let end_ms = t0.elapsed().as_millis() as u64;
    let code = exit_status.exit_code() as i32;
    let reason = if interrupted {
        "interrupted (Ctrl-C)".to_string()
    } else {
        match exit_status.signal() {
            Some(sig) => format!("killed by {sig}"),
            None => format!("exit {code}"),
        }
    };
    session.append_event(&Event::SessionEnd { at_ms: end_ms, reason })?;

    let mut finalized = meta;
    finalized.duration_ms = Some(end_ms);
    session.save_meta(&finalized)?;
    cast.finish()?;

    Ok(WrapperSummary { session, commands })
}

/// Windows: record a command through the native shell's stdout/stderr
/// pipes.
///
/// A ConPTY child never reports exit on headless runners, so instead of a
/// pseudo-terminal we capture the shell's output through OS pipes: the
/// output streams in real time and the child exit is observed via
/// `Child::wait`, which is reliable. The native shell (`cmd` by default,
/// PowerShell when `SCREENDOCP_SHELL=powershell`) runs the recorded
/// command line and exits with that command's code.
#[cfg(windows)]
fn run_wrapper_pipes(opts: &WrapperOptions) -> Result<WrapperSummary> {
    let shell = windows_recording_shell();
    let command_label = windows_command_line(&opts.command);

    let meta = Meta {
        id: new_session_id(),
        started_unix_ms: now_unix_ms(),
        shell: Some(shell.into()),
        cwd: opts.cwd.as_ref().map(|p| p.to_string_lossy().to_string()),
        mode: Mode::Wrapper,
        video: true,
        pane_target: None,
        duration_ms: None,
        title: opts.command.first().cloned(),
        group: None,
    };
    let session =
        if opts.encrypt { Session::create_encrypted(&meta)? } else { Session::create(&meta)? };
    session.set_active()?;

    // Build the native shell command with piped output (no ConPTY).
    let mut cmd = if shell == "powershell" {
        let script = hooks::powershell_wrapper_script(&command_label);
        let mut c = std::process::Command::new("powershell");
        c.arg("-NoProfile");
        c.arg("-Command");
        c.arg(script);
        c
    } else {
        let driver = session.dir().join("hook.cmd");
        std::fs::write(&driver, hooks::cmd_wrapper_driver(&command_label))?;
        let path = driver.to_string_lossy().to_string();
        let mut c = std::process::Command::new("cmd");
        c.arg("/c");
        c.arg(if path.contains(' ') { format!("\"{path}\"") } else { path });
        c
    };
    cmd.env("TERM", "xterm-256color");
    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().context("failed to spawn child")?;

    // Ctrl-C in the user's terminal stops the recording and terminates the
    // recorded command cleanly. The handler is installed once per process.
    let stop = sdoc_core::signal::flag();

    session.append_event(&Event::SessionStart {
        at_ms: 0,
        shell: Some(shell.into()),
        size: opts.size,
    })?;

    let mut cast = session.cast_writer(&opts.size)?;

    let t0 = Instant::now();
    let mut demuxer = MarkerReader::new();
    let mut commands = 0usize;

    // Stream both pipes into the cast in real time. Each pipe is drained by a
    // dedicated (detached) thread; chunks are interleaved on this thread so
    // Ctrl-C is honored mid-recording. The channel stays open until both
    // drains finish (end-of-stream on both pipes).
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    {
        let txo = tx.clone();
        std::thread::spawn(move || drain_pipe(stdout, txo));
        let txe = tx.clone();
        std::thread::spawn(move || drain_pipe(stderr, txe));
    }
    drop(tx);

    let mut interrupted = false;
    for chunk in rx {
        if stop.load(std::sync::atomic::Ordering::SeqCst) {
            interrupted = true;
            let _ = child.kill();
            break;
        }
        let now_ms = t0.elapsed().as_millis() as u64;
        let (clean, markers) = demuxer.feed(&chunk);
        let clean_str = String::from_utf8_lossy(&clean);
        cast.output(now_ms, &clean_str)?;
        for m in markers {
            for ev in m.to_events(now_ms) {
                if matches!(ev, Event::Command { .. }) {
                    commands += 1;
                }
                session.append_event(&ev)?;
            }
        }
    }

    let exit_status = if interrupted {
        let _ = child.kill();
        child.wait().unwrap_or_else(|_| std::process::ExitStatus::from_raw(1))
    } else {
        child.wait().context("failed to wait on child")?
    };
    let end_ms = t0.elapsed().as_millis() as u64;
    let code = exit_status.code().unwrap_or(0) as i32;

    if !interrupted && commands == 0 {
        // The native Windows shells emit no OSC-90 markers, so synthesize the
        // single command event (and its auto-chapter) from the exit code
        // once, mirroring what the bash demuxer path records on Unix.
        let marker =
            sdoc_core::Marker::Cmd { seq: 1, exit_code: code, command: command_label.clone() };
        for ev in marker.to_events(end_ms) {
            if matches!(ev, Event::Command { .. }) {
                commands += 1;
            }
            session.append_event(&ev)?;
        }
    }

    let reason =
        if interrupted { "interrupted (Ctrl-C)".to_string() } else { format!("exit {code}") };
    session.append_event(&Event::SessionEnd { at_ms: end_ms, reason })?;

    let mut finalized = meta;
    finalized.duration_ms = Some(end_ms);
    session.save_meta(&finalized)?;
    cast.finish()?;

    Ok(WrapperSummary { session, commands })
}

/// Drain a pipe to end-of-stream, forwarding each chunk to the sender.
///
/// Runs on a dedicated thread so a blocked read cannot stall the main loop.
/// The drain returns once the write side is closed.
#[cfg(windows)]
fn drain_pipe<R>(mut pipe: R, tx: std::sync::mpsc::Sender<Vec<u8>>)
where
    R: Read + Send + 'static,
{
    let mut buf = [0u8; 8192];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// Join argv into a single shell command string, quoting arguments as needed.
///
/// Bash-specific, used by the Unix recording path and its tests; excluded
/// from non-test Windows builds where the native-shell driver is used.
#[cfg(any(not(windows), test))]
fn shell_join(argv: &[String]) -> String {
    argv.iter().map(|a| quote_if_needed(a)).collect::<Vec<_>>().join(" ")
}

/// Shell-quote a single argument unless it is already safe to pass raw.
///
/// Bash-specific, used by the Unix recording path and its tests; excluded
/// from non-test Windows builds where the native-shell driver is used.
#[cfg(any(not(windows), test))]
fn quote_if_needed(arg: &str) -> String {
    const SAFE: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-./=@:^%+";
    if !arg.is_empty() && arg.bytes().all(|b| SAFE.contains(b as char)) {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// The native recording shell on Windows: `cmd` by default, PowerShell when
/// `SCREENDOCP_SHELL` requests it.
#[cfg(windows)]
fn windows_recording_shell() -> &'static str {
    match std::env::var("SCREENDOCP_SHELL").ok().as_deref() {
        Some("powershell") | Some("pwsh") => "powershell",
        _ => "cmd",
    }
}

/// Join argv into a command line for a native Windows shell (`cmd` or
/// PowerShell). Arguments containing spaces are double-quoted; embedded
/// double-quotes are escaped for PowerShell.
#[cfg(windows)]
fn windows_command_line(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if a.is_empty() || a.contains(' ') {
                format!("\"{}\"", a.replace('"', "\\\""))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_simple_args() {
        let argv = vec!["echo".to_string(), "hi".to_string()];
        assert_eq!(shell_join(&argv), "echo hi");
    }

    #[test]
    fn quotes_arguments_with_spaces() {
        let argv = vec!["echo".to_string(), "a b".to_string()];
        assert_eq!(shell_join(&argv), "echo 'a b'");
    }

    #[test]
    fn quotes_argument_with_single_quote() {
        let argv = vec!["echo".to_string(), "it's".to_string()];
        assert_eq!(shell_join(&argv), "echo 'it'\\''s'");
    }

    /// Injection safety: every argument must survive the
    /// `bash -c <shell_join(args)>` round-trip as a *literal* word, even
    /// when it contains shell metacharacters. The exact-output assertion
    /// proves nothing was executed: a mis-quoted `a;echo INJ1` argument
    /// would print `a` and `INJ1` as two lines instead of the literal.
    ///
    /// Unix only: bash is the recording shell there; Windows records via
    /// cmd/PowerShell pipes, where this property does not apply.
    #[test]
    #[cfg(not(windows))]
    fn shell_join_roundtrips_through_bash() {
        if std::process::Command::new("bash").arg("--version").output().is_err() {
            eprintln!("SKIP: bash not found");
            return;
        }
        let inner = "printf '%s\\n' 'a;echo INJ1' 'b`echo INJ2`' 'c$(echo INJ3)' 'd\n\
                     echo INJ4' 'e\"f\"' 'g\\\\h' 'i; j' 'é文'";
        let argv: Vec<String> = vec!["sh".into(), "-c".into(), inner.into()];

        let joined = shell_join(&argv);
        let out = std::process::Command::new("bash")
            .arg("-c")
            .arg(&joined)
            .output()
            .expect("run bash -c");
        assert!(out.status.success(), "bash failed: {}", String::from_utf8_lossy(&out.stderr));
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let expected =
            "a;echo INJ1\nb`echo INJ2`\nc$(echo INJ3)\nd\necho INJ4\ne\"f\"\ng\\\\h\ni; j\né文\n";
        assert_eq!(stdout, expected);
    }

    #[test]
    fn hostile_arguments_are_quoted() {
        let argv = vec![
            "cmd".to_string(),
            "a;rm".to_string(),
            "b$c".to_string(),
            "d`e`".to_string(),
            "f&g".to_string(),
            "h>i".to_string(),
            "j<k".to_string(),
            "m\"n\"".to_string(),
            "o!p".to_string(),
            "q#r".to_string(),
            "".to_string(),
            "é文".to_string(),
        ];
        let joined = shell_join(&argv);
        // Nothing but the first word stays unquoted.
        assert!(joined.contains("'a;rm'"));
        assert!(joined.contains("'b$c'"));
        assert!(joined.contains("'d`e`'"));
        assert!(joined.contains("'f&g'"));
        assert!(joined.contains("'h>i'"));
        assert!(joined.contains("'j<k'"));
        assert!(joined.contains("'m\"n\"'"));
        assert!(joined.contains("'o!p'"));
        assert!(joined.contains("'q#r'"));
        assert!(joined.contains("''"), "empty arg must become empty quotes");
        assert!(joined.contains("'é文'"));
    }
}
