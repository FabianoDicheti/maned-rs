// The rpath, re-emitted (MANEDRS_PLAN.md §4.5).
//
// `cargo:rustc-link-search` and `cargo:rustc-link-lib` from maned-sys
// propagate to every dependent's final link. `cargo:rustc-link-arg` does NOT:
// it applies only to the targets of the package whose build script emitted
// it. So maned-sys's rpath covers maned-sys's own test binary and nothing
// else, and this three-line build script is what keeps `maned`'s integration
// tests and examples loadable — libmaned_c's install_name is
// @rpath/libmaned_c.dylib, so without an LC_RPATH they link fine and die at
// startup, which is exactly the failure §4.5 calls out.
//
// The directory comes from `DEP_MANED_C_LIB_DIR`, which cargo sets for direct
// dependents of a `links = "maned_c"` crate — the metadata channel that is the
// whole reason for splitting the -sys crate out.
//
// A downstream binary crate that depends on `maned` does NOT get DEP_* vars
// (they reach direct dependents only) and will need the same two lines of its
// own, or an install into a directory already on the loader's path. The README
// says so.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=DEP_MANED_C_LIB_DIR");

    let Ok(dir) = std::env::var("DEP_MANED_C_LIB_DIR") else {
        // docsrs-stub, or any other build where maned-sys emitted no link
        // directives. Nothing to point at, nothing to do.
        return;
    };

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    if target_os == "macos" || target_os == "ios" {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path/../lib");
    } else {
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib");
    }
}
