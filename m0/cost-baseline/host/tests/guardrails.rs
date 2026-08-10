//! Machine-checked guardrails for the baseline cost figures.
//!
//! The companion to `m0/lez-probe/host/tests/guardrails.rs`, which pins the same
//! workload measured through LEZ. This file pins the bare RISC Zero numbers the
//! README publishes, for the same reason and by the same method: cycle counts are
//! a deterministic function of the guest ELF and its input, reproduced
//! byte-identically across x86_64 and ARM64, so pinning them exactly means any
//! toolchain bump or dependency change fails here and forces the published
//! figures to be re-tagged deliberately. A failure is a prompt to re-measure, not
//! necessarily a defect.
//!
//! # Why these assertions
//!
//! Each of the three configurations is defined by a `[patch.crates-io]` section
//! in its guest workspace, and such a section fails **silently**: a crate bump, a
//! moved tag, or a dropped section restores a different cost profile with a green
//! build. The configurations are therefore pinned both absolutely and against
//! each other, because the cross-checks are what actually catch a patch section
//! going missing:
//!
//! - `mixed` must match `software` on keccak256 and `accelerated` on recovery.
//!   That is the whole claim the configuration makes, and it is the one the
//!   recommendation rests on.
//! - `accelerated` and `mixed` must agree exactly on the recover-only update,
//!   which hashes nothing. If they diverge, something other than the keccak
//!   implementation differs between the two workspaces.
//!
//! Cycles cannot see the keccak coprocessor, which is precisely why gaining it
//! *lowers* cycles. Exact equality catches that, and the ignored proof-size test
//! makes it unmistakable in the dimension where it hurts.
//!
//! ```sh
//! cargo test --release -p kanon-cost-baseline               # cycles
//! cargo test --release -p kanon-cost-baseline -- --ignored  # proving
//! ```

use kanon_cost_baseline::{
    accelerated, execute_with, keccak_delta, mixed, recover_delta, software, verify_run, Opts,
    KECCAK_ITERS, VERIFY_SIGNERS,
};

/// Measured on risc0-zkvm 3.0.6, guest rustc 1.97.0, r0vm 3.0.5 and 3.0.6
/// (identical under both). Update together with the README and `m0/versions.md`.
mod expected {
    /// The empty guest. Identical on all three paths.
    pub const FLOOR: u64 = 2_940;

    /// Mean cycles per hash over `KECCAK_ITERS` hashes, gross of loop
    /// bookkeeping, at 32, 77, 141 and 512 message bytes. 77 is the RedStone
    /// size. The step between 77 and 141 is the 136-byte block boundary.
    pub const KECCAK_SOFTWARE: [(u32, u64); 4] =
        [(32, 17_277), (77, 17_592), (141, 34_510), (512, 70_037)];
    pub const KECCAK_ACCELERATED: [(u32, u64); 4] =
        [(32, 2_212), (77, 2_527), (141, 4_071), (512, 8_850)];

    /// The loop guards that stop the compiler hoisting or deleting the hash.
    /// Flat across message sizes, as expected for a fixed 32-byte fold.
    pub const KECCAK_BOOKKEEPING: u64 = 94;

    /// Total work cycles for an M-signer recovery check, at 1, 3 and 5 signers.
    pub const RECOVER_SOFTWARE: [(usize, u64); 3] =
        [(1, 11_074_701), (3, 33_228_606), (5, 55_375_167)];
    pub const RECOVER_ACCELERATED: [(usize, u64); 3] =
        [(1, 565_551), (3, 1_686_893), (5, 2_817_096)];

    /// The full 3-of-N update: three packages, each a 77-byte hash then a
    /// recovery.
    pub const VERIFY_SOFTWARE: u64 = 33_347_017;
    pub const VERIFY_ACCELERATED: u64 = 1_769_222;
    pub const VERIFY_MIXED: u64 = 1_813_434;

    /// The same update with hashing switched off, which is why the accelerated
    /// and mixed configurations must agree on it exactly.
    pub const VERIFY_RECOVER_ONLY: u64 = 1_761_478;

    /// Segment count, which is what quantises proof size.
    pub const SEGMENTS_SOFTWARE: usize = 34;
    pub const SEGMENTS_ACCELERATED: usize = 2;

    /// Proof sizes, the dimension in which the keccak coprocessor is
    /// unmistakable: 223 KB larger for 2.5% fewer cycles.
    pub const FLOOR_PROOF: usize = 209_570;
    pub const VERIFY_ACCELERATED_PROOF: usize = 785_897;
    pub const VERIFY_MIXED_PROOF: usize = 562_764;
}

/// Mean cycles per hash, rounded as the README reports it.
fn keccak_per_hash(elf: &[u8], msg_len: u32) -> u64 {
    let d = keccak_delta(elf, KECCAK_ITERS, msg_len, Opts::execute_only()).expect("execution");
    (d.work as f64 / f64::from(KECCAK_ITERS)).round() as u64
}

fn keccak_bookkeeping_per_iter(elf: &[u8], msg_len: u32) -> u64 {
    let d = keccak_delta(elf, KECCAK_ITERS, msg_len, Opts::execute_only()).expect("execution");
    (d.work as f64 / f64::from(KECCAK_ITERS)).round() as u64
}

fn recover_cycles(elf: &[u8], signers: usize) -> u64 {
    recover_delta(elf, signers, Opts::execute_only())
        .expect("execution")
        .work
}

/// Every other figure is relative to this one, so it is pinned first.
#[test]
fn zkvm_floor_is_unchanged() {
    for v in [software(), accelerated(), mixed()] {
        let run = execute_with(v.noop, &(), Opts::execute_only()).expect("execution");
        assert_eq!(
            run.cycles,
            expected::FLOOR,
            "zkVM floor moved on the {} path; re-measure and update the README",
            v.name
        );
    }
}

