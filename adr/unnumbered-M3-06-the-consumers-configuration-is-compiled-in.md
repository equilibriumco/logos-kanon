# [M3-06:02]. The consumer's configuration is compiled in

- **Status**: accepted
- **Milestone**: M3 (`M3-06`)
- **Requirements**: SEC2, U7
- **Artefacts**: `reference-consumers/pull/src/lib.rs`, `reference-consumers/pull/guest/src/bin/pull_consumer.rs`

## Context

SEC2 says the authorised signer set comes from the consumer and never from a payload.
`pull-lib` satisfies half of that and cannot satisfy the other half: the roster arrives
there as a slice, and a slice built from a constant is indistinguishable from one built
from the transaction's own instruction data. A consumer that read its signers off the
wire would pass every check in that crate and would have handed its caller the right to
speak for the feed. F9's and SEC2's rows have both said, since M3-01, that the boundary
is the reference consumer's to demonstrate.

So the question this task had to answer is where a consumer program's signer set,
threshold, staleness window and asset pair come from. Three answers were available.

## Decision

**The whole configuration is `const` in the consumer, and the only thing instruction data
may influence is which of a compiled table of feeds an order is priced against.**

`SIGNERS`, `THRESHOLD`, `MAX_AGE_MS`, `DECIMALS` and `FEEDS` are constants.
`settle` takes an order account, the clock account and a payload — there is no signer,
threshold, window or pair parameter, so the demonstration is structural rather than
asserted. `nothing_in_settles_instruction_data_can_reach_the_signer_set` reads that off
the published IDL rather than off the source, because the IDL is what a caller builds
against and an added parameter shows up there as a failure rather than as a diff nobody
read.

A feed index is the one opening, and it is not a roster: an out-of-range index is
`UnknownFeed`, and every feed in the table carries the same compiled signer set. A
consumer serving five pairs needs some way to say which, and this is the narrowest form
of that.

*Rejected: the configuration in an account the program owns.* It is the more realistic
shape for a deployment that wants to rotate a roster without a rebuild, and it is also
the aggregator's shape — a registered feed account holding a signer set, with an
admin-gated update path. Rejected because reproducing it here would reproduce the thing
pull mode exists to avoid. F9's requirement is that a pull consumer integrates *without
registering a feed*; a reference consumer whose first instruction registers its
configuration would be demonstrating push's ownership model in pull's clothing, and a
reader would reasonably conclude that pull mode needs one too. A deployment that wants a
rotatable roster can read this program and add the account; a reader who wants to know
what pull mode requires needs to see that it requires nothing.

*Rejected: a build-time input, the way the aggregator takes its genesis admin.*
`option_env!` suits a single 32-byte key and does not suit five feed ids, five pairs and
a signer set — a hex-decoding `const fn` per field, with no compiler help if one is
wrong. The aggregator's genesis key is a build input because devnet and mainnet must not
share one; a RedStone signer set is the same on both.

## Consequences

- **This build serves five feeds and rotating the roster is a rebuild.** Stated plainly
  rather than hedged, because it is the cost of the decision above and a deployment
  should know it before copying the file. `FEEDS.md` records which addresses served which
  data service when this was measured.
- **The roster is load-bearing rather than decorative.** Moving one byte of one address
  in `SIGNERS` fails seven tests, because the captured payloads then recover to a signer
  this build does not authorise. That is what makes the constants the binding rather than
  the comment above them.
- **`SettleError::Config` is unreachable in this build and is kept anyway.** Every input
  to `FeedConfig::try_new` is a constant here, so the variant can only be produced by a
  deployment that edited them wrongly. `the_compiled_in_configuration_is_usable` runs the
  construction over all five feeds, which is what makes the variant unreachable rather
  than merely unlikely — and it fails closed instead of panicking, because a panic in a
  guest aborts the transaction rather than refusing the instruction.
- **The asset ids are placeholders and are the one edit a deployment must make.** They are
  the tickers, right-padded to 32 bytes, because the LEZ account ids of the real assets
  are not something this repository can know. Nothing verifies them: no signer attests to
  which assets a feed prices, so a consumer that copies the wrong ids into both its
  `FeedConfig` and its `expected` pair gets a verified price under the wrong label and no
  error. Getting the feed id right is what prevents that, which is why the feed ids in
  the table are RedStone's real ones.
