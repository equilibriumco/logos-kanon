# 33. A feed lives at the address its id derives

- **Status**: accepted
- **Milestone**: M2 (`M2-07`)
- **Requirements**: F6, R2, U6
- **Artefacts**: `aggregator-program/src/register.rs`,
  `methods/guest/src/bin/aggregator.rs`, `kanon-idl/aggregator-idl.json`

## Context

M2-01 declared `register_feed` with `#[account(init)] feed` and no seeds, which
left the feed account's address undecided. Nothing had decided it: ADR 32 settled
the *price* account's address as a derivation from the feed account's own, and
ADR 16 rejected deriving the feed id from the asset pair, but every fixture in
the repository used an arbitrary constant. M2-07 could not be written without an
answer, because the answer decides what SPEL claims.

`#[account(init)]` is not neutral here. The macro generates `Claim::Pda` when the
declared account carries seeds and `Claim::Authorized` when it does not, so the
declaration chooses between two different programs:

- **Claim::Authorized.** The registrant generates a keypair per feed and signs
  with it. The address is arbitrary, so every consumer has to be told it out of
  band, and two accounts can both hold a feed calling itself BTC/USD with
  different signer sets — ADR 32's hazard arriving at the feed rather than at the
  price.
- **Claim::Pda.** The address is derived, so anyone can find a feed, and one feed
  id has exactly one account.

## Decision

**A feed lives at the address its id derives.**

```
feed  = AccountId::for_public_pda(program, sha256( zero_pad_32(feed_id) || zero_pad_32("KANON_FEED_ACCOUNT") ))
price = AccountId::for_public_pda(program, sha256( feed_account_id       || zero_pad_32("KANON_PRICE_ACCOUNT") ))
```

Declared as `#[account(init, pda = [arg("feed_id"), r#const("KANON_FEED_ACCOUNT")])]`
rather than checked in code, for the reason ADR 32 gives about the price account:
the derivation is what a client has to reproduce, and the constraint is what
publishes it in the IDL. The generated IDL now carries
`seeds: [{kind: arg, path: feed_id}, {kind: const, value: KANON_FEED_ACCOUNT}]`,
so the SDK, the CLI, the relayer and a consumer each derive it from the artefact
rather than from this document.

**`RegisterFeed.feed_id` is a padded `[u8; 32]` and no longer a `Vec<u8>`.** This
is forced rather than chosen. SPEL's `ToSeed` is implemented for `[u8; 32]`,
`u64`, `u32`, `String` and `&str`, and not for `Vec<u8>`. `String` is implemented
and routes through `seed_from_str`, which *panics* above 32 bytes — in generated
code that runs before any body this repository owns, which makes it a
caller-triggerable guest panic and so a defect under ADR 4. A fixed-width array
is the only seed type that cannot abort a transaction. The caller pads, exactly
as the wire pads.

Changing the surface is cheapest now, which is the other half of why it is done
here: the SDK is thirteen lines, the relayer eighteen, and the CLI does not
exist. `instruction_parity.rs` is what makes the change safe — the IDL and the
wire enum are compared per index, so a surface change that regenerates one and
not the other fails.

## Consequences

- **R2 costs no mechanism.** "Partial failure leaves existing registrations
  intact" is satisfied by the address space: a second registration of one feed id
  arrives at an account that is no longer default and is refused before anything
  is written, and two ids cannot collide. The deliverable for R2 is therefore the
  test that says so, not a rollback path.
- **A feed cannot be moved or re-created.** The address is the id's, so a
  registration is final for that id until `deregister_feed` (M2-09) empties it.
  What "empties" has to mean is now M2-09's question rather than an open one:
  anything short of `Account::default()` leaves the id unregisterable, because
  `register_feed` refuses a non-default account.
- **`registered` is not a state anyone stores.** Whether a feed exists is whether
  its derived account is non-default, which is a read a client can do without
  this program.
- **The feed id is padded twice, and the two must agree.** The caller pads to
  build the seed and `FeedAccount` stores the padded form.
  `FeedConfig::try_new` still refuses an all-zero or over-width id, so the
  validation is M1's and not restated.
- **One registration, one signer set.** Combined with ADR 30, a rotation is one
  `update_signer_set` per feed against an address the operator can derive, which
  is what makes the runbook writable.

## Alternatives considered

- **Keep `Claim::Authorized` and a caller-supplied address.** No surface change
  and no ADR, but the address has to travel out of band to every consumer, the
  runbook carries a throwaway keypair per feed, and nothing stops two feeds
  claiming one id. The last of those is the one that decided it: a signer set is
  what a price means, and two accounts able to answer for BTC/USD is a
  registration that means nothing.
- **Derive the feed account from the asset pair.** Rejected already, by ADR 16,
  and for a reason that has not changed: it needs a canonical symbol table with
  no authority behind it.
- **`String` as the seed type.** The natural IDL surface, and it panics above 32
  bytes in generated code. Ruled out by ADR 4 rather than by taste.
