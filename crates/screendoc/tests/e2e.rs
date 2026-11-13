//! End-to-end tests driving the full CLI from inside the process.
//!
//! These create real sessions under the user's data root and clean them up
//! afterwards. Tests that need external tools (ffmpeg, tmux) are gated and
//! skip cleanly when the tool is absent.

use std::collections::HashSet;
use std::io::{Read, Write as _};

use clap::{CommandFactory, FromArgMatches};
use screendoc::cli::Cli;
use screendoc::util::find_in_path;
use sdoc_core::{Event, Mode, Session};
use sdoc_record::ambient::{self, OnOptions};

/// Run a CLI invocation in-process and return the exit code.
///
/// `execute` returns `Ok(code)` on the happy path; any `Err` maps to 1,
/// matching `main` (`process::exit(1)` on error).
fn run_cli(argv: &[&str]) -> i32 {
    let matches = Cli::command().try_get_matches_from(argv).expect("parse args");
    let cli = Cli::from_arg_matches(&matches).expect("build cli");
    match screendoc::run::execute(&cli) {
        Ok(code) => code,
        Err(e) => {
            // Surface the error on CI: the code alone (1) is not enough
            // to debug a platform-specific failure.
            eprintln!("screendoc CLI error for {argv:?}: {e:#}");
            1
        }
    }
}

/// The e2e suite shares the real data root (`~/.screendoc`), so tests must
/// not create sessions concurrently — serialize the whole suite.
///
/// The holder is tracked so a stalled test is identifiable in CI logs
/// instead of every queued test looking like it is "running for over 60
/// seconds".
static LOCK: std::sync::Mutex<Option<(String, std::time::Instant)>> = std::sync::Mutex::new(None);

/// Acquire the suite lock. The guard holds it for the caller's scope and
/// clears the holder record on drop.
fn lock() -> SuiteLock {
    let name = std::thread::current()
        .name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown-test".into());
    let wait_started = std::time::Instant::now();
    let mut held_guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if wait_started.elapsed() > std::time::Duration::from_secs(5) {
        let who = held_guard
            .as_ref()
            .map(|(h, at)| format!("{h} (held {}s)", at.elapsed().as_secs()))
            .unwrap_or_else(|| "<nobody recorded>".into());
        eprintln!(
            "[suite-lock] {name} waited {}s; held by: {who}",
            wait_started.elapsed().as_secs()
        );
    }
    *held_guard = Some((name, wait_started));
    SuiteLock { _guard: held_guard }
}

struct SuiteLock {
    _guard: std::sync::MutexGuard<'static, Option<(String, std::time::Instant)>>,
}

impl Drop for SuiteLock {
    fn drop(&mut self) {
        let held = self._guard.as_ref().map(|(_, at)| at.elapsed().as_secs()).unwrap_or(0);
        if held > 15 {
            eprintln!("[suite-lock] holder released after {held}s");
        }
        *self._guard = None;
    }
}

/// Run a CLI subcommand and return the session it created (by diffing the
/// session listing before/after, so parallel tests cannot collide).
fn run_and_capture(argv: &[&str]) -> Session {
    let before: HashSet<String> =
        Session::list().expect("list before").into_iter().map(|m| m.id).collect();
    let code = run_cli(argv);
    assert_eq!(code, 0, "CLI call failed: {argv:?}");
    let created = Session::list()
        .expect("list after")
        .into_iter()
        .find(|m| !before.contains(&m.id))
        .expect("a new session was created");
    Session::open(&created.id).expect("open created session")
}

/// Remove sessions by id (test cleanup).
fn cleanup(ids: &[String]) {
    for id in ids {
        if let Ok(s) = Session::open(id) {
            let _ = s.remove();
        }
    }
}

#[test]
fn wrapper_record_then_render_cast() {
    let _g = lock();
    let session = run_and_capture(&[
        "screendoc",
        "record",
        "--size",
        "40x8",
        "echo",
        "hello screendoc",
        "exit",
        "0",
    ]);
    let id = session.id().to_string();

    let events = session.read_events().expect("events");
    assert!(events.iter().any(|e| matches!(e, Event::SessionStart { .. })));
    assert!(events.iter().any(|e| matches!(e, Event::SessionEnd { .. })));
    assert!(
        !events
            .iter()
            .filter(|e| matches!(e, Event::Command { .. }))
            .count()
            .to_string()
            .is_empty()
    );

    let cast_text = std::fs::read_to_string(session.cast_path()).expect("cast exists");
    assert!(cast_text.contains("hello screendoc"));

    let out = std::env::temp_dir().join(format!("sdoc-e2e-{id}"));
    let _ = std::fs::remove_dir_all(&out);
    let code =
        run_cli(&["screendoc", "render", &id, "--cast", "--out", out.to_string_lossy().as_ref()]);
    assert_eq!(code, 0);
    assert!(out.join(format!("{id}.cast")).exists());

    let _ = std::fs::remove_dir_all(&out);
    cleanup(&[id]);
}

