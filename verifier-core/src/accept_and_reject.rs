//! The accept and reject contract, as a caller sees it.
//!
//! Every other test in this crate is organised around the thing that
//! introduced it: framing cases sit in [`decode`](crate::decode), threshold
//! cases in [`feed`](crate::feed), scale cases in [`value`](crate::value). That
//! is the right shape for finding a regression and the wrong shape for
//! answering "is every failure mode covered, and does each one say something a
//! caller can act on?" — which is what U6 asks and what a reviewer has to be
//! able to check in one place.
//!
//! So this module is organised by the *contract* instead: one test per way a
//! verification can succeed, one per way it can fail, named after the mode
//! rather than after the mechanism. It calls nothing but the crate's public
//! API, which is the surface both modes see.
//!
//! # Both call paths
//!
//! M1-22 asks for the accept and reject paths exercised "from the in-program
//! call path and the consumer-side call path". Those are two callers of one
//! function: ADR 2 makes [`verify_feed`] the only thing in the workspace that
//! verifies, and it takes no argument that tells it which mode invoked it, so
//! there is no per-mode behaviour for a per-mode test to reach. Testing it once
//! through its public API is what "one audited implementation backs both modes"
//! means as evidence.
//!
//! What the two callers add on top — `submit_price` writing the result to an
//! account, `pull-lib` returning it to a consumer program — is theirs to test:
//! M2-02 and M3-01 respectively, with M3-04 asserting the two report the same
//! taxonomy for every shared failure mode.

extern crate std;

use std::vec::Vec;

use k256::ecdsa::SigningKey;

use crate::{
    backend::InProgramBackend,
    backend::SignerAddress,
    decode::{DecodeError, Payload},
    error::{ConfigError, VerifyError},
    feed::{verify_feed, AssetPair, FeedConfig, VerifiedFeed, MAX_RECOVERIES},
    test_support::{address_of, signing_key, PayloadBuilder},
    time::{TimeError, TimeSource},
    value::{Value, MAX_DECIMALS},
};

const FEED: &[u8] = b"BTC";

/// RedStone's usual exponent, so the conversion to the account's scale runs on
/// every test here rather than only where it is the subject.
const DECIMALS: u8 = 8;

const NOW_MS: u64 = 1_770_000_000_000;

/// Three minutes, RedStone's own default for this data service.
const MAX_AGE_MS: u64 = 180_000;

/// A price a signer reports: 100, on the wire's big-endian layout.
const HUNDRED: &[u8] = &[0, 0, 0, 100];

/// Minus one, as `int256`.
///
/// Full width on purpose: the sign is the top bit of the 32-byte value, and
/// `from_be_slice` right-aligns a shorter slice, so a four-byte `0xFFFFFFFF`
/// is a large positive number rather than a negative one.
const NEGATIVE: &[u8] = &[0xFF; 32];

fn pair() -> AssetPair {
    AssetPair::new([0xB7; AssetPair::ID_LEN], [0x05; AssetPair::ID_LEN])
}

fn keys(n: u8) -> Vec<SigningKey> {
    (1..=n).map(signing_key).collect()
}

fn addresses(keys: &[SigningKey]) -> Vec<SignerAddress> {
    keys.iter().map(address_of).collect()
}

fn config(signers: &[SignerAddress], threshold: u8) -> FeedConfig<'_> {
    FeedConfig::try_new(FEED, pair(), DECIMALS, MAX_AGE_MS, signers, threshold)
        .expect("a configuration these tests control")
}

/// The LEZ clock account, stood in for by whatever the test wants it to read —
/// including a failure, which is a case a caller has to handle.
struct Clock(Result<u64, TimeError>);

impl TimeSource for Clock {
    fn now_ms(&self) -> Result<u64, TimeError> {
        self.0
    }
}

/// Everything a caller does: decode, then verify against what it believes it
/// asked for. Split out so each test below is the case and nothing else.
fn verify(bytes: &[u8], config: &FeedConfig<'_>) -> Result<VerifiedFeed, VerifyError> {
    verify_at(bytes, config, &pair(), Ok(NOW_MS))
}

fn verify_at(
    bytes: &[u8],
    config: &FeedConfig<'_>,
    expected: &AssetPair,
    now: Result<u64, TimeError>,
) -> Result<VerifiedFeed, VerifyError> {
    let payload = Payload::decode(bytes).expect("these tests supply well-framed envelopes");
    verify_feed(
        &payload,
        config,
        expected,
        &InProgramBackend::new(),
        &Clock(now),
    )
}

