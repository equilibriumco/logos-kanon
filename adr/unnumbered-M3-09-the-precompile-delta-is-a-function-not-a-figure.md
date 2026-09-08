# [M3-09:01]. The precompile delta is a function, not a figure

- **Status**: accepted
- **Milestone**: M3 (`M3-09`)
- **Requirements**: P3
- **Artefacts**: `methods/tests/cost.rs`, `methods/tests/read_cost.rs`, `COSTS.md`

## Context

P3 asks for the cost delta between the in-program path and a hypothetical native
ECDSA and keccak256 precompile, reported separately per mode. The requirement exists
because RFP-020 wants to know whether LEZ should gain those primitives at all, so the
reader is the party who would build them.

What a precompile would remove and what would remain are both measured, though one of
them is measured in a different guest from the one it is subtracted from — `verify_cost`'s
component table prices the removable rows, and they come out of `submit_cost` and
`pull_cost`. The consequence is a residual good to tens of cycles rather than to one,
and `COSTS.md` says so where it publishes them. `cost.rs` prices keccak256
and recovery per signer count, which is what a precompile removes; the difference is
what remains; and M3-08 supplies the per-mode baselines the reduction is taken against.
The fourth term is what a call into a precompile costs, and no such precompile exists in
LEZ to measure.

What is not measured is the primitive itself, in three ways rather than one. What a
call costs is the obvious one. How many calls there are is the second, and it depends
on what the primitive returns: five packages need ten crossings if it hands back an
address, as the EVM's `ecrecover` does, and fifteen if it hands back a public key and
leaves the caller to hash it, which is what `address_of` does today. The third is what
it validates, because the row being removed includes parsing the signature and refusing
a malleable one, and a primitive that did neither would not remove all of it.

Nothing here is blocked on either. A delta against a hypothetical is still a delta —
`m0` published one for the push update path under the same conditions — so the question
is what to do about the terms nobody can measure.

## Decision 1: the delta is published as a function of the syscall cost

**Every figure is given at both ends of `m0`'s 1,000-to-10,000 cycles-per-call band and
at zero, and no single number is published.**

The band is not a rounding margin. A pull read comes to 41,115 cycles at the bottom of it
and 131,115 at the top, a **74.0x reduction against a 23.2x one** — the answer moves by a
factor of three on the term that is assumed. Collapsing that into one figure would publish
our guess about somebody else's engineering as though it were a measurement, in a document
whose whole claim is that its figures are measured.

Zero is included and labelled unachievable, because it is the one bound that survives
whatever a real precompile costs: below 31,115 cycles a pull read's *body* cannot go,
however free the crossing.

That is a bound on a body, and a body is a fraction of a transaction. M3-10 measures the
whole thing by running the product ELF over the inputs LEZ would hand it: a pull
settlement is 3,513,718 cycles against a 3,048,613-cycle body, the difference being what
LEZ spends reading the inputs and what the dispatcher, the generated validator and the
`SpelOutput` wrapping spend around it. A precompile touches none of that.

So the reduction a reader scoping the work should have in mind is **about 6x to 7x on a
settlement**, against the 22x to 80x these bodies give. `COSTS.md` publishes both with
the arithmetic between them, and `[M3-10:01]` has the measurement. An earlier draft of
this ADR put the transaction figure at 14x to 25x by adding an estimate of the
instruction read to the body; the estimate was low and it omitted everything except the
instruction.

`m0`'s band is reused rather than re-derived so the two reports can be read together. It
was an assumption there and it is an assumption here — and one band is charged to both
primitives, which is a further simplification: `m0` derived it for a *recovery* syscall
and left hashing in software. Written out the addition is `5·c_keccak + 5·c_recovery`,
or `10·c_keccak + 5·c_recovery` for a primitive returning a public key. Collapsing it to
a single `c` publishes one invented band instead of two.

*Rejected: pick a single plausible `c` and publish one reduction.* Reads better and is
worth less. The reader is choosing `c`, so handing them a figure that already contains
one of our own invites them to recompute it and wonder what else was assumed.

*Rejected: measure a risc0 accelerator syscall and use that as `c`.* It would narrow the
band with a real measurement of the wrong mechanism — an accelerator syscall inside the
zkVM is not a LEZ-native precompile — and would trade an honest parameter for a precise
irrelevance.

