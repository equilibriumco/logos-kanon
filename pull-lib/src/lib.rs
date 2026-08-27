//! Public-mode pull: verify a RedStone payload inline, in a consumer program.
//!
//! One function. Given a signed payload, the configuration the *consumer* holds,
//! the pair the consumer expects, and the LEZ clock account, it returns a
//! verified price or a typed reason.
//!
//! Nothing here decides anything about a payload. [`verifier_core`] does all of
//! it, which is what makes one audit cover both modes (ADR 2) and what makes
//! this crate's independence from the aggregator structural rather than
//! reviewed: there is no aggregator type in the signature and no code path that
//! could reach one.
//!
//! # What a consumer supplies
//!
//! The signer set, the threshold and `maxAge` come from the consumer and never
//! from the payload (SEC1). That is the whole security model of pull mode: a
//! payload is a set of signed claims, and which signatures count is the
//! consumer's decision.
//!
//! **The `dataServiceId` is carried and authenticates nothing.** RFP-020 puts it
//! in this configuration and it is there, on [`PullConfig`]. What it cannot do is
//! verify anything: `dataServiceId` is a key into RedStone's gateway and their
//! signer registry, not a field on the wire. A data package carries a feed id and
//! a value, and the only variable slot in the envelope sits outside every
//! signature, where whoever assembles the payload chooses it.
//!
//! The signer set is the binding. Those addresses *are* the data service, and a
//! package signed by anyone else is [`VerifyError::UnauthorisedSigner`] whatever
//! service it might claim to come from. So the label is here for the consumer's
//! own benefit — naming the service its roster came from, in its logs and its
//! own error paths — and verification never reads it. `FEEDS.md` records which
//! addresses served which service when this was measured, and ADR 30 records why
//! the set stays per feed.
//!
//! # The clock is not a parameter, and what that does and does not buy
//!
//! [`verify_price`] takes the clock *account* and builds the reading itself,
//! refusing any account but the pinned every-block one (ADR 13), rather than
//! accepting a `TimeSource` the consumer supplies.
//!
//! What that establishes: the consumer has named the every-block account and not
//! one of the other two, which hold real timestamps up to ten and fifty blocks
//! stale and are therefore the plausible mistake rather than an obvious one.
//!
//! **What it cannot establish is that the bytes came from that account.** The id
//! and the data arrive as two independent slices, so a consumer that passes
//! `CLOCK_ACCOUNT_ID` alongside sixteen bytes of its own choosing is believed.
//! Binding the two is only possible where a program meets its dispatcher, in the
//! `AccountWithMetadata` the chain hands it — and that type belongs to a `std`
//! crate this one cannot depend on, which is the same constraint that makes
//! `kanon-clock` declare the layout rather than import it.
//!
//! So the residual is real and it is the consumer's: read the clock from the
//! account the transaction supplied, and do not synthesise it. The push
//! aggregator has no such residual, because the dispatcher hands it an account
//! whose id and data are fields of one struct. `reference-consumers/pull` is
//! where the account-side half is demonstrated (M3-06); nothing in this crate can
//! check it.
#![no_std]
#![forbid(unsafe_code)]

use kanon_clock::LezClock;
use verifier_core::backend::VerifierBackend;
use verifier_core::decode::Payload;
use verifier_core::{verify_feed, AssetPair, FeedConfig, VerifiedFeed, VerifyError};

/// The verifier this crate delegates to. Re-exported so a consumer program pins
/// one verification implementation rather than two.
pub use verifier_core;

/// The account [`verify_price`] will accept a clock from, and no other.
///
/// Re-exported so a consumer needs one dependency to name the account it has to
/// pass, rather than reaching into `kanon-clock` for a constant.
pub use kanon_clock::CLOCK_ACCOUNT_ID;

/// What a consumer needs to build a configuration and read a result, in one
/// place.
pub use verifier_core::backend::{InProgramBackend, SignerAddress};
pub use verifier_core::error::ConfigError;
pub use verifier_core::feed::MAX_MAX_AGE_MS;
pub use verifier_core::value::Value;

/// The configuration RFP-020 asks a pull consumer to supply:
/// `(dataServiceId, feedId, authorised signer set, M-of-N threshold, maxAge)`.
///
/// Four of the five live in [`FeedConfig`], which is `verifier-core`'s own type
/// and the one the push path uses too, so there is one configuration shape behind
/// both modes rather than two kept in step. The fifth is the data service label,
/// which is here and is not verification's business — see the module header.
///
/// Transparent on purpose: two public fields, no constructor, no validation of
/// its own. [`FeedConfig::try_new`] already refuses a configuration that cannot
/// be used, and a wrapper that re-checked its contents would be a second set of
/// errors describing the same faults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PullConfig<'a> {
    /// RedStone's data service, `redstone-primary-prod` for the feeds `FEEDS.md`
    /// measured. Records which service the signer set below belongs to; nothing
    /// in verification reads it, because no payload attests to it.
    pub data_service_id: &'a str,
    /// The feed, its authorised signers, and how many of them must agree.
    pub feed: FeedConfig<'a>,
}

/// Verifies a payload against the consumer's own configuration.
///
/// `config` and `expected` are the consumer's: the signer set, the threshold and
/// the staleness window come from its configuration, and the pair is what it
/// expects the feed to price. `clock_account_id` and `clock_data` are the LEZ
/// clock account the consumer program was given.
///
/// `config.data_service_id` is not read. It is part of the configuration RFP-020
/// describes and it cannot be checked against a payload, so it is the consumer's
/// own record of which service its roster came from.
///
/// Returns the agreed price on both scales, how many signers reported, and the
/// observation's timestamp.
///
/// `clock_account_id` and `clock_data` must be the id and the data of the account
/// the transaction supplied. Nothing here can check that they came from the same
/// account — see the module header — so passing anything else is a way for a
/// consumer program to mislead its own users about how fresh a price is.
///
/// # Errors
///
/// [`VerifyError`], one variant per failure mode, which is what U6 asks for and
/// what the push path answers with for the same causes. In particular
/// [`VerifyError::NoClock`] for an account that is not the pinned one, before
/// any signature is recovered.
pub fn verify_price<B: VerifierBackend>(
    payload: &[u8],
    config: &PullConfig<'_>,
    expected: &AssetPair,
    clock_account_id: &[u8; 32],
    clock_data: &[u8],
    backend: &B,
) -> Result<VerifiedFeed, VerifyError> {
    // First, and for the reason the module header gives: a consumer that reaches
    // verification with a clock of its own choosing has already lost the
    // staleness check.
    let clock = LezClock::from_account(clock_account_id, clock_data)?;
    let payload = Payload::decode(payload)?;
    verify_feed(&payload, &config.feed, expected, backend, &clock)
}
