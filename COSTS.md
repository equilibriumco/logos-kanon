# What an update costs

Cycle costs of `verifier_core::verify_feed`, broken down by component, measured
inside the product guest over payloads RedStone published.

Every figure here is generated and asserted by `methods/tests/cost.rs`. It is not
transcribed: the same code that prints the table is what CI compares against, so
a published figure and an asserted figure cannot drift apart.

```sh
cargo test --release -p kanon-methods                                  # assert
cargo test --release -p kanon-methods -- --nocapture the_cost_table    # regenerate
```

## The table

Cycles, on the `redstone-primary-prod` BTC/USD payloads captured in
`verifier-core/tests/vectors/`. Five real packages, five real signatures, one
data point each; the 1- and 3-signer columns are the same payload cut short.

| component | 1 signer | 3 signers | 5 signers |
| --- | ---: | ---: | ---: |
| decode | 586 | 1,501 | 2,416 |
| keccak256 | 17,484 | 52,452 | 87,420 |
| recovery | 585,278 | 1,755,586 | 2,922,900 |
| signer-set membership | 162 | 540 | 990 |
| the rest of `verify_feed` | 18,392 | 22,414 | 27,124 |
| **whole update** | **621,902** | **1,832,493** | **3,040,850** |
| _harness floor, subtracted out_ | 25,324 | 62,294 | 99,494 |

At three signers, which is RFP-020's default threshold:

| component | share |
| --- | ---: |
| recovery | 95.80% |
| keccak256 | 2.86% |
| the rest of `verify_feed` | 1.22% |
| decode | 0.08% |
| signer-set membership | 0.03% |

One more signer costs about 604,000 cycles, whatever the signer count already is.

## How it is measured

`methods/guest/src/bin/verify_cost.rs` runs a **prefix** of the pipeline, chosen
by a stage number. Every stage takes byte-identical input and does identical
setup before it branches, so zkVM startup, input deserialization and the journal
commit are the same in every run and cancel when two stages are subtracted.

| stage | adds |
| --- | --- |
| 0 | nothing: input, setup, journal |
| 1 | `Payload::decode` and a full package walk |
| 2 | keccak256 over each package's signable span |
| 3 | signer recovery from each package's signature |
| 4 | looking each recovered address up in the configured set |
| 5 | the real `verify_feed`, whole |

Each row is a difference of adjacent stages, and the whole update is stage 5 less
stage 0. Stages 1 to 4 call exactly the functions `verify_feed` calls, in the
same order, through `VerifierBackend` — they are a prefix of the shipped path,
not a model of it.

The fifth row is the remainder, named rather than left implicit: the median,
value sanity, the staleness window, the threshold ladder and the Q64.64
conversion. A table whose rows do not add up to its total has somewhere to hide.
A sixth stage, outside the chain, is stage 4 plus one Q64.64 conversion, which is
how the remainder's largest single item gets a figure of its own.

Every stage reports how many packages it got through, and the harness asserts it
against the signer count. A run that took an early exit is cheap, and would
otherwise read as a fast component rather than as a failure.

### What the harness floor is

The floor grows with the signer count because the payload and the signer set
arrive through `env::read()`, and RISC Zero's deserializer walks them a word at a
time. That is the measurement harness's cost, not the verifier's: it is
subtracted out of every row above, and a program that reads its payload from an
account rather than from the guest input stream pays something different for it.

## What the numbers say

**Recovery is the whole cost.** At three signers it is 95.80% of an update, and
the four other components together are 4.20%. No arrangement of the remaining
code matters next to it. If LEZ ever offers a secp256k1 precompile, that is the
one lever worth pulling, and `VerifierBackend` exists so that pulling it is one
new implementor and one type parameter
(`adr/0003-cryptographic-primitives-behind-a-verifier-backend-trait.md`).

**Nothing amortises.** Hashing and recovery cost the same per signer at one,
three and five, to within 0.12%. A wider threshold is linearly priced and there is
no engineering remedy for it — the choice of `M` is a cost decision as much as a
security one.

**An update computes twice as many hashes as the keccak256 row shows.** Recovery
ends by deriving an Ethereum address, which is a keccak256 over the 64-byte
public key, so a three-signer update runs six hashes: three over the signable
spans, and three inside `recover_signer`. Only the first three are in the
keccak256 row. `methods/guest/Cargo.toml` weighs the keccak accelerator against
"an update's three" hashes; the number is six, and
`adr/0020-per-component-costs-are-measured-in-the-product-guest.md` records why
that does not change the decision.

