//! Public-mode pull: verify a RedStone payload inline, in a consumer program.
//!
//! One function. Given a signed payload, the configuration the *consumer* holds,
//! the pair the consumer expects, and the LEZ clock account, it returns a
//! verified price or a typed reason.
//!
//! Nothing here decides anything about a payload. [`verifier_core`] does all of
//! it, which is what makes one audit cover both modes (ADR 2) and what makes
//! this crate's independence from the aggregator structural: there is no
//! aggregator type in the signature and no code path that could reach one.
//!
//! # Independence from the aggregator, and what holds it
//!
//! A consumer using only pull registers nothing *with the aggregator* (F9). It
//! supplies a feed id, a signer set, a threshold and a window; it reads no feed
//! account, and there is no price account for it to derive.
//!
//! The qualification matters and dropping it caused a wrong decision once. F9 asks
//! for a configured tuple and no dependency on the aggregator's price account; it
//! says nothing about where a consumer keeps that tuple. A consumer that holds its
//! roster in an account it owns and governs is not registering a feed against the
//! aggregator, and `[M3-06:02]` records the reference consumer arriving at exactly
//! that after the shorter reading sent it somewhere worse.
//!
//! Nothing in this crate would notice if that stopped being true, because the
//! failure is an addition rather than a break: a `use` of an aggregator type to
//! borrow a field layout compiles, and every test below still passes. So the
//! guard is on the build graph instead — CI's `pull-independence` job pins every
//! non-registry package in this crate's dependency closure, and the reference
//! consumer's, to an allow-list, across every feature and every target. That
//! covers the price-account type as well as this repository's own crates: the
//! canonical one lives in `twap_oracle_core`, which arrives by git.
//! `Cargo.toml` records why an allow-list, and why dev-dependencies sit outside
//! it.
//!
//! # What a consumer supplies
//!
//! The signer set, the threshold and `maxAge` come from the consumer and never
//! from the payload (SEC2; SEC1 is the membership check the set is then used
//! for). That is the whole security model of pull mode: a payload is a set of
//! signed claims, and which signatures count is the consumer's decision.
//!
//! Verification does read signers out of the payload — recovering one from each
//! package that carries the requested feed is the whole mechanism, and packages
//! carrying only other feeds are skipped before that cost — but a recovered
//! address is only ever a lookup key. It is accepted when it already appears in
//! the roster you supplied, and is otherwise
//! [`VerifyError::UnauthorisedSigner`]. Nothing a payload carries joins
//! the roster, or the set of addresses the next package is checked against.
//!
//! **Where your roster came from is yours to guarantee, and this library cannot
//! check it.** The roster arrives as a slice, and one built from the transaction's
//! own instruction data is indistinguishable here from one built from a constant.
//! A consumer program that lets its caller supply the signer set has handed that
//! caller the whole security model, and every check in this crate will still pass.
//! Source it from constants compiled into the program, or from an account the
//! program owns — the reference consumer (M3-06) is where that boundary is
//! demonstrated rather than asserted.
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
//! # A roster narrower than the payload refuses rather than narrowing
//!
//! The set being the consumer's does not mean the consumer can pick any subset of
//! it and keep verifying the same payloads. Every package carrying the *requested
//! feed* is recovered, and the first signer outside the roster ends the
//! verification with [`VerifyError::UnauthorisedSigner`] (ADR 15). So authorising
//! three of the five that signed for this feed does not verify against those
//! three: it refuses the payload.
//!
//! The scope is the requested feed and only that. Packages carrying other feeds
//! are skipped before their signer is ever recovered, so a stranger reporting
//! something else in the same payload is not a refusal —
//! `strangers_reporting_only_another_feed_do_not_report_unauthorised_signer` in
//! `verifier-core` is where that is pinned.
//!
//! Read the other way, which is the way an operator needs it: **a roster has to
//! cover every signer of every package carrying the requested feed.** Dropping a
//! signer is not a narrowing of who counts, it is a refusal of every payload in
//! which that signer reports this feed — so a roster change and the payloads a
//! relayer is already assembling are one operational step, not two.
//!
//! The other direction is backward-compatible rather than free. Adding a valid
//! signer does not invalidate any payload that already verified, and an added
//! signer that did not report contributes nothing towards the threshold:
//! [`VerifyError::ThresholdNotMet`] reports what was met, not what was listed, and
//! the median is taken over the signers that reported rather than over the roster.
//! Both are `verifier-core`'s, tested there once for both modes. But adding is
//! still a change to who may speak for this feed, because that signer's report can
//! count towards an unchanged threshold from then on, and the roster has a ceiling
//! ([`MAX_SIGNERS`]) it is refused for exceeding. Adding is the safe order of
//! operations, not a free one.
//!
//! Order is not part of it. The roster's order is bookkeeping inside the walk — a
//! report is filed by the signer's position in it, which is what stops one signer
//! filling two slots — and a consumer writing the same addresses in a different
//! order is not configuring a different feed. A duplicate is refused outright, by
//! [`ConfigError::DuplicateSigner`] when the configuration is built.
//!
//! # The expected pair is checked against your own configuration
//!
//! [`verify_price`] takes an `expected` pair and compares it against the one in
//! `config.feed`. Both are the consumer's, so a mismatch means the consumer
//! contradicted itself and nothing more — no signer attests to which assets a
//! feed prices, as [`VerifyError::AssetMismatch`] says.
//!
//! That is a real difference from push, and not a thinner version of the same
//! check. There `config` is read from a registered feed account, so the
//! comparison is a caller's claim against a registration a program made earlier
//! (ADR 16); here there is no registration to claim against. A consumer that
//! copies the wrong ids into both its `FeedConfig` and its `expected` gets a
//! verified price labelled with the wrong pair and no error, which is exactly
//! what the parameter looks like it prevents. Getting the feed id right is what
//! prevents it.
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
use verifier_core::verify_feed;

