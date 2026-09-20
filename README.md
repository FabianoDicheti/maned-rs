# maned-rs

Call `.mnd` scripts from Rust, in-process, via link-time FFI against `libmaned_c`.

## Read this first: linking and licensing

- **Dynamic-link only.** Static linking (`libmaned_c.a`) is not licensed — the maned
  `LICENSE` §3 grants redistribution of the *shared library* in unmodified form, and a
  static archive would also defeat the symbol gate. `build.rs` refuses a `static` link
  request by design.
- **The library must exist on the build machine.** A `.crate` cannot ship per-platform
  binaries, so install the `maned-capi-$VERSION-$target.tar.gz` tarball first, then
  `cargo add maned`. The opt-in `vendored` feature downloads it at build time — and makes
  the *built artifact* mixed-licence (the crate itself stays pure Apache/MIT).
- **Licence-audit tooling cannot see the native library.** `cargo-deny`/`cargo-about`
  walk the crate graph; `libmaned_c` is not in it. A compliance scan will report a clean
  Apache/MIT tree while a proprietary-but-redistributable binary sits in the final
  executable. Nothing dishonest is happening, but only this README says so.

```rust
let mut scoring = maned::load("scoring.mnd")?;
let result = scoring.run(&serde_json::json!({ "x": [[2, 0], [0, 2]] }))?;
let y = result.tensor("y")?;
```

**Status: scaffold only. Nothing here is functional.** This workspace tracks
`MANEDRS_PLAN.md` in the maned workspace; the binding is blocked on the maned_lang side
(the `mnd_*` C ABI, `libmaned_c` + SONAME + `maned_c.pc`, and `posix_spawnp` — MANEDPY
Phases 0–2b). All functions are `unimplemented!()` and `build.rs` emits no link
directives yet.

## Non-goals (v1)

- **Windows** — the serve and net layers are POSIX throughout.
- **Cancellation** — calls are uninterruptible; `tokio::time::timeout` around
  `run_async` resolves while the work continues, and dropping the runtime waits for it.

## Crates

- `maned-sys` — raw FFI (`links = "maned_c"`); `src/bindings.rs` will be bindgen output,
  checked in, never hand-edited.
- `maned` — the safe API: `Script` (`&mut self`, `Send`, `!Sync`) and `SharedScript`
  (`Sync`, only constructible for non-routed scripts) encode the thread-safety table in
  the type system.

Licensed `Apache-2.0 OR MIT`.
