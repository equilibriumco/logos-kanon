//! Push-mode aggregator: verify on the write side, publish the result.
//!
//! A public-mode LEZ program that accepts signed RedStone data packages,
//! verifies them through [`verifier_core`], and writes the verified price into
//! the canonical RFP-019 price account that consumers read. Feed registration,
//! signer-set updates and deregistration are admin-gated per RFP-001.
//!
//! [`publish`] is the write itself: what a verified feed puts in each of the
//! canonical account's six fields, and when an existing account may be updated.
//!
//! # Not yet implemented
//!
//! Still to come: the SPEL skeleton and IDL surface, the single-transaction
//! `submit_price` path that calls [`publish`], and the admin-gated
//! instructions. `TRACEABILITY.md` maps each to its task.
#![forbid(unsafe_code)]

pub mod publish;

pub use kanon_idl;
pub use verifier_core;
