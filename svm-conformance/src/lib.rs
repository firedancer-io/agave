//! Conformance harnesses for the Solana SVM.
//!
//! Houses the `sol_compat_*` FFI entrypoints that Firedancer's conformance
//! tooling links against to exercise Agave's execution layer.

#[cfg(feature = "ffi")]
pub mod elf_loader;
#[cfg(feature = "ffi")]
pub mod instr;
#[cfg(feature = "ffi")]
pub mod serialization;
#[cfg(feature = "ffi")]
pub mod syscall;
#[cfg(feature = "ffi")]
pub mod txn;

#[cfg(feature = "ffi")]
pub use agave_conformance_ffi::{sol_compat_fini, sol_compat_init};
