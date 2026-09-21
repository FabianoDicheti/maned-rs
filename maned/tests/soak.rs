//! Gate 6 — ten thousand open / request / free cycles.
//!
//! Nothing in Rust's tooling will notice a leaked C allocation: `Drop` runs or
//! it does not, and a missing `mnd_response_free` looks exactly like a working
//! program until RSS says otherwise. This test cannot see a leak either — it
//! is a trip-wire for the gross failures (double free, use-after-free, a
//! handle closed twice by shutdown-plus-Drop), which abort the process, and a
//! harness for the real gate: run it under LeakSanitizer or watch RSS.
//!
//!     cargo test --release --test soak -- --nocapture
//!
//! Plain, no sanitizer, per its ticket — the sanitizer run is a separate,
//! manual gate because it needs a rebuilt libmaned_c.

const PAYLOAD: &[u8] = br#"{"h0": [[3,1],[2,4]]}"#;
const EXPECTED: &[u8] = br#"{"outputs":{"a":[[7,9],[2,4]]},"notes":[]}"#;

fn corpus(name: &str) -> std::path::PathBuf {
    let dir = std::env::var("MANED_CONFORMANCE_DIR")
        .unwrap_or_else(|_| "/Users/fabianodicheti/Documents/maned/capi-freeze/conformance".into());
    std::path::PathBuf::from(dir).join(name)
}

#[test]
fn ten_thousand_open_request_free_cycles() {
    let path = corpus("01_local_matmul.mnd");

    for i in 0..10_000 {
        let mut script = maned::load(&path).unwrap_or_else(|e| panic!("iteration {i}: {e}"));
        let response = script
            .request("POST", "/run.json", None, PAYLOAD)
            .unwrap_or_else(|e| panic!("iteration {i}: {e}"));
        assert_eq!(response.status(), 200, "iteration {i}");
        assert_eq!(response.body(), EXPECTED, "iteration {i}");
        // Both frees happen here: mnd_response_free, then mnd_close.
    }

    // Handle accounting (the registry behind maned::shutdown()) is asserted in
    // shutdown.rs, which is a test binary of its own and can therefore make
    // process-wide claims this one cannot.
}

/// The same cycle count, one handle, ten thousand responses — isolating the
/// `Response` free from the `Script` free.
#[test]
fn ten_thousand_responses_on_one_handle() {
    let mut script = maned::load(corpus("04_multi_output.mnd")).unwrap();
    for i in 0..10_000 {
        let r = script.request("POST", "/run.json", None, PAYLOAD).unwrap();
        assert_eq!(r.status(), 200, "iteration {i}");
        // Hold two responses alive at once now and then: the allocation the
        // second one gets must not be the first one's.
        if i % 1000 == 0 {
            let r2 = script.request("POST", "/run.json", None, PAYLOAD).unwrap();
            assert_eq!(r2.body(), r.body());
        }
    }
}
