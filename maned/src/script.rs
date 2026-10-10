//! Script / SharedScript / ScriptBuilder — the thread-safety table as types
//! (MANEDRS_PLAN.md §4.3).
//!
//! | | Safe? | Encoded as |
//! |---|---|---|
//! | Concurrent calls, one handle, non-routed | yes | [`SharedScript`], `Sync` |
//! | Concurrent calls, one handle, routed | **no** | `&mut self` on [`Script::run`] |
//! | Two handles, same routed device | **no** | `Script: !Sync` ⇒ `Arc<Script>: !Send` |

use crate::{multipart, Error, Response};
use maned_sys as sys;
use serde::Serialize;
use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

// ---------------------------------------------------------------------------
// The live-handle registry behind maned::shutdown() (hazard 5).
// ---------------------------------------------------------------------------

/// One script handle, owned jointly by its [`Script`] and the registry.
///
/// The pointer lives in an `AtomicPtr` so that "close it" is a swap-to-null:
/// whoever wins the swap does the `mnd_close` and everybody else sees NULL and
/// does nothing. That is what makes `shutdown()` and `Drop` racing each other
/// a non-event rather than a double free.
///
/// `active` counts in-flight `mnd_request` calls. The swap-to-null alone only
/// stops NEW calls; a [`SharedScript`] call already inside the engine on
/// another thread still holds the raw pointer, and an `mnd_close` under it
/// would be a use-after-free reachable from safe code. So `close()` DRAINS:
/// swap first (no new entrants), then wait for `active` to hit zero, then
/// free. The acquire side increments BEFORE re-checking the pointer, so the
/// two cannot miss each other: either the acquirer sees NULL and backs out,
/// or the closer sees the incremented count and waits.
struct Handle {
    ptr: AtomicPtr<sys::mnd_script>,
    active: AtomicUsize,
}

/// Proof of an in-flight call: holds the raw pointer and keeps `active`
/// non-zero until dropped, which is what `Handle::close` waits on.
struct CallGuard<'a> {
    handle: &'a Handle,
    raw: *mut sys::mnd_script,
}

impl Drop for CallGuard<'_> {
    fn drop(&mut self) {
        self.handle.active.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Handle {
    fn close(&self) {
        let p = self.ptr.swap(ptr::null_mut(), Ordering::SeqCst);
        if !p.is_null() {
            // Won the swap: no new call can enter. Drain the ones already in.
            // A call can legitimately run for seconds (a large matmul), so
            // this is a sleep loop, not a spin — and blocking here is the
            // point: shutdown-before-exit means "when the work has stopped",
            // and this is what makes that true rather than hoped.
            while self.active.load(Ordering::SeqCst) != 0 {
                std::thread::sleep(std::time::Duration::from_micros(500));
            }
            // SAFETY: the one and only close of this handle (the swap was
            // won), and no call is inside it (the drain completed).
            unsafe { sys::mnd_close(p) }
        }
    }

    fn acquire(&self) -> Result<CallGuard<'_>, Error> {
        // Increment FIRST, then re-check the pointer: if close() swapped
        // before our load we back out; if it swaps after, it sees our
        // increment and drains. Either way no call runs on freed memory.
        self.active.fetch_add(1, Ordering::SeqCst);
        let p = self.ptr.load(Ordering::SeqCst);
        if p.is_null() {
            self.active.fetch_sub(1, Ordering::SeqCst);
            return Err(Error::Library(
                "this script handle is already closed — maned::shutdown() closes \
                 every live handle, and a Script that outlives it is inert"
                    .to_string(),
            ));
        }
        Ok(CallGuard { handle: self, raw: p })
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// A Vec, not a HashMap, because `Vec::new` is const and the live-handle count
/// is single digits in every program that is not leaking.
static LIVE: Mutex<Vec<(u64, Arc<Handle>)>> = Mutex::new(Vec::new());

fn register(handle: Arc<Handle>) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut live) = LIVE.lock() {
        live.push((id, handle));
    }
    id
}

fn unregister(id: u64) {
    if let Ok(mut live) = LIVE.lock() {
        live.retain(|(other, _)| *other != id);
    }
}

