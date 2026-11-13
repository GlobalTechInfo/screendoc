//! Minimal ANSI color helpers.
//!
//! Replaces the previous `colored` dependency (relicensed to MPL-2.0 in
//! v3) so the dependency tree stays permissive-only (MIT/Apache/BSD).
//!
//! Colors are emitted only when stdout is a TTY and `NO_COLOR` is not
//! set; piped output stays clean.

/// Color helpers for `&str` and `String` (mirrors the old `Colorize`
/// surface used across the CLI).
pub trait AnsiText {
    /// Red text.
    fn red(&self) -> String;
    /// Green text.
    fn green(&self) -> String;
    /// Yellow text.
    fn yellow(&self) -> String;
    /// Blue text.
    fn blue(&self) -> String;
    /// Cyan text.
    fn cyan(&self) -> String;
    /// Dimmed (muted) text.
    fn dimmed(&self) -> String;
}

/// Bold, for chaining like `"✓".green().bold()`.
pub trait Bold {
    /// Bold text.
    fn bold(&self) -> String;
}

fn colors_enabled() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdout()) && std::env::var_os("NO_COLOR").is_none()
}

fn paint(s: &str, code: &str, reset: &str) -> String {
    if colors_enabled() { format!("\x1b[{code}m{s}\x1b[{reset}m") } else { s.to_string() }
}

impl AnsiText for str {
    fn red(&self) -> String {
        paint(self, "31", "39")
    }
    fn green(&self) -> String {
        paint(self, "32", "39")
    }
    fn yellow(&self) -> String {
        paint(self, "33", "39")
    }
    fn blue(&self) -> String {
        paint(self, "34", "39")
    }
    fn cyan(&self) -> String {
        paint(self, "36", "39")
    }
    fn dimmed(&self) -> String {
        paint(self, "2", "22")
    }
}

impl AnsiText for String {
    fn red(&self) -> String {
        <str as AnsiText>::red(self)
    }
    fn green(&self) -> String {
        <str as AnsiText>::green(self)
    }
    fn yellow(&self) -> String {
        <str as AnsiText>::yellow(self)
    }
    fn blue(&self) -> String {
        <str as AnsiText>::blue(self)
    }
    fn cyan(&self) -> String {
        <str as AnsiText>::cyan(self)
    }
    fn dimmed(&self) -> String {
        <str as AnsiText>::dimmed(self)
    }
}

impl Bold for str {
    fn bold(&self) -> String {
        if colors_enabled() { format!("\x1b[1m{self}\x1b[22m") } else { self.to_string() }
    }
}

impl Bold for String {
    fn bold(&self) -> String {
        <str as Bold>::bold(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piped_output_has_no_escapes() {
        // Under the test harness stdout is not a TTY: no escape codes.
        let s = "x".green();
        assert_eq!(s, "x");
        let s = "x".green().bold();
        assert_eq!(s, "x");
    }
}
