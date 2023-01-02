//! Generated by `rust-bindgen`.
//!
//! We use `rust-bindgen` to generate bindings for a limited set of simpler
//! structures.

#![allow(
    dead_code,
    deref_nullptr,
    improper_ctypes,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals
)]

include!(concat!(env!("OUT_DIR"), "/bindgen.rs"));

use crate::error::AtticResult;
use crate::hash::Hash as RustHash;

impl Hash {
    /// Converts this into the native Rust version of this hash.
    pub fn into_rust(self) -> AtticResult<RustHash> {
        RustHash::from_ffi_hash(self)
    }
}

impl HashType {
    /// Returns the identifier of the hashing algorithm.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        }
    }
}