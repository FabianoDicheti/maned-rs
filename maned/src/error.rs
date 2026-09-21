//! Error — hand-written Display/Error impls; no proc-macro dependency
//! (MANEDRS_PLAN.md §4.4). error_response bodies are plain text, not JSON;
//! branch on status before parsing, and keep the trailing newline out of
//! Display.

use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum Error {
    Load {
        path: PathBuf,
        message: String,
    },
    Input {
        status: u16,
        body: String,
    },
    Route {
        status: u16,
        body: String,
    },
    Runtime {
        status: u16,
        body: String,
    },
    Unsupported {
        body: String,
    },
    /// status == 0 — an escaped C++ exception.
    Internal {
        body: String,
    },
    Library(String),
    Json(serde_json::Error),
}

impl Error {
    /// The status→variant map of §4.4. `src/serve/server.cpp:154` sets
    /// `r.body = message + "\n"` — **plain text, not JSON** — so the body
    /// arrives here as text and nothing parses it.
    ///
    /// Statuses the map does not name fall to `Runtime`: a 5xx the engine
    /// grows later is a runtime failure until someone says otherwise, and
    /// inventing a ninth variant for it would be worse than a slightly broad
    /// eighth.
    pub(crate) fn from_status(status: u16, body: String) -> Error {
        match status {
            // Distinct from every HTTP code, including the engine's own 5xx:
            // the call itself failed and the body carries the message.
            0 => Error::Internal { body },
            400 | 413 | 431 => Error::Input { status, body },
            404 | 405 => Error::Route { status, body },
            501 => Error::Unsupported { body },
            _ => Error::Runtime { status, body },
        }
    }

    /// A response-decoding failure (a missing output, a ragged tensor, a
    /// body that is not the JSON it claimed to be).
    ///
    /// Deliberately **not** a ninth variant: §4.4 fixes the enum at eight,
    /// and `serde::de::Error::custom` builds exactly the "this JSON was not
    /// the shape I needed" error `Error::Json` already exists to carry.
    pub(crate) fn decode(message: impl fmt::Display) -> Error {
        use serde::de::Error as _;
        Error::Json(serde_json::Error::custom(message))
    }

    /// The HTTP status this error came back with, when it came from a
    /// response at all. `Some(0)` is the escaped-exception channel.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Input { status, .. }
            | Error::Route { status, .. }
            | Error::Runtime { status, .. } => Some(*status),
            Error::Unsupported { .. } => Some(501),
            Error::Internal { .. } => Some(0),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Load { path, message } => {
                write!(
                    f,
                    "failed to load {}: {}",
                    path.display(),
                    message.trim_end()
                )
            }
            Error::Input { status, body } => {
                write!(f, "input error ({status}): {}", body.trim_end())
            }
            Error::Route { status, body } => {
                write!(f, "route error ({status}): {}", body.trim_end())
            }
            Error::Runtime { status, body } => {
                write!(f, "runtime error ({status}): {}", body.trim_end())
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mapped(status: u16) -> Error {
        Error::from_status(status, "boom\n".to_string())
    }

    #[test]
    fn statuses_map_to_the_variants_of_section_4_4() {
        assert!(matches!(mapped(400), Error::Input { status: 400, .. }));
        assert!(matches!(mapped(413), Error::Input { status: 413, .. }));
        assert!(matches!(mapped(431), Error::Input { status: 431, .. }));
        assert!(matches!(mapped(404), Error::Route { status: 404, .. }));
        assert!(matches!(mapped(405), Error::Route { status: 405, .. }));
        assert!(matches!(mapped(500), Error::Runtime { status: 500, .. }));
        assert!(matches!(mapped(501), Error::Unsupported { .. }));
        assert!(matches!(mapped(0), Error::Internal { .. }));
        // Unnamed statuses fall to Runtime rather than inventing a variant.
        assert!(matches!(mapped(503), Error::Runtime { status: 503, .. }));
    }

    #[test]
    fn display_trims_the_trailing_newline_but_the_body_keeps_it() {
        let e = mapped(400);
        assert_eq!(e.to_string(), "input error (400): boom");
        match e {
            Error::Input { body, .. } => assert_eq!(body, "boom\n"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn status_accessor_reports_the_channel() {
        assert_eq!(mapped(404).status(), Some(404));
        assert_eq!(mapped(501).status(), Some(501));
        assert_eq!(mapped(0).status(), Some(0));
        assert_eq!(Error::Library("no dylib".into()).status(), None);
    }

    #[test]
    fn load_display_names_the_path() {
        let e = Error::Load {
            path: PathBuf::from("/tmp/x.mnd"),
            message: "parse error\n".into(),
        };
        assert_eq!(e.to_string(), "failed to load /tmp/x.mnd: parse error");
    }
}
