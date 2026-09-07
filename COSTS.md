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
program is entered, about 113 cycles a serialized `u32` word — which is about a
cycle count per *payload* byte, since serde writes one word per byte, and is not
113 per byte of the encoded instruction. It *is* material and is
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
| **the mode's read** | **3,041,385** | **6,803** |
| settling, beyond the read | 7,228 | 6,601 |
| the whole `settle` body | 3,048,613 | 13,404 |
| _harness floor, subtracted out_ | 147,529 | 73,332 |
| _the read, against LEZ's per-transaction budget_ | _9.06%_ | _0.0203%_ |
| _the `settle` body, against the same budget_ | _9.09%_ | _0.0399%_ |

**A pull read costs 447 times a push read.** Not a tuning difference — the pull
consumer has nothing published to fetch, so its read *is* a verification, and it
pays that on every read. The push consumer reads six fields the aggregator
already verified once, and recovers no signatures.

96% of the gap is signature recovery, and nothing else in it comes close.
`cost.rs` measures five-signer recovery at 2,922,890 against a difference of
3,034,582, and `the_gap_between_the_modes_is_signature_recovery` asserts that
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

| pull, as prefixes | cycles |
| --- | ---: |
| rebuilding the registered roster | 873 |
| `verify_price` over five packages at a threshold of three | 3,040,512 |

These two are exact, and add to the read, because the roster rebuild is a
genuine prefix of the read: one stage runs it and stops, the next runs it and
carries on, so the difference is the verification and nothing else.

The push side has no such split — `read_price` is one call — so its parts are
measured on their own, and **isolated figures do not decompose anything**:

| push, each measured on its own | cycles |
| --- | ---: |
| deriving the price account's address | 3,656 |
| `OraclePriceAccount::try_from_slice` | 1,551 |
| `LezClock::from_account` | 265 |

They come to 5,472 of the 6,803, and the 1,331 left over is *not* a row: it is
the four remaining checks plus whatever an inlined call costs less than an
isolated one. Naming it "the four checks" would be the mistake this file warns
about two paragraphs below and the ADR records removing elsewhere. What the
three figures support is the weaker claim they are here for, which is that the
address derivation dominates and no arrangement of the checks would matter next
to it.

The pull side's `verify_price` figure is within 722 cycles of the 3,039,790 the
component table gives for the same five-signer payload, and that agreement is
worth stating: it means the pull read is an update's verification and not
something adjacent to it. The 722 is one clock decode and the library call
around it.

**The largest item in a push read is hashing**, which is worth saying plainly
because an earlier draft of this file said the mode hashed nothing. `read_price`
opens by checking that the account it was handed is the one this feed publishes
to, and computing that address means `compute_pda` twice — once for the feed
account, once for the price account derived from it — each a SHA-256 over its
seeds. At 3,656 cycles that is 54% of the read, more than twice the account's
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
| the order's decode | 2,031 | 2,025 |
| filling the order and encoding it | 1,024 | 1,022 |

**These do not sum to the residual, and are not meant to**, for the reason M2-18
recorded: isolated calls do not cost what inlined ones do. The pull residual is
7,228 cycles and the three figures above it total 6,440, which is not a
contradiction in either direction — inlining can cost less than a call, and
`settle` also does things none of these three rows measures.

The write row is the write only. It was 2,917 and 2,838 in an earlier draft
because the stage decoded the order before filling it, so a row published as the
write also contained the row above it — the same double-attribution the read
stages were corrected for, one table further up. Both now fill and encode an
order the setup already decoded.

Two of the three rows agreeing across modes to within a handful of cycles is the
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
`core::hint::black_box` gives 1,551. The order decodes moved the same way, from
1,879 to 2,031.

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
dispatcher, the `SpelOutput` wrapping — and **what LEZ spends reading the inputs
before the program is entered**. Both are measured, in *What a transaction costs,
whole* below, and together they are most of a push read's transaction and 13% of
a pull settlement's. They are named here because a per-read comparison that
omitted them would understate the pull side, and because the figures above are
bodies: what a mode's code costs, not what a transaction costs.

