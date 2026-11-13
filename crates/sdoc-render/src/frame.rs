use std::collections::HashMap;

use anyhow::Result;
use image::RgbaImage;
use rusttype::{Font, Scale, point};
use vt100::{Color, Parser, Screen};

/// The embedded monospace font used to rasterize terminal cells.
///
/// Embedded as bytes so the renderer has no runtime filesystem or network
/// dependency.
pub const FONT: &[u8] = include_bytes!("../assets/DejaVuSansMono.ttf");

/// Terminal color options for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorTheme {
    /// Background color (R, G, B).
    pub bg: [u8; 3],
    /// Default foreground color (R, G, B).
    pub fg: [u8; 3],
}

impl Default for ColorTheme {
    fn default() -> Self {
        Self { bg: [13, 17, 23], fg: [229, 228, 225] }
    }
}

impl ColorTheme {
    /// A theme by name. Unknown names fall back to the default (xterm-like).
    pub fn named(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "dracula" => Self { bg: [38, 35, 51], fg: [245, 245, 245] },
            "nord" => Self { bg: [40, 42, 54], fg: [214, 212, 209] },
            "tokyo" => Self { bg: [19, 23, 35], fg: [199, 204, 226] },
            "solarized" => Self { bg: [11, 42, 58], fg: [238, 210, 188] },
            // "xterm" (default) and anything unknown.
            _ => Self::default(),
        }
    }
}

/// Information needed to draw the bottom overlay bar for one frame.
#[derive(Debug, Clone, Default)]
pub struct OverlaySpec {
    /// Chapter title, or `None` for no title.
    pub chapter: Option<String>,
    /// Current playback time in seconds.
    pub t: f64,
    /// Total playback time in seconds.
    pub total: f64,
    /// When the current chapter started (for the fade-in), in seconds.
    pub chapter_start: f64,
}

impl OverlaySpec {
    /// A no-chapter spec showing only the time bar.
    pub fn time_only(t: f64, total: f64) -> Self {
        Self { chapter: None, t, total, chapter_start: 0.0 }
    }
}

/// Options controlling frame rasterization.
pub struct FrameOptions {
    /// Terminal width in columns.
    pub cols: u16,
    /// Terminal height in rows.
    pub rows: u16,
    /// Pixel width of one cell.
    pub cell_w: u32,
    /// Pixel height of one cell.
    pub cell_h: u32,
    /// Whether to draw a bottom overlay bar with the current chapter.
    pub overlay: bool,
    /// Colors used for the terminal.
    pub theme: ColorTheme,
}

impl Default for FrameOptions {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            cell_w: 12,
            cell_h: 24,
            overlay: true,
            theme: ColorTheme::default(),
        }
    }
}

/// Height (in pixels) of the chapter overlay bar.
const OVERLAY_H: u32 = 22;

/// One cell of terminal text to rasterize (the glyph plus its styling).
struct Cell {
    /// The screen row.
    row: u16,
    /// The screen column.
    col: u32,
    /// The glyph.
    ch: char,
    /// Foreground color.
    fg: [u8; 3],
    /// Background color.
    bg: [u8; 3],
    /// Whether the glyph occupies two columns.
    wide: bool,
}

/// A terminal frame rasterizer backed by a cached glyph atlas.
pub struct Rasterizer {
    font: Font<'static>,
    scale: Scale,
    ascent: f32,
    opts: FrameOptions,
    /// Cache of glyph coverage per character.
    glyphs: HashMap<char, GlyphRaster>,
}

#[derive(Clone)]
struct GlyphRaster {
    /// Pixel x-offset of the coverage rect relative to the cell origin.
    x: i32,
    /// Pixel y-offset of the coverage rect relative to the cell origin.
    y: i32,
    /// Coverage width in pixels.
    w: u32,
    /// Per-pixel coverage in `0..=255` (row-major, `w` wide).
    mask: Vec<u8>,
}

impl Rasterizer {
    /// Create a rasterizer for the given options.
    pub fn new(opts: FrameOptions) -> Result<Self> {
        let font =
            Font::try_from_bytes(FONT).ok_or_else(|| anyhow::anyhow!("invalid embedded font"))?;
        let px = opts.cell_h as f32 * 0.8;
        let scale = Scale::uniform(px);
        let ascent = font.v_metrics(scale).ascent;
        Ok(Self { font, scale, ascent, opts, glyphs: HashMap::new() })
    }

    /// Get (and lazily rasterize) the coverage mask for a character.
    fn glyph(&mut self, ch: char) -> Option<&GlyphRaster> {
        if self.glyphs.contains_key(&ch) {
            return self.glyphs.get(&ch);
        }
        let g = self.rasterize_glyph(ch)?;
        self.glyphs.insert(ch, g);
        self.glyphs.get(&ch)
    }

