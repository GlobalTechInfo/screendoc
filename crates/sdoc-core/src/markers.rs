use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::event::{ChapterSource, Event};

/// The OSC channel screendoc uses for inline markers (`ESC ] 90 ; payload BEL`).
pub const OSC_CHANNEL: &str = "90";

/// Maximum accepted payload length (bytes) for a single inline marker.
const MAX_MARKER_PAYLOAD: usize = 8192;

/// An inline or file-based marker emitted by shell hooks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum Marker {
    /// A complete command record: sequence number, exit status, and command text.
    Cmd {
        /// 1-based command sequence number.
        seq: u64,
        /// Exit status, or `-1` when unknown.
        exit_code: i32,
        /// The command text as entered.
        command: String,
    },
    /// A command start record (shells without a prompt-time text, e.g. zsh).
    CmdStart {
        /// 1-based command sequence number.
        seq: u64,
        /// The command text as entered.
        command: String,
    },
    /// A command end record carrying the exit status.
    CmdEnd {
        /// 1-based command sequence number.
        seq: u64,
        /// Exit status of the command.
        exit_code: i32,
    },
    /// A manual chapter marker.
    Chapter {
        /// The chapter title.
        title: String,
    },
}

/// A marker record with a wall-clock timestamp, as written to `markers.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerRecord {
    /// Wall-clock timestamp, Unix milliseconds.
    pub ts_unix_ms: u64,
    /// The marker itself.
    pub marker: Marker,
}

impl Marker {
    /// Convert this marker into session events at the given timestamp.
    ///
    /// A complete command record produces both a [`Event::Command`] record and
    /// an automatic chapter, so the chapter list mirrors the commands.
    pub fn to_events(&self, at_ms: u64) -> Vec<Event> {
        match self {
            Self::Cmd { seq, exit_code, command } => vec![
                Event::Command {
                    at_ms,
                    seq: *seq,
                    exit_code: *exit_code,
                    command: command.clone(),
                },
                Event::Chapter { at_ms, title: shorten(command), source: ChapterSource::Auto },
            ],
            Self::Chapter { title } => {
                vec![Event::Chapter { at_ms, title: title.clone(), source: ChapterSource::Manual }]
            }
            Self::CmdStart { .. } | Self::CmdEnd { .. } => Vec::new(),
        }
    }
}

/// Convert a wall-clock ordered marker record stream into session events.
///
/// `t0_unix_ms` is the session start wall-clock time used to turn Unix
/// milliseconds into elapsed milliseconds. Split start/end records are
/// paired by sequence number; a start with no matching end is flushed with
/// an unknown exit status.
pub fn merge_markers(records: &[MarkerRecord], t0_unix_ms: u64) -> Vec<Event> {
    let mut events = Vec::new();
    let mut pending: std::collections::HashMap<u64, (u64, String)> =
        std::collections::HashMap::new();
    for rec in records {
        let at_ms = rec.ts_unix_ms.saturating_sub(t0_unix_ms);
        match &rec.marker {
            Marker::Cmd { .. } | Marker::Chapter { .. } => {
                events.extend(rec.marker.to_events(at_ms));
            }
            Marker::CmdStart { seq, command } => {
                pending.insert(*seq, (at_ms, command.clone()));
            }
            Marker::CmdEnd { seq, exit_code } => {
                if let Some((start_ms, command)) = pending.remove(seq) {
                    let title = shorten(&command);
                    if !command.is_empty() {
                        events.push(Event::Chapter {
                            at_ms: start_ms,
                            title,
                            source: ChapterSource::Auto,
                        });
                    }
                    events.push(Event::Command {
                        at_ms,
                        seq: *seq,
                        exit_code: *exit_code,
                        command,
                    });
                } else {
                    events.push(Event::Command {
                        at_ms,
                        seq: *seq,
                        exit_code: *exit_code,
                        command: String::new(),
                    });
                }
            }
        }
    }
    for (seq, (start_ms, command)) in pending.drain() {
        let title = shorten(&command);
        if !command.is_empty() {
            events.push(Event::Chapter { at_ms: start_ms, title, source: ChapterSource::Auto });
        }
        events.push(Event::Command { at_ms: start_ms, seq, exit_code: -1, command });
    }
    events
}

