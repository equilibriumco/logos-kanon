//! Measurement core for the LEZ cost probe, ported to LEZ v0.3.
//!
//! What changed from the v0.2 probe this replaces:
//!
//! - **A program is two kinds of run, not one.** v0.3 executes a `plan`, then
//!   one `apply` per shard the plan named. Both are metered, so what an
//!   operator pays is the sum, and this measures each separately.
//! - **Cycles are gas, one for one**, against a declared limit of at most
//!   `MAX_GAS_EXEC` = 10,000,000 — which is also a per-block cap. v0.2's
//!   `MAX_NUM_CYCLES_PUBLIC_EXECUTION` was 33,554,432 and bought nothing.
//! - **The call frames are borsh**, not risc0 serde words: a length-prefixed
//!   `CallKind`, then a length-prefixed `PlanInput` or `ApplyInput`. The
//!   instruction rides inside `PlanInput.instruction_data` as borsh bytes.
//!
//! The workload is unchanged: the same packages, the same keys, the same
//! keccak-then-recover per signer, so the figures are comparable to the
//! published M0 ones.

use std::time::Instant;

use anyhow::{ensure, Result};
use borsh::BorshSerialize;
use k256::ecdsa::SigningKey;
use lee_core::account::{AccountId, ProgramShardSelector, ShardData};
use lee_core::program::{AccountMeta, ApplyInput, CallKind, PlanInput, PlanOutput};
use lee_core::to_borsh_frame;
use risc0_zkvm::{default_executor, default_prover, ExecutorEnv};
use tiny_keccak::{Hasher, Keccak};

/// LEZ v0.3's cap on the action phase of a transaction, from
/// `lez::programs::fee::core::market::MAX_GAS_EXEC` (`2 * TARGET_GAS_EXEC`).
/// Gas is cycles one for one, so this is directly comparable to a cycle count.
pub const LEZ_GAS_LIMIT: u64 = 10_000_000;

/// What v0.2 allowed a public execution, for comparison only.
pub const V02_CYCLE_BUDGET: u64 = 1024 * 1024 * 32;

/// The signed byte range of one RedStone data package with a single data point
/// and a 32-byte value: `1 * (32 + 32) + 4 + 6 + 3`.
pub const REDSTONE_SIGNED_BYTES: usize = 77;

/// `(signed_bytes, signature, recovery_id)` per signer, matching the guest.
pub type Package = (Vec<u8>, Vec<u8>, u8);

/// The program's own account, and the one account it is given.
const PROGRAM: AccountId = AccountId::new([7u8; 32]);
const ACCOUNT: AccountId = AccountId::new([9u8; 32]);

/// A guest configuration. The two differ only in whether keccak256 is routed to
/// RISC Zero's coprocessor; secp256k1 is accelerated in both.
pub struct Variant {
    pub name: &'static str,
    pub noop: &'static [u8],
    pub plumbing: &'static [u8],
    pub verify: &'static [u8],
}

/// The recommended configuration: secp256k1 accelerated, keccak256 in software.
#[must_use]
pub fn mixed() -> Variant {
    Variant {
        name: "mixed",
        noop: lez_probe_methods_mixed::LEZ_NOOP_ELF,
        plumbing: lez_probe_methods_mixed::LEZ_PLUMBING_ELF,
        verify: lez_probe_methods_mixed::LEZ_VERIFY_ELF,
    }
}

/// Both accelerators, including the keccak256 coprocessor.
#[must_use]
pub fn accelerated() -> Variant {
    Variant {
        name: "accelerated",
        noop: lez_probe_methods_accelerated::LEZ_NOOP_ELF,
        plumbing: lez_probe_methods_accelerated::LEZ_PLUMBING_ELF,
        verify: lez_probe_methods_accelerated::LEZ_VERIFY_ELF,
    }
}

/// What one program run cost, phase by phase.
pub struct Run {
    /// The `plan` run: instruction decode, the application logic, the effects.
    pub plan: u64,
    /// One figure per shard the plan named, in the order it named them.
    pub applies: Vec<u64>,
    /// Every proving sample of the plan, in seconds.
    pub prove_samples: Vec<f64>,
    pub proof_bytes: Option<usize>,
}