    /// Rasterize a single glyph into a coverage mask.
    fn rasterize_glyph(&self, ch: char) -> Option<GlyphRaster> {
        let positioned = self.font.glyph(ch).scaled(self.scale).positioned(point(0.0, self.ascent));
        let bb = positioned.pixel_bounding_box()?;
        let w = (bb.max.x - bb.min.x).max(0) as u32;
        let h = (bb.max.y - bb.min.y).max(0) as u32;
        if w == 0 || h == 0 {
            return None;
        }
        let mut mask = vec![0u8; (w * h) as usize];
        positioned.draw(|gx, gy, cov| {
            if gx < w && gy < h {
                let idx = (gy as usize) * (w as usize) + gx as usize;
                let v = (cov * 255.0).round().clamp(0.0, 255.0) as u8;
                if v > mask[idx] {
                    mask[idx] = v;
                }
            }
        });
        Some(GlyphRaster { x: bb.min.x, y: bb.min.y, w, mask })
    }

    /// Rasterize the current screen into a fresh frame image.
    ///
    /// The overlay bar shows the current chapter and a time readout when
    /// overlays are enabled.
    pub fn render(&mut self, screen: &Screen, overlay: &OverlaySpec) -> RgbaImage {
        let w = (self.opts.cols as u32) * self.opts.cell_w;
        let grid_h = (self.opts.rows as u32) * self.opts.cell_h;
        let h = grid_h + if self.opts.overlay { OVERLAY_H } else { 0 };
        let mut img = RgbaImage::from_pixel(w, h, rgba(self.opts.theme.bg));

        for row in 0..self.opts.rows {
            let mut col = 0u32;
            while col < self.opts.cols as u32 {
                let Some(cell) = screen.cell(row, col as u16) else {
                    col += 1;
                    continue;
                };
                let wide = cell.is_wide();
                let span = if wide { 2 } else { 1 };
                let x0 = col * self.opts.cell_w;
                let y0 = row as u32 * self.opts.cell_h;
                let x1 = x0 + self.opts.cell_w * span;
                let y1 = y0 + self.opts.cell_h;
                let bg = self.color_of(cell.bgcolor(), true);
                self.fill_bg(&mut img, x0, y0, x1, y1, bg);

                if let Some(c) = cell.contents().chars().next()
                    && c != ' '
                {
                    let fg = self.color_of(cell.fgcolor(), false);
                    self.draw_char(&mut img, &Cell { row, col, ch: c, fg, bg, wide });
                }
                col += span;
            }
        }

        if self.opts.overlay {
            self.draw_overlay(&mut img, overlay);
        }
        img
    }

    /// Draw the chapter title and time readout inside the overlay bar, with a
    /// short fade-in whenever a new chapter begins.
    fn draw_overlay(&mut self, img: &mut RgbaImage, spec: &OverlaySpec) {
        let h = img.height();
        let y0 = h - OVERLAY_H;
        let bar = rgba([20, 24, 31]);
        for y in y0..h {
            for x in 0..img.width() {
                *img.get_pixel_mut(x, y) = bar;
            }
        }
        // Fade the whole bar content in over 0.5s after a chapter change.
        let since = spec.t - spec.chapter_start;
        let fade = if since < 0.0 {
            0.0f32
        } else if since >= 0.5 {
            1.0
        } else {
            (since / 0.5) as f32
        };

        if let Some(title) = &spec.chapter {
            self.draw_text_faded(img, 8, y0 + OVERLAY_H / 2, title, [229, 228, 225], fade);
        }
        let time = format!("{}/{}", fmt_mmss(spec.t), fmt_mmss(spec.total));
        let w = img.width();
        // Right-aligned time readout.
        let per_char = (self.opts.cell_w / 2).max(4);
        let x = (w.saturating_sub(time.len() as u32 * per_char + 8)).max(4);
        self.draw_text_faded(img, x, y0 + OVERLAY_H / 2, &time, [139, 148, 158], fade);
    }

    /// Like `draw_text`, but scaled to an overall alpha so callers can fade.
    fn draw_text_faded(
        &mut self,
        img: &mut RgbaImage,
        x0: u32,
        y0: u32,
        text: &str,
        color: [u8; 3],
        alpha: f32,
    ) {
        if alpha <= 0.0 {
            return;
        }
        let per_char = (self.opts.cell_w / 2).max(4);
        let budget = ((img.width() - x0) / per_char) as usize;
        let truncated: String = text.chars().take(budget).collect();
        let truncated =
            if text.chars().count() > budget { format!("{truncated}…") } else { truncated };
        let mut x = x0 as i32;
        for ch in truncated.chars() {
            if let Some(g) = self.glyph(ch) {
                for (iy, line) in g.mask.chunks_exact(g.w as usize).enumerate() {
                    let py = y0 as i32 + g.y + iy as i32;
                    if py < 0 || py >= img.height() as i32 {
                        continue;
                    }
                    for (ix, &cov) in line.iter().enumerate() {
                        let px = x + g.x + ix as i32;
                        if px < 0 || px >= img.width() as i32 || cov == 0 {
                            continue;
                        }
                        let a = cov as f32 / 255.0 * alpha;
                        let p = img.get_pixel_mut(px as u32, py as u32);
                        *p = image::Rgba([
                            blend(p[0], color[0], a),
                            blend(p[1], color[1], a),
                            blend(p[2], color[2], a),
                            255,
                        ]);
                    }
                }
            }
            x += per_char as i32;
        }
    }

