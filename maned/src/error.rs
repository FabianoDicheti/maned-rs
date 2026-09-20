//! Error — hand-written Display/Error impls; no proc-macro dependency
//! (MANEDRS_PLAN.md §4.4). error_response bodies are plain text, not JSON;
//! branch on status before parsing, and keep the trailing newline out of
//! Display.

use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum Error {
    Load { path: PathBuf, message: String },
    Input { status: u16, body: String },
    Route { status: u16, body: String },
    Runtime { status: u16, body: String },
    Unsupported { body: String },
    /// status == 0 — an escaped C++ exception.
    Internal { body: String },
    Library(String),
    Json(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Load { path, message } => {
                write!(f, "failed to load {}: {}", path.display(), message.trim_end())
            }
            Error::Input { status, body } => write!(f, "input error ({status}): {}", body.trim_end()),
            Error::Route { status, body } => write!(f, "route error ({status}): {}", body.trim_end()),
            Error::Runtime { status, body } => write!(f, "runtime error ({status}): {}", body.trim_end()),
            Error::Unsupported { body } => write!(f, "unsupported: {}", body.trim_end()),
            Error::Internal { body } => write!(f, "internal error: {}", body.trim_end()),
            Error::Library(msg) => write!(f, "library error: {msg}"),
            Error::Json(e) => write!(f, "json error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}
