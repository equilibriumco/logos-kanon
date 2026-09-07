//! Figures more than one cost test needs, in one place.
//!
//! Included by path rather than shared through a crate, the way
//! `verifier-core/tests/support/vectors.rs` is: each file under `tests/` is its
//! own crate, so a constant two of them need is otherwise retyped, and a retyped
//! constant is one nothing notices going stale. `cost.rs`, `read_cost.rs` and
//! `bytes.rs` all include this.

#![allow(dead_code)] // Each consumer reads a different subset of the figures.

/// LEZ's per-transaction execution budget.
///
/// `MAX_NUM_CYCLES_PUBLIC_EXECUTION`, which is a private `const` in LEZ and so
/// cannot be imported — this is a transcription, and nothing here compares it
/// against the pin. It was `1024 * 1024 * 32` at v0.2.0 and v0.2.1, and the file
/// holding it moved between those two releases, so the number has been stable
/// while its location has not.
///
/// P1 claims a transaction fits "the budget in force at delivery", so re-reading
/// this out of LEZ is part of moving the pin rather than something to do
/// afterwards. `m0/versions.md` records it in the steps for a pin move.
pub const CYCLE_BUDGET: u64 = 33_554_432;

/// The two rows a native ECDSA and keccak256 precompile would remove from a
/// five-signer verification.
///
/// `cost.rs` measures these as part of its component table and
/// `the_shared_figures_match_the_component_table` holds this copy against that
/// measurement, so the two cannot drift.
pub const FIVE_SIGNER_KECCAK: u64 = 87_380;
pub const FIVE_SIGNER_RECOVERY: u64 = 2_922_890;

/// What those two come to together.
pub const REMOVABLE: u64 = FIVE_SIGNER_KECCAK + FIVE_SIGNER_RECOVERY;
