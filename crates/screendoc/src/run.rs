use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use crate::ansi::{AnsiText as _, Bold as _};
use anyhow::{Context, Result};
use clap::CommandFactory;
use sdoc_core::{
    ChapterSource, Config, Event, Marker, MarkerRecord, Meta, Mode, Session, session::now_unix_ms,
};
use sdoc_record::ambient::{self, OnOptions};
use sdoc_record::tmux as tmux_rec;
use sdoc_record::wrapper::{self, WrapperOptions};

use crate::cli::{Cli, Command};
use crate::util::{find_in_path, fmt_duration, parse_duration, parse_size};

/// Execute a parsed CLI invocation and return the process exit code.
pub fn execute(cli: &Cli) -> Result<i32> {
    match &cli.command {
        Command::Record { tmux, tmux_group, size, cwd, command, encrypt } => {
            record(tmux, tmux_group.as_deref(), size, cwd, command, *encrypt)
        }
        Command::On { shell, encrypt } => on(shell, *encrypt),
        Command::Off { session } => off(session),
        Command::HookScript { shell } => hook_script(shell),
        Command::HookAppend { kind, seq, rc, text, session } => {
            hook_append(kind, *seq, *rc, text, session)
        }
        Command::Chapter { title, session } => chapter(title, session),
        Command::Render {
            id,
            gif,
            mp4,
            webm,
            av1,
            cast,
            web,
            chapters,
            no_full,
            frames,
            fps,
            gif_fps,
            speed,
            idle_limit,
            out,
        } => render(
            id,
            RenderFlags {
                gif: *gif,
                mp4: *mp4,
                webm: *webm,
                av1: *av1,
                cast: *cast,
                web: *web,
                chapters: *chapters,
                no_full: *no_full,
                frames: *frames,
                fps: *fps,
                gif_fps: *gif_fps,
                speed: *speed,
                idle_limit: *idle_limit,
            },
            out,
        ),
        Command::Play { id, speed, fast, from, to } => play(id, *speed, *fast, *from, *to),
        Command::Import { file, title, json } => import(file, title, *json),
        Command::Serve { id, port, open } => serve(id, *port, *open),
        Command::Stop { session } => stop(session),
        Command::List { json, csv } => list(*json, *csv),
        Command::Status { json } => status(*json),
        Command::Rm { id, yes } => remove(id, *yes),
        Command::Rename { from, to } => rename(from, to),
        Command::Prune { older_than, keep, all, yes, json } => {
            prune(older_than, *keep, *all, *yes, *json)
        }
        Command::Search { query, session, json } => search(query, session, *json),
        Command::Redact { id, pattern, replace, secret, rules, yes, json } => {
            redact(id, pattern, replace, *secret, rules, *yes, *json)
        }
        Command::Doctor { json } => doctor(*json),
        Command::Config { action } => match action {
            crate::cli::ConfigAction::Show => config_show(),
            crate::cli::ConfigAction::Get { key } => config_get(key),
            crate::cli::ConfigAction::Set { key, value } => config_set(key, value),
        },
        Command::Completions { shell } => completions(*shell),
    }
}

/// `screendoc record` — wrapper mode or tmux pane mode.
fn record(
    tmux: &Option<Option<String>>,
    tmux_group: Option<&str>,
    size: &str,
    cwd: &Option<PathBuf>,
    command: &[String],
    encrypt_flag: bool,
) -> Result<i32> {
    let cfg = Config::load();
    let encrypt = encrypt_flag || cfg.encrypt;
    if let Some(group) = tmux_group.filter(|g| !g.is_empty()) {
        let mut grp = tmux_rec::start_group(group, encrypt)?;
        println!(
            "{} recording all {} panes of tmux session {}…",
            "●".red(),
            grp.ids().len(),
            group.cyan()
        );
        println!(
            "{}",
            "  stop with Ctrl-C, or from another terminal: `screendoc stop <id>` per pane".yellow()
        );
        for id in grp.ids() {
            println!("  session {}", id.dimmed());
        }
        let stop = sdoc_core::signal::flag();
        loop {
            grp.poll()?;
            if stop.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(150));
        }
        let summaries = grp.stop()?;
        for s in &summaries {
            println!(
                "{} session {} finalized ({} commands).",
                "✓".green().bold(),
                s.session.id().bold(),
                s.commands
            );
        }
        return Ok(0);
    }
    match tmux {
        Some(target) => {
            let target = target.as_deref().filter(|s| !s.is_empty());
            let mut rec = tmux_rec::start(target, encrypt)?;
            println!(
                "{} recording tmux pane {} as session {}",
                "●".red(),
                rec.target().cyan(),
                rec.id().bold()
            );
            println!(
                "  stop with Ctrl-C, or from another terminal: {}",
                format!("screendoc stop {}", rec.id()).yellow()
            );
            let stop = sdoc_core::signal::flag();
            loop {
                rec.poll()?;
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                thread::sleep(Duration::from_millis(150));
            }
            let summary = rec.stop()?;
            println!(
                "{} session {} finalized ({} commands, {} ms).",
                "✓".green().bold(),
                summary.session.id().bold(),
                summary.commands,
                summary.session.read_meta()?.duration_ms.unwrap_or(0)
            );
            println!(
                "  render: {}",
                format!("screendoc render {} --gif --mp4 --cast", summary.session.id()).yellow()
            );
            Ok(0)
        }
        None => {
            if command.is_empty() {
                anyhow::bail!(
                    "no command given: run `screendoc record <command>`, or record a tmux pane with `screendoc record --tmux [target]`"
                );
            }
            let size = parse_size(size).context("bad --size; expected COLSXROWS, e.g. 120x30")?;
            let opts =
                WrapperOptions { command: command.to_vec(), size, cwd: cwd.clone(), encrypt };
            println!("{} recording `{}`…", "●".red(), command.join(" ").bold());
            if encrypt {
                println!("{}", "  encrypted at rest (AES-256-GCM, ~/.screendoc/key)".dimmed());
            }
            let summary = wrapper::run_wrapper(&opts)?;
            apply_retention(&cfg);
            let meta = summary.session.read_meta()?;
            println!(
                "{} session {} finalized: {} command(s), {}.",
                "✓".green().bold(),
                summary.session.id().bold(),
                summary.commands,
                fmt_duration(meta.duration_ms.unwrap_or(0))
            );
            println!(
                "  render: {}",
                format!("screendoc render {} --gif --mp4", summary.session.id()).yellow()
            );
            Ok(0)
        }
    }
}

