use std::fmt::Write as _;

/// Shells that screendoc can generate hooks for.
pub const SUPPORTED_SHELLS: [&str; 3] = ["bash", "zsh", "fish"];

/// Detect the user's shell from the `SHELL` environment variable.
///
/// Falls back to `bash` when the variable is missing or unrecognized.
pub fn detect_shell() -> &'static str {
    let shell = std::env::var("SHELL").unwrap_or_default();
    detect_shell_from(&shell)
}

/// Pure helper: classify a `SHELL` value into a supported shell name.
///
/// Exposed separately so it can be tested without touching the process
/// environment.
pub fn detect_shell_from(shell: &str) -> &'static str {
    let name = std::path::Path::new(shell).file_name().and_then(|n| n.to_str()).unwrap_or("");
    match name {
        "zsh" => "zsh",
        "fish" => "fish",
        _ => "bash",
    }
}

/// The bash rc file used by the pty wrapper.
///
/// It prints inline OSC-90 markers (stripped by the demuxer) at every
/// prompt and on exit, so each command becomes a chapter.
pub fn bash_wrapper_rcfile() -> String {
    let mut s = String::new();
    let _ = s.write_str(
        r#"# screendoc wrapper rc file (generated)
PS1='\w \$ '
PS2='> '
export TERM=xterm-256color
__sdoc_seq=0
__sdoc_prompt_seen=0
__sdoc_precmd() {
    local rc=$?
    local cmd="$BASH_COMMAND"
    if [ -n "$cmd" ]; then
        __sdoc_seq=$(( __sdoc_seq + 1 ))
        printf '\033]90;cmd;%s;%s;%s\007' "$__sdoc_seq" "$rc" "$cmd"
    fi
    __sdoc_prompt_seen=1
}
PROMPT_COMMAND='__sdoc_precmd'
__sdoc_exit() {
    local rc=$?
    local cmd="$BASH_COMMAND"
    if [ "$__sdoc_prompt_seen" -eq 0 ] && [ -n "$cmd" ]; then
        __sdoc_seq=$(( __sdoc_seq + 1 ))
        printf '\033]90;cmd;%s;%s;%s\007' "$__sdoc_seq" "$rc" "$cmd"
    fi
}
trap '__sdoc_exit' EXIT
"#,
    );
    s
}

/// The `cmd.exe` batch driver for the Windows record wrapper.
///
/// The driver runs the recorded command line and then exits with that
/// command's error level, so the wrapper's `child.wait()` observes the real
/// exit code.
pub fn cmd_wrapper_driver(command_line: &str) -> String {
    let mut s = String::new();
    let _ = s.write_str("@echo off\r\n");
    let _ = s.write_str(command_line);
    let _ = s.write_str("\r\nexit /b %ERRORLEVEL%\r\n");
    s
}

/// The PowerShell one-liner for the Windows record wrapper.
///
/// Runs the recorded command line, captures its last exit code (native
/// commands set `$LASTEXITCODE`; a missing value means the command itself
/// failed, which we surface as exit code 1), and exits the shell with it so
/// the wrapper's `child.wait()` observes the real exit code.
pub fn powershell_wrapper_script(command_line: &str) -> String {
    format!(
        r#"{command_line}
if ($null -ne $LASTEXITCODE) {{
    exit $LASTEXITCODE
}}
if ($?) {{ exit 0 }} else {{ exit 1 }}"#
    )
}

/// A ready-to-`eval` ambient hook snippet for the given shell.
///
/// The snippet exports `SCREENDOC_SESSION` and registers hooks that append
/// structured markers to the session's marker log via `screendoc
/// hook-append`.
pub fn ambient_hook_script(shell: &str, session_id: &str) -> Option<String> {
    match shell {
        "bash" => Some(bash_ambient(session_id)),
        "zsh" => Some(zsh_ambient(session_id)),
        "fish" => Some(fish_ambient(session_id)),
        _ => None,
    }
}

fn notice(session_id: &str) -> String {
    format!("screendoc: recording session {session_id} in ambient mode — stop with `screendoc off`")
}

