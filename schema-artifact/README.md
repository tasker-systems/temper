# Temper — Arc-1 destination schema artifact

A fresh, one-shot `schema.sql` written as a **destination artifact, not a migration**.
It encodes the shape the six Arc-1 specs describe, loads into a **separate Postgres
namespace** (`temper_next`) alongside the live `public.*` schema, and is exercised by
seed + scenario queries so the cognitive-map model can be evaluated *empirically* —
reading the real delta vs. the current schema — **before** any phased migration is written.

This is the payoff of the `jct/data-model-reconciliation-spec` arc. The migration phases
come *after* this evaluation, better-grounded for it.

## Files (load in order)

| File | Contents |
|------|----------|
| `01_schema.sql` | `CREATE SCHEMA temper_next` + ~7 enums, 24 tables, indexes |
| `02_functions.sql` | Access-gating functions (the two-principal sum type), `cogmaps_share_a_team`, the `sync_system_membership` trigger, `cogmap_genesis`, the Domain-B read projections, and the **reusable mutation functions** (`resource_create`/`relationship_assert`/`facet_set`/`lens_create` — each emits its event + projects in one txn, the `cogmap_genesis` mold) |
| `03_seed.sql` | One coherent worked scenario (the epd-team-a/-b intersection bridge, the directors' private edge, a `cogmap_genesis`-seeded charter + regulation, and the emergent-region falsification cast). Regions are **produced by the `temper-next` harness**, not hand-seeded. |
| `04_scenarios.sql` | Labeled queries that make every load-bearing invariant observable |
| `seeds/system.yaml` | System boot-seed (event-type registry + global lenses) — what any temper system needs, loaded by `temper-next::scenario::bootseed::seed_system` |
| `scenarios/onboarding-cogmap.yaml` | The onboarding-cogmap as a **declarative scenario** (substrate + S6a–h runbook), the YAML re-expression of `03_seed.sql`'s onboarding cast; roundtrip-validated by `temper-next/tests/scenario_roundtrip.rs` |

### Declarative scenarios (temper-next)

`scenarios/*.yaml` are the seed/scenario DSL consumed by `temper-next` (`src/scenario/`): a thin Rust
loader calls the `02_functions.sql` mutation functions with YAML inputs, and a runner drives the ordered
`steps` runbook (materialize / emit-event / assert) **in-process** — the same S6a–h falsification that
`run_eval.sh` runs via SQL + bash, now as one `cargo nextest` test. `scenario.schema.json` is the JSON
Schema emitted from the loader structs. See `docs/superpowers/specs/2026-06-07-scenario-yaml-seed-dsl-design.md`.

## Load & evaluate

Requires the dev DB up (`cargo make docker-up`); extensions `vector` and the
`uuid_generate_v7()` generator live in `public` and are reached via `search_path`.

```bash
DB="postgresql://temper:temper@localhost:5437/temper_development"
for f in 01_schema 02_functions 03_seed; do
  psql "$DB" -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql
done
psql "$DB" -f schema-artifact/04_scenarios.sql      # prints the scenario verdicts (S1-S5,S7,S8)
```

The **S6 region surface + staleness** lines in `04_scenarios.sql` read regions, which are produced by
the harness — run `cargo run -p temper-next -- onboarding-cogmap` after the seed to materialize them
(before `04_scenarios.sql`), or use the end-to-end **`schema-artifact/run_eval.sh`**, which loads,
materializes, and runs the full S6a–h falsification suite (`04b_region_suite.sql`). The Plan-1 readout
fixture `04a_plan1_fixture.sql` is self-contained (its own fixture region) and runs after `01->03`.

`01_schema.sql` begins with `DROP SCHEMA IF EXISTS temper_next CASCADE`, so re-running
from the top is idempotent and never touches `public.*`.

## What the scenarios demonstrate

- **S1 Consumer axis** — `resources_visible_to(person)`: ownership + profile-grant + team-grant
  (DAG-inherited down); `nomad` (disabled) sees nothing.
- **S2 Producer axis** — the least-privilege **team intersection**: "more teams = narrower reach"
  (a team-a-private doc is in `side-map(a)` but falls out of `bridge-map(a∩b)`), and **leak-safety**
  (a profile grant is in *neither* map — profile grants never enter a `vis(T)`).
- **S3 Edge-home protection** — a private edge between two *public* concepts is invisible to anyone
  who can't read its home cogmap, even though both endpoints are readable.
- **S4 Domain-B projections** — the charter body and question blocks via `resource_body_text` /
  `resource_blocks` (resolved through `cogmap_telos`), plus `cogmap_regulation`, with
  the principal gate (a profile that can't read the map gets an empty charter).
- **S5 Delegation priming** — `cogmaps_share_a_team` (the live ∃-one-shared-team predicate).
- **S6 Shape + staleness** — the region surface (member identities never exposed) and the **on-read**
  staleness aggregate (A3-3) reporting a stale shape after a later edge event.
- **S7 Entity launch-metadata** — the agent-instance's launch-metadata in the open `metadata jsonb`
  (no `entity_kind` enum).
- **S8 Descriptor coherence** — the `write|delete|grant ⇒ read` CHECK rejects an incoherent grant.

## The delta vs. current `public.*`

**New tables (Arc-1 additions):** `kb_entities` (the actor, `metadata jsonb`), `kb_resource_homes`,
`kb_resource_access` (4-boolean descriptor), `kb_content_blocks`, `kb_block_revisions`,
`kb_block_provenance`, `kb_properties`, `kb_cogmap_regions`, `kb_cogmap_region_members`.

**Renamed / reshaped:** `kb_scopes`→`kb_cogmaps` (+`telos_resource_id`, −`porosity`);
`kb_resource_edges`→`kb_edges` (polymorphic endpoints, `scope_id`→`home_anchor_*`);
`kb_team_resources`→`kb_resource_access`; `kb_team_scopes`→`kb_team_cogmaps`;
`kb_chunks` gains `block_id` (dedup at block grain, partial HNSW `WHERE is_current`).

**The headline delta — the event ledger:** current `kb_events` emits via `(profile_id, device_id)`
+ `payload`/`references` jsonb. The target emits via **`emitter_entity_id`** → the reintroduced
`kb_entities` (the event-substrate schema was unified into `public.kb_*`, so the actor table returns
here). `scope_id` → polymorphic provenance `producing_anchor_(table,id)`.

**Dropped:** `kb_resource_manifests`, `kb_doc_types`, `kb_resource_revisions`, `slug` (everywhere),
`porosity` (column + enum), `access_level` (enum).

## Decisions made concrete here

Every `[LEAN→DECISION]` marker in the SQL traces to the 2026-06-04 lean-promotion pass
(specs, commit `9335afb`). Two are worth calling out as *visible behaviour*:

- **Folding is a visibility act, orthogonal to currency** — `is_current` (chunk-local) and
  `is_folded` (block visibility) are independent gates; reads filter on both.
- **Approval auto-joins the root team** — a real `kb_team_members` row maintained by the
  `sync_system_membership` trigger, *not* a read-time `system_access` branch (this **revises** access
  spec §4 OQ-3's earlier "virtual membership, no stored row").

One decision was deliberately deferred to here and resolved against the DDL: the
`kb_events.scope_id` producing-anchor is modeled as **polymorphic provenance**
`producing_anchor_(table,id)` over `('kb_contexts','kb_cogmaps')` — every homed object already
carries its own gating anchor, so the event's anchor is provenance, not the gate.

## Scope notes

The artifact models the cognitive-map core. Operational/sync Domain-A tables not central to
evaluating the model are intentionally omitted (`kb_blob_files`, `kb_ingestion_records`,
`kb_device_sync_state`, `kb_transfers`, `kb_team_invitations`, `kb_join_requests`,
`kb_profile_auth_links`, the FTS index — rebuilt by trigger in prod). The functions reference
unqualified names and rely on `search_path = temper_next, public` (set by the scenario file); a
production cut would schema-qualify or pin `SET search_path` per function.
```
