# 33. A feed lives at the address its id derives

- **Status**: accepted
- **Milestone**: M2 (`M2-07`)
- **Requirements**: F6, R2, U6
- **Artefacts**: `aggregator-program/src/register.rs`,
  `methods/guest/src/bin/aggregator.rs`, `kanon-idl/aggregator-idl.json`

## Context

M2-01 declared `register_feed` with `#[account(init)] feed` and no seeds, which
left the feed account's address to M2-07. It is not that nothing had been said
about it: **ADR 32 rejected deriving the feed account from the asset pair, for a
reason that reaches any derived address**, and named this task as the place to
weigh it:

> Rejected because the enforcement is permanent and unrecoverable: ownership can
> never be released, `deregister_feed` can zero a feed's data but not un-own its
> account, and `init` compares against a default *account*, which an owned one
> with zeroed data still fails. One registration with the wrong exponent would
> burn that pair for the lifetime of the program build.
>
> — ADR 32, which adds that it is "worth raising against **M2-07, whose shape
> would change**"

That objection is correct and it survives this decision: deriving from the feed
id carries the same permanence. What ADR 32 rejected was the pair as the seed,
partly on ADR 16's grounds, so there is no decision here to overturn — what is
new is that **the trade-off is now taken deliberately**, with the cost stated in
the consequences rather than left implicit. A reader who finds ADR 32 first
should read this paragraph as the answer to it.

`#[account(init)]` — the declaration M2-01 left, and the one this decision was
taken against — is not neutral here. The macro generates `Claim::Pda` when the
declared account carries seeds and `Claim::Authorized` when it does not, so the
declaration chooses between two different programs:

- **Claim::Authorized.** The registrant generates a keypair per feed and signs
  with it. The address is arbitrary, so every consumer has to be told it out of
  band, and two accounts can both hold a feed calling itself BTC/USD with
  different signer sets — ADR 32's hazard arriving at the feed rather than at the
  price.
- **Claim::Pda.** The address is derived, so anyone can find a feed, and one feed
  id has exactly one account.

The seeds are what that turned on, and they are still declared. The `init` half
did not survive M2-09 — the consequences record why — so the claim is now built by
the body rather than by the generated helper, at the same address either way.

## Decision

**A feed lives at the address its id derives.**

```
feed  = AccountId::for_public_pda(program, sha256( zero_pad_32(feed_id) || zero_pad_32("KANON_FEED_ACCOUNT") ))
price = AccountId::for_public_pda(program, sha256( feed_account_id       || zero_pad_32("KANON_PRICE_ACCOUNT") ))
```

Declared as `#[account(mut, pda = [arg("feed_id"), r#const("KANON_FEED_ACCOUNT")])]`
rather than checked in code, for the reason ADR 32 gives about the price account:
the derivation is what a client has to reproduce, and the constraint is what
publishes it in the IDL. It was `init` until the feed account gained a third
pre-state; the consequences below record why that could not stay. The generated IDL now carries
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
- **A feed id outlives its registrations, and that took a third state — but its
  configuration does not.** Retiring `BTC` must not spend `BTC`, since RedStone's ids
  are fixed strings. LEZ will not let a program hand an account back, though: rule 4
  forbids giving up ownership and rule 3 forbids resetting the nonce, so a feed
  account is this program's from its first registration onwards and can never equal
  `Account::default()` again. `deregister_feed` (M2-09) therefore empties the data and
  keeps the account, and `register_feed` accepts three pre-states rather than two —
  which cost the `init` constraint: `#[account(init)]` emits
  `accounts[0] != Account::default()` in the dispatcher, so it would have refused every
  re-registration as `AccountAlreadyInitialized` before the body could accept it. The
  declaration is `#[account(mut, pda = [...])]` and the body is the only layer that
  tells the three states apart, which is the point: a constraint can check the address
  but not which of three pre-states an account is in. Pinned by
  `a_deregistered_feed_account_reaches_the_body`. The states are —
  fully default (first registration, claimed), ours and empty (re-registration, not
  claimed, because LEZ refuses a claim on an account it already owns), and anything
  else refused. `submit_price` reports an emptied account as `FeedDeregistered` rather
  than as undecodable bytes, because those go to different people. What no state
  recovers is the *configuration*: only `signers` and `threshold` have an update path
  (M2-08), so a wrong `decimals` or `max_age_ms` is fixed by retiring the feed and
  registering it again rather than by amending it — **except the pair, which is fixed
  from the feed id's first publication.** A retirement empties the feed account and
  leaves the price account untouched, and `submit_price` reads the expected pair off
  the published account on every write but the first, so a re-registration under a new
  pair would produce a feed whose every submission answers `AssetMismatch`.
  `register_feed` therefore reads the price account and refuses that with
  `PairChanged`. No loss: a RedStone feed id *is* its asset, so a different pair is a
  different feed id. And deriving the address removed
  the one escape `Claim::Authorized` had — abandoning the account and re-creating the
  feed at a fresh address under the same id. That is the cost ADR 32 named, paid
  knowingly: two accounts able to answer for BTC/USD with different signer sets is the
  worse hazard.
- **`registered` is not a separate state anyone stores.** A feed is live when its
  derived account is owned by this program and holds a non-empty `FeedAccount`;
  an owned account with empty data is retired, and a default one was never
  registered. Still a read a client can do without this program — but "non-default"
  alone is not the test, because a retirement leaves the account owned and empty.
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
  attack lands on the admin config account, whose address takes no input an attacker
  cannot predict. **Which layer refuses depends on the constraint, and it moved.** While
  the feed account carried `#[account(init)]` the dispatcher answered first, with
  `AccountAlreadyInitialized`; dropping `init` for M2-09's third pre-state moved the
  refusal into the body, where `RegisterError::AlreadyRegistered` is now what a caller
  meets as well as what the pure function returns. The admin config account has no third
  state, keeps `init`, and so still answers `AccountAlreadyInitialized` ahead of
  `AdminError::AlreadyInitialised`. The refusal is permanent either way — this decides
  what an operator reads, not whether the id survives. Not M2-07's to fix and not fixed
  here: recorded in `m0/versions.md` question 4 and pinned by
  `a_derived_address_can_be_squatted_and_this_pins_the_refusal`,
  `a_squatted_feed_account_passes_the_validator_and_the_body_refuses_it` and
  `a_squatted_admin_config_is_refused_by_the_generated_validator`.
- **Every later operation names the feed as well as addressing it.** The constraint
  checks the address; the body checks the id the account stores. Two checks for one
  property, because an account this program owns is not necessarily the feed the
  caller meant — an operator rotating five feeds could otherwise hand the wrong
  account and move the wrong signer set, which is not a mistake anyone notices.
  `update_signer_set` (M2-08) takes `feed_id` for that reason, and M2-09 and M2-10
  will. It is ADR 16's rule about the asset pair applied to the feed itself.
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
- **Derive the feed account from the asset pair.** Rejected already, by ADR 16 and
  again by ADR 32, and for a reason that has not changed: it needs a canonical
  symbol table with no authority behind it.
- **`String` as the seed type.** The natural IDL surface, and it panics above 32
  bytes in generated code. Ruled out by ADR 4 rather than by taste.
