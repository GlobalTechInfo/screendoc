//! Standalone web player: one self-contained `.html` file.
//!
//! `render --web` inlines the xterm.js distribution, the player template,
//! and the session's cast/events/meta data into a single HTML document,
//! so it can be opened from disk (or shared) with no server.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sdoc_core::Session;

/// The embedded xterm.js distribution.
const XTERM_JS: &[u8] = include_bytes!("../static/xterm/xterm.js");
/// The embedded xterm.css.
const XTERM_CSS: &[u8] = include_bytes!("../static/xterm/xterm.css");
/// The embedded fit addon.
const ADDON_FIT_JS: &[u8] = include_bytes!("../static/xterm/addon-fit.js");
/// The player page template (dual data source: fetch or embedded).
const INDEX_HTML: &str = include_str!("../static/player.html");

/// Produce a self-contained player HTML for `session` in `out_dir`.
pub fn render_web(session: &Session, out_dir: &Path) -> Result<PathBuf> {
    let meta = session.read_meta().context("read session metadata")?;
    let events = session.read_events().context("read session events")?;
    let cast_text = std::fs::read_to_string(session.cast_path()).context("read session cast")?;

    let payload = serde_json::json!({
        "castText": cast_text,
        "events": serde_json::to_value(&events).context("serialize events")?,
        "meta": serde_json::to_value(&meta).context("serialize meta")?,
    });
    // The payload is inlined into a <script> block; a literal `<` could end
    // the block early if a cast payload contains `</script>` — JSON has no
    // bare `<` outside string contents, so escaping every `<` as \u003c is
    // complete and safe.
    let payload_js = payload.to_string().replace('<', "\\u003c");

    let html = build_standalone(&payload_js);
    let path = out_dir.join(format!("{}.html", session.id()));
    std::fs::write(&path, html)?;
    Ok(path)
}

/// Assemble the standalone document from the shared player template.
fn build_standalone(payload_js: &str) -> String {
    let js = String::from_utf8_lossy(XTERM_JS).to_string();
    let css = String::from_utf8_lossy(XTERM_CSS).to_string();
    let fit = String::from_utf8_lossy(ADDON_FIT_JS).to_string();

    let mut html = INDEX_HTML.to_string();
    html = html.replace(
        "<link rel=\"stylesheet\" href=\"/xterm.css\">",
        &format!("<style>{}\n</style>", css.replace("</style>", "<\\/style>")),
    );
    html = html.replace(
        "<script src=\"/xterm.js\"></script>",
        &format!("<script>{}\n</script>", js.replace("</script>", "<\\/script>")),
    );
    html = html.replace(
        "<script src=\"/addon-fit.js\"></script>",
        &format!("<script>{}\n</script>", fit.replace("</script>", "<\\/script>")),
    );
    html = html.replace(
        "<!--SCREENDOC:EMBED-->",
        &format!("<script>window.__SCREENDOC__ = {payload_js};</script>"),
    );
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_marker_is_replaced() {
        let out = build_standalone("1");
        assert!(!out.contains("SCREENDOC:EMBED"));
        assert!(out.contains("window.__SCREENDOC__ = 1;"));
    }

    #[test]
    fn xterm_tags_are_inlined() {
        let out = build_standalone("1");
        assert!(!out.contains("<script src=\"/xterm.js\">"));
        assert!(!out.contains("<script src=\"/addon-fit.js\">"));
        assert!(!out.contains("/xterm.css"));
        assert!(out.contains("<style>"));
    }

    #[test]
    fn payload_escaping_neutralizes_script_close() {
        let payload = serde_json::json!({"castText": "evil</script><script>alert(1)</script>"});
        let js = payload.to_string().replace('<', "\\u003c");
        assert!(!js.contains("</script>"));
        // Round-trips through JSON parsing.
        let back: serde_json::Value = serde_json::from_str(&js).expect("valid JSON");
        assert_eq!(back, payload);
    }

    #[test]
    fn render_web_requires_a_real_session() {
        // Without a data root there is no session; the call must fail
        // cleanly rather than panic.
        let s = Session::open("definitely-missing").expect_err("no such session");
        assert!(matches!(s, sdoc_core::CoreError::SessionNotFound(_)));
    }
}
