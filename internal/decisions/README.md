# internal/decisions/

A decision record states **what is true, why it is acceptable, what would force a re-review, and what
holds it in place** — including what the enforcing mechanism does *not* prove.

They exist because a fail-closed default makes *deliberately excluded* and *not written yet*
indistinguishable from the code. Where a decision is load-bearing but invisible, the record is what
tells the next reader which one they are looking at.

## Conventions

- **One file per decision**, named `YYYY-MM-DD-<the-claim-as-a-statement>.md`.
- **Cite immutable anchors** — file path plus symbol name, migration filename, SQL function name, or
  test name. Never `file.rs:123`; line numbers rot.
- **Attribution follows `internal/README.md`.** `[provisional — <date>, judgement call]` is the
  default. `[ruled — <date>, <name>]` requires a citable record and is not to be used without one.
- **Back-pointer from the enforcement site.** Where practical, the code that enforces a decision names
  its record in a comment — as `admin_ledger_service.rs` does for the 2026-07-19 entry.

- **Name a record by the property it settles, never by the state of a control.** A filename is a
  disclosure surface on its own: it is greppable, permanent, and reaches a reader who never opens the
  file or sees any surrounding context. *"The MCP surface's gate is its bearer token"* and *"rmcp's
  DNS-rebinding protection is disabled"* document the same decision; only one of them is also an
  index entry for someone enumerating disabled controls.

### What belongs here, and what does not

This directory is **not** synced to the public documentation site, but it **is** in a public
repository. `internal/` means *not documentation*, not *not visible*.

So the test for a record here is **"is this safe to publish?"** — which is not the same question as
*"is it settled?"*, and the two come apart at exactly a **deliberately-accepted risk**. An accepted
residue is settled by definition and may still be the last thing worth publishing, because it is a
standing property of a live deployment rather than a defect in a superseded one.

A record of a decision we hold deliberately, whose disclosure tells a reader nothing they could act
on, belongs here. Something we intend to change belongs with the work, written once the work has
landed and describes a closed thing. Deployment-specific parameters belong to the instance's own
record rather than to the repository.

## The trust-assumption sweep — 2026-08-26

Six records written in one sitting, from a review of the trust assumptions this system makes but had
never stated. Task `01a035f2-d37a-7a83-9f6c-b93d58eb5847`.

**Every claim was re-derived against `main` rather than carried over from the review notes**, which
were written some thirty commits earlier. All six needed correction on at least one point, and
several on more than one — which is the argument for the exercise: an assumption nobody has re-checked
is not yet a decision, however confidently it is held.

| Record | What it settles |
|---|---|
| [The database is the outermost trust ring](./2026-08-26-the-database-is-the-outermost-trust-ring.md) | Where the event ledger's coverage stops, and what the operator owns below that line |
| [Stored region aggregates are region-truth](./2026-08-26-stored-region-aggregates-are-region-truth.md) | Why region readouts are materialize-time, and what bounds the disclosure |
| [The visibility verdict is computed once and carried](./2026-08-26-the-visibility-verdict-is-hoisted-once.md) | Why the verdict is hoisted, and what actually holds the invariant |
| [Cogmap genesis owns its bootstrap trust](./2026-08-26-cogmap-genesis-owns-its-bootstrap-trust.md) | Why the genesis exception is one named type with one call site |
| [The MCP surface's gate is its bearer token](./2026-08-26-the-mcp-surface-gate-is-its-bearer-token.md) | Why host validation is not the right control for this deployment shape |
| [The Slack mint gate — enablement checklist](./2026-08-26-slack-mint-enablement-checklist.md) | The preconditions an operator must satisfy before enabling the Slack surface |

### Rows from the sweep that are not recorded here

**Two are being taken up as work rather than recorded as settled.** Recording a choice as accepted
is a claim that we intend to keep it; where that is not what we intend, the honest move is to change
the thing and write the record afterwards, describing what now holds. Those records will land with
their work.

**One is deployment-specific** — parameters of a particular instance's release process rather than a
property of this codebase — and belongs in that instance's own record, per the boundary above.

**One is deliberately not recorded at all.** Whether a demoted originator retains access to resources
they still own is held open on purpose: recording it as accepted policy while its failures remain
hard to detect would record a policy nobody could check. It is a **declared hole, not a filed task**,
and it belongs to the goal *A departure de-provisions itself, and the ledger can say who revoked whom*
(`01a035eb-3aea-7ea0-9dd3-f13acdf8cb36`), which states the same position in its own *Stated silence*.

Its two blockers have both landed — the ledger read arms
(`01a035eb-cf9a-7942-b0ef-f31671614b9f`) and de-provisioning's trigger
(`01a035ec-5738-7f21-9387-fafcbe12da5f`). The detectability half has not: that goal still declares
`a-de-provisioning-that-did-not-happen-is-visible-to-an-operator` and
`how-stale-a-principal-s-reach-is-is-a-question-the-system-answers` uncovered, and the staleness task
(`01a03893-e2bf-7973-b885-54978e6088f6`) is in backlog. **The absence of a record here is the
decision, not an omission.**

**One is scheduled as its own writing.** The steward prompt-injection threat model is a document
rather than a record, filed as task `01a035f3-3d18-7132-b745-d78715dcb6c2` with its trigger already
named: before external-emitter content lands in the KB, or before any relaxation of an agent's
human-in-the-loop confirmation.
