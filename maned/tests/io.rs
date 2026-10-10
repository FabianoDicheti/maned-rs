//! The file routes — `POST /run` as multipart, `/frame`, `/frame.csv` — and
//! the `GET /health` contract they read.
//!
//! The multipart encoding is asserted **byte for byte** against the body
//! manedpy produces for the same inputs. The three bindings are one contract;
//! a body that differs by a header is a second one, and the difference would
//! only show up against an engine version that stops being lenient.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn corpus(name: &str) -> PathBuf {
    let dir = std::env::var("MANED_CONFORMANCE_DIR")
        .unwrap_or_else(|_| "/Users/fabianodicheti/Documents/maned/capi-freeze/conformance".into());
    PathBuf::from(dir).join(name)
}

// --- encoding -------------------------------------------------------------

/// The exact bytes `manedpy.script.encode_multipart("abc123", {"h0": ...})`
/// returns. Frozen here deliberately: this is the parity gate, so it must fail
/// when the encoder drifts, not follow it.
const MANEDPY_BODY: &[u8] = b"--abc123\r\n\
Content-Disposition: form-data; name=\"h0\"; filename=\"h0\"\r\n\
Content-Type: application/octet-stream\r\n\r\n\
3,1\n2,4\n\r\n\
--abc123--\r\n";

#[test]
fn encode_is_byte_identical_to_manedpy() {
    let body = maned::multipart::encode("abc123", &[("h0", b"3,1\n2,4\n".as_slice())]);
    assert_eq!(
        body,
        MANEDPY_BODY,
        "multipart encoding drifted from manedpy's:\n{}",
        String::from_utf8_lossy(&body)
    );
}

#[test]
fn round_trip_survives_nuls_and_crlf() {
    // A PNG signature carries both a NUL and a CRLF, and this payload ends
    // with the same CRLF that terminates a part — the case a decoder that
    // trims greedily gets wrong.
    let png: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00bin\r\n";
    let csv: &[u8] = b"1,2\n";
    let body = maned::multipart::encode("abc123", &[("img", png), ("csv", csv)]);

    let parts = maned::multipart::decode("multipart/form-data; boundary=abc123", &body).unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts["img"], png);
    assert_eq!(parts["csv"], csv);
}

#[test]
fn decode_reads_a_quoted_boundary_and_ignores_other_parameters() {
    let body = maned::multipart::encode("abc123", &[("a", b"1".as_slice())]);
    let parts = maned::multipart::decode(
        "multipart/form-data; charset=utf-8; boundary=\"abc123\"",
        &body,
    )
    .unwrap();
    assert_eq!(parts["a"], b"1");
}

#[test]
fn decode_refuses_a_body_with_no_boundary() {
    let err = maned::multipart::decode("application/json", b"{}").unwrap_err();
    assert!(
        err.to_string().contains("no boundary"),
        "unhelpful message: {err}"
    );
}

#[test]
fn boundary_never_occurs_in_the_payload() {
    let boundary = maned::multipart::boundary();
    assert!(boundary.starts_with("manedrs"));
    assert_ne!(boundary, maned::multipart::boundary(), "boundaries repeat");

    // The degenerate input: a part that already contains a boundary this
    // generator could produce.
    let poisoned = format!("--{}\r\n", maned::multipart::boundary()).into_bytes();
    let parts = [("x", poisoned.as_slice())];
    let chosen = maned::multipart::boundary_for(&parts);
    assert!(!poisoned
        .windows(chosen.len())
        .any(|w| w == chosen.as_bytes()));
}

// --- the file route -------------------------------------------------------

#[test]
fn run_files_round_trips_raw_bytes() {
    let mut script = maned::load(corpus("01_local_matmul.mnd")).unwrap();
    let parts = script
        .run_files(&[("h0", b"3,1\n2,4\n".as_slice())])
        .unwrap();

    // One output, so the bare body is keyed by the single declared out:: name.
    let expected: BTreeMap<String, Vec<u8>> = [("a".to_string(), b"7,9\n2,4\n".to_vec())]
        .into_iter()
        .collect();
    assert_eq!(parts, expected);
}

#[test]
fn run_files_returns_every_part_of_a_multi_output_script() {
    let mut script = maned::load(corpus("04_multi_output.mnd")).unwrap();
    let parts = script
        .run_files(&[("h0", b"3,1\n2,4\n".as_slice())])
        .unwrap();

    assert_eq!(parts["a"], b"7,9\n2,4\n");
    assert_eq!(parts["t"], b"7,2\n9,4\n");
    assert_eq!(parts.len(), 2);
}

#[test]
fn run_files_maps_a_refusal_to_an_error() {
    let mut script = maned::load(corpus("01_local_matmul.mnd")).unwrap();
    let err = script
        .run_files(&[("nosuch", b"1\n".as_slice())])
        .unwrap_err();
    assert!(
        matches!(err, maned::Error::Input { .. }),
        "an unknown in:: name should be a 400, got {err:?}"
    );
}

#[test]
fn run_files_works_through_a_shared_script() {
    let shared = maned::load(corpus("04_multi_output.mnd"))
        .unwrap()
        .into_shared()
        .unwrap();
    let parts = shared
        .run_files(&[("h0", b"3,1\n2,4\n".as_slice())])
        .unwrap();
    assert_eq!(parts["t"], b"7,2\n9,4\n");
}

// --- frame ----------------------------------------------------------------

#[test]
fn frame_and_frame_csv_agree_with_manedpy() {
    let payload = serde_json::json!({"t": [{"a": 1, "b": 2}, {"a": 3, "b": 4}]});
    let mut script = maned::load(corpus("03_frame_records.mnd")).unwrap();

    // The frame document IS the body on this route — not nested under a key.
    assert_eq!(script.frame(&payload).unwrap().json().unwrap(), payload);
    assert_eq!(script.frame_csv(&payload).unwrap(), b"a,b\n1,2\n3,4\n");
}

// --- the contract ---------------------------------------------------------

#[test]
fn health_reports_the_io_contract_without_the_fake_metric() {
    let script = maned::load(corpus("04_multi_output.mnd")).unwrap();

    let inputs = script.inputs().unwrap();
    assert_eq!(
        inputs.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
        ["h0", "w"]
    );
    assert_eq!(inputs[0].format, "csv");
    assert_eq!(inputs[0].default, Some(false));
    assert_eq!(inputs[1].default, Some(true), "w has a literal default");

    let outputs = script.outputs().unwrap();
    assert_eq!(
        outputs.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(),
        ["a", "t"]
    );
    assert_eq!(outputs[0].default, None, "an out:: cannot have a default");

    assert!(script.routes().unwrap().iter().any(|r| r == "POST /run"));
    assert!(
        script.health().unwrap().get("requests_served").is_none(),
        "requests_served counts this handle's calls, not a server's"
    );
}

#[test]
fn frame_routes_are_advertised_only_by_a_frame_script() {
    let plain = maned::load(corpus("01_local_matmul.mnd")).unwrap();
    assert!(!plain.routes().unwrap().iter().any(|r| r == "POST /frame"));

    let framed = maned::load(corpus("03_frame_records.mnd")).unwrap();
    assert!(framed.routes().unwrap().iter().any(|r| r == "POST /frame"));
}
