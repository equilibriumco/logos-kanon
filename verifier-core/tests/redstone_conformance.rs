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
//! The envelope around the packages — count, unsigned metadata, size, marker —
//! was assembled by `scripts/capture-redstone-vectors.py` per the wire format,
//! because the gateway serves packages rather than a finished payload and
//! RedStone's own SDK assembles it client-side the same way. So the honest claim
//! is: the signed span is conformance-tested, and the envelope is built to spec.
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

const VECTORS: &str = include_str!("vectors/redstone-primary-prod.json");

/// RedStone's own scale for `redstone-primary-prod`, confirmed by the capture:
/// no other exponent reproduces a signature they published.
const DECIMALS: u8 = 8;

struct FixedClock(u64);

impl TimeSource for FixedClock {
    fn now_ms(&self) -> Result<u64, TimeError> {
        Ok(self.0)
    }
}

struct Vector {
    feed_id: String,
    timestamp_ms: u64,
    signers: Vec<SignerAddress>,
    values: Vec<Value>,
    payload: Vec<u8>,
}

fn vectors() -> Vec<Vector> {
    let parsed: serde_json::Value = serde_json::from_str(VECTORS).expect("vectors parse");
    parsed["vectors"]
        .as_array()
        .expect("a vectors array")
        .iter()
        .map(|v| Vector {
            feed_id: v["feed_id"].as_str().expect("feed id").to_owned(),
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
            values: v["values"]
                .as_array()
                .expect("values")
                .iter()
                .map(|value| {
                    let digits: u128 = value.as_str().expect("value").parse().expect("an integer");
                    Value::from_be_slice(&digits.to_be_bytes()).expect("sixteen bytes fit")
                })
                .collect(),
            payload: hex::decode(v["payload_hex"].as_str().expect("payload")).expect("payload hex"),
        })
        .collect()
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
