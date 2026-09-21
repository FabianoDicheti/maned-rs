// MANEDRS_PLAN.md §4.3, third row of the table: two handles on the same
// routed device race that device's single-connection listener, so a Script
// must not reach a second thread while it is still shareable.
//
// `Arc<T>: Send` requires `T: Send + Sync`. `Script` is `Send` but deliberately
// `!Sync`, so `Arc<Script>` is `!Send` and this does not compile. The only
// door to a shareable handle is `into_shared()`, which checks
// `mnd_script_is_routed()` as it opens.
//
// Two notes on the shape of this file:
//
//  - The plan's own sketch put the `Arc::new` *inside* the closure. That
//    version compiles — moving a `Script` into a thread is exactly what `Send`
//    permits — so the Arc has to be built on this side of the boundary for the
//    test to test anything.
//  - The bound is spelled with a local helper rather than `thread::spawn` so
//    that the recorded diagnostic names only files in this project. Going
//    through `spawn` produces the same error plus a note pointing into the
//    standard library's own source tree, whose path differs per toolchain
//    build and would make this expectation unportable.

use std::sync::Arc;

fn goes_to_another_thread<T: Send>(_value: T) {}

fn main() {
    let script = maned::load("routed.mnd").unwrap();
    let shared: Arc<maned::Script> = Arc::new(script);

    goes_to_another_thread(shared);
}
