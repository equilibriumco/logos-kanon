//! Conformance against payloads RedStone actually published.
//!
//! Every other test in this crate builds its own payloads, which proves that
//! the builder and the decoder agree with each other and nothing more. These
//! vectors are different: the packages and the signatures in them came off
//! RedStone's gateway, so the assertions here are against a system nobody in
//! this repository controls.
//!
//! The signature is what does the work. It is RedStone's, computed over
//! RedStone's own serialisation of the package, so it can only recover to the
//! address they published if this decoder's view of the signed span — the field
//! order, the widths, and which bytes the signature covers — matches theirs
//! byte for byte. `signed_span_covers_the_points_and_the_three_trailing_fields`
//! asserts that span against a payload we built; this asserts it against one we
//! did not.
//!
//! The envelope around the packages — count, unsigned metadata size, marker — is
//! not covered by any signature, and in the captured vectors it was assembled by
//! `scripts/capture-redstone-vectors.py`: the gateway serves per-signer JSON with
//! decoded values, not a finished payload, so there is no served envelope to
//! capture. That half is checked by
//! `an_envelope_redstone_published_decodes_and_every_package_in_it_recovers`,
//! against a payload RedStone serialised themselves.
//!
//! Vectors are committed and read from disk. A test that reached the network
//! would fail on a bad day for reasons that have nothing to do with this code,
//! and would stop meaning anything the moment the market moved.

use verifier_core::{
    backend::{InProgramBackend, SignerAddress, VerifierBackend},
    decode::Payload,
    feed::{verify_feed, AssetPair, FeedConfig},
    time::{TimeError, TimeSource},
    value::Value,
};

#[path = "support/vectors.rs"]
mod vectors;

use vectors::all as vectors;

/// RedStone's own scale for `redstone-primary-prod`, confirmed by the capture:
/// no other exponent reproduces a signature they published.
const DECIMALS: u8 = 8;

struct FixedClock(u64);

impl TimeSource for FixedClock {
    fn now_ms(&self) -> Result<u64, TimeError> {
        Ok(self.0)
    }
}

fn pair() -> AssetPair {
    AssetPair::new([1; AssetPair::ID_LEN], [2; AssetPair::ID_LEN])
}

/// Generous, because these vectors age from the moment they are captured. What
/// is under test here is the format, not the clock; staleness has its own tests
/// against a clock the test controls.
const FOREVER: u64 = u64::MAX;

#[test]
fn every_captured_payload_decodes() {
    let vectors = vectors();
    assert!(!vectors.is_empty(), "the vector file is empty");

    for vector in &vectors {
        let payload = Payload::decode(&vector.payload)
            .unwrap_or_else(|e| panic!("{} did not decode: {e:?}", vector.feed_id));
        assert_eq!(
            payload.package_count(),
            vector.signers.len(),
            "{} package count",
            vector.feed_id
        );
    }
}

#[test]
fn every_published_signature_recovers_to_the_signer_redstone_named() {
    // The assertion the whole file exists for. If the signed span were wrong by
    // one byte in either direction, not one of these would recover.
    let backend = InProgramBackend::new();

    for vector in vectors() {
        let payload = Payload::decode(&vector.payload).expect("decodes");
        let mut recovered = Vec::new();

        payload
            .for_each_package(|package| {
                let digest = backend.keccak256(package.signable());
                let signer = backend
                    .recover_signer(&digest, &package.signature)
                    .unwrap_or_else(|e| panic!("{} recovery failed: {e:?}", vector.feed_id));
                recovered.push(signer);
                Ok::<(), core::convert::Infallible>(())
            })
            .expect("walks")
            .expect("no visitor error");

        let mut got = recovered.clone();
        let mut want = vector.signers.clone();
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!(
            got, want,
            "{}: recovered signers are not the ones RedStone published",
            vector.feed_id
        );
    }
}

