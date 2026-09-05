use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors that cross crate boundaries.
///
/// Each downstream crate defines its own error type and converts into this
/// where it needs to surface at the harness layer.
#[derive(Debug, Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("provider error: {0}")]
    Provider(String),

    #[error("tool `{tool}` failed: {message}")]
    Tool { tool: String, message: String },

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("cancelled")]
    Cancelled,

    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
