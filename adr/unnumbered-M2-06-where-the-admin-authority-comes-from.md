# [M2-06:01]. Where the admin authority comes from, and how it is established

- **Status**: accepted, unnumbered
- **Milestone**: M2 (`M2-06`)
- **Requirements**: F6, SEC2
- **Supersedes**: [14](0014-build-admin-gating-against-the-unmerged-spel-admin-authority.md),
  on the shape of the shim rather than on the choice of authority
- **Artefacts**: `aggregator-program/src/admin.rs`,
  `aggregator-program/src/instruction.rs`, `methods/guest/src/bin/aggregator.rs`,
  `kanon-idl/aggregator-idl.json`

## Context

F6 needs an authority that can register a feed, replace its signer set on a roster
change, deregister a feed and pause one. SEC2 needs the stored signer set updatable
**only** by that authority, with the update path itself tested.

[ADR 14](0014-build-admin-gating-against-the-unmerged-spel-admin-authority.md)
answered *which* authority: RFP-001's, through `logos-co/spel` PR #212, designed
against its `#[require_admin(config)]` and `AdminConfig`, with a local shim held in
reserve because "the shim implements the same surface locally, so the admin-gated
instructions are written once either way".

Three months on, the contingency is the live branch — #212 has had no activity since
2026-05-20, and SPEL has shipped v0.6.0 and moved to LEZ v0.2.4 since it was
written. And the reserve is not the like-for-like substitution ADR 14 assumed,
because the surface it named is larger than a check.

**What #212 provides.** `AdminState { admin: Option<[u8; 32]> }` with
`assert_admin`, `transfer_admin` and `revoke_admin`; an `AdminConfig` wrapping it
alongside a demonstration `config_value: u64`; and `#[require_admin(config)]`, which
`#[lez_program]` expands into a read of `AdminConfig` from the named account followed
by `assert_admin` against the signer. The state lives in a config PDA at seed
`literal("config")`, written by an `initialize` instruction in the sample program.

Two properties of it bear on this decision. The macro reads `AdminConfig` **by name**,
so a program that embeds `AdminState` in its own state — which the library's own
README offers as an option — cannot use the macro at all. And the sample's four
instructions each build a post-state and then return the account unmodified, so
nothing they mutate is persisted; their tests assert `is_ok()` and so do not reach it.

**What this program has.** Six instructions, none of which takes a config account,
and none of which could establish one. `register_feed` takes an `init` feed account
and a signer. `#[account(signer)]` expands to
`if !accounts[i].is_authorized { Unauthorized }` in the dispatcher — it proves that
*someone* signed, never *who*. With the feed account default on a first write there is
nothing stored to compare a claimed admin against.

So M2-06's own text, "thin admin shim / trait wrapping the RFP-001 authority
**check**", describes the half that is an afternoon. No task in M2 owns the half that
decides where the authority is *stored*, because the plan expected RFP-001 to supply
it.

## Decision

**The authority is an account the transaction passes in.** By elimination, not by
preference. The feed account cannot hold it, because the operation that most needs
guarding is the one that creates the feed, and it is default until then. A
compile-time-only authority cannot hold it either: the program id is the RISC0 image
id, `AccountId::for_public_pda` hashes the program id into every address, so rotating
the authority by rebuilding strands every account the previous build created — the
hazard [ADR 22](0022-the-price-account-write-and-its-three-undecided-fields.md)
already records for price accounts. And the runtime cannot be asked: SPEL's
`ProgramContext` carries `self_program_id` and `caller_program_id` and nothing else,
and LEZ's `Account` is `{ program_owner, balance, data, nonce }`. No deploy or upgrade
authority exists anywhere to consult.

**One config account for the program, at a derived address, and an `initialize`
instruction appended to the enum.** Appended rather than inserted: the variant's
position in the declaration is the discriminant a caller encodes, so inserting one
renumbers every instruction after it.

**`initialize` is gated on a compile-time genesis key, and hands the live authority to
the account.** An open `initialize` is not a launch-day race that careful deployment
wins once. Because PDAs hash the program id, and the program id is the image id, every
rebuild of this program presents a **fresh, default config account at a new address**.
First-caller-wins therefore recurs after every single deployment, and whoever wins owns
the aggregator: they set the signer sets that decide what a price means. The genesis
key closes that for every build rather than for the first one. Ordinary rotation does
not touch it, because the authority that instructions check is the account's.

**A handover is two steps: nominate, then accept.** A one-step transfer accepts any
key the authority names, and nothing makes the named key prove it can sign. A typo, an
account nobody holds, or a PDA that cannot sign becomes the authority immediately — and
that state is terminal: registration, roster rotation, deregistration and the emergency
pause all stop, `initialise` refuses a config account that is no longer default, and
rebuilding to recover changes the program id and strands every account the build
created. Refusing the zero key, which an earlier draft of this record did, guards one
value of a hazard whose whole class is unrecoverable.

