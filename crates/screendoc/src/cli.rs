use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

/// screendoc — record your terminal and publish it.
#[derive(Debug, Parser)]
#[command(
    name = "screendoc",
    version,
    about = "Record your terminal and publish it as GIF, MP4, asciinema cast, and web player",
    long_about = None
)]
pub struct Cli {
    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Record a command (wrapper) or a tmux pane.
    Record {
        /// Record a tmux pane instead of running a command.
        ///
        /// Optional target (`screendoc record --tmux main:0`); when omitted,
        /// the current pane is used (must run inside tmux).
        #[arg(long, num_args(0..=1), value_name = "TARGET")]
        tmux: Option<Option<String>>,

        /// Record every pane of a tmux session as linked sessions.
        #[arg(long, value_name = "SESSION")]
        tmux_group: Option<String>,

        /// Terminal size as `COLSXROWS` (wrapper mode).
        #[arg(long, default_value = "120x30")]
        size: String,

        /// Working directory for the recorded command.
        #[arg(long)]
        cwd: Option<PathBuf>,

        /// Encrypt this session's files at rest (AES-256-GCM, key at
        /// `~/.screendoc/key`). The `encrypt` config key also enables
        /// this by default.
        #[arg(long)]
        encrypt: bool,

        /// The command to record.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },

    /// Start an ambient recording of your running shell.
    On {
        /// Shell to generate hooks for (default: detected from `$SHELL`).
        #[arg(long)]
        shell: Option<String>,

        /// Encrypt this session's files at rest (AES-256-GCM).
        #[arg(long)]
        encrypt: bool,
    },

    /// Stop the ambient recording and finalize the session.
    Off {
        /// Session id (default: active session).
        #[arg(long)]
        session: Option<String>,
    },

    /// Print the hook snippet for a running shell (`eval` it).
    HookScript {
        /// Target shell: bash, zsh, or fish.
        shell: String,
    },

    /// Append a structured marker (used by shell hooks; not for humans).
    #[command(hide = true)]
    HookAppend {
        /// Marker kind: `cmd`, `cmd-start`, `cmd-end`, or `chapter`.
        kind: String,
        /// Command sequence number.
        #[arg(long)]
        seq: Option<u64>,
        /// Exit status.
        #[arg(long)]
        rc: Option<i32>,
        /// Command text (or chapter title).
        #[arg(long)]
        text: Option<String>,
        /// Session id (default: `$SCREENDOC_SESSION` or the active session).
        #[arg(long)]
        session: Option<String>,
    },

    /// Add a manual chapter marker to the active session.
    Chapter {
        /// The chapter title.
        title: String,
        /// Session id (default: active session).
        #[arg(long)]
        session: Option<String>,
    },

    /// Render derived artifacts (GIF / MP4 / WebM / AV1 / cast / stills / web).
    Render {
        /// The session id.
        id: String,
        /// Produce a GIF.
        #[arg(long)]
        gif: bool,
        /// Produce an MP4.
        #[arg(long)]
        mp4: bool,
        /// Produce a WebM (VP9).
        #[arg(long)]
        webm: bool,
        /// Produce an AV1 MP4.
        #[arg(long)]
        av1: bool,
        /// Export an asciinema cast copy.
        #[arg(long)]
        cast: bool,
        /// Produce a self-contained shareable web player (.html).
        #[arg(long)]
        web: bool,
        /// Render one short video per chapter.
        #[arg(long)]
        chapters: bool,
        /// With `--chapters`, skip the full-session video.
        #[arg(long)]
        no_full: bool,
        /// Emit a PNG still at each chapter boundary and the final frame.
        #[arg(long)]
        frames: bool,
        /// Override the video frame rate.
        #[arg(long)]
        fps: Option<u32>,
        /// Override the GIF frame rate.
        #[arg(long)]
        gif_fps: Option<u32>,
        /// Playback speed multiplier (1.0 = real time, 2.0 = 2× faster).
        #[arg(long)]
        speed: Option<f64>,
        /// Idle-time limit in seconds (0 disables compression).
        #[arg(long)]
        idle_limit: Option<f64>,
        /// Output directory.
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },

    /// Replay a session in this terminal, in real time.
    Play {
        /// The session id.
        id: String,
        /// Playback speed multiplier (1.0 = real time).
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
        /// Skip all waiting (writes the stream immediately; for piping).
        #[arg(long)]
        fast: bool,
        /// Start replay at N seconds from the beginning.
        #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
        from: Option<f64>,
        /// Stop replay at N seconds (defaults to the end).
        #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
        to: Option<f64>,
    },

    /// Import an asciinema cast file (v2 or v3, plain or gzip) as a session.
    Import {
        /// Path to the `.cast` / `.jsonl` (optionally `.gz`) file.
        file: PathBuf,
        /// A title to attach to the session.
        #[arg(long)]
        title: Option<String>,
        /// Print only the new session id as JSON (machine output).
        #[arg(long)]
        json: bool,
    },

    /// Serve the session's web player on a local port.
    Serve {
        /// The session id.
        id: String,
        /// Port to bind (default from config).
        #[arg(long)]
        port: Option<u16>,
        /// Open the player in the default browser.
        #[arg(long)]
        open: bool,
    },

    /// Stop a tmux recording that was started in the background.
    Stop {
        /// Session id (default: active session).
        #[arg(long)]
        session: Option<String>,
    },

    /// List recorded sessions.
    List {
        /// Print as JSON.
        #[arg(long)]
        json: bool,
        /// Print as CSV (id, started, duration_ms, commands, mode, title, encrypted).
        #[arg(long)]
        csv: bool,
    },

    /// Show the active session.
    Status {
        /// Print as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Remove a session and all its files.
    Rm {
        /// The session id.
        id: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Rename a session's id (updates metadata and the active pointer).
    Rename {
        /// The current session id.
        from: String,
        /// The new id (a single safe path component).
        to: String,
    },

    /// Delete old sessions to keep the data directory bounded.
    Prune {
        /// Delete sessions older than a duration (`30d`, `7w`, `24h`).
        #[arg(long, value_name = "DURATION")]
        older_than: Option<String>,
        /// Keep at most this many of the most recent sessions.
        #[arg(long, value_name = "N")]
        keep: Option<u64>,
        /// Delete every session.
        #[arg(long)]
        all: bool,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
        /// Print the removed ids as JSON (machine output).
        #[arg(long)]
        json: bool,
    },

    /// Search command text across all recorded sessions.
    Search {
        /// Substring (case-insensitive) to look for in commands.
        query: String,
        /// Only search this session id.
        #[arg(long)]
        session: Option<String>,
        /// Print results as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Mask secrets out of a session in place (irreversible).
    ///
    /// Give a regular expression, `--secret` (built-in credential
    /// detectors), `--rules FILE` (custom rule lines), or any combination.
    Redact {
        /// The session id.
        id: String,
        /// Regular expression to mask.
        pattern: Option<String>,
        /// Replacement text for PATTERN (default `***`).
        #[arg(long, default_value = "***")]
        replace: String,
        /// Apply the built-in high-precision secret detectors
        /// (cloud keys, VCS tokens, private-key blocks, …).
        #[arg(long)]
        secret: bool,
        /// Mask with a rules file: one rule per line,
        /// `regex` or `regex<TAB>replacement` (`#` comments allowed).
        #[arg(long, value_name = "FILE")]
        rules: Option<PathBuf>,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
        /// Print a per-rule summary as JSON (machine output).
        #[arg(long)]
        json: bool,
    },

    /// Check dependencies and the screendoc environment.
    Doctor {
        /// Print as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Show or edit the user configuration (`~/.config/screendoc/config.toml`).
    Config {
        /// The config subcommand.
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Emit shell completion scripts.
    Completions {
        /// Target shell.
        #[arg(value_enum)]
        shell: Shell,
    },
}

/// `screendoc config` subcommands.
#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Print the effective configuration (file values over defaults).
    Show,
    /// Print one key of the effective configuration.
    Get {
        /// The config key (e.g. `fps`, `theme`, `idle_limit`).
        key: String,
    },
    /// Set a key in the user config file (creating it if needed).
    Set {
        /// The config key (e.g. `fps`, `theme`, `idle_limit`).
        key: String,
        /// The new value (parsed per key type).
        value: String,
    },
}

impl Cli {
    /// Parse command-line arguments from the environment.
    pub fn parse() -> Self {
        <Self as clap::Parser>::parse()
    }
}
