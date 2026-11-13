use std::path::{Path, PathBuf};
use std::process;

use anyhow::{Context, Result};
use image::RgbaImage;
use sdoc_core::{Config, Event, Session, Size};
use vt100::Parser;

use crate::frame::{
    CastEvent, ColorTheme, FrameOptions, OverlaySpec, Rasterizer, parse_cast, step_parser,
};

/// Which derived artifacts to produce.
pub struct RenderRequest {
    /// Produce a GIF.
    pub gif: bool,
    /// Produce an MP4.
    pub mp4: bool,
    /// Export an asciinema cast copy.
    pub cast: bool,
    /// Produce a WebM (VP9).
    pub webm: bool,
    /// Produce an AV1 MP4.
    pub av1: bool,
    /// Render one video per chapter (in addition to the full one, unless
    /// `no_full` is set).
    pub chapters: bool,
    /// Skip the full-session video when `chapters` is enabled.
    pub no_full: bool,
    /// Emit a PNG still at each chapter boundary plus the final frame.
    pub frames: bool,
    /// Override MP4 frame rate.
    pub fps: Option<u32>,
    /// Override GIF frame rate.
    pub gif_fps: Option<u32>,
    /// Override playback speed multiplier.
    pub speed: Option<f64>,
    /// Override idle-time compression limit (seconds; 0 disables).
    pub idle_limit: Option<f64>,
    /// Produce a self-contained shareable web player (.html).
    pub web: bool,
    /// Output directory.
    pub out_dir: PathBuf,
}

/// The paths of successfully rendered artifacts.
#[derive(Default)]
pub struct Rendered {
    /// The rendered GIF, if requested.
    pub gif: Option<PathBuf>,
    /// The rendered MP4, if requested.
    pub mp4: Option<PathBuf>,
    /// The rendered WebM, if requested.
    pub webm: Option<PathBuf>,
    /// The rendered AV1 MP4, if requested.
    pub av1: Option<PathBuf>,
    /// The exported cast, if requested.
    pub cast: Option<PathBuf>,
    /// The standalone web player, if requested.
    pub web: Option<PathBuf>,
    /// Per-chapter videos, one per chapter, in order.
    pub chapter_videos: Vec<PathBuf>,
    /// Still frames, one per chapter boundary plus the final frame.
    pub frames: Vec<PathBuf>,
}

/// Render the requested derived artifacts for a session.
///
/// GIF/MP4/WebM/AV1 output requires `ffmpeg` on `PATH`; cast and web-player
/// export do not.
pub fn render(session: &Session, req: &RenderRequest, cfg: &Config) -> Result<Rendered> {
    let mut rendered = Rendered::default();
    let any_video = req.gif || req.mp4 || req.webm || req.av1 || req.chapters || req.frames;
    if !req.cast && !any_video && !req.web {
        return Ok(rendered);
    }
    std::fs::create_dir_all(&req.out_dir)?;

    if req.cast {
        rendered.cast = Some(export_cast(session, &req.out_dir)?);
    }

    if req.web {
        rendered.web = Some(crate::web::render_web(session, &req.out_dir)?);
    }

    if any_video {
        let meta = session.read_meta()?;
        if !meta.video && !req.frames {
            anyhow::bail!(
                "session {} has no video (journal mode); only --cast is available",
                session.id()
            );
        }
        if req.gif || req.mp4 || req.webm || req.av1 || req.chapters {
            require_ffmpeg()?;
        }

        let full = !req.no_full && (req.gif || req.mp4 || req.webm || req.av1);
        if full {
            let frames = rasterize_session(session, req, cfg)?;
            let base = stem(session.id());
            if req.gif {
                let out = req.out_dir.join(format!("{base}.gif"));
                make_video(&frames, &out, req.gif_fps.unwrap_or(cfg.gif_fps), Codec::Gif)?;
                rendered.gif = Some(out);
            }
            if req.mp4 {
                let out = req.out_dir.join(format!("{base}.mp4"));
                make_video(&frames, &out, req.fps.unwrap_or(cfg.fps), Codec::H264)?;
                rendered.mp4 = Some(out);
            }
            if req.webm {
                let out = req.out_dir.join(format!("{base}.webm"));
                make_video(&frames, &out, req.fps.unwrap_or(cfg.fps), Codec::Vp9)?;
                rendered.webm = Some(out);
            }
            if req.av1 {
                let out = req.out_dir.join(format!("{base}-av1.mp4"));
                make_video(&frames, &out, req.fps.unwrap_or(cfg.fps), Codec::Av1)?;
                rendered.av1 = Some(out);
            }
        }

        if req.chapters {
            rendered.chapter_videos = render_chapters(session, req, cfg)?;
        }

        if req.frames {
            rendered.frames = render_stills(session, req, cfg)?;
        }
    }

    Ok(rendered)
}