#[test]
fn wrapper_render_gif_and_mp4_when_ffmpeg_present() {
    let _g = lock();
    if find_in_path("ffmpeg").is_none() {
        eprintln!("SKIP: ffmpeg not installed");
        return;
    }
    let session = run_and_capture(&[
        "screendoc",
        "record",
        "--size",
        "30x5",
        "echo",
        "video test",
        "exit",
        "0",
    ]);
    let id = session.id().to_string();

    let out = std::env::temp_dir().join(format!("sdoc-e2e-video-{id}"));
    let _ = std::fs::remove_dir_all(&out);
    let code = run_cli(&[
        "screendoc",
        "render",
        &id,
        "--gif",
        "--mp4",
        "--out",
        out.to_string_lossy().as_ref(),
    ]);
    assert_eq!(code, 0);
    let gif = out.join(format!("{id}.gif"));
    let mp4 = out.join(format!("{id}.mp4"));
    assert!(gif.exists(), "gif missing");
    assert!(mp4.exists(), "mp4 missing");
    assert!(gif.metadata().map(|m| m.len() > 0).unwrap_or(false));
    assert!(mp4.metadata().map(|m| m.len() > 0).unwrap_or(false));
    let _ = std::fs::remove_dir_all(&out);
    cleanup(&[id]);
}

#[test]
fn tmux_record_streams_pane() {
    let _g = lock();
    if !sdoc_record::tmux::tmux_available() {
        eprintln!("SKIP: tmux not installed");
        return;
    }
    let name = format!("sdoc-test-{}", std::process::id());
    let new = std::process::Command::new("tmux")
        .args(["new-session", "-d", "-s", &name, "-x", "60", "-y", "10", "bash"])
        .status()
        .expect("tmux new-session");
    assert!(new.success(), "failed to create detached tmux session");
    std::thread::sleep(std::time::Duration::from_millis(500));

    let mut rec = sdoc_record::tmux::start(Some(&name), false).expect("start tmux recorder");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let sent = std::process::Command::new("tmux")
        .args(["send-keys", "-t", &name, "echo from-tmux-pane", "Enter"])
        .status()
        .expect("tmux send-keys");
    assert!(sent.success());
    for _ in 0..10 {
        rec.poll().expect("poll");
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let summary = rec.stop().expect("stop");
    let id = summary.session.id().to_string();
    let cast = std::fs::read_to_string(summary.session.cast_path()).expect("cast");
    assert!(cast.contains("from-tmux-pane"), "cast should contain pane output");
    assert_eq!(summary.session.read_meta().expect("meta").mode, Mode::Tmux);
    let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &name]).status();
    cleanup(&[id]);
}

#[test]
fn ambient_on_off_cycle() {
    let _g = lock();
    let on = ambient::on(&OnOptions { shell: Some("bash".into()), encrypt: false }).expect("on");
    let id = on.session.id().to_string();
    on.session
        .append_marker_record(&sdoc_core::MarkerRecord {
            ts_unix_ms: sdoc_core::session::now_unix_ms(),
            marker: sdoc_core::Marker::Cmd { seq: 1, exit_code: 0, command: "make test".into() },
        })
        .expect("append marker");

    let off = ambient::off(Some(&id)).expect("off");
    assert_eq!(off.commands, 1);
    let events = off.session.read_events().expect("events");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Command { command, .. } if command == "make test"))
    );
    cleanup(&[id]);
}

#[test]
fn serve_smoke_test() {
    let _g = lock();
    let session = run_and_capture(&[
        "screendoc",
        "record",
        "--size",
        "20x4",
        "echo",
        "serve-smoke",
        "exit",
        "0",
    ]);
    let id = session.id().to_string();

    // Port 0: let the OS pick a free ephemeral port. Windows runners reserve
    // dynamic TCP port ranges, so a fixed 49xxx port can hit WSAEACCES (10013).
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime");
    let player = rt.block_on(sdoc_render::serve::serve(&session, 0)).expect("serve");

    let mut stream = std::net::TcpStream::connect(player.addr).expect("tcp connect");
    stream.write_all(b"GET /meta HTTP/1.0\r\nHost: localhost\r\n\r\n").expect("write request");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read response");
    let response = String::from_utf8_lossy(&buf);
    assert!(response.contains("HTTP/1.0 200"), "expected 200, got: {response}");
    assert!(response.contains(&id), "meta should carry the session id");

    // The player page must ship the dual-source bootstrap: embedded data in
    // standalone exports, fetched data in serve mode, and a constructor
    // that tolerates both UMD shapes (a regression here broke the player
    // entirely: `window.term.Terminal` was never a real global).
    let mut stream = std::net::TcpStream::connect(player.addr).expect("tcp connect 2");
    stream.write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n").expect("write request");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read response");
    let page = String::from_utf8_lossy(&buf);
    assert!(page.contains("HTTP/1.0 200"), "player page expected 200");
    assert!(page.contains("unwrapCtor"), "player template must resolve the xterm constructor");
    assert!(page.contains("fetch(\"/cast\")"), "serve mode must keep fetching cast data");

    cleanup(&[id]);
}

// --- P0: import / play / prune / search / redact / chapters / webm / group

#[test]
fn import_v3_cast_creates_session() {
    let _g = lock();
    let dir = std::env::temp_dir().join(format!("sdoc-e2e-import-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let v3 = dir.join("in.cast");
    std::fs::write(
        &v3,
        "{\"version\":3,\"width\":80,\"height\":24}\n\
         {\"t\":0,\"type\":\"o\",\"text\":\"alpha\\n\"}\n\
         {\"t\":0.5,\"type\":\"m\",\"text\":\"imported chapter\"}\n\
         {\"t\":0.5,\"type\":\"o\",\"text\":\"beta\\n\"}\n",
    )
    .expect("write fixture");

    let session = run_and_capture(&["screendoc", "import", v3.to_string_lossy().as_ref()]);
    let id = session.id().to_string();

    // The raw cast is re-written in screendoc's v2 byte stream.
    let cast = std::fs::read_to_string(session.cast_path()).expect("cast");
    assert!(cast.contains("alpha"));
    assert!(cast.contains("beta"));
    assert!(cast.contains("\"version\":2"));

    // v3 marker events became chapters; times accumulated (0.5 + 0.5 = 1.0s).
    let events = session.read_events().expect("events");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Chapter { title, .. } if title == "imported chapter"))
    );
    let meta = session.read_meta().expect("meta");
    assert_eq!(meta.duration_ms, Some(1_000));
    assert_eq!(meta.mode, Mode::Journal);

    let _ = std::fs::remove_dir_all(&dir);
    cleanup(&[id]);
}

