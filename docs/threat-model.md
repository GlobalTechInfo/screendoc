# Threat model

screendoc records *terminals*, which routinely display secrets (passwords,
tokens, keys) even when the user did not intend to share them. This document
states the trust assumptions, the threats considered, and the mitigations.

## Trust assumptions

1. **Local user is the owner.** The machine and its `~/.screendoc` store
   belong to one user. Multi-user sharing of one home directory is out of
   scope.
2. **The tmux server is trusted.** Wrapper and pane-mode recording flows
   through tmux; an attacker who can control the tmux server can already
   read every pane, and the threat model adds nothing.
3. **The shell is semi-trusted.** Shell hooks run in your shell and can
   emit crafted bytes. The marker demuxer is a finite-state machine with
   no allocation growth on malformed input, so a hostile shell cannot
   crash or unbound the recorder — but it can produce garbage markers,
   which are dropped, not executed.
4. **ffmpeg is trusted** for the video pipeline; screendoc passes fixed
   arguments plus its own temp paths (see below).

## Threats and mitigations

### T1 — Secrets left on disk after a session is shared

A session recorded without `--encrypt` stores the raw terminal byte
stream in `~/.screendoc/sessions/<id>/raw.cast`. If you share a rendered
video but the session store stays on a synced/backuped disk, secrets that
appeared on screen persist in plaintext.

**Mitigations:** `screendoc redact` (in-place, all stored copies),
`--encrypt` at record time, `screendoc prune` with `retention_days`.
**Residual:** derived artifacts created *before* redaction are not
rewritten — delete and re-render.

### T2 — Key material on the same machine as the ciphertext

`--encrypt` uses AES-256-GCM under `~/.screendoc/key` (0600). This
protects the store against *other* users and against someone reading a
copy of the session directory, not against an attacker who also
compromised the key file.

**Mitigations:** 0600 key file, 0700 session dirs; the key never leaves
the machine and is not embedded in any artifact. **Residual:** full
host compromise defeats encryption by definition.

### T3 — Command injection via recorded content

Rendered video stills embed frame text; the web player re-renders the
cast in a sandboxed iframe. A recorded terminal can contain arbitrary
bytes, including HTML if you ever recorded a browser.

**Mitigations:** the web player exports a *self-contained* page with the
cast embedded as JSON (never raw HTML), xterm.js renders it as terminal
text, and `redact` can strip hostile payloads from the stored stream
before any export. **Residual:** a recorded page-viewer session will
render that viewer's content in the player — expected behavior.

### T4 — Path traversal / argument injection in the ffmpeg pipeline

`make_video` writes frames to a per-output temp dir and passes output
paths to ffmpeg.

**Mitigations:** temp dirs are private (`mktemp`-style unique names),
output filenames are derived from a sanitized session id
(alphanumeric + `-` only), and ffmpeg arguments are fixed vectors with
values appended positionally — no user string is ever joined into a
shell command. **Residual:** none known.

### T5 — Regex DoS in redaction

`redact` takes user-supplied regular expressions.

**Mitigations:** patterns are compiled once per rule and run by `regex`
(the crate is guaranteed linear-time on its class of patterns;
exponential backtracking is structurally impossible). **Residual:** a
pathologically large *replacement* or input still costs memory linear in
the input — expected.

### T6 — Unbounded growth of the session store

Long-running ambient recordings grow `raw.cast` linearly.

**Mitigations:** `idle_limit` compression at render time, `prune` +
`retention_days` for automatic expiry, and `screendoc status` showing
per-session size. **Residual:** a session left recording indefinitely
untils the disk — that is the user's filesystem to monitor.

## Out of scope

- Network attacks (the web server binds `127.0.0.1` only and is meant to
  be closed after use).
- Side channels (timing of render, font metrics) — no claims made.
- Multi-user session sharing — use the exported artifacts, not the store.

## Reporting

See [SECURITY.md](../SECURITY.md): report suspected leaks or vulns via
the private security advisory process; do not open a public issue with a
working exploit.
