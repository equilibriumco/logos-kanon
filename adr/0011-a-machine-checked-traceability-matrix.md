# 11. A traceability matrix that is generated and machine-checked, not maintained

- **Status**: accepted
- **Milestone**: M1 (`M1-06`; closed by `M5-06`)
- **Requirements**: S3
- **Artefacts**: `traceability/`, `TRACEABILITY.md`, `ci.yml` job `traceability`

## Context

RFP-020 Supportability 3 asks for at least one test per hard requirement, per mode,
across a named list of cases. The conventional answer is a table in a document mapping
requirements to tests.

The conventional answer fails predictably. A hand-maintained matrix drifts into fiction
by the third milestone: a test is renamed, a CI job is removed, a row claims
`verified` on the strength of a paragraph, and nothing anywhere notices. The table then
actively misleads, because it looks like evidence.

## Decision

The matrix is **generated from two TOML sources and checked against the repository**.

- `traceability/requirements.toml` — the inventory, transcribed from RFP-020.
- `traceability/matrix.toml` — the mapping: status, tasks, `implemented_in` paths, and
  `verified_by` evidence in `test:`, `ci:` or `doc:` form.
- `TRACEABILITY.md` — rendered from both. Marked *Generated. Do not edit.*

`cargo test -p traceability` fails if:

- a requirement has no row;
- a row names a test function, CI job or document that does not exist;
- a task id written anywhere in the repository appears in no row (the reverse check —
  this is what catches an orphaned task);
- the rendered file has drifted from its sources.

**A row cannot claim `verified` on the strength of a document alone.** That rule is what
makes S3 a fact about the repository rather than a claim in a table.

Three statuses, and they mean specific things: `planned` (nothing implemented),
`partial` (partly satisfied, or satisfied by M0 evidence that later tasks extend),
`verified` (satisfied now, with a test or CI gate behind it). S3 is itself `partial` by
construction until M5-06 leaves every row verified.

The renderer and the checker are deliberately separate modules. Keeping them apart means
the renderer cannot quietly paper over a missing test by omitting it.

## Consequences

- The document is trustworthy in a way a maintained one cannot be: the current count is
  **0 verified, 8 partial, 28 planned**, of 36 requirements, and it is accurate because
  no other count was expressible. OS1 was the one `verified` row until review showed the
  CI evidence proved the gate ran, not that the requirement was met.
- Renaming a test breaks the build until the matrix is updated. That is the cost, and it
  is the mechanism.
- Adding a task id anywhere in the repository without a matrix row breaks the build,
  which is how five orphaned task ids were caught rather than shipped.
- The Servicing and SLA obligations are explicitly **not** tracked here: they are
  contractual rather than code and have no test to name. Monthly operating reports are
  their evidence, and saying so in the document is better than a row that can never go
  green.
- `TRACEABILITY.md` is rendered rather than written, so its formatting is the renderer's
  problem — including the non-breaking hyphens in identifier cells that stop GitHub
  wrapping `M1-10` across two lines.

## Alternatives considered

- **A hand-maintained table.** Rejected for the reason above; it is the failure mode
  being designed against.
- **Doc comments as the source of truth, extracted.** Rejected: it cannot express a
  requirement that *nothing* implements yet, which is most of them, and the empty rows
  are the useful part.
- **An external tracker.** Rejected: evidence has to be resolvable against the
  repository at the commit being checked, which an external system cannot do.
