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
//! `submit_price` is built, the admin gate is M2-06, `register_feed` is M2-07 and
//! `update_signer_set` is M2-08 — so a feed can be created, read and rotated,
//! which is the operating loop the servicing commitment needs. Deregistration
//! and pausing still refuse with their task: M2-09 and M2-10.
//!
//! [`register`] is registration: what a feed's configuration is, where its
//! account lives, and why the address is the feed id's (ADR 33). [`manage`] is
//! the admin-gated operations on a feed that already exists.
//! `TRACEABILITY.md` maps each to its requirement.
#![forbid(unsafe_code)]

pub mod admin;
pub mod feed_account;
pub mod instruction;
pub mod manage;
pub mod publish;
pub mod register;
pub mod submit;

pub use feed_account::FeedAccount;
pub use instruction::Instruction;
pub use manage::{update_signer_set, ManageError};
pub use register::{register_feed, RegisterError};
pub use submit::{submit_price, SubmitError};

pub use kanon_idl;
pub use verifier_core;