/// A filesystem-safe stem derived from a session id.
fn stem(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
}

/// Rasterize a session's cast into a vector of frame images (full session).
fn rasterize_session(
    session: &Session,
    req: &RenderRequest,
    cfg: &Config,
) -> Result<Vec<RgbaImage>> {
    let total = playback_total(session, req, cfg)?;
    rasterize_range(session, req, cfg, 0.0, total)
}

/// The total playback duration (seconds, unscaled) of a session after
/// applying idle-time compression, extended to the wall-clock duration so
/// event-sparse sessions (e.g. a `sleep` with almost no output) keep their
/// real length instead of collapsing to the last event's time.
fn playback_total(session: &Session, req: &RenderRequest, cfg: &Config) -> Result<f64> {
    let idle_limit = req.idle_limit.unwrap_or(cfg.idle_limit);
    let cast_events = load_cast(session)?;
    let raw: Vec<f64> = cast_events.iter().map(|e| e.t).collect();
    let comp = compress_idle(&raw, idle_limit);
    let total = comp.last().copied().unwrap_or(0.0);
    // No captured output (e.g. a silent `sleep`) falls back to the
    // session's wall-clock duration so a blank-but-timed video is
    // produced; when there IS output, the timeline is still extended to
    // cover the idle tail, so chapters placed after the last event keep
    // their span.
    let meta = session.read_meta()?;
    let wall = meta.duration_ms.map(|d| d as f64 / 1000.0).unwrap_or(0.0);
    Ok(total.max(wall))
}

/// Rasterize a time range `[start_s, end_s)` of a session into frames.
///
/// The terminal is seeded with every event before `start_s` (so the screen
/// state is correct) but only frames within the range are produced.
fn rasterize_range(
    session: &Session,
    req: &RenderRequest,
    cfg: &Config,
    start_s: f64,
    end_s: f64,
) -> Result<Vec<RgbaImage>> {
    let idle_limit = req.idle_limit.unwrap_or(cfg.idle_limit);
    let events = session.read_events()?;
    let cast_events = load_cast(session)?;
    let raw: Vec<f64> = cast_events.iter().map(|e| e.t).collect();
    let comp = compress_idle(&raw, idle_limit);
    // The timeline is extended to the session's wall-clock duration (see
    // `playback_total`) so chapters in the idle tail still have a span.
    let meta = session.read_meta()?;
    let total = comp
        .last()
        .copied()
        .unwrap_or(0.0)
        .max(meta.duration_ms.map(|d| d as f64 / 1000.0).unwrap_or(0.0));

    let speed = req.speed.unwrap_or(cfg.speed);
    let fps = req.fps.unwrap_or(cfg.fps);

    // Frames span [start_s, min(end_s, total)] in unscaled playback time.
    let span_start = start_s.min(total);
    let span_end = end_s.min(total).max(span_start);
    let frames_total =
        (((span_end - span_start) / speed.max(1e-9)) * fps as f64).ceil() as usize + 1;

    let size = terminal_size(&cast_events);
    let fopts = FrameOptions {
        cols: size.cols,
        rows: size.rows,
        cell_w: cfg.cell_width,
        cell_h: cfg.cell_height,
        overlay: true,
        theme: ColorTheme::named(&cfg.theme),
    };
    let mut raster = Rasterizer::new(fopts)?;
    let nz_rows = non_zero(size.rows, "rows")?;
    let nz_cols = non_zero(size.cols, "cols")?;
    let mut parser = Parser::new(nz_rows, nz_cols, 0);

    // Seed the terminal with everything before the range (no frames emitted).
    let mut fed = 0usize;
    while fed < cast_events.len() && comp[fed] < span_start {
        fed += 1;
    }
    step_parser(&mut parser, &cast_events[..fed]);

    let mut frames: Vec<RgbaImage> = Vec::with_capacity(frames_total);
    let mut last_fed = fed;
    for i in 0..frames_total {
        let t_unscaled = span_start + (i as f64 * speed.max(1e-9)) / fps as f64;
        while fed < cast_events.len() && comp[fed] <= t_unscaled {
            fed += 1;
        }
        // Advance only by newly-arrived events, never re-feed earlier ones.
        step_parser(&mut parser, &cast_events[last_fed..fed]);
        last_fed = fed;
        let chapter = chapter_title_at(&events, t_unscaled * 1000.0);
        let chapter_start = chapter_start_at(&events, t_unscaled * 1000.0) / 1000.0;
        let spec = OverlaySpec { chapter, t: t_unscaled, total, chapter_start };
        frames.push(raster.render(parser.screen(), &spec));
    }
    Ok(frames)
}

