// Driver-neutral connection/query errors.
// Driver-specific conversions (e.g. from tiberius errors) live in each driver module.

use thiserror::Error;

/// Connection errors
#[derive(Error, Debug)]
pub enum ConnectionError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Connection not found: {0}")]
    NotFound(String),

    #[error("Connection pool error: {0}")]
    PoolError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Query execution error: {0}")]
    QueryError(String),

    #[error("Password expired. Please change your password using another tool properly.")]
    PasswordExpired,

    #[error("Timeout error")]
    Timeout,

    /// The query was cancelled while running (e.g. its connection was dropped).
    #[error("Query cancelled")]
    Cancelled,

    #[error("Unsupported database driver: {0}")]
    UnsupportedDriver(String),
}

impl<E: std::error::Error + 'static> From<bb8::RunError<E>> for ConnectionError {
    fn from(err: bb8::RunError<E>) -> Self {
        ConnectionError::PoolError(err.to_string())
    }
}