/// Close every live handle. See [`crate::shutdown`] for the caveats.
pub(crate) fn close_all() {
    let handles: Vec<Arc<Handle>> = match LIVE.lock() {
        Ok(mut live) => live.drain(..).map(|(_, h)| h).collect(),
        Err(_) => return,
    };
    for h in handles {
        h.close();
    }
}

/// How many handles `shutdown()` would close right now. Test hook, and an
/// honest answer to "did I leak a Script?".
pub(crate) fn live_count() -> usize {
    LIVE.lock().map(|l| l.len()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Script
// ---------------------------------------------------------------------------

/// One loaded `.mnd` script: parsed, linted, inlined, planned and validated
/// once, then answering requests.
pub struct Script {
    id: u64,
    handle: Arc<Handle>,
    notes: Vec<String>,
    path: PathBuf,
    routed: bool,
    /// The `GET /health` document, minus `requests_served`, fetched at most
    /// once. The contract it describes is frozen by `mnd_open` — the plan is
    /// built there and never rebuilt — so re-asking would buy nothing but a
    /// round trip per `run_files` call.
    health: OnceLock<serde_json::Value>,
    _not_sync: RoutedDeviceChannelsAreSingleConnection,
}

/// One `in::` or `out::` entry of the script's contract, from `GET /health`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoSpec {
    pub name: String,
    /// `"json"`, `"csv"`, `"image"`, `"parquet"`, `"text"` — the `in::`/`out::`
    /// source or sink keyword. `""` when the engine did not name one.
    pub format: String,
    /// Whether an `in::` has a literal default in the script, so a payload may
    /// omit it. `None` on an `out::`, which cannot have one.
    pub default: Option<bool>,
}

/// Keeps [`Script`] `!Sync` (and `!Send`, which the `unsafe impl` below then
/// restores deliberately). Without it the `Arc<Handle>` field would make the
/// whole struct `Sync` by inference and §4.3's third row would stop being
/// enforced.
///
/// It is a named type rather than a bare `PhantomData<*const ()>` field for
/// two reasons: the name appears in the compiler error that rejects
/// `Arc<Script>`, so the diagnostic says why and not just no; and holding the
/// raw pointer directly keeps `core::marker`'s own source path out of that
/// error, which is what lets the `trybuild` expectations be portable.
// The field is never read, and that is the point: it exists so that auto-trait
// inference sees a raw pointer inside `Script`. Reading it would be the bug.
#[allow(dead_code)]
struct RoutedDeviceChannelsAreSingleConnection(*const ());

impl RoutedDeviceChannelsAreSingleConnection {
    const MARKER: Self = RoutedDeviceChannelsAreSingleConnection(ptr::null());
}

// SAFETY: moving a script handle between threads is sound because the engine
// keeps no per-thread state that outlives a call. Two facts, both true of the
// engine today and neither written down anywhere in it:
//   1. `grep -rn 'thread_local\|__thread' src/ include/` returns ZERO matches,
//      so no call leaves anything behind in the thread that made it;
//   2. the one place the interpreter spawns threads — `parallel_rows`,
//      src/interp/parallel.cpp:27-48 — joins every one of them at :47 before
//      returning, so no worker outlives the call either.
// Send is only the right to MOVE the handle; it is not the right to use it
// from two threads at once, which is what `&mut self` on run() and the absence
// of a `Sync` impl below deny. If someone later adds a thread_local cache or a
// persistent worker pool to the engine, nothing here breaks loudly: this crate
// compiles exactly as before and the unsoundness is silent and data-race
// shaped. The guard therefore lives where the change would be made — a gate in
// the maned repo asserting that grep stays at zero, naming MANEDRS_PLAN.md
// §4.3 as the reason (§6, gate 8).
unsafe impl Send for Script {}

// Deliberately NOT Sync. `&mut self` on run() serializes calls with no lock at
// all — the borrow checker gives for free what manedpy pays an uncontended
// mutex for — and `Arc<T>: Send` requires `T: Send + Sync`, so `Arc<Script>`
// does not compile and a routed handle physically cannot reach a second
// thread. into_shared() is the only door out, and it checks.

impl Script {
    fn adopt(raw: *mut sys::mnd_script, path: PathBuf, notes: Vec<String>) -> Script {
        let handle = Arc::new(Handle {
            ptr: AtomicPtr::new(raw),
            active: AtomicUsize::new(0),
        });
        // SAFETY: `raw` is a live handle from mnd_open; is_routed only reads.
        let routed = unsafe { sys::mnd_script_is_routed(raw) } != 0;
        let id = register(Arc::clone(&handle));
        Script {
            id,
            handle,
            notes,
            path,
            routed,
            health: OnceLock::new(),
            _not_sync: RoutedDeviceChannelsAreSingleConnection::MARKER,
        }
    }

    /// The path this script was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Warnings and notes `mnd_open` produced while loading — the JSON array
    /// the ABI hands back through its fourth out-param, parsed once.
    pub fn load_notes(&self) -> &[String] {
        &self.notes
    }

    /// `POST /run.json` with `payload` serialized as the body, and the status
    /// map of §4.4 applied: a non-2xx response becomes an `Err`.
    ///
    /// `&mut self` is the whole thread-safety story: concurrent calls on one
    /// handle are only safe for a non-routed script, so they are only
    /// reachable through [`SharedScript`].
    pub fn run<T: Serialize + ?Sized>(&mut self, payload: &T) -> Result<Response, Error> {
        run_on(&self.handle, payload)
    }

    /// The raw request channel: any method, any target, no status mapping.
    /// This is `mnd_request` with the strings owned for you — the response
    /// comes back whatever its status, which is what a conformance harness
    /// and anyone handling 4xx themselves needs.
    pub fn request(
        &mut self,
        method: &str,
        target: &str,
        content_type: Option<&str>,
        body: &[u8],
    ) -> Result<Response, Error> {
        request_on(&self.handle, method, target, content_type, body)
    }

    /// `POST /frame` — the `frame::` tables as a JSON document, status-mapped
    /// like [`run`](Script::run).
    ///
    /// The frame document **is** the body here; unlike `/run.json` the tables
    /// are not nested under a `frames` key.
    pub fn frame<T: Serialize + ?Sized>(&mut self, payload: &T) -> Result<Response, Error> {
        frame_on(&self.handle, payload)
    }

    /// `POST /frame.csv` — the same tables as CSV text, bytes as written.
    ///
    /// Returned undecoded: a `text` column carries raw bytes `0..255` and is
    /// never promised to be UTF-8.
    pub fn frame_csv<T: Serialize + ?Sized>(&mut self, payload: &T) -> Result<Vec<u8>, Error> {
        frame_csv_on(&self.handle, payload)
    }

    /// `POST /run` as multipart — **the file route**: raw bytes in, raw bytes
    /// out, one part per declared `out::`.
    ///
    /// This is the only way to get the engine's actual file output. `/run.json`
    /// renders an `out::` image as a nested integer array and a csv as a JSON
    /// tensor; here a PNG comes back as a PNG.
    ///
    /// Each entry of `files` is an `in::` name and the raw bytes for it — CSV
    /// text, a PNG, a parquet file — in whatever format that input declares.
    ///
    /// ```no_run
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut script = maned::load("score.mnd")?;
    /// let parts = script.run_files(&[("h0", std::fs::read("rows.csv")?)])?;
    /// std::fs::write("out.png", &parts["heatmap"])?;
    /// # Ok(()) }
    /// ```
    pub fn run_files<K: AsRef<str>, V: AsRef<[u8]>>(
        &mut self,
        files: &[(K, V)],
    ) -> Result<BTreeMap<String, Vec<u8>>, Error> {
        run_files_on(&self.handle, &self.health, files)
    }

    /// `GET /health` — the script's contract, fetched once and cached.
    ///
    /// `requests_served` is removed: it counts calls on this handle only, so
    /// it looks like a server metric and is not one.
    pub fn health(&self) -> Result<&serde_json::Value, Error> {
        health_on(&self.handle, &self.health)
    }

    /// The `in::` contract — what a payload may or must carry.
    pub fn inputs(&self) -> Result<Vec<IoSpec>, Error> {
        Ok(io_specs(self.health()?, "inputs"))
    }

    /// The response schema — the `out::` names [`run_files`](Script::run_files)
    /// returns parts for.
    ///
    /// Under `synthesize_outputs` this lists the *synthesized* names, because
    /// synthesis happens in the engine's own `script.outputs`, so this and
    /// what `run` returns cannot disagree.
    pub fn outputs(&self) -> Result<Vec<IoSpec>, Error> {
        Ok(io_specs(self.health()?, "outputs"))
    }

    /// The routes this script actually answers, e.g. `"POST /frame.csv"` only
    /// when it declares a `frame::`.
    pub fn routes(&self) -> Result<Vec<String>, Error> {
        Ok(self
            .health()?
            .get("routes")
            .and_then(|r| r.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// 1 when any flow routes to a device (`@device` / `host="local"`).
    ///
    /// Read once at open, because it cannot change afterwards — the plan is
    /// built by `mnd_open` and frozen.
    pub fn is_routed(&self) -> bool {
        self.routed
    }

    /// The only door to [`SharedScript`], and it checks
    /// `mnd_script_is_routed()` at the moment it opens.
    ///
    /// A routed script comes back **inside the error**, so refusing to share
    /// it does not also destroy it: `mnd_open` is the expensive call, and a
    /// caller who discovers their script is routed should be able to keep
    /// using it single-threaded.
    pub fn into_shared(self) -> Result<SharedScript, RoutedScriptError> {
        if self.routed {
            return Err(RoutedScriptError { script: self });
        }
        Ok(SharedScript { inner: self })
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        unregister(self.id);
        self.handle.close();
    }
}

impl std::fmt::Debug for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Script")
            .field("path", &self.path)
            .field("routed", &self.routed)
            .field("closed", &self.handle.ptr.load(Ordering::Relaxed).is_null())
            .finish()
    }
}

/// `into_shared()` refused: the script routes to a device, and a device
/// channel is single-connection. The script itself is in here — take it back
/// with [`RoutedScriptError::into_script`].
#[derive(Debug)]
pub struct RoutedScriptError {
    script: Script,
}

impl RoutedScriptError {
    /// Recover the script the refusal was about.
    pub fn into_script(self) -> Script {
        self.script
    }

    pub fn path(&self) -> &Path {
        self.script.path()
    }
}

impl std::fmt::Display for RoutedScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} routes to a device, so it cannot be shared between threads: a \
             routed script drives single-connection device channels, and two \
             handles routing to the same device race that device's listener. \
             Keep the Script and call it from one thread, or put it behind a \
             Mutex and accept that you have serialized it.",
            self.script.path().display()
        )
    }
}

