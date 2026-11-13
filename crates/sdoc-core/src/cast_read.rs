//! Read asciinema cast files (v2 and v3) into a unified event list.

use serde_json::Value;

/// A single cast event in the unified form.
#[derive(Debug, Clone, PartialEq)]
pub struct CastEvent {
    /// Absolute time in seconds from the start of the recording.
    pub t: f64,
    /// Event type: `"o"` (output), `"i"` (input), `"r"` (resize), or `"m"` (marker).
    pub ty: String,
    /// The event payload.
    pub data: String,
}

/// Read a cast file, transparently decompressing gzip (`.cast.gz` or the
/// gzip magic bytes) and lossy-decoding non-UTF-8 content.
pub fn read_cast_file(
    path: &std::path::Path,
) -> std::result::Result<String, crate::err::CoreError> {
    let bytes = std::fs::read(path).map_err(|e| {
        crate::err::CoreError::Other(format!("failed to read cast file {}: {e}", path.display()))
    })?;
    let bytes = if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        use std::io::Read as _;
        let mut dec = flate2::read::GzDecoder::new(bytes.as_slice());
        let mut out = Vec::new();
        dec.read_to_end(&mut out)
            .map_err(|e| crate::err::CoreError::Other(format!("gunzip failed: {e}")))?;
        out
    } else {
        bytes
    };
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Parsed cast: terminal size plus all events, ordered by time.
#[derive(Debug, Clone, Default)]
pub struct ParsedCast {
    /// Terminal width in columns, if known.
    pub cols: Option<u16>,
    /// Terminal height in rows, if known.
    pub rows: Option<u16>,
    /// All events in time order.
    pub events: Vec<CastEvent>,
}

/// Parse a cast file's text into a unified event list.
///
/// Supports asciinema v2 (JSON-array lines, absolute times) and v3 (JSON
/// object lines, times relative to the previous event). Unknown lines are
/// skipped rather than treated as errors, so partially-recognised casts
/// still round-trip.
pub fn parse_cast_text(text: &str) -> Result<ParsedCast, String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = ParsedCast::default();
    let mut abs = 0.0f64; // running absolute time (v3 uses relative deltas)

    for line in lines.iter() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            // Not JSON (or damaged): skip, keep going — partially damaged
            // casts should still round-trip for what is readable.
            Err(_) => continue,
        };

        // A header object (v2: {"version":2,...}; v3: {"version":3,...}).
        if v.get("version").is_some() {
            out.cols = read_size(&v, &["terminal_width", "cols", "width"]);
            out.rows = read_size(&v, &["terminal_height", "rows", "height"]);
            continue;
        }

        // v3 event: {"t": rel, "type": "o", "text": "..."}
        if let (Some(t), Some(ty)) = (v.get("t").and_then(Value::as_f64), v.get("type")) {
            abs += t;
            let data = v
                .get("text")
                .or_else(|| v.get("data"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            out.events.push(CastEvent { t: abs, ty: ty.as_str().unwrap_or("o").to_string(), data });
            continue;
        }

        // v2 event: [time, type, data]
        if let Some(arr) = v.as_array()
            && arr.len() >= 3
        {
            let t = arr[0].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let ty = arr[1].as_str().unwrap_or("o").to_string();
            let data = arr[2].as_str().unwrap_or("").to_string();
            out.events.push(CastEvent { t, ty, data });
        }
    }
    Ok(out)
}

/// Parse only the terminal size from a cast header (v2 or v3).
pub fn parse_cast_size(text: &str) -> (Option<u16>, Option<u16>) {
    for line in text.lines() {
        let v: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("version").is_some() {
            let c = read_size(&v, &["terminal_width", "cols", "width"]);
            let r = read_size(&v, &["terminal_height", "rows", "height"]);
            return (c, r);
        }
    }
    (None, None)
}

