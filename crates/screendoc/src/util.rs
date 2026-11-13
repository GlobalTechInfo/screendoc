use std::path::PathBuf;

use anyhow::{Context, Result};
use sdoc_core::Size;

/// Locate an executable in `PATH`, accounting for the platform's extension
/// resolution (on Windows a bare `ffmpeg` resolves to `ffmpeg.exe`).
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        // On Windows executables carry an extension; honor PATHEXT (default
        // `.EXE;.CMD` when unset) in order. On other platforms the bare
        // name is the candidate.
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        for ext in pathext() {
            let cand = dir.join(format!("{name}{ext}"));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// The `PATHEXT` extension list for Windows executable resolution.
#[cfg(windows)]
fn pathext() -> Vec<String> {
    std::env::var("PATHEXT")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|v| v.split(';').filter(|e| !e.is_empty()).map(|e| e.to_uppercase()).collect())
        .unwrap_or_else(|| vec![".EXE".to_string(), ".CMD".to_string()])
}

/// Parse a `COLSXROWS` size string into a [`Size`].
pub fn parse_size(s: &str) -> Result<Size> {
    let (c, r) = s.split_once('x').with_context(|| format!("expected COLSXROWS, got `{s}`"))?;
    let cols: u16 = c.trim().parse().with_context(|| format!("bad column count in `{s}`"))?;
    let rows: u16 = r.trim().parse().with_context(|| format!("bad row count in `{s}`"))?;
    Ok(Size { cols, rows })
}

/// Format a duration in milliseconds as a compact human string.
pub fn fmt_duration(ms: u64) -> String {
    let s = ms / 1000;
    let m = s / 60;
    if m > 0 { format!("{m}m {}s", s % 60) } else { format!("{s}s") }
}

/// Parse a duration like `30d`, `7w`, `24h`, `90m`, or `60s` into
/// milliseconds.
pub fn parse_duration(s: &str) -> Result<u64> {
    let s = s.trim();
    let num: u64 = s
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .with_context(|| format!("bad duration `{s}`"))?;
    let unit = s.trim_start_matches(|c: char| c.is_ascii_digit()).to_ascii_lowercase();
    let mult = match unit.as_str() {
        "" | "s" => 1_000u64,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 7 * 86_400_000,
        _ => anyhow::bail!("unknown duration unit `{unit}` in `{s}`"),
    };
    num.checked_mul(mult).with_context(|| format!("duration `{s}` overflows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes() {
        let s = parse_size("120x30").unwrap();
        assert_eq!((s.cols, s.rows), (120, 30));
        assert!(parse_size("nope").is_err());
        assert!(parse_size("xx30").is_err());
    }

    #[test]
    fn formats_durations() {
        assert_eq!(fmt_duration(3_000), "3s");
        assert_eq!(fmt_duration(150_000), "2m 30s");
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("30").unwrap(), 30_000);
        assert_eq!(parse_duration("30s").unwrap(), 30_000);
        assert_eq!(parse_duration("90m").unwrap(), 90 * 60_000);
        assert_eq!(parse_duration("2h").unwrap(), 7_200_000);
        assert_eq!(parse_duration("30d").unwrap(), 30 * 86_400_000);
        assert_eq!(parse_duration("1w").unwrap(), 7 * 86_400_000);
        assert_eq!(parse_duration("0d").unwrap(), 0);
        assert!(parse_duration("nope").is_err());
        assert!(parse_duration("1d2h").is_err(), "compound durations are unsupported");
        // Overflow must error, not panic.
        assert!(parse_duration("99999999999999d").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn finds_ffmpeg_when_present() {
        // On a typical dev box ffmpeg is either present or not; we only assert
        // the helper does not panic.
        let _ = find_in_path("ffmpeg");
        let _ = find_in_path("definitely-not-a-real-binary-xyz");
    }
}