/// `screendoc on` — start an ambient recording.
fn on(shell: &Option<String>, encrypt_flag: bool) -> Result<i32> {
    let cfg = Config::load();
    let encrypt = encrypt_flag || cfg.encrypt;
    let res =
        ambient::on(&OnOptions { shell: shell.clone(), encrypt }).map_err(anyhow::Error::msg)?;
    if encrypt {
        println!("{}", "  encrypted at rest (AES-256-GCM, ~/.screendoc/key)".dimmed());
    }
    println!(
        "{} session {} started ({} mode, {} shell).",
        "●".red(),
        res.session.id().bold(),
        "journal".dimmed(),
        res.shell.cyan()
    );
    println!(
        "Now run this in your shell:\n  {}\nWhen you are done: {}",
        res.eval.yellow(),
        "screendoc off".yellow()
    );
    Ok(0)
}

/// `screendoc off` — stop the ambient recording.
fn off(session: &Option<String>) -> Result<i32> {
    let summary = ambient::off(session.as_deref()).map_err(anyhow::Error::msg)?;
    println!(
        "{} session {} finalized: {} command(s) captured.",
        "✓".green().bold(),
        summary.session.id().bold(),
        summary.commands
    );
    apply_retention(&Config::load());
    println!("  render: {}", format!("screendoc render {} --cast", summary.session.id()).yellow());
    Ok(0)
}

/// `screendoc hook-script <shell>` — print the eval snippet.
fn hook_script(shell: &str) -> Result<i32> {
    let script = ambient::hook_script(shell).map_err(anyhow::Error::msg)?;
    println!("{script}");
    Ok(0)
}

/// `screendoc hook-append` — fast marker append used by shell hooks.
fn hook_append(
    kind: &str,
    seq: Option<u64>,
    rc: Option<i32>,
    text: &Option<String>,
    session: &Option<String>,
) -> Result<i32> {
    ambient::append_marker(kind, seq, rc, text.clone(), session.as_deref())
        .map_err(anyhow::Error::msg)?;
    Ok(0)
}

/// `screendoc chapter <title>` — manual chapter marker.
fn chapter(title: &str, session: &Option<String>) -> Result<i32> {
    let id = ambient::resolve_session_id(session.as_deref()).map_err(anyhow::Error::msg)?;
    let sess = Session::open(&id)?;
    let meta = sess.read_meta()?;
    if meta.mode == Mode::Wrapper && meta.duration_ms.is_some() {
        let at_ms = meta.duration_ms.unwrap_or(0);
        sess.append_event(&Event::Chapter {
            at_ms,
            title: title.to_string(),
            source: ChapterSource::Manual,
        })?;
    } else {
        sess.append_marker_record(&MarkerRecord {
            ts_unix_ms: now_unix_ms(),
            marker: Marker::Chapter { title: title.to_string() },
        })?;
    }
    println!("{} chapter “{}” added to session {}.", "✓".green().bold(), title.cyan(), id.bold());
    Ok(0)
}

/// The render-output flags, grouped so the handler stays readable.
#[derive(Debug, Clone, Copy, Default)]
struct RenderFlags {
    /// Produce a GIF.
    gif: bool,
    /// Produce an MP4.
    mp4: bool,
    /// Produce a WebM (VP9).
    webm: bool,
    /// Produce an AV1 MP4.
    av1: bool,
    /// Export an asciinema cast copy.
    cast: bool,
    /// Produce a self-contained web player (.html).
    web: bool,
    /// Render one video per chapter.
    chapters: bool,
    /// With chapters, skip the full-session video.
    no_full: bool,
    /// Emit PNG stills at chapter boundaries.
    frames: bool,
    /// Override the video frame rate.
    fps: Option<u32>,
    /// Override the GIF frame rate.
    gif_fps: Option<u32>,
    /// Playback speed multiplier.
    speed: Option<f64>,
    /// Idle-time limit (seconds; 0 disables compression).
    idle_limit: Option<f64>,
}

impl RenderFlags {
    /// Whether any output was explicitly requested.
    fn any(&self) -> bool {
        self.gif
            || self.mp4
            || self.webm
            || self.av1
            || self.cast
            || self.web
            || self.chapters
            || self.frames
    }
}

