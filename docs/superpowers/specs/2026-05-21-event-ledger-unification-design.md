# Event Ledger Unification — Design

**Date:** 2026-05-21
**Context:** `temper`
**Mode:** plan → build
**Goal:** `resource-lifecycle-event-sourcing`
**Decision:** `2026-05-21-temper-reorients-to-event-primary-resources-and-the-graph-become-projections-of-an-event-ledger`

---

## Framing — why this work exists

This work began from one question: **knowledge-graph edges should become
projections of an event ledger — the same move PR 81 made for concept
documents.** Today the graph stores relationships as denormalized mutable
rows (`kb_resource_edges`, written `ON CONFLICT DO UPDATE`); a correction
destroys the prior topology instead of deforming it. Event-sourced edges
are the precondition for the semantic model's deformation, folding, and
scarification mechanics to be real for *relationships*, not only for
documents (research: `2026-05-19-event-sourced-knowledge-graph-relationships`).

Pursuing that question surfaced something larger. Temper already
proto-event-sources resource writes (`kb_events` + `kb_resource_audits`),
and PR 81 built a disciplined-but-separate event substrate
(`event_substrate` schema, `temper-events` crate) as a "think-with"
sibling project. The system has two event models and a deferred question
about whether they belong together. The reorientation decision answers
it: events become primary, and the substrate stops being a sibling.

This spec is scoped to **limb 0** — the unification that limbs 1–3 all
sit on. Edges-as-projections (limb 1) is the originating motivation and
the immediate downstream consumer; it is designed in its own cycle. The
framing is kept here deliberately so the downstream destination is never
lost behind the infrastructure pass.

## The reorientation (context, not scope)

Per the decision, `kb_resources` and `kb_resource_edges` become
materialized projections of an append-only event ledger rather than the
source of truth. The work decomposes into four limbs:

- **Limb 0 — event ledger unification.** *This spec.* One disciplined
  ledger in `public`; the substrate that the rest sits on.
- **Limb 1 — relationship-lifecycle events → edge/vertex projection.**
  The originating task. Carries four open gate questions (below).
- **Limb 2 — concept convergence.** Retire the separate concept
  projection; `kb_resources` with `doctype: concept` becomes the
  projection of concept events.
- **Limb 3 — resource lifecycle event-sourcing.** All resource writes
  become events; `kb_events`/`kb_resource_audits` re-rooted so the event
  is authoritative and the manifest/audit are projections.

The full reorientation is the destination; it is not taken all at once.
Limbs 1–3 each get their own spec → plan → build cycle.

## Current state — two event models

**`kb_events` (the proto-log, `public` schema).** Columns: `id`,
`profile_id`, `device_id`, `kb_context_id`, `resource_id`, `event_type`
(free `VARCHAR`), `payload` (JSONB), `created`. Every mutation routes
through `insert_event_and_audit()`, which atomically writes a `kb_events`
row plus a `kb_resource_audits` row; the payload is the
`{body_hash, managed_hash, open_hash}` rollup (⊕ optional enrichment
since `20260521000001_event_payload_extra.sql`). It is a *log*: no
append-only enforcement, no event-type registry, no `references` chain,
no topic, no scope, no `correlation_id`, no `occurred_at`/`recorded_at`
split. It is `resource_id`-scoped — too narrow for events about an edge
(a *pair* of resources) or a cognitive synthesis.

**`event_substrate.events` (the disciplined ledger).** Built by PR 81 in
a separate schema, exercised only by tests. It has everything `kb_events`
lacks: append-only trigger, `event_types` FK registry, `references`
jsonb with `Supersedes`/`DerivedFrom`, `correlation_id`,
`occurred_at`/`recorded_at`, `topic_id`, `scope_id`. Its emitter model
(`entities` → `profiles`) is a deliberate minimal stand-in: PR 81's
sibling boundary forbade referencing `kb_profiles`, so it built a
parallel identity model rather than reuse the real one.

**Two facts that bound this work:**

1. `event_substrate` has **never been emitted into in production** — it
   is wholly separate and test-only. Retiring it is a clean schema drop,
   not a data migration.
2. A resource's body **is only ever its chunk set.** `kb_resources` has
   no `content` and no `content_hash` (both dropped in
   `20260404000002_resource_manifests.sql`). Chunk text lives in
   `kb_chunk_content`, content-addressed and deduped; `kb_resource_revisions`
   anchors content versions; `resource_chunks_at_revision()` already
   reconstitutes the body as of any past revision. Point-in-time
   projection of resource content already exists. Events are hash-only
   by design and must stay that way — no content travels in an event.

## Decision — unify into `public`, retire `event_substrate`

Limb 0 is a **unification, not a bridge.** The `event_substrate` schema
is dropped wholesale; its design is rebuilt in `public`, evolving
`kb_events` into the one disciplined ledger.

Rationale:

1. The sibling boundary was a hedge — *"does the substrate model fit
   Temper-the-product?"* The reorientation decision answered yes. Two
   schemas perpetuate a resolved question.