impl std::error::Error for RoutedScriptError {}

impl From<RoutedScriptError> for Error {
    fn from(e: RoutedScriptError) -> Error {
        Error::Library(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// SharedScript
// ---------------------------------------------------------------------------

/// A script proven non-routed, and therefore callable from many threads at
/// once with no lock.
pub struct SharedScript {
    inner: Script,
}

// SAFETY: only constructible through Script::into_shared(), which refuses
// unless mnd_script_is_routed() == 0. The ABI's rule is exact: "Concurrent
// mnd_request calls on ONE handle are safe if and only if
// mnd_script_is_routed(s) == 0", and tests/serve/test_serve.cpp:920
// (testConcurrentHandleIsIdenticalGap025) proves 8x25 concurrent handle()
// calls on a non-routed script return identical bytes. `routed` is read once
// at open and frozen, so the check cannot go stale: mnd_open builds the plan
// and nothing afterwards can add a device to it. The Send half of this type
// comes from Script's own unsafe impl Send above, which carries the
// no-thread_local / parallel_rows-joins argument; this impl adds only the
// "many threads at once" half, and it is the routed check that earns it.
unsafe impl Sync for SharedScript {}

impl SharedScript {
    /// `POST /run.json`, from as many threads as you like.
    pub fn run<T: Serialize + ?Sized>(&self, payload: &T) -> Result<Response, Error> {
        run_on(&self.inner.handle, payload)
    }

    /// The raw request channel; see [`Script::request`].
    pub fn request(
        &self,
        method: &str,
        target: &str,
        content_type: Option<&str>,
        body: &[u8],
    ) -> Result<Response, Error> {
        request_on(&self.inner.handle, method, target, content_type, body)
    }

    /// `POST /frame`; see [`Script::frame`].
    pub fn frame<T: Serialize + ?Sized>(&self, payload: &T) -> Result<Response, Error> {
        frame_on(&self.inner.handle, payload)
    }

    /// `POST /frame.csv`; see [`Script::frame_csv`].
    pub fn frame_csv<T: Serialize + ?Sized>(&self, payload: &T) -> Result<Vec<u8>, Error> {
        frame_csv_on(&self.inner.handle, payload)
    }

    /// `POST /run` as multipart, from as many threads as you like; see
    /// [`Script::run_files`].
    pub fn run_files<K: AsRef<str>, V: AsRef<[u8]>>(
        &self,
        files: &[(K, V)],
    ) -> Result<BTreeMap<String, Vec<u8>>, Error> {
        run_files_on(&self.inner.handle, &self.inner.health, files)
    }

    /// `GET /health`, fetched once and cached; see [`Script::health`].
    pub fn health(&self) -> Result<&serde_json::Value, Error> {
        self.inner.health()
    }

    /// See [`Script::inputs`].
    pub fn inputs(&self) -> Result<Vec<IoSpec>, Error> {
        self.inner.inputs()
    }

    /// See [`Script::outputs`].
    pub fn outputs(&self) -> Result<Vec<IoSpec>, Error> {
        self.inner.outputs()
    }

    /// See [`Script::routes`].
    pub fn routes(&self) -> Result<Vec<String>, Error> {
        self.inner.routes()
    }

    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    pub fn load_notes(&self) -> &[String] {
        self.inner.load_notes()
    }

    /// Always `false` — that is why this type exists.
    pub fn is_routed(&self) -> bool {
        false
    }

    /// Back to the exclusive handle.
    pub fn into_script(self) -> Script {
        self.inner
    }
}

impl std::fmt::Debug for SharedScript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SharedScript").field(&self.inner).finish()
    }
}

// ---------------------------------------------------------------------------
// The two call paths, shared by both types.
// ---------------------------------------------------------------------------

fn run_on<T: Serialize + ?Sized>(handle: &Handle, payload: &T) -> Result<Response, Error> {
    let body = serde_json::to_vec(payload).map_err(Error::Json)?;
    request_on(handle, "POST", "/run.json", None, &body)?.into_result()
}

fn request_on(
    handle: &Handle,
    method: &str,
    target: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> Result<Response, Error> {
    let guard = handle.acquire()?;
    let c_method = cstring(method, "method")?;
    let c_target = cstring(target, "target")?;
    let c_type = content_type
        .map(|t| cstring(t, "content_type"))
        .transpose()?;
    let type_ptr = c_type.as_ref().map_or(ptr::null(), |c| c.as_ptr());
    // The ABI takes NULL for an empty body; an empty slice's pointer is
    // dangling-but-aligned, which is fine in Rust and not worth betting a
    // foreign library's `if (body)` on.
    let (body_ptr, body_len) = if body.is_empty() {
        (ptr::null(), 0)
    } else {
        (body.as_ptr() as *const std::ffi::c_char, body.len())
    };

    // SAFETY: `guard` proves the handle was live when we entered AND keeps
    // `Handle::close` (Drop or maned::shutdown, either thread) parked in its
    // drain loop until the guard drops - so the pointer cannot be freed under
    // this call. Every string is NUL-terminated and outlives the call;
    // body_len is the length of the slice we passed.
    let resp = unsafe {
        sys::mnd_request(
            guard.raw,
            c_method.as_ptr(),
            c_target.as_ptr(),
            type_ptr,
            body_ptr,
            body_len,
        )
    };
    drop(guard);
    Response::from_raw(resp)
}

fn frame_on<T: Serialize + ?Sized>(handle: &Handle, payload: &T) -> Result<Response, Error> {
    let body = serde_json::to_vec(payload).map_err(Error::Json)?;
    request_on(handle, "POST", "/frame", None, &body)?.into_result()
}

fn frame_csv_on<T: Serialize + ?Sized>(handle: &Handle, payload: &T) -> Result<Vec<u8>, Error> {
    let body = serde_json::to_vec(payload).map_err(Error::Json)?;
    let response = request_on(handle, "POST", "/frame.csv", None, &body)?.into_result()?;
    Ok(response.body().to_vec())
}

fn run_files_on<K: AsRef<str>, V: AsRef<[u8]>>(
    handle: &Handle,
    cache: &OnceLock<serde_json::Value>,
    files: &[(K, V)],
) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    let boundary = multipart::boundary_for(files);
    let body = multipart::encode(&boundary, files);
    let content_type = format!("multipart/form-data; boundary={boundary}");
    let response = request_on(handle, "POST", "/run", Some(&content_type), &body)?.into_result()?;

    let answered = response.content_type();
    if answered.to_ascii_lowercase().starts_with("multipart/") {
        return multipart::decode(answered, response.body());
    }
    // A single-output script answers with the bare file and no part name at
    // all, so the one declared out:: name is the key. Anything else would
    // make the return type depend on how many outputs the script happens to
    // have, which is exactly what a caller cannot branch on in advance.
    let names = io_specs(health_on(handle, cache)?, "outputs");
    let key = match names.as_slice() {
        [only] => only.name.clone(),
        _ => "body".to_string(),
    };
    let mut parts = BTreeMap::new();
    parts.insert(key, response.body().to_vec());
    Ok(parts)
}

fn health_on<'a>(
    handle: &Handle,
    cache: &'a OnceLock<serde_json::Value>,
) -> Result<&'a serde_json::Value, Error> {
    if let Some(document) = cache.get() {
        return Ok(document);
    }
    let response = request_on(handle, "GET", "/health", None, &[])?.into_result()?;
    let mut document: serde_json::Value =
        serde_json::from_slice(response.body()).map_err(Error::Json)?;
    if let Some(object) = document.as_object_mut() {
        object.remove("requests_served");
    }
    // Two threads racing here both fetch and one set() wins; the loser drops
    // an identical document. A Mutex would serialize a call that is idempotent
    // and cheap, and get_or_try_init is not stable on this MSRV.
    let _ = cache.set(document);
    Ok(cache
        .get()
        .expect("OnceLock was just set and is never cleared"))
}

