# 22. The price-account write, and the three fields nothing had decided

- **Status**: accepted
- **Milestone**: M1 (`M1-20`)
- **Requirements**: F5
- **Artefacts**: `aggregator-program/src/publish.rs`, `verifier-core/src/feed.rs`, `kanon-idl/src/lib.rs`

## Context

M1-20 was written as "the RFP-019 price-account write **or** the forward-compatible
fallback". The fallback branch is dead: ADR 9 found the canonical struct shipped in
`logos-blockchain/lez-programs` even though the grant meant to deliver it was closed, and
`kanon-idl` has re-exported it since. There is a real account to populate.

Populating it means answering for all six of its fields, and only three of them had an
answer:

| field | decided by |
| --- | --- |
| `base_asset`, `quote_asset` | ADR 16 — the registration claim |
| `price` | ADR 17 — the median on the account's `Q64.64` scale |
| `timestamp` | nothing |
| `source_id` | nothing |
| `confidence_interval` | nothing |

Two of the three turned out to be settled upstream and simply unread. The account's own
field documentation says `confidence_interval` is "Source-provided confidence interval, or
zero when the source does not provide one", which is what F5 asks for and what RedStone
leaves us with. And `timestamp` is documented as the "price observation timestamp", with
the unit left to the consumer to match against its `max_age` — an observation time, not a
write time.

That leaves the questions this ADR exists for: *which* observation, what a source
identifier is when the source is not on this chain, and what stops a price moving
backwards.

### What the Q64.64 convention actually rests on

ADR 17 wrote that `Q64.64` is "the only written specification that exists", on the strength
of the account constructor's doc comment. Reading further into the crate for this task
turns up more than that, and it is worth recording where the other decisions are.

`twap_oracle_core` exports the convention as a named constant:

```rust
/// Number of fractional bits in the [`OraclePriceAccount::price`] fixed-point value.
/// ... A consumer multiplies a token amount by the price with
/// `(amount * price) >> PRICE_FRACTIONAL_BITS`.
pub const PRICE_FRACTIONAL_BITS: u32 = 64;
```

and its own writer uses it: `publish_price` sets `price_account.price =
tick_to_oracle_price(twap_tick)`, which computes `1.0001^tick * 2^64`. So the convention is
not only written down, it is what the *other* writer of this shared account implements. The
account is generic across oracle types by design, and two writers disagreeing about the
scale of one `u128` would be undetectable by any reader.

`kanon-idl` therefore re-exports `PRICE_FRACTIONAL_BITS` alongside the account, so the
reference consumers can read the field with upstream's constant rather than a local `64`.
The convention should have one definition and it should not be ours.

## Decision

`aggregator-program/src/publish.rs`: `price_account` builds the account for a first write,
`publish` updates one that exists.

### `timestamp` is the oldest package behind the median

`VerifiedFeed` gains `timestamp_ms`, the smallest timestamp among the packages whose values
reached the median.

A median over N signers is only as current as the stalest report that shaped it. Publishing
the newest would let one prompt signer speak for the ones beside it, so a consumer applying
its own `maxAge` to this field is never told a price is fresher than every signer behind it
supports. The alternatives are worse in specific ways: the write time says nothing about
when anybody observed anything, and the median value's own timestamp is arbitrary, because
which value lands in the middle has no relationship to when it was signed.

Packages that did not count do not age it. A skipped value costs its own signer's slot and
nothing more (ADR 15), and letting its timestamp through would date the price by a report
that is not behind it.

It is tracked as a running minimum rather than a timestamp per slot: slots are only ever
filled, never cleared, so every update is a package that ends up behind the median, and the
array would cost 256 bytes of a guest frame that already holds about 2.7 KB.

### `source_id` is the adaptor's own identifier

```rust
pub fn adaptor_id(program_id: ProgramId) -> AccountId
```