/// Every signer reporting the same value at the same time.
fn agreed(keys: &[SigningKey], value: &[u8], at: u64) -> Vec<u8> {
    let mut builder = PayloadBuilder::default();
    for key in keys {
        builder = builder.signed_package(key, &[(FEED, value)], at);
    }
    builder.build()
}

#[test]
fn exactly_the_threshold_verifies_and_reports_how_many_signers_met_it() {
    let keys = keys(3);
    let set = addresses(&keys);

    let verified = verify(&agreed(&keys, HUNDRED, NOW_MS), &config(&set, 3)).expect("verifies");

    assert_eq!(verified.signers, 3);
    assert_eq!(verified.value, Value::from_be_slice(&[100]).expect("fits"));
    assert_eq!(verified.timestamp_ms, NOW_MS);
}

#[test]
fn more_signers_than_the_threshold_all_count_toward_the_median() {
    // The threshold is a floor, not a quota: a caller that configured 3-of-5
    // and got 5 should get the median of five, because discarding the surplus
    // would make the price depend on which packages happened to arrive first.
    let keys = keys(5);
    let set = addresses(&keys);

    let verified = verify(&agreed(&keys, HUNDRED, NOW_MS), &config(&set, 3)).expect("verifies");

    assert_eq!(verified.signers, 5);
}

#[test]
fn one_unusable_value_rejects_even_when_the_other_signers_reach_threshold() {
    // F4 rejects the package itself; reaching quorum with the remaining
    // packages must not turn that rejection into acceptance.
    let keys = keys(4);
    let set = addresses(&keys);
    let mut builder = PayloadBuilder::default();
    builder = builder.signed_package(&keys[0], &[(FEED, &[0, 0, 0, 0])], NOW_MS);
    for key in &keys[1..] {
        builder = builder.signed_package(key, &[(FEED, HUNDRED)], NOW_MS);
    }

    assert_eq!(
        verify(&builder.build(), &config(&set, 3)),
        Err(VerifyError::ValueOutOfRange)
    );
}

#[test]
fn a_package_duplicated_by_anyone_does_not_deny_the_feed_to_the_rest() {
    // Copying a package needs no key, so a copy must not be able to deny a
    // verified price to every consumer of that payload (ADR 24). A copy carries
    // the moment it was copied from, so it fills no second slot and moves
    // nothing. Its value is chosen so that admitting it would move the median.
    let keys = keys(3);
    let set = addresses(&keys);
    let bytes = PayloadBuilder::default()
        .signed_package(&keys[0], &[(FEED, &[0, 0, 0, 10])], NOW_MS)
        .signed_package(&keys[1], &[(FEED, &[0, 0, 0, 20])], NOW_MS)
        .signed_package(&keys[2], &[(FEED, &[0, 0, 0, 30])], NOW_MS)
        .signed_package(&keys[0], &[(FEED, &[0, 0, 0, 10])], NOW_MS)
        .build();

    let verified = verify(&bytes, &config(&set, 3)).expect("verifies");

    assert_eq!(verified.signers, 3, "the copy fills no slot of its own");
    assert_eq!(verified.value, Value::from_be_slice(&[20]).expect("fits"));
    assert_eq!(verified.timestamp_ms, NOW_MS);
}

#[test]
fn a_package_replayed_from_an_older_round_refuses_the_payload() {
    // The other half of the same attack, and the one this contract answers
    // differently: a package from a round still inside `maxAge` is validly
    // signed but describes another moment, so the payload no longer describes
    // one. Refused rather than skipped -- which is a denial available to anyone
    // who can add bytes to a payload, and the reason a submitter must own what
    // it submits (ADR 27).
    let keys = keys(3);
    let set = addresses(&keys);
    let bytes = PayloadBuilder::default()
        .signed_package(&keys[0], &[(FEED, &[0, 0, 0, 10])], NOW_MS)
        .signed_package(&keys[1], &[(FEED, &[0, 0, 0, 20])], NOW_MS)
        .signed_package(&keys[2], &[(FEED, &[0, 0, 0, 30])], NOW_MS)
        .signed_package(&keys[1], &[(FEED, &[0, 0, 0, 200])], NOW_MS - MAX_AGE_MS)
        .build();

    assert!(matches!(
        verify(&bytes, &config(&set, 3)),
        Err(VerifyError::TimestampMismatch { .. })
    ));
}

