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
| `publish::price_account` and encoding the result, first write only | 422 |

**These do not sum to 1,196, and that is not a discrepancy to chase.** Isolated
calls do not cost what inlined ones do. `read - claim` is 138 cycles. Taking all
four — what only an update does, less what only a first write does — gives
2,381 − (1,304 + 422) = 655: about half way, and still not it. Two attempts to
make the terms add up failed, and that is recorded here so a third is not
attempted. Each figure above is real, pinned by exact equality, and useful for
knowing what an operation costs; none is a component of the write.

### Keeping a measured value alive without charging for it

The build figure is build *and encode*, because the real path does both:
`post_states` assigns `Data::from(written)` straight after building. Encoding is
also what makes the figure measurable, and getting the observation right took two
corrections, both from @frenzox on #60:

| what the stage observed | figure | what was wrong |
| --- | ---: | --- |
| `built.timestamp` | 43 | equal to `verified.timestamp_ms`, so the construction was elided entirely — the figure was identical with the call deleted |
| one byte per field | 51 | the 32-byte copies were still elided |
| a fold over the encoded bytes | 1,782 | real, but ~1,360 of it was the fold, which the real path does not do |
| `core::hint::black_box(&encoded)` | **422** | the value escapes without generating work |

Two lessons in one row each. A figure too cheap to be plausible is the same
signal as a test that cannot fail: 43 cycles to build a 136-byte account should
not have been published. And an observation that keeps a value alive can cost more
than the thing being measured, so `black_box` is the tool rather than a checksum.

The `-705` this section used to report for the four-term estimate was an artefact
of the fold, not a property of the code.

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

## What a read costs, in each mode

The two tables above are what a *submission* spends. This is what a **consumer**
spends to obtain a usable price, which is where the two modes stop being
variations on one design (M3-08).

| | pull | push |
| --- | ---: | ---: |
| **the mode's read** | **3,041,427** | **6,803** |
| settling, beyond the read | 7,275 | 6,479 |
| the whole `settle` body | 3,048,702 | 13,282 |
| _harness floor, subtracted out_ | 147,494 | 70,584 |
| _the read, against LEZ's per-transaction budget_ | _9.06%_ | _0.0203%_ |

**A pull read costs 447 times a push read.** Not a tuning difference — the pull
consumer has nothing published to fetch, so its read *is* a verification, and it
pays that on every read. The push consumer reads six fields the aggregator
already verified once, and recovers no signatures.

96% of the gap is signature recovery, and nothing else in it comes close.
`cost.rs` measures five-signer recovery at 2,922,890 against a difference of
3,034,624, and `the_gap_between_the_modes_is_signature_recovery` asserts that
share rather than leaving it as a claim. The other 4% is the rest of
verification — the package walk, the hashing, the membership lookups — so the
claim is that the gap is one component and not that it is only one component.
That is the same conclusion the component table reaches for an update, and the
reason ADR 3's `VerifierBackend` exists.

Both stage 1s contain the same kind of work, which is what makes the two
residuals on the second row comparable. Everything a consumer holds *before* it
reads — the registration's decode on both sides, and the order's on the pull
side, where `verify_price` needs the asset pair off it — is done in the setup
that cancels. An earlier draft did those decodes inside the read and so charged
the reference consumer's limit order to pull mode and a fifth of the push figure
to push mode, which is what made the two residuals look four times further apart
than they are. `[M3-08:01]` records the correction.

### What each read is made of

| pull | cycles |
| --- | ---: |
| rebuilding the registered roster | 898 |
| `verify_price` over five packages at a threshold of three | 3,040,529 |

| push | cycles |
| --- | ---: |
| deriving the price account's address | 3,654 |
| `OraclePriceAccount::try_from_slice` | 1,575 |
| `LezClock::from_account` | 267 |
| the four remaining checks | the remainder, 1,307 |

The pull side's `verify_price` figure is within 739 cycles of the 3,039,790 the
component table gives for the same five-signer payload, and that agreement is
worth stating: it means the pull read is an update's verification and not
something adjacent to it. The 739 is one clock decode and the library call
around it.

