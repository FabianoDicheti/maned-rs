// Discovery per MANEDRS_PLAN.md §4.5 — not implemented yet, on purpose.
//
// When implementation starts, the resolution order is: MANED_LIB_DIR /
// MANED_INCLUDE_DIR -> pkg-config for maned_c -> conventional prefixes ->
// fail with a message naming every path tried. All branches key on TARGET /
// CARGO_CFG_TARGET_*, never the host (§5.7), and emit an rpath. A `static`
// link request is refused (§3.1); a `docsrs-stub` feature short-circuits to
// no-op directives for the sandboxed docs.rs builder.
//
// Until libmaned_c exists (MANEDPY_PLAN.md Phases 1-2b) this emits no link
// directives, so the stub crates compile without the library.
fn main() {
    println!("cargo:warning=maned-sys is a scaffold: libmaned_c does not exist yet; no link directives emitted");
    println!("cargo:rerun-if-env-changed=MANED_LIB_DIR");
}