## What a precompile would be worth, in each mode

P3 asks for the delta between the in-program path and a hypothetical native
ECDSA and keccak256 precompile, per mode. LEZ has no such primitive, so **the
figures rest on three things about it that nobody can measure: what a call into it
costs, what it hands back, and what it validates** (M3-09). What the third moves is
the delta itself: the tables below assume the primitive absorbs the signature
parsing and the malleability refusal that `recover_signer` does today, and if it
does not then 3,010,270 is an upper bound on what is removable and whatever is
retained is unmeasured. The other two move the band around the delta and how it
splits between the primitives, and that split rests on an estimate of its own, named
where it is used. What a
precompile would remove is two rows of the component table above; what would
remain is the difference; what it would add is a crossing into the precompile
and back, once per call. That last figure does not exist to measure, so it is
carried as `c` and every number below is given at both ends of `m0`'s
1,000-to-10,000 band and at zero.

Zero is not achievable and is not meant to be. It is the bound that holds
whatever a real precompile costs, which is the useful thing to hand someone
deciding whether to build one.

### The saving is the same in both modes. The frequency is not

A five-package verification does five keccak256 hashes and five recoveries, so a
precompile removes **3,010,270 cycles and adds ten crossings**, wherever that
verification happens.

Those ten are charged one `c` each, which is an assumption and not a small one: five
are keccak256 and five are recovery, and `m0` derived the band for a *recovery*
syscall while leaving hashing in software. Written out, the addition is
`5·c_keccak + 5·c_recovery`, and `10·c_keccak + 5·c_recovery` if the primitive hands
back a public key. This section collapses that to `10·c` because two bands would
publish twice as much invented precision, not because the two crossings are known to
cost the same. That is the same arithmetic in both modes, and it is not
where the modes differ.

They differ in how often a consumer pays it:

| a read | pull | push |
| --- | ---: | ---: |
| today | 3,041,385 | 6,803 |
| removable | 3,010,270 | 0 |
| with a precompile, `c = 0` | 31,115 (**97.7x**) | 6,803 (1x) |
| with a precompile, `c = 1,000` | 41,115 (**74.0x**) | 6,803 (1x) |
| with a precompile, `c = 10,000` | 131,115 (**23.2x**) | 6,803 (1x) |

**A pull consumer pays the whole saving on every read. A push consumer gets
nothing.** Not a small amount — nothing, because a push read performs no
recovery at all: it is cheaper than a single one, which
`a_precompile_is_worth_nothing_to_a_push_read` asserts rather than asserting an
arithmetic identity.

Push mode's saving is real, but it lands on the aggregator's update:

| a push update | cycles |
| --- | ---: |
| today | 3,048,414 |
| removable | 3,010,270 |
| with a precompile, `c = 0` | 38,144 (**79.9x**) |
| with a precompile, `c = 1,000` | 48,144 (**63.3x**) |
| with a precompile, `c = 10,000` | 138,144 (**22.1x**) |

So for a feed updated once and read `R` times before its next update, a
precompile is worth `3,010,270 - 10c` per read in pull mode and the same figure
divided by `R` in push mode. **It is worth `R` times more to a pull consumer**,
and that is P3's per-mode answer: one number, two frequencies.

With the cryptography gone, a pull settlement leaves 38,343 and a push update
38,144. Read that as "about the same size" and no further:
`the_precompile_read_table_is_reproducible` prints both so they can be checked, but
the 199 between them is smaller than the 722 cycles those guests disagree by on
identical verification, and the two are different operations besides — a settlement
against a submission. What it supports is that neither mode's remaining work is
large, not that the modes converge or why. What is
left in both cases is the framework and each program's own domain.

### These are bodies, and a settlement is several times its body