2. `kb_events` always referenced real product entities (`profiles`,
   `contexts`, `resources`) because a ledger must — an emitter is a real
   profile, a scope is real visibility. `event_substrate`'s parallel
   `entities`/`profiles` exist *only* because the boundary forbade
   referencing `kb_profiles`. Drop the boundary and that parallel
   identity model is pure reconciliation debt; emitter becomes
   `kb_profiles.id`.
3. The design value of `event_substrate` — append-only trigger,
   event-type registry, `references`/Supersedes, `correlation_id`,
   `occurred_at`/`recorded_at`, topics, scopes/porosity, replay-pure
   projection discipline — is **not tied to the schema**. Every piece
   transplants into `public` unchanged. Unifying keeps 100% of what
   PR 81 figured out and drops only the hedge.

The churn cost — `event_substrate` and `temper-events` are days old
(PR 81) — is accepted: the project is one month old, the standing
posture is no premature backward-compat, and the intentional pivot is
worth more than dragging a sibling project forward.

## Limb 0 design

### Data tiers

- **Ledger tier (`public`).** The evolved event table + `kb_event_types`
  + `kb_topics` + `kb_scopes`. Append-only.
- **Projection tier (`public`).** `kb_resources`, `kb_resource_manifests`,
  `kb_resource_edges`, `kb_chunks` / `kb_chunk_content` /
  `kb_resource_revisions`. **Untouched by limb 0.**

### Schema migration

**Drop `event_substrate` schema wholesale** (`DROP SCHEMA event_substrate
CASCADE`), including its `concepts` table — concept projection is limb 2.

**New `public` tables** (`kb_` prefix, per project convention):

- `kb_event_types` — `(id, name UNIQUE, description, is_deprecated,
  created)`. The FK target for the ledger's `event_type_id`. Seeded with
  the event-type names that exist today (the distinct `kb_events.event_type`
  strings — `resource_created`, `body_updated`, `managed_meta_updated`,
  …) plus `ConceptCreated` / `ConceptMutated` carried over as harmless
  registry rows. Relationship-lifecycle type names are **not** seeded
  here — limb 1 adds them in its own migration.
- `kb_topics` — `(id, fqdn UNIQUE, parent_id self-FK nullable, created)`.
  Hierarchical FQDN namespace.
- `kb_scopes` — `(id, name UNIQUE, porosity, created)` where `porosity`
  is the `access` | `attention` enum. No default on `porosity` — forces
  explicit declaration, per PR 81's fail-closed rule.

**Evolve the event table (`kb_events`, name kept — evolution in place,
least churn):**

- `event_type_id UUID NOT NULL REFERENCES kb_event_types(id)` — backfill
  by inserting a registry row per distinct existing `event_type` string
  and mapping. Drop the legacy `event_type VARCHAR` column after backfill
  (no premature compat).
- `topic_id UUID REFERENCES kb_topics(id)` — **nullable** in limb 0:
  resource-write events have no meaningful topic yet; limb 1+ event
  families populate it.
- `scope_id UUID REFERENCES kb_scopes(id)` — **nullable** in limb 0.
- `kb_context_id` — **kept.** Context (a vault namespace) and scope
  (porosity / visibility precedence) are orthogonal; an event carries
  both. Whether scope ever subsumes context is explicitly deferred.
- `references JSONB NOT NULL DEFAULT '[]'` — array of
  `{ "kind": <ReferenceKind>, "event_id": <uuid> }`. GIN-indexed
  (`jsonb_path_ops`). `event_id` always resolves within the event table —
  references are event→event only, never resource-pointing (a firm
  invariant; relationship-event *endpoints* travel in the typed payload,
  not in `references`).
- `correlation_id UUID` — nullable in limb 0; populated by new event
  families.
- `occurred_at TIMESTAMPTZ` — added, defaulting to the existing `created`
  value on backfill. The existing `created` column is the `recorded_at`
  (commit time); `occurred_at` is business time.
- **Emitter** — `kb_events.profile_id` already *is* the emitter. The
  reconciliation onto `kb_profiles` is free; `device_id` is kept.
- **Append-only trigger** — `BEFORE UPDATE OR DELETE ON kb_events FOR
  EACH ROW` raises. Supersession and correction are themselves events.

**Seed** a root `kb_topics` FQDN row and a default `kb_scopes` row
(transplanting PR 81's `event_substrate.bootstrap` topic + `public`
access-scope), with deterministic UUIDv7 ids so fixtures can reference
them.

All id columns use `DEFAULT public.uuid_generate_v7()` (the portability
shim), defensive against multiple writers.

### Non-breaking guarantee

The existing resource-write path keeps working with **no observable
behavior change**:

- `insert_event_and_audit()` evolves minimally — it resolves the
  event-type *name* to an `event_type_id` (insert-or-get against
  `kb_event_types`) and leaves the new ledger columns null/defaulted.
  `kb_resource_audits`, `kb_resource_revisions`, `kb_resource_manifests`,
  and all chunk machinery are **untouched**.
- Resource event-sourcing *proper* — events authoritative, manifest and
  audit demoted to projections, `insert_event_and_audit` replaced by the
  `append_event` discipline — is **limb 3**, not limb 0.
