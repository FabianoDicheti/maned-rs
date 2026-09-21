//! Response and Tensor (MANEDRS_PLAN.md §4.2).
//!
//! body() borrows the C allocation, zero copy — the one place this binding is
//! faster than both predecessors. body is bytes, never CStr: the ABI documents
//! body_len bytes that may contain NULs, and a CStr would truncate a PNG at
//! its first zero byte. decimal() hands back the engine's printed text
//! verbatim; never enable serde_json's arbitrary_precision from a library.

use crate::Error;
use maned_sys as sys;
use serde::de::DeserializeOwned;
use std::ffi::CStr;
use std::ptr::NonNull;

pub struct Response {
    raw: NonNull<sys::mnd_response>,
    /// Parsed EAGERLY, once, at construction. `notes` is a JSON array of
    /// strings that is `"[]"` in the overwhelming majority of responses, so
    /// the lazy version would buy a `OnceCell` and a `!Sync` obligation in
    /// exchange for skipping a two-byte parse.
    notes: Vec<String>,
}

// SAFETY: a Response owns one heap allocation and nothing else. It is not a
// handle: it holds no reference to the mnd_script it came from, no device
// channel, no file descriptor, and the library keeps no back-pointer to it —
// after mnd_request returns, the only two operations the ABI defines on it are
// "read its fields" and "mnd_response_free it exactly once", and neither is
// documented or implemented as thread-affine (mnd_response_free is a delete of
// a C++-allocated block, and the C++ global allocator is thread-safe by
// [basic.stc.dynamic.allocation]). Send is also load-bearing rather than
// decorative: §4.6's run_async returns the Response out of a spawn_blocking
// task, which requires it. The type is deliberately NOT Sync — &Response is
// read-only today, but nothing in the ABI promises that concurrent readers of
// one response stay safe if the struct ever grows an interior cache.
unsafe impl Send for Response {}

impl Response {
    /// Adopt the allocation `mnd_request` just handed us.
    ///
    /// The ABI says this is NEVER NULL — "even out-of-memory returns a static
    /// response (status 0)". Hazard 9: a NULL deref is UB in Rust, not a
    /// panic, so the check costs nothing and the trust costs a miscompile.
    pub(crate) fn from_raw(raw: *mut sys::mnd_response) -> Result<Response, Error> {
        let raw = NonNull::new(raw).ok_or_else(|| Error::Internal {
            body: "mnd_request returned NULL, which the ABI forbids".to_string(),
        })?;
        let notes = unsafe { parse_notes(raw.as_ref().notes) };
        Ok(Response { raw, notes })
    }

    fn as_ref(&self) -> &sys::mnd_response {
        // SAFETY: `raw` came from mnd_request and is freed exactly once, in
        // Drop. It is valid for the whole life of `self`.
        unsafe { self.raw.as_ref() }
    }

    /// HTTP status, or 0 when the call itself failed (an escaped C++
    /// exception — the body carries its message).
    pub fn status(&self) -> u16 {
        self.as_ref().status.clamp(0, u16::MAX as i32) as u16
    }

    /// The response body, borrowed from the C allocation. Zero copy.
    ///
    /// `body_len` bytes, which **may contain NULs** — for a multipart /run
    /// response carrying PNG bytes this is the difference between two copies
    /// of the image and none.
    pub fn body(&self) -> &[u8] {
        let r = self.as_ref();
        if r.body.is_null() || r.body_len == 0 {
            return &[];
        }
        // SAFETY: body_len bytes at body, valid while self is alive; the ABI
        // guarantees body_len is the truth and never strlen's it.
        unsafe { std::slice::from_raw_parts(r.body as *const u8, r.body_len) }
    }

