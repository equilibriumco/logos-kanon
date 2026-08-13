//! RedStone data-package verification, shared by both modes.
//!
//! The push aggregator (`aggregator-program`) and the public-mode pull library
//! (`pull-lib`) both call this crate, and nothing else does the verifying,
//! which is what makes one audit cover both paths.
//!
//! # Not yet implemented
//!
//! The wire-format decoder ([`decode`]) and the primitive backend
//! ([`backend`]) are in place. The rest of the verification path is not:
//!
//! - M-of-N threshold enforcement
//! - a `TimeSource` over the LEZ clock account
//! - `maxAge` staleness and replay rejection
//! - asset-identity checks
//! - value sanity and scaling bounds
//! - a typed error enum for every failure mode
//!
//! `TRACEABILITY.md` maps each of these to the requirement it satisfies, the
//! task that delivers it and the tests that verify it.
//!
//! [`VerifierBackend`] is what keeps a future host precompile a localised
//! change: no caller reaches a signature primitive except through it.
#![no_std]
#![forbid(unsafe_code)]

pub mod backend;
pub mod decode;
pub mod error;
pub mod value;

pub use backend::{BackendError, Signature, SignerAddress, VerifierBackend};
pub use decode::{DataPackage, DataPoint, DecodeError, Payload};

/// This crate's version, for callers that record which verifier produced a
/// result.
///
/// It is also what the guest in `methods/guest` links against, so the
/// `riscv32` cross-compile of product code is exercised continuously rather
/// than first attempted once the verification path is complete.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
