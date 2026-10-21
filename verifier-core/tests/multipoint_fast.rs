//! Conformance against a multi-point package RedStone actually published.
//!
//! `redstone_conformance.rs` proves the format against single-point packages:
//! one feed, one value, one signature. This vector is RedStone's `___FAST___`
//! package set off the live `redstone-primary-prod` gateway — nine data points
//! (USDC, USDT, DAI, BNB, BTC, ETH, SOL, MON, MEGA) under one signature per
//! signer, five signers — captured by the same reconstruct-until-the-signature-
//! recovers method as the committed vectors (`scratchpad capture, per-point
//! rounding search`). What it establishes:
//!
//! 1. The decoder's view of a multi-point signed span matches RedStone's byte
//!    for byte, against signatures nobody in this repository produced.
//! 2. `verify_feed` counts a package toward a feed when the feed's point is one
//!    of many — BTC, ETH and SOL each verify to the full five-signer roster out
//!    of the same payload, at the production threshold.
//! 3. A feed that is not in the package (XMR) reaches no threshold against it.
//!
//! Which is the mechanism behind the 25-recoveries-to-15 (or, trimmed, 9)
//! reduction discussed with Logos: one recovery attests three of our feeds.

use verifier_core::{
    backend::{InProgramBackend, SignerAddress, VerifierBackend},
    decode::Payload,
    feed::{verify_feed, AssetPair, FeedConfig},
    time::{TimeError, TimeSource},
    value::Value,
};

const VECTOR: &str = include_str!("vectors/fast-multipoint.json");
const DECIMALS: u8 = 8;
const THRESHOLD: u8 = 3;
const POINTS: usize = 9;
const OUR_FEEDS_IN_FAST: [&str; 3] = ["BTC", "ETH", "SOL"];

struct FixedClock(u64);

impl TimeSource for FixedClock {
    fn now_ms(&self) -> Result<u64, TimeError> {
        Ok(self.0)
    }
}

fn pair() -> AssetPair {
    AssetPair::new([1; AssetPair::ID_LEN], [2; AssetPair::ID_LEN])
}

struct Capture {
    timestamp_ms: u64,
    signers: Vec<SignerAddress>,
    values_per_feed: std::collections::BTreeMap<String, Vec<Value>>,
    payload: Vec<u8>,
}

fn capture() -> Capture {
    let v: serde_json::Value = serde_json::from_str(VECTOR).expect("vector parses");
    Capture {
        timestamp_ms: v["timestamp_ms"].as_u64().expect("timestamp"),
        signers: v["signers"]
            .as_array()
            .expect("signers")
            .iter()
            .map(|s| {
                let bytes = hex::decode(s.as_str().expect("signer").trim_start_matches("0x"))
                    .expect("signer hex");
                SignerAddress(bytes.try_into().expect("twenty bytes"))
            })
            .collect(),
        values_per_feed: v["values_per_feed"]
            .as_object()
            .expect("values per feed")
            .iter()
            .map(|(feed, values)| {
                let values = values
                    .as_array()
                    .expect("values")
                    .iter()
                    .map(|value| {
                        let digits: u128 =
                            value.as_str().expect("value").parse().expect("an integer");
                        Value::from_be_slice(&digits.to_be_bytes()).expect("sixteen bytes fit")
                    })
                    .collect();
                (feed.clone(), values)
            })
            .collect(),
        payload: hex::decode(v["payload_hex"].as_str().expect("payload")).expect("payload hex"),
    }
}

#[test]
fn the_multipoint_payload_decodes_with_nine_points_per_package() {
    let capture = capture();
    let payload = Payload::decode(&capture.payload).expect("decodes");
    assert_eq!(payload.package_count(), capture.signers.len());

    payload
        .for_each_package(|package| {
            assert_eq!(package.data_points().count(), POINTS, "points per package");
            Ok::<(), core::convert::Infallible>(())
        })
        .expect("walks")
        .expect("no visitor error");
}

#[test]
fn every_multipoint_signature_recovers_to_the_signer_redstone_named() {
    let capture = capture();
    let payload = Payload::decode(&capture.payload).expect("decodes");
    let backend = InProgramBackend::new();
    let mut recovered = Vec::new();

    payload
        .for_each_package(|package| {
            let digest = backend.keccak256(package.signable());
            recovered.push(
                backend
                    .recover_signer(&digest, &package.signature)
                    .expect("recovery over a nine-point signed span"),
            );
            Ok::<(), core::convert::Infallible>(())
        })
        .expect("walks")
        .expect("no visitor error");

    let mut got = recovered;
    let mut want = capture.signers.clone();
    got.sort_unstable();
    want.sort_unstable();
    assert_eq!(
        got, want,
        "recovered signers are not the ones RedStone published"
    );
}

#[test]
fn each_of_our_feeds_verifies_to_the_full_roster_out_of_one_payload() {
    // The claim under test: one five-signature payload carries BTC, ETH and
    // SOL at once, and each verifies exactly as it would from its own payload.
    let capture = capture();
    let payload = Payload::decode(&capture.payload).expect("decodes");

    for feed in OUR_FEEDS_IN_FAST {
        let config = FeedConfig::try_new(
            feed.as_bytes(),
            pair(),
            DECIMALS,
            verifier_core::feed::MAX_MAX_AGE_MS,
            &capture.signers,
            THRESHOLD,
        )
        .expect("valid config");

        let verified = verify_feed(
            &payload,
            &config,
            &pair(),
            &InProgramBackend::new(),
            &FixedClock(capture.timestamp_ms),
        )
        .unwrap_or_else(|e| panic!("{feed} did not verify from the multipoint payload: {e:?}"));

        assert_eq!(
            usize::from(verified.signers),
            capture.signers.len(),
            "{feed}: all five signers should count"
        );

        let mut expected = capture.values_per_feed[feed].clone();
        let median = verifier_core::value::median(&mut expected).expect("a median");
        assert_eq!(verified.value, median, "{feed} median");
        assert!(verified.price > 0, "{feed}: zero sentinel");
    }
}

#[test]
fn a_feed_outside_the_package_reaches_no_threshold() {
    // XMR is on the service but not in ___FAST___: the same payload must not
    // verify it, or a package would vouch for feeds it never priced.
    let capture = capture();
    let payload = Payload::decode(&capture.payload).expect("decodes");
    let config = FeedConfig::try_new(
        b"XMR",
        pair(),
        DECIMALS,
        verifier_core::feed::MAX_MAX_AGE_MS,
        &capture.signers,
        THRESHOLD,
    )
    .expect("valid config");

    assert!(
        verify_feed(
            &payload,
            &config,
            &pair(),
            &InProgramBackend::new(),
            &FixedClock(capture.timestamp_ms),
        )
        .is_err(),
        "XMR verified from a package that does not carry it"
    );
}
