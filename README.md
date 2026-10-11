![the maned mark, geared](https://raw.githubusercontent.com/FabianoDicheti/maned-rs/master/docs/img/logo.png)

# maned-rs

Call `.mnd` scripts from Rust, in-process, via link-time FFI against `libmaned_c`.

```rust
let mut scoring = maned::load("scoring.mnd")?;
let result = scoring.run(&serde_json::json!({ "x": [[2, 0], [0, 2]] }))?;
let y = result.tensor("y")?;        // Tensor { shape: [2, 2], data: [...] }
```

One entry point in the engine — `mnd_request` — answers the same
request→response contract as `maned-serve`'s REST API, with no socket and no
HTTP framing. A response here is byte-identical to what the REST API returns
for the same script and body, and the test suite walks the shared conformance
corpus to prove it.

## Quickstart

Step by step, from nothing to a running program:

1. Install the engine library — download `maned-capi-0.4.0-<target>.tar.gz`
   from https://github.com/FabianoDicheti/maned/releases, unpack it, and
   either copy `lib/` + `include/` into a standard prefix (`/usr/local`)
   or remember its path for step 4.

2. Create a project and add the crate (published on crates.io):

```sh
cargo new scoring && cd scoring
cargo add maned serde_json
```

3. Save the script as `scoring.mnd` and put this in `src/main.rs`:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut script = maned::load("scoring.mnd")?;
    let result = script.run(&serde_json::json!({"x": [[2, 0], [0, 2]]}))?;
    let y = result.tensor("y")?;
    println!("y = shape {:?}, data {:?}", y.shape, y.data);
    Ok(())
}
```

4. Build and run. With the tarball in a standard prefix, plain `cargo run`
   works; with a custom location, name it:

```sh
$ MANED_LIB_DIR=/path/to/capi/lib MANED_INCLUDE_DIR=/path/to/capi/include cargo run
y = shape [2, 2], data [2, 4, 0, 2]
```

(If the binary dies at startup with `Library not loaded: @rpath/...`, read
[Downstream binaries need the rpath too](#downstream-binaries-need-the-rpath-too).)

The same program lives in this repo as `maned/examples/scoring.rs` —
`cargo run --example scoring`. Three deep dives sit beside it:
`--example files` (file routes), `--example decimals_tokio --features tokio`
(exact decimals + async), `--example routed` (the local Bark VM).

## Read this first: linking and licensing

- **Dynamic-link only.** Static linking (`libmaned_c.a`) is not licensed — the maned
  `LICENSE` §3 grants redistribution of the *shared library* in unmodified form, and a
  static archive would also defeat the symbol gate. `build.rs` refuses a `static` link
  request by design, at length.
- **The library must exist on the build machine.** A `.crate` cannot ship per-platform
  binaries, so install the `maned-capi-$VERSION-$target.tar.gz` tarball first, then
  `cargo add maned`. The opt-in `vendored` feature (not implemented yet) would download it
  at build time — and would make the *built artifact* mixed-licence, while the crate
  itself stays pure Apache/MIT.
- **Licence-audit tooling cannot see the native library.** `cargo-deny`/`cargo-about`
  walk the crate graph; `libmaned_c` is not in it. A compliance scan will report a clean
  Apache/MIT tree while a proprietary-but-redistributable binary sits in the final
  executable. Nothing dishonest is happening, but only this README says so.

## Building and testing

`maned-sys/build.rs` resolves the library in this order, always keyed on the
**target** and never the host:

1. `MANED_LIB_DIR` / `MANED_INCLUDE_DIR` — explicit, wins over everything.
2. `pkg-config --libs maned_c`, honouring `PKG_CONFIG_PATH` and
   `PKG_CONFIG_SYSROOT_DIR`. Cross builds need `PKG_CONFIG_ALLOW_CROSS=1`.
3. `/usr/local/lib`, `/opt/homebrew/lib`, `$HOME/.local/lib`.
4. Otherwise the build fails with a message naming every path tried, the
   tarball to install and the `MANED_LIB_DIR` override.

This checkout points at the capi artifact beside it through `.cargo/config.toml`,
so the whole suite runs with no arguments:

    cargo test                  # default features
    cargo test --features tokio # + the async surface

Anything else, point it at an unpacked tarball:

    MANED_LIB_DIR=/path/to/maned-capi/lib \
    MANED_CONFORMANCE_DIR=/path/to/maned-capi/conformance \
    cargo test

**MSRV is 1.77**, not the 1.65 the plan proposed. The layout gate uses
`core::mem::offset_of!`, stable in 1.77, and asserting the `mnd_response`
offsets from `maned_c_abi.json` is worth more than twelve versions of
compiler compatibility. Everything else in the crate would build on 1.65.

### Downstream binaries need the rpath too

`cargo:rustc-link-search` and `-lmaned_c` propagate through the dependency
graph; `cargo:rustc-link-arg` does not — it only ever applies to the targets of
the package that emitted it. `libmaned_c`'s install name is
`@rpath/libmaned_c.dylib`, so a binary that links it without an `LC_RPATH`
builds fine and dies at startup. `maned-sys` covers its own tests, `maned`'s
`build.rs` covers `maned`'s, and a downstream binary crate wants one of:

- a `build.rs` of its own with
  `println!("cargo:rustc-link-arg=-Wl,-rpath,{}", std::env::var("MANED_LIB_DIR")?);`
- or the library installed somewhere the loader already looks.

### The `docsrs-stub` feature is mutually exclusive with linking

It exists so docs.rs — sandboxed, no network, no library — can render the API:
`build.rs` short-circuits to no link directives. `cargo test --all-features`
therefore does **not** work; use `--features tokio`.

## The file routes

`run()` goes to `/run.json`, which renders an `out::` image as a nested integer
array and a csv as a JSON tensor. For the engine's actual file bytes there is
the multipart route, and it takes file bytes on the way in too — a `csv()` /
`image()` / `parquet()` `in::` never has to be decoded into JSON first.

| Call | Route | Returns |
|---|---|---|
| `script.run_files(&[("h0", bytes)])` | `POST /run` multipart | `BTreeMap<String, Vec<u8>>`, one entry per `out::` |
| `script.frame(&payload)` | `POST /frame` | `Response` — the frame document *is* the body |
| `script.frame_csv(&payload)` | `POST /frame.csv` | `Vec<u8>`, undecoded |
| `script.inputs()` / `.outputs()` / `.routes()` / `.health()` | `GET /health` | the contract, fetched once and cached |

`maned::multipart::{encode, decode, boundary_for}` are public for anyone driving
`request()` by hand. The encoding is byte-identical to manedpy's and
maned-kotlin's — `tests/io.rs` asserts it against a frozen body, so a drift is
red rather than merely different.

`SharedScript` has the same four calls plus `run_files_async` / `frame_async`
under the `tokio` feature.

![a maned wolf with a crab](https://raw.githubusercontent.com/FabianoDicheti/maned-rs/master/docs/img/wolf.png)

## Non-goals (v1)

- **Windows** — the serve and net layers are POSIX throughout. `build.rs`
  refuses the target explicitly rather than failing obscurely.
- **Cancellation** — calls are uninterruptible; `tokio::time::timeout` around
  `run_async` resolves while the work continues, and dropping the runtime waits
  for it. There is deliberately no `timeout` parameter, because it could not
  honour its name.
- **`static` linking** — see above.

## Crates

- `maned-sys` — raw FFI (`links = "maned_c"`); `src/bindings.rs` is destined to
  be bindgen output, checked in, never hand-edited. Its unit test asserts
  `size_of::<mnd_response>()` and every field offset against the tarball's
  `maned_c_abi.json`.
- `maned` — the safe API. `Script` (`&mut self`, `Send`, `!Sync`) and
  `SharedScript` (`Sync`, only constructible for non-routed scripts) encode the
  thread-safety table in the type system; `tests/compile_fail/` asserts the
  programs that must *not* compile.

## Threads, in one table

| | Safe? | How the crate says so |
|---|---|---|
| Concurrent calls, one handle, non-routed | yes | `SharedScript: Sync` |
| Concurrent calls, one handle, **routed** | **no** | `&mut self` on `Script::run` |
| Two handles, same routed device | **no** | `Script: !Sync` ⇒ `Arc<Script>: !Send` |

`into_shared()` is the only door from the second row to the first, and it
checks `mnd_script_is_routed()` as it opens. On a routed script it hands the
`Script` back inside the error, so refusing to share it does not destroy it.

Licensed `Apache-2.0 OR MIT`.
