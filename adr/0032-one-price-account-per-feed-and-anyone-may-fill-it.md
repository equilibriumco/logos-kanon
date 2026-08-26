# 32. One price account per feed, and anyone may fill it

- **Status**: accepted
- **Milestone**: M2 (`M2-02`)
- **Requirements**: F1, F5, P1
- **Artefacts**: `aggregator-program/src/submit.rs`,
  `methods/guest/src/bin/aggregator.rs`, `kanon-idl/aggregator-idl.json`,
  `MANIPULATION-ANALYSIS.md`

## Context

`submit_price` is the first instruction with a body, and filling it forced three
questions that the surface M2-01 settled had left open. Each one is a property of
the account layout rather than of the code, which is why they are recorded
together: they are the shape a relayer, an SDK and a consumer all have to agree
on.

**Nothing tied the feed to the account it writes.** A caller supplies both. The
write already refuses an account priced in another pair, and that catches a
misrouted write only when the pairs differ — so two feeds registered against the
same pair, on different data services or different signer sets, could each write
the other's account. The canonical account carries no field that would say which
of them produced the number it holds: the six fields are RFP-019's, and
`source_id` is the same constant for every Kanon feed
([ADR 22](0022-the-price-account-write-and-its-three-undecided-fields.md)).

**Nothing created the account.** None of the six instructions takes a price
account except this one, and it takes an existing one.

**Nothing decided who may submit.** `MANIPULATION-ANALYSIS.md` carried the
question as an explicit dependency on this task, because how far an attacker with
some of the signing keys can get depends on it.

## Decision

**One price account per feed, at an address derived from the feed's own.**

```
combined = sha256( feed_account_id[32] || zero_pad_32("KANON_PRICE_ACCOUNT") )
address  = AccountId::for_public_pda(program_id, PdaSeed::new(combined))
```

Declared as `#[account(mut, pda = [account("feed"), r#const("KANON_PRICE_ACCOUNT")])]`
rather than checked in code, because the derivation is what has to be reproduced
outside this repository and the constraint is what publishes it: the generated
IDL carries the seeds.

**The first submission creates the account; there is no second instruction.**
LEZ permits a data write when the whole pre-state is default and a claim while
the owner is default, so a first submission claims the address the constraint
checked and every later one writes as owner.

**Submission is permissionless.** The transaction's sender is never consulted.
Authenticity comes from the registered signer set and nowhere else, because a
sender attests to nothing about the bytes it carries. Restricting it would help
only to the degree that the relayer assembles payloads from packages it fetched
itself, which is a discipline for the relayer (M2-13) and not a check the program
can make. RFP-020 names "a relayer (or any caller)" as who may submit.

**The pair `verify_feed` is given is the one the account publishes**, and the
feed's own only on a first write.
[ADR 16](0016-the-asset-pair-is-a-registration-claim.md) makes that argument the
caller's claim against the registration; in push mode the standing claim is what
consumers are already reading. It moves the refusal of a misrouted write ahead of
the signature recoveries instead of after them.

**The feed's ownership is checked in Rust, not by a constraint.** SPEL's IDL
generator parses `owner` and discards it, so `#[account(owner = self_program_id)]`
would read as published and reach no generated client. The rule instead: declare
what clients must derive, check in code what only this program can explain.

## Consequences

- **A consumer needs the feed account's id, not just an asset pair.** Upstream
  asks the same — `compute_oracle_price_account_pda` takes a `price_source_id` —
  and how a consumer finds it is M2-04's read path.
- **Two feeds on the same pair are possible and are two accounts.** That is a
  capability: a second data service, or an A/B migration between signer sets, has
  somewhere to live. "One feed per pair" is an operational property of the admin
  path (M2-06, M2-07) rather than a structural one.
- **The price account has three states, and the third is refused.** An account
  that holds data but is still unowned is neither a create nor an update this
  program may perform: `validate_execution` refuses the data write, because the
  pre-state is not default and this program is not the owner, and refuses the
  post-state too, because the claim loop assigns ownership only after validation
  has run. Deciding the claim from the owner alone — which is what
  `new_claimed_if_default` does — would classify it as a create and hand the
  chain a post-state it rejects, naming none of this. Both the claim and the
  write come from one full-default comparison instead.
- **`MANIPULATION-ANALYSIS.md` is no longer contingent.** Its signer-compromise
  section held two readings depending on this decision, and now holds one.
- **Every leaf cause keeps its own error code.** `SpelError` carries a number and
  a string, so wrapping the four typed errors would have bought nothing if the
  number collapsed them: a malformed envelope and a malleable signature are
  different numbers reached through the same outer variant. A cause reachable two
  ways keeps one number, because these name causes rather than enum positions.
- **The guest workspace is now tested and formatted in CI.** The dispatcher and
  the constraints exist only after macro expansion, so the PDA check has no
  reachable test anywhere else. Turning the formatting gate on also found drift
  that an ungated workspace had accumulated.
- **A rebuilt aggregator still abandons its accounts.** The program id is hashed
  into every address, so a new build derives a fresh set and the old ones freeze
  holding a plausible, permanently stale price. ADR 22 records it and points at
  M2-04 and M2-06; deriving per feed neither helps nor worsens it.
- **A mis-registered feed can only be abandoned.** Of the seven stored fields
  only the signer set and threshold have an update path (M2-08), and ownership is
  never released, so a wrong `decimals` or `max_age_ms` is permanent for that
  feed. It is why the *feed* account is not itself derived from the asset pair —
  see below — and it is worth raising against M2-07, whose shape would change.

## Alternatives considered

- **Derive the price account from the asset pair.** It reads like the way to make
  one price per pair structural and does the opposite: both feeds derive the same
  address, the first submission claims it, and thereafter both write it. Strict
  newness becomes a race between relayers and the account cannot say which signer
  set stands behind the number.
- **Derive the *feed* account from the asset pair.** This one does enforce
  uniqueness — a second registration hits the same address and SPEL's `init`
  check refuses it, because the account is no longer default. Rejected because
  the enforcement is permanent and unrecoverable: ownership can never be
  released, `deregister_feed` can zero a feed's data but not un-own its account,
  and `init` compares against a default *account*, which an owned one with zeroed
  data still fails. One registration with the wrong exponent would burn that pair
  for the lifetime of the program build.
- **Store the price account's id in `FeedAccount`.** Explicit, and it moves the
  decision into M2-07: `submit_price` would only be checking a binding
  established elsewhere. It also widens the stored state and the IDL for
  something a hash already determines.
- **A seventh instruction that creates the account.** Cleaner separation, and it
  makes a feed unusable until two transactions have landed, adds an instruction
  the SDK and the relayer both have to learn, and buys nothing that
  `new_claimed_if_default`'s underlying rule does not already give.
- **Restrict submission to an authorised relayer.** Rejected on the grounds
  `MANIPULATION-ANALYSIS.md` already gave: it protects only against submitters,
  and a submitter is not what a payload's authenticity rests on.
- **Give `SubmitError` one code per variant.** Eight numbers instead of forty, and
  it would undo the reason M1 kept the error taxonomy discriminating at all
  ([ADR 23](0023-the-accept-and-reject-suite-is-shaped-by-the-contract.md)).