#[test]
fn one_stranger_rejects_even_when_the_authorised_signers_reach_threshold() {
    // SEC1 rejects every package signed outside the configured set; an
    // otherwise sufficient quorum does not make the package authorised.
    let keys = keys(3);
    let set = addresses(&keys);
    let stranger = signing_key(0x9E);
    let mut builder = PayloadBuilder::default();
    for key in &keys {
        builder = builder.signed_package(key, &[(FEED, HUNDRED)], NOW_MS);
    }
    builder = builder.signed_package(&stranger, &[(FEED, &[0, 0, 0, 200])], NOW_MS);

    assert_eq!(
        verify(&builder.build(), &config(&set, 3)),
        Err(VerifyError::UnauthorisedSigner)
    );
}

#[test]
fn a_package_at_the_far_edge_of_the_window_is_still_current() {
    // Exactly `max_age` old. The boundary is inclusive, so a caller that set a
    // three-minute window gets three minutes rather than a hair under.
    let keys = keys(3);
    let set = addresses(&keys);
    let bytes = agreed(&keys, HUNDRED, NOW_MS - MAX_AGE_MS);

    let verified = verify(&bytes, &config(&set, 3)).expect("verifies");

    assert_eq!(verified.timestamp_ms, NOW_MS - MAX_AGE_MS);
}

#[test]
fn an_accepted_price_carries_both_scales_so_a_caller_need_not_convert() {
    // RedStone's integer and its exponent on one side, the account's Q64.64 on
    // the other. A caller that had to do this itself is a caller that can get
    // it wrong.
    let keys = keys(3);
    let set = addresses(&keys);

    let verified = verify(&agreed(&keys, HUNDRED, NOW_MS), &config(&set, 3)).expect("verifies");

    assert_eq!(verified.value, Value::from_be_slice(&[100]).expect("fits"));
    assert_eq!(
        verified.price,
        (100u128 << 64) / 100_000_000,
        "100 at 8 decimals is 0.000001, on the account's scale"
    );
}

#[test]
fn a_malformed_payload_is_named_as_malformed_and_carries_the_decoder_reason() {
    // U6's "malformed package". The reason travels with it: a caller that only
    // learned "malformed" would have nothing to put in a log that helps.
    let keys = keys(1);
    let set = addresses(&keys);
    let valid = agreed(&keys, &[0, 0, 0, 10], NOW_MS);
    let mut bytes = std::vec![0xDEu8; 16];
    bytes.extend_from_slice(&valid);

    assert_eq!(
        verify(&bytes, &config(&set, 1)),
        Err(VerifyError::Malformed(DecodeError::TrailingBytes(16)))
    );
}

#[test]
fn an_envelope_that_is_not_redstones_is_refused_before_any_verification() {
    // The other half of "malformed": a payload whose envelope does not decode
    // never reaches `verify_feed` at all, so a caller has two places to handle
    // rather than one. Worth pinning, because a caller that only matches on
    // `VerifyError` will miss this one.
    assert!(Payload::decode(&[0u8; 8]).is_err());
}

#[test]
fn too_few_signers_reports_how_many_arrived_against_how_many_were_needed() {
    // U6's "threshold not met", and the numbers are the actionable part: an
    // operator reading `met: 2, required: 3` knows whether to wait or to look.
    let keys = keys(3);
    let set = addresses(&keys);
    let bytes = agreed(&keys[..2], HUNDRED, NOW_MS);

    assert_eq!(
        verify(&bytes, &config(&set, 3)),
        Err(VerifyError::ThresholdNotMet {
            met: 2,
            required: 3
        })
    );
}

#[test]
fn signers_nobody_configured_are_named_as_the_fault_rather_than_the_count() {
    // U6's "unauthorised signer". The distinction from `ThresholdNotMet` is the
    // whole value of the variant: this says the configured set is wrong for
    // this payload, not that too few signers signed.
    let configured = keys(3);
    let set = addresses(&configured);
    let strangers = [signing_key(0x91), signing_key(0x92), signing_key(0x93)];

    assert_eq!(
        verify(&agreed(&strangers, HUNDRED, NOW_MS), &config(&set, 3)),
        Err(VerifyError::UnauthorisedSigner)
    );
}

