//! Secret redaction for recorded sessions.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::err::{CoreError, Result};
use crate::session::Session;

/// One named masking rule.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    /// Human-readable rule name (used in summaries and `--rules` files).
    pub name: String,
    /// The compiled regular expression.
    pub re: Regex,
    /// The replacement text (capture groups supported).
    pub replacement: String,
}

/// Summary of a redaction pass.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RedactSummary {
    /// Matches replaced in the raw cast byte stream.
    pub cast_replacements: usize,
    /// Matches replaced in the structured event log.
    pub event_replacements: usize,
    /// Per-rule hit counts (empty for single-pattern redaction).
    pub by_rule: Vec<RuleHit>,
}

impl RedactSummary {
    /// Total replacements across both logs.
    pub fn total(&self) -> usize {
        self.cast_replacements + self.event_replacements
    }
}

/// Hits for a single rule across the whole session.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RuleHit {
    /// Rule name.
    pub rule: String,
    /// Number of matches masked.
    pub masked: usize,
}

/// Compile a user-supplied masking pattern.
pub fn compile_pattern(pattern: &str) -> Result<regex::Regex> {
    Regex::new(pattern).map_err(|e| CoreError::Other(format!("invalid pattern `{pattern}`: {e}")))
}

/// Mask every occurrence of `pattern` in a terminal byte string.
///
/// The regex is applied only inside printable text segments — escape
/// sequences pass through untouched. Returns the masked string and the
/// replacement count; errors only on an invalid regular expression.
pub fn redact_segments(
    input: &str,
    pattern: &str,
    replacement: &str,
) -> std::result::Result<(String, usize), CoreError> {
    let re = Regex::new(pattern)
        .map_err(|e| CoreError::Other(format!("invalid pattern `{pattern}`: {e}")))?;
    Ok(mask_segments(input, &re, replacement))
}

/// Redact a session in place: rewrite `raw.cast` and `events.jsonl` so that
/// every occurrence of the pattern is replaced.
///
/// The regular expression is applied only inside printable text segments —
/// ANSI/escape sequences pass through untouched, so the terminal stream
/// stays well-formed even when a secret is interleaved with colors.
pub fn redact_session(
    session: &Session,
    pattern: &str,
    replacement: &str,
) -> Result<RedactSummary> {
    let rule = CompiledRule {
        name: "pattern".into(),
        re: Regex::new(pattern)
            .map_err(|e| CoreError::Other(format!("invalid pattern `{pattern}`: {e}")))?,
        replacement: replacement.to_string(),
    };
    redact_session_rules(session, &[rule])
}

