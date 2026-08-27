//! Public-mode pull: verify a RedStone payload inline, in a consumer program.
//!
//! One function. Given a signed payload, the feed configuration the *consumer*
//! holds, the pair the consumer expects, and the LEZ clock account, it returns a
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
//! **A payload names no data service.** RedStone's `dataServiceId` is a key into
//! their gateway and their signer registry, not a field on the wire: a data
//! package carries a feed id and a value, and the only variable slot in the
//! envelope sits outside every signature, where whoever assembles the payload
//! chooses it. The signer set is therefore the binding — those addresses *are*
//! the data service, and a package signed by anyone else is
//! [`VerifyError::UnauthorisedSigner`] whatever service it might claim to come
//! from. `FEEDS.md` records which addresses served which service when this was
//! measured, and ADR 30 records why the set stays per feed.
//!
//! # The clock is not a parameter
//!
//! [`verify_price`] takes the clock *account* and builds the reading itself,
//! refusing any account but the pinned every-block one (ADR 13). A `TimeSource`
//! parameter would have been more flexible and would have handed every consumer
//! the ability to defeat its own staleness check, which is the one thing pinning
//! exists to prevent. In pull mode the caller is the consumer, so this is the
//! mode where that matters most.
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

/// Verifies a payload against the consumer's own configuration.
///
/// `config` and `expected` are the consumer's: the signer set, the threshold and
/// the staleness window come from its configuration, and the pair is what it
/// expects the feed to price. `clock_account_id` and `clock_data` are the LEZ
/// clock account the consumer program was given.
///
/// Returns the agreed price on both scales, how many signers reported, and the
/// observation's timestamp.
///
/// # Errors
///
/// [`VerifyError`], one variant per failure mode, which is what U6 asks for and
/// what the push path answers with for the same causes. In particular
/// [`VerifyError::NoClock`] for an account that is not the pinned one, before
/// any signature is recovered.
pub fn verify_price<B: VerifierBackend>(
    payload: &[u8],
    config: &FeedConfig<'_>,
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
    verify_feed(&payload, config, expected, backend, &clock)
}