impl Run {
    /// What the transaction is billed: every metered run, summed.
    #[must_use]
    pub fn cycles(&self) -> u64 {
        self.plan + self.applies.iter().sum::<u64>()
    }

    /// The fastest proving sample: contention only ever adds time.
    #[must_use]
    pub fn prove_secs(&self) -> Option<f64> {
        self.prove_samples
            .iter()
            .copied()
            .fold(None, |acc: Option<f64>, x| {
                Some(acc.map_or(x, |a| a.min(x)))
            })
    }

    #[must_use]
    pub fn prove_spread_pct(&self) -> Option<f64> {
        let min = self.prove_secs()?;
        let max = self.prove_samples.iter().copied().fold(f64::MIN, f64::max);
        (self.prove_samples.len() > 1).then(|| 100.0 * (max - min) / min)
    }

    /// Share of the gas a transaction may declare, as a percentage.
    #[must_use]
    pub fn gas_pct(&self) -> f64 {
        100.0 * self.cycles() as f64 / LEZ_GAS_LIMIT as f64
    }

    /// Share of what v0.2 allowed, so the two generations can be compared on
    /// the same denominator as the published M0 figures.
    #[must_use]
    pub fn v02_budget_pct(&self) -> f64 {
        100.0 * self.cycles() as f64 / V02_CYCLE_BUDGET as f64
    }
}

/// The one account the program is given: its own shard, so the plan may write it.
fn account_meta() -> AccountMeta {
    AccountMeta {
        account_id: ACCOUNT,
        is_authorized: false,
        program_account_id: PROGRAM,
    }
}

/// One guest run, in the frames LEZ writes: the call kind, then its input.
fn run(elf: &[u8], kind: CallKind, input: &impl BorshSerialize, limit: bool) -> Result<u64> {
    let mut bytes = to_borsh_frame(&kind);
    bytes.extend(to_borsh_frame(input));
    let mut builder = ExecutorEnv::builder();
    if limit {
        builder.session_limit(Some(LEZ_GAS_LIMIT));
    }
    let env = builder.write_slice(&bytes).build()?;
    Ok(default_executor().execute(env, elf)?.cycles())
}

/// Executes, and optionally proves, a LEZ v0.3 program over `instruction`.
///
/// Runs the plan, reads back the effects it committed, and runs one apply per
/// effect — which is what the chain does, and what it bills.
pub fn measure<T: BorshSerialize>(
    elf: &[u8],
    instruction: &T,
    prove: bool,
    repeat: usize,
) -> Result<Run> {
    let input = PlanInput {
        self_account_id: PROGRAM,
        caller_account_id: None,
        accounts: vec![account_meta()],
        instruction_data: borsh::to_vec(instruction)?,
    };

    let plan_cycles = run(elf, CallKind::Plan, &input, true)?;

    // Re-run the plan to read its effects back. Executing twice costs wall-clock
    // and nothing else: a run is deterministic, so the cycle figure above is the
    // figure for this journal too.
    let mut frame = to_borsh_frame(&CallKind::Plan);
    frame.extend(to_borsh_frame(&input));
    let env = ExecutorEnv::builder().write_slice(&frame).build()?;
    let session = default_executor().execute(env, elf)?;
    let output: lee_core::program::GuestOutput =
        borsh::from_slice(&session.journal.bytes[4..]).expect("a plan commits a GuestOutput");
    let plan_output: PlanOutput = match output {
        lee_core::program::GuestOutput::Plan(p) => p,
        lee_core::program::GuestOutput::Apply(_) => anyhow::bail!("a plan call returned an apply"),
    };

    let mut applies = Vec::new();
    for effect in &plan_output.effects {
        ensure!(
            effect.selector == ProgramShardSelector::from(&account_meta()),
            "an effect names an account the plan was not given"
        );
        applies.push(run(
            elf,
            CallKind::Apply,
            &ApplyInput {
                self_account_id: PROGRAM,
                selector: effect.selector.clone(),
                pre_data: ShardData::try_from(Vec::new()).expect("an empty shard fits"),
                effect_data: effect.data.clone(),
            },
            true,
        )?);
    }

    let mut out = Run {
        plan: plan_cycles,
        applies,
        prove_samples: Vec::new(),
        proof_bytes: None,
    };
    ensure!(
        out.cycles() < LEZ_GAS_LIMIT,
        "execution exceeded the gas a transaction may declare"
    );

    for _ in 0..if prove { repeat.max(1) } else { 0 } {
        let mut bytes = to_borsh_frame(&CallKind::Plan);
        bytes.extend(to_borsh_frame(&input));
        let env = ExecutorEnv::builder().write_slice(&bytes).build()?;
        let started = Instant::now();
        let info = default_prover().prove(env, elf)?;
        out.prove_samples.push(started.elapsed().as_secs_f64());
        // borsh over `receipt.inner`, matching the v0.2 probe, so proof sizes
        // are comparable within this harness across the two generations.
        out.proof_bytes = Some(borsh::to_vec(&info.receipt.inner)?.len());
    }
    Ok(out)
}

