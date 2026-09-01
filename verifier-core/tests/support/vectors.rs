//! The committed RedStone capture, read once.
//!
//! Three test targets in three packages read this file: `redstone_conformance`
//! here, `cost` in `kanon-methods`, and `settle` in `reference-consumer-pull`.
//! Included by path rather than shared through a crate, because a fixture reader
//! has no business in the published surface of a crate consumer programs link --
//! the reason ADR 21 already gives for keeping `test_support` behind `cfg(test)`.
//! Each of the three declares `serde_json` and `hex` as dev-dependencies for it,
//! so a new import here is a change to three manifests.

#![allow(dead_code)] // Each consumer reads a different subset of the fields.

use verifier_core::{backend::SignerAddress, value::Value};

pub const VECTORS: &str = include_str!("../vectors/redstone-primary-prod.json");

/// One feed's capture: the payload RedStone's packages were assembled into, and
/// what the tests expect to come back out of it.
pub struct Vector {
    pub feed_id: String,
    pub timestamp_ms: u64,
    pub signers: Vec<SignerAddress>,
    pub values: Vec<Value>,
    pub payload: Vec<u8>,
}

pub fn all() -> Vec<Vector> {
    let parsed: serde_json::Value = serde_json::from_str(VECTORS).expect("vectors parse");
    parsed["vectors"]
        .as_array()
        .expect("a vectors array")
        .iter()
        .map(parse)
        .collect()
}

pub fn named(feed: &str) -> Vector {
    all()
        .into_iter()
        .find(|v| v.feed_id == feed)
        .unwrap_or_else(|| panic!("{feed} is not in the capture"))
}

fn parse(v: &serde_json::Value) -> Vector {
    Vector {
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
    }
}