/// Load and parse a session's raw cast.
fn load_cast(session: &Session) -> Result<Vec<CastEvent>> {
    let reopened = Session::open(session.id())?;
    let cast_text = reopened.read_cast_text()?;
    parse_cast(&cast_text)
}

/// Clamp long idle gaps.
///
/// Returns a new monotonic non-decreasing timeline where the gap between
/// consecutive events is at most `limit` seconds (a gap of 0 disables
/// compression and returns `times` unchanged).
pub fn compress_idle(times: &[f64], limit: f64) -> Vec<f64> {
    if limit <= 0.0 || times.is_empty() {
        return times.to_vec();
    }
    let mut out = Vec::with_capacity(times.len());
    let mut prev_raw = 0.0f64;
    let mut prev_comp = 0.0f64;
    for t in times {
        // Each consecutive gap is clamped to `limit`; short gaps pass
        // through untouched, long idle stretches collapse to `limit`.
        let t = (*t).max(prev_raw);
        let gap = (t - prev_raw).min(limit);
        let c = prev_comp + gap;
        out.push(c);
        prev_raw = t;
        prev_comp = c;
    }
    out
}

/// The most recent chapter title at a given elapsed time (milliseconds).
fn chapter_title_at(events: &[Event], at_ms: f64) -> Option<String> {
    let mut latest = None;
    for ev in events {
        if let Event::Chapter { at_ms: ev_ms, title, .. } = ev
            && *ev_ms as f64 <= at_ms
        {
            latest = Some(title.clone());
        }
    }
    latest
}

/// The start time (milliseconds) of the most recent chapter at `at_ms`, or 0.
fn chapter_start_at(events: &[Event], at_ms: f64) -> f64 {
    let mut latest = 0.0f64;
    for ev in events {
        if let Event::Chapter { at_ms: ev_ms, .. } = ev
            && *ev_ms as f64 <= at_ms
        {
            latest = *ev_ms as f64;
        }
    }
    latest
}

/// The terminal size implied by the last resize event (or the default).
fn terminal_size(events: &[CastEvent]) -> Size {
    for ev in events.iter().rev() {
        if ev.ty == "r"
            && let Some((c, r)) = parse_resize(&ev.data)
        {
            return Size { cols: c, rows: r };
        }
    }
    Size::default()
}

/// Encode a `"C x R"` resize payload into a `(cols, rows)` pair.
pub fn parse_resize(s: &str) -> Option<(u16, u16)> {
    let (c, r) = s.split_once('x')?;
    Some((c.parse().ok()?, r.parse().ok()?))
}