/// Proves the plan the way LEZ's private path proves its **second** stage, with
/// `ProverOpts::succinct()`, and reports `(seconds, proof bytes)`.
///
/// Why this exists, and what it settles. Every proof figure published for M0 —
/// and the first v0.3 re-run — is a **composite** receipt: one segment proof per
/// execution segment, so its size grows with the program. LEZ's private path
/// does not put one of those on the wire. It proves the program composite in
/// stage one, then proves the privacy circuit over the result with
/// `ProverOpts::succinct()` (`privacy_preserving_transaction/circuit/mod.rs`),
/// and a succinct receipt is constant-size by construction.
///
/// That distinction is load-bearing rather than pedantic, because v0.3 charges
/// a private transaction `PRIVATE_GAS_STOR` for its bytes. If proof size is ever
/// priced for real, what matters is the succinct size and whether the guest's
/// configuration moves it — not the composite size, which nobody pays for.
///
/// This is not the privacy circuit, which is LEZ's and needs its whole stage-one
/// output to run. It is the same program proved succinct, which is what isolates
/// the question actually being asked: *does the keccak coprocessor change the
/// size of a recursively compressed proof, or only of an uncompressed one?*
pub fn prove_succinct<T: BorshSerialize>(elf: &[u8], instruction: &T) -> Result<(f64, usize)> {
    let input = PlanInput {
        self_account_id: PROGRAM,
        caller_account_id: None,
        accounts: vec![account_meta()],
        instruction_data: borsh::to_vec(instruction)?,
    };
    let mut bytes = to_borsh_frame(&CallKind::Plan);
    bytes.extend(to_borsh_frame(&input));
    let env = ExecutorEnv::builder().write_slice(&bytes).build()?;

    let started = Instant::now();
    let info = default_prover().prove_with_opts(
        env,
        elf,
        &risc0_zkvm::ProverOpts::succinct(),
    )?;
    let secs = started.elapsed().as_secs_f64();
    Ok((secs, borsh::to_vec(&info.receipt.inner)?.len()))
}

