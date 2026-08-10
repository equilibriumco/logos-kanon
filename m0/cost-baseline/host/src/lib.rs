//! Measurement core for the M0 cost baseline, shared by the CLI and the
//! guardrail tests.
//!
//! See the crate README for what is measured and why. The essentials:
//!
//! - Every workload runs twice against byte-identical input, once with the
//!   cryptographic work enabled and once with it skipped. The difference is the
//!   cost of the primitive; zkVM startup, input deserialization and the journal
//!   commit cancel out. That difference is a [`Delta`].
//! - Cycles are deterministic across machines and need execution only, so they
//!   are cheap to assert on. They are **not** sound across configurations: the
//!   keccak accelerator moves work into a coprocessor the cycle count cannot
//!   see, so it looks cheaper in cycles while proving more slowly and producing
//!   a larger proof. Cross-configuration claims need `prove`.
//!
//! The CLI reports these quantities; the guardrail tests pin them. Both go
//! through the same functions, so a published figure and an asserted figure
//! cannot drift apart.

use std::time::Instant;

use anyhow::{ensure, Result};
use k256::ecdsa::SigningKey;
use risc0_zkvm::{default_executor, default_prover, ExecutorEnv};
use tiny_keccak::{Hasher, Keccak};

/// Message sizes to hash, in bytes.
///
/// 77 is the size that actually matters: it is what RedStone signs per data
/// package for a single data point, per the wire format in RedStone's own Rust
/// SDK (`crates/redstone/src/protocol/constants.rs`):
///
/// ```text
/// data_point_count * (value_size + DATA_FEED_ID_BS)
///   + DATA_POINT_VALUE_BYTE_SIZE_BS + TIMESTAMP_BS + DATA_POINTS_COUNT_BS
/// = 1 * (32 + 32) + 4 + 6 + 3
/// ```
///
/// The signature covers that range only, so each signer's digest is a 77-byte
/// hash, not a hash of the whole payload.
///
/// 32 is the single-permutation floor, 141 is the two-data-point package, and
/// 512 extends the curve far enough to show how cost scales with size.
pub const KECCAK_MSG_LENS: &[u32] = &[32, 77, 141, 512];

/// Hashes per run. Enough that the per-hash figure is not dominated by loop
/// setup, small enough that the software baseline still finishes quickly.
pub const KECCAK_ITERS: u32 = 16;

/// Signer counts to recover. 3 is RFP-020's default M-of-N threshold; 1 is the
/// unit cost; 5 shows whether the cost is linear in the signer count, which is
/// what decides whether a wider threshold is affordable.
pub const RECOVER_SIGNER_COUNTS: &[usize] = &[1, 3, 5];

/// The signed byte range of one RedStone data package carrying a single data
/// point with a 32-byte value: `1 * (32 + 32) + 4 + 6 + 3`.
pub const REDSTONE_SIGNED_BYTES: u32 = 77;

/// RFP-020's default threshold.
pub const VERIFY_SIGNERS: usize = 3;

/// The digest the recovery workload signs over. Fixed so the measurement is
/// reproducible run to run.
pub const RECOVER_MESSAGE: &[u8] = b"kanon m0 cost baseline";

/// One RedStone data package as the guest receives it:
/// `(signed_bytes, digest, signature, recovery_id)`.
pub type Package = (Vec<u8>, Vec<u8>, Vec<u8>, u8);

/// The guest ELFs of one configuration. The three configurations differ only in
/// which of RISC Zero's accelerated crate forks they patch in.
pub struct Elfs {
    pub name: &'static str,
    pub noop: &'static [u8],
    pub keccak: &'static [u8],
    pub keccak_overhead: &'static [u8],
    pub recover: &'static [u8],
    pub verify: &'static [u8],
}

/// No accelerators: upstream `k256` and `tiny-keccak`.
#[must_use]
pub fn software() -> Elfs {
    Elfs {
        name: "software",
        noop: kanon_methods_software::NOOP_ELF,
        keccak: kanon_methods_software::KECCAK_ELF,
        keccak_overhead: kanon_methods_software::KECCAK_OVERHEAD_ELF,
        recover: kanon_methods_software::RECOVER_ELF,
        verify: kanon_methods_software::VERIFY_ELF,
    }
}

/// Both accelerators, including the keccak256 coprocessor.
#[must_use]
pub fn accelerated() -> Elfs {
    Elfs {
        name: "accelerated",
        noop: kanon_methods_accelerated::NOOP_ELF,
        keccak: kanon_methods_accelerated::KECCAK_ELF,
        keccak_overhead: kanon_methods_accelerated::KECCAK_OVERHEAD_ELF,
        recover: kanon_methods_accelerated::RECOVER_ELF,
        verify: kanon_methods_accelerated::VERIFY_ELF,
    }
}

