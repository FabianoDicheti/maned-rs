//! Gate 7 — the thread-safety table as programs that must NOT compile.
//!
//! `trybuild` asserts that each file in `compile_fail/` fails to compile with
//! the error recorded beside it in `.stderr`. This has no analogue in the
//! Python or Kotlin bindings, and it is the most interesting gate in the plan:
//! the table stops being documentation and becomes a test that fails when
//! somebody weakens an impl. Add an `unsafe impl Sync for Script` and this
//! suite goes red without anyone having to notice.
//!
//! The positive half — eight threads, twenty-five calls, identical bytes —
//! is in `threads.rs`.
//!
//! Note: `.stderr` files are rustc output, so a compiler upgrade can reword
//! them. Regenerate with `TRYBUILD=overwrite cargo test --test compile_fail`
//! and read the diff before committing it.

#[test]
fn the_thread_safety_table_is_enforced_by_the_compiler() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