**The largest item in a push read is hashing**, which is worth saying plainly
because an earlier draft of this file said the mode hashed nothing. `read_price`
opens by checking that the account it was handed is the one this feed publishes
to, and computing that address means `compute_pda` twice — once for the feed
account, once for the price account derived from it — each a SHA-256 over its
seeds. At 3,654 cycles that is 54% of the read, more than twice the account's
decode, and it is why the consumer's guest manifest pins `sha2` to the risc0
accelerator. What push mode avoids is signature *recovery*, not cryptography.

The four checks left in the remainder are the aggregator's ownership of the
account, the `source_id`, the asset pair, and the staleness window in both
directions. `read.rs` names all five in the order they run, cheapest refusal
first, and the address is first because everything after it is a statement about
the wrong account otherwise.

### The operations each residual bundles

`2 - 1` is a residual, so the same discipline the write section uses applies:
each operation inside it is measured on its own, and none of them is called a
component.

| measured in isolation | pull | push |
| --- | ---: | ---: |
| the consumer's own registration (`trust::read` / `source::read`) | 3,385 | 1,839 |
| the order's decode | 2,034 | 2,021 |
| filling the order and encoding it | 2,917 | 2,838 |

**These do not sum to the residual, and are not meant to**, for the reason M2-18
recorded: isolated calls do not cost what inlined ones do. The pull residual is
7,275 cycles and the three figures above it total 8,336, which is not a
contradiction — `settle` inlines what a separate call cannot.

Two of the three rows agreeing across modes to within about ten cycles is the
expected result rather than a coincidence, though not because the type is shared:
the two consumers own separate `OrderAccount` types, and the pull one carries a
`decimals` the push one does not, because a pull consumer has to know the scale
a payload's integer is on and a push consumer is handed a `Q64.64` already. What
M3-05 and M3-06 hold in common is the *shape* — the same seven fields of the same
widths, plus that one byte — which is why the decode and the write price out
within a few cycles of each other. Where the two genuinely differ is `trust::read`
against `source::read`, and the 1,546-cycle difference is a `FeedTrust` carrying a
signer roster and a data service label against a `PriceSource` carrying five
fixed-width fields.

### The elision trap, again

The published account's decode first measured **45 cycles**, which is not a
plausible price for reading 136 bytes — the component table gives 1,442 for the
same decode. The stage returned one field of the decoded value, so the optimiser
elided every other field. Observing the whole value through
`core::hint::black_box` gives 1,575. The order decodes moved the same way, from
1,879 to 2,034.

The whole-`settle` stages had the same hole and it went unnoticed longer, because
the figure it produced was not implausible. Those stages returned only
`post_states.len()`, so every field of every returned account was a value nothing
read, and an optimiser is entitled to skip building what never escapes. They now
observe the post-states themselves before taking the length. It is the same fix
in the stage that dominates the table, which is the one place a silent elision
would have been worth the most.

This is the third time this file records that failure and the third time
`black_box` is the fix rather than a checksum. A figure too cheap to be plausible
is the same signal as a test that cannot fail — and a figure that is merely
*plausible* is not evidence that the value was kept alive.

### What is not in these figures

The same exclusions the write section lists — the generated validator, the
dispatcher, the `SpelOutput` wrapping — and one that matters more on the pull
side than anywhere else: **what LEZ spends reading the instruction data before
the program is entered**, about 113 cycles a byte. A pull settlement carries the
whole payload as instruction data, so a 1,200-byte capture is roughly 136,000
cycles of reading before `settle` begins, against a push settlement's feed id.
ADR 26 bounds it and `MAX_PAYLOAD_BYTES` is the limit; it is named here because
a per-read comparison that omitted it would understate the pull side.

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
cargo test --release -p kanon-methods --test read_cost \
    -- --nocapture the_read_table                                          # the third
```

Every table is printed by the code that asserts it, so a published figure and an
asserted figure cannot drift apart.
