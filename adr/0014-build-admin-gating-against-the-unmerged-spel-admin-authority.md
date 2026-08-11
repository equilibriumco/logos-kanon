# 14. Build admin gating against the unmerged SPEL admin-authority library, with a shim held in reserve

- **Status**: accepted, contingency live
- **Milestone**: M1 (`M1-07` answered; built in `M2-06`–`M2-10`)
- **Requirements**: F6, SEC2
- **Artefacts**: `logos-co/spel` PR #212 upstream; tracked in `m0/versions.md` (*Open: questions outstanding with Logos*, item 2)

## Context

F6 requires an admin authority — RFP-001, via SPEL — that can register feeds, update a
feed's signer set on roster changes, and deregister feeds. SEC2 requires that in push
mode the stored signer set be updatable **only** by that authority, with the update path
itself tested.

Both depend on an interface defined outside this repository. M1-07 was the task of
establishing what that interface is.

What exists: RFP-001 was awarded and its library is written. Milestones M1 and M2 are
closed (`logos-co/rfp#131`, `#132`, grantee `mmlado`, parent proposal `#46`); M3 —
tests, documentation, delivery — is still open at `#133`.

The implementation is **`logos-co/spel` PR #212**, *"feat(rfp-001): add
spel-admin-authority library and `#[require_admin]` macro"*. It adds a
`spel-admin-authority` crate exporting `AdminConfig`, held Borsh-encoded in a config
PDA, with `initialize`, `transfer_admin` and `revoke_admin`; a `#[require_admin(config)]`
attribute macro in `spel-framework-macros`; and a sample program demonstrating it.

**It is open, and has had no activity since 2026-05-20.**

## Decision

Design F6 and SEC2 against `#[require_admin(config)]` plus `AdminConfig`. That shape is
demonstrated and stable enough to build M2-06 through M2-10 on, and building against a
different shape would mean redoing the work if — as is likely — the PR eventually lands.

**The shim contingency in M2-06 stays live regardless.** The PR sits in a repository
outside this one, and nothing says it merges before M2 needs it. The shim implements the
same surface locally, so the admin-gated instructions are written once either way and
the swap is a dependency change rather than a redesign.

## Consequences

- M2's admin instructions can be specified and written now, rather than blocking on
  an external review queue.
- If #212 merges before M2-06, the shim is deleted and nothing else moves. If it does
  not, the shim ships alongside the same instructions.
- The question to Logos is now narrow, and that narrowing is the deliverable of M1-07:
  not *"is there an admin-authority interface"* but *"is #212 merging, and when"*.
- One naming trap, recorded so it is not rediscovered: the milestone text promises
  `renounce_admin` where the code implements `revoke_admin`.
- SPEL's version question is separate and also unresolved — SPEL stays pinned to v0.1.2
  regardless of which LEZ the sequencer runs, so the aggregator program in M2-01 is where
  the version skew actually bites. A v0.2.x-aligned SPEL exists (scaffold `3d639076`
  vendors v0.2.0-rc3 against the default `73fc462e`'s v0.1.2) and is the first thing to
  try. See ADR 8 and `m0/versions.md`.

## Alternatives considered

- **Wait for #212 to merge.** Rejected: no activity since 2026-05-20, and M2 has a date.
- **Design a local admin-authority model and ignore RFP-001.** Rejected: F6 names
  RFP-001 explicitly, and diverging would leave a second admin convention in the estate.
- **Vendor the PR's code.** Rejected in favour of a shim over the same surface — vendoring
  an unmerged branch means inheriting unreviewed changes, and a locally written shim is
  easier to delete.