    /// Paint one glyph into its cell.
    fn draw_char(&mut self, img: &mut RgbaImage, cell: &Cell) {
        let row = cell.row;
        let col = cell.col;
        let ch = cell.ch;
        let fg = cell.fg;
        let bg = cell.bg;
        let wide = cell.wide;
        let cw = self.opts.cell_w;
        let chh = self.opts.cell_h;
        let x0 = col * cw;
        let y0 = row as u32 * chh;
        let span = if wide { 2 } else { 1 };
        let cx1 = x0 + cw * span;
        let cy1 = y0 + chh;
        let g = match self.glyph(ch) {
            Some(g) => g.clone(),
            None => return,
        };

        for (iy, line) in g.mask.chunks_exact(g.w as usize).enumerate() {
            let py = y0 as i32 + g.y + iy as i32;
            if py < 0 || py >= cy1 as i32 {
                continue;
            }
            for (ix, &cov) in line.iter().enumerate() {
                let px = x0 as i32 + g.x + ix as i32;
                if px < x0 as i32 || px >= cx1 as i32 {
                    continue;
                }
                if cov == 0 {
                    continue;
                }
                let a = cov as f32 / 255.0;
                let blended =
                    [blend(bg[0], fg[0], a), blend(bg[1], fg[1], a), blend(bg[2], fg[2], a)];
                let p = img.get_pixel_mut(px as u32, py as u32);
                *p = image::Rgba([blended[0], blended[1], blended[2], 255]);
            }
        }
    }

    fn fill_bg(&self, img: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, bg: [u8; 3]) {
        let p = rgba(bg);
        for y in y0..y1.min(img.height()) {
            for x in x0..x1.min(img.width()) {
                *img.get_pixel_mut(x, y) = p;
            }
        }
    }

    fn color_of(&self, c: Color, is_bg: bool) -> [u8; 3] {
        match c {
            Color::Default => {
                if is_bg {
                    self.opts.theme.bg
                } else {
                    self.opts.theme.fg
                }
            }
            Color::Idx(i) => palette(i),
            Color::Rgb(r, g, b) => [r, g, b],
        }
    }
}

/// Blend two channels by an alpha factor in `[0,1]`.
fn blend(base: u8, top: u8, a: f32) -> u8 {
    let v = base as f32 * (1.0 - a) + top as f32 * a;
    v.round().clamp(0.0, 255.0) as u8
}

/// Build an opaque RGBA pixel from an RGB triple.
fn rgba(c: [u8; 3]) -> image::Rgba<u8> {
    image::Rgba([c[0], c[1], c[2], 255])
}

/// Format seconds as `m:ss` for the overlay time readout.
fn fmt_mmss(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Map a 256-color index to an RGB color (standard xterm palette).
fn palette(i: u8) -> [u8; 3] {
    match i {
        0..=7 => base8(i),
        8..=15 => base8(i - 8).map(|c| c.saturating_add(80)),
        16..=231 => {
            let i = i - 16;
            let r = i / 36;
            let g = (i / 6) % 6;
            let b = i % 6;
            [level(r), level(g), level(b)]
        }
        232..=255 => {
            let g = (i - 232) * 10 + 8;
            [g, g, g]
        }
    }
}

fn base8(i: u8) -> [u8; 3] {
    const BASE: [[u8; 3]; 8] = [
        [0, 0, 0],
        [205, 89, 89],
        [103, 197, 103],
        [255, 197, 48],
        [103, 103, 255],
        [205, 103, 205],
        [103, 216, 216],
        [229, 228, 225],
    ];
    BASE[i as usize % 8]
}

fn level(v: u8) -> u8 {
    if v == 0 { 0 } else { 135 + (v - 1) * 40 }
}

/// A single parsed cast event.
#[derive(Debug, Clone)]
pub struct CastEvent {
    /// Time in seconds since session start.
    pub t: f64,
    /// Event type (`"o"` output, `"r"` resize, `"i"` input).
    pub ty: String,
    /// The payload.
    pub data: String,
}

/// Parse a cast file into its event list (skipping the header line).
pub fn parse_cast(text: &str) -> Result<Vec<CastEvent>> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if i == 0 || line.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line)?;
        let arr = v.as_array().ok_or_else(|| anyhow::anyhow!("bad cast line {i}"))?;
        if arr.len() < 3 {
            continue;
        }
        let t = arr[0].as_f64().unwrap_or(0.0);
        let ty = arr[1].as_str().unwrap_or("").to_string();
        let data = arr[2].as_str().unwrap_or("").to_string();
        out.push(CastEvent { t, ty, data });
    }
    Ok(out)
}