The field names the source that populated the account, and the account's documentation
gives "a TWAP program or external adaptor" as what goes there. The aggregator is the
external adaptor, so it names itself: a LEZ program reads its own `ProgramId` at run time,
and `publish` takes the resulting `AccountId` rather than holding one.

Upstream's own writer does something narrower than its doc describes.
`create_oracle_price_account` sets `source_id: price_source_id` — the AMM pool, which is
where the price came from rather than the program that wrote it — and `publish_price` never
touches the field again. So the doc names the populating program and the code names the
data origin, and for a TWAP those are two different accounts. For this adaptor they are not
two options: the data origin is RedStone, an off-chain signer set with nothing on this
chain to point at, which leaves the writer as the only identity available and the doc's
plain reading as the one to follow.

The first version of this decision was a constant — thirty-two bytes of ASCII spelling out
that a RedStone adaptor wrote the account — on the argument that a consumer asking "is this
a RedStone price?" wants the same answer on every network. That argument does not survive
contact with what the field is for. A constant is a label the writer asserts about itself,
and any program can write those same bytes into an account it controls, so the question it
was meant to answer is exactly the question it cannot answer. An identifier the chain
assigns is not assertable by anybody else.

A LEZ `ProgramId` is the RISC Zero image id of the program's ELF, computed by the runtime
from the binary it is about to execute and written into the guest's input stream by the
host. The guest reads it; it cannot state it. So this is not merely harder to forge than a
constant, it is not assertable at all.

Naming the writer also keeps two deployments of this adaptor distinguishable, which matters
in the direction that costs something: a consumer that has decided to trust one deployment
has not thereby trusted its replacement.

`ProgramId` is `[u32; 8]` and the field is thirty-two bytes, so the conversion has an
endianness. Little-endian, which is how LEZ lays a program id out when it feeds one to a
hash, and a test pins it — getting it wrong is silent, since every id would still map to a
distinct thirty-two bytes, just not the ones the chain knows the program by.

### `confidence_interval` is zero, which is a specified value here

F5 asks for zero and the account documents zero as what a source without a confidence
interval writes. The alternative considered was the spread across reporting signers, which
is nearly free — `median` already sorts, so the extremes are the ends of that array — and
genuinely informative. It is still not what the field means: the account asks for the
*source's* confidence, and inventing one from our own aggregation would put a number there
that RedStone never published.

### A write refuses rather than repoints, and only moves forward

`publish` changes `price` and `timestamp` and nothing else. A mismatch in the asset pair or
in `source_id` means this is the wrong account, not an account needing an update, and
overwriting the identifiers would turn a misrouted write into a plausible-looking one.

It also requires the offered observation to be strictly newer than the stored one. The
staleness window bounds a package's age against the chain clock; it says nothing about its
age against what is already published, so replaying a payload from earlier in the same
window is free and would otherwise move the price backwards. Equal timestamps are refused
too — the same observation twice is not an update.

## Consequences

- **`VerifiedFeed` is wider by eight bytes and the pull path gets the field too.** It is
  the same struct both modes return, and a pull consumer deciding staleness for itself
  needs the same number for the same reason.
- **`kanon-idl` takes `lee_core` directly**, for `AccountId`, which three of the six fields
  are. It resolves to the pin the workspace already names through `twap_oracle_core`, so
  the tree gains nothing; ADR 8's argument that the two pins are one decision is what makes
  that safe to do.
- **`AssetPair` is now re-exported from `verifier-core`'s root.** It was reachable only as
  `verifier_core::feed::AssetPair`, which was an oversight rather than a decision —
  `FeedConfig::try_new` takes one.
- **The timestamp costs no cycles.** `per_component_cycles_are_unchanged` asserts the
  published figures to the cycle and still passes, which was surprising enough to check
  against a deliberate probe: a `black_box` loop in the same place moves the 1-signer
  remainder from 25,323 to 25,324, so the harness does rebuild the guest and does see one
  cycle. The running minimum folds into the branch already there. `COSTS.md` is unchanged.