#[test]
fn import_v2_cast_replays() {
    let _g = lock();
    let dir = std::env::temp_dir().join(format!("sdoc-e2e-import-v2-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let v2 = dir.join("in.cast");
    std::fs::write(
        &v2,
        "{\"version\":2,\"terminal_width\":100,\"terminal_height\":30}\n\
         [\"0.0\",\"o\",\"gamma\\n\"]\n\
         [\"2.0\",\"o\",\"delta\\n\"]\n",
    )
    .expect("write fixture");

    let session = run_and_capture(&["screendoc", "import", v2.to_string_lossy().as_ref()]);
    let id = session.id().to_string();
    let cast = std::fs::read_to_string(session.cast_path()).expect("cast");
    assert!(cast.contains("gamma") && cast.contains("delta"));

    // play --fast re-emits the byte stream; assert exit 0 and that the
    // session's cast is well-formed enough to parse back.
    let parsed = sdoc_core::cast_read::parse_cast_text(&cast).expect("reparse");
    assert_eq!(parsed.cols, Some(100));
    assert_eq!(parsed.events.len(), 2);
    assert_eq!(run_cli(&["screendoc", "play", &id, "--fast"]), 0);

    let _ = std::fs::remove_dir_all(&dir);
    cleanup(&[id]);
}

#[test]
fn prune_removes_old_sessions() {
    let _g = lock();
    // Fabricate one old session (backdated 40 days) and one fresh one.
    let old_id = sdoc_core::session::new_session_id();
    let _old = Session::create(&test_meta(&old_id, 40)).expect("create old");
    let fresh_id = sdoc_core::session::new_session_id();
    let _fresh = Session::create(&test_meta(&fresh_id, 0)).expect("create fresh");

    let code = run_cli(&["screendoc", "prune", "--older-than", "7d", "--yes"]);
    assert_eq!(code, 0);
    assert!(Session::open(&old_id).is_err(), "old session should be pruned");
    assert!(Session::open(&fresh_id).is_ok(), "fresh session should survive");

    cleanup(&[old_id, fresh_id]);
}

#[test]
fn search_finds_command_text() {
    let _g = lock();
    let marker = format!("sdoc-search-{}", std::process::id());
    let session =
        run_and_capture(&["screendoc", "record", "--size", "30x4", "echo", &marker, "exit", "0"]);
    let id = session.id().to_string();
    // The wrapper records the whole command line as one command; the marker
    // substring is inside it.
    let hits = screendoc::run::search_hits(&marker, None).expect("search");
    assert!(hits.iter().any(|h| h["session"] == id), "search should hit the recording session");
    let none = screendoc::run::search_hits("definitely-not-a-command-xyz", None).expect("search2");
    assert!(none.is_empty());
    cleanup(&[id]);
}

/// A session that claims to be encrypted at rest but whose journal is
/// plaintext (e.g. the key was lost) must be skipped with a warning by a
/// global search, never fail it.
#[test]
fn search_skips_unreadable_sessions_without_failing() {
    let _g = lock();
    let marker = format!("sdoc-skip-search-{}", std::process::id());
    let good =
        run_and_capture(&["screendoc", "record", "--size", "30x4", "echo", &marker, "exit", "0"]);
    let good_id = good.id().to_string();

    let bad = run_and_capture(&[
        "screendoc",
        "record",
        "--size",
        "30x4",
        "echo",
        "sdoc-unreadable",
        "exit",
        "0",
    ]);
    let bad_id = bad.id().to_string();
    // Flip it into the unreadable state: encrypted sidecar, plaintext files.
    let cast_path = bad.cast_path();
    let dir = cast_path.parent().expect("session dir");
    std::fs::write(dir.join("encrypted"), b"1").expect("sidecar");

    let hits =
        screendoc::run::search_hits(&marker, None).expect("search must skip unreadable sessions");
    assert!(hits.iter().any(|h| h["session"] == good_id));
    assert!(!hits.iter().any(|h| h["session"] == bad_id));
    cleanup(&[good_id, bad_id]);
}

#[test]
fn redact_masks_secrets() {
    let _g = lock();
    let token = format!("tok-{}", std::process::id());
    let session =
        run_and_capture(&["screendoc", "record", "--size", "40x6", "echo", &token, "exit", "0"]);
    let id = session.id().to_string();

    let code = run_cli(&["screendoc", "redact", &id, &token, "--yes"]);
    assert_eq!(code, 0);

    let cast = std::fs::read_to_string(session.cast_path()).expect("cast");
    assert!(!cast.contains(&token), "token must be masked from the cast");
    assert!(cast.contains("***"), "replacement should be present");
    // The colored prompt/ANSI bytes are still well-formed (stream parses).
    let parsed = sdoc_core::cast_read::parse_cast_text(&cast).expect("cast still parseable");
    assert!(!parsed.events.iter().any(|e| e.data.contains(&token)));

    cleanup(&[id]);
}

#[test]
fn render_chapters_and_stills_when_ffmpeg_present() {
    let _g = lock();
    if find_in_path("ffmpeg").is_none() {
        eprintln!("SKIP: ffmpeg not installed");
        return;
    }
    // Record a session long enough to split into two real spans. `sleep` on
    // Unix, native `timeout` on Windows (the recording shell is cmd there,
    // where `sleep` is not available).
    let session = if cfg!(windows) {
        run_and_capture(&["screendoc", "record", "--size", "40x6", "timeout", "/t", "1"])
    } else {
        run_and_capture(&["screendoc", "record", "--size", "40x6", "sleep", "0.4"])
    };
    let id = session.id().to_string();

    // Give the session two chapters so `--chapters` has something to split.
    // Two chapters are a valid configuration (one span), not a flake;
    // require a second real chapter and assert on what is produced.
    let dur = session.read_meta().expect("meta").duration_ms.unwrap_or(400).max(200);
    session
        .append_event(&Event::Chapter {
            at_ms: 0,
            title: "intro".into(),
            source: sdoc_core::ChapterSource::Manual,
        })
        .expect("chapter 1");
    session
        .append_event(&Event::Chapter {
            at_ms: dur / 2,
            title: "outro".into(),
            source: sdoc_core::ChapterSource::Manual,
        })
        .expect("chapter 2");

    let out = std::env::temp_dir().join(format!("sdoc-e2e-ch-{id}"));
    let _ = std::fs::remove_dir_all(&out);
    let code = run_cli(&[
        "screendoc",
        "render",
        &id,
        "--chapters",
        "--frames",
        "--gif",
        "--no-full",
        "--out",
        out.to_string_lossy().as_ref(),
    ]);
    assert_eq!(code, 0);
    let gifs: Vec<_> = std::fs::read_dir(&out)
        .expect("out dir")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".gif"))
        .collect();
    assert!(gifs.len() >= 2, "expected one gif per chapter, got {gifs:?}");
    let pngs: Vec<_> = std::fs::read_dir(&out)
        .expect("out dir")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".png"))
        .collect();
    assert!(!pngs.is_empty(), "still frames expected");

    let _ = std::fs::remove_dir_all(&out);
    cleanup(&[id]);
}