#[test]
fn every_published_value_survives_the_decoder() {
    let vectors = vectors();

    for vector in &vectors {
        let payload = Payload::decode(&vector.payload).expect("decodes");
        let mut seen = Vec::new();

        payload
            .for_each_package(|package| {
                for point in package.data_points() {
                    assert_eq!(
                        package.timestamp_ms, vector.timestamp_ms,
                        "{} timestamp",
                        vector.feed_id
                    );
                    assert_eq!(
                        point.feed_id_trimmed(),
                        vector.feed_id.as_bytes(),
                        "{} feed id",
                        vector.feed_id
                    );
                    seen.push(Value::from_be_slice(point.value).expect("a representable value"));
                }
                Ok::<(), core::convert::Infallible>(())
            })
            .expect("walks")
            .expect("no visitor error");

        let mut got = seen;
        let mut want = vector.values.clone();
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!(got, want, "{} values", vector.feed_id);
    }
}

#[test]
fn a_published_payload_verifies_end_to_end() {
    // Everything at once, through the public entry point: real packages, real
    // signatures, the real signer set, at the threshold the RFP names as the
    // default.
    for vector in vectors() {
        let payload = Payload::decode(&vector.payload).expect("decodes");
        let config = FeedConfig::try_new(
            vector.feed_id.as_bytes(),
            pair(),
            DECIMALS,
            FOREVER,
            &vector.signers,
            3,
        )
        .expect("valid config");

        let verified = verify_feed(
            &payload,
            &config,
            &pair(),
            &InProgramBackend::new(),
            &FixedClock(vector.timestamp_ms),
        )
        .unwrap_or_else(|e| panic!("{} did not verify: {e:?}", vector.feed_id));

        assert_eq!(
            usize::from(verified.signers),
            vector.signers.len(),
            "{} signer count",
            vector.feed_id
        );

        let mut expected = vector.values.clone();
        let median = verifier_core::value::median(&mut expected).expect("a median");
        assert_eq!(verified.value, median, "{} median", vector.feed_id);

        assert!(
            verified.price > 0,
            "{}: a real price must not reach the account's zero sentinel",
            vector.feed_id
        );
    }
}

#[test]
fn a_signer_set_that_is_not_the_published_one_reaches_no_threshold() {
    // The mirror of the recovery test. Real signatures, real packages, and a
    // signer set of strangers: if this verified, the signer check would be
    // decorative.
    let strangers: Vec<SignerAddress> = (1..=5u8)
        .map(|b| SignerAddress([b; SignerAddress::LEN]))
        .collect();

    for vector in vectors() {
        let payload = Payload::decode(&vector.payload).expect("decodes");
        let config = FeedConfig::try_new(
            vector.feed_id.as_bytes(),
            pair(),
            DECIMALS,
            FOREVER,
            &strangers,
            3,
        )
        .expect("valid config");

        assert!(
            verify_feed(
                &payload,
                &config,
                &pair(),
                &InProgramBackend::new(),
                &FixedClock(vector.timestamp_ms),
            )
            .is_err(),
            "{} verified against a signer set that never signed it",
            vector.feed_id
        );
    }
}

#[test]
fn a_wrong_signed_span_recovers_to_nobody() {
    // The negative control, without which the test above proves nothing: a
    // check that cannot fail is not a check. Each of these is a plausible way
    // to get the span wrong -- stopping before the trailing fields, taking the
    // points alone, hashing the whole package including its signature -- and
    // none of them may recover to a signer RedStone published.
    //
    // Measured against these vectors when they were captured: the correct span
    // recovers every package and each of seven wrong ones recovers none. That
    // gap is the test's whole value.
    let backend = InProgramBackend::new();

    for vector in vectors() {
        let payload = Payload::decode(&vector.payload).expect("decodes");
        let published = vector.signers.clone();

        payload
            .for_each_package(|package| {
                let correct = package.signable();
                let wrong = [
                    // Short of `point_count`.
                    correct.get(..correct.len() - 3),
                    // Short of `value_size` and `point_count` both.
                    correct.get(..correct.len() - 7),
                    // The data points alone, with none of the trailing fields.
                    correct.get(..correct.len() - 13),
                    // One byte long, the off-by-one nobody would notice by eye.
                    correct.get(..correct.len() - 1),
                ];

                for span in wrong.into_iter().flatten() {
                    let digest = backend.keccak256(span);
                    if let Ok(signer) = backend.recover_signer(&digest, &package.signature) {
                        assert!(
                            !published.contains(&signer),
                            "{}: a wrong span recovered a published signer, so the \
                             conformance assertion above proves nothing",
                            vector.feed_id
                        );
                    }
                }
                Ok::<(), core::convert::Infallible>(())
            })
            .expect("walks")
            .expect("no visitor error");
    }
}

