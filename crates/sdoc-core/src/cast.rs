use std::fs::File;
use std::io::{BufWriter, Write};

use crate::event::Size;

/// Writer for asciinema 2.x cast files.
///
/// The file consists of a JSON header object followed by one JSON array per
/// event, e.g. `["12.345", "o", "data"]`.
pub struct CastWriter {
    w: BufWriter<Box<dyn Write>>,
}

impl std::fmt::Debug for CastWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CastWriter").finish_non_exhaustive()
    }
}

impl CastWriter {
    /// Open (create) a new cast file for a terminal of the given size.
    pub fn new(path: &std::path::Path, size: Size) -> std::io::Result<Self> {
        let f = File::create(path)?;
        Self::new_w(Box::new(f), size)
    }

    /// Create a cast writer over an arbitrary `Write` sink (used for
    /// encrypted sessions, where the sink is an encrypted record stream).
    pub fn new_w(w: Box<dyn Write>, size: Size) -> std::io::Result<Self> {
        let mut w = BufWriter::new(w);
        let header = serde_json::json!({
            "version": 2,
            "writer": format!("screendoc/{}", env!("CARGO_PKG_VERSION")),
            "terminal_width": size.cols,
            "terminal_height": size.rows,
        });
        writeln!(w, "{header}")?;
        Ok(Self { w })
    }

    /// Append an output event at the given time.
    pub fn output(&mut self, at_ms: u64, data: &str) -> std::io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let ev = serde_json::json!([fmt_time(at_ms), "o", data]);
        self.write_event(&ev)
    }

    /// Append a resize event at the given time.
    pub fn resize(&mut self, at_ms: u64, size: Size) -> std::io::Result<()> {
        let ev = serde_json::json!([fmt_time(at_ms), "r", format!("{}x{}", size.cols, size.rows)]);
        self.write_event(&ev)
    }

    /// Flush buffered events to disk.
    pub fn flush(&mut self) -> std::io::Result<()> {
        self.w.flush()
    }

    /// Close the writer, flushing remaining bytes.
    pub fn finish(mut self) -> std::io::Result<()> {
        self.flush()
    }

    fn write_event(&mut self, ev: &serde_json::Value) -> std::io::Result<()> {
        writeln!(self.w, "{ev}")?;
        Ok(())
    }
}

/// Format cast-file timestamps: whole seconds plus three fractional digits.
fn fmt_time(ms: u64) -> String {
    format!("{s}.{m:03}", s = ms / 1000, m = ms % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::err::Result;

    #[test]
    fn writes_header_and_events() -> Result<()> {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("sdoc-cast-{}.cast", std::process::id()));
        let mut w = CastWriter::new(&path, Size { cols: 80, rows: 24 })?;
        w.output(0, "hello")?;
        w.resize(1_000, Size { cols: 100, rows: 30 })?;
        w.finish()?;

        let text = std::fs::read_to_string(&path)?;
        std::fs::remove_file(&path)?;
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].contains("\"version\":2"));
        assert_eq!(lines[1], r#"["0.000","o","hello"]"#);
        assert!(lines[2].contains(r#""1.000""#));
        Ok(())
    }

    #[test]
    fn empty_output_is_skipped() -> Result<()> {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("sdoc-cast-{}.cast", std::process::id() + 1));
        let mut w = CastWriter::new(&path, Size { cols: 80, rows: 24 })?;
        w.output(0, "")?;
        w.finish()?;

        let text = std::fs::read_to_string(&path)?;
        std::fs::remove_file(&path)?;
        assert_eq!(text.lines().count(), 1);
        Ok(())
    }
}
