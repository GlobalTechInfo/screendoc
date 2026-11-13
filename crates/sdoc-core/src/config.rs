use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// User configuration, loaded from `~/.config/screendoc/config.toml`.
///
/// Any missing key falls back to the default; an unreadable or invalid file
/// is ignored silently.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Frame rate for MP4 rendering.
    pub fps: u32,
    /// Frame rate for GIF rendering.
    pub gif_fps: u32,
    /// Playback speed multiplier for rendered video (1.0 = real time).
    pub speed: f64,
    /// Default terminal width in columns.
    pub cols: u16,
    /// Default terminal height in rows.
    pub rows: u16,
    /// Default web player port.
    pub port: u16,
    /// Raster cell width in pixels.
    pub cell_width: u32,
    /// Raster cell height in pixels.
    pub cell_height: u32,
    /// Maximum seconds of consecutive idle time kept in rendered video;
    /// longer gaps are compressed to this value (0 disables compression).
    pub idle_limit: f64,
    /// Render theme name (`xterm`, `dracula`, `nord`, `solarized`, `tokyo`).
    pub theme: String,
    /// Encrypt new session content files at rest (AES-256-GCM, key file
    /// `~/.screendoc/key`). Off by default.
    pub encrypt: bool,
    /// Automatic retention: delete sessions older than this many days
    /// after each recording finalizes (0 disables).
    pub retention_days: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fps: 20,
            gif_fps: 15,
            speed: 1.0,
            cols: 120,
            rows: 30,
            port: 4999,
            cell_width: 12,
            cell_height: 24,
            idle_limit: 2.0,
            theme: "xterm".into(),
            encrypt: false,
            retention_days: 0,
        }
    }
}

impl Config {
    /// The path of the user config file.
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|c| c.join("screendoc").join("config.toml"))
    }

    /// Load user configuration, falling back to defaults.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str::<Config>(&s).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = Config::default();
        assert!(c.fps >= c.gif_fps);
        assert!(c.cols > 0 && c.rows > 0);
    }

    #[test]
    fn parses_partial_file() {
        let c: Config = toml::from_str("fps = 30").unwrap();
        assert_eq!(c.fps, 30);
        assert_eq!(c.rows, Config::default().rows);
    }

    #[test]
    fn load_returns_defaults_when_no_file() {
        let c = Config::load();
        assert_eq!(c, Config::default());
    }
}
