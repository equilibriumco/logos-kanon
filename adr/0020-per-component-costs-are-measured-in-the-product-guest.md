# 20. Per-component costs are measured in the product guest, by differencing pipeline prefixes

- **Status**: accepted
- **Milestone**: M1 (`M1-25`)
- **Requirements**: P2
- **Artefacts**: `COSTS.md`, `methods/tests/cost.rs`, `methods/guest/src/bin/verify_cost.rs`, `.github/workflows/guardrails.yml`

## Context

M1-25 asks for a published cost table with four named components: per-signer
recovery, keccak256, decode, and the signer-set membership check.

`m0/cost-baseline` already publishes cycle figures, and they are the reason ADR 6
chose the mixed accelerator configuration. They are not this table:

- **The workload is synthetic.** `bench-lib`'s `verify_bench` builds its own
  packages from a fixed `0x5a` fill and signs them with the scalars `1..n`. It
  never decodes a payload, never checks a signer set, never takes a median and
  never converts to the account's scale.
- **The code is not the shipped code.** `bench-lib` calls `k256` and
  `tiny-keccak` directly. `verifier-core` reaches them through
  `VerifierBackend`, which is the whole point of ADR 3, and the difference is not
  zero.
- **Two of the four components had never been measured.** Neither decode nor the
  membership check appears anywhere in `m0/`.

So M1-25 is a second measurement, of the real path, and the M0 figures become the
cross-check rather than the answer.

## Decision

Measure `verify_feed` in the product guest by running **prefixes of the
pipeline** and differencing them.

`methods/guest/src/bin/verify_cost.rs` takes a stage number and runs everything
the stage below it runs, plus one component. Every stage takes byte-identical
input and performs identical setup before it branches, so zkVM startup, input
deserialization, the signer vector and the journal commit are the same in every
run and cancel on subtraction. Each row of the table is a difference of adjacent
stages.

Stages 1 to 4 call the same functions `verify_feed` calls, in the same order,
through the same trait: `Payload::decode`, `for_each_package`,
`backend.keccak256`, `backend.recover_signer`, and `position` over
`config.signers()`. They are a prefix of the shipped path rather than a model of
it, which is the property that makes the differences attributable.

Three consequences of that choice are worth stating, because they are what
separates this from a benchmark:

**The remainder is a row, not a rounding error.** Stage 5 is the real
`verify_feed`, so `stage 5 - stage 4` is everything the four named components
leave out — median, value sanity, the staleness window, the threshold ladder and
the Q64.64 conversion. A table whose rows do not add up to its total has
somewhere to hide. A sixth stage sits outside the chain and adds one Q64.64
conversion to stage 4, so the largest single item in that remainder has a figure
of its own rather than an estimate.

**Every stage reports how far it got, and the harness asserts it.** A run that
took an early exit — a payload that did not decode, a signature that did not
recover — costs a fraction of the work it claims to measure, and without the
check it would read as a fast component rather than as a failure. This is M0's
`ensure_recovered` rule, applied to every stage.

**The workload is the M1-21 capture.** Five real packages, five real signatures,
one data point each. The 1- and 3-signer figures are the same payload cut short:
the packages are a fixed 142-byte stride apart, so the first `n` of them are a
valid payload once the count is re-emitted, and the untouched signatures still
recover to the first `n` published signers. Stage 5 verifying to a price at each
point is the proof that the cut payload is genuine.

### Where it lives, and why CI moves with it

`methods/tests/cost.rs`, in the product workspace, with `risc0-zkvm` as a
dev-dependency of `kanon-methods`.

Not in `m0/`. The figures have to be a property of the *product* pins — risc0
3.0.5, the product `[patch.crates-io]`, the LEZ graph — and m0's pins are frozen
to the toolchain its published figures were measured on (ADR 7, ADR 8). Not a new
crate either: `kanon-methods` already owns the product ELFs, and a new workspace
member would mean a second lockfile that could drift from the product's, which is
the failure ADR 7 exists to avoid.

The cost is that `kanon-methods`' tests now need real guest ELFs, which
`RISC0_SKIP_BUILD` stubs out. `ci.yml`'s `build-test` job therefore excludes the
crate, and `guardrails.yml` — the workflow that installs the RISC Zero toolchain
and exists to be the cycle gate — gains a job that runs it. That is the split
`m0/` already had, applied to one more crate, and it keeps a red tick meaning one
thing in each workflow.

