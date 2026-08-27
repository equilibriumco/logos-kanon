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

use pull_lib::{verify_price, InProgramBackend, PullConfig, CLOCK_ACCOUNT_ID};
use verifier_core::value::median;
use verifier_core::{AssetPair, FeedConfig, VerifyError};

#[path = "../../verifier-core/tests/support/vectors.rs"]
mod vectors;

use vectors::Vector;

/// RedStone's scale for `redstone-primary-prod`.
const DECIMALS: u8 = 8;
const MAX_AGE_MS: u64 = 60_000;
const THRESHOLD: u8 = 3;

/// The service the captured roster belongs to, per `FEEDS.md`.
const DATA_SERVICE: &str = "redstone-primary-prod";

/// The configuration a consumer holds, built the way RFP-020 describes it.
fn config<'a>(v: &Vector, signers: &'a [verifier_core::SignerAddress]) -> PullConfig<'a> {
    PullConfig {
        data_service_id: DATA_SERVICE,
        feed: FeedConfig::try_new(
            v.feed_id.as_bytes(),
            pair(),
            DECIMALS,
            MAX_AGE_MS,
            signers,
            THRESHOLD,
        )
        .expect("a valid consumer configuration"),
    }
}

fn pair() -> AssetPair {
    AssetPair::new([0xB7; AssetPair::ID_LEN], [0x05; AssetPair::ID_LEN])
}

/// The clock account's sixteen bytes: `block_id` then `timestamp`, both
/// little-endian.
///
/// Synthesised, which is the point worth naming rather than hiding: these tests
/// exercise what `verify_price` does with a clock, and not that the clock came
/// from the account it claims. Nothing in this crate can check the second --
/// binding an id to its data needs the `AccountWithMetadata` a dispatcher hands
/// a program -- so that half is the reference consumer's to demonstrate (M3-06).
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
        let config = config(&v, &signers);

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
    let config = config(&v, &signers);

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

    // And the clock is settled before the payload is even framed. Bytes that are
    // not a payload, with an account that is not the clock: a run that decodes
    // first answers `Malformed`, so this is what pins the order rather than the
    // refusal.
    assert_eq!(
        verify_price(
            &[0xFF; 16],
            &config,
            &pair(),
            &[0u8; 32],
            &clock_data(v.timestamp_ms),
            &InProgramBackend::new(),
        ),
        Err(VerifyError::NoClock(
            verifier_core::time::TimeError::WrongAccount
        ))
    );
}

#[test]
fn a_signer_outside_the_consumers_set_cannot_reach_the_threshold() {
    // The signer set is the consumer's, and it is also what binds a feed to a
    // data service: nothing on the wire names one.
    let v = vectors::named("BTC");
    let mut signers = v.signers.clone();
    signers[0] = verifier_core::SignerAddress([0xAA; 20]);
    let config = config(&v, &signers);

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
    let config = config(&v, &signers);
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
    let config = config(&v, &signers);

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
fn a_consumer_handed_bytes_that_are_not_a_payload_is_told_so() {
    // Named for the pull path rather than for the failure. The push side has a
    // test making the same assertion, and `traceability/tests/matrix.rs` resolves
    // a cited name by searching the concatenated source -- so two tests sharing
    // one name means either can satisfy a row meant for the other.
    let v = vectors::named("BTC");
    let signers = v.signers.clone();
    let config = config(&v, &signers);

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

#[test]
fn the_data_service_label_changes_nothing_about_a_verification() {
    // What "carried and authenticates nothing" means, asserted rather than
    // documented: the same payload and the same roster verify to the same price
    // under any label, including an empty one and a service the roster does not
    // belong to. If this ever starts failing, something has begun reading a field
    // no payload attests to.
    let v = vectors::named("BTC");
    let signers = v.signers.clone();
    let mut results = Vec::new();

    for label in ["redstone-primary-prod", "redstone-avalanche-prod", "", "🙂"] {
        let mut config = config(&v, &signers);
        config.data_service_id = label;

        results.push(
            verify_price(
                &v.payload,
                &config,
                &pair(),
                &CLOCK_ACCOUNT_ID,
                &clock_data(v.timestamp_ms),
                &InProgramBackend::new(),
            )
            .unwrap_or_else(|err| panic!("{label:?} should verify: {err:?}")),
        );
    }

    assert!(
        results.windows(2).all(|pair| pair[0] == pair[1]),
        "the label reached the verification"
    );
    assert_eq!(results[0].price, expected_price(&v));
}