/// Encode an inline command-boundary marker for printing into a pty stream.
pub fn encode_cmd_marker(seq: u64, exit_code: i32, command: &str) -> String {
    let flat = command.replace('\n', " ");
    let mut s = String::from("\x1b]");
    let _ = write!(s, "{OSC_CHANNEL};cmd;{seq};{exit_code};{flat}");
    s.push('\x07');
    s
}

/// Encode an inline manual chapter marker for printing into a pty stream.
pub fn encode_chapter_marker(title: &str) -> String {
    let flat = title.replace('\n', " ");
    let mut s = String::from("\x1b]");
    let _ = write!(s, "{OSC_CHANNEL};ch;{flat}");
    s.push('\x07');
    s
}

/// Truncate a string to at most `max_chars` characters, appending an ellipsis.
pub fn shorten(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 48 {
        s.to_string()
    } else {
        let mut out: String = chars.iter().take(45).collect();
        out.push_str("...");
        out
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Plain terminal bytes.
    #[default]
    Normal,
    /// Saw an `ESC`.
    Esc,
    /// Saw `ESC ]`.
    EscBracket,
    /// Saw `ESC ] 9`.
    EscNine,
    /// Saw `ESC ] 9 0`.
    EscZero,
    /// Inside a marker payload, terminated by `BEL` or `ESC \`.
    InMarker,
    /// Inside a marker payload and saw an `ESC` (possible `ST` terminator).
    InMarkerEsc,
}

/// A stateful demultiplexer that splits a raw terminal byte stream into
/// clean bytes and parsed markers.
///
/// Clean bytes are safe to forward to a terminal emulator or cast file;
/// markers are stripped before forwarding so users never see them.
#[derive(Debug, Default)]
pub struct MarkerReader {
    state: State,
    prefix: Vec<u8>,
    payload: Vec<u8>,
}

impl MarkerReader {
    /// Create an empty demultiplexer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of bytes.
    ///
    /// Returns the cleaned bytes (markers removed) and any complete markers
    /// parsed from this chunk.
    pub fn feed(&mut self, buf: &[u8]) -> (Vec<u8>, Vec<Marker>) {
        let mut out: Vec<u8> = Vec::new();
        let mut markers: Vec<Marker> = Vec::new();
        for &b in buf {
            match self.state {
                State::Normal => match b {
                    b'\x1b' => {
                        self.prefix.clear();
                        self.prefix.push(b'\x1b');
                        self.state = State::Esc;
                    }
                    _ => out.push(b),
                },
                State::Esc => match b {
                    b']' => {
                        self.prefix.push(b']');
                        self.state = State::EscBracket;
                    }
                    _ => {
                        out.append(&mut self.prefix);
                        out.push(b);
                        self.state = State::Normal;
                    }
                },
                State::EscBracket => match b {
                    b'9' => {
                        self.prefix.push(b);
                        self.state = State::EscNine;
                    }
                    _ => {
                        out.append(&mut self.prefix);
                        out.push(b);
                        self.state = State::Normal;
                    }
                },
                State::EscNine => match b {
                    b'0' => {
                        self.prefix.push(b);
                        self.state = State::EscZero;
                    }
                    _ => {
                        out.append(&mut self.prefix);
                        out.push(b);
                        self.state = State::Normal;
                    }
                },
                State::EscZero => match b {
                    b';' => {
                        self.prefix.push(b);
                        self.payload.clear();
                        self.state = State::InMarker;
                    }
                    _ => {
                        out.append(&mut self.prefix);
                        out.push(b);
                        self.state = State::Normal;
                    }
                },
                State::InMarker => match b {
                    b'\x07' => {
                        self.finish_marker(&mut markers);
                        self.state = State::Normal;
                    }
                    b'\x1b' => {
                        self.state = State::InMarkerEsc;
                    }
                    _ => {
                        if self.payload.len() < MAX_MARKER_PAYLOAD {
                            self.payload.push(b);
                        } else {
                            self.abort_marker(&mut out, b);
                            self.state = State::Normal;
                        }
                    }
                },
                State::InMarkerEsc => match b {
                    b'\\' => {
                        self.finish_marker(&mut markers);
                        self.state = State::Normal;
                    }
                    _ => {
                        self.payload.extend([b'\x1b', b]);
                        self.state = State::InMarker;
                    }
                },
            }
        }
        (out, markers)
    }

    /// Flush any pending partial sequence.
    ///
    /// Call once at end-of-stream so trailing partial markers are emitted as
    /// plain bytes instead of being silently dropped.
    pub fn flush(&mut self, out: &mut Vec<u8>) {
        match self.state {
            State::InMarker | State::InMarkerEsc => self.abort_marker(out, 0),
            _ => {
                out.append(&mut self.prefix);
                self.state = State::Normal;
            }
        }
    }

    fn finish_marker(&mut self, markers: &mut Vec<Marker>) {
        let text = String::from_utf8_lossy(&self.payload).to_string();
        self.payload.clear();
        // Drop the pending `ESC ] 9 0 ;` prefix; it has been consumed by the
        // marker. Leaving it would make a later flush() re-emit it.
        self.prefix.clear();
        if let Some(m) = parse_marker_payload(&text) {
            markers.push(m);
        }
        self.state = State::Normal;
    }

    fn abort_marker(&mut self, out: &mut Vec<u8>, last: u8) {
        out.append(&mut self.prefix);
        out.append(&mut self.payload);
        if last != 0 {
            out.push(last);
        }
        self.prefix.clear();
    }
}

/// Parse the payload portion of an inline marker (`cmd;seq;rc;command` or
/// `ch;title`).
fn parse_marker_payload(payload: &str) -> Option<Marker> {
    let mut parts = payload.splitn(4, ';');
    match parts.next()? {
        "cmd" => {
            let seq: u64 = parts.next()?.parse().ok()?;
            let exit_code: i32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(-1);
            let command = parts.next().unwrap_or("").to_string();
            Some(Marker::Cmd { seq, exit_code, command })
        }
        "ch" => Some(Marker::Chapter { title: parts.next().unwrap_or("").to_string() }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_through_plain_bytes() {
        let mut r = MarkerReader::new();
        let (out, markers) = r.feed(b"hello\n");
        assert_eq!(out, b"hello\n");
        assert!(markers.is_empty());
    }

    #[test]
    fn strips_cmd_marker() {
        let marker = encode_cmd_marker(1, 0, "ls -la");
        let mut r = MarkerReader::new();
        let (out, markers) = r.feed(marker.as_bytes());
        assert!(out.is_empty());
        assert_eq!(markers, vec![Marker::Cmd { seq: 1, exit_code: 0, command: "ls -la".into() }]);
    }

    #[test]
    fn strips_chapter_marker() {
        let marker = encode_chapter_marker("deploy step");
        let mut r = MarkerReader::new();
        let (_, markers) = r.feed(marker.as_bytes());
        assert_eq!(markers, vec![Marker::Chapter { title: "deploy step".into() }]);
    }

    #[test]
    fn preserves_non_marker_escapes() {
        let input: &[u8] = &[
            0x1b, b'[', b'3', b'1', b'm', // CSI color on
            b'r', b'e', b'd', 0x1b, b'[', b'0', b'm', // reset
            0x1b, b']', b'1', b'1', b';', b'?', b'x', 0x07, // OSC 11, not 90
        ];
        let mut r = MarkerReader::new();
        let (out, markers) = r.feed(input);
        assert_eq!(out, input);
        assert!(markers.is_empty());
    }

    #[test]
    fn splits_marker_across_chunks() {
        let marker = encode_cmd_marker(2, 1, "make test");
        let marker = marker.as_bytes();
        let mut r = MarkerReader::new();
        let half = marker.len() / 2;
        let (o1, m1) = r.feed(&marker[..half]);
        let (o2, m2) = r.feed(&marker[half..]);
        assert!(o1.is_empty() && o2.is_empty());
        assert!(m1.is_empty());
        assert_eq!(m2.len(), 1);
    }

    #[test]
    fn supports_st_terminator() {
        let mut r = MarkerReader::new();
        let seq = "\x1b]90;ch;title\x1b\\".to_string();
        let (_, markers) = r.feed(seq.as_bytes());
        assert_eq!(markers, vec![Marker::Chapter { title: "title".into() }]);
    }

    #[test]
    fn unknown_rc_defaults_to_minus_one() {
        let mut r = MarkerReader::new();
        let seq = "\x1b]90;cmd;5;;git status\x07";
        let (_, markers) = r.feed(seq.as_bytes());
        assert_eq!(
            markers,
            vec![Marker::Cmd { seq: 5, exit_code: -1, command: "git status".into() }]
        );
    }

    #[test]
    fn oversize_payload_falls_back_to_plain_bytes() {
        let big = "x".repeat(MAX_MARKER_PAYLOAD + 100);
        let seq = format!("\x1b]90;ch;{big}\x07");
        let mut r = MarkerReader::new();
        let (out, markers) = r.feed(seq.as_bytes());
        assert!(markers.is_empty());
        assert_eq!(out, seq.as_bytes());
    }

    #[test]
    fn marker_command_keeps_semicolons() {
        let marker = encode_cmd_marker(3, 0, "echo a; echo b");
        let mut r = MarkerReader::new();
        let (_, markers) = r.feed(marker.as_bytes());
        assert_eq!(
            markers,
            vec![Marker::Cmd { seq: 3, exit_code: 0, command: "echo a; echo b".into() }]
        );
    }

    #[test]
    fn esc_at_buffer_end_is_held_until_flush() {
        // An incomplete escape at the end of a chunk must not be emitted as
        // output early, and a flush must surface it verbatim.
        let marker = encode_cmd_marker(9, 0, "x");
        let marker = marker.as_bytes();
        let mut r = MarkerReader::new();
        let cut = marker.len() - 1; // split right before the final byte
        let (o1, m1) = r.feed(&marker[..cut]);
        assert!(o1.is_empty() && m1.is_empty());
        let (o2, m2) = r.feed(&marker[cut..]);
        assert!(o2.is_empty());
        assert_eq!(m2.len(), 1);
    }

    #[test]
    fn st_terminator_split_across_chunks() {
        // ESC at the end of one chunk, backslash at the start of the next.
        let seq = "\x1b]90;ch;split\x1b".to_string();
        let mut r = MarkerReader::new();
        let (o1, m1) = r.feed(seq.as_bytes());
        assert!(o1.is_empty() && m1.is_empty());
        let (o2, m2) = r.feed(b"\\");
        assert!(o2.is_empty());
        assert_eq!(m2, vec![Marker::Chapter { title: "split".into() }]);
    }

    #[test]
    fn binary_garbage_with_escapes_is_passthrough() {
        let mut r = MarkerReader::new();
        let garbage: Vec<u8> = (0u8..=255).collect();
        let (out, markers) = r.feed(&garbage);
        // No 90-marker can be formed from raw 0..=255 bytes: `]90;` plus a
        // BEL/ST terminator all within the byte range would need the
        // sequence 0x1b,0x5d,0x39,0x30,0x3b — check what actually came out.
        assert!(markers.is_empty(), "markers = {markers:?}");
        assert_eq!(out, garbage, "garbage must round-trip byte for byte");
        // Trailing partial escape flushes back out.
        let mut tail = Vec::new();
        r.flush(&mut tail);
        assert!(tail.is_empty());
    }

    #[test]
    fn chunked_and_whole_feeds_agree() {
        // Property: feeding a marker byte-by-byte yields the same markers as
        // feeding it whole.
        let seq = format!("abc\x1b]90;cmd;7;0;git {} def\x07xyz", "push");
        let mut whole = MarkerReader::new();
        let (wo, wm) = whole.feed(seq.as_bytes());

        let mut piece = MarkerReader::new();
        let mut po = Vec::new();
        let mut pm = Vec::new();
        for b in seq.as_bytes() {
            let (o, m) = piece.feed(std::slice::from_ref(b));
            po.extend_from_slice(&o);
            pm.extend(m);
        }
        let mut ptail = Vec::new();
        piece.flush(&mut ptail);
        po.extend(ptail);

        assert_eq!(wo, po);
        assert_eq!(wm, pm);
        assert_eq!(wm.len(), 1);
        if let Marker::Cmd { command, .. } = &wm[0] {
            assert_eq!(command.as_str(), "git push def");
        }
    }

    #[test]
    fn utf8_marker_text_survives() {
        let seq = "\x1b]90;ch;日本語 テスト\u{00e9}\x07";
        let mut r = MarkerReader::new();
        let (_, markers) = r.feed(seq.as_bytes());
        assert_eq!(markers, vec![Marker::Chapter { title: "日本語 テスト\u{00e9}".into() }]);
    }

    #[test]
    fn bare_bel_outside_osc_is_data() {
        let mut r = MarkerReader::new();
        let (out, markers) = r.feed(b"a\x07b");
        assert_eq!(out, b"a\x07b");
        assert!(markers.is_empty());
    }
}
