# 3. Cryptographic primitives behind a `VerifierBackend` trait

- **Status**: accepted
- **Milestone**: M1 (`M1-10`, `M1-11`)
- **Requirements**: F1
- **Artefacts**: `verifier-core/src/backend/mod.rs`, `verifier-core/src/backend/in_program.rs`

## Context

RFP-020 Functionality 1 asks that the signature path be structured so that swapping in
a host precompile stays a localised change. M0 explains why that is worth paying for:
secp256k1 recovery and signature parsing are **88.97%** of a 3-of-N update — 1,906,737
cycles in total, 5.68% of LEZ's 32M per-transaction budget. A recovery precompile
addresses 89.0% of the program, and LEZ does not offer one today. It is not a clean
order of magnitude, because the framework's own 158,290 cycles survive a cryptographic
precompile.

So the question is not which primitive implementation to use. It is how to keep the
cost of changing that answer proportional to the change itself.

## Decision

Two primitives — `keccak256` and `recover_signer` — behind one trait, `VerifierBackend`,
with a single rule stated in the module's own doc comment:

> **No code outside this module may call a signature primitive directly.**

Everything reaches hashing and recovery through the trait, so a precompile-backed
implementation means writing one new implementor and changing one type parameter,
rather than auditing every call site. `InProgramBackend` (M1-11) is the implementation
a precompile would replace, and everything about it is local to one file.

Four properties are deliberate:

**The trait says nothing about RedStone.** It is two operations over byte slices. That
is what keeps it implementable by a precompile that knows nothing about data packages.

**It is object safe, and there is a test that says so** (`the_trait_is_object_safe`).
Object safety is what would let a caller hold a backend it did not choose at compile
time. The test earns its place by catching a generic method appearing later; it does
not, on its own, argue against the alternative below.

**`recover_signer` takes a digest, not a message.** The caller needs the hash
separately anyway, and hashing twice would be measurable against a budget where
recovery already dominates.

**Recovery-id normalisation is the backend's job.** Ethereum tooling emits both the
`0/1` and `27/28` conventions, and RedStone signers are Ethereum tooling. Accepting
both is interoperability rather than laxity: the recovery byte carries no authority,
because a wrong recovery id yields a *different* address, which then fails the
signer-set check.

`BackendError` is deliberately coarse — `InvalidRecoveryId`, `InvalidSignature`,
`RecoveryFailed`. A caller cannot act differently on a malformed `r` than a malformed
`s`, and a verifier that reports precisely which component was wrong tells an attacker
more than it tells an operator. The typed enum the *whole* verification path reports
through is a separate thing (M1-18, U6); this is only what a backend can distinguish.

High-`s` signatures are rejected, by using `k256`'s `Signature::from_slice`, which
refuses a non-canonical `s`. Every ECDSA signature has a second valid form with `s`
replaced by `n − s`, so accepting both makes a package malleable and gives it two
distinct encodings — which defeats any replay defence keyed on signature bytes.

## Consequences

- A **precompile** swap is one new implementor and one type parameter, which is what F1
  asks for and what the proposal promised. A **scheme** swap is not: it needs a method
  signature change. The proposal's §2 claim of "no change to the aggregator, the pull
  library, the SDK surface, or the tests" holds for the first case and not the second,
  and that narrowing should be stated to Logos at the M1 gate review rather than left
  implicit.
- A precompile arriving in LEZ is a one-file change plus a type parameter, and the
  cost of being ready for it is one trait.
- The same source compiles against RISC Zero's accelerated forks without any change
  here, because the substitution is a `[patch.crates-io]` in the guest manifest and is
  invisible at this level. That is exactly what made M0's three-way comparison
  possible, and it is the mechanism ADR 6 depends on.
- `InProgramBackend` uses the same `k256` and `tiny-keccak` M0 measured, so the
  published cycle figures describe *this* code rather than something adjacent to it.
- The coarse error enum means a debugging session on a genuinely malformed signature
  has less to go on. Accepted deliberately.

## Alternatives considered

- **Free functions, swapped by `cfg`.** Rejected: a compile-time switch cannot express
  a runtime choice, and `cfg` scatters rather than localises.
- **A trait that takes a whole data package.** Rejected: it would make the backend know
  about RedStone, which is what stops a generic precompile implementing it.
- **Hashing inside `recover_signer`.** Rejected on cost — the caller needs the digest
  anyway, and recovery already dominates the program at 88.97%.
- **The generic trait in the accepted proposal.** RFP-020's proposal (`logos-co/rfp#117`,
  §2) specifies `type Signature; type Signer; type Error;` and states that the
  associated types are what admit "a Schnorr/BIP-340 impl (the FROST-friendly scheme for
  the private-pull follow-on)". Rejected, but **not** for object safety: associated types
  preserve it, and the proposal's trait compiles as
  `&dyn VerifierBackend<Signature = …, Signer = …, Error = …>`.

  It was rejected on arity. `recover(digest, sig) -> Signer` has no parameter for a
  claimed signer, and a scheme without key recovery needs one — the proposal concedes as
  much in the same comment ("Schnorr has no key recovery, so that backend implements
  `recover` as verification against the claimed signer"). So the associated types did not
  buy what they were described as buying. Two further costs: RedStone fixes the signature
  at 65 bytes on the wire, so an associated `Signature` would propagate generics through
  `decode` and both modes to model one concrete case; and an associated `Error` prevents a
  caller matching the specific variants U6 requires.

  The seam a recovery-less scheme would actually need is a candidate-set method —
  `attribute(digest, sig, &[SignerAddress]) -> Option<usize>` — addable as a provided
  method without touching existing implementors. Not added: its only consumer would be a
  backend that does not exist, and `verify_feed` does its own membership lookup in three
  lines.
