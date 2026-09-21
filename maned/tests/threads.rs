//! Gate 7, the positive half — `testConcurrentHandleIsIdenticalGap025`
//! (`tests/serve/test_serve.cpp:920`) ported to the binding it justifies.
//!
//! The compile-fail half lives in `compile_fail.rs`: what must NOT compile is
//! as much of the contract as what must run.

use std::sync::Arc;

const SCRIPT: &str = "01_local_matmul.mnd";
const PAYLOAD: &str = r#"{"h0": [[3,1],[2,4]]}"#;

fn corpus(name: &str) -> std::path::PathBuf {
    let dir = std::env::var("MANED_CONFORMANCE_DIR")
        .unwrap_or_else(|_| "/Users/fabianodicheti/Documents/maned/capi-freeze/conformance".into());
    std::path::PathBuf::from(dir).join(name)
}

fn payload() -> serde_json::Value {
    serde_json::from_str(PAYLOAD).unwrap()
}

/// Eight threads, twenty-five calls each, one handle, no lock — and every one
/// of the two hundred responses byte-identical.
///
/// The ABI's rule is exact: concurrent `mnd_request` on ONE handle is safe if
/// and only if `mnd_script_is_routed(s) == 0`. `SharedScript` is the type that
/// can only exist when that holds.
#[test]
fn shared_script_is_identical_across_eight_threads() {
    let script = maned::load(corpus(SCRIPT))
        .unwrap()
        .into_shared()
        .expect("01_local_matmul is not routed");
    assert!(!script.is_routed());

    let expected = script.run(&payload()).unwrap().body().to_vec();
    assert_eq!(
        expected,
        br#"{"outputs":{"a":[[7,9],[2,4]]},"notes":[]}"#.to_vec()
    );

    let script = Arc::new(script);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let script = Arc::clone(&script);
        let expected = expected.clone();
        handles.push(std::thread::spawn(move || {
            for _ in 0..25 {
                let r = script.run(&payload()).unwrap();
                assert_eq!(r.status(), 200);
                assert_eq!(r.body(), expected.as_slice());
            }
        }));
    }
    for h in handles {
        h.join().expect("no thread panicked");
    }
}

/// `Arc<SharedScript>` is `Send` — the thing `Arc<Script>` deliberately is
/// not. A static assertion, so weakening either impl fails the build here as
/// well as in the trybuild suite.
#[test]
fn the_send_and_sync_impls_are_what_section_4_3_says() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    assert_send::<maned::Script>();
    assert_sync::<maned::SharedScript>();
    assert_send::<maned::SharedScript>();
    assert_send::<Arc<maned::SharedScript>>();
    // A Response must be Send or run_async could not return one (§4.6).
    assert_send::<maned::Response>();

    // maned::Script: !Sync and Arc<maned::Script>: !Send are proven by
    // compile_fail/, because a negative bound cannot be asserted here.
}

/// A `Script` moves between threads (that is `Send`), one owner at a time.
#[test]
fn a_script_can_be_moved_to_another_thread() {
    let mut script = maned::load(corpus(SCRIPT)).unwrap();
    let body = std::thread::spawn(move || script.run(&payload()).unwrap().body().to_vec())
        .join()
        .unwrap();
    assert!(body.starts_with(br#"{"outputs""#));
}
