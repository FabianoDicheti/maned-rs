// GENERATED FILE — destined to be produced by `bindgen` from maned_c.h and
// checked in, never hand-edited; drift is caught by a byte comparison against
// a fresh bindgen run (MANEDRS_PLAN.md §6, gate 2).
//
// Written by hand for now, because `bindgen` is not installed on the machine
// that produced this file. It is a faithful transcription of the frozen
// `include/maned_c.h` (MND_ABI_VERSION 1) in bindgen's own output shape — the
// same item order, the same `_unused: [u8; 0]` opaque struct, the same `u32`
// constants — so that the day gate 2 runs, the byte comparison has something
// honest to compare against. The layout half of the contract is asserted
// against `maned_c_abi.json` in lib.rs.
//
// Nine entry points, one struct, two flags, one version macro. That is the
// whole ABI.

use core::ffi::c_char;

pub const MND_ABI_VERSION: u32 = 1;

/// `MND_SYNTHESIZE_OUTPUTS` — return-only scripts become servable.
pub const MND_SYNTHESIZE_OUTPUTS: u32 = 1;
/// `MND_NO_VM_FALLBACK` — an unreachable device is a hard error.
pub const MND_NO_VM_FALLBACK: u32 = 2;

/// Opaque; owns one loaded script.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct mnd_script {
    _unused: [u8; 0],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct mnd_response {
    /// HTTP status, or 0 = the call itself failed (an escaped implementation
    /// exception — the body carries its message).
    pub status: i32,
    /// NUL-terminated; never NULL.
    pub content_type: *const c_char,
    /// `body_len` bytes; MAY CONTAIN NULs (images). NUL-terminated as a
    /// courtesy, but `body_len` is the truth — never `strlen` this.
    pub body: *const c_char,
    pub body_len: usize,
    /// JSON array of strings; `"[]"` if none.
    pub notes: *const c_char,
}

extern "C" {
    /// The running library's ABI generation.
    pub fn mnd_abi_version() -> i32;

    /// Engine version ("1.2.3"); static storage — never free it.
    pub fn mnd_version() -> *const c_char;

    /// Parse -> lint -> inline -> plan -> validate ONCE. NULL on failure with
    /// `*err` set (heap; `mnd_free_string`). `*notes`, when non-NULL on either
    /// outcome, is a heap JSON array of load warnings/notes. `err`/`notes` may
    /// be passed as NULL.
    pub fn mnd_open(
        path: *const c_char,
        flags: u32,
        err: *mut *mut c_char,
        notes: *mut *mut c_char,
    ) -> *mut mnd_script;

    /// NULL is a no-op.
    pub fn mnd_close(s: *mut mnd_script);

    /// One request -> one response, exactly as maned-serve would answer it.
    /// NEVER returns NULL — even out-of-memory returns a static response
    /// (status 0) that `mnd_response_free` recognizes.
    pub fn mnd_request(
        s: *mut mnd_script,
        method: *const c_char,
        target: *const c_char,
        content_type: *const c_char,
        body: *const c_char,
        body_len: usize,
    ) -> *mut mnd_response;

    /// Frees the struct AND every string it points at. NULL is a no-op.
    pub fn mnd_response_free(r: *mut mnd_response);

    /// NULL is a no-op.
    pub fn mnd_free_string(p: *mut c_char);

    /// PROCESS-GLOBAL interpreter worker-thread count.
    pub fn mnd_set_kernel_threads(n: i32);

    /// 1 when any flow routes to a device (`@device` / `host="local"`), else 0.
    pub fn mnd_script_is_routed(s: *const mnd_script) -> i32;
}