**Decoding is free, and so is the signer-set check.** Together they are 0.11% of
an update. The wire format is a backwards walk over fixed-width fields with no
allocation, and the membership check is a linear scan of at most 32 addresses.
That scan is quadratic in the signer count — every package scans the whole set —
which the numbers show and which is still not worth acting on: 990 cycles at five
signers.

**Converting to the account's scale costs 11,330 cycles**, about 64% of one
keccak256 and most of the remainder row at one signer. It is a
16-byte long division with 128-bit intermediates
(`adr/0017-value-sanity-is-two-level-and-prices-convert-to-q64-64.md`), and it
runs once per update rather than once per package, which is why that row barely
grows with the signer count. At 0.6% of a three-signer update there is nothing to
do about it.

## What a payload can spend

The rows above are one update's cost. They are not the most an update can cost,
because the number of packages is the payload's to choose: the count sits in the
envelope, outside every signature, and a package can be copied without a key.

At 602,749 cycles for a hash and a recovery, 56 packages exhaust LEZ's
33,554,432-cycle public budget. Measured rather than extrapolated: an 8 KB
payload of repeated packages came to 34,746,599 cycles, and a transaction that
reaches the limit aborts instead of publishing.

Two things bound it, both in `verify_feed`:

- **A package carrying no data point for the requested feed is skipped before it
  is hashed.** RedStone payloads are multi-feed by design, so most of what
  arrives belongs to somebody else. Before this, verifying one feed of a
  five-feed payload cost 15,477,759 cycles — 46% of the budget for 25 packages,
  20 of which could not have changed the answer.
- **`MAX_RECOVERIES` packages for the requested feed, then the payload is
  refused.** At the ceiling an update costs about 19.9M cycles, 59% of the
  budget, and past it the work stops growing.

`the_most_a_payload_can_cost_still_fits_in_one_transaction` asserts both, and
`adr/0026-a-payload-cannot-choose-how-much-of-the-budget-verification-spends.md`
records why the ceiling refuses rather than truncates.

Neither bounds what a payload costs to *read*. LEZ reads a program's whole
instruction data before the program's first instruction, at about 113 cycles a
byte, so those cycles are spent before any code here runs: a 127,814-byte payload
came to 33,792,622 cycles and overran the budget on the read alone, with
verification still correctly bounded at 19.3M of it.

`MAX_PAYLOAD_BYTES` is the answer, at 32 KiB. The largest payload the decoder
accepts — 230 packages, every one carrying the requested feed — reads and
verifies in **23,042,390 cycles, 69% of the budget**, leaving 31% for the program
doing the verifying. `the_largest_payload_the_decoder_accepts_is_read_and_verified_inside_the_budget`
asserts it. The limit is a cost rule rather than a framing one, and refusing at
this point does not refund the read; what it does is state the size above which a
caller must not submit, so a relayer or a sequencer can refuse it where refusing
is still free. `adr/0027-a-maximum-payload-size-and-where-it-has-to-be-enforced.md`
records the derivation.

## Agreement with the M0 baseline

`m0/cost-baseline` measured the two primitives in isolation, on a synthetic
workload, calling `k256` and `tiny-keccak` directly. This measures them inside
`verify_feed`, through `VerifierBackend`, on real payloads. The two agree:

| | M0, in isolation | here, in `verify_feed` |
| --- | ---: | ---: |
| software keccak256 over 77 bytes | 17,498 | 17,475 |
| accelerated recovery, per signer | 565,551 | 585,274 |

The 77 bytes are not a coincidence: a RedStone package carrying one data point
signs `1 * (32 + 32) + 4 + 6 + 3` bytes, and every captured package carries one.

The recovery gap is 19,723 cycles, and it is the address derivation the M0
harness did not include: one keccak256 over 64 bytes accounts for 17,475 of it,
leaving 2,248 for parsing and normalising the 65-byte signature.

## Reproducing

Measured on `risc0-zkvm` 3.0.5 with guest `rustc` 1.97.0, and confirmed identical
on the 3.0.6 toolchain — only the image id moves, which is the finding
`adr/0008-exact-pins-and-tracking-the-estate.md` records. `guardrails.yml` runs
the assertions on every push, with the pinned toolchain installed.

Cycle counts are a deterministic function of the guest ELF and its input, so the
figures are pinned by exact equality rather than by a ceiling. A failure means a
figure moved; it is a prompt to re-measure and re-publish this file, not
necessarily a defect.
