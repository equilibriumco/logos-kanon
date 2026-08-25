# 26. A maximum payload size, and where it has to be enforced

- **Status**: accepted
- **Milestone**: M1 (`M1-12`)
- **Requirements**: P1
- **Artefacts**: `verifier-core/src/decode.rs`, `methods/tests/cost.rs`, `COSTS.md`

## Context

ADR 25 bounded what verification spends on a payload. It did not bound what the
payload costs to *read*, and that turns out to be the larger half.

Getting bytes into guest memory costs about 113 cycles each. A payload of 900
repeated packages — 127,814 bytes, well formed, and correctly refused by ADR 25's
recovery ceiling — measured **33,792,622 cycles against a 33,554,432 budget**.
Verification behaved exactly as ADR 25 says it does: flat at 19.3M whatever the
package count. The other 14.5M was the read, and the transaction ran out of
cycles before anything could report `TooManyPackages`.

**No LEZ program can defend itself against this.** `lee_core`'s
`read_lee_inputs` does `let instruction_words: InstructionData = env::read()`
and then deserializes into the program's own instruction type. Both happen
before the program's first instruction. `InstructionData` is a `Vec<u32>` with
no length bound anywhere in `lee_core`. By the time `Payload::decode` runs, or
any other line this repository owns, the cycles are already spent.

That makes it a property of the platform rather than of this adaptor, and it
applies to any LEZ program whose instruction data comes from its caller. What
this repository can do is state the number.

## Decision

**`MAX_PAYLOAD_BYTES = 32 KiB`, checked first in `Payload::decode`.**

The derivation, from measurements rather than from the wire format: the budget is
33,554,432 cycles, ADR 25's ceiling puts verification at about 19.4M of them, and
the read costs 113 a byte. That leaves room for roughly 125 KB of payload before
the two together exhaust the budget — with nothing at all left for the program
doing the verifying. 32 KiB is a third of that. The largest payload the decoder
accepts, every package of it carrying the requested feed, reads and verifies in
**23,042,390 cycles, 69% of the budget**, and the remaining 31% belongs to the
caller.

It is not a tight fit for anything real: 32 KiB is 230 packages, a bundle of ten
feeds at twenty signers each and then some, against RedStone payloads that run to
hundreds of bytes per feed.

**It is a cost rule, and `DecodeError::TooLong` says so.** Every other variant in
that enum means the payload is not well formed. This one means it is too
expensive, which is a different thing and belongs in a different variant even
though both refuse.

**Checked before the marker.** Everything else `decode` does is proportional to
the payload, and this is the only check that is not.

**Its real job is to be a number other people can enforce.** Refusing here does
not refund the read. What the limit gives a relayer, an aggregator or a sequencer
is the size above which a payload will not verify, so it can be refused where
refusing is still free — before the transaction is built, or before it is
admitted.

## Consequences

- **The pair is now asserted, not argued.**
  `the_largest_payload_the_decoder_accepts_is_read_and_verified_inside_the_budget`
  builds a payload at the limit and requires read plus verification under the
  budget, with more than a quarter of it left over. P1's row carries it. Together
  with ADR 25's test, the two cover both halves of what a payload can spend.
- **An honest update costs 42 cycles more**, from one length comparison. The
  published figures moved with it, as they do for any change to the walk.
- **The residual is upstream and named.** A caller that submits a 127 KB payload
  still burns a transaction, because nothing in the guest can stop it. Bounding
  `InstructionData` is LEZ's to do, and worth raising: every program that takes
  caller-supplied instruction data has this, and none of them can see it coming.
- **M2-02 and M3-01 inherit an obligation.** The aggregator must refuse an
  oversized payload before submitting, and the pull library's callers must not
  hand one to a program. Neither is written yet; both now have a number to
  enforce rather than a judgement to make.

## Alternatives considered

- **Read the payload incrementally and stop at the limit.** The right shape, and
  not available: `read_lee_inputs` has consumed the whole instruction stream
  before any program code runs.
- **Derive the limit from `MAX_RECOVERIES` instead of choosing it.** 32 packages
  is 4,544 bytes, which would refuse every multi-feed payload — the mistake ADR
  26 already rejected in the other direction. The limit has to admit packages for
  feeds this call is not verifying, so it cannot come from the recovery ceiling.
- **Set it at the arithmetic maximum, about 125 KB.** It fits, and only if the
  program does nothing else. A verifier that leaves no cycles for the write it
  exists to perform has not bounded anything useful.
- **Leave it to the caller entirely.** The caller needs a number, and this crate
  is where the two measurements that produce it live. Stating it here and
  enforcing it there is the division that survives M2-02 changing shape.