/// Redact a session in place using a list of named rules.
///
/// Rules are applied in order; each rule's replacement (capture groups
/// supported) is used independently. Returns the aggregate summary with
/// per-rule hit counts.
pub fn redact_session_rules(session: &Session, rules: &[CompiledRule]) -> Result<RedactSummary> {
    let mut summary = RedactSummary::default();

    // Raw cast (asciinema v2): mask the payload of o/i/r events only.
    // Decryption for encrypted sessions is transparent.
    let cast_text = session.read_cast_text()?;
    let mut out = String::new();
    for line in cast_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[')
            && let Ok(mut v) = serde_json::from_str::<Value>(trimmed)
        {
            let arr = v.as_array_mut();
            if let Some(arr) = arr
                && arr.len() >= 3
                && matches!(arr[1].as_str(), Some("o") | Some("i"))
                && arr[2].is_string()
            {
                let (masked, counts) = mask_rules_segments(arr[2].as_str().unwrap_or(""), rules);
                summary.cast_replacements += total_of(&counts);
                add_by_rule(&mut summary.by_rule, &counts);
                arr[2] = Value::String(masked);
            }
            // Re-serialize the (possibly masked) event.
            out.push_str(&serde_json::to_string(&v).expect("valid JSON"));
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    session.write_cast_text(&out)?;

    // Event log: mask command text and chapter titles.
    let ev_text = session.read_events_text()?;
    let mut out = String::new();
    for line in ev_text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty()
            && let Ok(mut v) = serde_json::from_str::<Value>(trimmed)
        {
            let obj = v.as_object_mut();
            if let Some(obj) = obj {
                if let Some(Value::String(c)) = obj.get_mut("command") {
                    let (masked, counts) = mask_rules(c, rules);
                    summary.event_replacements += total_of(&counts);
                    add_by_rule(&mut summary.by_rule, &counts);
                    *c = masked;
                }
                if let Some(Value::String(t)) = obj.get_mut("title") {
                    let (masked, counts) = mask_rules(t, rules);
                    summary.event_replacements += total_of(&counts);
                    add_by_rule(&mut summary.by_rule, &counts);
                    *t = masked;
                }
            }
            out.push_str(&serde_json::to_string(&v).expect("valid JSON"));
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    session.write_events_text(&out)?;

    Ok(summary)
}

/// Apply every rule to plain (no-escape) text; per-rule hit counts.
pub fn mask_rules(input: &str, rules: &[CompiledRule]) -> (String, Vec<(String, usize)>) {
    let mut text = input.to_string();
    let mut counts = Vec::with_capacity(rules.len());
    for rule in rules {
        let n = rule.re.find_iter(&text).count();
        if n > 0 {
            text = rule.re.replacen(&text, n, &rule.replacement).into_owned();
        }
        counts.push((rule.name.clone(), n));
    }
    (text, counts)
}

/// Apply every rule to the printable segments of a terminal byte string
/// (escape sequences untouched); per-rule hit counts.
pub fn mask_rules_segments(input: &str, rules: &[CompiledRule]) -> (String, Vec<(String, usize)>) {
    let mut out = String::new();
    let mut counts: Vec<(String, usize)> = Vec::with_capacity(rules.len());
    for r in rules {
        counts.push((r.name.clone(), 0));
    }
    let bytes = input.as_bytes();
    let mut i = 0usize;
    let mut seg_start = 0usize;

    while i < bytes.len() {
        if bytes[i] == 0x1b {
            // Keep str slices on char boundaries: an escape "sequence" that
            // runs into a multibyte character swallows the whole character
            // rather than splitting it.
            let raw = escape_end(bytes, i).min(input.len());
            let end = input.ceil_char_boundary(raw);
            let seg = &input[seg_start..i];
            let (masked, c) = mask_rules(seg, rules);
            out.push_str(&masked);
            for (k, pair) in counts.iter_mut().enumerate() {
                pair.1 += c.get(k).map(|p| p.1).unwrap_or(0);
            }
            out.push_str(&input[i..end]);
            i = end;
            seg_start = end;
            continue;
        }
        i += 1;
    }
    let (masked, c) = mask_rules(&input[seg_start..], rules);
    out.push_str(&masked);
    for (k, pair) in counts.iter_mut().enumerate() {
        pair.1 += c.get(k).map(|p| p.1).unwrap_or(0);
    }
    (out, counts)
}

fn total_of(counts: &[(String, usize)]) -> usize {
    counts.iter().map(|(_, n)| n).sum()
}

fn add_by_rule(target: &mut Vec<RuleHit>, counts: &[(String, usize)]) {
    for (name, n) in counts {
        if *n == 0 {
            continue;
        }
        match target.iter_mut().find(|h| &h.rule == name) {
            Some(h) => h.masked += n,
            None => target.push(RuleHit { rule: name.clone(), masked: *n }),
        }
    }
}

/// Built-in high-precision secret detectors.
///
/// These catch the common credential shapes (cloud keys, VCS tokens, private
/// key blocks, bearer tokens, credential assignments) without the false
/// positives of generic "long random string" heuristics. All replacements
/// are `***` unless a rule says otherwise; use [`parse_rules_file`] for
/// custom rules.
pub fn secret_rules() -> Vec<CompiledRule> {
    fn rule(name: &str, pattern: &str, replacement: &str) -> CompiledRule {
        CompiledRule {
            name: name.to_string(),
            re: Regex::new(pattern).expect("built-in rule compiles"),
            replacement: replacement.to_string(),
        }
    }
    vec![
        rule("aws-access-key", r"\bAKIA[0-9A-Z]{16}\b", "***"),
        rule("github-token", r"\bgh[pousr]_[A-Za-z0-9]{20,255}\b", "***"),
        rule("google-api-key", r"\bAIza[0-9A-Za-z_\-]{35}\b", "***"),
        rule("slack-token", r"\bxox[baprs]-[A-Za-z0-9\-]{10,}\b", "***"),
        rule("stripe-key", r"\b[rs]k_(live|test)_[0-9a-zA-Z]{10,255}\b", "***"),
        rule("openai-key", r"\bsk-[A-Za-z0-9]{20,}\b", "***"),
        rule("huggingface-token", r"\bhf_[A-Za-z0-9]{20,255}\b", "***"),
        rule(
            "private-key-block",
            r"-----BEGIN (?:[A-Z]+ )?PRIVATE KEY-----[\s\S]*?-----END (?:[A-Z]+ )?PRIVATE KEY-----",
            "***private key***",
        ),
        rule("bearer-token", r"\bBearer [A-Za-z0-9\-._~+/\s]{16,}=", "Bearer ***"),
        rule(
            "credential-assignment",
            r"(?i)\b(\w*(?:password|passwd|pwd|secret|token|access[_-]?key|api[_-]?key|auth[_-]?key)\w*)(\s*[:=]\s*)(\S+)",
            "$1$2***",
        ),
    ]
}

/// Parse a rules file: one rule per line, `#` comments and blank lines
/// ignored. A line is `regex` (replacement defaults to `***`) or
/// `regex<TAB>replacement`. Rule names are `rule-1`, `rule-2`, …
pub fn parse_rules_file(text: &str) -> Result<Vec<CompiledRule>> {
    let mut rules = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (pattern, replacement) = match line.find('\t') {
            Some(t) => (&line[..t], &line[t + 1..]),
            None => (line, "***"),
        };
        let re = Regex::new(pattern).map_err(|e| {
            CoreError::Other(format!("rule {}: invalid regex `{pattern}`: {e}", n + 1))
        })?;
        rules.push(CompiledRule {
            name: format!("rule-{}", rules.len() + 1),
            re,
            replacement: replacement.trim().to_string(),
        });
    }
    if rules.is_empty() {
        return Err(CoreError::Other("rules file contains no rules".to_string()));
    }
    Ok(rules)
}

/// Apply the regex to every printable segment of a terminal byte string.
///
/// Escape sequences (`ESC [ ... final byte` and `ESC ] ... BEL/ST`) are
/// returned verbatim; the regex only ever sees text between them.
fn mask_segments(input: &str, re: &Regex, replacement: &str) -> (String, usize) {
    let mut out = String::new();
    let mut count = 0usize;
    let bytes = input.as_bytes();
    let mut i = 0usize;
    let mut seg_start = 0usize;

    while i < bytes.len() {
        if bytes[i] == 0x1b {
            // Keep str slices on char boundaries: an escape "sequence" that
            // runs into a multibyte character swallows the whole character
            // rather than splitting it.
            let raw = escape_end(bytes, i).min(input.len());
            let end = input.ceil_char_boundary(raw);
            let seg = &input[seg_start..i];
            let (masked, k) = mask_plain(seg, re, replacement);
            out.push_str(&masked);
            count += k;
            out.push_str(&input[i..end]);
            i = end;
            seg_start = end;
            continue;
        }
        i += 1;
    }
    let (masked, k) = mask_plain(&input[seg_start..], re, replacement);
    out.push_str(&masked);
    count += k;
    (out, count)
}

/// The index just past the escape sequence starting at `start`.
fn escape_end(bytes: &[u8], start: usize) -> usize {
    let n = bytes.len();
    let mut i = start + 1; // skip ESC
    if i < n {
        match bytes[i] {
            b'[' => {
                // CSI: ESC [ ... final byte (0x40..=0x7E)
                i += 1;
                while i < n && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                i.saturating_add(1)
            }
            b']' => {
                // OSC: ESC ] ... terminated by BEL or ST (ESC \)
                i += 1;
                while i < n {
                    if bytes[i] == 0x07 {
                        return i + 1;
                    }
                    if bytes[i] == 0x1b && i + 1 < n && bytes[i + 1] == b'\\' {
                        return i + 2;
                    }
                    i += 1;
                }
                n
            }
            _ => i.saturating_add(1), // two-byte ESC sequences
        }
    } else {
        n
    }
}

/// Apply the regex to plain (no-escape) text.
fn mask_plain(text: &str, re: &Regex, replacement: &str) -> (String, usize) {
    let n = re.find_iter(text).count();
    let masked = re.replacen(text, n, replacement).into_owned();
    (masked, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_plain_text() {
        let re = Regex::new(r"sk-[a-z0-9]{10}").unwrap();
        let (out, n) = mask_plain("key=sk-abcdef1234 done", &re, "***");
        assert_eq!(n, 1);
        assert_eq!(out, "key=*** done");
    }

    #[test]
    fn escapes_pass_through_and_text_is_masked() {
        let re = Regex::new(r"sk-[a-z0-9]{10}").unwrap();
        // ESC[1;31m (red) + secret + ESC[0m, in one string.
        let s = "\x1b[1;31msecret sk-abcdef1234 leak\x1b[0m";
        let (out, n) = mask_segments(s, &re, "***");
        assert_eq!(n, 1);
        // The color codes survive intact.
        assert!(out.starts_with("\x1b[1;31msecret "));
        assert!(out.ends_with(" leak\x1b[0m"));
        assert!(out.contains("***"));
        assert!(!out.contains("sk-abcdef1234"));
    }

    #[test]
    fn multiple_matches_all_replaced() {
        let re = Regex::new(r"token").unwrap();
        let (out, n) = mask_plain("token token", &re, "x");
        assert_eq!(n, 2);
        assert_eq!(out, "x x");
    }

    #[test]
    fn regex_metacharacters_are_literal_in_segments() {
        // Dots in the pattern are regex metachars; escape them for literals.
        let r = redact_segments("abc.def abcXdef", r"abc\.def", "[redacted]");
        let (out, n) = r.unwrap();
        assert_eq!(n, 1);
        assert_eq!(out, "[redacted] abcXdef");
    }

    #[test]
    fn replacement_supports_capture_groups() {
        let (out, n) = redact_segments("user=tok-123", r"(tok)-(\w+)", "$2=$1").unwrap();
        assert_eq!(n, 1);
        assert_eq!(out, "user=123=tok");
    }

    #[test]
    fn patterns_do_not_match_inside_escape_sequences() {
        // A color code that *looks like* part of the pattern stays intact.
        let (out, n) =
            redact_segments("\x1b[31mx\x1b[0m\x1b[31msecret\x1b[0m", "\x1b\\[31m", "XXX").unwrap();
        // `\x1b[31m` exists only inside escapes, which we never match.
        assert_eq!(n, 0);
        assert_eq!(out, "\x1b[31mx\x1b[0m\x1b[31msecret\x1b[0m");
    }

    #[test]
    fn invalid_pattern_errors() {
        assert!(redact_segments("text", "([unclosed", "x").is_err());
    }

    #[test]
    fn empty_input_is_noop() {
        let (out, n) = redact_segments("", "x", "y").unwrap();
        assert_eq!((out, n), (String::new(), 0));
    }

    #[test]
    fn text_between_many_escapes_is_still_masked() {
        let s = "\x1b[1m\x1b[31mkey=AKIASECRET\x1b[0m \x1b[32mok\x1b[0m";
        let (out, n) = redact_segments(s, "AKIA\\w{5}", "***").unwrap();
        assert_eq!(n, 1);
        assert!(!out.contains("AKIASECRET"));
        // All escape sequences survive.
        assert!(out.contains("\x1b[1m\x1b[31m"));
        assert!(out.contains("***"));
        assert!(out.contains("\x1b[32mok\x1b[0m"));
    }

    /// Regression: an escape sequence that runs into a multibyte character
    /// (e.g. `ESC [` immediately followed by a 3-byte UTF-8 char) used to
    /// compute a slice index mid-character and panic. Found by fuzzing.
    #[test]
    fn escape_adjacent_to_multibyte_char_does_not_panic() {
        // ESC '[' then a 3-byte char: the CSI scan lands mid-char; the mask
        // must swallow the whole char instead of splitting it.
        let input = "\x1b[\u{20AC}\x1b[31msecret\x1b[0m€€";
        let (out, n) = redact_segments(input, "secret", "***").unwrap();
        assert_eq!(n, 1);
        assert!(out.contains("***"));
        assert!(!out.contains("secret"));
        // No trailing CSI without a final byte: swallow to end, no panic.
        let _ = redact_segments("\x1b[€€€", "€", "x").unwrap();
        // Truncated OSC: no BEL/ST terminator.
        let _ = redact_segments("\x1b]0;€€€", "€", "x").unwrap();
    }

    #[test]
    fn builtin_secret_detectors_mask_known_shapes() {
        let rules = secret_rules();
        let google = format!("AIza{}", "A".repeat(35));
        // Stripe/github-token fixtures are built at runtime: a literal
        // `sk_live_<24 alnum>` in source trips GitHub's secret scanner
        // (push protection), while the constructed value still exercises
        // the real detector.
        let stripe = format!("sk_live_{}", "FAKE0000000000000000");
        let github = format!("ghp_{}", "ABCDEFghijklmnopQKLMNOpqrs");
        let text = format!(
            "export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n\
             {github}\n\
             {google}\n\
             xoxb-1234567890-abcdefg\n\
             {stripe}\n\
             hf_AbCdEfGhIjKlMnOpQrStUvWxYz\n\
             Bearer abcdefghij1234567890==\n"
        );
        let (out, counts) = mask_rules(&text, &rules);
        for pair in &counts {
            let want = match pair.0.as_str() {
                "aws-access-key"
                | "github-token"
                | "google-api-key"
                | "slack-token"
                | "stripe-key"
                | "huggingface-token"
                | "bearer-token"
                | "credential-assignment" => 1,
                _ => 0,
            };
            assert_eq!(pair.1, want, "{} expected {want} hit(s), got {}", pair.0, pair.1);
        }
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!out.contains("ghp_ABCDEF"));
        assert!(!out.contains("sk_live_"));
        assert!(out.contains("***"));
    }

    #[test]
    fn builtin_detectors_ignore_lookalikes() {
        let rules = secret_rules();
        // Too short / wrong shape: no matches, no mask.
        let (out, counts) = mask_rules("AKIA123 ghp_short sk_live_9 xoxb-123", &rules);
        assert_eq!(out, "AKIA123 ghp_short sk_live_9 xoxb-123");
        assert!(counts.iter().all(|(_, n)| *n == 0));
    }

    #[test]
    fn private_key_block_is_masked_wholesale() {
        let text = concat!(
            "-----BEGIN RSA PRIVATE KEY-----\n",
            "MIIEpAIBAAKCAQEA7\n",
            "-----END RSA PRIVATE KEY-----\n",
        );
        let rules = secret_rules();
        let (out, counts) = mask_rules(text, &rules);
        let block = counts.iter().find(|(n, _)| n == "private-key-block").expect("rule present");
        assert_eq!(block.1, 1);
        assert!(out.contains("***private key***"));
        assert!(!out.contains("MIIEpAIBAAKCAQEA7"));
    }

    #[test]
    fn credential_assignment_keeps_the_key_name() {
        let rules = secret_rules();
        // Prefixed names (DB_PASSWORD) and suffixed ones (AWS_ACCESS_KEY_ID)
        // both match; the value is masked, the name and separator kept.
        let (out, counts) =
            mask_rules("export DB_PASSWORD=hunter2 AWS_ACCESS_KEY_ID=AKIAZZZZZZZZZZZZZZZZ", &rules);
        assert!(out.contains("DB_PASSWORD=***"), "got: {out}");
        assert!(out.contains("AWS_ACCESS_KEY_ID="), "got: {out}");
        assert!(!out.contains("hunter2"));
        // The plain AWS rule masks the key itself in the same pass.
        assert!(counts.iter().any(|(n, n2)| n == "credential-assignment" && *n2 >= 2));
        assert!(counts.iter().any(|(n, n2)| n == "aws-access-key" && *n2 >= 1));
    }

    #[test]
    fn rules_file_parses_with_tabs_comments_and_defaults() {
        // A real TAB separates regex from replacement; `\s`/`\t` in the
        // regex stay escaped sequences in the file.
        let text = "# a rule\nMYREGEX\nother\\s+thing\t[hit]\n\n".to_string();
        let rules = parse_rules_file(&text).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].replacement, "***");
        assert_eq!(rules[1].replacement, "[hit]");
        assert_eq!(rules[0].name, "rule-1");
        assert_eq!(rules[1].name, "rule-2");
    }

    #[test]
    fn rules_file_rejects_invalid_regex_and_empty() {
        assert!(parse_rules_file("([unclosed").is_err());
        assert!(parse_rules_file("# only a comment\n").is_err());
    }

    #[test]
    fn rule_masking_applies_in_order_with_per_rule_counts() {
        let rules = vec![
            CompiledRule {
                name: "a".into(),
                re: Regex::new(r"alpha").unwrap(),
                replacement: "A".into(),
            },
            CompiledRule {
                name: "b".into(),
                re: Regex::new(r"A").unwrap(),
                replacement: "B".into(),
            },
        ];
        let (out, counts) = mask_rules("alpha", &rules);
        // Rule a replaces alpha→A, then rule b replaces A→B.
        assert_eq!(out, "B");
        assert_eq!(counts, vec![("a".to_string(), 1), ("b".to_string(), 1)]);
    }
}
