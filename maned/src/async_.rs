//! feature = "tokio" (MANEDRS_PLAN.md §4.6).
//!
//! Only on SharedScript — spawn_blocking needs Send + 'static, Arc<Script> is
//! not Send, so routed scripts have no async API, correctly. spawn_blocking,
//! never a plain async body and never block_in_place. No timeout parameter:
//! dropping a JoinHandle does not stop the task, so a timeout could not honour
//! its name (hazard 3).

use crate::{Error, Response, SharedScript};
use serde::Serialize;
use std::sync::Arc;

impl SharedScript {
    pub async fn run_async<T: Serialize + Send + 'static>(
        self: Arc<Self>,
        _payload: T,
    ) -> Result<Response, Error> {
        unimplemented!("{}", crate::BLOCKED)
    }
}
