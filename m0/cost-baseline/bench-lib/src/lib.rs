//! The two primitive workloads RFP-020's cost question turns on.
//!
//! Every function takes a `do_work` flag. Running the same guest twice, once
//! with `do_work = true` and once with `false`, and subtracting the cycle
//! counts isolates the cryptographic work from the fixed costs that surround it
//! (zkVM startup, input deserialization, journal commit). Those fixed costs are
//! real but they are not the quantity under measurement, and they would
//! otherwise be attributed to the primitive and inflate it.

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use tiny_keccak::{Hasher, Keccak};

/// `iters` keccak256 hashes over a `msg_len`-byte message.
///
/// The first byte of the message is perturbed per iteration so the hashes are
/// genuinely distinct; without that the compiler is free to hoist a pure,
/// loop-invariant hash out of the loop, leaving one hash measured rather than
/// `iters` of them.
pub fn keccak_bench(do_work: bool, iters: u32, msg_len: u32) -> [u8; 32] {
    let mut msg = vec![0xa5u8; msg_len as usize];
    let mut acc = [0u8; 32];
    if !do_work {
        return acc;
    }
    for i in 0..iters {
        if !msg.is_empty() {
            msg[0] = (i & 0xff) as u8;
        }
        let mut hasher = Keccak::v256();
        hasher.update(&msg);
        let mut out = [0u8; 32];
        hasher.finalize(&mut out);
        for (a, o) in acc.iter_mut().zip(out.iter()) {
            *a ^= *o;
        }
    }
    acc
}

/// One data package per signer, as RedStone actually sends them: hash that
/// signer's own signed byte range, then recover that signer.
///
/// Each signer publishes its own value and its own millisecond timestamp, so an
/// M-of-N update carries M distinct messages and needs M hashes, not one hash
/// checked against M signatures.
///
/// `hash` toggles only whether each digest is computed in-guest or taken from the
/// input. Both settings recover identical digests with identical signatures, so
/// subtracting the two runs gives keccak256's marginal cost inside a real
/// verification rather than in a synthetic hashing loop. That distinction matters
/// because the keccak accelerator's work does not appear in the cycle count, so
/// its true share of an update is only visible in proof time and proof size.
///
/// Each package is `(signed_bytes, digest, signature, recovery_id)`.
pub fn verify_bench(hash: bool, packages: &[(Vec<u8>, Vec<u8>, Vec<u8>, u8)]) -> (u32, u8) {
    let mut recovered = 0u32;
    let mut checksum = 0u8;
    for (msg, digest, sig_bytes, recid) in packages {
        let digest: [u8; 32] = if hash {
            let mut hasher = Keccak::v256();
            hasher.update(msg);
            let mut out = [0u8; 32];
            hasher.finalize(&mut out);
            out
        } else {
            digest
                .as_slice()
                .try_into()
                .expect("digest must be 32 bytes")
        };

        let sig = Signature::from_slice(sig_bytes).expect("malformed signature");
        let recid = RecoveryId::from_byte(*recid).expect("bad recovery id");
        if let Ok(vk) = VerifyingKey::recover_from_prehash(&digest, &sig, recid) {
            recovered += 1;
            checksum ^= vk.to_encoded_point(false).as_bytes()[1];
        }
    }
    (recovered, checksum)
}

/// `keccak_bench`'s loop with the hash removed: the input perturbation, the
/// 32-byte XOR fold, and the loop itself, with a stand-in for the digest.
///
/// Those three things exist to stop the compiler deleting or hoisting the hash,
/// but they are inside the measured region, so they inflate the reported
/// per-hash cost. Subtracting this gives the hash alone. The `[i; 32]` stand-in
/// is at least as expensive as `finalize` writing its output, so this
/// overestimates the bookkeeping rather than flattering it.
pub fn keccak_loop_overhead(do_work: bool, iters: u32, msg_len: u32) -> [u8; 32] {
    let mut msg = vec![0xa5u8; msg_len as usize];
    let mut acc = [0u8; 32];
    if !do_work {
        return acc;
    }
    for i in 0..iters {
        if !msg.is_empty() {
            msg[0] = (i & 0xff) as u8;
        }
        let out = [(i & 0xff) as u8; 32];
        for (a, o) in acc.iter_mut().zip(out.iter()) {
            *a ^= *o;
        }
    }
    acc
}

/// One secp256k1 ECDSA public-key recovery per entry in `sigs`.
///
/// This is the shape of a real RedStone M-of-N check: several distinct signers
/// sign the *same* digest, and the verifier recovers each signer's address to
/// test it against the authorized set. Recovery is what makes it expensive --
/// it is a scalar multiplication on the curve, not a hash.
///
/// Returns `(recovered_count, checksum)`. The count lets the host assert that
/// every recovery actually succeeded, so a run that silently took the cheap
/// error path cannot be mistaken for a valid measurement. The checksum keeps
/// the recovered keys live so the work is not optimized away.
pub fn recover_bench(do_work: bool, digest: &[u8], sigs: &[(Vec<u8>, u8)]) -> (u32, u8) {
    if !do_work {
        return (0, 0);
    }
    let digest: [u8; 32] = digest.try_into().expect("digest must be 32 bytes");
    let mut recovered = 0u32;
    let mut checksum = 0u8;
    for (sig_bytes, recid) in sigs {
        let sig = Signature::from_slice(sig_bytes).expect("malformed signature");
        let recid = RecoveryId::from_byte(*recid).expect("bad recovery id");
        if let Ok(vk) = VerifyingKey::recover_from_prehash(&digest, &sig, recid) {
            recovered += 1;
            checksum ^= vk.to_encoded_point(false).as_bytes()[1];
        }
    }
    (recovered, checksum)
}
