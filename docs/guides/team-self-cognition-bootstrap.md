# Bootstrapping a team's self-cognition cognitive map

This runbook births + binds a **team self-cognition map**: a cognitive map born 1:1 with a
team, whose ingest source is the team's own temper resources, and whose telos is "understand how
this team works" (see [`schema-artifact/manifests/team-self-cognition.yaml`](../../schema-artifact/manifests/team-self-cognition.yaml)).
This is the foundation the Eve steward (see the
[act-model + cogmap-resource vocabulary design](../superpowers/specs/2026-06-30-steward-act-model-cogmap-resource-vocabulary-design.md))
tends going forward — this SoP only covers birth + bind, not ongoing stewardship.

**Audience:** an operator (or the steward's own deploy step) standing up self-cognition for a
single team on an already-usable temper instance (org bootstrap already done — see
[org-bootstrap.md](./org-bootstrap.md) — an org-identity map is not a prerequisite for this
runbook, but the instance must have at least one admin and an authenticated `temper` binary).

This is a **standard operating procedure** (SoP): every step is a surfaced, already-existing,
idempotent `temper` command. Unlike [org-bootstrap.md](./org-bootstrap.md), there is deliberately
**no interpolation into a manifest** — the telos manifest is team-agnostic prose and the team's
identity rides entirely on the `--name` flag at apply time, so the same artifact serves every team.

## Why this composes existing primitives, not a new command

A team self-cognition map is architecturally identical to the org-identity map (a cognitive map
born from a genesis manifest, then bound to a team) — it just applies at team granularity instead
of org granularity, and it skips the `reconcile`-with-landmarks step (a team self-cognition map is
**dogfed** from the team's own resources via the steward's ordinary `assert`/`facet`/`fold` acts,
not pre-populated with authored landmark content). No new CLI command is needed: `team create`,
`context create`, `cogmap create --manifest`, and `cogmap bind` already compose the whole sequence.

## What you end up with

| Outcome | Produced by |
|---------|-------------|
| The team (you become its owner) | `temper team create <team-slug> --name "<Team>"` |
| The team's working context — the ingest source | `temper context create <ctx> --owner +<team-slug>` |
| A self-cognition cognitive map, born with the templated telos charter | `temper cogmap create --manifest schema-artifact/manifests/team-self-cognition.yaml --name "<Team> — self-cognition"` |
| The map reaching the team's shared corpus | `temper cogmap bind <cogmap-ref> <team-slug>` |

## Prerequisites

- **A usable instance.** At least one system admin exists (see [org-bootstrap.md](./org-bootstrap.md)
  §0–1 if this is a fresh install) — `cogmap create` / `cogmap bind` are admin-gated (interim gate
  is `is_system_admin`, the same seam org-identity maps use; see
  [`reference_l0_content_delivery_admin_gate`](./l0-content-delivery.md) for how that gate is
  granted).
- **An `embed`-capable `temper` binary.** `cogmap create` embeds the charter client-side (ONNX). A
  non-`embed` build returns a clear `requires the 'embed' feature` error rather than running.
- **Authentication.** You must be logged in (`temper auth login`, or `TEMPER_TOKEN` exported) as a
  system admin before step 3/4 below (steps 1/2 only need an authenticated profile, not admin).

## The sequence

Placeholders: `<team-slug>` (globally-unique team slug), `<Team>` (display name), `<ctx>` (the
team's working context name, e.g. `building`).

### 1. Create the team

```bash
temper team create <team-slug> --name "<Team>"
```

You become the team's owner. Idempotent by slug — re-running against an existing slug is a no-op
(no duplicate team, no error escalation beyond the existing-slug case).

### 2. Create the team's working context

This is the **ingest source** the self-cognition map eventually dogfeeds from — resources written
here are what the steward distills nodes from.

```bash
temper context create <ctx> --owner +<team-slug>
```

`--owner +<team-slug>` marks this a team-owned context (requires owner/maintainer on the team,
which step 1 already granted you). Omitting `--owner` defaults to a personal `@me`-owned context,
which is **not** what a team self-cognition map wants — always pass `--owner` here.

### 3. Birth the self-cognition cognitive map

Genesis births a new map with its telos charter from the reusable, team-agnostic genesis manifest —
no per-team edits to the manifest are needed, only the `--name` override:

```bash
temper cogmap create --manifest schema-artifact/manifests/team-self-cognition.yaml \
  --name "<Team> — self-cognition"
```

The output reports the realized identity:

```json
{ "cogmap_id": "019f…", "telos_resource_id": "019f…", "created": true }
```

**Capture `cogmap_id`** — step 4 needs it. Genesis is idempotent at a given id: pin `cogmap_id` (and
optionally `telos_resource_id`) in a per-team copy of the manifest, or pass `--id <cogmap-ref>`, and
a re-run is a no-op (`created: false`). Without a pinned id the CLI mints a fresh uuidv7 each run —
the same id-capture-vs-pin tradeoff [`install-profile.yaml`](../../schema-artifact/install-profile.yaml)
documents for the org-identity map. For a one-off team this SoP is typically run by hand and the
printed id is captured directly into step 4; pin it only when you want a reproducible, re-runnable
genesis (e.g. driving this SoP from a script or a per-team profile file).

### 4. Bind the map to the team

Binding widens the map's reach to the team's shared resources (an unbound map reaches nothing
through the team — empty join, default-closed):

```bash
temper cogmap bind <captured-cogmap-ref> <team-slug>
```

This is the **1:1 team↔cogmap join**. Idempotent on the join primary key — re-running is a no-op.

The team's self-cognition is now live: the map is born with the templated "how this team works"
charter, and it reaches resources written into `+<team-slug>/<ctx>`. From here the Eve steward
tends it (create / assert / facet / fold acts against the team's own resources); regions emerge
from `materialize` — the steward never clusters directly.

## Idempotency

Idempotency is **inherited from the primitives**, not implemented by this SoP: `team create` is
idempotent by slug, `context create` is idempotent by name+owner, `cogmap create` is idempotent at
a given id (pinned or captured), and `cogmap bind` is idempotent on the join PK. Re-running this
whole sequence therefore **converges** rather than duplicating — the same property
[org-bootstrap.md](./org-bootstrap.md) relies on for its applier script.

## Verification

```bash
temper cogmap shape <captured-cogmap-ref>
```

Shows the map (initially with no materialized regions — it was just born, nothing has been
asserted/folded/materialized into it yet). The map's telos carries the templated statement, five
questions, and three framing lines verbatim from
[`team-self-cognition.yaml`](../../schema-artifact/manifests/team-self-cognition.yaml). Today the
charter prose itself is not surfaced by `cogmap shape` (a regions/analytics view, not a charter
read) — once T1's `cogmap_read_charter` MCP tool lands, it reads the telos prose directly; until
then, confirm the charter landed correctly by inspecting the `telos_resource_id` resource (e.g.
`temper resource show <telos_resource_id>`).

### Verifying against a local dev stack (not run this session)

This SoP's steps target the CLI's configured API (production, temperkb.io, by default) — there is
no local server running in this environment, so the sequence above was **not executed live this
session**. To verify end-to-end against a local dev stack instead of prod:

```bash
cargo make docker-up
cargo build -p temper-cli --bin temper --features embed
# Point the CLI at localhost and authenticate as a local admin, then run steps 1–4 above for a
# scratch team (e.g. --team t2-selfcog-smoke), confirming:
#   - step 3 prints created: true and a cogmap_id on first run
#   - re-running step 3 at the same id (--id <captured-cogmap-ref>) prints created: false
#   - step 4's bind produces exactly one kb_team_cogmaps row for the team, on first run and re-run
```

## Not run in most installs

Creating the specific **`temper` team + `building` context** and re-homing an existing personal
corpus into it is a **one-time operator migration for J's own install**, not a step most installs
need — this runbook is the reusable, team-agnostic procedure; that specific migration is deferred
and tracked outside this durable deliverable (see the Deferred section of the
[T2 implementation plan](../superpowers/plans/2026-06-30-t2-templated-team-telos-genesis.md)).

## Deferred seams

- **Cogmap-write gate vs. team roles.** Like org-identity maps, the interim gate for `cogmap create`
  / `bind` is `is_system_admin` — eventually maintainers of the team itself should be able to write
  their own team's self-cognition map without needing system-admin.
- **An applier script.** `docs/guides/org-bootstrap.md` has `scripts/bootstrap/system-bootstrap.sh`;
  a `scripts/bootstrap/team-self-cognition.sh` echo-then-apply script for this sequence is deferred
  to the steward's deploy step (T6), not authored in this pass.
- **Auto-birth-of-self-cogmap-per-team.** The MVP is this on-demand SoP; automatically birthing a
  self-cognition map whenever a team is created is out of scope here.

## References

- Surfaced commands: `temper team create`, `temper context create`, `temper cogmap create`,
  `temper cogmap bind` (`crates/temper-cli/src/cli.rs`).
- Template / shape precedent: [org-bootstrap.md](./org-bootstrap.md) +
  [`schema-artifact/manifests/org-identity.yaml`](../../schema-artifact/manifests/org-identity.yaml).
- The reusable genesis manifest:
  [`schema-artifact/manifests/team-self-cognition.yaml`](../../schema-artifact/manifests/team-self-cognition.yaml).
- Steward architecture this map feeds: `docs/superpowers/specs/2026-06-30-steward-act-model-cogmap-resource-vocabulary-design.md`.
- Implementation plan this SoP delivers: `docs/superpowers/plans/2026-06-30-t2-templated-team-telos-genesis.md`.