#[test]
fn a_pair_that_is_not_the_callers_is_refused_before_a_single_recovery() {
    // U6's "asset mismatch". `expected` is a parameter rather than a separate
    // method precisely so a caller cannot reach a price without stating what it
    // thought it was asking for.
    let keys = keys(3);
    let set = addresses(&keys);
    let elsewhere = AssetPair::new([0x11; AssetPair::ID_LEN], [0x05; AssetPair::ID_LEN]);

    assert_eq!(
        verify_at(
            &agreed(&keys, HUNDRED, NOW_MS),
            &config(&set, 3),
            &elsewhere,
            Ok(NOW_MS)
        ),
        Err(VerifyError::AssetMismatch)
    );
}

#[test]
fn a_payload_older_than_the_window_is_stale_rather_than_short_of_signers() {
    // U6's "stale package". Naming the threshold here would send an operator
    // looking for signers that are already in the payload.
    let keys = keys(3);
    let set = addresses(&keys);
    let bytes = agreed(&keys, HUNDRED, NOW_MS - MAX_AGE_MS - 1);

    assert_eq!(
        verify(&bytes, &config(&set, 3)),
        Err(VerifyError::StalePackage)
    );
}

#[test]
fn a_payload_dated_ahead_of_the_clock_is_its_own_failure_not_a_stale_one() {
    // Not named by U6, but a caller that saw `StalePackage` for a future-dated
    // package would go looking at the relayer's lag when the fault is a clock.
    let keys = keys(3);
    let set = addresses(&keys);
    let bytes = agreed(&keys, HUNDRED, NOW_MS + MAX_AGE_MS * 10);

    assert_eq!(
        verify(&bytes, &config(&set, 3)),
        Err(VerifyError::FuturePackage)
    );
}

#[test]
fn zero_and_negative_prices_are_rejected_directly() {
    // F4 and U6 require the package to be rejected, independently of threshold.
    let keys = keys(3);
    let set = addresses(&keys);

    assert_eq!(
        verify(&agreed(&keys, &[0, 0, 0, 0], NOW_MS), &config(&set, 3)),
        Err(VerifyError::ValueOutOfRange)
    );
    assert_eq!(
        verify(&agreed(&keys, NEGATIVE, NOW_MS), &config(&set, 3)),
        Err(VerifyError::ValueOutOfRange),
        "a negative value is as unusable as a zero one"
    );
}

#[test]
fn one_signer_supplying_the_feed_twice_counts_once_and_is_short_of_the_threshold() {
    // Not named by U6. A signer that can occupy two slots can reach a threshold
    // alone, which is the one thing M-of-N is counting on being impossible. The
    // second package is skipped rather than fatal (ADR 24), so what a caller
    // sees is the threshold failure it is.
    let keys = keys(3);
    let set = addresses(&keys);
    let bytes = PayloadBuilder::default()
        .signed_package(&keys[0], &[(FEED, HUNDRED), (FEED, HUNDRED)], NOW_MS)
        .build();

    assert_eq!(
        verify(&bytes, &config(&set, 3)),
        Err(VerifyError::ThresholdNotMet {
            met: 1,
            required: 3
        })
    );
}

#[test]
fn values_from_different_rounds_are_not_made_into_one_price() {
    // Not named by U6. Each package is validly signed and inside the window;
    // what is wrong is that they describe three moments rather than one, and a
    // median across them is an observation nobody published. Refused rather
    // than answered from part of the payload (ADR 27).
    let keys = keys(3);
    let set = addresses(&keys);
    let bytes = PayloadBuilder::default()
        .signed_package(&keys[0], &[(FEED, HUNDRED)], NOW_MS)
        .signed_package(&keys[1], &[(FEED, HUNDRED)], NOW_MS - 10_000)
        .signed_package(&keys[2], &[(FEED, HUNDRED)], NOW_MS - 20_000)
        .build();

    assert_eq!(
        verify(&bytes, &config(&set, 3)),
        Err(VerifyError::TimestampMismatch {
            expected: NOW_MS - 20_000,
            found: NOW_MS - 10_000
        })
    );
}