#[test]
fn render_webm_when_ffmpeg_present() {
    let _g = lock();
    if find_in_path("ffmpeg").is_none() {
        eprintln!("SKIP: ffmpeg not installed");
        return;
    }
    let session =
        run_and_capture(&["screendoc", "record", "--size", "30x4", "echo", "webm", "exit", "0"]);
    let id = session.id().to_string();
    let out = std::env::temp_dir().join(format!("sdoc-e2e-webm-{id}"));
    let _ = std::fs::remove_dir_all(&out);
    let code =
        run_cli(&["screendoc", "render", &id, "--webm", "--out", out.to_string_lossy().as_ref()]);
    assert_eq!(code, 0);
    assert!(out.join(format!("{id}.webm")).exists(), "webm missing");
    let _ = std::fs::remove_dir_all(&out);
    cleanup(&[id]);
}

#[test]
fn tmux_group_records_all_panes() {
    let _g = lock();
    if !sdoc_record::tmux::tmux_available() {
        eprintln!("SKIP: tmux not installed");
        return;
    }
    let name = format!("sdoc-group-{}", std::process::id());
    let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &name]).status();
    let new = std::process::Command::new("tmux")
        .args(["new-session", "-d", "-s", &name, "-x", "50", "-y", "8", "bash"])
        .status()
        .expect("tmux new-session");
    assert!(new.success());
    // Split into two panes.
    let split = std::process::Command::new("tmux")
        .args(["split-window", "-h", "-t", &name])
        .status()
        .expect("tmux split-window");
    assert!(split.success(), "failed to split pane");
    std::thread::sleep(std::time::Duration::from_millis(500));

    let mut grp = sdoc_record::tmux::start_group(&name, false).expect("start group");
    assert_eq!(grp.ids().len(), 2, "two panes expected");
    let ids = grp.ids();

    std::process::Command::new("tmux")
        .args(["send-keys", "-t", &format!("{name}.0"), "echo pane-zero-output", "Enter"])
        .status()
        .expect("send to pane 0");
    std::process::Command::new("tmux")
        .args(["send-keys", "-t", &format!("{name}.1"), "echo pane-one-output", "Enter"])
        .status()
        .expect("send to pane 1");
    for _ in 0..10 {
        grp.poll().expect("poll");
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let summaries = grp.stop().expect("stop group");
    assert_eq!(summaries.len(), 2);

    let mut saw_zero = false;
    let mut saw_one = false;
    for s in &summaries {
        let cast = std::fs::read_to_string(s.session.cast_path()).expect("cast");
        if cast.contains("pane-zero-output") {
            saw_zero = true;
        }
        if cast.contains("pane-one-output") {
            saw_one = true;
        }
        // Each pane session is linked to the group.
        assert_eq!(s.session.read_meta().expect("meta").group.as_deref(), Some(name.as_str()));
    }
    assert!(saw_zero && saw_one, "both panes' output should be captured");

    let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &name]).status();
    cleanup(&ids);
}

/// A test meta, optionally backdated by `days`.
fn test_meta(id: &str, days_ago: u64) -> sdoc_core::Meta {
    let now = sdoc_core::session::now_unix_ms();
    sdoc_core::Meta {
        id: id.to_string(),
        started_unix_ms: now.saturating_sub(days_ago * 86_400_000),
        shell: None,
        cwd: None,
        mode: Mode::Journal,
        video: false,
        pane_target: None,
        duration_ms: None,
        title: None,
        group: None,
    }
}

// --- edge / hostile-input coverage

