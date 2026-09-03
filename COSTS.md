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
| decode | 582 | 1,495 | 2,408 |
| keccak256 | 17,476 | 52,428 | 87,380 |
| recovery | 585,276 | 1,755,580 | 2,922,890 |
| signer-set membership | 162 | 534 | 970 |
| the rest of `verify_feed` | 17,773 | 21,601 | 26,142 |
| **whole update** | **621,269** | **1,831,638** | **3,039,790** |
| _harness floor, subtracted out_ | 25,324 | 62,295 | 99,491 |

At three signers, which is RFP-020's default threshold:

| component | share |
| --- | ---: |
| recovery | 95.85% |
| keccak256 | 2.86% |
| the rest of `verify_feed` | 1.18% |
| decode | 0.08% |
| signer-set membership | 0.03% |

One more signer costs about 605,000 cycles, whatever the signer count already is.

## What the write costs

The table above is verification. A submission also has to read its accounts and
write the price, and that is the other half of what a push transaction spends
(M2-18). Same payload, five signers:

| | first write | update |
| --- | ---: | ---: |
| verification | 3,039,832 | 3,039,832 |
| **the write** | **7,386** | **8,582** |
| whole `submit_price` body | 3,047,218 | 3,048,414 |
| _the write, as a share of the body_ | _0.242%_ | _0.282%_ |
| _harness floor, subtracted out_ | 138,475 | 153,882 |
| _the write, against LEZ's per-transaction budget_ | _0.0220%_ | _0.0256%_ |

**The write is a rounding error against verifying.** Under three tenths of one
percent of the body, and 8,582 cycles is 0.0256% of the 33,554,432-cycle
per-transaction budget. The cost of a push submission is the cost of recovering
signatures; no optimisation on the write side is worth looking for.

### The body is not the whole instruction

Those figures measure `aggregator_program::submit_price`, which is the delegated
body. LEZ runs more than that. `price_account` is declared

```rust
#[account(mut, pda = [account("feed"), r#const("KANON_PRICE_ACCOUNT")])]
```

so SPEL generates a validator that derives and checks that address *before* the
body is entered, and the dispatcher decodes the instruction and wraps the result
afterwards. Measured separately:

| | cycles |
| --- | ---: |
| the generated validator's PDA derivation | 1,722 |

The dispatcher and the `SpelOutput` wrapping are in none of these figures, and
cannot be reached by calling a function: they need the macro's generated entry
point, which takes a whole LEZ transaction. So what is published here is **the
body, plus a named validator cost** — not the instruction as a chain executes it.
Against 3.05M cycles of verification the gap is immaterial; it is stated because a
figure called "the instruction" that excluded a validator would be wrong.

### The two paths

`submit_price` branches on whether the price account already exists:

- a **first write** builds the account from nothing. Once per feed, ever.
- an **update** decodes the published `OraclePriceAccount` — 136 bytes — and
  checks the pair it carries. Every heartbeat and every deviation trigger after
  that.

So the recurring cost is the update's. The 1,196-cycle difference between them is
a **net between two paths, not a component**: an update pays for a read and for
`publish`'s three checks, while a first write pays for a claim in `post_states`
and for building the account. Each of those was measured on its own:

| measured in isolation | cycles |
| --- | ---: |
| published-account read, update only | 1,442 |
| `publish`'s checks, update only (includes the read above) | 2,381 |
| `AutoClaim::pda_from_seeds`, first write only | 1,304 |
| `publish::price_account` and encoding the result, first write only | 1,782 |

**These do not sum to 1,196, and that is not a discrepancy to chase.** Isolated
calls do not cost what inlined ones do. `read - claim` is 138 cycles. Taking all
four — what only an update does, less what only a first write does — gives
2,381 − (1,304 + 1,782) = **−705**, the wrong sign. Two attempts to make the terms
add up failed, the second further from the answer than the first, and that is
recorded here so a third is not attempted. Each figure above is real, pinned by
exact equality, and useful for knowing what an operation costs; none is a
component of the write.

The build figure is build *and encode*, deliberately. A construction whose fields
are never all read is dead code: with the stage observing only the built
account's timestamp, the compiler elided the call entirely and the figure was
identical with the call deleted — 43 cycles of nothing, which @frenzox found on
#60. Observing one byte per field still elided the 32-byte copies, at 51. The
real path builds the account and then assigns `Data::from(written)`, so measuring
the pair is both faithful and the only version that cannot be optimised away.

What "the write" contains, named in full because a residual should not be called a
component: the ownership check on the feed account, decoding the clock from the
account LEZ pins, decoding the stored `FeedAccount`, the paused check, rebuilding
the signer set and `FeedConfig` from what was stored, triaging the price account
between the two cases, the read or the claim above, choosing the asset pair to
check against, computing the new account, and producing the post-states LEZ
applies.

Not in these figures: what LEZ spends reading the instruction data before the
program is entered, about 113 cycles a byte, which *is* material and is
`MAX_PAYLOAD_BYTES`'s subject (ADR 26).

### Registering a feed

P2 names the push side's "account write **and** registration", so registration is
the other figure it asks for:

| | cycles |
| --- | ---: |
| a first registration | 5,270 |
| _as a share of the per-transaction budget_ | _0.0157%_ |

