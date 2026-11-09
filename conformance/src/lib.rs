#![cfg(feature = "agave-unstable-api")]

//! Cross-domain conformance harnesses for Agave.

#[cfg(feature = "ffi")]
pub mod block;
#[cfg(feature = "ffi")]
pub mod cost;
#[cfg(feature = "ffi")]
pub mod gossip;
#[cfg(feature = "ffi")]
pub mod shred;
#[cfg(feature = "ffi")]
pub mod txn;

#[cfg(feature = "ffi")]
pub use agave_conformance_ffi::{sol_compat_fini, sol_compat_init};