    /// The body as UTF-8, still borrowed. Fails rather than lossily
    /// converting, because a failure here means the body was never text.
    pub fn body_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.body())
    }

    /// Never NULL per the ABI; `""` when absent. Checked anyway (hazard 9).
    pub fn content_type(&self) -> &str {
        let r = self.as_ref();
        if r.content_type.is_null() {
            return "";
        }
        // SAFETY: NUL-terminated string owned by the response.
        unsafe { CStr::from_ptr(r.content_type) }
            .to_str()
            .unwrap_or("")
    }

    /// Load/serve notes, parsed once from the response's JSON array.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// The body parsed as JSON.
    ///
    /// Returns the mapped status error *before* parsing on any non-2xx
    /// response: error bodies are plain text (`src/serve/server.cpp:154`) and
    /// handing them to serde_json would report a parse failure where the real
    /// failure is a 400 with a perfectly good message in it (§4.4).
    pub fn json(&self) -> Result<serde_json::Value, Error> {
        self.ok_or_status()?;
        serde_json::from_slice(self.body()).map_err(Error::Json)
    }

    pub fn deserialize<T: DeserializeOwned>(&self) -> Result<T, Error> {
        self.ok_or_status()?;
        serde_json::from_slice(self.body()).map_err(Error::Json)
    }

    /// One output of a `/run.json` response as a dense tensor.
    ///
    /// `i64` ↔ `int64_t` round-trips losslessly (`interp/value.h:23-24`), and
    /// `src/serve/json.cpp:341` emits `%lld` at `quantres == 0`. At
    /// `quantres > 0` the outputs are printed decimals and this refuses them —
    /// use [`Response::decimal`], which hands back the exact printed text.
    ///
    /// Non-rectangular output is rejected rather than flattened.
    pub fn tensor(&self, name: &str) -> Result<Tensor, Error> {
        self.ok_or_status()?;
        let root: serde_json::Value = serde_json::from_slice(self.body()).map_err(Error::Json)?;
        tensor_from_run_json(&root, name)
    }

    /// The exact text the engine printed for one output, borrowed from the
    /// body — the answer to `quantres > 0` (§4.2).
    ///
    /// `json.cpp:343` emits `%.*f` of a decoded double, and plain serde_json
    /// would round that text into an `f64`. The obvious fix,
    /// `features = ["arbitrary_precision"]`, is a *global* cargo feature that
    /// silently changes `serde_json::Number` for every other crate in the
    /// graph and breaks `#[serde(flatten)]`; a library must never enable it.
    /// So: the bytes, verbatim, and the caller picks their own decimal type.
    ///
    /// Returns the value's raw JSON span, so a scalar comes back as `"1.25"`
    /// and a matrix as `"[[1.25,2.50],[0.00,1.00]]"` — digits exactly as
    /// printed, no round trip through a binary float anywhere.
    pub fn decimal(&self, name: &str) -> Result<&str, Error> {
        self.ok_or_status()?;
        let text = self
            .body_str()
            .map_err(|e| Error::decode(format!("response body is not UTF-8: {e}")))?;
        let outputs = json_member(text, "outputs").ok_or_else(|| {
            Error::decode("response has no 'outputs' object (is this a /run.json response?)")
        })?;
        json_member(outputs, name)
            .map(str::trim)
            .ok_or_else(|| Error::decode(format!("no output named '{name}'")))
    }

    /// `Ok(self)` on 2xx, the mapped error otherwise (§4.4).
    pub fn into_result(self) -> Result<Response, Error> {
        match self.ok_or_status() {
            Ok(()) => Ok(self),
            Err(e) => Err(e),
        }
    }

    fn ok_or_status(&self) -> Result<(), Error> {
        let status = self.status();
        if (200..300).contains(&status) {
            return Ok(());
        }
        Err(Error::from_status(
            status,
            String::from_utf8_lossy(self.body()).into_owned(),
        ))
    }
}

impl Drop for Response {
    fn drop(&mut self) {
        // Frees the struct AND every string it points at — the caller never
        // calls free() on anything this library returns (ABI, ownership rule
        // 2). `body()` borrows from here, so the borrow checker has already
        // proven no slice outlives this.
        unsafe { sys::mnd_response_free(self.raw.as_ptr()) }
    }
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status())
            .field("content_type", &self.content_type())
            .field("body_len", &self.body().len())
            .field("notes", &self.notes)
            .finish()
    }
}

