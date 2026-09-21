//! maned — call `.mnd` scripts in-process.
//!
//! One entry point in the engine, [`mnd_request`](maned_sys::mnd_request),
//! answers the same request→response contract as `maned-serve`'s REST API
//! with no socket and no HTTP framing. This crate is the safe face of it
//! (MANEDRS_PLAN.md §4).
//!
//! ```no_run
//! let mut scoring = maned::load("scoring.mnd")?;
//! let result = scoring.run(&serde_json::json!({
//!     "x": [[2, 0], [0, 2]],
//!     "w": [[3, 4], [5, 6]],
//! }))?;
//! let y = result.tensor("y")?;        // Tensor { shape: [2, 2], data: [...] }
//! # Ok::<(), maned::Error>(())
//! ```
//!
//! # The library has to be installed first
//!
//! A `.crate` cannot ship per-platform binaries, so `cargo add maned` is not
//! enough: install the `maned-capi-$VERSION-$target` tarball, or point
//! `MANED_LIB_DIR` at an unpacked one (§4.5). `maned-sys`'s build script says
//! so, at length, if it cannot find the library.
//!
//! # Threads
//!
//! [`Script`] is `Send` and **not** `Sync`: `run` takes `&mut self`, so
//! concurrent calls on one handle are impossible and `Arc<Script>` does not
//! compile. [`Script::into_shared`] checks `mnd_script_is_routed()` and, on a
//! non-routed script, hands back a [`SharedScript`] that *is* `Sync` and can
//! be called from as many threads as you like with no lock at all (§4.3).
//!
//! # Non-goals
//!
//! Windows (the engine's serve and net layers are POSIX throughout) and
//! cancellation (calls are uninterruptible; see `SharedScript::run_async`'s
//! documentation, under the `tokio` feature).

#![cfg_attr(docsrs, feature(doc_cfg))]

mod error;
mod response;
mod script;

#[cfg(feature = "tokio")]
mod async_;

pub use error::Error;
pub use response::{Response, Tensor};
pub use script::{RoutedScriptError, Script, ScriptBuilder, SharedScript};

use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Load a script from a literal path.
///
/// This is `builder(path).open()`. There is deliberately **no cache keyed on
/// mtime**: manedpy needs one because a REPL holds a module across edits, but
/// a Rust program that wants a reload can call `load` again, and a hidden
/// global cache inside a `Send`-critical crate is a liability with no matching
/// benefit (§4.1).
pub fn load(path: impl AsRef<Path>) -> Result<Script, Error> {
    builder(path).open()
}

/// Find a script by name: `$MANEDPATH` → [`search_path`] → CWD → `./mnd/`,
/// identical to manedpy.
///
/// A bare name gets `.mnd` appended if it has no extension, and both
/// spellings are tried in each directory. A miss names **every directory
/// tried** — the refusal message is the API contract, same house style as
/// `serve/http.h`.
pub fn find(name: &str) -> Result<Script, Error> {
    let mut tried: Vec<String> = Vec::new();
    for dir in search_dirs() {
        for candidate in candidate_names(name) {
            let path = dir.join(&candidate);
            if path.is_file() {
                return load(path);
            }
            tried.push(path.display().to_string());
        }
    }
    Err(Error::Load {
        path: PathBuf::from(name),
        message: format!(
            "no script named '{name}'. Tried, in order:\n  {}\n\
             (search order: $MANEDPATH, maned::search_path(), the working \
             directory, ./mnd/)",
            tried.join("\n  ")
        ),
    })
}

/// Start building a script handle: flags, then
/// [`open`](ScriptBuilder::open).
pub fn builder(path: impl AsRef<Path>) -> ScriptBuilder {
    ScriptBuilder::new(path.as_ref().to_path_buf())
}

/// Process-global interpreter worker-thread count
/// (`src/interp/parallel.cpp:16`, one `std::atomic<int>`; setter at `:24`).
///
/// A free function, never a [`ScriptBuilder`] method, because there is one
/// setting per process and it changes timing, never values — nobody should be
/// able to write `builder.threads(4)` and expect isolation.
///
/// **Rust-specific footgun**: a program already running `rayon` at
/// `num_cpus` that also calls `set_kernel_threads(num_cpus)` oversubscribes
/// multiplicatively, and the symptom is a slowdown with no error anywhere
/// (hazard 8).
pub fn set_kernel_threads(n: i32) {
    // SAFETY: a store to one process-global atomic; no handle involved.
    unsafe { maned_sys::mnd_set_kernel_threads(n) }
}