/// `screendoc render <id>` — produce derived artifacts.
fn render(id: &str, flags: RenderFlags, out: &Path) -> Result<i32> {
    let session = Session::open(id)?;
    let cfg = Config::load();
    let mut flags = flags;
    let any = flags.any();
    if !any {
        let meta = session.read_meta()?;
        if !meta.video {
            // Journal-only sessions can only export cast / web.
            anyhow::bail!(
                "session {id} has no video (journal mode); use `screendoc render {id} --cast` or `--web`"
            );
        }
        flags.gif = true;
        flags.mp4 = true;
        flags.cast = true;
    }
    let req = sdoc_render::video::RenderRequest {
        gif: flags.gif,
        mp4: flags.mp4,
        webm: flags.webm,
        av1: flags.av1,
        cast: flags.cast,
        web: flags.web,
        chapters: flags.chapters,
        no_full: flags.no_full,
        frames: flags.frames,
        fps: flags.fps,
        gif_fps: flags.gif_fps,
        speed: flags.speed,
        idle_limit: flags.idle_limit,
        out_dir: out.to_path_buf(),
    };
    let rendered = sdoc_render::video::render(&session, &req, &cfg)?;
    if let Some(p) = &rendered.cast {
        println!("  cast:  {}", p.display().to_string().yellow());
    }
    if let Some(p) = &rendered.web {
        println!(
            "  web:   {} (self-contained: open directly, no server needed)",
            p.display().to_string().yellow()
        );
    }
    if let Some(p) = &rendered.gif {
        println!("  gif:   {}", p.display().to_string().yellow());
    }
    if let Some(p) = &rendered.mp4 {
        println!("  mp4:   {}", p.display().to_string().yellow());
    }
    if let Some(p) = &rendered.webm {
        println!("  webm:  {}", p.display().to_string().yellow());
    }
    if let Some(p) = &rendered.av1 {
        println!("  av1:   {}", p.display().to_string().yellow());
    }
    for p in &rendered.chapter_videos {
        println!("  chap:  {}", p.display().to_string().yellow());
    }
    for p in &rendered.frames {
        println!("  frame: {}", p.display().to_string().yellow());
    }
    Ok(0)
}

/// `screendoc play <id>` — replay a session's cast in this terminal.
fn play(id: &str, speed: f64, fast: bool, from: Option<f64>, to: Option<f64>) -> Result<i32> {
    let session = Session::open(id)?;
    let cast_text = session.read_cast_text()?;
    let parsed = sdoc_core::cast_read::parse_cast_text(&cast_text)
        .map_err(|e| anyhow::anyhow!("failed to parse cast: {e}"))?;
    let start = from.unwrap_or(0.0).max(0.0);
    let stop = to; // None = to the end
    let tty = !fast && std::io::IsTerminal::is_terminal(&std::io::stdout());
    let stop_flag = sdoc_core::signal::flag();
    let mut out = std::io::stdout();
    let mut last = start;
    for ev in &parsed.events {
        if stop_flag.load(Ordering::SeqCst) {
            break;
        }
        if ev.t < start {
            continue;
        }
        if let Some(end) = stop
            && ev.t > end
        {
            break;
        }
        if tty {
            let gap = (ev.t - last).max(0.0) / speed.max(1e-9);
            let mut remaining = gap;
            while remaining > 0.0 && !stop_flag.load(Ordering::SeqCst) {
                let chunk = remaining.min(0.02);
                thread::sleep(Duration::from_secs_f64(chunk));
                remaining -= chunk;
            }
            last = ev.t;
        }
        replay_event(ev, &mut out)?;
    }
    out.flush()?;
    Ok(0)
}