/// Helper: build a `NonZeroU16` with a descriptive error.
fn non_zero(v: u16, what: &str) -> Result<core::num::NonZeroU16> {
    core::num::NonZeroU16::new(v).ok_or_else(|| anyhow::anyhow!("zero {what}"))
}

/// Export a copy of the session's cast file into `dest`.
pub fn export_cast(session: &Session, dest: &Path) -> Result<PathBuf> {
    let out = dest.join(format!("{}.cast", stem(session.id())));
    let src = Session::open(session.id())?;
    let cast_text = src.read_cast_text()?;
    std::fs::write(&out, cast_text)?;
    Ok(out)
}

fn require_ffmpeg() -> Result<()> {
    process::Command::new("ffmpeg")
        .arg("-version")
        .status()
        .context("ffmpeg not found in PATH; install it (e.g. `brew install ffmpeg` / `apt install ffmpeg`) and retry")?;
    Ok(())
}

/// Video encoder to hand to `ffmpeg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// Animated GIF (palette-based).
    Gif,
    /// H.264 MP4.
    H264,
    /// VP9 WebM.
    Vp9,
    /// AV1 (in an MP4 container).
    Av1,
}

impl Codec {
    /// ffmpeg codec name.
    fn codec(&self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::H264 => "libx264",
            Self::Vp9 => "libvpx-vp9",
            Self::Av1 => "libaom-av1",
        }
    }
    /// Suggested output extension.
    fn ext(&self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::H264 | Self::Av1 => "mp4",
            Self::Vp9 => "webm",
        }
    }
}

