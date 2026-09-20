//! Script / SharedScript / ScriptBuilder — the thread-safety table as types
//! (MANEDRS_PLAN.md §4.3).

use crate::{Error, Response};
use serde::Serialize;

pub struct Script {
    _handle: *mut core::ffi::c_void,
}

// SAFETY: sound because the engine keeps no per-thread state that outlives a
// call — zero `thread_local` in src/ and include/, and parallel_rows joins
// before returning (src/interp/parallel.cpp:47). Guarded by the grep gate in
// the maned repo (MANEDRS_PLAN.md §6, gate 8).
unsafe impl Send for Script {}
// Deliberately !Sync: `&mut self` on run() serializes calls with no lock, and
// Arc<Script> does not compile — a routed handle cannot reach a second thread.

impl Script {
    pub fn run<T: Serialize>(&mut self, _payload: &T) -> Result<Response, Error> {
        unimplemented!("{}", crate::BLOCKED)
    }

    pub fn is_routed(&self) -> bool {
        unimplemented!("{}", crate::BLOCKED)
    }

    /// The only door to SharedScript; checks mnd_script_is_routed() as it opens.
    pub fn into_shared(self) -> Result<SharedScript, Error> {
        unimplemented!("{}", crate::BLOCKED)
    }
}

pub struct SharedScript {
    _inner: Script,
}

// SAFETY: only constructible when !is_routed(); concurrent handle() calls on a
// non-routed script are proven identical by tests/serve/test_serve.cpp:920.
unsafe impl Sync for SharedScript {}

impl SharedScript {
    pub fn run<T: Serialize>(&self, _payload: &T) -> Result<Response, Error> {
        unimplemented!("{}", crate::BLOCKED)
    }
}

pub struct ScriptBuilder {
    _private: (),
}
