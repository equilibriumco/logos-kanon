//! Payload fixtures. Compiled only under `cfg(test)`.
//!
//! `decode`'s tests need framing they control byte for byte; `feed`'s tests need
//! signatures that recover to a known address. One builder serves both:
//! `opaque_package` fills the signature with a chosen byte, `signed_package`
//! signs the span the decoder will hand to keccak256.

extern crate std;

use std::vec::Vec;

use k256::ecdsa::SigningKey;
use tiny_keccak::{Hasher, Keccak};

use crate::{
    backend::{Signature, SignerAddress},
    decode::{FEED_ID_BYTES, REDSTONE_MARKER},
};

/// A deterministic key, so every assertion is reproducible.
///
/// `seed` must not be zero: an all-zero scalar is not a valid secret key, and
/// this would panic. Every caller here uses 1 or above.
pub fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32].into()).expect("a non-zero repeated byte is a valid scalar")
}

/// The Ethereum address of a key: keccak256 of the uncompressed point without its
/// `0x04` tag, last twenty bytes.
pub fn address_of(key: &SigningKey) -> SignerAddress {
    let point = key.verifying_key().to_encoded_point(false);
    let mut hasher = Keccak::v256();
    hasher.update(&point.as_bytes()[1..]);
    let mut digest = [0u8; 32];
    hasher.finalize(&mut digest);
    let mut address = [0u8; SignerAddress::LEN];
    address.copy_from_slice(&digest[12..]);
    SignerAddress(address)
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

#[derive(Default)]
pub struct PayloadBuilder {
    packages: Vec<Vec<u8>>,
    metadata: Vec<u8>,
}

impl PayloadBuilder {
    /// The signed span: data points, then timestamp, value size and point count.
    fn body(points: &[(&[u8], &[u8])], timestamp_ms: u64) -> Vec<u8> {
        let value_size = points.first().map_or(0, |(_, value)| value.len());
        let mut out = Vec::new();
        for (feed, value) in points {
            let mut id = [0u8; FEED_ID_BYTES];
            id[..feed.len()].copy_from_slice(feed);
            out.extend_from_slice(&id);
            out.extend_from_slice(value);
        }
        out.extend_from_slice(&timestamp_ms.to_be_bytes()[2..]);
        out.extend_from_slice(&u32::try_from(value_size).unwrap_or(0).to_be_bytes());
        out.extend_from_slice(&u32::try_from(points.len()).unwrap_or(0).to_be_bytes()[1..]);
        out
    }

    /// A package whose signature is `sig_byte` repeated. For framing tests.
    pub fn opaque_package(
        mut self,
        points: &[(&[u8], &[u8])],
        timestamp_ms: u64,
        sig_byte: u8,
    ) -> Self {
        let mut out = Self::body(points, timestamp_ms);
        out.extend_from_slice(&[sig_byte; Signature::LEN]);
        self.packages.push(out);
        self
    }

    /// A package signed by `key` over exactly the span the decoder will produce.
    pub fn signed_package(
        mut self,
        key: &SigningKey,
        points: &[(&[u8], &[u8])],
        timestamp_ms: u64,
    ) -> Self {
        let body = Self::body(points, timestamp_ms);
        let (signature, recovery_id) = key
            .sign_prehash_recoverable(&keccak256(&body))
            .expect("signing a 32-byte prehash");

        let mut out = body;
        out.extend_from_slice(&signature.to_bytes());
        out.push(recovery_id.to_byte());
        self.packages.push(out);
        self
    }

    pub fn metadata(mut self, bytes: &[u8]) -> Self {
        self.metadata = bytes.to_vec();
        self
    }

    pub fn build(self) -> Vec<u8> {
        let mut out = Vec::new();
        for package in &self.packages {
            out.extend_from_slice(package);
        }
        out.extend_from_slice(&(self.packages.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.metadata);
        out.extend_from_slice(
            &u32::try_from(self.metadata.len())
                .unwrap_or(0)
                .to_be_bytes()[1..],
        );
        out.extend_from_slice(&REDSTONE_MARKER);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::{InProgramBackend, VerifierBackend},
        decode::Payload,
    };

    #[test]
    fn a_signed_package_recovers_to_the_key_that_signed_it() {
        // The fixture's own correctness check. If this fails, every threshold
        // test built on it is testing the builder rather than the verifier.
        let key = signing_key(0x11);
        let bytes = PayloadBuilder::default()
            .signed_package(&key, &[(b"BTC", &[0, 0, 0, 42])], 1_770_000_000_000)
            .build();

        let payload = Payload::decode(&bytes).expect("well formed");
        let backend = InProgramBackend::new();

        let mut recovered = None;
        payload
            .for_each_package(|package| {
                let digest = backend.keccak256(package.signable());
                recovered = backend.recover_signer(&digest, &package.signature).ok();
                Ok::<(), ()>(())
            })
            .expect("decodes")
            .expect("no rejection");

        assert_eq!(recovered, Some(address_of(&key)));
    }
}