/// RedStone's own sample payload, vendored from their SDK.
///
/// `verifier-core/tests/vectors/redstone-sdk-sample-payload.hex` is a byte-for-byte
/// copy of `sample-data/payload.hex` in
/// <https://github.com/redstone-finance/rust-sdk>, at commit
/// `50cd703eb908d249d65f322192af1757e56afc97`. Boost Software License 1.0; `NOTICE`
/// records it.
const PUBLISHED_PAYLOAD: &str = include_str!("vectors/redstone-sdk-sample-payload.hex");

/// The signer set RedStone publishes for this payload's data service, copied from
/// `crates/samples/src/package_signers.rs` (`AVAX_SIGNERS`) at the same commit.
const PUBLISHED_SIGNERS: [&str; 5] = [
    "109b4a318a4f5ddcbca6349b45f881b4137deafb",
    "12470f7aba85c8b81d63137dd5925d6ee114952b",
    "1ea62d73edf8ac05dfcea1a34b9796e937a29eff",
    "2c59617248994d12816ee1fa77ce0a64eeb456bf",
    "83cba8c619fb629b81a65c2e67fe15cf3e3c9747",
];

#[test]
fn an_envelope_redstone_published_decodes_and_every_package_in_it_recovers() {
    // The one thing the rest of this file cannot check. Our other vectors carry
    // RedStone's packages inside an envelope this repository assembled, so the
    // marker, the package count and the metadata size are only ever read back by
    // the code that wrote them. This payload was serialised by RedStone, so those
    // three fields are checked against a producer nobody here controls.
    //
    // The two structural claims are the ones RedStone's own decoder test asserts
    // over the same file: fifteen packages, and nothing left over. `Payload::decode`
    // returns `TrailingBytes` rather than ignoring a remainder, so decoding at all
    // is the second assertion.
    let bytes = hex::decode(PUBLISHED_PAYLOAD.trim()).expect("vendored payload is hex");
    let payload = Payload::decode(&bytes).expect("a payload RedStone serialised");

    assert_eq!(
        payload.package_count(),
        15,
        "fifteen packages, as their own test asserts"
    );

    // Three feeds of five signers, so a wrong envelope would still have to produce
    // fifteen recoverable signatures over the right spans to get this far.
    let backend = InProgramBackend::new();
    let mut recovered = Vec::new();
    payload
        .for_each_package(|package| {
            let digest = backend.keccak256(package.signable());
            let signer = backend
                .recover_signer(&digest, &package.signature)
                .expect("every package in a published payload recovers");
            recovered.push(hex::encode(signer.0));
            Ok::<(), core::convert::Infallible>(())
        })
        .expect("the walk reaches the end")
        .expect("no package is refused");

    assert_eq!(recovered.len(), 15);
    let mut distinct = recovered;
    distinct.sort_unstable();
    distinct.dedup();
    let mut want: Vec<String> = PUBLISHED_SIGNERS.iter().map(|s| (*s).to_owned()).collect();
    want.sort_unstable();
    assert_eq!(
        distinct, want,
        "the fifteen signatures recover to the five signers RedStone publishes for this service"
    );
}
