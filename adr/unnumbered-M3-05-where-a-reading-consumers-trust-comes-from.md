# [M3-05:01]. Where a reading consumer's trust comes from

- **Status**: accepted
- **Milestone**: M3 (`M3-05`)
- **Requirements**: U7, R1
- **Artefacts**: `reference-consumers/aggregator-read/src/source.rs`, `reference-consumers/aggregator-read/src/read.rs`, `reference-consumers/aggregator-read/guest/src/bin/aggregator_read_consumer.rs`

## Context

In push mode a consumer sends this adaptor no transaction at all. It decodes the
canonical price account and acts on what it finds, which is what R1's row says and why
that requirement is asserted from the aggregator's IDL rather than from a read
instruction that does not exist.

That leaves one question, and the whole mode turns on it: **which account?**

Nothing in RFP-019's six fields identifies the writer. `source_id` is the constant
`"RedStone"` on every Kanon feed — F5 asks it to name where the numbers came from, not
which program published them — and the pair, price and timestamp are values, not
provenance. A caller hands a consumer program the accounts it will read, so a consumer
that trusted the bytes would be letting its caller choose the price.

The only thing that binds a price account to a particular aggregator is its **address**,
which hashes the aggregator's program id (ADR 32, ADR 33). And in LEZ a program's id is
its RISC0 image id, so that id moves whenever the aggregator's build inputs move.

## Decision

**The aggregator's program id, the feed's pair and the consumer's staleness window live
in a `PriceSource` account the consumer program owns, one per feed, written only behind
an authority gate.** `read::read_price` derives the price account's address from that
registration and refuses any other account before it looks at a single field.

The authority is the same two-step arrangement reference consumer B uses and for the
same structural reasons: a genesis key carried as a build input so the first write is
not a race, and a nominate-then-accept handover so a mistyped key costs a nomination
rather than the program.

**`update_aggregator` is the operation the decision exists for.** It moves one field and
nothing else, so a consumer follows the next aggregator build in one transaction.

### The alternative, and why it loses

A compiled aggregator id is smaller, and its failure mode is not obviously bad: the
derived address goes dead, every read refuses `Unavailable`, and refusing is what U7
asks for. That is a real argument and it is why this was worth writing down.

It loses on what recovery costs. Following the rebuild would mean rebuilding this
consumer, and a rebuild moves this program's own id — so the config account, every
source account and every open order move with it and the previous build's state is
abandoned. An aggregator fix would cost the order book. `[M3-06:02]` rejected a compiled
roster on the same mechanism with a different trigger: B follows RedStone's signer
rotations, this consumer follows the aggregator's builds. Neither event is the
consumer's to schedule and both are certain.

`an_order_survives_the_aggregator_being_rebuilt` is the assertion, and it is written as
a before-and-after: the new build's account is refused under the old registration, one
`update_aggregator` lands, and the same order fills.

## Consequences

- **Three records, two comparisons, and that is what makes the pair check real.** The
  price account's pair is the aggregator's claim; the source account's is this
  consumer's authority's; the order's is its owner's. `read_price` compares the first
  two and `terms_still_hold` the second and third. Holding the account's pair against
  itself would satisfy the words of U7 and check nothing, which is the mistake
  `[M3-06:02]` records B making and fixing.
- **The consumer's staleness window is not the aggregator's, and is legitimately
  wider.** The aggregator's `max_age_ms` bounds how old a package may be when it is
  verified; this one bounds how old a published price may be when it is read, and that
  clock keeps running after the write. `this_consumer_is_never_stricter_about_time_than_the_verifier`
  pins the ordering, because the other direction would refuse observations the
  aggregator was entitled to publish. The window is bounded at a day, since a
  `max_age_ms` near `u64::MAX` puts the lower edge at zero and turns the check off.
- **The price account is the one account in the program with no `pda` constraint.** SPEL
  derives a constraint's address under `self_program_id`, and this address is another
  program's, so the constraint cannot express it. The check moves into Rust and runs
  first. Two tests hold the pair together: `the_price_account_carries_no_pda_constraint`
  records the omission as deliberate, so nobody "fixes" it into an address no aggregator
  writes to, and `the_generated_validator_does_not_check_the_price_account` shows the
  dispatcher accepting an arbitrary account — which is why the Rust check has to be
  there.
- **Three constants are copied out of the aggregator rather than imported.** Two seed
  strings and the RedStone source id. A Logos module in another repository could not link
  `aggregator-program`, so a reference that did would be demonstrating something no
  reader could copy; `tests/derivation.rs` fails if a copy and its original disagree,
  which is the arrangement `kanon_idl::PRICE_ACCOUNT_FIELDS` already uses.
- **An authority can repoint a feed under an open order.** The exposure that comes with
  the decision. What it cannot change silently is what the order means: `terms_still_hold`
  destructures a whole `PriceSource` with no `..`, binds the pair and the window, and
  deliberately does not bind the aggregator — following a rebuild moves where a price is
  read from, not what it means.
- **This consumer's guest pins no accelerators.** It recovers no signatures, so risc0's
  `k256`, `crypto-bigint` and `sha2` forks buy it nothing. `tests/no_accelerators.rs`
  asserts both halves — that this manifest has none and that the two verifying guests do
  — because a copied manifest would carry pins nobody reads twice, and the difference
  between the modes is what M3-09's per-mode precompile delta is about.
- **Eight instructions, of which two are the domain.** The same ratio reference consumer
  B has, and the honest one: reading a price is a few lines, and everything around it is
  what makes the account read the authority's choice rather than the caller's.
