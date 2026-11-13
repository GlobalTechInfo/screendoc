//! Process-wide Ctrl-C flag.
//!
//! A handler is registered exactly once, lazily, on first use: the CLI
//! dispatches many subcommands in one process (and the test suite drives
//! them in-process), and `ctrlc` allows a single handler per process.
//!
//! Backend per platform:
//! - Unix: `signal_hook::flag` — a signal-safe handler that flips the
//!   atomic directly in the signal context; no threads, no valgrind
//!   findings.
//! - Windows: `ctrlc` — the portable console-control equivalent.

use std::sync::Once;
use std::sync::atomic::AtomicBool;

/// Flipped by Ctrl-C anywhere in the process.
pub static CTRL_C: AtomicBool = AtomicBool::new(false);

static REGISTERED: Once = Once::new();

/// Ensure the process-wide handler is installed (no-op after the first
/// call, and also a no-op where the OS handler cannot be installed — the
/// flag simply stays clear) and return it.
#[must_use]
pub fn flag() -> &'static AtomicBool {
    REGISTERED.call_once(|| {
        #[cfg(unix)]
        {
            use signal_hook::consts::SIGINT;
            // Signal-safe: flips the atomic without allocation or locks.
            // Best effort — if registration fails, Ctrl-C falls back to
            // the OS default (terminate) instead of our graceful finalize.
            let _ = signal_hook::flag::register(SIGINT, &CTRL_C);
        }
        #[cfg(windows)]
        {
            use std::sync::atomic::Ordering;
            let _ = ctrlc::set_handler(|| {
                CTRL_C.store(true, Ordering::SeqCst);
            });
        }
    });
    &CTRL_C
}
