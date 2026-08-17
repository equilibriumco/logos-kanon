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
| decode | 551 | 1,435 | 2,319 |
| keccak256 | 17,475 | 52,425 | 87,375 |
| recovery | 585,274 | 1,755,574 | 2,922,880 |
| signer-set membership | 165 | 546 | 995 |
| the rest of `verify_feed` | 17,939 | 20,962 | 24,681 |
| **whole update** | **621,404** | **1,830,942** | **3,038,250** |
| _harness floor, subtracted out_ | 25,323 | 62,293 | 99,493 |

At three signers, which is RFP-020's default threshold:

| component | share |
| --- | ---: |
| recovery | 95.88% |
| keccak256 | 2.86% |
| the rest of `verify_feed` | 1.14% |
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

**Recovery is the whole cost.** At three signers it is 95.88% of an update, and
the four other components together are 4.12%. No arrangement of the remaining
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
which the numbers show and which is still not worth acting on: 995 cycles at five
signers.

**Converting to the account's scale costs 11,270 cycles**, about 64% of one
keccak256 and most of the remainder row at one signer. It is a
16-byte long division with 128-bit intermediates
(`adr/0017-value-sanity-is-two-level-and-prices-convert-to-q64-64.md`), and it
runs once per update rather than once per package, which is why that row barely
grows with the signer count. At 0.6% of a three-signer update there is nothing to
do about it.

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
