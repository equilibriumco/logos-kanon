//! What must hold for every input, rather than for the inputs somebody thought
//! of.
//!
//! The example tests next to each module say what the code does in a case a
//! reader can follow. These say what stays true across a generated space, and
//! they are here for the three places where the space is too large to enumerate
//! and a wrong answer is expensive:
//!
//! - **the threshold**, where the invariant is that `M` gates a count it does
//!   not influence, so lowering it can never turn a price into a rejection;
//! - **decoding**, where the payload is attacker-supplied and the invariant is
//!   that anything well formed survives a round trip and anything else is
//!   refused without panicking;
//! - **the scale conversion**, where the invariant is an arithmetic identity
//!   over a domain of `2^128` inputs.
//!
//! Panic-freedom is the fourth, and it cuts across all three: ADR 4 makes a
//! panic in guest-reachable code an aborted transaction rather than a rejected
//! payload, so a corrupted payload that panics is a denial of service for
//! everyone using the feed. The last property here walks the composed path —
//! decode, recover, threshold, convert — over mutated payloads for exactly that
//! reason.
//!
//! `proptest`'s default of 256 cases per property stands, including for the two
//! that sign and recover real signatures: the whole module is half a second in
//! release, which is what CI runs, and eight in a debug `cargo test`.
//!
//! A failure writes the case that caused it to `proptest-regressions/`, and that
//! file is committed. It is the one place in this repository where a test's
//! input is decided by a random seed, so a failure that is not written down is a
//! failure that may not recur; see ADR 21.

extern crate std;

use std::vec::Vec;

use proptest::prelude::*;

use crate::{
    backend::{InProgramBackend, Signature, SignerAddress},
    decode::{Payload, EMPTY_ENVELOPE_BYTES, FEED_ID_BYTES},
    error::VerifyError,
    feed::{verify_feed, AssetPair, FeedConfig, VerifiedFeed, MAX_SIGNERS},
    test_support::{address_of, signing_key, PayloadBuilder},
    time::{TimeError, TimeSource},
    value::{median, Value, MAX_DECIMALS},
};

const NOW_MS: u64 = 1_770_000_000_000;
const MAX_AGE_MS: u64 = 60_000;
const DECIMALS: u8 = 8;
const FEED: &[u8] = b"BTC";

struct FixedClock;

impl TimeSource for FixedClock {
    fn now_ms(&self) -> Result<u64, TimeError> {
        Ok(NOW_MS)
    }
}

fn pair() -> AssetPair {
    AssetPair::new([0xB7; AssetPair::ID_LEN], [0x05; AssetPair::ID_LEN])
}

fn value(number: u128) -> Value {
    Value::from_be_slice(&number.to_be_bytes()).expect("16 bytes fit in 32")
}

/// The exponent's power of ten, as a `u128` so the identity below can be
/// written without a second implementation of the thing under test.
fn pow10(decimals: u8) -> u128 {
    10u128.pow(u32::from(decimals))
}

/// A set of reported values, over the whole 32-byte width rather than the range
/// a price plausibly occupies.
///
/// Full width on purpose. The midpoint is written as `(a >> 1) + (b >> 1) +
/// carry` precisely because `a + b` does not fit, and two values drawn from
/// `u128` can never make it not fit: they leave the sixteen high bytes zero, so
/// a generator built on `u128` agrees with the overflowing implementation on
/// every case it produces.
fn reports(count: core::ops::RangeInclusive<usize>) -> impl Strategy<Value = Vec<Value>> {
    prop::collection::vec(prop::array::uniform32(any::<u8>()).prop_map(Value), count)
}

