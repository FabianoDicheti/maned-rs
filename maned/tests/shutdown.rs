//! Hazard 5 — `maned::shutdown()` and the registry behind it.
//!
//! A test binary of its own, and deliberately a **single** `#[test]`:
//! `shutdown()` closes every live handle in the process, so it cannot share a
//! process with tests that are holding one. Cargo gives each integration test
//! file its own binary, which is exactly the isolation this needs.

fn corpus(name: &str) -> std::path::PathBuf {
    let dir = std::env::var("MANED_CONFORMANCE_DIR")
        .unwrap_or_else(|_| "/Users/fabianodicheti/Documents/maned/capi-freeze/conformance".into());
    std::path::PathBuf::from(dir).join(name)
}

#[test]
fn shutdown_closes_live_handles_and_leaves_them_inert() {
    let path = corpus("01_local_matmul.mnd");
    let payload = serde_json::json!({"h0": [[3,1],[2,4]]});

    assert_eq!(maned::live_handles(), 0, "a fresh process owns nothing");

    // Drop is the normal path and it deregisters.
    {
        let _a = maned::load(&path).unwrap();
        let _b = maned::load(&path).unwrap();
        assert_eq!(maned::live_handles(), 2);
    }
    assert_eq!(maned::live_handles(), 0, "Drop deregisters");

    // A SharedScript inside an Arc is still one registered handle.
    let shared = std::sync::Arc::new(maned::load(&path).unwrap().into_shared().unwrap());
    let mut owned = maned::load(&path).unwrap();
    assert_eq!(maned::live_handles(), 2);
    assert_eq!(owned.run(&payload).unwrap().status(), 200);

    maned::shutdown();
    assert_eq!(maned::live_handles(), 0, "shutdown drains the registry");

    // Inert, not dangling: the handle was swapped to NULL under both owners,
    // so a call after shutdown is an error rather than a use-after-free.
    let err = owned.run(&payload).unwrap_err().to_string();
    assert!(err.contains("already closed"), "{err}");
    assert!(err.contains("shutdown"), "{err}");
    let err = shared.run(&payload).unwrap_err().to_string();
    assert!(err.contains("already closed"), "{err}");

    // And dropping them afterwards must not close anything twice — if the
    // swap-to-NULL were not atomic this is where the double free would land.
    drop(owned);
    drop(shared);
    assert_eq!(maned::live_handles(), 0);

    // shutdown() is idempotent, including on an empty registry.
    maned::shutdown();
    maned::shutdown();

    // The process is still usable afterwards: shutdown closes handles, it does
    // not unload the library.
    let mut fresh = maned::load(&path).unwrap();
    assert_eq!(fresh.run(&payload).unwrap().status(), 200);
    assert_eq!(maned::live_handles(), 1);
}
