//! screendoc renderer: terminal frame rasterization, ffmpeg GIF/MP4 pipeline,
//! and a local web player.
//!
//! The renderer turns a session's asciinema byte stream plus its chapter
//! events into derived artifacts: animated GIF/MP4 (via `ffmpeg`), an
//! exported cast copy, and a browser-based live player.

/// Frame rasterization, cast parsing, and color palettes.
pub mod frame;
/// The local web player (axum server + embedded xterm.js).
pub mod serve;
/// GIF/MP4 video encoding and cast export.
pub mod video;
/// Standalone (self-contained) web player export.
pub mod web;

pub use frame::{ColorTheme, FrameOptions, OverlaySpec, Rasterizer};
pub use serve::{chapters, player_url};
pub use video::{Codec, RenderRequest, Rendered, compress_idle};
pub use web::render_web;