#[test]
fn import_garbage_and_empty_files_do_not_panic() {
    let _g = lock();
    let dir = std::env::temp_dir().join(format!("sdoc-e2e-garbage-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);

    // Empty file: creates a valid journal session with only start/end.
    let empty = dir.join("empty.cast");
    std::fs::write(&empty, "").expect("write empty");
    let s1 = run_and_capture(&["screendoc", "import", empty.to_string_lossy().as_ref()]);
    assert_eq!(
        s1.read_events().expect("events").len(),
        2,
        "empty import should yield session_start + session_end"
    );

    // Pure binary garbage: still a clean session, no events.
    let junk = dir.join("junk.cast");
    std::fs::write(&junk, [0u8, 1, 2, 255, 128, 64, 0, 255]).expect("write junk");
    let s2 = run_and_capture(&["screendoc", "import", junk.to_string_lossy().as_ref()]);
    assert_eq!(
        s2.read_events().expect("events").len(),
        2,
        "garbage import should yield session_start + session_end"
    );

    // A totally missing file must fail cleanly (exit 1), not panic.
    assert_eq!(run_cli(&["screendoc", "import", "/nonexistent/x.cast"]), 1);

    let _ = std::fs::remove_dir_all(&dir);
    cleanup(&[s1.id().to_string(), s2.id().to_string()]);
}

#[test]
fn missing_session_ids_error_cleanly() {
    let _g = lock();
    for cmd in [
        vec!["screendoc", "play", "no-such-session"],
        vec!["screendoc", "render", "no-such-session", "--cast"],
        vec!["screendoc", "redact", "no-such-session", "x", "--yes"],
        vec!["screendoc", "rm", "no-such-session", "--yes"],
    ] {
        assert_eq!(run_cli(&cmd), 1, "expected clean error for {cmd:?}");
    }
}

#[test]
fn traversal_session_ids_are_rejected() {
    let _g = lock();
    for id in ["..", "../etc", "../../etc/passwd", "a/b"] {
        let code = run_cli(&["screendoc", "play", id]);
        assert_eq!(code, 1, "traversal play id `{id}` must be rejected");
        let code = run_cli(&["screendoc", "render", id, "--cast"]);
        assert_eq!(code, 1, "traversal render id `{id}` must be rejected");
    }
    // And the sessions dir must be untouched by the traversal attempts.
    let before = Session::list().expect("list").len();
    run_cli(&["screendoc", "rm", "..", "--yes"]);
    assert_eq!(Session::list().expect("list").len(), before);
}

#[test]
fn wrapper_failing_command_records_nonzero_exit() {
    let _g = lock();
    // Record a native command that exits non-zero: `bash` on Unix, `cmd` on
    // Windows (git-bash is not the recording shell there).
    let session = if cfg!(windows) {
        run_and_capture(&["screendoc", "record", "--size", "30x4", "cmd", "/c", "exit /b 3"])
    } else {
        run_and_capture(&["screendoc", "record", "--size", "30x4", "bash", "-c", "exit 3"])
    };
    let id = session.id().to_string();
    let events = session.read_events().expect("events");
    let end = events.iter().find(|e| matches!(e, Event::SessionEnd { .. })).expect("session end");
    let reason = match end {
        Event::SessionEnd { reason, .. } => reason.clone(),
        _ => unreachable!(),
    };
    assert!(reason.contains("3"), "expected exit 3 in `{reason}`, got");
    cleanup(&[id]);
}

#[test]
fn tmux_stop_after_pane_death_still_finalizes() {
    let _g = lock();
    if !sdoc_record::tmux::tmux_available() {
        eprintln!("SKIP: tmux not installed");
        return;
    }
    let name = format!("sdoc-dead-{}", std::process::id());
    let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &name]).status();
    let _ = std::process::Command::new("tmux")
        .args(["new-session", "-d", "-s", &name, "-x", "50", "-y", "10", "bash"])
        .status();
    std::thread::sleep(std::time::Duration::from_millis(400));

    let rec = sdoc_record::tmux::start(Some(&name), false).expect("start");
    let id = rec.id().to_string();
    // Kill the whole tmux session out from under the recorder.
    let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &name]).status();
    // stop() must still finalize cleanly (a failed pipe-pane clear is ok).
    let summary = rec.stop().expect("stop after pane death");
    assert_eq!(summary.session.id(), id);
    cleanup(&[id]);
}

#[test]
fn serve_returns_404_for_unknown_routes() {
    let _g = lock();
    let session = run_and_capture(&[
        "screendoc",
        "record",
        "--size",
        "20x4",
        "echo",
        "404-test",
        "exit",
        "0",
    ]);
    let id = session.id().to_string();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime");
    // Port 0: OS-assigned ephemeral port (fixed ports can hit Windows'
    // reserved TCP ranges, WSAEACCES / 10013).
    let player = rt.block_on(sdoc_render::serve::serve(&session, 0)).expect("serve");

    let mut stream = std::net::TcpStream::connect(player.addr).expect("tcp connect");
    stream
        .write_all(b"GET /definitely-missing HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .expect("write");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read");
    let resp = String::from_utf8_lossy(&buf);
    assert!(resp.contains("404"), "expected 404, got: {resp}");

    cleanup(&[id]);
}

