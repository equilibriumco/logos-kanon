//! Public-mode pull: verify a RedStone payload inline, in a consumer program.
//!
//! Given a signed payload, a `(dataServiceId, feedId, signer set, M-of-N,
//! maxAge)` configuration supplied by the *consumer*, and the consumer's
//! expected `(base_asset, quote_asset)`, this returns a verified price and
//! timestamp or a typed error.
//!
//! # Not yet implemented
//!
//! Still to come: the public API over [`verifier_core`], and the assertions
//! that this crate carries no aggregator dependency, that the signer set comes
//! from the consumer and never from the payload, and that its typed errors
//! match the push path's. `TRACEABILITY.md` maps each to its task.
#![no_std]
#![forbid(unsafe_code)]

/// The verifier this crate delegates to. Re-exported so a consumer program
/// pins one verification implementation rather than two.
pub use verifier_core;