A settlement is not a `settle` body. M3-10 measures the whole transaction by
running the product ELF over the inputs LEZ would hand it: **3,513,718 cycles**,
against the 3,048,613 the body costs. The difference is what LEZ spends reading
the inputs and what the dispatcher, the generated validator and the `SpelOutput`
wrapping spend around the body — and a precompile touches none of it.

So the reduction a precompile buys on a settlement is much smaller than the body
figures suggest:

| a pull settlement, whole | cycles | reduction |
| --- | ---: | ---: |
| today | 3,513,718 | |
| with a precompile, `c = 0` | 503,448 | **7.0x** |
| with a precompile, `c = 1,000` | 513,448 | 6.8x |
| with a precompile, `c = 10,000` | 603,448 | 5.8x |

**About 6x to 7x on a transaction, against 22x to 80x on the body.** An earlier
draft of this section put it at 14x to 25x by adding an estimate of the
instruction read to the body; that estimate was low and it omitted everything but
the instruction. The figures above rest on a measurement instead, and
`a_precompile_is_worth_less_against_a_transaction_than_against_a_body` asserts and
prints them.

Like every difference between a transaction and a body here, this subtracts a
figure measured in `verify_cost` out of one measured in the product ELF, so it
carries their disagreement as well — 42 to 722 cycles where this file has measured
it, against a residual of half a million.

The band all but closes at transaction level, because the term that varies with
`c` is small beside the terms that do not.

### If only one of the two is built, build ECDSA

Of the 3,010,270 a precompile removes, the **recovery row is 97.10% and the
keccak256 row 2.90%**, at one, three and five signers alike — so it does not depend
on how many packages a payload carries.

It does depend on what is *in* them. Verification hashes each package's whole
signable span, so a package carrying more data points hashes more without recovering
more, and the keccak share rises with it. Every captured package carries exactly one
data point and a 77-byte span, and that is the shape this split is for. A feed
publishing several points per package would move it toward keccak, and nothing here
measures by how much.

**Those are rows, and rows are not primitives.** The recovery row is everything
`recover_signer` does: parsing the 65-byte signature, refusing a malleable one, the
recovery itself, encoding the point, and then a *second* keccak256 — `address_of`
hashes the 64-byte uncompressed point to get an Ethereum address. So the keccak row
is not all of the hashing.

**How much more is an estimate, and it is the one figure in this section that is not
measured.** Nothing isolates the address hash. Taking it at 17,476 borrows the
measured cost of the 77-byte message hash, on the reasoning that both fit inside one
136-byte keccak block. On that estimate the removable 3,010,270 is roughly **174,760
of keccak256 and 2,835,510 of ECDSA proper, about 5.8% against 94.2%**, rather than
the 2.90/97.10 the rows give. Measuring that hash on its own would settle it, and
nothing here does.

Underneath it sits a third assumption about the interface, beside the two above:
what the primitive validates. The row being removed includes signature parsing and the
malleability check, and one that skipped either would not remove all of it.

The recommendation survives the *estimate* — ECDSA is the larger half whether the
address hash is counted with it or against it. It does not survive an arbitrary
package shape: hashing grows with the signable span and recovery does not, so a feed
publishing enough data points per package would move the balance, and nothing here
establishes where. Read "build ECDSA first" as scoped to packages the shape of the
captured ones, which is the shape RedStone's primary feeds publish today.

The hashing side is weaker still than even 5.8% suggests, because most of it is
already available without LEZ changing anything. `the_keccak_accelerator_is_still_declined`
records that this build declines risc0's keccak coprocessor, which would divide that
row by about seven (ADR 6) — worth roughly 150,000 cycles on the primitive split
rather than the 75,000 the row split implies. A keccak256 precompile would be
competing with a configuration change.

### What is measured and what is not

Measured: what a precompile removes, and what remains — on the assumption that the
primitive absorbs the parsing and the malleability refusal inside the row being
removed. Both are rows this file already publishes, and the tests that pin them are
the same tests. The one new measurement is the size of the instruction a settlement
carries, which `wire.rs` pins.

