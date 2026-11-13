//! screendoc — record your terminal and publish it.
//!
//! The binary entry point: parses CLI arguments, dispatches to the command
//! handlers in [`screendoc::run`], and prints colored diagnostics.

use std::process;

use screendoc::ansi::{AnsiText as _, Bold as _};

fn main() {
    let cli = screendoc::cli::Cli::parse();
    match screendoc::run::execute(&cli) {
        Ok(code) => process::exit(code),
        Err(e) => {
            let msg = e.to_string();
            eprintln!("{} {msg}", "error:".red().bold());
            for cause in e.chain().skip(1) {
                eprintln!("  {}: {cause}", "caused by".dimmed());
            }
            process::exit(1);
        }
    }
}