- The existing `temper-api` and e2e resource-CRUD suites must pass
  unmodified. That is the operational definition of the guarantee.

### `temper-events` crate

Stays its own **leaf crate** — justified forward-looking as the home for
"how to know what has happened" tooling (event subscriptions, topic
thresholds and saturations, FQDN-topic namespaces, scopes), which
references resources but is not 1:1 with the resource space. The leaf
property is preserved by the crate dealing only in `Uuid` emitters /
references and `serde_json::Value` payloads — typed payload structs live
in `temper-core`.

Changes in limb 0:

- Retarget the ledger types (`Event`, `EventToWrite`, `EventReference`,
  `ReferenceKind`, `EventType`, `Topic`, `Scope`) to the `public` tables.
- Emitter field becomes a profile `Uuid` (was `emitter_entity_id`). The
  stand-in `entities` / `profiles` tables and `create_entity` /
  `move_entity` / `discard_profile` are retired.
- The `concepts` projection and `project_concept` / `rebuild_concept`
  are concept-specific — removed in limb 0 (no premature retention). The
  replay-purity *discipline* is preserved as a documented ledger
  invariant and re-tested when limb 2 builds the concept projection
  against `kb_resources`.
- `append_event` and the ledger primitives (topic / scope / event-type
  operations, reference validation, append-only enforcement) stay.

Migrations remain in the central `migrations/` directory (sqlx
convention); the crate owns the ledger *code*, not a migration tree. The
per-crate `crates/temper-events/.sqlx/` cache pattern is retained.

### Error handling

`LedgerError` stays. `UnknownEntity` becomes `UnknownProfile`; the
`UnknownTopic` / `UnknownScope` / `UnknownEventType` / `DanglingReference`
/ reference-invariant variants carry over unchanged. No silent-fallback
paths — every variant is a specific malformed write or precondition
failure. The append-only trigger is the database-level backstop.

### Testing

- **Transplant** the PR 81 ledger-discipline / replay-pure test suite,
  retargeted to the `public` tables: append-only enforcement, event-type
  FK integrity, reference validation, `correlation_id` grouping.
- **Unification regression tests:** `event_substrate` is gone; `kb_events`
  is disciplined (append-only trigger fires on UPDATE and DELETE;
  `event_type_id` FK rejects unknown types).
- **Non-breaking proof:** the existing resource-write path
  (`insert_event_and_audit` → audit + revision + chunk machinery) runs
  green end-to-end, and the existing `temper-api` + e2e resource-CRUD
  suites pass unmodified.
- Feature-gating and per-crate `.sqlx/` conventions per project memory
  (`test-db` gate on `#[sqlx::test]` files; per-crate prepare for
  `temper-events`).

## Limbs 1–3 — sequenced, designed in their own cycles

**Limb 1 — relationship-lifecycle events → edge/vertex projection.** The
originating task. Relationship-events (assert, re-type, re-weight, decay,
fold, correct) land in the unified ledger; `kb_resource_edges` becomes a
rebuildable projection; edge mechanics (label / weight / type) become
first-class API/CLI/MCP surfaces. Edges are greenfield — they have no
legacy event/audit/revision machinery — and purely structural, so their
projection rebuilds bit-identically. Limb 1 carries **four open gate
questions**, resolved in its own brainstorm:

1. Topic-taxonomy placement of relationship-lifecycle events — own topic
   class, or distributed across declaration / deformation / judgment?
2. Cache reconciliation policy — rebuild-on-event vs lazy vs
   per-perspective; staleness semantics.
3. Vertex lifecycle — own event topics, or covered by resource-lifecycle
   events?
4. Edge typing — evaluate the SSTorytime four-type taxonomy
   (`2026-05-19-sstorytime-semantic-spacetime-analysis`) as the edge-type
   vocabulary.

**Limb 2 — concept convergence.** Retire the separate concept
projection; `kb_resources` with `doctype: concept` becomes the
materialized projection of concept events, chunked and embedded like any
resource.

**Limb 3 — resource lifecycle event-sourcing.** All resource writes
become events in the unified ledger; `kb_resource_manifests` and
`kb_resource_audits` are re-rooted as projections; `insert_event_and_audit`
gives way to the `append_event` discipline.

## Out of scope (limb 0)

- Any relationship-event type or edge projection (limb 1).
- Any change to how resources are written or projected (limb 3).
- The concept projection into `kb_resources` (limb 2).
- Topic governance, subscription semantics, saturation thresholds —
  future `temper-events` tooling.
- Embedding determinism on replay — a carried tension, named in the
  decision: replay is bit-pure for frontmatter, body, and edges, but
  embeddings are not deterministic across model versions.

## Validation

Limb 0 is complete when:

- `event_substrate` is dropped; the ledger lives entirely in `public`.
- `kb_events` is append-only-enforced, event-type-registry-backed, and
  carries `references` / `correlation_id` / `topic_id` / `scope_id` /
  `occurred_at`.
- The existing resource-write path and its full test suites pass
  unmodified.
- The transplanted ledger-discipline tests pass against `public`.
- `cargo make check` and the workspace test suites are green.