/// Read the first present u16-valued key.
fn read_size(v: &Value, keys: &[&str]) -> Option<u16> {
    keys.iter().find_map(|k| v.get(k).and_then(Value::as_u64).and_then(|n| u16::try_from(n).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v2_abs_times() {
        let text = "{\"version\":2,\"terminal_width\":80,\"terminal_height\":24}\n\
                    [\"0.0\",\"o\",\"hi\"]\n[\"1.5\",\"r\",\"100x30\"]\n[\"2.0\",\"o\",\"yo\"]";
        let p = parse_cast_text(text).unwrap();
        assert_eq!(p.cols, Some(80));
        assert_eq!(p.rows, Some(24));
        assert_eq!(p.events.len(), 3);
        assert_eq!(p.events[0].t, 0.0);
        assert_eq!(p.events[1].ty, "r");
        assert_eq!(p.events[2].data, "yo");
    }

    #[test]
    fn parses_v3_relative_times() {
        let text = "{\"version\":3,\"width\":100,\"height\":30}\n\
                   {\"t\":0.0,\"type\":\"o\",\"text\":\"a\"}\n\
                   {\"t\":1.0,\"type\":\"o\",\"text\":\"b\"}\n\
                   {\"t\":2.5,\"type\":\"o\",\"text\":\"c\"}";
        let p = parse_cast_text(text).unwrap();
        assert_eq!(p.cols, Some(100));
        assert_eq!(p.rows, Some(30));
        // v3 times are relative deltas -> accumulate.
        assert_eq!(p.events[0].t, 0.0);
        assert_eq!(p.events[1].t, 1.0);
        assert_eq!(p.events[2].t, 3.5);
    }

    #[test]
    fn empty_is_empty() {
        let p = parse_cast_text("").unwrap();
        assert!(p.events.is_empty());
        assert_eq!(p.cols, None);
    }

    #[test]
    fn skips_unknown_lines() {
        let text = "not-json\n{\"version\":2}\n[\"0.0\",\"o\",\"x\"]";
        let p = parse_cast_text(text).unwrap();
        assert_eq!(p.events.len(), 1);
    }

    #[test]
    fn v3_all_event_types_round_trip() {
        let text = "{\"version\":3,\"width\":120,\"height\":40}\n\
                    {\"t\":0.0,\"type\":\"o\",\"text\":\"out\"}\n\
                    {\"t\":0.1,\"type\":\"i\",\"text\":\"in\"}\n\
                    {\"t\":0.2,\"type\":\"r\",\"text\":\"80x24\"}\n\
                    {\"t\":0.3,\"type\":\"m\",\"text\":\"checkpoint\"}";
        let p = parse_cast_text(text).unwrap();
        let tys: Vec<&str> = p.events.iter().map(|e| e.ty.as_str()).collect();
        assert_eq!(tys, vec!["o", "i", "r", "m"]);
        // Times are relative deltas: 0, 0.1, 0.3, 0.6.
        assert!((p.events[3].t - 0.6).abs() < 1e-9);
        assert_eq!(p.cols, Some(120));
        assert_eq!(p.rows, Some(40));
    }

    #[test]
    fn v2_large_and_unicode_payloads() {
        let big = "héllo wörld 日本語";
        let text = format!(
            "{{\"version\":2,\"terminal_width\":200,\"terminal_height\":50}}\n[\"12345.678\",\"o\",\"{big}\"]"
        );
        let p = parse_cast_text(&text).unwrap();
        assert_eq!(p.events.len(), 1);
        assert!((p.events[0].t - 12345.678).abs() < 1e-9);
        assert_eq!(p.events[0].data, big);
        assert_eq!(p.cols, Some(200));
    }

    #[test]
    fn damaged_lines_are_skipped_not_fatal() {
        let text = "garbage here\n{\"version\":2}\n[broken json\n[\"0.5\",\"o\",\"ok\"]\n";
        let p = parse_cast_text(text).unwrap();
        assert_eq!(p.events.len(), 1);
        assert_eq!(p.events[0].data, "ok");
    }

    #[test]
    fn empty_and_header_only_files() {
        assert!(parse_cast_text("").unwrap().events.is_empty());
        let p = parse_cast_text("{\"version\":2,\"terminal_width\":80,\"terminal_height\":24}")
            .unwrap();
        assert!(p.events.is_empty());
        assert_eq!((p.cols, p.rows), (Some(80), Some(24)));
    }

    #[test]
    fn negative_v3_deltas_do_not_panic() {
        // Malformed but possible input: a negative relative delta.
        let text = "{\"version\":3}\n{\"t\":5.0,\"type\":\"o\",\"text\":\"a\"}\n\
                    {\"t\":-9.0,\"type\":\"o\",\"text\":\"b\"}\n\
                    {\"t\":0.5,\"type\":\"o\",\"text\":\"c\"}";
        let p = parse_cast_text(text).unwrap();
        assert_eq!(p.events.len(), 3);
        // Accumulation: 5.0, -4.0, -3.5. Consumers clamp gaps to >= 0.
        assert_eq!(p.events[0].t, 5.0);
        assert!((p.events[1].t - -4.0).abs() < 1e-9);
        assert!((p.events[2].t - -3.5).abs() < 1e-9);
    }

    #[test]
    fn gzip_cast_is_detected_and_decompressed() {
        use std::io::Write as _;
        let text = "{\"version\":2}\n[\"0.0\",\"o\",\"compressed hi\"]";
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(text.as_bytes()).unwrap();
        let gz_bytes = gz.finish().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rec.cast.gz");
        std::fs::write(&path, &gz_bytes).unwrap();
        let read = read_cast_file(&path).unwrap();
        let parsed = parse_cast_text(&read).unwrap();
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].data, "compressed hi");

        // Plain text is untouched by the same function.
        let plain = dir.path().join("plain.cast");
        std::fs::write(&plain, text).unwrap();
        assert_eq!(read_cast_file(&plain).unwrap(), text);

        // Truncated gzip is an error, not a panic.
        let bad = dir.path().join("truncated.gz");
        std::fs::write(&bad, &gz_bytes[..gz_bytes.len() / 2]).unwrap();
        assert!(read_cast_file(&bad).is_err());
    }
}