#[test]
fn more_packages_for_this_feed_than_verification_will_pay_for_is_refused() {
    // Not named by U6. The package count sits in the envelope, outside every
    // signature, so without a ceiling the payload decides how much of the
    // transaction's cycle budget verification spends -- and past about 56
    // packages it spends all of it and the transaction aborts, which is not a
    // failure a caller can act on because it never gets to see one (ADR 25).
    let keys = keys(3);
    let set = addresses(&keys);
    let mut builder = PayloadBuilder::default();
    for i in 0..=MAX_RECOVERIES {
        builder = builder.signed_package(&keys[i % 3], &[(FEED, HUNDRED)], NOW_MS);
    }

    assert_eq!(
        verify(&builder.build(), &config(&set, 3)),
        Err(VerifyError::TooManyPackages {
            max: MAX_RECOVERIES
        })
    );
}

#[test]
fn a_price_the_accounts_scale_cannot_hold_is_reported_not_wrapped() {
    // Not named by U6, and not reachable from a real feed. The alternative to
    // reporting it is writing a wrapped number that looks like a price.
    let keys = keys(3);
    let set = addresses(&keys);

    assert_eq!(
        verify(&agreed(&keys, &[0x7F; 12], NOW_MS), &config(&set, 3)),
        Err(VerifyError::ScalingOutOfRange)
    );
}

#[test]
fn a_clock_that_cannot_be_read_fails_the_verification_rather_than_defaulting() {
    // Not named by U6. Falling back to any other time source would make a stale
    // price look current, which is the failure this whole window exists to
    // prevent — so the read failing is a refusal, not a degraded mode.
    let keys = keys(3);
    let set = addresses(&keys);
    let bytes = agreed(&keys, HUNDRED, NOW_MS);

    assert_eq!(
        verify_at(
            &bytes,
            &config(&set, 3),
            &pair(),
            Err(TimeError::WrongAccount)
        ),
        Err(VerifyError::NoClock(TimeError::WrongAccount))
    );
}

#[test]
fn an_unusable_configuration_is_refused_before_a_payload_is_ever_seen() {
    // U6 covers failure modes of a verification; this one fails earlier, which
    // is the point. `FeedConfig::try_new` is the only way to build one, so a
    // threshold no signer set can reach cannot be handed to `verify_feed` at
    // all — and `VerifyError::InvalidConfig` is how a caller that keeps one
    // error type reports it.
    let signers = [SignerAddress([1; SignerAddress::LEN])];
    let err = FeedConfig::try_new(FEED, pair(), DECIMALS, MAX_AGE_MS, &signers, 2)
        .expect_err("a threshold of two over one signer is unreachable");

    assert_eq!(
        err,
        ConfigError::ThresholdExceedsSigners {
            threshold: 2,
            signers: 1
        }
    );
    assert_eq!(
        VerifyError::from(err),
        VerifyError::InvalidConfig(ConfigError::ThresholdExceedsSigners {
            threshold: 2,
            signers: 1
        })
    );
    assert_eq!(
        FeedConfig::try_new(FEED, pair(), MAX_DECIMALS + 1, MAX_AGE_MS, &signers, 1)
            .expect_err("an exponent past the scale's limit"),
        ConfigError::DecimalsOutOfRange {
            decimals: MAX_DECIMALS + 1,
            max: MAX_DECIMALS
        }
    );
}

#[test]
fn a_signature_that_recovers_to_nobody_is_reported_directly() {
    // U6 promises a signature-specific error. It must remain distinct from a
    // well-signed payload that simply falls short of the threshold.
    let keys = keys(3);
    let set = addresses(&keys);
    let bytes = PayloadBuilder::default()
        .opaque_package(&[(FEED, HUNDRED)], NOW_MS, 0x00)
        .opaque_package(&[(FEED, HUNDRED)], NOW_MS, 0x01)
        .opaque_package(&[(FEED, HUNDRED)], NOW_MS, 0x02)
        .build();

    assert_eq!(
        verify(&bytes, &config(&set, 3)),
        Err(VerifyError::InvalidSignature(
            crate::backend::BackendError::InvalidRecoveryId
        ))
    );
}

