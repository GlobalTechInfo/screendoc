use std::net::SocketAddr;

use anyhow::{Context, Result};
use axum::{
    Router,
    http::HeaderMap,
    response::{Html, Json},
    routing::get,
};
use sdoc_core::{Event, Meta, Mode, Session};

/// The embedded xterm.js distribution.
const XTERM_JS: &[u8] = include_bytes!("../static/xterm/xterm.js");
/// The embedded xterm.css.
const XTERM_CSS: &[u8] = include_bytes!("../static/xterm/xterm.css");
/// The embedded fit addon.
const ADDON_FIT_JS: &[u8] = include_bytes!("../static/xterm/addon-fit.js");
/// The embedded player page.
const INDEX_HTML: &str = include_str!("../static/player.html");

/// A running web player for a session.
pub struct Player {
    /// The bound address.
    pub addr: SocketAddr,
}

/// Serve a session's web player on `127.0.0.1:port`.
///
/// The server task is detached; the returned handle carries the bound address.
pub async fn serve(session: &Session, port: u16) -> Result<Player> {
    let app = build_router(session)?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .context("failed to bind local web player port")?;
    let addr = listener.local_addr().context("failed to determine bound address")?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    Ok(Player { addr })
}

/// The URL the player was bound to.
pub fn player_url(addr: &SocketAddr) -> String {
    format!("http://{addr}/")
}

fn byte_response(bytes: Vec<u8>, content_type: &str) -> (HeaderMap, Vec<u8>) {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", content_type.parse().expect("static header"));
    headers.insert("cache-control", "no-store".parse().expect("static header"));
    (headers, bytes)
}

fn build_router(session: &Session) -> Result<Router> {
    let cast_bytes = session.read_cast_text().unwrap_or_default().into_bytes();
    let events = session.read_events().unwrap_or_default();
    let meta = session.read_meta().unwrap_or_else(|_| Meta {
        id: session.id().to_string(),
        started_unix_ms: 0,
        shell: None,
        cwd: None,
        mode: Mode::Wrapper,
        video: false,
        pane_target: None,
        duration_ms: None,
        title: None,
        group: None,
    });

    let events_json = serde_json::to_value(&events).unwrap_or_default();
    let meta_json = serde_json::to_value(&meta).unwrap_or_default();
    let cast_owned = cast_bytes;
    let app = Router::new()
        .route("/", get(move || async move { Html(INDEX_HTML.to_string()) }))
        .route(
            "/xterm.js",
            get(move || async move {
                byte_response(cast_js_bytes(), "application/javascript; charset=utf-8")
            }),
        )
        .route(
            "/xterm.css",
            get(move || async move { byte_response(css_bytes(), "text/css; charset=utf-8") }),
        )
        .route(
            "/addon-fit.js",
            get(move || async move {
                byte_response(fit_bytes(), "application/javascript; charset=utf-8")
            }),
        )
        .route(
            "/cast",
            get(move || async move {
                byte_response(cast_owned, "application/octet-stream; charset=utf-8")
            }),
        )
        .route("/events", get(move || async move { Json(events_json) }))
        .route("/meta", get(move || async move { Json(meta_json) }))
        .fallback(get(fallback));
    Ok(app)
}

/// Copy of the embedded xterm.js bytes.
fn cast_js_bytes() -> Vec<u8> {
    XTERM_JS.to_vec()
}

/// Copy of the embedded xterm.css bytes.
fn css_bytes() -> Vec<u8> {
    XTERM_CSS.to_vec()
}

/// Copy of the embedded fit-addon bytes.
fn fit_bytes() -> Vec<u8> {
    ADDON_FIT_JS.to_vec()
}

/// 404 handler for unknown routes.
async fn fallback() -> axum::http::StatusCode {
    axum::http::StatusCode::NOT_FOUND
}

/// All chapter events in a session, in log order.
pub fn chapters(events: &[Event]) -> Vec<Event> {
    events.iter().filter(|e| matches!(e, Event::Chapter { .. })).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapters_filter() {
        let evs = vec![
            Event::SessionStart { at_ms: 0, shell: None, size: sdoc_core::Size::default() },
            Event::Chapter { at_ms: 10, title: "x".into(), source: sdoc_core::ChapterSource::Auto },
            Event::Command { at_ms: 20, seq: 1, exit_code: 0, command: "ls".into() },
        ];
        assert_eq!(chapters(&evs).len(), 1);
    }

    #[test]
    fn url_format() {
        let a: SocketAddr = "127.0.0.1:4999".parse().unwrap();
        assert_eq!(player_url(&a), "http://127.0.0.1:4999/");
    }
}