/// How much of a recovery a native `ecrecover` could actually remove.
///
/// Returns `(lincomb, recover, rounds)` in cycles, each already net of the zkVM
/// floor, over `rounds` iterations.
///
/// # Why this measurement exists
///
/// Section 3 of the addendum estimates what a recovery precompile would save by
/// assuming recovery collapses to the cost of one syscall. That assumption is
/// only safe if the curve arithmetic is *not* already accelerated — and it is.
/// `risc0-bigint2` ships dedicated circuits for secp256k1 point addition and
/// doubling, with scalar multiplication built on them, and the patched `k256`
/// this build pins calls into them unconditionally in a guest.
///
/// So a recovery is already part-circuit, and the question is what fraction. The
/// curve arithmetic — `u1·G + u2·R` — is work a precompile still has to do and
/// so cannot remove. Everything else is removable. `ec_lincomb` runs only the
/// former; `ec_recover` runs the whole operation over the same inputs and the
/// same iteration count. The difference is the ceiling on what a precompile
/// could buy.
pub fn ec_breakdown() -> Result<(u64, u64, u64, u64, u64, u32)> {
    const ROUNDS: u32 = 4;

    let floor = bare_cycles(lez_probe_methods_mixed::EC_FLOOR_ELF, &())?;

    let raw = bare_cycles(lez_probe_methods_mixed::EC_RAW_ELF, &())?;
    let calls = bare_cycles(lez_probe_methods_mixed::EC_CALLS_ELF, &())?;
    // 256 doublings in that guest; report the per-call cost alongside.
    let per_call = calls.saturating_sub(floor) / 256;
    let lincomb = bare_cycles(lez_probe_methods_mixed::EC_LINCOMB_ELF, &())?;

    // One real signature, as `make_packages` builds them.
    let (msg, sig_bytes, recid) = make_packages(1)?.remove(0);
    let mut hasher = Keccak::v256();
    hasher.update(&msg);
    let mut digest = [0u8; 32];
    hasher.finalize(&mut digest);
    let recover = bare_cycles(
        lez_probe_methods_mixed::EC_RECOVER_ELF,
        &(digest, sig_bytes.clone(), recid),
    )?;
    let r0crypto = bare_cycles(
        lez_probe_methods_mixed::EC_R0CRYPTO_ELF,
        &(digest, sig_bytes, recid),
    )?;

    Ok((
        per_call,
        raw.saturating_sub(floor),
        lincomb.saturating_sub(floor),
        recover.saturating_sub(floor),
        r0crypto.saturating_sub(floor),
        ROUNDS,
    ))
}

/// Executes a guest with no LEZ framing, writing `input` with risc0 serde.
fn bare_cycles<T: serde::Serialize>(elf: &[u8], input: &T) -> Result<u64> {
    let env = ExecutorEnv::builder().write(input)?.build()?;
    Ok(default_executor().execute(env, elf)?.cycles())
}

/// One data package per signer, each over its own signed bytes, as RedStone
/// publishes an M-of-N update. Keys are deterministic so runs reproduce.
///
/// Byte-for-byte what the v0.2 probe built, so the two generations measure the
/// same workload.
pub fn make_packages(n: usize) -> Result<Vec<Package>> {
    make_packages_spanned(n, REDSTONE_SIGNED_BYTES)
}

/// A multi-point package's signed span: 9 data points of 64 bytes (feed id,
/// value) plus the three trailing fields — the shape of RedStone's live
/// `___FAST___` package on `redstone-primary-prod`, which carries BTC, ETH and
/// SOL (with six other feeds) under one signature per signer. Confirmed against
/// captured production signatures by `verifier-core/tests/multipoint_fast.rs`
/// on the optimization branch.
pub const FAST_SIGNED_BYTES: usize = 9 * 64 + 13;

/// `make_packages` at an arbitrary signed-span length. The guests loop over
/// whatever spans they are handed, so a multi-point package differs from a
/// single-point one only in the bytes keccak walks — recovery is per package
/// either way.
pub fn make_packages_spanned(n: usize, span: usize) -> Result<Vec<Package>> {
    (0..n)
        .map(|i| {
            let mut msg = vec![0x5au8; span];
            msg[32] = i as u8;
            msg[span - 1] = (i as u8).wrapping_mul(7);

            let mut hasher = Keccak::v256();
            hasher.update(&msg);
            let mut digest = [0u8; 32];
            hasher.finalize(&mut digest);

            let mut scalar = [0u8; 32];
            scalar[31] = (i + 1) as u8;
            let key = SigningKey::from_bytes(&scalar.into())?;
            let (sig, recid) = key.sign_prehash_recoverable(&digest)?;
            Ok((msg, sig.to_bytes().to_vec(), recid.to_byte()))
        })
        .collect()
}
