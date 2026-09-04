# [M3-08:01]. What a read costs is two figures per mode

- **Status**: accepted
- **Milestone**: M3 (`M3-08`)
- **Requirements**: P2, P3
- **Artefacts**: `reference-consumers/pull/guest/src/bin/pull_cost.rs`,
  `reference-consumers/aggregator-read/guest/src/bin/read_cost.rs`,
  `methods/tests/read_cost.rs`, `COSTS.md`

## Context

P2 asks for cost measurement "per mode". M1-25 measured verification, M2-18 the push
write and a registration. What neither answers is the question a consumer author asks:
**what does it cost me to obtain a price?** In push mode that is an account read. In pull
mode there is no published account, so it is a verification — and the two therefore
differ by a factor of several hundred rather than by a tuning margin. P3's per-mode
precompile delta rests on both figures existing, which is why this task precedes it.

Three things had to be decided before a number could be published, and all three are
decisions about *what to measure* rather than about how.

## Decision 1: two figures per mode, not one

**Each mode publishes the mode's own read and the whole `settle` body it sits inside.**

The reference consumers are the only programs in this repository that perform a read, and
each holds a limit order. That order is not the mode's: `open_order`, the limit
comparison and the order account are M3-05 and M3-06's domain, chosen to be thin so the
two consumers differ in one thing. A single figure taken from `settle` would therefore
publish a mode's cost with a reference consumer's toy trade folded into it, and a
consumer author comparing modes would be comparing our order book.

So the published per-mode figure is the read itself — `pull_lib::verify_price` on one
side, `read::read_price` plus the clock decode it needs on the other — and the `settle`
body is published beside it, named as what a read costs inside a program that also does
something with the price.

*Rejected: measure only the library call.* It is the portable number, but a figure with
no program around it invites the reply that a real consumer pays more, and there would be
nothing published to answer with. The residual between the two is small on both sides
(7,275 and 6,479 cycles) and that smallness is itself worth publishing: it says the
domain is not where a read's cost goes.

*Rejected: measure only `settle`.* Publishes our limit order as the mode's price.

## Decision 2: the read is what the mode forces, and the setup absorbs the rest

**A stage 1 arm performs only the mode's own read; everything a consumer holds before it
reads is decoded in the setup that cancels.**

The two modes' reads are compared to each other, and the residuals beside them are
compared to each other, so what a stage 1 contains has to mean the same thing on both
sides. The first draft did not manage that. Pull's stage 1 decoded the `FeedTrust` *and*
the reference consumer's `OrderAccount`, because `verify_price` needs the asset pair off
the order; push's decoded the `PriceSource` and no order at all. So one mode charged its
limit order to the read and the other charged the same work to the residual, and the two
residuals — published side by side, one line apart — were measuring different things.
The size of the error is not marginal: the order decode is about 2,000 cycles and the
registration decode about 1,800, roughly a fifth of the whole push figure.

Both decodes therefore moved above the branch. A setup runs identically in every stage,
so it cancels out of every difference, and what remains in stage 1 is the roster rebuild
and `verify_price` on one side, the clock decode and `read_price` on the other — which is
what Decision 1 says a read is. Corrected, the residuals are 7,275 and 6,479 rather than
2,025 and 5,163: close together, which is the result the argument predicted and the first
draft's numbers contradicted.

A consequence worth naming: stage 1 is **not a prefix of `settle`'s execution**, and no
longer claims to be. The real settlement decodes the order first, reaches the registration
through `trust::read`'s guards, and checks the order's terms before it reads a price at
all. Stage 1 is a measurement of one operation over values already in hand. `2 - 1` is
still what settling costs beyond reading, because subtraction does not care what order the
work was done in, but "the same calls in the same order" was a claim the code did not
support and it has been removed from both guests.

*Rejected: decode inside stage 1 on both sides, symmetrically.* It equalises the error
instead of removing it, and publishes a "mode's read" that includes the reference
consumer's toy trade — which is the thing Decision 1 exists to keep out.

## Decision 3: the measurement guests go in the consumers' own workspaces

**Each cost guest is a binary in the guest workspace of the consumer it measures, and
neither adds a dependency edge to that workspace.**

