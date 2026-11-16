//! The one error type: a message the step prints and exits 1 on.

use std::fmt;

#[derive(Debug)]
pub struct SyncError(pub String);

pub type Result<T> = std::result::Result<T, SyncError>;

impl SyncError {
    pub fn new(message: impl Into<String>) -> Self {
        SyncError(message.into())
    }
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SyncError {}

impl From<std::io::Error> for SyncError {
    fn from(err: std::io::Error) -> Self {
        SyncError(format!("I/O error: {err}"))
    }
}

impl From<serde_json::Error> for SyncError {
    fn from(err: serde_json::Error) -> Self {
        SyncError(format!("invalid JSON: {err}"))
    }
}