#[test]
fn no_two_failure_modes_answer_with_the_same_variant() {
    // The assertion the per-mode tests above cannot make individually. A
    // taxonomy is only actionable if it discriminates: if two distinct causes
    // both answered `ThresholdNotMet`, every test above would still pass and U6
    // would still be unmet, because an operator could not tell which happened.
    //
    // Compared by variant rather than by value. `ThresholdNotMet { met: 0 }`
    // and `ThresholdNotMet { met: 1 }` are different values and the same
    // answer, so comparing values would let exactly the collapse this test
    // exists to catch slip through.
    let keys = keys(3);
    let set = addresses(&keys);
    let strangers = [signing_key(0x91), signing_key(0x92), signing_key(0x93)];
    let elsewhere = AssetPair::new([0x11; AssetPair::ID_LEN], [0x05; AssetPair::ID_LEN]);
    let mut trailing = std::vec![0xDEu8; 16];
    trailing.extend_from_slice(&agreed(&keys, HUNDRED, NOW_MS));
    let garbage_signature = PayloadBuilder::default()
        .opaque_package(&[(FEED, HUNDRED)], NOW_MS, 0xFF)
        .build();

    let observed = [
        ("malformed", verify(&trailing, &config(&set, 1))),
        (
            "invalid signature",
            verify(&garbage_signature, &config(&set, 1)),
        ),
        (
            "too few signers",
            verify(&agreed(&keys[..1], HUNDRED, NOW_MS), &config(&set, 3)),
        ),
        (
            "unauthorised signers",
            verify(&agreed(&strangers, HUNDRED, NOW_MS), &config(&set, 3)),
        ),
        (
            "another pair",
            verify_at(
                &agreed(&keys, HUNDRED, NOW_MS),
                &config(&set, 3),
                &elsewhere,
                Ok(NOW_MS),
            ),
        ),
        (
            "stale",
            verify(
                &agreed(&keys, HUNDRED, NOW_MS - MAX_AGE_MS - 1),
                &config(&set, 3),
            ),
        ),
        (
            "future dated",
            verify(
                &agreed(&keys, HUNDRED, NOW_MS + MAX_AGE_MS * 10),
                &config(&set, 3),
            ),
        ),
        (
            "unusable values",
            verify(&agreed(&keys, &[0, 0, 0, 0], NOW_MS), &config(&set, 3)),
        ),
        ("mixed rounds", {
            let bytes = PayloadBuilder::default()
                .signed_package(&keys[0], &[(FEED, HUNDRED)], NOW_MS)
                .signed_package(&keys[1], &[(FEED, HUNDRED)], NOW_MS - 10_000)
                .signed_package(&keys[2], &[(FEED, HUNDRED)], NOW_MS - 20_000)
                .build();
            verify(&bytes, &config(&set, 3))
        }),
        ("too many packages", {
            let mut builder = PayloadBuilder::default();
            for i in 0..=MAX_RECOVERIES {
                builder = builder.signed_package(&keys[i % 3], &[(FEED, HUNDRED)], NOW_MS);
            }
            verify(&builder.build(), &config(&set, 3))
        }),
        (
            "past the account scale",
            verify(&agreed(&keys, &[0x7F; 12], NOW_MS), &config(&set, 3)),
        ),
        (
            "no clock",
            verify_at(
                &agreed(&keys, HUNDRED, NOW_MS),
                &config(&set, 3),
                &pair(),
                Err(TimeError::WrongAccount),
            ),
        ),
    ];

    let variants: Vec<(&str, core::mem::Discriminant<VerifyError>)> = observed
        .iter()
        .map(|(cause, result)| {
            let err = result.as_ref().expect_err(cause);
            (*cause, core::mem::discriminant(err))
        })
        .collect();

    for (i, (cause, variant)) in variants.iter().enumerate() {
        for (other, other_variant) in variants.iter().skip(i + 1) {
            assert_ne!(
                variant, other_variant,
                "\"{cause}\" and \"{other}\" are different faults reported as the same variant"
            );
        }
    }
}

/// Twice the honest value, so a median that moved is a median that says so.
const TWO_HUNDRED: &[u8] = &[0, 0, 0, 200];

fn value(n: u8) -> Value {
    Value::from_be_slice(&[n]).expect("fits")
}

/// A payload an attacker assembled: `held` packages carrying [`TWO_HUNDRED`],
/// then honest packages carrying [`HUNDRED`], `threshold` in total.
///
/// Every honest report is the same value here, which is deliberate and also a
/// limit: it makes the tests below read as the attacker choosing the answer, and
/// it hides the sub-majority case that
/// `signers_below_the_bar_still_choose_which_honest_report_wins` covers with
/// honest reports that differ.
///
/// The honest packages are the point. An attacker short of the threshold needs no
/// further keys, because every honest package for the round is public and it can
/// put in as many as the threshold requires.
fn assembled_by_an_attacker(keys: &[SigningKey], held: usize, threshold: usize) -> Vec<u8> {
    let mut builder = PayloadBuilder::default();
    for (i, key) in keys.iter().take(threshold).enumerate() {
        let reported: &[u8] = if i < held { TWO_HUNDRED } else { HUNDRED };
        builder = builder.signed_package(key, &[(FEED, reported)], NOW_MS);
    }
    builder.build()
}