fn io_specs(document: &serde_json::Value, key: &str) -> Vec<IoSpec> {
    document
        .get(key)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(IoSpec {
                        name: item.get("name")?.as_str()?.to_string(),
                        format: item
                            .get("format")
                            .and_then(|f| f.as_str())
                            .unwrap_or("")
                            .to_string(),
                        default: item.get("default").and_then(|d| d.as_bool()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn cstring(s: &str, what: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| {
        Error::Library(format!(
            "{what} contains an interior NUL byte, which a C string cannot carry"
        ))
    })
}

// ---------------------------------------------------------------------------
// ScriptBuilder
// ---------------------------------------------------------------------------

/// Flags for `mnd_open`. `maned::load(path)` is `builder(path).open()`.
///
/// There is deliberately no `threads()` here: `mnd_set_kernel_threads` is
/// process-global (`src/interp/parallel.cpp:16`, one `std::atomic<int>`), so
/// it is a free function and nobody can write `builder.threads(4)` and expect
/// isolation (hazard 8).
#[derive(Debug, Clone)]
pub struct ScriptBuilder {
    path: PathBuf,
    flags: u32,
}

impl ScriptBuilder {
    pub(crate) fn new(path: PathBuf) -> ScriptBuilder {
        ScriptBuilder { path, flags: 0 }
    }

    /// `MND_SYNTHESIZE_OUTPUTS` — make a return-only script servable by
    /// synthesizing outputs for its returned values.
    pub fn synthesize_outputs(mut self, yes: bool) -> ScriptBuilder {
        self.set(sys::MND_SYNTHESIZE_OUTPUTS, yes);
        self
    }

    /// `MND_NO_VM_FALLBACK` — an unreachable device is a hard error instead of
    /// falling back to the local VM.
    pub fn no_vm_fallback(mut self, yes: bool) -> ScriptBuilder {
        self.set(sys::MND_NO_VM_FALLBACK, yes);
        self
    }

    /// The raw `mnd_open` flag word, as it will be passed.
    pub fn flags(&self) -> u32 {
        self.flags
    }

    fn set(&mut self, bit: u32, yes: bool) {
        if yes {
            self.flags |= bit;
        } else {
            self.flags &= !bit;
        }
    }

    /// Parse -> lint -> inline -> plan -> validate, once.
    pub fn open(self) -> Result<Script, Error> {
        let c_path = CString::new(self.path.as_os_str().as_bytes()).map_err(|_| Error::Load {
            path: self.path.clone(),
            message: "path contains an interior NUL byte".to_string(),
        })?;

        let mut err: *mut std::ffi::c_char = ptr::null_mut();
        let mut notes: *mut std::ffi::c_char = ptr::null_mut();

        // SAFETY: c_path is NUL-terminated and outlives the call; err/notes
        // are valid out-params that we drain with mnd_free_string below,
        // exactly as ownership rule 1 requires.
        let raw = unsafe { sys::mnd_open(c_path.as_ptr(), self.flags, &mut err, &mut notes) };

        let load_notes = take_owned_string(notes)
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_default();
        let message = take_owned_string(err);

        if raw.is_null() {
            return Err(Error::Load {
                path: self.path.clone(),
                message: with_cwd_clause(
                    message
                        .unwrap_or_else(|| "mnd_open returned NULL without a message".to_string()),
                ),
            });
        }
        Ok(Script::adopt(raw, self.path, load_notes))
    }
}

/// Hazard 6: `src/serve/server.cpp:1058-1078` hands `in::w = csv("weights.csv")`
/// to `io::readCsvFile` verbatim, so relative data paths resolve against the
/// **process** working directory. Rust can `set_current_dir`, unlike the JVM —
/// and must not: it is process-global and races every other thread, including
/// the ones `parallel_rows` spawns. So the clarifying clause goes in the load
/// error instead, where somebody will read it.
fn with_cwd_clause(message: String) -> String {
    match std::env::current_dir() {
        Ok(cwd) => format!(
            "{}\n(note: relative paths inside a .mnd resolve against the process \
             working directory, currently {} — prefer absolute paths in the script)",
            message.trim_end(),
            cwd.display()
        ),
        Err(_) => message,
    }
}

/// Take ownership of a `char*` out-param and hand it back to
/// `mnd_free_string` — ownership rule 1. The caller NEVER calls `free()`.
fn take_owned_string(p: *mut std::ffi::c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a heap NUL-terminated string the ABI just gave us, freed once,
    // here, with the library's own deallocator.
    let owned = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { sys::mnd_free_string(p) };
    Some(owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_flags_map_to_the_header_bits() {
        let b = ScriptBuilder::new(PathBuf::from("x.mnd"));
        assert_eq!(b.flags(), 0);
        assert_eq!(b.clone().synthesize_outputs(true).flags(), 1);
        assert_eq!(b.clone().no_vm_fallback(true).flags(), 2);
        assert_eq!(
            b.clone()
                .synthesize_outputs(true)
                .no_vm_fallback(true)
                .flags(),
            3
        );
        // The setters are idempotent and reversible, so a builder can be
        // configured from a config file without the caller tracking state.
        assert_eq!(
            b.clone()
                .synthesize_outputs(true)
                .synthesize_outputs(false)
                .flags(),
            0
        );
    }

    #[test]
    fn interior_nul_is_refused_rather_than_truncated() {
        let e = cstring("POST\0GET", "method").unwrap_err();
        assert!(e.to_string().contains("interior NUL"), "{e}");
    }

    // The soundness contract behind `unsafe impl Sync for SharedScript`:
    // close() must DRAIN in-flight calls, not free under them. Deterministic:
    // the guard is held by hand, so the closer is provably parked in its
    // drain loop until release - no timing-dependent request needed.
    #[test]
    fn close_drains_in_flight_calls_before_freeing() {
        let dir = std::env::temp_dir().join(format!(
            "maned_rs_drain_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("drain.mnd");
        std::fs::write(
            &path,
            "mnd::quantmax=30000;\nmnd::quantmin=-30000;\nmnd::quantres=0;\n\
             calc::lambda_flow f(x) {\n    x relu y =\n} return y;\n",
        )
        .unwrap();
        let script = crate::builder(&path).synthesize_outputs(true).open().unwrap();
        let shared = script.into_shared().unwrap();

        let guard = shared.inner.handle.acquire().expect("live handle");
        let closer = Arc::clone(&shared.inner.handle);
        let t = std::thread::spawn(move || closer.close());
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            !t.is_finished(),
            "close() returned while a call guard was outstanding"
        );
        drop(guard); // the in-flight call ends...
        t.join().unwrap(); // ...and only now does close() complete.

        // The handle is inert afterwards, and a late acquire backs out
        // cleanly (its increment must not wedge a future closer).
        assert!(shared.inner.handle.acquire().is_err());
        assert_eq!(shared.inner.handle.active.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
