# Reblock a corpus

**For deployment operators** — someone who administers a Temper deployment and wants existing
resources partitioned under the current blocking policy: a corpus that predates the policy, or
one affected by a policy change.

## Outcome

By the end of this playbook you will have surveyed your corpus without changing it, walked it
under the blocking policy in bounded, verified batches, read each batch's receipt and acted on
what it names, and validated the walk against the ledger — knowing at every step exactly how far
the operation has gotten and why.

## Prerequisites

- **Temper installed and authenticated** — see [Install Temper](./install-temper.md) and
  [Authenticate](./authenticate.md).
- **Comfortable with refs** — resources and contexts are addressed by refs; see
  [Contexts and refs](../concepts/contexts-and-refs.md).
- **The rights you already have are the rights you get.** Every resource the operation touches
  is gated by your own update rights on that resource — walking a batch never widens what you
  could have done one resource at a time.

## What one act does

One re-blocking act re-partitions one resource's content blocks to the current blocking policy
(currently: documents partition at heading-aligned sections). What it never changes: the
resource's content, its retrieval behavior, or the addresses into blocks whose content is
unchanged. Block-level attribution is preserved — attribution into folded blocks is carried, and
carried attribution stays distinguishable from asserted attribution.

The corpus walk is many such acts: each invocation considers a bounded window of candidates,
acts on the ones it can, and returns a receipt you can verify before continuing.

## The loop: survey → act → re-survey

Every batch runs the same three steps:

1. **Survey** (`--dry-run`) — classifies every candidate in the window without touching
   anything.
2. **Act** — the same machinery, now re-blocking.
3. **Re-survey** — the same survey again. The remaining `would_change` count drops by exactly
   the number the act re-blocked; the second survey is the batch's evidence.

Nothing runs between invocations. There is no background pass and no cron: the corpus moves
only when you invoke a batch, and every batch is recorded in the ledger under one batch
correlation id.

## Walk a scope

Exactly one scope per invocation:

```bash
# One resource — survey, then act.
temper admin reblock --resource <resource-ref> --dry-run
temper admin reblock --resource <resource-ref>

# One context — the walk's normal unit. Survey first.
temper admin reblock --context <context-ref> --dry-run
temper admin reblock --context <context-ref> --limit 50

# Resume a walk from a receipt's cursor.
temper admin reblock --context <context-ref> --dry-run --after-id <last-cursor>

# The whole deployment — requires system-administrator standing, and is never a default.
temper admin reblock --all --dry-run
```

Start small: one resource, then a low-stakes context, `all` last. Omitting `--limit` applies a
default of 500 — sized so a window of changing resources completes comfortably in one
invocation; the `after_id` cursor resumes exactly where the previous receipt stopped, and
re-running from the top is always safe — every already-conformed resource classifies as a
no-op and is left untouched.

The same operation is reachable as the MCP tool `resource_reblock` (scope `resource`, `context`,
or `all`; a context ref must be one you can read) and as `POST /api/resources/reblock` with a
body naming `scope`, `dry_run`, `limit`, and `after_id`. The CLI prints the receipt as JSON by
default. Flag details: [the CLI reference](../reference/cli/admin.md).

## Read the receipt

Each invocation returns one receipt: the batch correlation id, one outcome per candidate,
per-class summary counts, the `in_progress` count for the scope, and the continuation cursor.

| Outcome | What it means | What it asks of you |
|---|---|---|
| `reblocked` | The act fired; the ledger carries it under the correlation id. | Nothing — this is the walk working. |
| `no_op` | Already conformed. Fires nothing — the ledger is indistinguishable from the act never running. | Nothing. |
| `would_change` | Survey only: acting now would re-block. | Act, then re-survey. |
| `denied` | Your grants stop at this resource. The batch continued around it. | Nothing, or grant more — the boundary is working, and it is visible per row on purpose. |
| `in_progress` | A segmented upload is still arriving; a partition decision over it would be a guess. | Let the upload finish — the policy applies when it finalizes — or address the resource directly once complete. |
| `byteless` | The resource stores no verbatim bytes (derived-shape content). | Nothing — it is outside the blocking policy's reach by design and stays as authored. |
| `drift` | A fresh chunking of the body does not reproduce the stored chunking, so no trustworthy baseline exists. | Re-save the resource with an ordinary whole-body update — the write applies the policy as it lands — then include the row in a later batch. |
| `error` | The server hit an internal error on this row; the batch continued. | Retry that row alone. The real diagnostic is in the server logs, joined by the correlation id. |

The summary's `in_progress` count names the still-arriving uploads in the scope that this
invocation did not consider — the population stays visible instead of being silently skipped.

## Validate the walk

- **The re-survey is the evidence.** After acting on a window, re-run the survey: the
  `would_change` count should have dropped by the `reblocked` count, and the remainder tells you
  exactly what is left.
- **The ledger holds every real act.** All `reblocked` acts in a batch share one correlation id,
  which pairs the receipt with the ledger's record of the batch: the act's event on the
  resource's trail (`temper trail node <resource>`) carries that same `correlation_id`.
  No-ops are deliberately absent
  from the ledger — the receipt is their only record, and absence of events is never read as
  completion.
- **The survey is the authority on remaining scope.** Because no-ops leave no ledger trace,
  "what is left to walk" is a survey question, never a ledger question.

## The boundaries

- **Nothing restructures on its own.** No merge, upgrade, migration, or background process moves
  your corpus; every batch is an act someone invoked.
- **The walk never exceeds your own rights.** A resource you could not have updated one at a
  time is declined `denied` in the receipt, with the batch alive around it.
- **Every invocation is bounded.** The `limit` sizes the window and the cursor resumes it —
  the numbers are conveniences; no surface can enumerate or mutate an unbounded scope in one
  act.
