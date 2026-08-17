//! RedStone data-package verification, shared by both modes.
//!
//! The push aggregator (`aggregator-program`) and the public-mode pull library
//! (`pull-lib`) both call this crate, and nothing else does the verifying,
//! which is what makes one audit cover both paths.
//!
//! # What a verification decides
//!
//! [`verify_feed`] walks a payload once and answers with a price or with a
//! reason. Decoding ([`decode`]) and the primitives ([`backend`]) are the two
//! steps below it; asset identity, value sanity, the staleness window read
//! through [`time::TimeSource`], the M-of-N threshold and the conversion to the
//! price account's scale are the decisions above it.
//!
//! `TRACEABILITY.md` maps each of those to the requirement it satisfies, the
//! task that delivered it and the tests that verify it, and `COSTS.md` to what
//! it costs.
//!
//! [`VerifierBackend`] is what keeps a future host precompile a localised
//! change: no caller reaches a signature primitive except through it.
#![no_std]
#![forbid(unsafe_code)]

pub mod backend;
pub mod decode;
pub mod error;
pub mod feed;
pub mod time;
pub mod value;

#[cfg(test)]
mod properties;
#[cfg(test)]
mod test_support;

pub use backend::{BackendError, Signature, SignerAddress, VerifierBackend};
pub use decode::{DataPackage, DataPoint, DecodeError, Payload};
pub use error::{ConfigError, VerifyError};
pub use feed::{verify_feed, FeedConfig, VerifiedFeed, MAX_SIGNERS};
pub use value::{median, Value};

/// This crate's version, for callers that record which verifier produced a
/// result.
///
/// It is also what the guest in `methods/guest` links against, so the
/// `riscv32` cross-compile of product code is exercised continuously rather
/// than first attempted once the verification path is complete.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