#[test]
fn keccak_cycles_per_hash_are_unchanged() {
    for (len, want) in expected::KECCAK_SOFTWARE {
        assert_eq!(
            keccak_per_hash(software().keccak, len),
            want,
            "software, {len} bytes"
        );
    }
    for (len, want) in expected::KECCAK_ACCELERATED {
        assert_eq!(
            keccak_per_hash(accelerated().keccak, len),
            want,
            "accelerated, {len} bytes"
        );
    }
}

/// The bookkeeping is 0.5% of a software hash but 4.3% of an accelerated one, so
/// the net figures depend on it and it is pinned rather than assumed negligible.
#[test]
fn keccak_loop_bookkeeping_is_flat_and_unchanged() {
    for v in [software(), accelerated(), mixed()] {
        for (len, _) in expected::KECCAK_SOFTWARE {
            assert_eq!(
                keccak_bookkeeping_per_iter(v.keccak_overhead, len),
                expected::KECCAK_BOOKKEEPING,
                "{} path, {len} bytes",
                v.name
            );
        }
    }
}

#[test]
fn recovery_cycles_are_unchanged() {
    for (n, want) in expected::RECOVER_SOFTWARE {
        assert_eq!(
            recover_cycles(software().recover, n),
            want,
            "software, {n} signers"
        );
    }
    for (n, want) in expected::RECOVER_ACCELERATED {
        assert_eq!(
            recover_cycles(accelerated().recover, n),
            want,
            "accelerated, {n} signers"
        );
    }
}

/// The absence of amortisation is a finding in its own right, and a load-bearing
/// one: it is why widening the threshold is linearly priced with no engineering
/// remedy. A future accelerator that shared work between recoveries would break
/// this, and that should be noticed rather than silently improve the numbers.
#[test]
fn per_recovery_cost_does_not_amortise() {
    let one = recover_cycles(accelerated().recover, 1) as f64;
    for (n, _) in expected::RECOVER_ACCELERATED {
        let per = recover_cycles(accelerated().recover, n) as f64 / n as f64;
        let drift = 100.0 * (per - one).abs() / one;
        assert!(
            drift < 1.0,
            "per-recovery cost drifted {drift:.2}% at {n} signers; recoveries may now share work"
        );
    }
}

/// The headline figure, and the two comparison arms it is chosen against.
#[test]
fn full_update_cycles_are_unchanged() {
    let run = |v: kanon_cost_baseline::Elfs, hash: bool| {
        verify_run(v.verify, VERIFY_SIGNERS, hash, Opts::execute_only()).expect("execution")
    };

    let m = run(mixed(), true);
    assert_eq!(m.cycles, expected::VERIFY_MIXED);
    assert_eq!(m.segments, expected::SEGMENTS_ACCELERATED);

    let a = run(accelerated(), true);
    assert_eq!(a.cycles, expected::VERIFY_ACCELERATED);
    assert_eq!(a.segments, expected::SEGMENTS_ACCELERATED);

    let s = run(software(), true);
    assert_eq!(s.cycles, expected::VERIFY_SOFTWARE);
    assert_eq!(s.segments, expected::SEGMENTS_SOFTWARE);

    // Hashing nothing, so the keccak implementation cannot matter. Divergence
    // here means the two workspaces differ in something else.
    assert_eq!(
        run(mixed(), false).cycles,
        expected::VERIFY_RECOVER_ONLY,
        "mixed, recover only"
    );
    assert_eq!(
        run(accelerated(), false).cycles,
        expected::VERIFY_RECOVER_ONLY,
        "accelerated, recover only"
    );
}

/// What the mixed configuration *is*: software keccak, accelerated recovery.
/// Pinning it against the other two catches a `[patch.crates-io]` section going
/// missing on either side, which absolute values alone would only catch as an
/// unexplained number change.
#[test]
fn mixed_configuration_is_the_combination_it_claims() {
    for (len, _) in expected::KECCAK_SOFTWARE {
        assert_eq!(
            keccak_per_hash(mixed().keccak, len),
            keccak_per_hash(software().keccak, len),
            "mixed keccak256 should be the software implementation, at {len} bytes"
        );
    }
    for (n, _) in expected::RECOVER_ACCELERATED {
        assert_eq!(
            recover_cycles(mixed().recover, n),
            recover_cycles(accelerated().recover, n),
            "mixed recovery should be the accelerated implementation, at {n} signers"
        );
    }
}

/// Proving takes minutes per sample, so this is ignored by default. It pins the
/// dimension cycles cannot see: the keccak coprocessor attaches a separate proof,
/// which is 223 KB and the reason the accelerator is declined.
#[test]
#[ignore = "proving takes minutes; run with --ignored"]
fn proof_sizes_distinguish_the_configurations() {
    let prove = Opts {
        prove: true,
        repeat: 1,
        ..Opts::default()
    };

    let floor = execute_with(mixed().noop, &(), prove).expect("proving");
    assert_eq!(floor.proof_bytes, Some(expected::FLOOR_PROOF));

    let m = verify_run(mixed().verify, VERIFY_SIGNERS, true, prove).expect("proving");
    assert_eq!(m.proof_bytes, Some(expected::VERIFY_MIXED_PROOF));

    let a = verify_run(accelerated().verify, VERIFY_SIGNERS, true, prove).expect("proving");
    assert_eq!(a.proof_bytes, Some(expected::VERIFY_ACCELERATED_PROOF));

    assert!(
        a.proof_bytes > m.proof_bytes,
        "the keccak coprocessor should make the proof larger, not smaller"
    );
}