/// Accelerated recovery, software keccak: the combination neither of the other
/// two tests, and the one the proving data says is optimal.
#[must_use]
pub fn mixed() -> Elfs {
    Elfs {
        name: "mixed",
        noop: kanon_methods_mixed::NOOP_ELF,
        keccak: kanon_methods_mixed::KECCAK_ELF,
        keccak_overhead: kanon_methods_mixed::KECCAK_OVERHEAD_ELF,
        recover: kanon_methods_mixed::RECOVER_ELF,
        verify: kanon_methods_mixed::VERIFY_ELF,
    }
}

/// All three, in the order the CLI reports them.
#[must_use]
pub fn variants() -> [Elfs; 3] {
    [software(), accelerated(), mixed()]
}

/// How to run a measurement.
#[derive(Clone, Copy, Default)]
pub struct Opts {
    /// Cap the cycles per segment at 2^N. Lowering this cuts the prover's peak
    /// memory at the cost of more segments, which is how GPU proving fits on a
    /// card whose VRAM cannot hold a full-size segment. Comparisons are only
    /// valid between runs that used the same value.
    pub segment_po2: Option<u32>,
    /// Generate real proofs, for proving wall-clock and proof size. Much slower
    /// than cycle counting.
    pub prove: bool,
    /// Prove this many times and keep every sample. Only meaningful with
    /// `prove`.
    pub repeat: usize,
}

impl Opts {
    /// Execution only: cycles and segments, no proving.
    #[must_use]
    pub fn execute_only() -> Self {
        Self::default()
    }
}

/// What one guest run cost.
pub struct Run {
    pub cycles: u64,
    pub segments: usize,
    pub journal_words: Vec<u8>,
    /// Every proving sample, in seconds. [`Run::prove_secs`] reports the
    /// minimum, which is the least noise-contaminated estimate of the real cost.
    pub prove_samples: Vec<f64>,
    pub proof_bytes: Option<usize>,
    /// Receipt flavour, and how many assumptions it carries. An accelerator that
    /// yields a composite receipt with unresolved assumptions changes what an
    /// on-chain verifier has to do, so it is not a free swap.
    pub receipt_kind: Option<String>,
}

impl Run {
    pub fn journal_decode<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        Ok(risc0_zkvm::serde::from_slice(&self.journal_words)?)
    }

    /// The fastest proving sample. Noise on a shared machine only ever adds
    /// time, so the minimum is the best estimate of the underlying cost.
    #[must_use]
    pub fn prove_secs(&self) -> Option<f64> {
        self.prove_samples
            .iter()
            .copied()
            .fold(None, |acc: Option<f64>, x| {
                Some(acc.map_or(x, |a| a.min(x)))
            })
    }

    /// Spread across samples, as a percentage of the minimum. Reported so a
    /// reader can see whether a difference between configurations is larger than
    /// the measurement noise.
    #[must_use]
    pub fn prove_spread_pct(&self) -> Option<f64> {
        let min = self.prove_secs()?;
        let max = self.prove_samples.iter().copied().fold(f64::MIN, f64::max);
        (self.prove_samples.len() > 1).then(|| 100.0 * (max - min) / min)
    }
}

/// A workload measured twice, with the cryptographic work enabled and skipped.
///
/// `work` is the quantity the report and the guardrails both use: the cost of
/// the primitive with the fixed costs around it subtracted out.
pub struct Delta {
    /// `busy.cycles - idle.cycles`, saturating.
    pub work: u64,
    pub busy: Run,
    pub idle: Run,
}

/// `iters` hashes of a `msg_len`-byte message, measured against the same loop
/// with hashing skipped.
///
/// Pass `elfs.keccak` for the hash itself, or `elfs.keccak_overhead` for the
/// loop's own bookkeeping: both guests take the same `(do_work, iters, len)`
/// input, which is what makes one subtractable from the other.
pub fn keccak_delta(elf: &[u8], iters: u32, msg_len: u32, opts: Opts) -> Result<Delta> {
    // Only the busy run is ever proved; the idle run exists to be subtracted,
    // and proving it would cost minutes for a number nothing reads.
    delta(elf, &(false, iters, msg_len), &(true, iters, msg_len), opts)
}

/// One secp256k1 recovery per signer over a single shared digest: the shape of a
/// RedStone M-of-N check.
///
/// Fails rather than reporting if the guest did not recover exactly `signers`
/// keys, because a run that silently took the cheap error path is not a
/// measurement of `signers` recoveries.
pub fn recover_delta(elf: &[u8], signers: usize, opts: Opts) -> Result<Delta> {
    let digest = keccak256(RECOVER_MESSAGE);
    let sigs = sign_with_n_signers(&digest, signers)?;
    let out = delta(
        elf,
        &(false, digest.to_vec(), sigs.clone()),
        &(true, digest.to_vec(), sigs),
        opts,
    )?;
    ensure_recovered(&out.busy, signers)?;
    Ok(out)
}

