//! Machine-checked guardrails for the v0.3 figures, in the same style as
//! `m0/lez-probe/host/tests/guardrails.rs` pins the v0.2.1 ones.
//!
//! # Exact equality is intentional
//!
//! Cycle counts are a deterministic function of the guest ELF and its input.
//! Pinning them exactly means any toolchain bump, dependency change or code change
//! fails here and forces the published figures to be re-tagged deliberately. A
//! failure is not necessarily a defect; it is a prompt to re-measure and update the
//! constants and `FINDINGS.md` together.
//!
//! # What this suite adds over the v0.2 one
//!
//! Two assertions have no counterpart there, because v0.2 could not express them:
//!
//! - `the_cryptography_did_not_move_between_lez_generations` is the result the
//!   whole re-run turns on. If the crypto had moved, every M0 recommendation would
//!   have needed re-deciding rather than re-confirming.
//! - `a_submission_is_a_material_share_of_a_whole_block` pins the block-level
//!   constraint v0.3 introduced, which M0 never asked about.

use lez_probe::{accelerated, make_packages, measure, mixed, LEZ_GAS_LIMIT};

/// Measured on risc0-zkvm 3.0.6, r0vm 3.0.5, guest rustc 1.97.0, against
/// logos-execution-zone v0.3.0 (`db66590a`). Update together with `FINDINGS.md`.
mod expected {
    /// `plan` + `apply` for a program that decodes the call, names one shard and
    /// writes it back. v0.2's equivalent floor was 39,103.
    pub const FRAMEWORK_FLOOR: u64 = 14_038;
    pub const FRAMEWORK_FLOOR_PLAN: u64 = 8_057;
    pub const FRAMEWORK_FLOOR_APPLY: u64 = 5_981;

    pub const MIXED_1_SIGNER: u64 = 600_881;
    pub const MIXED_3_SIGNERS: u64 = 1_766_937;
    pub const MIXED_5_SIGNERS: u64 = 2_933_979;
    pub const ACCELERATED_3_SIGNERS: u64 = 1_722_716;

    /// The framework's own cost with the real instruction to carry but no
    /// cryptography, at 1, 3 and 5 signers. v0.2's were (78,972), (158,290),
    /// (237,608) -- borsh instruction bytes replaced risc0 serde words, which
    /// spend four bytes per byte, and the handling cost fell about 27x with them.
    pub const MIXED_PLUMBING: [(usize, u64); 3] = [(1, 15_730), (3, 18_480), (5, 21_236)];

    /// What `lez_verify` costs above `lez_plumbing`: the cryptography alone, with
    /// the framework's per-package handling subtracted rather than folded in.
    /// These are the v0.2 figures, and they must still hold.
    pub const CRYPTO_ALONE_V02: [(usize, u64); 3] =
        [(1, 585_145), (3, 1_748_447), (5, 2_912_729)];

    /// Proof sizes, borsh over `receipt.inner`, three signers. v0.2's were
    /// 564,637 and 787,749 -- both fell by exactly 1,350, so the coprocessor's
    /// 223,112-byte penalty is identical across the two LEZ generations.
    pub const MIXED_3_SIGNERS_PROOF: usize = 563_287;
    pub const ACCELERATED_3_SIGNERS_PROOF: usize = 786_399;

    /// The same program proved with `ProverOpts::succinct()`, which is what
    /// LEZ's private path puts on the wire. The *same* in both configurations,
    /// which is the point. Close to LEZ's own provisional `PROOF_BYTES` of
    /// 223,551, which is corroboration that the constant was measured from a
    /// real succinct receipt rather than guessed.
    pub const SUCCINCT_PROOF: usize = 223_789;

    /// A recovery, split three ways, per recovery. The first is curve
    /// arithmetic in a `risc0-bigint2` circuit and is the floor a precompile
    /// cannot go below; the gaps above it are what one could remove.
    pub const EC_RAW: u64 = 164_447;
    /// One `sys_bigint2_*` round trip. Two full-width scalar multiplications
    /// make roughly 320 of them, which is why `EC_RAW` is mostly overhead
    /// rather than arithmetic.
    pub const EC_PER_CALL: u64 = 256;