The first half follows M1-25's reasoning (ADR 20): a guest workspace is the only place
its own `[patch.crates-io]` applies, and a cycle figure measured under different patches
is a figure for a different program. The pull consumer's manifest already committed to
this, pinning its patch set so "the pull-side cycle figures (M3-08)" stay comparable with
the push side's.

The second half is the part that was not obvious, and it was found by measuring rather
than by reasoning. `pull_cost.rs` needs `verify_price`, which lives in `pull-lib`, and the
consumer's guest workspace does not depend on it directly. Adding `pull-lib` and `borsh`
to that manifest compiled, ran, and **moved `PULL_CONSUMER_ID` from `3055…` to `3559…`**.

That is not a cosmetic change. A SPEL program's id *is* its RISC Zero image id, every
account the program owns is a PDA derived from it, and `[M3-05:01]` records what happens
when it moves: the old accounts go dead, every open order is abandoned, and there is no
migration path. A measurement binary is not a reason to pay that.

So the two crates re-export what their guests need instead — `pub use pull_lib` on one
side, `pub use kanon_clock` and `pub use kanon_idl` on the other — and the guests reach
them through the consumer they already depend on. Re-measured: all three product image
ids identical to the byte, and no guest lockfile moved.

The re-exports are not a measurement artefact, which is what makes this the right fix
rather than a workaround. `read_price` takes a `&LezClock` in its public signature, so
before this a caller could not name a type the API requires. `pull-lib` re-exports
`verifier_core` for exactly that reason, and says so.

*Rejected: a separate guest workspace per cost binary.* Keeps the consumers' manifests
untouched, at the price of two more workspaces, two more lockfiles and two more entries
in `methods/Cargo.toml`, to avoid an edge the re-exports avoid for nothing.

*Rejected: measuring from the host.* The figures would be x86 cycles, which are not the
unit LEZ charges in and not comparable with anything else in `COSTS.md`.

## Consequences

- **The headline figure is a ratio, and it is asserted.** A pull read is 3,041,427
  cycles against a push read's 6,803 — 447×, and
  `a_pull_read_costs_hundreds_of_times_more_than_a_push_read` pins it.
  `the_gap_between_the_modes_is_signature_recovery` pins the explanation: five-signer
  recovery is 96% of the difference, so the gap is one component rather than a pile of
  small ones. It is 96% and not all of it, and both this file and `COSTS.md` say so:
  the remaining 4% is the rest of verification.
- **`verify_price` agrees with the component table to within 739 cycles**, which is the
  cross-check that the pull read really is an update's verification rather than something
  adjacent to it. Those 739 are one clock decode and the library call.
- **Push mode's read is mostly hashing, and an earlier draft said it hashed nothing.**
  `read_price` checks the account's address first, which derives two PDAs, each a SHA-256
  over its seeds: 3,654 cycles, 54% of the read and more than twice the account's decode.
  It is now its own measured stage rather than part of a remainder attributed to "the
  checks". What push mode avoids is signature recovery, not cryptography — which is also
  why the consumer's guest manifest pins `sha2` to the accelerator.
- **Two observation holes were found, and the second was the expensive kind.** The
  published decode reported 45 cycles for 136 bytes because the stage returned one field;
  `core::hint::black_box` over the whole value gives 1,575. The whole-`settle` stages had
  the same hole and hid it better, returning only `post_states.len()` so that every field
  of every returned account was a value nothing read. That one produced a *plausible*
  figure, in the stage that dominates the table, which is what makes it the more
  instructive of the two: implausibility is a signal that a figure is elided, but
  plausibility is not evidence that it is not.
- **What LEZ spends reading instruction data is outside every figure here**, and it falls
  asymmetrically: a pull settlement carries the payload, at about 113 cycles a byte, so a
  1,200-byte capture is roughly 136,000 cycles before `settle` is entered. A push
  settlement carries a feed id. ADR 26 bounds it; `COSTS.md` names it so a per-read
  comparison is not read as complete without it.
- **P3 now has both baselines it needs.** A precompile removes recovery, which is 96% of
  a pull read and none of a push read, so the delta is per-mode for a structural reason
  rather than a presentational one.
