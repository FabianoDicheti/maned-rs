# Tests (MANEDRS_PLAN.md §6)

- `conformance.rs` will walk `tests/conformance/` from the capi tarball (gate 3).
- `compile_fail/` is the trybuild suite (gate 7): the thread-safety table as
  programs that must NOT compile — `Arc<Script>` crossing a thread, two
  simultaneous `&mut` borrows of one `Script`.

Both are blocked on the capi tarball existing.