/// Write a single cast event to a writer. Pure enough to test without a TTY.
pub fn replay_event(ev: &sdoc_core::CastEvent, w: &mut impl std::io::Write) -> Result<()> {
    match ev.ty.as_str() {
        "o" | "i" => {
            w.write_all(ev.data.as_bytes())?;
        }
        "r" => {
            if let Some((c, r)) = sdoc_render::video::parse_resize(&ev.data) {
                write!(w, "\x1b[8;{r};{c}t")?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// `screendoc import <file>` — bring an asciinema cast in as a session.
fn import(file: &Path, title: &Option<String>, json: bool) -> Result<i32> {
    // Plain text or gzip (magic-byte detected), non-UTF-8 tolerated:
    // garbage lines are skipped by the parser, so a partially-corrupt cast
    // still imports what is readable.
    let text = sdoc_core::cast_read::read_cast_file(file)?;
    let parsed = sdoc_core::cast_read::parse_cast_text(&text)
        .map_err(|e| anyhow::anyhow!("unsupported cast file: {e}"))?;

    let size =
        sdoc_core::Size { cols: parsed.cols.unwrap_or(120), rows: parsed.rows.unwrap_or(30) };
    let last = parsed.events.iter().map(|e| e.t).fold(0.0f64, f64::max);
    let id = sdoc_core::session::new_session_id();
    let meta = sdoc_core::Meta {
        id: id.clone(),
        started_unix_ms: sdoc_core::session::now_unix_ms(),
        shell: None,
        cwd: None,
        mode: sdoc_core::Mode::Journal,
        video: true,
        pane_target: None,
        duration_ms: Some((last * 1000.0) as u64),
        title: title.clone(),
        group: None,
    };
    let session = sdoc_core::Session::create(&meta)?;

    // Rewrite the (possibly v3) cast into screendoc's v2 byte stream.
    let mut writer = sdoc_core::CastWriter::new(&session.cast_path(), size)?;
    for ev in &parsed.events {
        let at_ms = (ev.t * 1000.0) as u64;
        match ev.ty.as_str() {
            "o" | "i" => writer.output(at_ms, &ev.data)?,
            "r" => {
                if let Some((c, r)) = sdoc_render::video::parse_resize(&ev.data) {
                    writer.resize(at_ms, sdoc_core::Size { cols: c, rows: r })?;
                }
            }
            _ => {}
        }
    }
    writer.finish()?;

    // Emit structured events so the session is searchable and chapterable.
    session.append_event(&Event::SessionStart { at_ms: 0, shell: None, size })?;
    for ev in &parsed.events {
        if ev.ty == "m" {
            session.append_event(&Event::Chapter {
                at_ms: (ev.t * 1000.0) as u64,
                title: ev.data.clone(),
                source: sdoc_core::ChapterSource::Manual,
            })?;
        }
    }
    let at_ms = (last * 1000.0) as u64;
    session.append_event(&Event::SessionEnd { at_ms, reason: "imported".into() })?;

    if json {
        println!("{}", serde_json::json!({ "id": id }));
    } else {
        println!(
            "{} imported {} as session {}",
            "✓".green().bold(),
            file.display().to_string().cyan(),
            id.bold()
        );
        println!("  render: {}", format!("screendoc render {id} --gif --mp4").yellow());
    }
    Ok(0)
}

/// `screendoc prune` — delete old sessions.
fn prune(
    older_than: &Option<String>,
    keep: Option<u64>,
    all: bool,
    yes: bool,
    json: bool,
) -> Result<i32> {
    let metas = Session::list()?;
    let now_ms = now_unix_ms();
    let older_ms = match older_than {
        Some(d) => parse_duration(d).context("bad --older-than; expected e.g. 30d, 7w, 24h")?,
        None => 0,
    };
    let mut doomed: Vec<Meta> = if all {
        metas
    } else {
        let mut v: Vec<Meta> = metas
            .iter()
            .filter(|m| now_ms.saturating_sub(m.started_unix_ms) > older_ms)
            .cloned()
            .collect();
        if let Some(n) = keep {
            // Never drop the newest `n`.
            v.retain(|m| !metas.iter().take(n as usize).any(|k| k.id == m.id));
        }
        v
    };
    if keep.is_none() && !all {
        // Nothing to bound by recency unless older_than given; keep is a floor.
    }
    doomed.sort_by_key(|m| m.started_unix_ms);
    if doomed.is_empty() {
        if json {
            println!("{}", serde_json::json!({ "removed": [] }));
        } else {
            println!("{} nothing to prune.", "✓".green().bold());
        }
        return Ok(0);
    }
    if !yes {
        if json {
            anyhow::bail!("prune --json is machine output; pass --yes to act without prompting");
        }
        println!("Would remove {} session(s):", doomed.len());
        for m in &doomed {
            println!("  {}", m.id.dimmed());
        }
        print!("Remove? [y/N] ");
        std::io::stdout().flush()?;
        let mut a = String::new();
        std::io::stdin().read_line(&mut a)?;
        if !a.trim().eq_ignore_ascii_case("y") {
            println!("{}", "aborted.".dimmed());
            return Ok(1);
        }
    }
    let mut removed: Vec<String> = Vec::new();
    for m in doomed {
        if Session::open(&m.id)?.remove().is_ok() {
            removed.push(m.id);
        }
    }
    if json {
        println!("{}", serde_json::json!({ "removed": removed }));
    } else {
        println!("{} removed {} session(s).", "✓".green().bold(), removed.len());
    }
    Ok(0)
}

/// `screendoc search <query>` — find commands across sessions.
fn search(query: &str, session: &Option<String>, json: bool) -> Result<i32> {
    let hits = search_hits(query, session.as_deref())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&hits)?);
        return Ok(0);
    }
    if hits.is_empty() {
        println!("{}", format!("no matches for `{query}`.").dimmed());
        return Ok(0);
    }
    for h in &hits {
        match h["kind"].as_str() {
            Some("output") => {
                println!(
                    "{} {}  {}",
                    h["session"].as_str().unwrap_or("").dimmed(),
                    "output".dimmed(),
                    h["excerpt"].as_str().unwrap_or("").cyan()
                );
            }
            _ => {
                let rc = if h["exit_code"] == 0 {
                    "✓".green().to_string()
                } else {
                    "✗".red().to_string()
                };
                println!(
                    "{} {} cmd {}ms {}",
                    h["session"].as_str().unwrap_or("").dimmed(),
                    rc,
                    h["at_ms"].as_u64().unwrap_or(0),
                    truncate(h["command"].as_str().unwrap_or(""), 60).cyan()
                );
            }
        }
    }
    Ok(0)
}

/// Find events matching `query` (case-insensitive substring) across
/// sessions, optionally restricted to one session.
///
/// Matches command events (structured) and, when a session has a raw cast,
/// the captured terminal output as well (`kind: "output"`).
pub fn search_hits(query: &str, only: Option<&str>) -> Result<Vec<serde_json::Value>> {
    let needle = query.to_ascii_lowercase();
    let metas = Session::list()?;
    let mut hits: Vec<serde_json::Value> = Vec::new();
    for m in &metas {
        if let Some(only) = only
            && only != m.id.as_str()
        {
            continue;
        }
        // A listed session whose directory was removed mid-run (or is
        // corrupt/unreadable, e.g. encrypted under a missing key) must not
        // break a global search: skip it with a note.
        let sess = match Session::open(&m.id) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{} search: skipping {}: {e}", "⚠".yellow(), m.id);
                continue;
            }
        };
        match sess.read_events() {
            Ok(events) => {
                for e in events {
                    if let Event::Command { at_ms, seq, exit_code, command } = &e
                        && command.to_ascii_lowercase().contains(&needle)
                    {
                        hits.push(serde_json::json!({
                            "kind": "command",
                            "session": m.id,
                            "at_ms": at_ms,
                            "seq": seq,
                            "exit_code": exit_code,
                            "command": command,
                        }));
                    }
                }
            }
            Err(e) => {
                eprintln!("{} search: skipping {}: {e}", "⚠".yellow(), m.id);
            }
        }
        // Also scan the raw byte stream (journal sessions have none).
        if m.video
            && let Ok(cast) = sess.read_cast_text()
            && cast.to_ascii_lowercase().contains(&needle)
        {
            let excerpt = extract_excerpt(&cast, &needle);
            hits.push(serde_json::json!({
                "kind": "output",
                "session": m.id,
                "excerpt": excerpt,
            }));
        }
    }
    Ok(hits)
}

