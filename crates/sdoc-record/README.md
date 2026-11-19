# sdoc-record

screendoc recording engines: pty wrapper, shell hook generation, tmux
pipe-pane.

Three recording modes share one event model from `sdoc-core`:

- `wrapper` — run a command inside a recorded pseudo-terminal (Mode 1).
- `ambient` — journal-style ambient hooks for a running shell (Mode 2).
- `tmux` — stream an existing tmux pane via `pipe-pane` (Mode 3).

- Documentation: <https://github.com/GlobalTechInfo/screendoc>
- License: MIT OR Apache-2.0