#[test]
fn two_compromised_signers_decide_a_threshold_of_three() {
    // The price is a median, so an attacker needs more than half of the counted
    // slots rather than all of them. Nothing here is a defect: every package is
    // validly signed by an authorised signer, for the right feed, at one moment.
    // That is what leaves the threshold as the only thing in the way, and why its
    // arithmetic is worth pinning rather than describing.
    let keys = keys(5);
    let set = addresses(&keys);
    let config = config(&set, 3);

    let two = verify(&assembled_by_an_attacker(&keys, 2, 3), &config).expect("verifies");
    assert_eq!(
        two.value,
        value(200),
        "two of three slots decide the median outright"
    );

    let one = verify(&assembled_by_an_attacker(&keys, 1, 3), &config).expect("verifies");
    assert_eq!(
        one.value,
        value(100),
        "one of three moves nothing, and the honest majority holds"
    );
}

#[test]
fn a_threshold_of_four_is_no_harder_to_move_than_a_threshold_of_three() {
    // An even threshold averages the two middle values, so half the slots reach
    // the answer without holding a majority of them. Raising a threshold from
    // three to four therefore buys nothing, which is the whole reason the
    // recommendation names five.
    let keys = keys(5);
    let set = addresses(&keys);

    let moved = verify(&assembled_by_an_attacker(&keys, 2, 4), &config(&set, 4)).expect("verifies");

    assert_eq!(
        moved.value,
        value(150),
        "two of four slots pull the midpoint halfway, and further with a wider value"
    );
    assert_ne!(moved.value, value(100), "the honest value did not survive");
}

#[test]
fn a_threshold_of_five_needs_three_compromised_signers() {
    // The odd step up is the one that helps. Two slots of five cannot reach the
    // middle of the sorted values, so the honest report is the answer.
    let keys = keys(5);
    let set = addresses(&keys);
    let config = config(&set, 5);

    let two = verify(&assembled_by_an_attacker(&keys, 2, 5), &config).expect("verifies");
    assert_eq!(two.value, value(100), "two of five cannot reach the middle");

    let three = verify(&assembled_by_an_attacker(&keys, 3, 5), &config).expect("verifies");
    assert_eq!(three.value, value(200), "three of five decide it");
}

#[test]
fn signers_below_the_bar_still_choose_which_honest_report_wins() {
    // The other tests here give every honest signer the same value, so an
    // attacker under the majority bar looks powerless. It is not: a median picks
    // a position, and two of five slots pushed to one extreme move that position
    // onto the highest or the lowest honest report. The honest values bound the
    // outcome and the attacker picks which of them is the outcome, which is a
    // weaker position than deciding the price and a stronger one than nothing.
    let keys = keys(5);
    let set = addresses(&keys);
    let config = config(&set, 5);

    // Three honest reports that disagree, and two slots the attacker holds.
    let spread: [&[u8]; 3] = [&[0, 0, 0, 95], &[0, 0, 0, 100], &[0, 0, 0, 105]];
    let attacked = |attacker_value: &[u8]| {
        let mut builder = PayloadBuilder::default();
        for (key, honest) in keys[..3].iter().zip(spread) {
            builder = builder.signed_package(key, &[(FEED, honest)], NOW_MS);
        }
        for key in &keys[3..] {
            builder = builder.signed_package(key, &[(FEED, attacker_value)], NOW_MS);
        }
        verify(&builder.build(), &config).expect("verifies").value
    };

    assert_eq!(
        attacked(TWO_HUNDRED),
        value(105),
        "pushed high, the top honest report becomes the median"
    );
    assert_eq!(
        attacked(&[0, 0, 0, 1]),
        value(95),
        "pushed low, the bottom one does"
    );

    // And the bound: the attacker cannot reach past the honest reports, however
    // far it pushes. Without this the test above would not distinguish influence
    // from control.
    assert_ne!(attacked(TWO_HUNDRED), value(200), "200 is not reachable");
    assert_ne!(attacked(&[0, 0, 0, 1]), value(1), "nor is 1");
}