/// A short, printable excerpt of `hay` around the first (case-insensitive)
/// occurrence of `needle`.
fn extract_excerpt(hay: &str, needle: &str) -> String {
    let hay_low = hay.to_ascii_lowercase();
    let start = hay_low.find(needle).unwrap_or(0);
    let lo = start.saturating_sub(20);
    let hi = (start + needle.len() + 40).min(hay.len());
    // Snap to char boundaries.
    let (lo, hi) = (hay.floor_char_boundary(lo), hay.ceil_char_boundary(hi).min(hay.len()));
    format!("…{}…", &hay[lo..hi])
}

/// `screendoc redact <id>` — mask secrets out of a session in place.
///
/// Rule sources, applied in this order: built-in secret detectors
/// (`--secret`), rules-file rules (`--rules`), and finally the ad-hoc
/// PATTERN (so the user's pattern wins on overlaps).
fn redact(
    id: &str,
    pattern: &Option<String>,
    replace: &str,
    secret: bool,
    rules: &Option<PathBuf>,
    yes: bool,
    json: bool,
) -> Result<i32> {
    use sdoc_core::redact::{CompiledRule, parse_rules_file, redact_session_rules, secret_rules};

    let mut rule_list: Vec<CompiledRule> = Vec::new();
    if secret {
        rule_list.extend(secret_rules());
    }
    if let Some(f) = rules {
        let text = std::fs::read_to_string(f)
            .with_context(|| format!("failed to read rules file {}", f.display()))?;
        rule_list.extend(parse_rules_file(&text)?);
    }
    if let Some(p) = pattern {
        rule_list.push(CompiledRule {
            name: "pattern".into(),
            re: sdoc_core::redact::compile_pattern(p)
                .map_err(|e| anyhow::anyhow!("invalid pattern `{p}`: {e}"))?,
            replacement: replace.to_string(),
        });
    }
    if rule_list.is_empty() {
        anyhow::bail!("give a pattern, or --secret, or --rules FILE");
    }

    let session = Session::open(id)?;
    let desc: Vec<String> = rule_list.iter().map(|r| r.name.clone()).collect();
    if !yes {
        print!(
            "Redact session {id} with {} rule(s): {} (irreversible)? [y/N] ",
            desc.len(),
            desc.join(", ")
        );
        std::io::stdout().flush()?;
        let mut a = String::new();
        std::io::stdin().read_line(&mut a)?;
        if !a.trim().eq_ignore_ascii_case("y") {
            println!("{}", "aborted.".dimmed());
            return Ok(1);
        }
    }
    let summary = redact_session_rules(&session, &rule_list)?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "id": id,
                "total": summary.total(),
                "cast_replacements": summary.cast_replacements,
                "event_replacements": summary.event_replacements,
                "by_rule": summary.by_rule,
            })
        );
    } else {
        println!(
            "{} masked {} cast occurrence(s), {} event occurrence(s) in {}.",
            "✓".green().bold(),
            summary.cast_replacements,
            summary.event_replacements,
            id.bold()
        );
        for hit in &summary.by_rule {
            if hit.rule == "pattern" {
                continue;
            }
            println!("  {} [{}]", hit.rule.cyan(), hit.masked);
        }
    }
    Ok(0)
}

/// `screendoc rename <from> <to>` — rename a session id.
fn rename(from: &str, to: &str) -> Result<i32> {
    let session = Session::open(from)?;
    let renamed = session.rename(to)?;
    println!("{} renamed {} → {}.", "✓".green().bold(), from.cyan(), renamed.id().bold());
    Ok(0)
}

/// `screendoc config show` — the effective configuration.
fn config_show() -> Result<i32> {
    let cfg = Config::load();
    println!("{}", serde_json::to_string_pretty(&cfg)?);
    if let Some(p) = Config::path() {
        if p.exists() {
            println!("{}", format!("(file: {})", p.display()).dimmed());
        } else {
            println!("{}", "(defaults only; no config file yet)".dimmed());
        }
    }
    Ok(0)
}

/// `screendoc config get <key>` — one effective key.
fn config_get(key: &str) -> Result<i32> {
    let cfg = Config::load();
    let v = config_key_value(&cfg, key)?;
    println!("{v}");
    Ok(0)
}

