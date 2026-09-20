//! maned — call `.mnd` scripts in-process.
//!
//! Scaffold per MANEDRS_PLAN.md §4; blocked on the maned_lang C ABI and
//! `libmaned_c` (MANEDPY_PLAN.md Phases 0-2b). Everything is `unimplemented!`.

mod error;
mod response;
mod script;
#[cfg(feature = "tokio")]
mod async_;

pub use error::Error;
pub use response::{Response, Tensor};
pub use script::{Script, ScriptBuilder, SharedScript};

use std::path::Path;

const BLOCKED: &str =
    "maned-rs is a scaffold: libmaned_c does not exist yet (MANEDPY_PLAN.md Phases 0-2b)";

/// Load a script from a literal path.
pub fn load(_path: impl AsRef<Path>) -> Result<Script, Error> {
    unimplemented!("{BLOCKED}")
}

/// Search $MANEDPATH -> search_path() -> CWD -> ./mnd/; a miss names every
/// directory tried. No mtime cache, deliberately (§4.1).
pub fn find(_name: &str) -> Result<Script, Error> {
    unimplemented!("{BLOCKED}")
}

pub fn builder(_path: impl AsRef<Path>) -> ScriptBuilder {
    unimplemented!("{BLOCKED}")
}

/// Process-global (src/interp/parallel.cpp:16) — a free function, never on
/// ScriptBuilder. Beware multiplicative oversubscription with rayon
/// (MANEDRS_PLAN.md hazard 8).
pub fn set_kernel_threads(_n: i32) {
    unimplemented!("{BLOCKED}")
}

/// Close every live handle. `std::process::exit` and `panic = "abort"` skip
/// Drop, orphaning any bark-vm child (hazard 5) — call this first.
pub fn shutdown() {
    unimplemented!("{BLOCKED}")
}