- **The guards were checked by breaking them.** Each of the four — the oldest-package
  minimum, the pair check, the source check, and strict newness — was inverted or removed in
  turn, and each time a test that claims to cover it failed. The habit is ADR 21's.
- **`SourceMismatch` may end up as a second line of defence.** Upstream pins the source
  through the account's address — the price account is a PDA over
  `(oracle_program_id, price_source_id, window_duration)` — rather than by comparing the
  field on write, which is why `publish_price` can leave the identifiers alone without
  checking them. If M2-01 derives Kanon's PDA the same way, a misrouted write is already
  impossible before the check runs. The check stays: it costs a comparison, and it is what
  makes the write correct on its own terms rather than only in the context of a derivation
  decided in another milestone.
- **An upgrade to the aggregator moves every price account to a new address, and nothing
  can migrate one.** This is a LEZ property rather than a Kanon decision, and it is worth
  stating here because the write is where it first bites. A `ProgramId` is the image id of
  the ELF, so a dependency bump is a different program; `Account::program_owner` binds each
  account to the id that created it; and `validate_execution` forbids both writing an
  account another program owns and changing an owner at all — a program may claim an
  account only while its owner is still `DEFAULT_PROGRAM_ID`. There is therefore no
  operation, admin-gated or otherwise, that hands the old accounts to the new binary.

  What happens instead is quieter. `AccountId::for_public_pda` hashes the program id into
  the address, so the rebuilt adaptor derives a different set of PDAs, finds them empty,
  claims them and runs. The old accounts are frozen holding their last price — non-zero,
  with a plausible timestamp — so a consumer reading a hardcoded address sees a price that
  is valid-looking and permanently stale, and only the timestamp going cold says otherwise.
  Discovery, not migration, is what a consumer needs, and where that lands is M2-04's
  read path and the reference consumers.

- **`SourceMismatch` is defence against a caller, not against another program.** LEZ rule 6
  already makes it impossible to write an account this program does not own. What the check
  still catches is `publish` being called on an account that was never initialised, whose
  zeroed `source_id` matches nothing — a caller mistake rather than an attack, and one that
  would otherwise write a real price into an account with no assets named in it.
- **Nothing calls `publish` yet.** M2-01 brings the SPEL skeleton and M2-02 the
  `submit_price` path that calls it; M1-26 conforms the encoded layout on top of it. The
  write is a pure function over a verified feed until then, which is why it is testable at
  all this early.
- **One question stays open with Logos**, and it is smaller than ADR 17 left it: we write
  `Q64.64`, matching `PRICE_FRACTIONAL_BITS` and what the TWAP oracle publishes, and the
  thing worth confirming is only that consumers do the same. A consumer assuming a base-10
  integer is out by roughly `1.8 x 10^19` with nothing on chain to signal it.

## Alternatives considered

- **The newest contributing package, or the write time.** Both overstate freshness, which
  is the direction that matters: a consumer's `maxAge` exists to refuse a price that is too
  old, and a timestamp that is too new defeats it silently.
- **A constant naming RedStone rather than the writer.** Stable across networks and
  legible in a hex dump, and forgeable by any program that cares to write the same bytes,
  which makes it useless for the one question it was for. Recorded here because it was the
  first answer and the reasoning that displaced it is the point.
- **The signer spread as `confidence_interval`.** Cheap and meaningful, and not what the
  field asks for. If Logos ever wants it, the shape of the answer is here in the record.
- **Overwriting the identifiers on every write, treating the account as ours.** It removes
  two branches and makes a misrouted write indistinguishable from a correct one. The price
  account is the only thing consumers read, so the wrong number in the right account is the
  worst failure available to this program.
- **Leaving monotonicity to M2-15's update-window work.** It is one comparison, it is a
  property of the write rather than of the transaction around it, and a replay that moves a
  published price backwards is not a test failure anybody would see first.
