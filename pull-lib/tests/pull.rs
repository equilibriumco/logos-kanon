//! The pull path over payloads from the committed RedStone capture.
//!
//! What M3-01 owns is the surface: that a consumer's own configuration verifies
//! a real payload, that the clock account is pinned, and that failures arrive as
//! the typed taxonomy. The properties of a verification -- the threshold, the
//! window, the scale -- belong to `verifier-core` and are tested there once for
//! both modes.
//!
//! The signatures here are RedStone's. The envelope around them was assembled by
//! `scripts/capture-redstone-vectors.py`, because the gateway serves per-signer
//! JSON rather than a finished payload.

use pull_lib::{verify_price, InProgramBackend, CLOCK_ACCOUNT_ID};
use verifier_core::value::median;
use verifier_core::{AssetPair, FeedConfig, VerifyError};

#[path = "../../verifier-core/tests/support/vectors.rs"]
mod vectors;

use vectors::Vector;

/// RedStone's scale for `redstone-primary-prod`.
const DECIMALS: u8 = 8;
const MAX_AGE_MS: u64 = 60_000;
const THRESHOLD: u8 = 3;

fn pair() -> AssetPair {
    AssetPair::new([0xB7; AssetPair::ID_LEN], [0x05; AssetPair::ID_LEN])
}

/// The clock account's sixteen bytes: `block_id` then `timestamp`, both
/// little-endian.
fn clock_data(now_ms: u64) -> [u8; 16] {
    let mut data = [0u8; 16];
    data[8..].copy_from_slice(&now_ms.to_le_bytes());
    data
}

fn expected_price(v: &Vector) -> u128 {
    let mut values = v.values.clone();
    median(&mut values)
        .expect("five values have a median")
        .to_q64_64(DECIMALS)
        .expect("a RedStone price fits Q64.64")
}

#[test]
fn a_consumers_own_configuration_verifies_a_captured_payload() {
    for v in vectors::all() {
        let signers = v.signers.clone();
        let config = FeedConfig::try_new(
            v.feed_id.as_bytes(),
            pair(),
            DECIMALS,
            MAX_AGE_MS,
            &signers,
            THRESHOLD,
        )
        .expect("a valid consumer configuration");

        let verified = verify_price(
            &v.payload,
            &config,
            &pair(),
            &CLOCK_ACCOUNT_ID,
            &clock_data(v.timestamp_ms),
            &InProgramBackend::new(),
        )
        .unwrap_or_else(|err| panic!("{} should verify: {err:?}", v.feed_id));

        assert_eq!(verified.price, expected_price(&v), "{}", v.feed_id);
        assert_eq!(verified.timestamp_ms, v.timestamp_ms, "{}", v.feed_id);
        assert!(verified.signers >= THRESHOLD, "{}", v.feed_id);
    }
}

#[test]
fn a_clock_account_the_consumer_chose_is_refused_before_any_recovery() {
    // The decision this crate's shape rests on. The 10- and 50-block accounts
    // hold real timestamps, so they are the plausible mistake rather than a
    // random id -- and an account of the consumer's own is the attack.
    let v = vectors::named("BTC");
    let signers = v.signers.clone();
    let config = FeedConfig::try_new(
        v.feed_id.as_bytes(),
        pair(),
        DECIMALS,
        MAX_AGE_MS,
        &signers,
        THRESHOLD,
    )
    .expect("valid");

    for id in [
        *b"/LEZ/ClockProgramAccount/0000010",
        *b"/LEZ/ClockProgramAccount/0000050",
        [0u8; 32],
    ] {
        assert_eq!(
            verify_price(
                &v.payload,
                &config,
                &pair(),
                &id,
                &clock_data(v.timestamp_ms),
                &InProgramBackend::new(),
            ),
            Err(VerifyError::NoClock(
                verifier_core::time::TimeError::WrongAccount
            )),
            "a consumer must not be able to supply its own clock"
        );
    }
}

#[test]
fn a_signer_outside_the_consumers_set_cannot_reach_the_threshold() {
    // The signer set is the consumer's, and it is also what binds a feed to a
    // data service: nothing on the wire names one.
    let v = vectors::named("BTC");
    let mut signers = v.signers.clone();
    signers[0] = verifier_core::SignerAddress([0xAA; 20]);
    let config = FeedConfig::try_new(
        v.feed_id.as_bytes(),
        pair(),
        DECIMALS,
        MAX_AGE_MS,
        &signers,
        THRESHOLD,
    )
    .expect("valid");

    assert_eq!(
        verify_price(
            &v.payload,
            &config,
            &pair(),
            &CLOCK_ACCOUNT_ID,
            &clock_data(v.timestamp_ms),
            &InProgramBackend::new(),
        ),
        Err(VerifyError::UnauthorisedSigner)
    );
}

#[test]
fn a_pair_the_consumer_did_not_expect_is_refused_before_any_recovery() {
    let v = vectors::named("BTC");
    let signers = v.signers.clone();
    let config = FeedConfig::try_new(
        v.feed_id.as_bytes(),
        pair(),
        DECIMALS,
        MAX_AGE_MS,
        &signers,
        THRESHOLD,
    )
    .expect("valid");
    let elsewhere = AssetPair::new([0xEE; AssetPair::ID_LEN], [0x05; AssetPair::ID_LEN]);

    assert_eq!(
        verify_price(
            &v.payload,
            &config,
            &elsewhere,
            &CLOCK_ACCOUNT_ID,
            &clock_data(v.timestamp_ms),
            &InProgramBackend::new(),
        ),
        Err(VerifyError::AssetMismatch)
    );
}

#[test]
fn a_clock_past_the_consumers_window_reports_a_stale_package() {
    let v = vectors::named("BTC");
    let signers = v.signers.clone();
    let config = FeedConfig::try_new(
        v.feed_id.as_bytes(),
        pair(),
        DECIMALS,
        MAX_AGE_MS,
        &signers,
        THRESHOLD,
    )
    .expect("valid");

    assert_eq!(
        verify_price(
            &v.payload,
            &config,
            &pair(),
            &CLOCK_ACCOUNT_ID,
            &clock_data(v.timestamp_ms + MAX_AGE_MS + 1),
            &InProgramBackend::new(),
        ),
        Err(VerifyError::StalePackage)
    );
}

#[test]
fn bytes_that_are_not_a_payload_are_reported_as_malformed() {
    let v = vectors::named("BTC");
    let signers = v.signers.clone();
    let config = FeedConfig::try_new(
        v.feed_id.as_bytes(),
        pair(),
        DECIMALS,
        MAX_AGE_MS,
        &signers,
        THRESHOLD,
    )
    .expect("valid");

    assert_eq!(
        verify_price(
            &[0xFF; 16],
            &config,
            &pair(),
            &CLOCK_ACCOUNT_ID,
            &clock_data(v.timestamp_ms),
            &InProgramBackend::new(),
        ),
        Err(VerifyError::Malformed(
            verifier_core::DecodeError::MissingMarker
        ))
    );
}
