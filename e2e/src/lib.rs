//! Nothing ships from here. The crate exists so `tests/` has a target to hang
//! off, and so the client half of LEZ resolves in a workspace of its own --
//! `Cargo.toml` records why that matters.
//!
//! The chain plumbing both suites share is `tests/support/chain.rs`, included by
//! path rather than declared here. That keeps the sequencer's RPC client and the
//! Bedrock node behind it on dev-dependency edges, which is where `[M2-19:01]`
//! put them and where the licence gate expects to find them.
