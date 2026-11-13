use thiserror::Error;

/// Errors that can occur in screendoc core.
#[derive(Debug, Error)]
pub enum CoreError {
    /// An underlying I/O error.
    #[error("i/o error: {0}")]
    Io(#[source] std::io::Error),

    /// A JSON serialization or deserialization error.
    #[error("json error: {0}")]
    Json(#[source] serde_json::Error),

    /// The requested session does not exist.
    #[error("session not found: {0}")]
    SessionNotFound(String),

    /// No session is currently active.
    #[error("no active session; start one with `screendoc on` or `screendoc record`")]
    NoActiveSession,

    /// An inline marker failed to parse.
    #[error("malformed marker: {0}")]
    BadMarker(String),

    /// An encrypted session failed to decrypt (wrong key or corrupt data).
    #[error("decryption failed: {0}")]
    Decryption(String),

    /// A message-only error.
    #[error("{0}")]
    Other(String),
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// Convenience result type for screendoc core operations.
pub type Result<T> = std::result::Result<T, CoreError>;
