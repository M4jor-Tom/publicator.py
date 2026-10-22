use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid state: {0}")]
    InvalidState(String),
    #[error("busy: {0}")]
    Busy(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("prompt archive unavailable: {0}")]
    PromptArchiveUnavailable(String),
    #[error(
        "publication database commit at {path} may be visible but is not confirmed durable; refresh before retrying: {source}"
    )]
    DurabilityUncertain {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("reconciliation required: {0}")]
    ReconciliationRequired(String),
}