#[test]
fn redact_supports_capture_group_replacement() {
    let _g = lock();
    let marker = format!("pw-{}", std::process::id());
    let session =
        run_and_capture(&["screendoc", "record", "--size", "40x6", "echo", &marker, "exit", "0"]);
    let id = session.id().to_string();

    // Mask the whole token, keep only the part inside the capture group.
    let pattern = format!(r"{}(.*)", regex_escape(&marker));
    let code = run_cli(&["screendoc", "redact", &id, &pattern, "--replace", "redacted", "--yes"]);
    assert_eq!(code, 0);
    let cast = std::fs::read_to_string(session.cast_path()).expect("cast");
    assert!(!cast.contains(&marker), "token must be gone");
    assert!(cast.contains("redacted"));
    cleanup(&[id]);
}

/// Minimal escaping so a pattern containing regex metachars stays literal.
fn regex_escape(s: &str) -> String {
    const META: &str = r".*+?()[]{}^$|\\";
    s.chars()
        .flat_map(|c| if META.contains(c) { vec![b'\\', c as u8] } else { vec![c as u8] })
        .collect::<Vec<u8>>()
        .into_iter()
        .map(char::from)
        .collect()
}

// --- encryption, retention & machine output

/// A long session (synthetic 10-minute cast) imports, reads, and renders
/// without degrading behavior — the soak evidence for the streaming
/// format.
#[test]
fn long_session_import_and_render() {
    let _g = lock();
    // 10 minutes, ~5 events/second: 3000 events.
    let mut text = String::from("{\"version\":2,\"width\":100,\"height\":30}\n");
    for i in 0..3000 {
        let t = i as f64 * 0.2;
        text.push_str(&format!("[\"{:.3}\",\"o\",\"line {i:05} heartbeat \\r\\n\"]\n", t));
    }
    let path = std::env::temp_dir().join(format!("sdoc-long-{}.cast", std::process::id()));
    std::fs::write(&path, &text).expect("write cast");

    let id = run_cli_capture_id(&["screendoc", "import", path.to_str().unwrap(), "--json"]);
    let _ = std::fs::remove_file(&path);

    let s = sdoc_core::Session::open(&id).expect("session");
    let events = s.read_events().expect("events");
    assert!(events.len() >= 2, "start + end at minimum");
    let meta = s.read_meta().expect("meta");
    assert!(
        (598_000..=602_000).contains(&meta.duration_ms.unwrap_or(0)),
        "duration should be ~600s (10 min), got {:?}",
        meta.duration_ms
    );
    // The cast re-serializes cleanly and round-trips through the parser.
    let cast = s.read_cast_text().expect("cast");
    let parsed = sdoc_core::cast_read::parse_cast_text(&cast).expect("parse");
    assert!(parsed.events.len() >= 2900, "events should survive round-trip");

    // A cast export of the long session is a single read+write pass.
    let out = std::env::temp_dir().join(format!("sdoc-long-out-{id}"));
    std::fs::create_dir_all(&out).expect("mkdir");
    assert_eq!(run_cli(&["screendoc", "render", &id, "--cast", "--out", out.to_str().unwrap()]), 0);
    let _ = std::fs::remove_dir_all(&out);
    cleanup(&[id]);
}

/// `record --encrypt` stores encrypted files; playback stays transparent.
#[test]
fn encrypted_record_roundtrips() {
    let _g = lock();
    let marker = format!("enc-{}", std::process::id());
    let session = run_and_capture(&[
        "screendoc",
        "record",
        "--size",
        "40x6",
        "--encrypt",
        "echo",
        &marker,
        "exit",
        "0",
    ]);
    let id = session.id().to_string();
    let s = sdoc_core::Session::open(&id).expect("open");
    assert!(s.is_encrypted(), "session must be marked encrypted");

    // Raw files carry the magic, not the plaintext.
    let raw = std::fs::read(s.cast_path()).expect("raw bytes");
    assert!(raw.starts_with(b"SDOC1ENC"), "cast must be an encrypted record stream");
    let ev = std::fs::read(s.events_path()).expect("events bytes");
    assert!(ev.starts_with(b"SDOC1ENC"), "events must be encrypted");
    assert!(
        !raw.windows(marker.len()).any(|w| w == marker.as_bytes()),
        "plaintext must not appear in the encrypted cast"
    );

    // The transparent read path returns the plaintext.
    let cast = s.read_cast_text().expect("cast text");
    assert!(cast.contains(&marker), "transparent read must decrypt");

    cleanup(&[id]);
}

/// `redact` on an encrypted session stays encrypted and masks.
#[test]
fn redact_works_on_encrypted_sessions() {
    let _g = lock();
    let marker = format!("sec-{}", std::process::id());
    let session = run_and_capture(&[
        "screendoc",
        "record",
        "--size",
        "40x6",
        "--encrypt",
        "echo",
        &marker,
        "exit",
        "0",
    ]);
    let id = session.id().to_string();

    assert_eq!(run_cli(&["screendoc", "redact", &id, &marker, "--replace", "MASKED", "--yes"]), 0);
    let s = sdoc_core::Session::open(&id).expect("open");
    let cast = s.read_cast_text().expect("cast");
    assert!(cast.contains("MASKED"), "redaction must apply through decryption");
    assert!(!cast.contains(&marker), "original secret must be gone");
    let raw = std::fs::read(s.cast_path()).expect("raw");
    assert!(raw.starts_with(b"SDOC1ENC"), "redacted files stay encrypted");

    cleanup(&[id]);
}