So a nomination is stored and is not the authority. It becomes the authority only when
the nominated key signs for itself, which a key nobody controls can never do; a mistyped
nomination costs a second nomination. Two steps rather than a co-signed transfer because
the one handover this engagement has actually committed to — the servicing handover at
the end of the operating period — is asynchronous and between two parties, and a
co-signature would make it a coordination exercise across organisations.

**Revocation is exposed.** An earlier draft of this record omitted it: nothing in
RFP-020's own text asks for it, and a revoked authority is a feed whose signer set can
never be rotated again. Nor does the feed go quiet, which is the half that earns this
paragraph its place: `submit_price` is permissionless and ungated, so it keeps
publishing against a set nobody can change, and it cannot be paused or deregistered
either — pausing is gated on the authority that was given up. What has been lost control
of is still running. That reasoning survives as a hazard to document, and it does not
survive as a reason to narrow the contract: F6 names RFP-001's authority, and revocation
is one of RFP-001's four hard functionality requirements. Omitting it is a scope
variance to agree with Logos, not a local product choice — and since `None` is already a
stored state every read refuses, exposing it costs one instruction, which is cheaper
than the conversation. Revocation clears any pending nomination with it, or a nominee
could accept afterwards and take an authority its holder had given up.

**The check runs in `aggregator-program`, not in a macro on the guest handler.**
`#[require_admin(config)]` injects its check into `#[lez_program]`-expanded dispatcher
code, and code that exists only after macro expansion has no reachable test on the
host: the guest's own note about the PDA constraint says it plainly — nothing in
`aggregator-program` can reach it and no host test in that crate can either, which is
why those tests live in the guest workspace and why `cargo test --workspace` never
runs them. SEC2 does not only require that the signer set be admin-gated; it requires
the update path to be *tested, including the unauthorised-caller reject*. Putting that
gate in expanded guest code moves its only test into a workspace CI runs separately
from the suite a contributor runs. This is the same reason `submit_price` checks the
feed's owner in Rust rather than leaning on a constraint
([ADR 32](0032-one-price-account-per-feed-and-anyone-may-fill-it.md)).

**Our own state type, and not `AdminConfig`.** `{ admin: Option<[u8; 32]>, pending:
Option<[u8; 32]> }`. The `admin` field alone would have been byte-identical to
`AdminState` under borsh, and an earlier draft of this record claimed that as a reason
to keep it — adopting the upstream type later would be a deletion rather than a
migration. The nomination field ends that: borsh consumes its input exactly, so a
two-field account does not decode as a one-field one. The trade is worth taking, and
plainly: byte identity bought a cheaper version of a swap this record already assesses
as close to cosmetic, while the second field is what stops the authority being bricked
by a typo. Pre-mainnet the migration it costs is one `initialise` on a fresh config
account.

`AdminConfig` is `AdminState` plus a demonstration `config_value: u64`, and the macro
reads `AdminConfig` by name — so the macro and a program with its own config state are
mutually exclusive whatever the field costs. Having declined the macro on testability,
there is nothing left to buy the integer with.

**All of it behind one trait in `aggregator-program`, with a single local
implementation**, so the instructions are written once and the swap is a change of
implementation rather than of call sites — which is the part of ADR 14 that still
holds.

## Consequences

- **Five instructions each gain an account, and the IDL regenerates.** This is the
  moment to do it: the SDK is thirteen lines, the relayer eighteen, the CLI does not
  exist, and the only consumers of the instruction surface are two in-repo tests. The
  same change after M4 touches everything generated from the IDL.
- **The admin key must be an account some program owns, and the program refuses one that
  is not.** Ownership is the property, not existence: an account can exist and hold a
  balance and still be owned by nobody, in which case it is already stranded. LEZ
  increments every signer's nonce after applying a state diff, outside program execution
  (`state.rs:212-216`), and rule 7 refuses a post-state whose program owner is the
  default one unless the pre-state was pristine (`program.rs:725`). So a default-owned
  key passes exactly once, while it is still pristine, and is refused in every
  post-state afterwards — of any program, not only this one, with whatever balance it
  holds frozen with it. `initialise` and `accept` therefore refuse a default-owned
  account with `AdminUnowned`, because the alternative was establishing an authority
  that could never be exercised, and on `accept` doing it irreversibly: acceptance moves
  the authority first, and `initialise` refuses a config account that is no longer
  default. `Claim::Authorized` would rescue a *pristine* key — the claim is honoured
  before the nonce bump, so the account comes out program-owned — and this program
  deliberately does not take that route: rule 4 would make that ownership permanent and
  rule 5 would let this program move the operator's balance. What the claim does not
  rescue is a key that has already transacted unclaimed, which is the case
  `an_unowned_admin_that_has_transacted_cannot_be_used` pins. The runbook and the
  servicing handover still have to carry the requirement, because a typed refusal at
  deployment time is cheaper than a diagnosis.