/// The verifier this crate delegates to. Re-exported so a consumer program pins
/// one verification implementation rather than two.
pub use verifier_core;

/// The account [`verify_price`] will accept a clock from, and no other.
///
/// Re-exported so a consumer needs one dependency to name the account it has to
/// pass, rather than reaching into `kanon-clock` for a constant.
pub use kanon_clock::CLOCK_ACCOUNT_ID;

/// What a consumer needs to build a configuration, make the call, and act on
/// what comes back, in one place.
///
/// Every type in [`verify_price`]'s signature is here, and so is every error
/// type reachable by matching on a [`VerifyError`]. `tests/pull.rs` is what
/// holds this honest: it names no crate but this one, so a re-export missing
/// from here fails to compile there.
pub use verifier_core::backend::{BackendError, InProgramBackend, SignerAddress};
pub use verifier_core::decode::DecodeError;
pub use verifier_core::error::{ConfigError, VerifyError};
pub use verifier_core::feed::{AssetPair, FeedConfig, VerifiedFeed, MAX_MAX_AGE_MS, MAX_SIGNERS};
pub use verifier_core::time::TimeError;
pub use verifier_core::value::Value;

/// The largest payload that will verify, for a caller to enforce *before* it
/// builds a transaction.
///
/// Re-exported because that enforcement is the consumer's and it cannot happen
/// here. LEZ reads a program's whole instruction data before its first
/// instruction and charges for it, so by the time [`verify_price`] could refuse
/// an oversized payload the transaction has already paid about 113 cycles a byte
/// for the bytes (ADR 26). `Payload::decode` still refuses one — that keeps the
/// bound a property of verification rather than of a caller's diligence — but
/// the refusal that saves anything happens before the transaction exists.
pub use verifier_core::decode::MAX_PAYLOAD_BYTES;

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
/// the staleness window come from its configuration, and so does the pair the
/// `expected` one is compared against — a mismatch here is a consumer
/// disagreeing with itself, not a payload caught out. See the module header.
/// `clock_account_id` and `clock_data` are the LEZ clock account the consumer
/// program was given.
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
