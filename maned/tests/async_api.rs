//! feature = "tokio" (MANEDRS_PLAN.md §4.6).
//!
//! Compiled away entirely without the feature, which is also the point: the
//! async surface is opt-in and tokio is not a dependency of the default build.

#![cfg(feature = "tokio")]

use std::sync::Arc;

fn corpus(name: &str) -> std::path::PathBuf {
    let dir = std::env::var("MANED_CONFORMANCE_DIR")
        .unwrap_or_else(|_| "/Users/fabianodicheti/Documents/maned/capi-freeze/conformance".into());
    std::path::PathBuf::from(dir).join(name)
}

fn shared() -> Arc<maned::SharedScript> {
    Arc::new(
        maned::load(corpus("01_local_matmul.mnd"))
            .unwrap()
            .into_shared()
            .unwrap(),
    )
}

#[tokio::test]
async fn run_async_returns_the_same_bytes_as_run() {
    let script = shared();
    let sync_body = script
        .run(&serde_json::json!({"h0": [[3,1],[2,4]]}))
        .unwrap()
        .body()
        .to_vec();

    let r = Arc::clone(&script)
        .run_async(serde_json::json!({"h0": [[3,1],[2,4]]}))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.body(), sync_body.as_slice());
}

/// Thirty-two concurrent futures over one handle: `spawn_blocking` fans them
/// out across the blocking pool, and the responses are identical — the async
/// restatement of `testConcurrentHandleIsIdenticalGap025`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_concurrent_run_async_calls_agree() {
    let script = shared();
    let expected = br#"{"outputs":{"a":[[7,9],[2,4]]},"notes":[]}"#.to_vec();

    let mut futures = Vec::new();
    for _ in 0..32 {
        futures.push(tokio::spawn(
            Arc::clone(&script).run_async(serde_json::json!({"h0": [[3,1],[2,4]]})),
        ));
    }
    for f in futures {
        let r = f.await.unwrap().unwrap();
        assert_eq!(r.body(), expected.as_slice());
    }
}

/// The error channel survives the trip through the blocking pool.
#[tokio::test]
async fn run_async_propagates_the_status_map() {
    let script = shared();
    let err = script
        .run_async(serde_json::json!({"h0": [[3,1],[2]]}))
        .await
        .unwrap_err();
    assert_eq!(err.status(), Some(400));
    assert!(matches!(err, maned::Error::Input { .. }), "{err:?}");
    assert!(err.to_string().contains("ragged array"), "{err}");
}

/// Hazard 3, stated as a test rather than only as a doc comment: a timeout
/// around `run_async` resolves, and the work keeps going. There is no
/// `timeout` parameter on `run_async` precisely because it could not honour
/// its name — this asserts the documented behaviour is the real one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_timeout_does_not_cancel_the_call() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static FINISHED: AtomicUsize = AtomicUsize::new(0);

    let script = shared();
    let task = tokio::spawn(async move {
        let r = script
            .run_async(serde_json::json!({"h0": [[3,1],[2,4]]}))
            .await;
        FINISHED.fetch_add(1, Ordering::SeqCst);
        r
    });

    // A zero-length timeout on an already-spawned task: the timeout resolving
    // tells us nothing about whether the work stopped.
    let _ = tokio::time::timeout(std::time::Duration::from_nanos(1), async {}).await;

    let r = task.await.unwrap().unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        FINISHED.load(Ordering::SeqCst),
        1,
        "the call ran to completion"
    );
}
