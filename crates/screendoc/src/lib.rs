//! screendoc CLI internals shared by the binary and integration tests.
//!
//! Every subcommand is implemented as a plain function in [`run`], taking
//! its parsed argument struct, so tests can drive the full CLI without
//! spawning a process.

/// The CLI argument model (clap derive).
/// Minimal ANSI color output (replaces the colored crate).
pub mod ansi;
/// CLI argument parsing.
pub mod cli;
/// Subcommand handlers.
pub mod run;
/// Small shared helpers (PATH lookup, size parsing, formatting).
pub mod util;