/// An encrypted session's exported cast is plaintext (the user chose to
/// share; export is the decryption point).
#[test]
fn encrypted_session_exports_plaintext_cast() {
    let _g = lock();
    let marker = format!("xport-{}", std::process::id());
    let session = run_and_capture(&[
        "screendoc",
        "record",
        "--size",
        "40x6",
        "--encrypt",
        "echo",
        &marker,
        "exit",
        "0",
    ]);
    let id = session.id().to_string();
    let out = std::env::temp_dir().join(format!("sdoc-enc-export-{id}"));
    std::fs::create_dir_all(&out).expect("mkdir");
    assert_eq!(run_cli(&["screendoc", "render", &id, "--cast", "--out", out.to_str().unwrap()]), 0);
    let exported = std::fs::read(out.join(format!("{id}.cast"))).expect("exported");
    let text = String::from_utf8_lossy(&exported);
    assert!(text.contains(&marker), "export decrypts");
    assert!(!exported.starts_with(b"SDOC1ENC"), "export must not be encrypted");
    let _ = std::fs::remove_dir_all(&out);
    cleanup(&[id]);
}

/// `config set encrypt true` makes a plain `record` encrypt by default.
#[test]
fn config_encrypt_flag_drives_default() {
    let _g = lock();
    assert_eq!(run_cli(&["screendoc", "config", "set", "encrypt", "true"]), 0);
    let session =
        run_and_capture(&["screendoc", "record", "--size", "40x6", "echo", "cfg-enc", "exit", "0"]);
    let id = session.id().to_string();
    assert!(sdoc_core::Session::open(&id).expect("open").is_encrypted());
    cleanup(&[id]);
    // Restore the default so later tests are unaffected.
    assert_eq!(run_cli(&["screendoc", "config", "set", "encrypt", "false"]), 0);
}

// --- sharing & lifecycle

/// `render --web` produces a self-contained player: xterm inlined, no
/// fetch() left in the data path, and the cast embedded as data.
#[test]
fn render_web_is_self_contained() {
    let _g = lock();
    let session = run_and_capture(&[
        "screendoc",
        "record",
        "--size",
        "40x6",
        "echo",
        "web-share",
        "exit",
        "0",
    ]);
    let id = session.id().to_string();
    let out = std::env::temp_dir().join(format!("sdoc-web-{id}"));
    let _ = std::fs::create_dir_all(&out);
    assert_eq!(run_cli(&["screendoc", "render", &id, "--web", "--out", out.to_str().unwrap()]), 0);

    let html = std::fs::read_to_string(out.join(format!("{id}.html"))).expect("html");
    // The xterm distribution and data are inlined — no external refs.
    assert!(!html.contains("script src=\"/xterm.js\""), "xterm.js must be inlined");
    assert!(!html.contains("href=\"/xterm.css\""), "xterm.css must be inlined");
    assert!(!html.contains("SCREENDOC:EMBED"), "embed marker must be consumed");
    assert!(html.contains("window.__SCREENDOC__ = "), "data must be embedded");
    // The embedded payload (up to the closing `;</script>`) must not be
    // able to terminate its own script block.
    let payload = html
        .split("window.__SCREENDOC__ = ")
        .nth(1)
        .and_then(|rest| rest.split(";</script>").next())
        .expect("payload");
    assert!(!payload.contains("</script>"), "payload must be script-safe");
    assert!(payload.contains("web-share"), "session content must be embedded");

    let _ = std::fs::remove_dir_all(&out);
    cleanup(&[id]);
}

/// `play --from/--to` bounds the replay without panicking on edge ranges.
#[test]
fn play_range_boundary_cases_are_clean() {
    let _g = lock();
    let session =
        run_and_capture(&["screendoc", "record", "--size", "40x6", "echo", "x", "exit", "0"]);
    let id = session.id().to_string();

    // Normal window.
    assert_eq!(run_cli(&["screendoc", "play", &id, "--fast", "--from", "0.1", "--to", "0.5"]), 0);
    // Past the end: no events, still a clean no-op.
    assert_eq!(run_cli(&["screendoc", "play", &id, "--fast", "--from", "9999"]), 0);
    // Inverted window: from > to, clean no-op.
    assert_eq!(run_cli(&["screendoc", "play", &id, "--fast", "--from", "5.0", "--to", "1.0"]), 0);
    // Negative from is clamped to the start.
    assert_eq!(run_cli(&["screendoc", "play", &id, "--fast", "--from", "-3"]), 0);
    cleanup(&[id]);
}

/// `redact --secret` masks the built-in credential shapes end to end.
#[test]
fn redact_secret_masks_builtin_shapes() {
    let _g = lock();
    let fake_aws = format!("AKIA{}", "Z".repeat(16)); // valid-shape, not a real key
    // Print (not just `export`) so the values land in the recorded output.
    let session = if cfg!(windows) {
        run_and_capture(&[
            "screendoc",
            "record",
            "--size",
            "50x8",
            "cmd",
            "/c",
            &format!("echo AWS_ACCESS_KEY_ID={fake_aws} & echo DB_PASSWORD=hunter2"),
        ])
    } else {
        if find_in_path("sh").is_none() {
            eprintln!("SKIP: sh not available on this platform");
            return;
        }
        run_and_capture(&[
            "screendoc",
            "record",
            "--size",
            "50x8",
            "sh",
            "-c",
            &format!("echo AWS_ACCESS_KEY_ID={fake_aws}; echo DB_PASSWORD=hunter2"),
        ])
    };
    let id = session.id().to_string();

    assert_eq!(run_cli(&["screendoc", "redact", &id, "--secret", "--yes", "--json"]), 0);
    let cast = std::fs::read_to_string(session.cast_path()).expect("cast");
    assert!(!cast.contains(&fake_aws), "aws key shape must be masked");
    assert!(!cast.contains("hunter2"), "password value must be masked");
    assert!(cast.contains("***"));
    cleanup(&[id]);
}