/// `screendoc config set <key> <value>` — update the user config file.
fn config_set(key: &str, value: &str) -> Result<i32> {
    let cfg = Config::load();
    let mut next = cfg.clone();
    apply_config_key(&mut next, key, value)?;
    let path = Config::path()
        .ok_or_else(|| anyhow::anyhow!("cannot locate a config directory on this platform"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, toml::to_string_pretty(&next)?)?;
    println!("{} set {key} = {} ({})", "✓".green().bold(), value.bold(), path.display());
    Ok(0)
}

/// The current value of a config key, as a display string.
fn config_key_value(cfg: &Config, key: &str) -> Result<String> {
    Ok(match key {
        "fps" => cfg.fps.to_string(),
        "gif_fps" => cfg.gif_fps.to_string(),
        "speed" => cfg.speed.to_string(),
        "cols" => cfg.cols.to_string(),
        "rows" => cfg.rows.to_string(),
        "port" => cfg.port.to_string(),
        "cell_width" => cfg.cell_width.to_string(),
        "cell_height" => cfg.cell_height.to_string(),
        "idle_limit" => cfg.idle_limit.to_string(),
        "theme" => cfg.theme.clone(),
        "encrypt" => cfg.encrypt.to_string(),
        "retention_days" => cfg.retention_days.to_string(),
        other => anyhow::bail!(
            "unknown config key `{other}`; known keys: fps, gif_fps, speed, cols, rows, port, \
             cell_width, cell_height, idle_limit, encrypt, retention_days, theme"
        ),
    })
}

/// Set one config key from its string form, type-checked per key.
fn apply_config_key(cfg: &mut Config, key: &str, value: &str) -> Result<()> {
    match key {
        "fps" => cfg.fps = parse_u32(key, value)?,
        "gif_fps" => cfg.gif_fps = parse_u32(key, value)?,
        "speed" => cfg.speed = parse_f64(key, value)?,
        "cols" => cfg.cols = parse_u16(key, value)?,
        "rows" => cfg.rows = parse_u16(key, value)?,
        "port" => cfg.port = parse_u16(key, value)?,
        "cell_width" => cfg.cell_width = parse_u32(key, value)?,
        "cell_height" => cfg.cell_height = parse_u32(key, value)?,
        "idle_limit" => cfg.idle_limit = parse_f64(key, value)?,
        "encrypt" => match value {
            "true" | "1" => cfg.encrypt = true,
            "false" | "0" => cfg.encrypt = false,
            other => anyhow::bail!("config key `encrypt` needs true/false, got `{other}`"),
        },
        "retention_days" => {
            let v = parse_u32(key, value)?;
            if v > 3650 {
                anyhow::bail!("retention_days must be <= 3650 (10 years)");
            }
            cfg.retention_days = v;
        }
        "theme" => {
            let known = ["xterm", "dracula", "nord", "tokyo", "solarized"];
            if !known.contains(&value) {
                anyhow::bail!("unknown theme `{value}`; known themes: {}", known.join(", "));
            }
            cfg.theme = value.to_string();
        }
        other => anyhow::bail!(
            "unknown config key `{other}`; known keys: fps, gif_fps, speed, cols, rows, port, \
             cell_width, cell_height, idle_limit, encrypt, retention_days, theme"
        ),
    }
    Ok(())
}

fn parse_u32(key: &str, v: &str) -> Result<u32> {
    v.parse().map_err(|_| anyhow::anyhow!("config key `{key}` needs a positive integer, got `{v}`"))
}

fn parse_u16(key: &str, v: &str) -> Result<u16> {
    v.parse()
        .map_err(|_| anyhow::anyhow!("config key `{key}` needs an integer 1..65535, got `{v}`"))
}

fn parse_f64(key: &str, v: &str) -> Result<f64> {
    v.parse().map_err(|_| anyhow::anyhow!("config key `{key}` needs a number, got `{v}`"))
}

/// `screendoc serve <id>` — local web player.
fn serve(id: &str, port: Option<u16>, open: bool) -> Result<i32> {
    let session = Session::open(id)?;
    let cfg = Config::load();
    let port = port.unwrap_or(cfg.port);
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let player = rt
        .block_on(sdoc_render::serve::serve(&session, port))
        .with_context(|| format!("failed to start web player on port {port}"))?;
    let url = sdoc_render::player_url(&player.addr);
    println!("{} session {} at {}\n  stop with Ctrl-C", "●".red(), session.id().bold(), url.cyan());
    if open {
        webbrowser::open(&url).ok();
    }
    let stop = sdoc_core::signal::flag();
    while !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(200));
    }
    println!("{}", "stopped.".dimmed());
    Ok(0)
}

/// `screendoc stop` — rescue-finalize a tmux recording.
fn stop(session: &Option<String>) -> Result<i32> {
    let sess = Session::resolve(session.as_deref())?;
    let meta = sess.read_meta()?;
    let events = sess.read_events()?;
    if let Some(target) = &meta.pane_target
        && let Some(tmux) = find_in_path("tmux")
    {
        let _ =
            process::Command::new(&tmux).arg("pipe-pane").arg("-t").arg(target).arg("-o").status();
    }
    if !events.iter().any(|e| matches!(e, Event::SessionEnd { .. })) {
        let at_ms = now_unix_ms().saturating_sub(meta.started_unix_ms);
        sess.append_event(&Event::SessionEnd { at_ms, reason: "stopped".into() })?;
        let mut finalized = meta.clone();
        if finalized.duration_ms.is_none() {
            finalized.duration_ms = Some(at_ms);
        }
        sess.save_meta(&finalized)?;
        Session::clear_active(sess.id())?;
    }
    println!("{} session {} stopped.", "✓".green().bold(), sess.id().bold());
    Ok(0)
}

