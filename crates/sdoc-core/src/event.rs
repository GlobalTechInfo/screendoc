use serde::{Deserialize, Serialize};

/// Terminal grid size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    /// Width in columns.
    pub cols: u16,
    /// Height in rows.
    pub rows: u16,
}

impl Default for Size {
    fn default() -> Self {
        Self { cols: 120, rows: 30 }
    }
}

impl From<(u16, u16)> for Size {
    fn from((cols, rows): (u16, u16)) -> Self {
        Self { cols, rows }
    }
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.cols, self.rows)
    }
}

/// Origin of a chapter marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChapterSource {
    /// Derived automatically from a command boundary.
    Auto,
    /// Added manually by the user (`screendoc chapter`).
    Manual,
}

/// How a session was recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Command executed inside a recorded pseudo-terminal.
    Wrapper,
    /// Existing tmux pane streamed via `pipe-pane`.
    Tmux,
    /// Shell journal only (ambient hooks, no raw byte stream).
    Journal,
}

/// A structured event in a session log.
///
/// All timestamps are milliseconds since session start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "e", rename_all = "snake_case")]
pub enum Event {
    /// The session started.
    SessionStart {
        /// Milliseconds since start (always 0).
        at_ms: u64,
        /// The recorded shell, if known.
        shell: Option<String>,
        /// The initial terminal size.
        size: Size,
    },
    /// A command boundary captured at the prompt, with exit status.
    Command {
        /// Milliseconds since start.
        at_ms: u64,
        /// 1-based sequence number of the command within the session.
        seq: u64,
        /// Exit status of the command, or `-1` when unknown.
        exit_code: i32,
        /// The command text as entered.
        command: String,
    },
    /// A chapter marker (automatic or manual).
    Chapter {
        /// Milliseconds since start.
        at_ms: u64,
        /// Chapter title.
        title: String,
        /// Where the chapter came from.
        source: ChapterSource,
    },
    /// The terminal was resized.
    Resize {
        /// Milliseconds since start.
        at_ms: u64,
        /// The new size.
        size: Size,
    },
    /// The session ended.
    SessionEnd {
        /// Milliseconds since start.
        at_ms: u64,
        /// Human-readable end reason.
        reason: String,
    },
}

impl Event {
    /// The millisecond timestamp of this event.
    pub fn at_ms(&self) -> u64 {
        match self {
            Self::SessionStart { at_ms, .. }
            | Self::Command { at_ms, .. }
            | Self::Chapter { at_ms, .. }
            | Self::Resize { at_ms, .. }
            | Self::SessionEnd { at_ms, .. } => *at_ms,
        }
    }
}

/// Persisted session metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    /// The opaque session identifier.
    pub id: String,
    /// Wall-clock start time, Unix milliseconds.
    pub started_unix_ms: u64,
    /// The recorded shell, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Working directory the session started in, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// How the session was recorded.
    pub mode: Mode,
    /// Whether a raw byte stream (video) is available.
    pub video: bool,
    /// tmux pane target this session was streamed from, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_target: Option<String>,
    /// Duration in milliseconds, set at finalization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Human-provided title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Group name linking sessions recorded from the same tmux session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}
