//! Measurement core for the LEZ cost probe, shared by the CLI and the guardrail
//! tests.
//!
//! See the crate README for what is measured and why. The essentials:
//!
//! - Public execution on LEZ is capped in risc0 cycles at
//!   `MAX_NUM_CYCLES_PUBLIC_EXECUTION` and is **not proven**, so cycles are the
//!   cost, directly comparable to the budget with no conversion.
//! - Private execution proves the same program, with **no session limit**, so
//!   there proving time and proof size are the cost.
//!
//! `Program::execute` upstream is `pub(crate)` and discards cycle counts, so the
//! input-writing order is reproduced here. It must stay in step with
//! `Program::write_inputs`: program id, caller program id, pre-states, instruction
//! words. If it drifts, the guest fails to deserialize rather than silently
//! mismeasuring.

use std::time::Instant;

use anyhow::{ensure, Result};
use k256::ecdsa::SigningKey;
use lee_core::{
    account::{Account, AccountId, AccountWithMetadata},
    program::{InstructionData, ProgramId},
};
use risc0_zkvm::{default_executor, default_prover, ExecutorEnv};
use tiny_keccak::{Hasher, Keccak};

/// LEZ's cap on a public execution, from `MAX_NUM_CYCLES_PUBLIC_EXECUTION`.
pub const LEZ_CYCLE_BUDGET: u64 = 1024 * 1024 * 32;

/// The signed byte range of one RedStone data package with a single data point
/// and a 32-byte value: `1 * (32 + 32) + 4 + 6 + 3`.
pub const REDSTONE_SIGNED_BYTES: usize = 77;

/// `(signed_bytes, signature, recovery_id)` per signer, matching the guest.
pub type Package = (Vec<u8>, Vec<u8>, u8);

/// A guest configuration. The two differ only in whether keccak256 is routed to
/// RISC Zero's coprocessor; secp256k1 is accelerated in both.
pub struct Variant {
    pub name: &'static str,
    pub noop: &'static [u8],
    /// `noop` carrying the same instruction as `verify`, so that the framework's
    /// per-package instruction handling can be separated from the cryptography
    /// rather than folded into it.
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

/// What one program run cost.
pub struct Run {
    pub cycles: u64,
    /// Every proving sample, in seconds.
    pub prove_samples: Vec<f64>,
    pub proof_bytes: Option<usize>,
}

impl Run {
    /// The fastest sample: contention only ever adds time, so the minimum is the
    /// cleanest estimate of the underlying cost.
    #[must_use]
    pub fn prove_secs(&self) -> Option<f64> {
        self.prove_samples
            .iter()
            .copied()
            .fold(None, |acc: Option<f64>, x| {
                Some(acc.map_or(x, |a| a.min(x)))
            })
    }

    /// Spread across samples as a percentage of the minimum, so a reader can see
    /// whether a difference clears the measurement noise.
    #[must_use]
    pub fn prove_spread_pct(&self) -> Option<f64> {
        let min = self.prove_secs()?;
        let max = self.prove_samples.iter().copied().fold(f64::MIN, f64::max);
        (self.prove_samples.len() > 1).then(|| 100.0 * (max - min) / min)
    }

    /// Share of LEZ's public-execution budget, as a percentage.
    #[must_use]
    pub fn budget_pct(&self) -> f64 {
        100.0 * self.cycles as f64 / LEZ_CYCLE_BUDGET as f64
    }
}

/// Executes, and optionally proves, a LEZ program.
///
/// Execution applies LEZ's public-mode session limit; proving deliberately does
/// not, mirroring `execute_and_prove_program`, which sets none.
pub fn measure<T: serde::Serialize>(
    elf: &[u8],
    instruction: &T,
    prove: bool,
    repeat: usize,
) -> Result<Run> {
    let program_id: ProgramId = risc0_zkvm::compute_image_id(elf)?
        .as_words()
        .try_into()
        .expect("an image id is 8 words");

    // One uninitialized account for the program to claim, as LEZ's hello-world
    // example does.
    let pre_states = vec![AccountWithMetadata::new(
        Account::default(),
        true,
        AccountId::new([7u8; 32]),
    )];
    let instruction_data: InstructionData = risc0_zkvm::serde::to_vec(instruction)?;

    let build_env = |apply_limit: bool| -> Result<ExecutorEnv<'static>> {
        let mut builder = ExecutorEnv::builder();
        if apply_limit {
            builder.session_limit(Some(LEZ_CYCLE_BUDGET));
        }
        builder.write(&program_id)?;
        builder.write(&None::<ProgramId>)?;
        builder.write(&pre_states)?;
        builder.write(&instruction_data)?;
        builder.build()
    };

    let session = default_executor().execute(build_env(true)?, elf)?;
    let mut out = Run {
        cycles: session.cycles(),
        prove_samples: Vec::new(),
        proof_bytes: None,
    };
    ensure!(
        out.cycles < LEZ_CYCLE_BUDGET,
        "execution hit the LEZ public budget"
    );

    for _ in 0..if prove { repeat.max(1) } else { 0 } {
        let started = Instant::now();
        let info = default_prover().prove(build_env(false)?, elf)?;
        out.prove_samples.push(started.elapsed().as_secs_f64());
        out.proof_bytes = Some(borsh::to_vec(&info.receipt.inner)?.len());
    }
    Ok(out)
}

/// One data package per signer, each over its own signed bytes, as RedStone
/// publishes an M-of-N update. Keys are deterministic so runs reproduce.
pub fn make_packages(n: usize) -> Result<Vec<Package>> {
    (0..n)
        .map(|i| {
            let mut msg = vec![0x5au8; REDSTONE_SIGNED_BYTES];
            // Stand-ins for the per-signer value and timestamp fields.
            msg[32] = i as u8;
            msg[REDSTONE_SIGNED_BYTES - 1] = (i as u8).wrapping_mul(7);

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