/// `screendoc list`.
fn list(json: bool, csv: bool) -> Result<i32> {
    let metas = Session::list()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&metas)?);
        return Ok(0);
    }
    if metas.is_empty() {
        if csv {
            println!("id,started_unix_ms,duration_ms,commands,mode,title,encrypted");
        } else {
            println!("{}", "no sessions recorded yet.".dimmed());
        }
        return Ok(0);
    }
    if csv {
        println!("id,started_unix_ms,duration_ms,commands,mode,title,encrypted");
        for m in &metas {
            let cmds = command_count(m);
            let enc = encrypted_mark(m);
            let title = m.title.clone().unwrap_or_default().replace(',', " ");
            println!(
                "{},{},{},{},{},{},{}",
                m.id,
                m.started_unix_ms,
                m.duration_ms.unwrap_or(0),
                cmds,
                mode_str(m.mode),
                title,
                enc
            );
        }
        return Ok(0);
    }
    let header = format!(
        "{:<24} {:>8} {:>8} {:>6} {:<8} {}",
        "ID", "STARTED", "DURATION", "CMDS", "MODE", "TITLE"
    );
    println!("{}", header.bold());
    for m in &metas {
        let cmds = command_count(m);
        let dur = m.duration_ms.map(fmt_duration).unwrap_or_else(|| "…".into());
        let title = m.title.clone().unwrap_or_default();
        let id = format!("{}{}", truncate(&m.id, 24), encrypted_mark(m));
        let line = format!(
            "{:<24} {:>8} {:>8} {:>6} {:<8} {}",
            id,
            fmt_started(m.started_unix_ms),
            dur,
            cmds,
            mode_str(m.mode),
            truncate(&title, 36)
        );
        println!("{line}");
    }
    Ok(0)
}

/// A `*` marker for encrypted sessions (sidecar check, no key needed).
fn encrypted_mark(m: &Meta) -> &'static str {
    if Session::open(&m.id).ok().map(|s| s.is_encrypted()).unwrap_or(false) { "*" } else { " " }
}

/// Apply the configured automatic retention (config `retention_days`),
/// after each recording finalizes. Silent; prints one line per removal.
fn apply_retention(cfg: &Config) {
    if cfg.retention_days == 0 {
        return;
    }
    let older_ms = u64::from(cfg.retention_days).saturating_mul(86_400_000);
    let metas = match Session::list() {
        Ok(metas) => metas,
        Err(_) => return,
    };
    let now_ms = now_unix_ms();
    for m in &metas {
        if now_ms.saturating_sub(m.started_unix_ms) > older_ms
            && Session::open(&m.id).ok().as_ref().is_some_and(|s| s.remove().is_ok())
        {
            println!(
                "{} retained: removed session {} (older than {}d).",
                "ℹ".yellow(),
                m.id.dimmed(),
                cfg.retention_days
            );
        }
    }
}

/// Count command events in a session (best effort).
fn command_count(meta: &Meta) -> String {
    match Session::open(&meta.id) {
        Ok(s) => s
            .read_events()
            .map(|e| e.iter().filter(|e| matches!(e, Event::Command { .. })).count())
            .unwrap_or_default()
            .to_string(),
        Err(_) => "-".into(),
    }
}

/// `screendoc status`.
fn status(json: bool) -> Result<i32> {
    let id = match Session::active_id() {
        Some(id) => id,
        None => {
            if json {
                println!("{}", serde_json::json!({ "active": null }));
            } else {
                println!("{}", "no active session.".dimmed());
            }
            return Ok(0);
        }
    };
    let sess = Session::open(&id)?;
    let meta = sess.read_meta()?;
    if json {
        let mut v = serde_json::to_value(&meta)?;
        if let Some(obj) = v.as_object_mut() {
            obj.insert("active".into(), serde_json::json!(true));
        }
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!(
            "{} {} — {} mode, {} video, {}",
            "active".green().bold(),
            id.bold(),
            mode_str(meta.mode),
            if meta.video { "with" } else { "without" },
            meta.title.clone().unwrap_or_else(|| "(untitled)".to_string())
        );
    }
    Ok(0)
}

/// `screendoc rm <id>`.
fn remove(id: &str, yes: bool) -> Result<i32> {
    let sess = Session::open(id)?;
    if !yes {
        print!("Remove session {id}? [y/N] ");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("{}", "aborted.".dimmed());
            return Ok(1);
        }
    }
    Session::clear_active(id)?;
    sess.remove()?;
    println!("{} session {} removed.", "✓".green().bold(), id.bold());
    Ok(0)
}

