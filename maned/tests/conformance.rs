//! Gate 3 — the cross-binding conformance corpus, in Rust.
//!
//! One fixture set, walked by every consumer of the `mnd_*` ABI: the in-repo
//! `capi` suite, manedpy, maned-kotlin and this crate, so the parity corpus
//! exists ONCE and cannot fork (MANEDKT_PLAN.md §3.3). Each case is a triple —
//! `<case>.mnd`, `<case>.request.json`, `<case>.expect.json` — and the
//! comparison is **byte for byte**, which is the entire point: a binding that
//! reshapes a response has not implemented the REST contract, it has
//! implemented something adjacent to it.
//!
//! The corpus ships in the `maned-capi-*` tarball; point
//! `MANED_CONFORMANCE_DIR` at it (this checkout does so in
//! `.cargo/config.toml`).

use serde_json::Value;
use std::path::PathBuf;

const DEFAULT_DIR: &str = "/Users/fabianodicheti/Documents/maned/capi-freeze/conformance";

fn corpus_dir() -> PathBuf {
    match std::env::var_os("MANED_CONFORMANCE_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(DEFAULT_DIR),
    }
}

struct Case {
    name: String,
    script: PathBuf,
    request: Value,
    expect: Value,
}

fn cases() -> Vec<Case> {
    let dir = corpus_dir();
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| {
        panic!(
            "cannot read the conformance corpus at {}: {e}\n\
             Set MANED_CONFORMANCE_DIR to the `conformance/` directory of an \
             unpacked maned-capi tarball.",
            dir.display()
        )
    });

    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "mnd"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    names.sort();

    names
        .into_iter()
        .map(|name| {
            let read = |suffix: &str| -> Value {
                let p = dir.join(format!("{name}{suffix}"));
                let text =
                    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                serde_json::from_str(&text)
                    .unwrap_or_else(|e| panic!("{}: not JSON: {e}", p.display()))
            };
            Case {
                script: dir.join(format!("{name}.mnd")),
                request: read(".request.json"),
                expect: read(".expect.json"),
                name,
            }
        })
        .collect()
}

#[test]
fn every_fixture_matches_byte_for_byte() {
    let cases = cases();
    assert!(
        cases.len() >= 9,
        "expected the whole corpus, found {} case(s) in {}",
        cases.len(),
        corpus_dir().display()
    );

    for case in &cases {
        // `flags` is an optional array of mnd_open flag names.
        let mut builder = maned::builder(&case.script);
        if let Some(flags) = case.request.get("flags").and_then(Value::as_array) {
            for flag in flags {
                match flag.as_str() {
                    Some("synthesize_outputs") => {
                        builder = builder.synthesize_outputs(true);
                    }
                    Some("no_vm_fallback") => {
                        builder = builder.no_vm_fallback(true);
                    }
                    other => panic!("{}: unknown flag {other:?}", case.name),
                }
            }
        }

        let mut script = builder
            .open()
            .unwrap_or_else(|e| panic!("{}: {e}", case.name));

        let method = case.request["method"].as_str().expect("method");
        let target = case.request["target"].as_str().expect("target");
        // `body` is the EXACT raw request body as a JSON string ("" for none).
        let body = case.request["body"].as_str().expect("body");
        let content_type = case.request.get("content_type").and_then(Value::as_str);

        let response = script
            .request(method, target, content_type, body.as_bytes())
            .unwrap_or_else(|e| panic!("{}: {e}", case.name));

        let want_status = case.expect["status"].as_u64().expect("status") as u16;
        let want_type = case.expect["content_type"].as_str().expect("content_type");
        let want_body = case.expect["body"].as_str().expect("body");

        assert_eq!(response.status(), want_status, "{}: status", case.name);
        assert_eq!(
            response.content_type(),
            want_type,
            "{}: content_type",
            case.name
        );
        assert_eq!(
            response.body(),
            want_body.as_bytes(),
            "{}: body\n  want: {want_body}\n   got: {}",
            case.name,
            String::from_utf8_lossy(response.body())
        );
    }
}

/// The error half of the corpus, read through the *typed* API rather than the
/// raw one: the status map of §4.4 has to agree with the fixtures it was
/// written from, and error bodies are plain text — nothing parses them.
#[test]
fn error_fixtures_map_to_the_right_error_variants() {
    use maned::Error;

    for case in cases() {
        let status = case.expect["status"].as_u64().unwrap() as u16;
        if (200..300).contains(&status) {
            continue;
        }
        let mut script = maned::builder(&case.script).open().unwrap();
        let body = case.request["body"].as_str().unwrap();
        let payload: Value = serde_json::from_str(body).unwrap_or(Value::Null);

        // Only /run.json cases can go through run(); the rest are exercised
        // above.
        if case.request["target"] != "/run.json" {
            continue;
        }

        let err = script.run(&payload).expect_err(&case.name);
        assert_eq!(err.status(), Some(status), "{}: {err}", case.name);
        match (status, &err) {
            (400, Error::Input { .. }) => {}
            (404 | 405, Error::Route { .. }) => {}
            (500, Error::Runtime { .. }) => {}
            (501, Error::Unsupported { .. }) => {}
            _ => panic!("{}: status {status} mapped to {err:?}", case.name),
        }
        // Plain text, not JSON (src/serve/server.cpp:154) — and the trailing
        // newline the engine appends is kept in the body and trimmed only in
        // Display.
        let want_body = case.expect["body"].as_str().unwrap();
        assert!(want_body.ends_with('\n'), "{}: fixture", case.name);
        assert!(
            err.to_string().ends_with(want_body.trim_end()),
            "{}: {err}",
            case.name
        );
    }
}

/// `/health` is excluded from the byte-comparison corpus because it embeds a
/// script path and a request counter, so it gets a structural assertion of its
/// own — the corpus README asks every binding to do exactly this.
#[test]
fn health_answers_structurally() {
    let case = cases().into_iter().next().expect("at least one case");
    let mut script = maned::builder(&case.script).open().unwrap();
    let r = script.request("GET", "/health", None, b"").unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().unwrap();
    assert!(v.get("script").is_some(), "{v}");
}
