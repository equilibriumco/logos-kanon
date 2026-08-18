//! IDL artefacts for the aggregator and the canonical price account.
//!
//! The canonical account is defined upstream, not here. Logos publishes it as
//! [`OraclePriceAccount`] in `logos-blockchain/lez-programs`, and RFP-019
//! requires external adaptors to populate that struct rather than inventing
//! one. This crate re-exports it: re-exported, not forked. Nothing here
//! redefines a field.
//!
//! `reference/twap_oracle-idl.json` is the published IDL artefact, vendored at a
//! named commit. It is not a second source of truth — the type above is — but it
//! is what lets a test notice if the two ever stop agreeing, which is the whole
//! job of the conformance test alongside it.
//!
//! # Not yet implemented
//!
//! What this crate re-exports is settled. Still to come: the aggregator's own
//! IDL surface, emitting it as a standalone artefact, and the write path into
//! the account described here. `TRACEABILITY.md` maps each to its task.
#![forbid(unsafe_code)]

/// The canonical LEZ oracle price account, re-exported from the crate that
/// defines it.
///
/// Six fields: `base_asset`, `quote_asset`, `price`, `timestamp`, `source_id`
/// and `confidence_interval`. Producers own how it is written and consumers only
/// read and validate, so the push aggregator writes this and the
/// aggregator-read reference consumer reads it.
pub use twap_oracle_core::OraclePriceAccount;

/// The account identifier type the price account's three id fields are made of,
/// re-exported from where the account is defined.
///
/// Here so that writing a price account needs one dependency rather than two,
/// and so that the type a Kanon caller constructs is the type upstream declares
/// rather than one that happens to have the same shape.
pub use lee_core::account::AccountId;

/// Fractional bits in [`OraclePriceAccount::price`], re-exported from upstream.
///
/// The account carries no exponent, so this constant is the whole of what a
/// consumer needs in order to read the field: the real price is
/// `price / 2^PRICE_FRACTIONAL_BITS`, and a token amount converts with
/// `(amount * price) >> PRICE_FRACTIONAL_BITS`. Re-exported rather than
/// restated as 64, because a convention two writers have to agree on should
/// have one definition and it should be upstream's.
pub use twap_oracle_core::PRICE_FRACTIONAL_BITS;

/// The vendored reference IDL, so the conformance test reads it from the binary
/// rather than reaching for a path relative to an unpredictable working
/// directory.
pub const REFERENCE_TWAP_ORACLE_IDL: &str = include_str!("../reference/twap_oracle-idl.json");

/// The fields the published IDL declares for the price account, in order.
///
/// Duplicated from the artefact deliberately: if this list and the artefact
/// disagree, one of them changed, and that is exactly what the test wants to
/// catch. Keeping it here rather than deriving it is what makes the assertion
/// mean something.
pub const PRICE_ACCOUNT_FIELDS: [(&str, &str); 6] = [
    ("base_asset", "account_id"),
    ("quote_asset", "account_id"),
    ("price", "u128"),
    ("timestamp", "u64"),
    ("source_id", "account_id"),
    ("confidence_interval", "u128"),
];