- **Deployment grows a step, and the runbook with it.** A build is not operable until
  `initialize` has landed, and the servicing handover has to carry the genesis key's
  custody alongside the operator journey.
- **The genesis key is a build input, so devnet and mainnet builds have different
  program ids.** Already true of any configuration difference under image-id
  addressing, and arguably a property worth having.
- **Losing the genesis key matters only before `initialize`.** After it, the account
  holds the authority and the constant is spent. Before it, recovery is a rebuild —
  which, this early, costs nothing but a redeploy.
- **When #212 lands, the swap is to its library and not to its macro, and it is a
  migration.** `AdminState` has no nomination field, so adopting it means either
  embedding it beside our own `pending` or giving up the two-step handover. Reading it
  from the same slot at the same seed is still an implementation change behind the
  trait; it is no longer a deletion.
- **And that swap would be close to cosmetic, which is worth saying rather than
  filing as an integration.** What RFP-001 was for is a shared admin convention across
  the estate — one shape that tooling, auditors and the next program recognise. Under
  this decision we decline the config type, the macro and the bootstrap, and adopt a
  struct of two `Option`s with four lifecycle functions plus `authorise`. F6 names RFP-001,
  and this satisfies the naming; it does not deliver the standardisation, because the
  parts carrying the convention are the parts that do not fit. That is a finding about
  #212 rather than a cost of this decision, and it is the more useful thing to send
  Logos than another ask about the merge date: not *when* does it land, but that as
  written it does not fit a program keeping its own config state, and why.
- **ADR 14's "the same surface locally" is now false, in the direction of more rather
  than less.** Every operation #212 has is here; on top of it are a genesis-gated
  bootstrap, which #212 has no equivalent of because its sample's `initialize` is open
  to whoever calls it first, and a nomination step, which #212's `transfer_admin` has
  no equivalent of because it moves the authority on one signature.
- **The rebuild-orphans-state problem now has a third instance**: price accounts, feed
  registrations, and the config account. A PDA hashes the program id and the program id
  is the image id, so a rebuild presents fresh addresses and strands everything the
  previous build created. That is a platform property rather than a decision of this
  repository, and it is not yet written down anywhere: `m0/versions.md` carries the
  questions outstanding with Logos but has no section on this one, so it is recorded
  here until it does.
- **One feed per pair stays operational rather than structural.** [ADR
  32](0032-one-price-account-per-feed-and-anyone-may-fill-it.md) left that property to
  the admin path; this decision is what makes it enforceable at all, and it is enforced
  by whoever holds the authority rather than by the layout.

## Alternatives considered

- **An open `initialize`, first caller wins, deployed carefully.** Rejected on the
  recurrence: it is not one race but one per deployment, and a program whose authority
  can be taken by a mempool watcher after any rebuild cannot be operated under an SLA.
- **A compile-time authority and no config account.** The smallest change, and it
  cannot express transfer. Rotation would mean a rebuild, a new program id, and every
  price account frozen holding a plausible permanently-stale price — the failure ADR 22
  describes, chosen deliberately instead of inherited.
- **The admin key per feed, in `FeedAccount`.** No wire change at all, and circular:
  registration is the operation that would need the authority it is establishing, so
  the first registration of every feed is unauthenticated and the program's
  registrations mean nothing.
- **A one-step transfer, refusing only the zero key.** What this record decided first,
  and wrong: it guards one value of a hazard whose class is unrecoverable, and the
  terminal state it admits stops every administrative instruction with no path back.
- **A one-step transfer co-signed by the incoming authority.** Sound, and one field and
  one instruction cheaper. Rejected on the handover: the incoming authority is another
  organisation at the end of the operating period, and requiring both signatures in one
  transaction makes a cross-party handover a scheduling problem rather than two
  independent steps.
- **Mirror `AdminConfig` byte for byte, and use the macro.** Byte identity is already
  given up by the nomination field, so the one thing mirroring buys is
  `#[require_admin]`, which reads `AdminConfig` by name. It
  therefore falls with the macro: six saved lines per instruction, against a security
  gate whose only test lives in the workspace `cargo test --workspace` does not reach,
  and a `config_value` on chain that nothing writes.
- **Wait for #212.** Rejected for the reason ADR 14 gave and time has strengthened: no
  activity in three months, and it is written against a framework three releases old,
  so it needs rework whoever picks it up. M2 gates registering the five day-one feeds.
- **Vendor #212's code.** Rejected in ADR 14 for inheriting unreviewed changes, and
  now also for inheriting the unpersisted writes in its sample.