/// `screendoc doctor`.
fn doctor(json: bool) -> Result<i32> {
    let ffmpeg = find_in_path("ffmpeg").is_some();
    let tmux = find_in_path("tmux").is_some();
    let shell = find_in_path("bash").is_some();
    let cfg_path = sdoc_core::Config::path();
    let data = sdoc_core::session::data_root();
    let sessions = Session::list()?.len();
    if json {
        println!(
            "{}",
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "ffmpeg": ffmpeg,
                "tmux": tmux,
                "shell": shell,
                "config_path": cfg_path.map(|p| p.to_string_lossy().to_string()),
                "data_root": data.to_string_lossy(),
                "sessions": sessions,
            })
        );
        return Ok(0);
    }
    print_check("ffmpeg (video renders)", ffmpeg);
    print_check("tmux (optional, for `record --tmux`)", tmux);
    print_check("bash (shell the wrapper records)", shell);
    println!(
        "  config:  {}",
        cfg_path.map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "-".into())
    );
    println!("  data:    {}", data.to_string_lossy());
    println!("  sessions: {sessions}");

    // Platform-specific remediation, because the same miss means different
    // things on each OS.
    if !ffmpeg {
        println!(
            "  hint:    video rendering needs ffmpeg; install it ({}).",
            install_hint("ffmpeg")
        );
    }
    if !shell {
        println!(
            "  hint:    the recording wrapper launches `bash`; install it via {}.",
            install_hint("git")
        );
    }
    if !tmux && cfg!(windows) {
        println!(
            "  hint:    tmux is not on Windows; use `record` (wrapper) or run screendoc inside WSL for tmux mode."
        );
    }
    Ok(0)
}

/// A short, OS-aware "how to install X" string for doctor hints.
fn install_hint(tool: &str) -> &'static str {
    // Termux (Android) advertises itself via $PREFIX.
    let termux = std::env::var("PREFIX").ok().map(|p| p.contains("com.termux")).unwrap_or(false);
    if termux {
        match tool {
            "ffmpeg" => "pkg install ffmpeg",
            "git" => "pkg install bash",
            _ => "pkg install …",
        }
    } else if cfg!(windows) {
        match tool {
            "ffmpeg" => "winget install Gyan.FFmpeg",
            "git" => "winget install Git.Git",
            _ => "winget",
        }
    } else if cfg!(target_os = "macos") {
        "brew install …"
    } else {
        "apt-get/brew/dnf/pacman install …"
    }
}

fn print_check(label: &str, ok: bool) {
    if ok {
        println!("  {} {}", "✓".green(), label);
    } else {
        println!("  {} {}", "✗".red(), label);
    }
}

/// `screendoc completions <shell>`.
fn completions(shell: clap_complete::Shell) -> Result<i32> {
    let mut buf = Vec::new();
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "screendoc", &mut buf);
    let mut out = std::io::stdout();
    out.write_all(&buf)?;
    Ok(0)
}

// ---- small helpers ---------------------------------------------------------

/// Truncate a string to `n` characters (for table columns).
fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n { format!("{t}…") } else { t }
}

/// A session mode as a short label.
fn mode_str(m: Mode) -> &'static str {
    match m {
        Mode::Wrapper => "wrapper",
        Mode::Tmux => "tmux",
        Mode::Journal => "journal",
    }
}

/// Format a Unix-millis timestamp as a short local date-time.
fn fmt_started(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}Z")
}

/// Convert a day count since the Unix epoch to a (year, month, day) triple.
///
/// Based on Howard Hinnant's `civil_from_days` algorithm (see the
/// `days_from_civil`/`civil_from_days` pair in his reference C++ `date.h`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::{civil_from_days, extract_excerpt, fmt_started, replay_event};

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 1970-03-01: January (31) + February (28)
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        // 2000-02-29 (leap day): 30 years incl. 7 leap days + 31 + 28
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        // 1900-02-28 (century non-leap)
        assert_eq!(civil_from_days(-25_509), (1900, 2, 28));
        // Century-leap boundaries (verified against Python's calendar).
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_days(47_540), (2100, 2, 28));
        assert_eq!(civil_from_days(47_541), (2100, 3, 1));
        assert_eq!(civil_from_days(-135_080), (1600, 3, 1));
    }

    #[test]
    fn started_format() {
        assert_eq!(fmt_started(0), "1970-01-01 00:00:00Z");
        assert_eq!(fmt_started(86_400_000 + 3_600_000), "1970-01-02 01:00:00Z");
        // No overflow-panic for recent timestamps in debug builds either.
        let _ = fmt_started(1_794_034_359_667);
    }

    #[test]
    fn replay_event_writes_payloads() {
        use sdoc_core::CastEvent;
        let mut buf = Vec::new();
        replay_event(&CastEvent { t: 0.0, ty: "o".into(), data: "hello".into() }, &mut buf)
            .unwrap();
        replay_event(&CastEvent { t: 1.0, ty: "r".into(), data: "120x40".into() }, &mut buf)
            .unwrap();
        // Markers are not terminal bytes.
        replay_event(&CastEvent { t: 2.0, ty: "m".into(), data: "x".into() }, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("hello"));
        assert!(s.contains("\x1b[8;40;120t"));
        assert!(!s.contains("x\r"));
    }

    #[test]
    fn replay_event_ignores_bad_resize() {
        use sdoc_core::CastEvent;
        let mut buf = Vec::new();
        replay_event(&CastEvent { t: 0.0, ty: "r".into(), data: "nonsense".into() }, &mut buf)
            .unwrap();
        assert!(buf.is_empty(), "unparseable resize must not emit bytes");
    }

    #[test]
    fn excerpt_handles_boundaries_and_utf8() {
        // Match at the very start (no left context).
        assert_eq!(extract_excerpt("needle here", "needle"), "…needle here…");
        // CJK characters at the window edges must not split a char.
        let hay = "日本語日本語日本語needle日本語日本語";
        let ex = extract_excerpt(hay, "needle");
        assert!(ex.contains("needle"));
        std::str::from_utf8(ex.as_bytes()).expect("valid utf-8");
        // Empty needle falls back to a context window from 0.
        assert!(!extract_excerpt("abc", "").is_empty());
    }
}