Two caveats on how precisely, because both residuals are ~38K left after
subtracting two figures near 3M. The removable rows are measured in `verify_cost`
and subtracted from `submit_cost` on the push side and from `pull_cost` on the
pull side, so each residual inherits whatever those guests disagree by. That is
42 cycles between `verify_cost` and `submit_cost`, which
`both_harnesses_agree_about_what_verification_costs` pins, and 722 between
`verify_cost` and `pull_cost` for the same payload. Both residuals are therefore
good to a few tens or hundreds of cycles rather than to one — enough for a
reduction factor, and not enough to read the 199 cycles between the two residuals
as more than "about the same size".

Assumed: `c`, and one thing about the precompile's shape. Ten calls assumes it
returns an **address**, as the EVM's `ecrecover` does. One returning a public key
leaves the caller to keccak the point — which is what `address_of` does today — and
costs three crossings a package rather than two, putting a pull read at 181,115
rather than 131,115 at the top of the band, 16.8x rather than 23.2x. That is the
reader's assumption to overturn, since they are the ones choosing the interface.

And what it validates, which is the one that moves the delta rather than the band:
the tables assume the primitive absorbs the parsing and the malleability refusal
inside the row being removed, and 3,010,270 is an upper bound if it does not.

Beyond those three: nothing. `c` matters more than an assumption usually would,
because the answer moves by a factor of three across the band — 23.2x to 74.0x
on a pull read — which is why no single number is published here. `[M3-09:01]`
records that choice. `m0`'s report is not directly comparable and it is worth
saying why, because all three of its terms differ: its 89% is *recovery alone*
against the whole 3-of-N program, with keccak a separate 2.72% left in software,
over 3 packages and so 3 calls. The figures here are recovery **and** keccak
against verification only, over 5 packages and so 10 calls. Its 8x to 9x and this
section's 23x to 74x are answers to different questions, and the gap between them
is mostly the framework floor and the per-package instruction handling that a
crypto precompile does not touch — but not only that.

Not covered: what a precompile would do to proof size or proving time, and the
per-chain reference points the delta is compared against, which are M5-07's.

The input read *is* covered, and exactly: M3-10 measures it rather than estimating
it, at 113 cycles a word to the cycle. What remains approximate is the split
between a transaction and its body, because that subtraction crosses two ELFs and
so carries their disagreement — tens to hundreds of cycles against residuals in
the hundreds of thousands.

## What a transaction costs, whole

Every other figure in this file is a program **body**, and a body is a fraction of
a transaction. LEZ reads the inputs before any body is entered — `read_lee_inputs`
takes the program's own id, the caller's, the pre-state accounts, then the
instruction words — then the dispatcher decodes the instruction into the program's
own enum, the generated validator checks the accounts, the body runs, and
`SpelOutput` wraps the result. None of that is reachable by calling a function.

It is reachable by running the program, which is what M3-10 does: the product ELFs
over the inputs LEZ would hand them, with the accounts at their real derived
addresses.

| | to be read | body | the rest | **transaction** | of budget |
| --- | ---: | ---: | ---: | ---: | ---: |
| pull settlement | 184,456 | 3,048,613 | 280,649 | **3,513,718** | 10.47% |
| push update | 175,826 | 3,048,414 | 259,469 | **3,483,709** | 10.38% |
| push read | 122,783 | 13,404 | 188,480 | **324,667** | 0.97% |

Every body here is a `settle` or `submit_price` body, because every transaction
here executes one. The push read's is 13,404 — the whole settlement, not the 6,803
read inside it, which is what `read_cost.rs` compares against a pull read.

"The rest" is taken as the difference, so it is the dispatcher, the validator, the
instruction's decode and the wrapping **plus whatever the three harnesses disagree
by**: the transaction comes from the product program, the body from a cost guest
and the read from `input_cost`, and this file already records two of those
differing by 42 and 722 cycles on identical verification. At 188,480 to 280,649
that variance is a rounding error inside it, and the column is still a difference
between measurements rather than a measurement of its own. Only a product ELF that
bracketed its own body would make it one.

