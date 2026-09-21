//! Raw FFI for the `mnd_*` C ABI (MANEDRS_PLAN.md §4).
//!
//! `bindings.rs` is `bindgen` output against `maned_c.h`, checked in and
//! never hand-edited — copy four of the ABI contract, kept honest by a
//! byte-comparison gate against a fresh `bindgen` run (§6, gate 2). This
//! crate carries `links = "maned_c"`, so exactly one copy of it can appear in
//! a dependency graph and its `build.rs` metadata (`DEP_MANED_C_LIB_DIR`,
//! `DEP_MANED_C_INCLUDE_DIR`, `DEP_MANED_C_ABI_VERSION`) is readable by
//! direct dependents.
//!
//! Nothing here is safe. The safe API is the `maned` crate.

#![allow(non_camel_case_types, non_snake_case)]

include!("bindings.rs");

/// The `MND_ABI_VERSION` this crate's declarations were transcribed from.
///
/// Per the stability policy in `maned_c.h`, a binding must accept a **range**
/// of versions and never require an exact match; version `N-1` stays accepted
/// for one minor release cycle. [`abi_version_supported`] is that range.
pub const BINDINGS_ABI_VERSION: i32 = MND_ABI_VERSION as i32;

/// The oldest ABI generation these declarations still work against.
pub const MIN_SUPPORTED_ABI_VERSION: i32 = 1;

/// Is `runtime` an ABI generation this crate can talk to?
pub fn abi_version_supported(runtime: i32) -> bool {
    (MIN_SUPPORTED_ABI_VERSION..=BINDINGS_ABI_VERSION).contains(&runtime)
}

// ---------------------------------------------------------------------------
// The layout half of the ABI contract.
//
// Numbers below are transcribed from `maned_c_abi.json` in the capi tarball:
//   {"abi_version":1,"structs":{"mnd_response":{"sizeof":40,"fields":{
//     "status":0,"content_type":8,"body":16,"body_len":24,"notes":32}}}}
// If this test fails, `bindings.rs` and the library disagree about memory and
// every field read through it is garbage — do not "fix" it by editing the
// expectations (MANEDKT_PLAN.md §3.1).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod abi_layout {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};

    #[test]
    fn mnd_response_matches_maned_c_abi_json() {
        assert_eq!(size_of::<mnd_response>(), 40, "sizeof(mnd_response)");
        assert_eq!(offset_of!(mnd_response, status), 0, "offsetof status");
        assert_eq!(
            offset_of!(mnd_response, content_type),
            8,
            "offsetof content_type"
        );
        assert_eq!(offset_of!(mnd_response, body), 16, "offsetof body");
        assert_eq!(offset_of!(mnd_response, body_len), 24, "offsetof body_len");
        assert_eq!(offset_of!(mnd_response, notes), 32, "offsetof notes");
        // Not in the manifest, but implied by it: 8-byte alignment is the only
        // way those offsets can be right on a 64-bit target.
        assert_eq!(align_of::<mnd_response>(), 8, "alignof(mnd_response)");
    }

    #[test]
    fn flag_bits_match_the_header() {
        assert_eq!(MND_SYNTHESIZE_OUTPUTS, 1 << 0);
        assert_eq!(MND_NO_VM_FALLBACK, 1 << 1);
        assert_eq!(MND_ABI_VERSION, 1);
    }

    /// The linked library agrees, at runtime, with the header we transcribed —
    /// checked as a RANGE, never `== N` (stability policy, `maned_c.h`).
    #[test]
    fn linked_library_reports_a_supported_abi() {
        let runtime = unsafe { mnd_abi_version() };
        assert!(
            abi_version_supported(runtime),
            "libmaned_c reports ABI {runtime}, outside the supported range \
             {MIN_SUPPORTED_ABI_VERSION}..={BINDINGS_ABI_VERSION}"
        );
        let v = unsafe { core::ffi::CStr::from_ptr(mnd_version()) };
        assert!(!v.to_bytes().is_empty(), "mnd_version() returned \"\"");
    }
}