/// Feed a run of output/resize events into a parser up to (and including)
/// the event at index `last`.
/// Feed a batch of newly-arrived cast events into the terminal parser.
///
/// `new_events` must be the events that have not yet been processed;
/// feeding the same event twice would re-print its output.
pub fn step_parser(parser: &mut Parser, new_events: &[CastEvent]) {
    for ev in new_events {
        match ev.ty.as_str() {
            "o" => parser.process(ev.data.as_bytes()),
            "r" => {
                if let Some((c, r)) = parse_resize(&ev.data) {
                    use core::num::NonZeroU16;
                    if let (Some(cols), Some(rows)) = (NonZeroU16::new(c), NonZeroU16::new(r)) {
                        parser.set_size(rows, cols);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Parse a `"100x30"` resize payload into `(cols, rows)`.
fn parse_resize(s: &str) -> Option<(u16, u16)> {
    let (c, r) = s.split_once('x')?;
    Some((c.parse().ok()?, r.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cast_events() {
        let text = r#"{"version":2}"#.to_string()
            + "\n[\"0.000\",\"o\",\"hi\"]\n[\"1.000\",\"r\",\"80x24\"]\n";
        let evs = parse_cast(&text).unwrap();
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].ty, "o");
        assert_eq!(evs[1].ty, "r");
    }

    #[test]
    fn palette_base() {
        assert_eq!(palette(0), [0, 0, 0]);
        // The xterm gray ramp peaks at 238 (i = 255).
        assert_eq!(palette(255), [238, 238, 238]);
    }

    #[test]
    fn theme_presets_differ() {
        let xterm = ColorTheme::named("xterm");
        let dracula = ColorTheme::named("dracula");
        let nord = ColorTheme::named("nord");
        let unknown = ColorTheme::named("does-not-exist");
        assert_ne!(dracula.bg, xterm.bg);
        assert_ne!(nord.bg, xterm.bg);
        // Unknown names fall back to the default.
        assert_eq!(unknown, xterm);
    }

    #[test]
    fn rasterizer_renders_a_frame() {
        let opts = FrameOptions {
            cols: 20,
            rows: 5,
            cell_w: 10,
            cell_h: 20,
            overlay: false,
            theme: ColorTheme::default(),
        };
        let mut r = Rasterizer::new(opts).unwrap();
        let mut parser = Parser::new(
            core::num::NonZeroU16::new(5).unwrap(),
            core::num::NonZeroU16::new(20).unwrap(),
            0,
        );
        parser.process(b"hello");
        let img = r.render(parser.screen(), &OverlaySpec::time_only(0.0, 1.0));
        assert_eq!(img.width(), 200);
        assert_eq!(img.height(), 100);
        // Some pixels should be non-background (the "hello" glyphs).
        let non_bg = img.pixels().filter(|p| p.0 != [13, 17, 23, 255]).count();
        assert!(non_bg > 0);
    }

    #[test]
    fn wide_cell_step_parser_resizes() {
        let evs = parse_cast("x\n[\"0\",\"r\",\"120x40\"]").unwrap();
        let mut p = Parser::new(
            core::num::NonZeroU16::new(24).unwrap(),
            core::num::NonZeroU16::new(80).unwrap(),
            0,
        );
        step_parser(&mut p, &evs);
        let (rows, cols) = p.screen().size();
        assert_eq!((rows.get(), cols.get()), (40, 120));
    }

    #[test]
    fn incremental_feeding_does_not_duplicate() {
        // Regression: feeding the same output event across two frames must
        // print it once, not re-print it. This mirrors the per-frame loop in
        // `video::rasterize_session` (only newly-arrived events are fed).
        let evs =
            parse_cast("{\"version\":2}\n[\"0.0\",\"o\",\"abc\\n\"]\n[\"1.0\",\"o\",\"def\\n\"]")
                .unwrap();
        let mut p = Parser::new(
            core::num::NonZeroU16::new(24).unwrap(),
            core::num::NonZeroU16::new(80).unwrap(),
            0,
        );
        step_parser(&mut p, &evs[0..1]); // frame advances past the 1st event
        step_parser(&mut p, &evs[1..2]); // frame advances past the 2nd
        let text: String = p
            .screen()
            .rows(0, 80)
            .collect::<Vec<_>>()
            .join("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert_eq!(text, "abcdef");
    }
}
