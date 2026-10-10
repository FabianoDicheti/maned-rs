//! `multipart/form-data` for the file routes — `POST /run` and `POST /frame`.
//!
//! `/run.json` renders an `out::` image as a nested integer array and an
//! `out::` csv as a JSON tensor; the only way to get the engine's real file
//! bytes back is the multipart `/run` route, which answers with one part per
//! declared output. That route also takes its *inputs* as file bytes, so a
//! `csv()` / `image()` / `parquet()` `in::` never has to be decoded into JSON
//! by the caller first.
//!
//! The encoding here is byte-identical to manedpy's `encode_multipart`
//! (`manedpy/manedpy/script.py`) — same part order, same header spelling, same
//! CRLF placement — because the three bindings are checked against one frozen
//! expectation and a body that differs by a header is not the same contract.
//!
//! `decode` is hand-rolled rather than delegating to an HTTP crate: the parts
//! are arbitrary binary (PNG, parquet), and every text-oriented MIME parser
//! either re-encodes them or truncates at the first NUL.

use crate::Error;
use std::collections::hash_map::RandomState;
use std::collections::BTreeMap;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// One form-data part per `(name, bytes)`, in the order given.
///
/// The `name` is the `in::` name the bytes are for; it is repeated as the
/// `filename` because the engine's multipart reader keys on `name` and some
/// intermediaries drop a part with no filename.
///
/// The caller picks the boundary. Use [`boundary_for`] unless you have a
/// reason not to — it is the only spelling that is checked against the part
/// bytes, and a boundary that occurs inside a part truncates the body.
pub fn encode<K: AsRef<str>, V: AsRef<[u8]>>(boundary: &str, parts: &[(K, V)]) -> Vec<u8> {
    let marker = format!("--{boundary}");
    let mut out = Vec::with_capacity(
        parts
            .iter()
            .map(|(k, v)| v.as_ref().len() + k.as_ref().len() * 2 + marker.len() + 96)
            .sum::<usize>()
            + marker.len()
            + 4,
    );
    for (name, data) in parts {
        let name = name.as_ref();
        out.extend_from_slice(marker.as_bytes());
        out.extend_from_slice(b"\r\nContent-Disposition: form-data; name=\"");
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b"\"; filename=\"");
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b"\"\r\nContent-Type: application/octet-stream\r\n\r\n");
        out.extend_from_slice(data.as_ref());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(marker.as_bytes());
    out.extend_from_slice(b"--\r\n");
    out
}

/// `multipart/form-data` body → one entry per part, keyed by part name.
///
/// A `BTreeMap` rather than the wire order: the names are `out::` names, which
/// the engine already refuses to duplicate (W009), so nothing is lost and
/// callers get both lookup and a deterministic iteration order.
///
/// A part with no `name=` in its Content-Disposition is skipped rather than
/// invented a key for — the engine does not emit one, and guessing would hide
/// a future protocol change behind a plausible-looking result.
pub fn decode(content_type: &str, body: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    let boundary = boundary_of(content_type).ok_or_else(|| {
        Error::decode(format!(
            "no boundary in Content-Type '{content_type}'; this is not a multipart body"
        ))
    })?;
    let marker = format!("--{boundary}").into_bytes();

    let mut parts = BTreeMap::new();
    for segment in split_on(body, &marker) {
        // The preamble before the first marker, the "\r\n" between parts and
        // the "--\r\n" epilogue after the last one.
        let segment = strip_prefix_crlf(segment);
        if segment.is_empty() || segment == b"--" || segment == b"--\r\n" {
            continue;
        }
        let Some(split) = find(segment, b"\r\n\r\n") else {
            continue;
        };
        let (head, rest) = segment.split_at(split);
        let mut data = &rest[4..];
        // The CRLF that terminates the part belongs to the framing, not to the
        // payload — and exactly one is removed, so a PNG whose own last bytes
        // are \r\n survives the round trip.
        if data.ends_with(b"\r\n") {
            data = &data[..data.len() - 2];
        }
        if let Some(name) = part_name(head) {
            parts.insert(name, data.to_vec());
        }
    }
    Ok(parts)
}

/// A boundary that is guaranteed not to occur in any of `parts`.
///
/// manedpy bets 96 bits of entropy on the same thing; this checks, because the
/// check is a memchr over bytes already in cache and the failure it prevents is
/// a silently truncated request body rather than an error.
pub fn boundary_for<K, V: AsRef<[u8]>>(parts: &[(K, V)]) -> String {
    for _ in 0..8 {
        let candidate = boundary();
        let needle = candidate.as_bytes();
        if !parts
            .iter()
            .any(|(_, v)| find(v.as_ref(), needle).is_some())
        {
            return candidate;
        }
    }
    // Unreachable short of a broken entropy source: eight independent 128-bit
    // draws would all have to collide with the payload.
    boundary()
}

/// `"manedrs"` + 32 hex digits.
///
/// No `rand` dependency for a string that never leaves the process: the seed
/// is `RandomState`'s per-instance hash key — the same randomness `HashMap`
/// relies on to resist collision attacks — mixed with the clock and a counter
/// so that two calls in the same nanosecond still differ.
pub fn boundary() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.write_u64(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0),
    );
    let lo = hasher.finish();
    let hi = RandomState::new().build_hasher().finish();
    format!("manedrs{hi:016x}{lo:016x}")
}

/// The `boundary=` parameter of a Content-Type, unquoted.
pub(crate) fn boundary_of(content_type: &str) -> Option<String> {
    for token in content_type.split(';').skip(1) {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("boundary") {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn part_name(head: &[u8]) -> Option<String> {
    for line in split_on(head, b"\r\n") {
        let Ok(text) = std::str::from_utf8(line) else {
            continue;
        };
        if !text
            .to_ascii_lowercase()
            .starts_with("content-disposition:")
        {
            continue;
        }
        for token in text.split(';').skip(1) {
            if let Some((key, value)) = token.split_once('=') {
                if key.trim().eq_ignore_ascii_case("name") {
                    return Some(value.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    None
}

fn strip_prefix_crlf(segment: &[u8]) -> &[u8] {
    let mut at = 0;
    while at < segment.len() && (segment[at] == b'\r' || segment[at] == b'\n') {
        at += 1;
    }
    &segment[at..]
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn split_on<'a>(body: &'a [u8], separator: &[u8]) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(at) = find(rest, separator) {
        out.push(&rest[..at]);
        rest = &rest[at + separator.len()..];
    }
    out.push(rest);
    out
}
