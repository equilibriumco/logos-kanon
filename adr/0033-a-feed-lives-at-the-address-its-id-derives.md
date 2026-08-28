# 33. A feed lives at the address its id derives

- **Status**: accepted
- **Milestone**: M2 (`M2-07`)
- **Requirements**: F6, R2, U6
- **Artefacts**: `aggregator-program/src/register.rs`,
  `methods/guest/src/bin/aggregator.rs`, `kanon-idl/aggregator-idl.json`

## Context

M2-01 declared `register_feed` with `#[account(init)] feed` and no seeds, which
left the feed account's address to M2-07. It is not that nothing had decided it.
**ADR 32 decided it the other way**, and named this task as the place to
reconsider:

> Rejected because the enforcement is permanent and unrecoverable: ownership can
> never be released, `deregister_feed` can zero a feed's data but not un-own its
> account, and `init` compares against a default *account*, which an owned one
> with zeroed data still fails. One registration with the wrong exponent would
> burn that pair for the lifetime of the program build.
>
> — ADR 32, which adds that it is "worth raising against **M2-07, whose shape
> would change**"

That objection is correct and it survives this decision: deriving from the feed
id carries the same permanence. **This ADR supersedes ADR 32 on that point**, and
the cost is stated in the consequences rather than left implicit. A reader who
finds ADR 32 first should read this paragraph as the answer to it.

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
- **A registration is permanent for that feed id, and no implementation can change
  that.** `Account::default()` requires `program_owner == DEFAULT_PROGRAM_ID`, and
  rule 4 forbids a program changing an account's owner, so once a registration claims
  the account it can never equal `Account::default()` again whatever a later
  deregistration writes. M2-09 therefore has no design space here and should not begin
  by looking for it: the most it can do is empty the data and keep the account, which
  means `register_feed` will have to admit a third pre-state. What is genuinely
  permanent is the *configuration set at registration*: only `signers` and `threshold`
  are scheduled for an update path (M2-08), so a wrong `decimals` or `max_age_ms`
  burns that feed id for the life of the build. Deriving the address also removes the
  one escape `Claim::Authorized` had, which was to abandon the account and re-create
  the feed at a fresh address under the same id. That is the cost ADR 32 named, paid
  knowingly: two accounts able to answer for BTC/USD with different signer sets is the
  worse hazard, and an update path for the remaining fields is the mitigation if it is
  ever wanted.
- **`registered` is not a state anyone stores.** Whether a feed exists is whether
  its derived account is non-default, which is a read a client can do without
  this program.
- **The feed id is padded twice, and the two must agree.** The caller pads to
  build the seed and `FeedAccount` stores the padded form.
  `FeedConfig::try_new` still refuses an all-zero or over-width id, so the
  validation is M1's and not restated.
- **A derived address can be squatted, and this program cannot prevent it.** The
  address is publicly derivable; rule 5 guards balance *decreases* only and rule 7's
  guard is `pre != default`, so anyone may put one unit of balance on a pristine
  derived account. It is then non-default and default-owned — the third state ADR 32
  identified at the price account, arriving by choice rather than by accident — and
  rule 6 refuses a data change while rule 7 refuses any post-state keeping the default
  owner. A claim cannot rescue it, because the claim loop runs after
  `validate_execution`. That kills the feed id for the life of the build, and the same
  attack lands on the admin config account, whose address takes no per-deployment
  input at all. Not M2-07's to fix and not fixed here: recorded in `m0/versions.md`
  question 4 and pinned by
  `a_derived_address_can_be_squatted_and_this_pins_the_refusal`.
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
