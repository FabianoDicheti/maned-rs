//! Response and Tensor (MANEDRS_PLAN.md §4.2).
//!
//! body() borrows the C allocation, zero copy — the one place this binding is
//! faster than both predecessors. body is bytes, never CStr: the ABI documents
//! body_len bytes that may contain NULs, and a CStr would truncate a PNG at
//! its first zero byte. decimal() hands back the engine's printed text
//! verbatim; never enable serde_json's arbitrary_precision from a library.

use crate::Error;
use serde::de::DeserializeOwned;

pub struct Response {
    _private: (),
}

impl Response {
    pub fn status(&self) -> u16 {
        unimplemented!("{}", crate::BLOCKED)
    }
    pub fn body(&self) -> &[u8] {
        unimplemented!("{}", crate::BLOCKED)
    }
    pub fn body_str(&self) -> Result<&str, std::str::Utf8Error> {
        unimplemented!("{}", crate::BLOCKED)
    }
    pub fn content_type(&self) -> &str {
        unimplemented!("{}", crate::BLOCKED)
    }
    pub fn notes(&self) -> &[String] {
        unimplemented!("{}", crate::BLOCKED)
    }
    pub fn json(&self) -> Result<serde_json::Value, Error> {
        unimplemented!("{}", crate::BLOCKED)
    }
    pub fn deserialize<T: DeserializeOwned>(&self) -> Result<T, Error> {
        unimplemented!("{}", crate::BLOCKED)
    }
    pub fn tensor(&self, _name: &str) -> Result<Tensor, Error> {
        unimplemented!("{}", crate::BLOCKED)
    }
    /// The exact bytes the engine printed at quantres > 0 — caller's choice of
    /// decimal type.
    pub fn decimal(&self, _name: &str) -> Result<&str, Error> {
        unimplemented!("{}", crate::BLOCKED)
    }
}

/// i64 <-> int64_t round-trips losslessly (include/maned/interp/value.h:23-24).
pub struct Tensor {
    pub shape: Vec<i64>,
    pub data: Vec<i64>,
}