/// `i64` <-> `int64_t` round-trips losslessly (include/maned/interp/value.h:23-24).
///
/// > Routed flows wrap mod-2³² on bark; local flows do not. `i64` reports
/// > faithfully whichever you got — it does not reconcile them (MANEDPY §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tensor {
    pub shape: Vec<i64>,
    pub data: Vec<i64>,
}

impl Tensor {
    /// Product of the dimensions — always `data.len()`.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Parsing, factored out so it is unit-testable without a live library.
// ---------------------------------------------------------------------------

fn parse_notes(p: *const std::ffi::c_char) -> Vec<String> {
    if p.is_null() {
        // Hazard 9 again: the ABI says never NULL, and we check anyway.
        return Vec::new();
    }
    let raw = unsafe { CStr::from_ptr(p) };
    match raw.to_str() {
        Ok(s) => serde_json::from_str::<Vec<String>>(s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub(crate) fn tensor_from_run_json(root: &serde_json::Value, name: &str) -> Result<Tensor, Error> {
    let outputs = root
        .get("outputs")
        .and_then(|o| o.as_object())
        .ok_or_else(|| {
            Error::decode("response has no 'outputs' object (is this a /run.json response?)")
        })?;
    let value = outputs.get(name).ok_or_else(|| {
        let mut names: Vec<&str> = outputs.keys().map(String::as_str).collect();
        names.sort_unstable();
        Error::decode(if names.is_empty() {
            format!("no output named '{name}'; the response has no outputs")
        } else {
            format!(
                "no output named '{name}'; outputs are: {}",
                names.join(", ")
            )
        })
    })?;

    let mut w = Walk {
        name,
        shape: Vec::new(),
        data: Vec::new(),
        leaf_depth: None,
    };
    w.walk(value, 0)?;
    Ok(Tensor {
        shape: w.shape,
        data: w.data,
    })
}

/// Rectangularity is two invariants, not one: every sibling row has the same
/// length (`shape[depth]`), and every leaf sits at the same depth
/// (`leaf_depth`). Checking only the first accepts `[[1,2],[[3],[4]]]`.
struct Walk<'a> {
    name: &'a str,
    shape: Vec<i64>,
    data: Vec<i64>,
    leaf_depth: Option<usize>,
}

impl Walk<'_> {
    fn walk(&mut self, v: &serde_json::Value, depth: usize) -> Result<(), Error> {
        let name = self.name;
        match v {
            serde_json::Value::Array(items) => {
                if let Some(leaf) = self.leaf_depth {
                    if depth >= leaf {
                        return Err(Error::decode(format!(
                            "output '{name}' is not rectangular: an array at depth {depth} \
                         where a scalar belongs"
                        )));
                    }
                }
                if depth == self.shape.len() {
                    self.shape.push(items.len() as i64);
                } else if self.shape[depth] != items.len() as i64 {
                    return Err(Error::decode(format!(
                    "output '{name}' is not rectangular: row of {} at depth {depth}, expected {}",
                    items.len(),
                    self.shape[depth]
                )));
                }
                for item in items {
                    self.walk(item, depth + 1)?;
                }
                Ok(())
            }
            serde_json::Value::Number(n) => {
                if depth != self.shape.len() {
                    return Err(Error::decode(format!(
                        "output '{name}' is not rectangular: scalar at depth {depth}, \
                     expected an array of {}",
                        self.shape[depth]
                    )));
                }
                match self.leaf_depth {
                    Some(leaf) if leaf != depth => {
                        return Err(Error::decode(format!(
                            "output '{name}' is not rectangular: scalar at depth {depth}, \
                         but other scalars sit at depth {leaf}"
                        )))
                    }
                    Some(_) => {}
                    None => self.leaf_depth = Some(depth),
                }
                match n.as_i64() {
                    Some(i) => {
                        self.data.push(i);
                        Ok(())
                    }
                    None => Err(Error::decode(format!(
                        "output '{name}' holds {n}, which is not an integer — the script \
                     runs at quantres > 0, so use decimal(\"{name}\") for the exact \
                     printed text"
                    ))),
                }
            }
            other => Err(Error::decode(format!(
                "output '{name}' is not a numeric tensor (found {})",
                kind_of(other)
            ))),
        }
    }
}

fn kind_of(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// Return the **raw JSON text** of `key`'s value inside the object `obj`,
/// without parsing any number. This is what makes `decimal()` exact: the only
/// way to hand back the engine's printed digits is never to turn them into a
/// number in the first place.
///
/// Returns `None` if `obj` is not an object or has no such key at its top
/// level. Deliberately not a general JSON pointer — one level, one key.
pub(crate) fn json_member<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
    let b = obj.as_bytes();
    let mut i = skip_ws(b, 0);
    if b.get(i)? != &b'{' {
        return None;
    }
    i += 1;
    loop {
        i = skip_ws(b, i);
        match b.get(i)? {
            b'}' => return None,
            b'"' => {}
            _ => return None,
        }
        let (k, next) = scan_string(b, i)?;
        i = skip_ws(b, next);
        if b.get(i)? != &b':' {
            return None;
        }
        i = skip_ws(b, i + 1);
        let start = i;
        i = skip_value(b, i)?;
        if k == key {
            return obj.get(start..i);
        }
        i = skip_ws(b, i);
        match b.get(i)? {
            b',' => i += 1,
            b'}' => return None,
            _ => return None,
        }
    }
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// `b[i]` must be `"`. Returns (unescaped-enough key, index past the closing
/// quote). Keys the engine emits are plain identifiers, so the "unescaping"
/// only has to be correct for a key that contains no escapes; a key that does
/// simply will not compare equal, which is the safe direction.
fn scan_string(b: &[u8], i: usize) -> Option<(&str, usize)> {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => {
                let s = std::str::from_utf8(b.get(i + 1..j)?).ok()?;
                return Some((s, j + 1));
            }
            _ => j += 1,
        }
    }
    None
}

/// Index just past the JSON value starting at `i`.
fn skip_value(b: &[u8], i: usize) -> Option<usize> {
    match *b.get(i)? {
        b'"' => Some(scan_string(b, i)?.1),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => j = scan_string(b, j)?.1,
                    b'{' | b'[' => {
                        depth += 1;
                        j += 1;
                    }
                    b'}' | b']' => {
                        depth -= 1;
                        j += 1;
                        if depth == 0 {
                            return Some(j);
                        }
                    }
                    _ => j += 1,
                }
            }
            None
        }
        _ => {
            // number / true / false / null: runs until a structural byte.
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
            {
                j += 1;
            }
            (j > i).then_some(j)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tensor(v: serde_json::Value, name: &str) -> Result<Tensor, Error> {
        tensor_from_run_json(&json!({ "outputs": v, "notes": [] }), name)
    }

    #[test]
    fn parses_a_matrix() {
        let t = tensor(json!({ "a": [[7, 9], [2, 4]] }), "a").unwrap();
        assert_eq!(t.shape, vec![2, 2]);
        assert_eq!(t.data, vec![7, 9, 2, 4]);
        assert_eq!(t.len(), 4);
    }

    #[test]
    fn parses_a_vector_a_scalar_and_a_3d_block() {
        assert_eq!(
            tensor(json!({ "a": [1, 2, 3] }), "a").unwrap().shape,
            vec![3]
        );
        let s = tensor(json!({ "a": 5 }), "a").unwrap();
        assert_eq!(s.shape, Vec::<i64>::new());
        assert_eq!(s.data, vec![5]);
        let c = tensor(json!({ "a": [[[1, 2]], [[3, 4]]] }), "a").unwrap();
        assert_eq!(c.shape, vec![2, 1, 2]);
        assert_eq!(c.data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn negatives_and_the_int64_extremes_round_trip() {
        let t = tensor(json!({ "a": [[-1, i64::MIN, i64::MAX]] }), "a").unwrap();
        assert_eq!(t.data, vec![-1, i64::MIN, i64::MAX]);
    }

    #[test]
    fn rejects_a_ragged_row() {
        let e = tensor(json!({ "a": [[3, 1], [2]] }), "a").unwrap_err();
        let m = e.to_string();
        assert!(m.contains("not rectangular"), "{m}");
        assert!(m.contains("row of 1 at depth 1"), "{m}");
    }

    #[test]
    fn rejects_a_scalar_where_a_row_belongs() {
        let e = tensor(json!({ "a": [[3, 1], 2] }), "a").unwrap_err();
        assert!(e.to_string().contains("scalar at depth 1"), "{e}");
    }

    #[test]
    fn rejects_a_deeper_row() {
        // The mirror of the ragged case: too many dimensions, not too few.
        let e = tensor(json!({ "a": [[1, 2], [[3], [4]]] }), "a").unwrap_err();
        assert!(e.to_string().contains("not rectangular"), "{e}");
    }

    #[test]
    fn missing_output_names_the_ones_that_exist() {
        let e = tensor(json!({ "a": [[1]], "t": [[1]] }), "zz").unwrap_err();
        let m = e.to_string();
        assert!(m.contains("no output named 'zz'"), "{m}");
        assert!(m.contains("outputs are: a, t"), "{m}");
    }

    #[test]
    fn decimals_are_refused_by_tensor_and_point_at_decimal() {
        let e = tensor(json!({ "y": [[1.25]] }), "y").unwrap_err();
        let m = e.to_string();
        assert!(m.contains("not an integer"), "{m}");
        assert!(m.contains("decimal(\"y\")"), "{m}");
    }

    #[test]
    fn a_body_that_is_not_run_json_says_so() {
        let e = tensor_from_run_json(&json!({ "t": [{ "a": 1 }] }), "t").unwrap_err();
        assert!(e.to_string().contains("no 'outputs' object"), "{e}");
    }

    // ---- the raw-span scanner behind decimal() ----------------------------

    #[test]
    fn json_member_returns_the_verbatim_span() {
        let body = r#"{"outputs":{"y":[[3.00,5.00],[0.10,1.00]],"z":7},"notes":[]}"#;
        let outputs = json_member(body, "outputs").unwrap();
        assert_eq!(outputs, r#"{"y":[[3.00,5.00],[0.10,1.00]],"z":7}"#);
        // The trailing zeros an f64 round trip would have eaten are still here.
        assert_eq!(
            json_member(outputs, "y").unwrap(),
            "[[3.00,5.00],[0.10,1.00]]"
        );
        assert_eq!(json_member(outputs, "z").unwrap(), "7");
        assert_eq!(json_member(body, "notes").unwrap(), "[]");
        assert_eq!(json_member(outputs, "nope"), None);
    }

    #[test]
    fn json_member_is_not_confused_by_strings_or_whitespace() {
        let body = r#"{ "a" : "}{,\"x\"" , "b" : { "a" : 1 } , "c" : -2.5e-3 }"#;
        assert_eq!(json_member(body, "a").unwrap(), r#""}{,\"x\"""#);
        assert_eq!(json_member(body, "b").unwrap(), r#"{ "a" : 1 }"#);
        assert_eq!(json_member(body, "c").unwrap(), "-2.5e-3");
    }

    #[test]
    fn json_member_declines_non_objects() {
        assert_eq!(json_member("[1,2]", "a"), None);
        assert_eq!(json_member("", "a"), None);
        assert_eq!(json_member("{", "a"), None);
        assert_eq!(json_member("not json", "a"), None);
    }
}
