# [M3-10:01]. A transaction is not its body, and the difference is measured

- **Status**: accepted
- **Milestone**: M3 (`M3-10`)
- **Requirements**: P1, P2
- **Artefacts**: `methods/guest/src/bin/input_cost.rs`, `methods/tests/bytes.rs`, `COSTS.md`

## Context

Every figure this repository published before this task is a program **body**. That was
deliberate and it was stated, but it was also incomplete in a way that kept surfacing:
M3-09's report spent several review rounds on the term outside the body, and got its size
wrong twice — once by using the wrong codec, once by multiplying the right size by the
wrong unit.

P1 asks whether a verify-and-publish fits in one LEZ transaction. A body fitting is not
that. `read_lee_inputs` reads the program's own id, the caller's, the pre-state accounts
and the instruction words before any body runs, and those cycles are spent whatever the
program then does — a program cannot refuse bytes it has already been charged for
(ADR 26).

## Decision: the boundary is measured, not modelled

**Each mode's figure is one execution of its real program over the inputs LEZ would hand
it, and the input read is measured separately by a guest whose only work is the same four
reads.**

The alternative was to count words and multiply by ADR 26's rate. It is the obvious
approach, it is what the first draft of this task did, and measuring showed it wrong in
two ways at once.

**A transaction pays before it carries anything.** A run of that guest over four empty
inputs is 4,426 cycles — a gross figure, carrying zkVM startup and the journal commit as
well as the reads, because nothing subtracts a floor from it. The same term sits inside
every per-mode read figure, so it cancels between them and does not when one is quoted
alone. A word count times a rate has nowhere to put it at all.

**An account costs far more than its width.** Instruction words and account *data* both
cost exactly 113, linear to the cycle across two orders of magnitude — which confirms
ADR 26's estimate and settles its unit as a word rather than a byte. But an
`AccountWithMetadata` costs about 16,156 on top of its data, because deserialising one
builds a `Data`, an `AccountId` and a `ProgramId` rather than copying a span. Modelling
understated every mode, and the push read by nearly half: 63,506 against a measured
122,783.

So the figures are measured end to end per mode. The decomposition above is published
because it explains them, not because anything is computed from it.

*Rejected: multiply a word count by the rate.* What the first draft did. It is defensible
only if the rate is the only thing that varies, and two things do.

**And the transaction itself is measured, not assembled.** The first draft of this task
stopped at the input read and called body-plus-read a transaction, on the reasoning that
the dispatcher, the generated validator and the `SpelOutput` wrapping need the macro's
entry point rather than a function call. That is true and it is not a reason: the product
ELFs are exported, and running one over the inputs LEZ would hand it exercises all of
them. So each mode's figure is one execution of the real program, and the wrapping turns
out to be 188,480 to 280,649 cycles depending on the mode — larger than the input read it
sits beside, and larger than the entire body of a push consumer's read.

## Consequences

- **The per-mode headline changes order rather than gaining a caveat.** A pull
  settlement's body is 227 times a push settlement's. As executed transactions the two
  are **ten** times apart, because a push settlement is 13,404 cycles of body inside a
  324,667-cycle transaction — four percent of it. Both figures are published, because
  they answer different questions: what a mode's code costs, and what a transaction
  costs. The 448 the read tables carry is a read against a read and belongs beside
  neither.
- **M3-09's transaction estimate was low twice over.** It counted the instruction and not
  the accounts, multiplied rather than measured, and stopped at the body rather than the
  transaction. Its 14x-to-25x settlement reduction is about 6x to 7x against a measured
  transaction, and that section now carries the measured figure.
- **The bodies that can be executed here are, rather than copied.** `bytes.rs` runs the
  same guests and stages `read_cost.rs` measures for the pull settlement and the push
  read, and asserts they agree, so those two cannot drift apart silently. The push
  update's body is `cost.rs`'s: `submit_cost` takes a fifteen-field input this file would
  have to duplicate, and a duplicated fixture is the same failure in a new place. That
  row is therefore carried between files rather than checked across them, and `COSTS.md`
  says so where it prints it.
- **The residual is a difference, not a measurement.** "The rest" — dispatch, validation,
  decode and wrapping — is a transaction from the product ELF minus a body from a cost
  guest minus a read from `input_cost`, so it carries whatever those three disagree by;
  this repository has measured two of them at 42 and 722 cycles. Against 188,480 to
  280,649 that is a rounding error, and it is still a subtraction across harnesses rather
  than a figure anything brackets directly.
- **The read figures carry the read guest's own floor.** A run of `input_cost` over four
  empty inputs is 4,426 cycles, and that includes zkVM startup and the journal commit
  rather than being four framed reads alone. The same floor is inside every per-mode read
  figure, so it cancels between them and does not when one is quoted by itself. The
  marginal rates are clean, because differencing two input sizes removes it.
- **P1 can state what it was assuming.** A pull settlement is 3,513,718 cycles of the
  33,554,432-cycle budget, **10.47%**, executed end to end, and
  `every_transaction_fits_the_budget` asserts the share rather than leaving it to the
  reader's division.
- **One instruction is counted from a rule rather than encoded.** The push consumer's
  enum is generated inside its guest and has no host-side type (`[M3-06:01]`), so its
  33 words are a variant tag and a `feed_id`. The rule is checked against the pull
  consumer's real `Settle`, which comes to exactly `1 + 32 + 1 + 724`.