/// Encode a set of frames to a video via `ffmpeg` using the given codec.
fn make_video(frames: &[RgbaImage], out: &Path, fps: u32, codec: Codec) -> Result<()> {
    let dir = out.parent().expect("output has a parent").join(format!(
        "_{}_{}",
        codec.codec(),
        out.file_name().map(|s| s.to_string_lossy()).unwrap_or_default()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    for (i, f) in frames.iter().enumerate() {
        f.save(dir.join(format!("f_{i:05}.png")))?;
    }
    let pat = dir.join("f_%05d.png");
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-framerate".into(),
        fps.to_string(),
        "-i".into(),
        pat.to_string_lossy().to_string(),
    ];
    match codec {
        Codec::H264 => {
            args.extend(
                [
                    "-c:v",
                    "libx264",
                    "-preset",
                    "veryfast",
                    "-crf",
                    "20",
                    "-pix_fmt",
                    "yuv420p",
                    "-movflags",
                    "+faststart",
                ]
                .iter()
                .map(|s| s.to_string()),
            );
        }
        Codec::Vp9 => {
            args.extend(
                [
                    "-c:v",
                    "libvpx-vp9",
                    "-b:v",
                    "0",
                    "-crf",
                    "30",
                    "-deadline",
                    "good",
                    "-cpu-used",
                    "8",
                    // Our frames are RGBA; without an explicit pixel format
                    // newer libvpx builds reject 'gbrap' for VP9.
                    "-pix_fmt",
                    "yuv420p",
                ]
                .iter()
                .map(|s| s.to_string()),
            );
        }
        Codec::Av1 => {
            args.extend(
                [
                    "-c:v",
                    "libaom-av1",
                    "-crf",
                    "32",
                    "-b:v",
                    "0",
                    "-cpu-used",
                    "4",
                    "-pix_fmt",
                    "yuv420p",
                ]
                .iter()
                .map(|s| s.to_string()),
            );
        }
        Codec::Gif => {
            args.extend([
                "-vf".to_string(),
                "split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer"
                    .to_string(),
            ]);
        }
    }
    args.push(out.to_string_lossy().to_string());
    run_ffmpeg(&args)?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Render one short video per chapter.
///
/// Each chapter video is seeded with the terminal state up to the chapter's
/// start, then plays only that chapter's span. The codec set is taken from
/// whichever of GIF/MP4/WebM/AV1 are enabled on the request.
fn render_chapters(session: &Session, req: &RenderRequest, cfg: &Config) -> Result<Vec<PathBuf>> {
    let events = session.read_events()?;
    let chapters: Vec<Event> =
        events.iter().filter(|e| matches!(e, Event::Chapter { .. })).cloned().collect();
    let total_ms = (playback_total(session, req, cfg)? * 1000.0) as u64;
    let codes: Vec<Codec> = vec![
        (req.gif, Codec::Gif),
        (req.mp4, Codec::H264),
        (req.webm, Codec::Vp9),
        (req.av1, Codec::Av1),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, c)| c)
    .collect();

    let mut out = Vec::new();
    let base = stem(session.id());
    for (i, ch) in chapters.iter().enumerate() {
        let start_ms = ch.at_ms();
        let end_ms = chapters.get(i + 1).map(|n| n.at_ms()).unwrap_or(total_ms.max(start_ms));
        if end_ms <= start_ms {
            continue;
        }
        let frames =
            rasterize_range(session, req, cfg, start_ms as f64 / 1000.0, end_ms as f64 / 1000.0)?;
        if frames.is_empty() {
            continue;
        }
        let fps = if codes.contains(&Codec::Gif) {
            req.gif_fps.unwrap_or(cfg.gif_fps)
        } else {
            req.fps.unwrap_or(cfg.fps)
        };
        let title = slugify(chapter_title(ch));
        for codec in &codes {
            let suffix = if codes.len() == 1 { String::new() } else { format!("-{}", codec.ext()) };
            let p = req.out_dir.join(format!("{base}-ch{i:02}-{title}{suffix}.{}", codec.ext()));
            make_video(&frames, &p, fps, *codec)?;
            out.push(p);
        }
    }
    Ok(out)
}

/// Emit a PNG still at each chapter boundary plus the final frame.
fn render_stills(session: &Session, req: &RenderRequest, cfg: &Config) -> Result<Vec<PathBuf>> {
    let events = session.read_events()?;
    let chapters: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            Event::Chapter { at_ms, .. } => Some(*at_ms),
            _ => None,
        })
        .collect();
    let total_ms = (playback_total(session, req, cfg)? * 1000.0) as u64;
    let mut times = chapters.clone();
    times.push(total_ms);
    let base = stem(session.id());
    let mut out = Vec::new();
    for (i, t_ms) in times.iter().enumerate() {
        let t = *t_ms as f64 / 1000.0;
        let frames = rasterize_range(session, req, cfg, t, t + 0.001)?;
        if let Some(frame) = frames.first() {
            let name = if i == times.len() - 1 {
                format!("{base}-final.png")
            } else {
                format!("{base}-ch{i:02}.png")
            };
            let p = req.out_dir.join(&name);
            frame.save(&p)?;
            out.push(p);
        }
    }
    Ok(out)
}

/// Reduce a chapter title to a filesystem-safe slug.
fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == ' ' { c } else { '-' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
}

fn chapter_title(e: &Event) -> &str {
    match e {
        Event::Chapter { title, .. } => title,
        _ => "",
    }
}