/// Close every live [`Script`] handle.
///
/// `std::process::exit` skips every destructor, so `mnd_close` never runs, the
/// `VmChannel` destructor (`src/net/local_vm.cpp:224-246`) never runs, and a
/// `bark-vm` child is left orphaned holding a pipe. Rust's `std` has no
/// `atexit`, so this is the hook (hazard 5).
///
/// **Caveats, plainly:**
/// - `std::process::exit` **bypasses this**. Call it yourself, first.
/// - A release profile with `panic = "abort"` turns every panic into the same
///   leak, for the same reason.
/// - Handles closed here become inert, not dangling: a [`Script`] that
///   outlives the call returns an error from `run` instead of touching freed
///   memory, and its own `Drop` becomes a no-op.
/// - It must not race an **in-flight** call. Closing a handle another thread
///   is currently inside is a use-after-free, and no amount of atomics on this
///   side can fix that — the fix is to call this when the work has stopped,
///   which is what "before `process::exit`" means in practice.
pub fn shutdown() {
    script::close_all();
}

/// How many script handles [`shutdown`] would close right now.
pub fn live_handles() -> usize {
    script::live_count()
}

/// The ABI generation of the library actually linked in.
///
/// Compare as a **range**, never `== N`: the stability policy in `maned_c.h`
/// promises that adding a route or a flag never bumps this, and that version
/// `N-1` stays accepted for one minor release cycle.
pub fn abi_version() -> i32 {
    unsafe { maned_sys::mnd_abi_version() }
}

/// The engine version of the library actually linked in ("1.2.3").
pub fn version() -> &'static str {
    // SAFETY: static storage owned by the library; never freed, never moved.
    unsafe { CStr::from_ptr(maned_sys::mnd_version()) }
        .to_str()
        .unwrap_or("<non-utf8>")
}

// ---------------------------------------------------------------------------
// The search path
// ---------------------------------------------------------------------------

/// The process-global extra directories [`find`] searches, after `$MANEDPATH`
/// and before the working directory.
///
/// This is the Rust answer to Kotlin's `Maned.io["scoring"]` and Python's
/// `__getattr__` sugar: neither has an analogue here, so the sugar is a
/// function. It is process-global — deliberately, since `$MANEDPATH` is too —
/// and returns a snapshot rather than a guard, so holding it cannot deadlock
/// a `find` on another thread.
pub fn search_path() -> Vec<PathBuf> {
    SEARCH_PATH.lock().map(|p| p.clone()).unwrap_or_default()
}

/// Append a directory to [`search_path`].
pub fn add_search_path(dir: impl AsRef<Path>) {
    if let Ok(mut p) = SEARCH_PATH.lock() {
        p.push(dir.as_ref().to_path_buf());
    }
}

/// Replace [`search_path`] wholesale.
pub fn set_search_path<I, P>(dirs: I)
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    if let Ok(mut p) = SEARCH_PATH.lock() {
        *p = dirs.into_iter().map(|d| d.as_ref().to_path_buf()).collect();
    }
}

static SEARCH_PATH: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("MANEDPATH") {
        dirs.extend(std::env::split_paths(&p).filter(|d| !d.as_os_str().is_empty()));
    }
    dirs.extend(search_path());
    dirs.push(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    dirs.push(PathBuf::from("mnd"));
    dirs
}

fn candidate_names(name: &str) -> Vec<PathBuf> {
    let p = Path::new(name);
    if p.extension().is_some() {
        vec![p.to_path_buf()]
    } else {
        vec![p.with_extension("mnd"), p.to_path_buf()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_names_appends_mnd_only_when_there_is_no_extension() {
        assert_eq!(
            candidate_names("scoring"),
            vec![PathBuf::from("scoring.mnd"), PathBuf::from("scoring")]
        );
        assert_eq!(
            candidate_names("scoring.mnd"),
            vec![PathBuf::from("scoring.mnd")]
        );
        assert_eq!(candidate_names("a/b.mnd"), vec![PathBuf::from("a/b.mnd")]);
    }

    #[test]
    fn a_miss_names_every_directory_tried() {
        let e = find("definitely-not-a-script-9d3f").unwrap_err();
        let m = e.to_string();
        assert!(m.contains("definitely-not-a-script-9d3f.mnd"), "{m}");
        assert!(m.contains("mnd/definitely-not-a-script-9d3f.mnd"), "{m}");
        assert!(m.contains("search order"), "{m}");
    }

    #[test]
    fn the_linked_library_is_the_one_we_were_compiled_against() {
        assert!(
            maned_sys::abi_version_supported(abi_version()),
            "linked libmaned_c reports ABI {}",
            abi_version()
        );
        assert!(!version().is_empty());
    }
}