    /// The whole LEZ program with recovery through `risc0-crypto-evm` instead
    /// of `k256`, at 1, 3 and 5 signers. Against MIXED_* above, that is 3.52x,
    /// 3.67x and 3.70x.
    pub const R0CRYPTO: [(usize, u64); 3] = [(1, 170_757), (3, 481_474), (5, 792_176)];
    pub const EC_THROUGH_K256: u64 = 274_289;
    pub const EC_FULL_RECOVERY: u64 = 570_683;
}

fn gas_of(elf: &[u8], signers: usize) -> u64 {
    let packages = make_packages(signers).expect("building packages");
    measure(elf, &packages, false, 1).expect("execution").cycles()
}

/// v0.3 splits the floor across two metered runs, so both halves are pinned: a
/// single total would hide work moving between the phases.
#[test]
fn framework_floor_is_unchanged() {
    let run = measure(mixed().noop, &Vec::<u8>::new(), false, 1).expect("execution");
    assert_eq!(run.plan, expected::FRAMEWORK_FLOOR_PLAN, "plan");
    assert_eq!(
        run.applies,
        vec![expected::FRAMEWORK_FLOOR_APPLY],
        "one apply, for the one shard the plan names"
    );
    assert_eq!(run.cycles(), expected::FRAMEWORK_FLOOR, "total");
}

/// The framework's per-package instruction handling, which the floor cannot see.
///
/// It is what separates the cryptography from the framework in the published
/// decomposition, so it is pinned in its own right: if it moved and only the
/// totals were pinned, the movement would be silently reattributed to recovery.
#[test]
fn instruction_handling_cost_is_unchanged() {
    for (n, want) in expected::MIXED_PLUMBING {
        assert_eq!(gas_of(mixed().plumbing, n), want, "mixed, {n} signers");
        assert_eq!(
            gas_of(accelerated().plumbing, n),
            want,
            "accelerated, {n} signers; this program hashes nothing, so the two \
             configurations cannot differ here"
        );
    }
}

#[test]
fn mixed_configuration_gas_is_unchanged() {
    let v = mixed();
    assert_eq!(gas_of(v.verify, 1), expected::MIXED_1_SIGNER);
    assert_eq!(gas_of(v.verify, 3), expected::MIXED_3_SIGNERS);
    assert_eq!(gas_of(v.verify, 5), expected::MIXED_5_SIGNERS);
}

#[test]
fn accelerated_configuration_gas_is_unchanged() {
    assert_eq!(
        gas_of(accelerated().verify, 3),
        expected::ACCELERATED_3_SIGNERS
    );
}

/// **The result the re-run turns on.**
///
/// v0.3 rewrote the program model around the verification, not the verification.
/// Subtracting `lez_plumbing` from `lez_verify` leaves the cryptography alone, and
/// it must still equal what M0 measured under v0.2.1 -- within a handful of cycles
/// of inlining difference, since the surrounding code is not the same code.
///
/// If this ever fails, the comparison in `FINDINGS.md` is void: the two
/// generations would no longer be measuring one workload, and every M0
/// recommendation would need re-deciding rather than re-confirming.
#[test]
fn the_cryptography_did_not_move_between_lez_generations() {
    for (n, v02) in expected::CRYPTO_ALONE_V02 {
        let v03 = gas_of(mixed().verify, n) - gas_of(mixed().plumbing, n);
        let drift = v03.abs_diff(v02);
        assert!(
            drift <= 32,
            "the cryptography moved at {n} signers: v0.2 {v02}, v0.3 {v03}, \
             a difference of {drift}"
        );
    }
}

