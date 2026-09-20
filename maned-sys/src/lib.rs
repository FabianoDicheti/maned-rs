//! Raw FFI for the `mnd_*` C ABI (MANEDRS_PLAN.md §4).
//!
//! `bindings.rs` will be `bindgen` output against `maned_c.h`, checked in and
//! never hand-edited — copy four of the ABI contract, kept honest by a
//! byte-comparison gate against a fresh `bindgen` run (§6, gate 2).

#![allow(non_camel_case_types, non_snake_case)]

include!("bindings.rs");
