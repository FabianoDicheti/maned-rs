# Tests (MANEDRS_PLAN.md §6)

The plan's tree drew a `tests/` at the workspace root. Cargo integration tests
have to live inside a package, so they are in **`maned/tests/`** — the same
files, one directory deeper:

| File | Gate | Proves |
|---|---|---|
| `maned/tests/conformance.rs` | 3 | the shared fixture corpus, byte for byte, plus the status→variant map and a structural `/health` assertion |
| `maned/tests/compile_fail.rs` + `compile_fail/` | 7 | the thread-safety table, at compile time: `Arc<Script>` crossing a thread and two simultaneous `run` borrows both fail to build |
| `maned/tests/threads.rs` | 7 (positive) | `SharedScript` across 8 threads × 25 calls, every response byte-identical — `testConcurrentHandleIsIdenticalGap025` ported |
| `maned/tests/soak.rs` | 6 | 10 000 open/request/free cycles, plain; run it under LeakSanitizer or watch RSS for the real gate |
| `maned/tests/shutdown.rs` | §5.5 | `maned::shutdown()` closes live handles and leaves them inert; its own binary, because it makes process-wide claims |
| `maned/tests/script_api.rs` | §4.1–4.4 | flags, the routed `into_shared` refusal, `decimal()` at `quantres > 0`, the byte body, `find()`'s miss message |
| `maned/tests/async_api.rs` | §4.6 | `run_async` under `--features tokio`, including that a timeout does not cancel |
| `maned-sys` unit tests | MANEDKT §3.1 | `sizeof`/`offsetof` against `maned_c_abi.json`, and the linked library's ABI version as a *range* |

Everything needs the capi artifact. `.cargo/config.toml` points at
`../capi-freeze`; override with `MANED_LIB_DIR` and `MANED_CONFORMANCE_DIR`.

    cargo test
    cargo test --features tokio

Regenerate the trybuild expectations after a compiler upgrade with
`TRYBUILD=overwrite cargo test --test compile_fail`, and read the diff before
committing it.

Gates not covered here, and where they belong: gate 1, 8 and 12 are in the
maned repo; gate 2 (`bindings.rs` == fresh `bindgen`) needs `bindgen`
installed; gate 4 (three-way cross-binding parity) needs manedpy and
maned-kotlin present; gate 5 needs `maned-run`; gate 10 needs a second
installed rustup target; gate 11 is a manual `process::exit` check.