/// **How much of a recovery a precompile could actually remove.**
///
/// M0 estimated a recovery precompile at 8x–9x, and an early draft of this
/// addendum at 18x–24x. Both assumed a recovery collapses to the cost of one
/// syscall. It cannot: `risc0-bigint2` already runs the curve arithmetic in
/// dedicated circuits, which this build already uses, and that work has to
/// happen however it is packaged.
///
/// Three guests over the same inputs isolate the three layers. Pinned because
/// the published 5x figure is computed from the ratio between them, and because
/// an unnoticed move here would silently change what we are asking Logos for.
#[test]
fn the_irreducible_share_of_a_recovery_is_unchanged() {
    let (per_call, raw, lincomb, recover, _r0crypto, rounds) =
        lez_probe::ec_breakdown().expect("the EC probes run");
    assert_eq!(per_call, expected::EC_PER_CALL, "one accelerator round trip");
    let per = |x: u64| x / u64::from(rounds);

    assert_eq!(per(raw), expected::EC_RAW, "bigint2 circuits alone");
    assert_eq!(per(lincomb), expected::EC_THROUGH_K256, "through k256");
    assert_eq!(per(recover), expected::EC_FULL_RECOVERY, "full recovery");

    // The claim the addendum publishes, as a bound rather than a figure.
    let share = 100.0 * raw as f64 / recover as f64;
    assert!(
        (20.0..40.0).contains(&share),
        "the irreducible share of a recovery is {share:.1}%, outside the band section 3 \
         reasons from; the 3x figure needs recomputing"
    );
}

/// **What the primitive M0 asked for is actually worth.**
///
/// `risc0-crypto-evm` ships `secp256k1_ecrecover`, backed by the same
/// accelerator `k256` already uses. This pins the whole LEZ program with
/// recovery routed through it — not the primitive in isolation, and not an
/// arithmetic substitution, but the same `plan`/`apply` program with one call
/// changed.
///
/// Pinned because section 3 of the addendum publishes the ratio, and because a
/// move here changes a recommendation rather than a figure.
#[test]
fn the_r0crypto_program_is_unchanged() {
    for (n, want) in expected::R0CRYPTO {
        let packages = make_packages(n).expect("building packages");
        let run = measure(
            lez_probe_methods_mixed::LEZ_VERIFY_R0CRYPTO_ELF,
            &packages,
            false,
            1,
        )
        .expect("execution");
        assert_eq!(run.cycles(), want, "{n} signers");
    }
}

/// Cost must stay linear in the signer count: nothing is shared between signers,
/// so a departure from linearity means the workload is no longer the one measured.
#[test]
fn per_signer_cost_is_linear() {
    let v = mixed();
    let one = gas_of(v.verify, 1);
    let three = gas_of(v.verify, 3);
    let five = gas_of(v.verify, 5);

    let first = (three - one) / 2;
    let second = (five - three) / 2;
    let drift = (first as f64 - second as f64).abs() / first as f64;
    assert!(
        drift < 0.01,
        "per-signer cost is not linear: {first} then {second} gas"
    );
}

/// The semantic guard M0 carried, restated against the limit that now applies.
///
/// Without the secp256k1 accelerator a 3-of-N update is 33,347,017 cycles. Under
/// v0.2's 33,554,432 that was 99.4% of the budget and did not fit; under v0.3's
/// 10,000,000 it is 333% of it. The accelerator decides feasibility by a wider
/// margin than it used to, so this bound is correspondingly tighter.
#[test]
fn a_3_of_n_update_still_fits_one_transaction_with_room() {
    let run = measure(mixed().verify, &make_packages(3).unwrap(), false, 1).expect("execution");
    let pct = run.gas_pct();
    assert!(
        pct < 25.0,
        "3-of-N uses {pct:.2}% of the {LEZ_GAS_LIMIT} gas a transaction may \
         declare; the secp256k1 accelerator has probably been lost"
    );
}

/// **New at v0.3, and absent from M0.**
///
/// `MAX_GAS_EXEC` is not only the most a transaction may declare: `accumulate_exec_gas`
/// sums each transaction's charged cycles across a block and rejects the block past
/// the same 10,000,000 (`lez/chain_state/src/apply.rs`). So a submission's share of
/// a *block* is a real constraint, chain-wide and shared with every other program.
///
/// This pins the shape of that constraint rather than a figure: one five-signer
/// submission is a material fraction of a whole block, and the five production
/// feeds cannot all update within one.
#[test]
fn a_submission_is_a_material_share_of_a_whole_block() {
    let five = measure(mixed().verify, &make_packages(5).unwrap(), false, 1)
        .expect("execution")
        .cycles();

    assert!(
        five > LEZ_GAS_LIMIT / 4,
        "a five-signer submission is {five} gas, under a quarter of a block's \
         {LEZ_GAS_LIMIT}; the block-contention finding in FINDINGS.md section 6 \
         no longer holds and should be restated"
    );
    assert!(
        five * 5 > LEZ_GAS_LIMIT,
        "the five production feeds now fit one block at five signers each \
         ({} gas against {LEZ_GAS_LIMIT}); FINDINGS.md section 6 should be restated",
        five * 5
    );
}