/// The full M-of-N update: one data package per signer, each hashed over its own
/// signed bytes and then recovered.
///
/// `hash` toggles only whether the digests are computed in-guest or taken from
/// the input, so two runs differing in it isolate keccak256's marginal cost
/// inside a real verification rather than in a synthetic loop.
pub fn verify_run(elf: &[u8], signers: usize, hash: bool, opts: Opts) -> Result<Run> {
    let packages = make_packages(signers)?;
    let run = execute_with(elf, &(hash, packages), opts)?;
    ensure_recovered(&run, signers)?;
    Ok(run)
}

/// The guest reports how many recoveries succeeded; a shortfall means the cycle
/// count measures error paths rather than the work.
fn ensure_recovered(run: &Run, expected: usize) -> Result<()> {
    let (recovered, _checksum): (u32, u8) = run.journal_decode()?;
    ensure!(
        recovered as usize == expected,
        "expected {expected} successful recoveries, guest reported {recovered}"
    );
    Ok(())
}

/// Runs one guest twice and subtracts. `opts.prove` applies to the busy run
/// only.
fn delta<T: serde::Serialize>(elf: &[u8], idle_in: &T, busy_in: &T, opts: Opts) -> Result<Delta> {
    let idle = execute_with(
        elf,
        idle_in,
        Opts {
            prove: false,
            ..opts
        },
    )?;
    let busy = execute_with(elf, busy_in, opts)?;
    Ok(Delta {
        work: busy.cycles.saturating_sub(idle.cycles),
        busy,
        idle,
    })
}

/// Executes, and optionally proves, one guest against one input.
pub fn execute_with<T: serde::Serialize>(elf: &[u8], input: &T, opts: Opts) -> Result<Run> {
    // Rebuilt per use: executing and proving each consume the env.
    let build_env = || -> Result<ExecutorEnv<'static>> {
        let mut builder = ExecutorEnv::builder();
        if let Some(po2) = opts.segment_po2 {
            builder.segment_limit_po2(po2);
        }
        builder.write(input)?;
        builder.build()
    };

    let session = default_executor().execute(build_env()?, elf)?;
    let mut run = Run {
        cycles: session.cycles(),
        segments: session.segments.len(),
        journal_words: session.journal.bytes.clone(),
        prove_samples: Vec::new(),
        proof_bytes: None,
        receipt_kind: None,
    };

    for _ in 0..if opts.prove { opts.repeat.max(1) } else { 0 } {
        let started = Instant::now();
        let info = default_prover().prove(build_env()?, elf)?;
        run.prove_samples.push(started.elapsed().as_secs_f64());
        run.proof_bytes = Some(bincode::serialize(&info.receipt)?.len());
        let flavour = match &info.receipt.inner {
            risc0_zkvm::InnerReceipt::Composite(_) => "composite",
            risc0_zkvm::InnerReceipt::Succinct(_) => "succinct",
            risc0_zkvm::InnerReceipt::Groth16(_) => "groth16",
            _ => "other",
        };
        let assumptions = info
            .receipt
            .claim()
            .ok()
            .and_then(|c| c.as_value().ok().cloned())
            .and_then(|c| c.output.as_value().ok().cloned().flatten())
            .map(|o| match o.assumptions.as_value() {
                Ok(a) => a.iter().count(),
                Err(_) => 0,
            })
            .unwrap_or(0);
        run.receipt_kind = Some(format!("{flavour}, {assumptions} assumption(s)"));
    }
    Ok(run)
}

/// One data package per signer, each carrying its own signed bytes and its own
/// signature, which is how RedStone actually publishes an M-of-N update. The
/// per-signer bytes differ, so each package needs its own hash.
pub fn make_packages(n: usize) -> Result<Vec<Package>> {
    (0..n)
        .map(|i| {
            let mut msg = vec![0x5au8; REDSTONE_SIGNED_BYTES as usize];
            // Stand-ins for the per-node value and timestamp fields.
            msg[32] = i as u8;
            msg[REDSTONE_SIGNED_BYTES as usize - 1] = (i as u8).wrapping_mul(7);
            let digest = keccak256(&msg);

            let mut scalar = [0u8; 32];
            scalar[31] = (i + 1) as u8;
            let key = SigningKey::from_bytes(&scalar.into())?;
            let (sig, recid) = key.sign_prehash_recoverable(&digest)?;
            Ok((
                msg,
                digest.to_vec(),
                sig.to_bytes().to_vec(),
                recid.to_byte(),
            ))
        })
        .collect()
}

/// Deterministic signing keys, so the measurement is reproducible run to run.
pub fn sign_with_n_signers(digest: &[u8; 32], n: usize) -> Result<Vec<(Vec<u8>, u8)>> {
    (0..n)
        .map(|i| {
            let mut scalar = [0u8; 32];
            scalar[31] = (i + 1) as u8;
            let key = SigningKey::from_bytes(&scalar.into())?;
            let (sig, recid) = key.sign_prehash_recoverable(digest)?;
            Ok((sig.to_bytes().to_vec(), recid.to_byte()))
        })
        .collect()
}

#[must_use]
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}