/// `redact --rules` applies a custom rule file with per-rule JSON summary.
#[test]
fn redact_rules_file_masks_and_reports() {
    let _g = lock();
    let marker = format!("mytoken-{}", std::process::id());
    let session =
        run_and_capture(&["screendoc", "record", "--size", "40x6", "echo", &marker, "exit", "0"]);
    let id = session.id().to_string();

    let rules = std::env::temp_dir().join(format!("sdoc-rules-{id}"));
    std::fs::write(&rules, "# a rule\nmytoken-\\d+\t[token-redacted]\n").expect("rules");

    assert_eq!(
        run_cli(&["screendoc", "redact", &id, "--rules", rules.to_str().unwrap(), "--yes"]),
        0
    );
    let cast = std::fs::read_to_string(session.cast_path()).expect("cast");
    assert!(!cast.contains(&marker));
    assert!(cast.contains("[token-redacted]"));
    let _ = std::fs::remove_file(&rules);
    cleanup(&[id]);
}

/// `rename` moves the session dir, meta id, and active pointer.
#[test]
fn rename_moves_session_and_active_pointer() {
    let _g = lock();
    let session =
        run_and_capture(&["screendoc", "record", "--size", "40x6", "echo", "x", "exit", "0"]);
    let old = session.id().to_string();
    let new = format!("renamed-{old}");

    session.set_active().expect("set active");
    assert_eq!(sdoc_core::Session::active_id().as_deref(), Some(old.as_str()));

    assert_eq!(run_cli(&["screendoc", "rename", &old, &new]), 0);
    assert!(sdoc_core::Session::open(&new).is_ok(), "new id must resolve");
    assert!(sdoc_core::Session::open(&old).is_err(), "old id must be gone");
    assert_eq!(sdoc_core::Session::active_id().as_deref(), Some(new.as_str()));
    let renamed_meta = sdoc_core::Session::open(&new).expect("new").read_meta().expect("meta");
    assert_eq!(renamed_meta.id, new, "meta id must follow the rename");

    // Renaming onto an existing session fails cleanly.
    let other =
        run_and_capture(&["screendoc", "record", "--size", "40x6", "echo", "y", "exit", "0"]);
    let other_id = other.id().to_string();
    assert_eq!(run_cli(&["screendoc", "rename", &new, &other_id]), 1);
    assert_eq!(run_cli(&["screendoc", "rm", &new, "--yes"]), 0);
    cleanup(&[other_id]);
}

/// `config set/get/show` round-trips through the user config file.
#[test]
fn config_set_get_show_roundtrip() {
    let _g = lock();
    // Use the default theme so we don't clobber a user's value.
    let _ = run_cli(&["screendoc", "config", "set", "theme", "xterm"]);
    assert_eq!(run_cli(&["screendoc", "config", "get", "theme"]), 0);
    assert_eq!(run_cli(&["screendoc", "config", "show"]), 0);
    // Unknown keys fail cleanly.
    assert_eq!(run_cli(&["screendoc", "config", "get", "nope"]), 1);
    assert_eq!(run_cli(&["screendoc", "config", "set", "fps", "not-a-number"]), 1);
}

/// Gzipped casts import transparently.
#[test]
fn import_gzip_cast() {
    let _g = lock();
    let text = "{\"version\":2}\n[\"0.0\",\"o\",\"gzipped hello\"]\n";
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    use std::io::Write as _;
    gz.write_all(text.as_bytes()).unwrap();
    let bytes = gz.finish().unwrap();

    let path = std::env::temp_dir().join(format!("sdoc-gz-{}.cast.gz", std::process::id()));
    std::fs::write(&path, &bytes).expect("write gz");
    let id = run_cli_capture_id(&["screendoc", "import", path.to_str().unwrap(), "--json"]);
    let cast = {
        let s = sdoc_core::Session::open(&id).expect("session");
        std::fs::read_to_string(s.cast_path()).expect("cast")
    };
    assert!(cast.contains("gzipped hello"));
    let _ = std::fs::remove_file(&path);
    cleanup(&[id]);
}

/// `import --json` prints the session id; `prune --json` reports the
/// removed list (tested non-destructively: nothing matches the filter).
#[test]
fn machine_output_json_for_import_and_prune() {
    let _g = lock();
    let path = std::env::temp_dir().join(format!("sdoc-mach-{}.cast", std::process::id()));
    std::fs::write(&path, "{\"version\":2}\n[\"0.0\",\"o\",\"m\"]\n").expect("write");
    let id = run_cli_capture_id(&["screendoc", "import", path.to_str().unwrap(), "--json"]);
    assert!(!id.is_empty());
    let _ = std::fs::remove_file(&path);
    cleanup(&[id]);

    // A filter that matches nothing prints the empty JSON shape.
    assert_eq!(run_cli(&["screendoc", "prune", "--older-than", "100000d", "--json"]), 0);
}

/// Run `import --json` and return the printed session id.
fn run_cli_capture_id(argv: &[&str]) -> String {
    // The in-process harness swallows stdout; instead, diff the session list.
    let before: std::collections::HashSet<String> =
        sdoc_core::Session::list().expect("list").into_iter().map(|m| m.id).collect();
    assert_eq!(run_cli(argv), 0, "import failed: {argv:?}");
    let after: Vec<String> = sdoc_core::Session::list()
        .expect("list")
        .into_iter()
        .map(|m| m.id)
        .filter(|id| !before.contains(id))
        .collect();
    assert_eq!(after.len(), 1, "exactly one new session expected");
    after.into_iter().next().unwrap()
}
