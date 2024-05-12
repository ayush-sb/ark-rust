//! # Data Resource
//!
//! `data-resource` is a crate for managing resource identifiers. It provides different
//! implementations of resource identifiers ([`ResourceId`]) based on various hash algorithms.
//!
//! ## Features
//!
//! - `non-cryptographic-hash`: Enables the use of a non-cryptographic hash function to define `ResourceId`.
//! - `cryptographic-hash`: Enables the use of cryptographic hash functions to define `ResourceId`.
//!
//! By default, `cryptographic-hash` feature is enabled.

use core::{fmt::Display, str::FromStr};
use data_error::Result;
use serde::Serialize;
use std::{fmt::Debug, hash::Hash, path::Path};

cfg_if::cfg_if! {
    if #[cfg(feature = "hash_crc32")] {
        mod crc32;
        pub use crc32::ResourceId;
    } else if #[cfg(feature = "hash_blake3")] {
        mod blake3;
        pub use blake3::ResourceId;
    } else {
        compile_error!("At least one of the hash functions must be enabled");
    }
}

/// This trait defines a generic type representing a resource identifier.
///
/// Resources are identified by a hash value, which is computed from the resource's data.
/// The hash value is used to uniquely identify the resource.
///
/// Implementors of this trait must provide a way to compute the hash value from the resource's data.
pub trait ResourceIdTrait: Debug
+ Display //todo: I guess this chain of traits can be coded in a nicer way
+ FromStr
+ Clone
+ PartialEq
+ Eq
+ Ord
+ PartialOrd
+ Hash
+ Serialize {
    /// Computes the resource identifier from the given file path
    fn from_path<P: AsRef<Path>>(file_path: P) -> Result<Self>;

    /// Computes the resource identifier from the given bytes
    fn from_bytes(data: &[u8]) -> Result<Self>;
}