proptest! {
    /// The conversion is `(value << 64) / 10^decimals`, truncating.
    ///
    /// Stated over `u64` inputs because that is the widest domain where the
    /// identity can be written in `u128` arithmetic — `value << 64` needs 192
    /// bits in general. It covers every price a feed can report: a
    /// `10^8`-scaled bitcoin price is around `2^43`.
    #[test]
    fn the_scale_conversion_is_a_shift_and_a_truncating_divide(
        number in any::<u64>(),
        decimals in 0u8..=MAX_DECIMALS,
    ) {
        let expected = (u128::from(number) << 64) / pow10(decimals);
        prop_assert_eq!(value(u128::from(number)).to_q64_64(decimals), Some(expected));
    }

    /// A price converts exactly when the account's scale can hold it, and the
    /// boundary is `2^64 * 10^decimals` rather than anything softer.
    ///
    /// Both sides of the boundary are generated: at nineteen decimals the limit
    /// is about `1.8 x 10^38`, and `u128` reaches `3.4 x 10^38`.
    #[test]
    fn the_representable_range_is_exactly_what_q64_64_can_hold(
        number in any::<u128>(),
        decimals in 0u8..=MAX_DECIMALS,
    ) {
        let limit = (1u128 << 64) * pow10(decimals);
        prop_assert_eq!(value(number).to_q64_64(decimals).is_some(), number < limit);
    }

    /// Zero is the price account's "no valid price" sentinel, so no real price
    /// may convert into it. The doc comment on `to_q64_64` claims this; the
    /// claim is worth more asserted over the domain than at one point.
    #[test]
    fn a_real_price_never_converts_to_the_no_price_sentinel(
        number in 1u128..,
        decimals in 0u8..=MAX_DECIMALS,
    ) {
        if let Some(price) = value(number).to_q64_64(decimals) {
            prop_assert!(price > 0, "{number} at 10^-{decimals} became the sentinel");
        }
    }

    /// Truncation may collapse two prices onto one, but it may never swap them.
    /// A conversion that reordered would make the median meaningless downstream.
    #[test]
    fn the_conversion_never_reorders_two_prices(
        low in any::<u128>(),
        high in any::<u128>(),
        decimals in 0u8..=MAX_DECIMALS,
    ) {
        let (low, high) = if low <= high { (low, high) } else { (high, low) };
        if let (Some(a), Some(b)) = (
            value(low).to_q64_64(decimals),
            value(high).to_q64_64(decimals),
        ) {
            prop_assert!(a <= b);
        }
    }

    /// The agreed price is one the signers bracket. A median outside the range
    /// its inputs span would mean the overflow-safe midpoint had wrapped.
    #[test]
    fn the_median_lies_between_the_lowest_and_the_highest_report(
        mut values in reports(1..=MAX_SIGNERS),
    ) {
        let low = *values.iter().min().expect("non-empty");
        let high = *values.iter().max().expect("non-empty");

        let agreed = median(&mut values).expect("non-empty");
        prop_assert!(low <= agreed && agreed <= high);
    }

    /// Packages arrive in whatever order the payload was written in, and an
    /// attacker chooses that order. The median must not.
    #[test]
    fn the_median_does_not_depend_on_the_order_the_reports_arrived_in(
        mut values in reports(1..=MAX_SIGNERS),
        swaps in prop::collection::vec((0usize..MAX_SIGNERS, 0usize..MAX_SIGNERS), 0..64),
    ) {
        let mut shuffled = values.clone();
        for (left, right) in swaps {
            let len = shuffled.len();
            shuffled.swap(left % len, right % len);
        }

        prop_assert_eq!(median(&mut values), median(&mut shuffled));
    }

    /// An odd number of reports agrees on a number one of them actually sent.
    /// Only the even case averages, and only the even case may invent a value.
    #[test]
    fn an_odd_number_of_reports_agrees_on_one_that_was_reported(
        mut values in reports(1..=MAX_SIGNERS).prop_map(|mut values| {
            if values.len() % 2 == 0 {
                values.pop();
            }
            values
        }),
    ) {
        let agreed = median(&mut values).expect("non-empty");
        prop_assert!(values.contains(&agreed));
    }
}

