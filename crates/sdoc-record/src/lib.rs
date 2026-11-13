//! screendoc recording engines: pty wrapper, shell hook generation, tmux pipe-pane.
//!
//! Three recording modes share one event model from [`sdoc_core`]:
//!
//! * [`wrapper`] — run a command inside a recorded pseudo-terminal (Mode 1).
//! * [`ambient`] — journal-style ambient hooks for a running shell (Mode 2).
//! * [`tmux`] — stream an existing tmux pane via `pipe-pane` (Mode 3).

/// Shell hook generation (wrapper rc files and ambient `eval` snippets).
pub mod ambient;
/// Shell hook script generation.
pub mod hooks;
/// Tmux pane streaming via `pipe-pane`.
pub mod tmux;
/// Pseudo-terminal wrapper recording.
pub mod wrapper;