*Rejected: wait for the precompile.* P3 asks for the delta against a hypothetical. There
is nothing to wait for, and a milestone spent waiting would deliver the same analysis
later.

## Decision 2: "per mode" is answered with one saving and two frequencies

**The saving is published once, and the modes are distinguished by how often each pays
it.**

A five-package verification is five hashes and five recoveries wherever it runs, so a
precompile removes 3,010,270 cycles — on the assumption that it absorbs the signature
parsing and the malleability refusal inside the row being removed, without which that
figure is an upper bound — and adds ten crossings, in both modes. Reporting that
twice, once per mode, would suggest the arithmetic differs. It does not.

What differs is frequency, and it is the whole of P3's per-mode answer. A pull consumer
verifies on every read, so it collects the entire saving per read. A push consumer
verifies never — its read is cheaper than one recovery — so it collects nothing, and the
saving lands on the aggregator's update instead, once per update however many times the
price is read. For a feed read `R` times between updates, **a precompile is worth `R`
times more to a pull consumer than to a push one.**

That framing also makes the null result legible. "A precompile is worth nothing to a push
read" is the kind of claim that reads as an oversight, and it is the correct answer: the
mode's read has no signature to recover. `a_precompile_is_worth_nothing_to_a_push_read`
asserts it by showing the read costs less than a single recovery, rather than by asserting
that zero equals zero.

*Rejected: report two independent per-mode deltas.* Symmetrical, and it hides that the
modes share the arithmetic and differ only in amortisation — which is the finding.

## Consequences

- **No new cycle measurement was executed.** Every cycle figure is one `cost.rs` or
  `read_cost.rs` already pins, and the delta's arithmetic is asserted in the files that
  own those figures rather than in a third that would re-run the same guests to learn the
  same numbers. One thing did have to be measured: `wire.rs` sizes the instruction a
  settlement carries, because the read in front of the body is computed from it and two
  successive drafts got that size wrong by arithmetic.
- **The recommendation is one-sided for the packages we measured, and that is the useful
  part.** If one of the two is built, it is ECDSA — for payloads shaped like the captured
  ones. Hashing scales with a package's signable span and recovery does not, so a feed
  publishing enough data points per package would move the balance, and nothing here
  bounds where; every captured package carries one point in 77 bytes. Within that shape,
  the figure to scope the other half from is not the 2.90% the component table's keccak
  row shows: the recovery row contains a second keccak256, because `address_of` hashes
  the recovered point to an Ethereum address, and nothing isolates that hash. Taking it
  at the message hash's measured 17,476 puts the primitive split at roughly 5.8% keccak
  against 94.2% ECDSA. The keccak side is
  weaker still than that, because `the_keccak_accelerator_is_still_declined` records that
  risc0's coprocessor would divide that row by about seven today with no LEZ change at
  all (ADR 6) — worth about 150,000 cycles, not the 75,000 a row split implies.
- **The call count is an assumption about the interface, not a measurement.** Two
  crossings a package assumes the precompile returns an address, as the EVM's `ecrecover`
  does; one returning a public key leaves the caller to hash it and costs three, which
  moves the top of the band from 23.2x to 16.8x. Stated where the band is stated, because
  it is the reader's to choose.
- **With the cryptography gone the two residuals are about equal, and that is as far as
  it goes.** A pull settlement leaves 38,343 cycles and a push update 38,144. The 199
  between them is not a finding: they are residuals from different guests, and the
  disagreement those guests show on identical verification is 42 and 722 cycles, so 199
  is inside the noise rather than a measurement of anything. They are also not the same
  operation — a settlement against a submission — so even an exact agreement would not
  show the modes converging. What can be said is the weaker and still useful thing: once
  the cryptography is gone, neither mode's remaining work is large, and the three orders
  of magnitude between them today are one primitive rather than a structural difference
  in what they do.
- **A push read keeps its hashing.** Its largest item is 3,656 cycles of SHA-256 for the
  PDA derivation ([M3-08:01]), and neither primitive RFP-020 names reaches SHA-256, so
  the majority of a push read survives every precompile under discussion.
- **P3 stays partial.** The delta is delivered; the per-chain reference points it is
  compared against are M5-07's.
