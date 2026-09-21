//! feature = "tokio" (MANEDRS_PLAN.md §4.6).
//!
//! Only on SharedScript — spawn_blocking needs Send + 'static, `Arc<Script>` is
//! not Send, so routed scripts have no async API, correctly. spawn_blocking,
//! never a plain async body and never block_in_place. No timeout parameter:
//! dropping a JoinHandle does not stop the task, so a timeout could not honour
//! its name (hazard 3).

use crate::{Error, Response, SharedScript};
use serde::Serialize;
use std::sync::Arc;

impl SharedScript {
    /// `POST /run.json` on a blocking-pool thread.
    ///
    /// Takes `Arc<Self>` and an **owned** payload because that is what
    /// `spawn_blocking` requires (`Send + 'static`), and it falls out of §4.3
    /// rather than being a decision: `Arc<Script>` is not `Send`, so a routed
    /// script cannot reach this API at all — which is correct, because it is
    /// not concurrent anyway. Anyone who wants a routed script behind an async
    /// interface can hold an `Arc<Mutex<Script>>` and know they have
    /// serialized it.
    ///
    /// # This is not cancellable, and `spawn_blocking` makes that worse
    ///
    /// - Dropping the future does **not** stop the work. A
    ///   `tokio::time::timeout` around this resolves while `mnd_request` keeps
    ///   running — the "looks handled" failure. There is deliberately no
    ///   `timeout` parameter here, because it could not honour its name.
    /// - `Runtime::drop` **waits** for blocking tasks, so a long matmul turns
    ///   process shutdown into an apparent hang. Reach for
    ///   [`tokio::runtime::Runtime::shutdown_timeout`] if that matters.
    /// - Real cancellation needs `mnd_cancel` plus an interpreter poll point.
    ///   Out of scope for v1, exactly as in the Python and Kotlin bindings.
    ///
    /// Never `block_in_place` and never a plain `async fn` body: a blocking
    /// `mnd_request` on a tokio worker thread stalls every other task on that
    /// worker.
    #[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
    pub async fn run_async<T: Serialize + Send + 'static>(
        self: Arc<Self>,
        payload: T,
    ) -> Result<Response, Error> {
        // Response is Send (see response.rs) — that impl is what lets the
        // result come back out of the blocking pool at all.
        match tokio::task::spawn_blocking(move || self.run(&payload)).await {
            Ok(result) => result,
            Err(join) => Err(Error::Library(format!(
                "the blocking task running mnd_request did not complete: {join}"
            ))),
        }
    }
}