/// The bash ambient hook snippet.
///
/// Uses only `PROMPT_COMMAND` and the `EXIT` trap (no DEBUG trap), so it is
/// immune to the well-known bash DEBUG-trap prompt gotcha. Each command is
/// recorded as a complete marker when the next prompt is shown.
pub fn bash_ambient(session_id: &str) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        r#"
export SCREENDOC_SESSION="{session_id}"
__sdoc_seq=0
__sdoc_prompt_seen=0
__sdoc_precmd() {{
    local rc=$?
    local cmd="$BASH_COMMAND"
    if [ -n "$cmd" ]; then
        __sdoc_seq=$(( __sdoc_seq + 1 ))
        screendoc hook-append --cmd --seq "$__sdoc_seq" --rc "$rc" --text "$cmd" || true
    fi
    __sdoc_prompt_seen=1
}}
__sdoc_exit() {{
    local rc=$?
    local cmd="$BASH_COMMAND"
    if [ "$__sdoc_prompt_seen" -eq 0 ] && [ -n "$cmd" ]; then
        __sdoc_seq=$(( __sdoc_seq + 1 ))
        screendoc hook-append --cmd --seq "$__sdoc_seq" --rc "$rc" --text "$cmd" || true
    fi
}}
PROMPT_COMMAND='__sdoc_precmd'
trap '__sdoc_exit' EXIT"#,
        session_id = session_id
    );
    let _ = write!(s, "\necho \"{}\"", notice(session_id));
    s
}

/// The zsh ambient hook snippet.
pub fn zsh_ambient(session_id: &str) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        r#"
export SCREENDOC_SESSION="{session_id}"
__sdoc_seq=0
__sdoc_preexec_hook() {{
    __sdoc_seq=$(( __sdoc_seq + 1 ))
    screendoc hook-append --cmd-start --seq "$__sdoc_seq" --text "$1" || true
}}
__sdoc_precmd_hook() {{
    local rc=$?
    screendoc hook-append --cmd-end --seq "$__sdoc_seq" --rc "$rc" || true
}}
preexec_functions+=(__sdoc_preexec_hook)
precmd_functions+=(__sdoc_precmd_hook)"#,
        session_id = session_id
    );
    let _ = write!(s, "\necho \"{}\"", notice(session_id));
    s
}

/// The fish ambient hook snippet.
pub fn fish_ambient(session_id: &str) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        r#"
set -gx SCREENDOC_SESSION "{session_id}"
set -g __sdoc_seq 0
function __sdoc_preexec_hook --on-event fish_preexec
    set -g __sdoc_seq (math $__sdoc_seq + 1)
    screendoc hook-append --cmd-start --seq $__sdoc_seq --text "$argv[1]" | /dev/null
end
function __sdoc_postexec_hook --on-event fish_postexec
    screendoc hook-append --cmd-end --seq $__sdoc_seq --rc "$status" | /dev/null
end"#,
        session_id = session_id
    );
    let _ = write!(s, "\necho \"{}\"", notice(session_id));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_shell_classifies_values() {
        assert_eq!(detect_shell_from("/bin/bash"), "bash");
        assert_eq!(detect_shell_from("/usr/bin/zsh"), "zsh");
        assert_eq!(detect_shell_from("/usr/local/bin/fish"), "fish");
        assert_eq!(detect_shell_from("/opt/tcsh"), "bash");
        assert_eq!(detect_shell_from(""), "bash");
    }

    #[test]
    fn bash_rcfile_contains_markers() {
        let rc = bash_wrapper_rcfile();
        assert!(rc.contains("\\033]90;cmd;"));
        assert!(rc.contains("PROMPT_COMMAND"));
        assert!(rc.contains("trap"));
    }

    #[test]
    fn ambient_scripts_generated_for_all_shells() {
        for shell in SUPPORTED_SHELLS {
            let script = ambient_hook_script(shell, "abc").expect("script");
            assert!(script.contains("hook-append"), "shell={shell}");
            assert!(script.contains("SCREENDOC_SESSION"), "shell={shell}");
        }
        assert!(ambient_hook_script("tcsh", "abc").is_none());
    }

    #[test]
    fn bash_ambient_uses_prompt_command() {
        let script = bash_ambient("abc");
        assert!(script.contains("PROMPT_COMMAND='__sdoc_precmd'"));
        assert!(script.contains("hook-append --cmd"));
        assert!(script.contains("recording session abc"));
    }
}
