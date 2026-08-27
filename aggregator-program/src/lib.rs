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
//! [`submit`] is the submission path: what one instruction does with a payload,
//! a registered feed and the clock, from the checks it runs to the account
//! states it hands back.
//!
//! [`instruction`] is the instruction set and [`feed_account`] the state a
//! registered feed keeps. The SPEL program that dispatches on them is the guest
//! binary at `methods/guest/src/bin/aggregator.rs`, because that is what LEZ
//! executes; it delegates here, which is what makes the logic host-testable.
//!
//! # Not yet implemented
//!
//! The admin-gated instruction bodies. `submit_price` is built; the rest still
//! refuse with the task that fills them — the admin gate is M2-06, and
//! registration, signer-set updates, deregistration and pausing are M2-07 to
//! M2-10. Until M2-07 lands there is no in-repository way to register the feed
//! `submit_price` reads, so the push path is testable and not yet operable.
//! `TRACEABILITY.md` maps each to its requirement.
#![forbid(unsafe_code)]

pub mod feed_account;
pub mod instruction;
pub mod publish;
pub mod submit;

pub use feed_account::FeedAccount;
pub use instruction::Instruction;
pub use submit::{submit_price, SubmitError};

pub use kanon_idl;
pub use verifier_core;