/// One package to build: its points all share a width, because the wire
/// declares `value_size` once per package.
#[derive(Debug, Clone)]
struct PackageSpec {
    points: Vec<(Vec<u8>, Vec<u8>)>,
    timestamp_ms: u64,
    signature_byte: u8,
}

fn package_spec() -> impl Strategy<Value = PackageSpec> {
    (1usize..=32, 1usize..=4).prop_flat_map(|(width, count)| {
        (
            prop::collection::vec(
                (
                    prop::collection::vec(any::<u8>(), 1..=FEED_ID_BYTES),
                    prop::collection::vec(any::<u8>(), width..=width),
                ),
                count..=count,
            ),
            // Six bytes on the wire, so a timestamp above 2^48 is not one this
            // format can carry.
            0u64..(1 << 48),
            any::<u8>(),
        )
            .prop_map(|(points, timestamp_ms, signature_byte)| PackageSpec {
                points,
                timestamp_ms,
                signature_byte,
            })
    })
}

fn payload_spec() -> impl Strategy<Value = (Vec<PackageSpec>, Vec<u8>)> {
    (
        prop::collection::vec(package_spec(), 1..=6),
        prop::collection::vec(any::<u8>(), 0..=40),
    )
}

fn build(packages: &[PackageSpec], metadata: &[u8]) -> Vec<u8> {
    let mut builder = PayloadBuilder::default().metadata(metadata);
    for package in packages {
        let points: Vec<(&[u8], &[u8])> = package
            .points
            .iter()
            .map(|(feed, value)| (feed.as_slice(), value.as_slice()))
            .collect();
        builder = builder.opaque_package(&points, package.timestamp_ms, package.signature_byte);
    }
    builder.build()
}

/// The feed id as the wire carries it: right-padded with zeros to 32 bytes.
fn padded(feed: &[u8]) -> [u8; FEED_ID_BYTES] {
    let mut out = [0u8; FEED_ID_BYTES];
    out[..feed.len()].copy_from_slice(feed);
    out
}

proptest! {
    /// Every well-formed payload decodes back to what was written into it:
    /// the same packages, the same points, the same timestamps and signatures,
    /// byte for byte — and in the reverse order, which is a documented property
    /// of the format rather than an accident of the walk.
    ///
    /// Framing is what this exercises. The widths, counts and metadata length
    /// are all declared inside the payload, so a decoder that read one of them
    /// from the wrong place still works on a payload where they happen to agree.
    #[test]
    fn a_payload_decodes_back_to_what_was_written_into_it(
        (packages, metadata) in payload_spec(),
    ) {
        let bytes = build(&packages, &metadata);
        let payload = Payload::decode(&bytes).expect("the builder writes well-formed payloads");
        prop_assert_eq!(payload.package_count(), packages.len());

        let mut visited = 0usize;
        let walk = payload.for_each_package(|package| {
            // Last-first: the payload is walked backwards.
            let spec = &packages[packages.len() - 1 - visited];
            visited += 1;

            if package.timestamp_ms != spec.timestamp_ms {
                return Err("timestamp");
            }
            if package.signature != Signature([spec.signature_byte; Signature::LEN]) {
                return Err("signature");
            }
            if package.len() != spec.points.len() {
                return Err("point count");
            }
            for (point, (feed, value)) in package.data_points().zip(spec.points.iter()) {
                if point.feed_id != &padded(feed) || point.value != value.as_slice() {
                    return Err("data point");
                }
            }
            Ok(())
        });

        prop_assert_eq!(walk, Ok(Ok(())));
        prop_assert_eq!(visited, packages.len());
    }
}

