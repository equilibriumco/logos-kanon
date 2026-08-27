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

**Transfer, and no revocation.** #212 has permanent revocation; nothing in RFP-020
asks for it. A revoked authority is a feed whose signer set can never be rotated
again, on infrastructure this engagement operates under an availability target through
March 2028 — RedStone rotating its roster after a revocation would leave every feed
permanently unverifiable, with pausing the only remaining lever. The capability's
downside is unbounded and its upside is a requirement nobody has.

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

**Our own state type, carrying `AdminState`'s field shape and not `AdminConfig`.**
Declared as `{ admin: Option<[u8; 32]> }`, it is byte-identical to `AdminState` under
borsh, so adopting the upstream type later is a swap and not a migration.
`AdminConfig` is that plus a demonstration `config_value: u64`, and the macro reads
`AdminConfig` by name — so the macro and a program with its own config state are
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
- **Deployment grows a step, and the runbook with it.** A build is not operable until
  `initialize` has landed, and the servicing handover has to carry the genesis key's
  custody alongside the operator journey.
- **The genesis key is a build input, so devnet and mainnet builds have different
  program ids.** Already true of any configuration difference under image-id
  addressing, and arguably a property worth having.
- **Losing the genesis key matters only before `initialize`.** After it, the account
  holds the authority and the constant is spent. Before it, recovery is a rebuild —
  which, this early, costs nothing but a redeploy.
- **When #212 lands, the swap is to its library and not to its macro.** Reading
  `AdminState` from the same slot at the same seed is an implementation change behind
  the trait, and byte-identical state makes it a deletion rather than a migration.
- **And that swap would be close to cosmetic, which is worth saying rather than
  filing as an integration.** What RFP-001 was for is a shared admin convention across
  the estate — one shape that tooling, auditors and the next program recognise. Under
  this decision we decline the config type, the macro, the bootstrap and the
  revocation, and adopt a struct of one `Option` and three functions. F6 names RFP-001,
  and this satisfies the naming; it does not deliver the standardisation, because the
  parts carrying the convention are the parts that do not fit. That is a finding about
  #212 rather than a cost of this decision, and it is the more useful thing to send
  Logos than another ask about the merge date: not *when* does it land, but that as
  written it does not fit a program keeping its own config state, and why.
- **ADR 14's "the same surface locally" is now false in both directions.** This
  implements less than #212 — no revocation — and more: a bootstrap #212 has no
  equivalent of, because its sample's `initialize` is open to whoever calls it first.
- **The rebuild-orphans-state problem now has a third instance**: price accounts,
  feed registrations, and the config account. That is a platform property rather than
  a decision of this repository, so it belongs in `m0/versions.md` with the questions
  outstanding with Logos, not here.
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
- **Mirror `AdminConfig` byte for byte, and use the macro.** The migration argument
  for this is moot — our own type is already byte-identical to `AdminState` — so the one
  thing mirroring buys is `#[require_admin]`, which reads `AdminConfig` by name. It
  therefore falls with the macro: six saved lines per instruction, against a security
  gate whose only test lives in the workspace `cargo test --workspace` does not reach,
  and a `config_value` on chain that nothing writes.
- **Wait for #212.** Rejected for the reason ADR 14 gave and time has strengthened: no
  activity in three months, and it is written against a framework three releases old,
  so it needs rework whoever picks it up. M2 gates registering the five day-one feeds.
- **Vendor #212's code.** Rejected in ADR 14 for inheriting unreviewed changes, and
  now also for inheriting the unpersisted writes in its sample.