Cheap for the same reason the write is: no cryptography. What a registration
spends is the authority gate, Borsh, one PDA derivation and a handful of
comparisons.

The gate is in the figure because the handler runs it: `register_feed`'s generated
handler calls `admin::authorise` before delegating, and a measurement that skipped
it published 3,821 — the cost of the helper rather than of registering a feed.
@frenzox found that on #60.

Measured on a **first** registration — the feed account fully default, which is
the state that claims it. A re-registration after a deregistration takes a
different branch; it happens once per retired feed and never on an operating
path, so it is not measured. On the same terms as the write, the generated
validator and the dispatcher are outside it.

It lives in its own guest, `methods/guest/src/bin/register_cost.rs`, rather than
as another stage of `submit_cost.rs`. Cycle counts are a property of an ELF, so
a stage added to one guest moves every figure that guest publishes — keeping
registration separate means the write figures do not have to be re-pinned when
registration changes, and a sibling binary does not perturb them.

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

The write is measured the same way by a second guest,
`methods/guest/src/bin/submit_cost.rs`, which brackets the body instead of the
pipeline:

| stage | adds |
| --- | --- |
| 0 | nothing: input, setup, the three accounts built, journal |
| 1 | `Payload::decode` and `verify_feed`, called as `submit_price` calls them |
| 2 | the real `submit_price`, whole |

So `1 - 0` is verification, `2 - 0` is the body, and `2 - 1` is the write.

Five further stages sit outside that chain, each calling one thing the shipped
path calls: the generated validator's PDA derivation, the published-account read,
the auto-claim, `publish::price_account` and `publish`. They are what turn the
figures in *The two paths* from assertions into measurements, and each is pinned
separately. Each of the two cases is measured with its own stage 0, because the price
account is built during setup: subtracting an update's stage 2 from a *first
write*'s stage 1 credits the update with 15,410 cycles of setup it never paid,
which is how the first version of this measurement was wrong.

Stage 2 reports how many post-states came out and the harness requires three. A
submission that refused returns an error and no post-states, so without that
check a broken submission would publish as an implausibly cheap write — which is
not hypothetical: pointing the guest at a clock account the program may not read
makes every one of these figures collapse, and the check is what turns that into
a failure.

Two guests reaching `verify_feed` by different routes also corroborate each
other: they agree on verification to 42 cycles in 3,039,790, which is 0.0014%.
That comparison is the one figure here bounded rather than pinned exactly, and
the reason is that a cycle count is a property of a guest ELF rather than of the
work — two independently compiled binaries running identical source are not
obliged to agree to the cycle.

### What the harness floor is

The floor grows with the signer count because the payload and the signer set
arrive through `env::read()`, and RISC Zero's deserializer walks them a word at a
time. That is the measurement harness's cost, not the verifier's: it is
subtracted out of every row above, and a program that reads its payload from an
account rather than from the guest input stream pays something different for it.

## What the numbers say

**Recovery is the whole cost.** At three signers it is 95.85% of an update, and
the four other components together are 4.15%. No arrangement of the remaining
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
which the numbers show and which is still not worth acting on: 970 cycles at five
signers.

**Converting to the account's scale costs 11,294 cycles**, about 65% of one
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

At about 602,700 cycles for a hash and a recovery, 56 packages exhaust LEZ's
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
`adr/0025-a-payload-cannot-choose-how-much-of-the-budget-verification-spends.md`
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
is still free. `adr/0026-a-maximum-payload-size-and-where-it-has-to-be-enforced.md`
records the derivation.

## Agreement with the M0 baseline

`m0/cost-baseline` measured the two primitives in isolation, on a synthetic
workload, calling `k256` and `tiny-keccak` directly. This measures them inside
`verify_feed`, through `VerifierBackend`, on real payloads. The two agree:

| | M0, in isolation | here, in `verify_feed` |
| --- | ---: | ---: |
| software keccak256 over 77 bytes | 17,498 | 17,476 |
| accelerated recovery, per signer | 565,551 | 585,276 |

The 77 bytes are not a coincidence: a RedStone package carrying one data point
signs `1 * (32 + 32) + 4 + 6 + 3` bytes, and every captured package carries one.

The recovery gap is 19,725 cycles, and it is the address derivation the M0
harness did not include: one keccak256 over 64 bytes accounts for 17,476 of it,
leaving 2,249 for parsing and normalising the 65-byte signature.

## Reproducing

Measured on `risc0-zkvm` 3.0.5 with guest `rustc` 1.97.0, and confirmed identical
on the 3.0.6 toolchain — only the image id moves, which is the finding
`adr/0008-exact-pins-and-tracking-the-estate.md` records. `guardrails.yml` runs
the assertions on every push, with the pinned toolchain installed.

Cycle counts are a deterministic function of the guest ELF and its input, so the
figures are pinned by exact equality rather than by a ceiling. A failure means a
figure moved; it is a prompt to re-measure and re-publish this file, not
necessarily a defect.

```sh
cargo test --release -p kanon-methods                                      # the assertions
cargo test --release -p kanon-methods -- --nocapture the_cost_table        # the first table
cargo test --release -p kanon-methods -- --nocapture the_push_write_cost   # the second
```

Both tables are printed by the code that asserts them, so a published figure and
an asserted figure cannot drift apart.