/// A feed configured for the first `signers` deterministic keys, with a payload
/// signed by the first `reporting` of them, each reporting a distinct price.
fn signed_feed(signers: usize, reporting: usize, base_price: u64) -> (Vec<SignerAddress>, Vec<u8>) {
    let keys: Vec<_> = (0..signers)
        .map(|index| signing_key(index as u8 + 1))
        .collect();
    let addresses = keys.iter().map(address_of).collect();

    let mut builder = PayloadBuilder::default();
    for (index, key) in keys.iter().take(reporting).enumerate() {
        // Distinct per signer, so a median that silently picked the first
        // report rather than the middle one would show up as a changed price.
        let price = base_price.saturating_add(index as u64);
        builder = builder.signed_package(key, &[(FEED, &price.to_be_bytes())], NOW_MS);
    }

    (addresses, builder.build())
}

fn verify(
    bytes: &[u8],
    signers: &[SignerAddress],
    threshold: u8,
) -> Result<VerifiedFeed, VerifyError> {
    let config = FeedConfig::try_new(FEED, pair(), DECIMALS, MAX_AGE_MS, signers, threshold)?;
    let payload = Payload::decode(bytes)?;
    verify_feed(
        &payload,
        &config,
        &pair(),
        &InProgramBackend::new(),
        &FixedClock,
    )
}

proptest! {
    /// The threshold gates a count it does not influence. So for one payload and
    /// one signer set there is a single number — how many authorised signers
    /// reported — and every threshold at or below it verifies to the *same*
    /// price, while every threshold above it fails, and fails by naming that
    /// number.
    ///
    /// This is the invariant that makes `M` a policy dial rather than part of
    /// the verification: if the price moved with the threshold, an operator
    /// raising `M` would be changing what the feed says, not how sure it is.
    #[test]
    fn lowering_the_threshold_never_changes_the_price_and_never_loses_it(
        signers in 1usize..=5,
        reporting in 1usize..=5,
        base_price in 1u64..1_000_000,
    ) {
        let reporting = reporting.min(signers);
        let (addresses, bytes) = signed_feed(signers, reporting, base_price);
        let met = u8::try_from(reporting).expect("at most five");

        let agreed = verify(&bytes, &addresses, met).expect("the threshold it exactly meets");
        prop_assert_eq!(agreed.signers, met);

        for threshold in 1..=u8::try_from(signers).expect("at most five") {
            let outcome = verify(&bytes, &addresses, threshold);
            if threshold <= met {
                prop_assert_eq!(outcome, Ok(agreed), "threshold {} of {}", threshold, signers);
            } else {
                prop_assert_eq!(
                    outcome,
                    Err(VerifyError::ThresholdNotMet { met, required: threshold })
                );
            }
        }
    }

    /// The composed path — decode, recover, count, take the median, convert —
    /// over payloads that have been corrupted after signing.
    ///
    /// ADR 4 is what makes this worth generating rather than reasoning about: a
    /// panic here aborts the transaction instead of rejecting the payload, so a
    /// single malformed package would take the feed down for everyone rather
    /// than for its sender. Any answer is acceptable. Not answering is not.
    ///
    /// The corruption lands inside the packages and the envelope is left
    /// intact, deliberately. A payload whose marker or metadata length has been
    /// touched fails at the first few bytes of decoding, and the decoder's own
    /// tests already cover that; leaving it alone is what makes most cases
    /// reach recovery, the threshold and the conversion, which is the path this
    /// property exists to walk.
    #[test]
    fn a_corrupted_payload_is_answered_rather_than_panicked_on(
        signers in 1usize..=4,
        mutations in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..4),
        truncate_by in prop_oneof![8 => Just(0usize), 1 => 1usize..16],
        threshold in 1u8..=4,
    ) {
        let (addresses, bytes) = signed_feed(signers, signers, 42_000);
        let (body, tail) = bytes.split_at(bytes.len() - EMPTY_ENVELOPE_BYTES);

        let mut body = body.to_vec();
        for (index, byte) in mutations {
            let len = body.len();
            body[index.index(len)] = byte;
        }
        body.truncate(body.len().saturating_sub(truncate_by));
        body.extend_from_slice(tail);

        let threshold = threshold.min(u8::try_from(signers).expect("at most four"));
        let _ = verify(&body, &addresses, threshold);
    }
}