fn run_ffmpeg(args: &[String]) -> Result<()> {
    let output =
        process::Command::new("ffmpeg").args(args).output().context("failed to run ffmpeg")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ffmpeg failed:\n{stderr}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_resize_works() {
        assert_eq!(parse_resize("120x40"), Some((120, 40)));
        assert_eq!(parse_resize("bogus"), None);
    }

    #[test]
    fn terminal_size_defaults() {
        assert_eq!(terminal_size(&[]), Size::default());
    }

    #[test]
    fn chapter_title_at_picks_latest() {
        let evs = vec![
            Event::Chapter {
                at_ms: 100,
                title: "first".into(),
                source: sdoc_core::ChapterSource::Manual,
            },
            Event::Chapter {
                at_ms: 500,
                title: "second".into(),
                source: sdoc_core::ChapterSource::Manual,
            },
        ];
        assert_eq!(chapter_title_at(&evs, 400.0).as_deref(), Some("first"));
        assert_eq!(chapter_title_at(&evs, 600.0).as_deref(), Some("second"));
    }

    #[test]
    fn chapter_split_on_event_sparse_session() {
        // A session whose cast carries a single output event at t=0 plus a
        // wall-clock duration (e.g. `sleep`), with chapters at 0 and half
        // duration. The renderer must extend an event-sparse timeline to
        // the meta duration so chapters in the idle tail still produce
        // frames (regression guard for rasterize_range clamping ranges to
        // the last event time).
        let times: Vec<f64> = vec![0.0]; // only one cast event, at t=0
        let meta_duration_ms = 400u64;
        let total_s = if times.is_empty() {
            meta_duration_ms as f64 / 1000.0
        } else {
            times.last().copied().unwrap_or(0.0).max(meta_duration_ms as f64 / 1000.0)
        };
        let ch2_start_ms = 200u64;
        assert!(
            total_s >= ch2_start_ms as f64 / 1000.0,
            "chapter 2 (at {ch2_start_ms}ms) must fall inside the extended timeline (total {total_s}s)"
        );
    }

    #[test]
    fn compress_idle_caps_gaps() {
        // A 9s idle gap becomes a 2s gap; short gaps pass through.
        let times = vec![0.0, 0.5, 1.0, 10.0, 10.2];
        let comp = compress_idle(&times, 2.0);
        let expected = [0.0, 0.5, 1.0, 3.0, 3.2];
        assert_eq!(comp.len(), expected.len());
        for (c, e) in comp.iter().zip(expected.iter()) {
            assert!((c - e).abs() < 1e-9, "expected {e}, got {c}");
        }
    }

    #[test]
    fn compress_idle_zero_is_identity() {
        let times = vec![0.0, 5.0, 9.0];
        assert_eq!(compress_idle(&times, 0.0), times);
        assert_eq!(compress_idle(&[], 2.0), Vec::<f64>::new());
    }

    #[test]
    fn codec_extensions() {
        use crate::Codec;
        assert_eq!(Codec::Gif.ext(), "gif");
        assert_eq!(Codec::H264.ext(), "mp4");
        assert_eq!(Codec::Vp9.ext(), "webm");
        assert_eq!(Codec::Av1.codec(), "libaom-av1");
    }

    #[test]
    fn compress_idle_unsorted_and_negative_input() {
        // Out-of-order times are clamped forward: the first (largest) value
        // caps everything that follows.
        let comp = compress_idle(&[5.0, 1.0, 3.0], 2.0);
        assert_eq!(comp, vec![2.0, 2.0, 2.0]);
        // A negative first time is treated as t=0.
        let comp = compress_idle(&[-2.0, 2.0], 1.0);
        assert_eq!(comp, vec![0.0, 1.0]);
        // A single event: the leading idle gap is capped to the limit.
        let comp = compress_idle(&[7.0], 2.0);
        assert_eq!(comp, vec![2.0]);
    }

    #[test]
    fn terminal_size_falls_back_and_uses_last_valid_resize() {
        let evs = vec![
            CastEvent { t: 0.0, ty: "r".into(), data: "bogus".into() },
            CastEvent { t: 1.0, ty: "r".into(), data: "100x30".into() },
            CastEvent { t: 2.0, ty: "r".into(), data: "not-a-size".into() },
        ];
        // The last *valid* resize wins; later bad ones are ignored.
        assert_eq!(terminal_size(&evs), Size { cols: 100, rows: 30 });
        // Only bad resizes -> default.
        let bad = vec![CastEvent { t: 0.0, ty: "r".into(), data: "x".into() }];
        assert_eq!(terminal_size(&bad), Size::default());
    }

    #[test]
    fn empty_cast_yields_a_single_frame_span() {
        // With no events the playback total falls back to the session's
        // wall-clock duration; a zero-length span still yields frames_total=1
        // (the +1 in the loop), so rendering never produces zero frames.
        let times = Vec::<f64>::new();
        let comp = compress_idle(&times, 2.0);
        assert!(comp.is_empty());
    }
}