/// Proof size is where the keccak coprocessor is undeniable: it adds roughly
/// 223 KB while *reducing* cycles, so cycles alone cannot catch it.
///
/// Ignored by default because proving a 1.77M-cycle program takes minutes. Run
/// explicitly:
///
/// ```sh
/// cargo test --release -- --ignored
/// ```
#[test]
#[ignore = "proving takes minutes; run with --ignored"]
fn proof_sizes_distinguish_the_configurations() {
    let packages = make_packages(3).unwrap();

    let m = measure(mixed().verify, &packages, true, 1).expect("proving");
    assert_eq!(
        m.proof_bytes,
        Some(expected::MIXED_3_SIGNERS_PROOF),
        "mixed proof size moved"
    );

    let a = measure(accelerated().verify, &packages, true, 1).expect("proving");
    assert_eq!(
        a.proof_bytes,
        Some(expected::ACCELERATED_3_SIGNERS_PROOF),
        "accelerated proof size moved"
    );

    // 223,112 on v0.2.1 as well, to the byte.
    let penalty = a.proof_bytes.unwrap() - m.proof_bytes.unwrap();
    assert_eq!(
        penalty, 223_112,
        "the keccak coprocessor's proof-size penalty moved; it was identical \
         across v0.2.1 and v0.3.0, which is what FINDINGS.md section 7 reports"
    );
}

/// **The composite penalty above does not survive recursion, and that corrects
/// M0.**
///
/// Every proof figure M0 published, and the one in the test above, is a
/// *composite* receipt — one segment proof per execution segment, so its size
/// grows with the program. LEZ's private path puts no such receipt on the wire:
/// it proves the program composite in stage one, then proves the privacy circuit
/// over the result with `ProverOpts::succinct()`, and a succinct receipt is
/// constant-size by construction.
///
/// Measured here rather than argued: the same program, proved succinct, is the
/// same number of bytes in both configurations. So the keccak coprocessor's
/// 223,112-byte penalty is an artefact of the uncompressed receipt and costs
/// nothing a private transaction would ever be charged for.
///
/// M0 section 5.1 gave two reasons for software keccak256: it saves proving time
/// *and* 223 KB of proof. The second reason does not hold for what the chain
/// carries. The recommendation is unchanged — the time is real, and larger here
/// than M0 measured — but it now rests on one leg rather than two.
///
/// Ignored by default: succinct proving is recursion over a 1.77M-cycle program
/// and takes minutes per configuration.
#[test]
#[ignore = "succinct proving takes minutes; run with --ignored"]
fn the_coprocessor_penalty_does_not_survive_recursion() {
    let packages = make_packages(3).unwrap();

    let (mixed_secs, mixed_bytes) =
        lez_probe::prove_succinct(mixed().verify, &packages).expect("proving");
    let (accel_secs, accel_bytes) =
        lez_probe::prove_succinct(accelerated().verify, &packages).expect("proving");

    assert_eq!(
        mixed_bytes, accel_bytes,
        "a succinct receipt is constant-size, so the two configurations must agree; \
         if they ever do not, the storage half of M0 section 5.1 is live again"
    );
    assert_eq!(
        mixed_bytes, expected::SUCCINCT_PROOF,
        "the succinct proof size moved"
    );

    // What the coprocessor actually costs, with bytes ruled out.
    assert!(
        accel_secs > mixed_secs,
        "the keccak coprocessor proved faster than software keccak ({accel_secs:.2} s \
         against {mixed_secs:.2} s); that inverts the only remaining argument for \
         software keccak and M0 section 5.1 should be revisited"
    );
}