The reads and the transactions are measured by `bytes.rs`. The pull and push-read
bodies are executed there too, against the same guests and stages `read_cost.rs`
pins, so the two files cannot disagree silently. The push update's body is
`cost.rs`'s: its guest brackets that body differently and `bytes.rs` does not
re-execute it, so that one row is a figure carried between files rather than
checked across them.

### This changes the per-mode headline rather than qualifying it

A pull settlement's body is **227 times** a push settlement's. As executed
transactions the two are **ten times apart**. A push settlement is 13,404 cycles
of body inside a 324,667-cycle transaction — **four percent of it** — because
everything else is being read and being dispatched, and neither scales with what
the body does.

Both sides here are settlements. The 448 the read tables publish is a read against
a read, which answers what the two modes' *reads* cost and does not belong beside
a ratio of transactions.

Both figures are published because they answer different questions. What a mode's
*code* costs is the read tables above, and it is what a consumer author comparing
implementations wants. What a *transaction* costs is this, and it is what somebody
sizing a chain wants. Quoting the first as though it were the second is the error
this section exists to prevent, and an earlier draft of the precompile section
made it.

### Where the reading goes

The read half is measured separately, because it is the half a program can bound
and the rest is the framework's:

| | |
| --- | ---: |
| a read of four empty inputs | **4,426** |
| an instruction word | **113** |
| a byte of an account's data | **113** |
| an account, on top of its data | **~16,156** |

The first row is a run of the read guest, so it carries what any guest run costs —
zkVM startup and the journal commit — as does every per-mode read figure above.
That floor cancels when two of them are compared and does not when one is quoted
alone; what it bounds is a transaction carrying nothing. The marginal rates below
it are unaffected, because differencing two input sizes cancels the whole term.

The 113 is exact and linear to the cycle from 1,024 words to 16,384, which
confirms ADR 26's "about 113" and settles its unit: it is a **word**, so
multiplying a serialised size in bytes counts each word four times.

The other two rows are why the transaction figures are measured rather than
assembled. Four framed reads cost something with nothing in them, and an account
costs far more than its width — deserialising an `AccountWithMetadata` builds a
`Data`, an `AccountId` and a `ProgramId` rather than copying a span. A word count
times a rate understates every mode, and the push read by nearly half.

| | account words | instruction words |
| --- | ---: | ---: |
| pull settlement | 496 | 758 |
| push update | 454 | 726 |
| push read | 562 | 33 |

### What is still outside

Proving. Everything above is execution, which is what LEZ's per-transaction
budget is denominated in.

And the push consumer's instruction is counted from a rule rather than encoded,
because its enum is generated inside its guest and there is no host-side type to
serialise (`[M3-06:01]`): one word for the variant tag and thirty-two for its
`feed_id`. The rule is checked against the pull consumer's real `Settle`, which is
a tag, a feed id and a length-prefixed payload and comes to exactly
`1 + 32 + 1 + 724`.

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

The recovery gap is 19,725 cycles, and it is what the M0 harness did not include:
`recover_signer` parses the 65-byte signature, refuses a malleable one, and hashes
the recovered point to an address. Nothing here isolates those, so how the 19,725
divides between them is not established — the address hash is a keccak256 over 64
bytes, and the message hash over 77 measures 17,476, which is the nearest thing to
a figure for it and is a stand-in rather than a measurement.

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
cargo test --release -p kanon-methods -- --nocapture the_precompile_table  # the update delta
cargo test --release -p kanon-methods --test read_cost \
    -- --nocapture the_precompile_read_table                               # the read delta
cargo test --release -p kanon-methods --test bytes \
    -- --nocapture the_boundary_table                                      # what a transaction reads
```

Every table is printed by the code that asserts it, so a published figure and an
asserted figure cannot drift apart.
