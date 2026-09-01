# [M3-06:02]. Where a pull consumer's trust comes from

- **Status**: accepted
- **Milestone**: M3 (`M3-06`)
- **Requirements**: SEC2, U7, F9
- **Artefacts**: `reference-consumers/pull/src/authority.rs`, `reference-consumers/pull/src/trust.rs`, `reference-consumers/pull/guest/src/bin/pull_consumer.rs`

## Context

SEC2 says the authorised signer set comes from the consumer and never from the payload.
`pull-lib` satisfies half of that and cannot satisfy the other half: the roster arrives
there as a slice, and a slice built from a constant is indistinguishable from one built
from the transaction's own instruction data. A consumer that read its signers off the
wire would pass every check in that crate and would have handed its caller the right to
speak for the feed. F9's and SEC2's rows have both said, since M3-01, that the boundary
is the reference consumer's to demonstrate.

So the question is where a consumer program's signer set, threshold, staleness window
and asset pair come from.

## Decision

**They live in a trust account the consumer program owns, one per feed, written only
behind an authority gate.**

`settle` takes an order account, a trust account, the clock and a payload. It writes
neither the trust account nor the config, and there is no signer, threshold, window or
pair parameter anywhere in its instruction data —
`nothing_in_settles_instruction_data_can_reach_the_signer_set` reads that off the
published IDL rather than off the source, because the IDL is what a caller builds
against and an added parameter shows up there as a failure rather than as a diff nobody
read. What protects the roster is `every_instruction_that_moves_what_the_program_trusts_declares_a_signer`
and the `authority::authorise` gate behind it.

**All of a feed's configuration is per feed, which is ADR 30 reaching its pull caller.**
That ADR rejects a global roster in as many words — it cannot represent an estate where
one feed's set has moved, and the repair once feeds are live is a migration rather than
an edit — and its consequence list says a pull consumer configures its own set per feed
too. The argument reaches the threshold and the window as well, because a threshold is
only meaningful against the set it counts, so `FeedTrust` carries all of it and is the
compile-time counterpart of the `FeedAccount` the push side stores. One configuration
shape behind both modes is what makes ADR 2's "one verification, two modes" visible to
somebody reading the two programs side by side.

**The authority is established from a build input, and handover is two steps.** The
first write to the config account decides who governs the program, so an unguarded
`establish` is a race whoever watches for a deployment wins; the genesis key closes it,
and `establish` refuses a build that configured none. `nominate` and `accept` are
separate so a mistyped key costs a second nomination rather than the whole
administrative surface, and `accept` refuses a nominee whose account is pristine —
LEZ can never honour a write signed by a default-owned account, so an authority that is
one is an authority that cannot act. All three are the aggregator's choices, reached
independently here and deliberately spelled the same way; `[M2-06:01]` carries the
reasoning and the review that found the pristine-key case.

*Rejected: a roster compiled into the program.* **This was the first version of this
crate, and the argument for it did not survive reading F9.** The claim was that F9 wants
a consumer that registers nothing, so state of the consumer's own would be pull mode
wearing push's clothes. F9 says something narrower: a configured tuple, and "no
dependency on **the aggregator's price account**". Registering a roster in an account
this program owns is not registering a feed against the aggregator, and SEC2's "comes
from the consumer" is satisfied better by governed state than by a constant, because it
also shows *how* a consumer governs its set. The paraphrase that misled the first
version — "a consumer using only pull registers nothing (F9)" — has been corrected where
it was written.

The design was also worse than unnecessary. RedStone rotates: ADR 30 records that the
captured addresses are unchanged since the capture and that a source two and a half
years older shares none of them, which is exactly why the push path has
`update_signer_set` rather than a table. A compiled roster makes that certain event a
redeployment, and in LEZ a redeployment is not a restart — a program's id is its RISC0
image id and every derived address hashes the program id, so a changed build input
presents fresh addresses and abandons what the previous build created. Rotating a signer
set would have cost every open order.

*Rejected: an authority with a revoke.* The aggregator has one because RFP-001 asks for
it. A consumer that revoked its own authority would freeze every trust account it owns
with no way back and nothing gained, so the surface is not carried here.

## Consequences

- **A rotation is a transaction, and it changes what verifies.**
  `a_rotation_changes_which_payloads_verify_without_a_redeployment` asserts it over the
  captured signatures in both directions: narrowing the roster below the set that signed
  a payload refuses that payload outright (ADR 15), and widening it invalidates nothing,
  because an added signer that did not report contributes nothing towards the threshold.
- **An authority can change who speaks for a feed under an open order.** The exposure
  that comes with the decision, and
  `an_order_opened_before_a_rotation_is_settled_under_the_roster_in_force` states it
  rather than leaving it implicit. It is the same exposure a push consumer has to
  `update_signer_set`. What an authority cannot change is what an order is priced
  against: the pair, the scale and the window are fixed at registration, and
  `rotate_signers` moves only the roster and its threshold.
- **Three derived addresses, and all three can be squatted.** The config account's
  address derives from the program id alone and a reproducible build makes the next
  one computable from published source; a trust account's derives from a feed id and an
  order's from an order id. An account that holds a balance but is still unowned can
  never be written by any program, so each of the three refuses it with its own cause
  rather than reporting "already there" — the advice differs, and for the config account
  it means the build is unusable until an input changes. This is the same hazard #48
  found on the aggregator's feed accounts.
- **`expected` is structurally dead in this consumer, and that is stated in the code.**
  `verify_feed` compares the caller's claimed pair against the configuration's, and here
  both come from one trust account, so the comparison cannot fail. The push path is the
  contrast rather than a thicker version of the same check: there the pair comes from the
  price account and the configuration from the feed account, so it holds two
  independently written records against each other (ADR 32). A pull consumer that grows a
  second record gets a real check back by passing it.
- **Seven instructions, which is more than a reference consumer would need to show
  verification alone.** Two are the domain, five are governance. That ratio is the honest
  one: the verification call is a single line, and everything around it is what makes the
  roster the consumer's rather than its caller's.