### ADR 6's two ceilings land here, and both are cycle counts

`adr/README.md` records ADR 6's CI ceilings as outstanding, and
`methods/guest/Cargo.toml` says what they were meant to be: "a cycle ceiling on
one recovery catches losing secp256k1, and a proof-size ceiling on a 3-of-N
update catches accidentally gaining keccak."

The proof-size half was necessary only while the workload was a single opaque
number. The keccak coprocessor's work does not appear in the cycle count, so
*gaining* the accelerator makes the total 2.4% cheaper — no ceiling on the total
can catch that, and the honest signal was 223 KB of extra proof.

Once keccak256 is a row of its own, both directions are visible in cycles:

- losing the `k256` patch multiplies the recovery row by about twenty;
- gaining the `sha2`/keccak patch divides the keccak256 row by about seven.

So both ceilings are execution-only and deterministic, and cost seconds rather
than the minutes a proving run needs. Exact equality rather than an inequality,
matching M0: a ceiling passes quietly when a figure improves for a reason nobody
has understood yet, and the point is to force a deliberate re-tag.

## Consequences

- **Recovery is 95.88% of a three-signer update**, and the other four components
  together are 4.12%. That settles where optimisation effort can and cannot go: a
  secp256k1 precompile is the only lever that matters, and ADR 3's trait is what
  keeps reaching for it a localised change.
- **Widening the threshold is linearly priced.** Hashing and recovery cost the
  same per signer at one, three and five, to within 0.12%, so the choice of `M`
  is a cost decision as much as a security one. M0 asserts this of bare recovery;
  it now holds of the real path.
- **An update runs twice as many keccak256 hashes as the table's keccak row.**
  `recover_signer` finishes by deriving an Ethereum address, which is a keccak256
  over the 64-byte public key, so a three-signer update hashes six times, not
  three. The measurement shows it: the recovery row is 19,723 cycles per signer
  above M0's bare recovery, and a 64-byte software hash is 17,475 of that.

  ADR 6's arithmetic used three. It is six, which halves the number of hashes the
  coprocessor charge has to amortise against — but the decision does not move,
  because it did not rest on the hash count. It rested on 26.90 s of proving and
  223 KB of proof per update in private mode, against a 2.4% cycle saving in
  public mode, and on the accelerated configuration failing to fit in 6 GiB of
  VRAM where mixed proved successfully. Those figures are unchanged. This record
  corrects the count rather than superseding the decision.
- **The remainder is small and mostly one thing.** The Q64.64 conversion is
  11,270 cycles, about 64% of one keccak256, and it runs once per update rather
  than once per package. At 0.6% of a three-signer update, the 16-byte long
  division ADR 17 chose is not worth revisiting.
- **`COSTS.md` is generated by the code that asserts it.** The test that prints
  the table and the test that pins it read the same measurements, so a published
  figure and an asserted figure cannot drift apart. A guardrail failure is a
  prompt to re-measure and re-publish, not necessarily a defect.
- **A second product guest ELF is built on every push.** It is measurement code
  in a directory that otherwise holds product code, which is a real cost in
  clarity, paid for the thing that makes the measurement worth anything: the
  product's `[patch.crates-io]` only applies inside the product guest workspace,
  and a figure measured under different patches is a figure for a different
  program.

## Alternatives considered

- **Extend `m0/cost-baseline` instead.** It would measure the product's code
  against m0's frozen pins, which describes a program that does not ship. The
  workspace split exists precisely so those two answers can differ (ADR 7).
- **One boolean flag per component, as `m0` does for one primitive.** Two runs
  and one flag isolate one primitive cleanly. Five components would need five
  flags and 2^5 combinations to say anything about interactions, or five separate
  guests. A cumulative prefix is one guest, six runs, and the differences are the
  table's rows.
- **Instrument `verify_feed` itself with cycle counters.** It would put
  measurement code on the shipped path, where ADR 4 says a panic aborts a
  transaction and every branch is audited. The cost of the measurement would also
  land inside the thing being measured.
- **A synthetic payload built by `PayloadBuilder`.** Available, deterministic, and
  it would describe a payload this repository invented. The M1-21 capture is real
  wire data and was already committed for exactly this kind of use.
- **Ceilings rather than exact equality.** An inequality tolerates a figure
  drifting downwards, which is how a silently-changed `[patch.crates-io]` would
  present. M0 settled this and the reasoning is unchanged (ADR 10).
