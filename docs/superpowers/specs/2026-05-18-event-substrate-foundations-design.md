# Event Substrate Foundations — Design

**Date:** 2026-05-18
**Context:** `temper`
**Mode:** plan
**Effort:** medium
**Branch:** `jct/eventing-foundations`

**Conceptual predecessors (session arc, all in `temper` context):**
- 2026-05-10 — Working-Context-and-Attention Manifesto
- 2026-05-14 — Coordination substrate design session
- 2026-05-14 — Institutional-memory consolidation and MVP re-pivot
- 2026-05-17 — Event-sourcing substrate, privacy, topic segmentation
- 2026-05-18 (AM) — Topic ⊥ scope, agent-scoping, trajectory translation
- 2026-05-18 (PM) — Concept-genesis, foundation-routed translation, cluster detection

**Companion research (filed during the arc):**
- Coordination-substrate synthesis (resource `019e26b5-1713-7ed0-9996-1ce1f44e05b8`)
- Substrate-design session note (resource `019e26c5-8af4-7b80-98e7-bdcf8a5991cc`)

---

## Problem

Five sessions of conceptual pressure have produced a coherent set of
primitives — event-primary cognition; topic ⊥ scope with declared
porosity; agent-inherent scope-assignment as the relief for the routing
problem; event-intrinsic references; concept-as-resource with the
event-stream as authoritative and the resource as materialized
projection; rebuildability by replay. The primitives keep being
rediscovered under independent pressures, which is evidence they are
well-chosen.

What does not yet exist is a place to *think with* them in code. Markdown
prose can carry a self-consistent model that breaks the moment it meets
SQL: column-shapes force decisions the conceptual register defers (NULL
or NOT NULL? enum or jsonb? FK or denormalized?). A pure-prose model is
also missing the closed loop that disciplines event-sourced thinking:
emit → project → replay → assert-equality. Until that loop runs, the
event-primary / resource-as-projection primitive is a claim, not a
verified property.

The `eugene-khyst/postgresql-event-sourcing` reference repo (reviewed in
2026-05-17) is a faithful implementation of *one* tradition — DDD
aggregate-oriented per-aggregate streams — whose assumptions do not
survive contact with the substrate (multi-referential events, single
append-only ledger, no write-time invariant enforcement beyond the bare
minimum). Adopting it wholesale would import the wrong mental model. The
infrastructure layer of event-sourcing best practice (append-only INSERT
ledger, transactional outbox, at-least-once delivery with idempotent
consumers, watermark-based ordered reads) is well-solved and should be
imported; the semantic layer is where the design lives.

This spec covers only the semantic-layer minimum: schema, types, and a
write-and-project loop tight enough that holes in the conceptual model
surface as failing tests or unsatisfiable constraints.

## Sibling-project posture

Per the 2026-05-14 session: "Temper as it currently stands is in active
daily use and shouldn't be broken. The new substrate is a sibling
project, not a reorientation of Temper." This spec honors that posture
at three boundaries:

- **Schema boundary** — all tables live in a new `event_substrate`
  Postgres schema. The existing `public` schema (resources, chunks,
  knowledge_graph_edges, audits, manifests, revisions) is untouched and
  unreferenced. The substrate can be dropped and rebuilt without risk
  to production-Temper data.
- **Crate boundary** — `temper-events` is a new workspace crate with
  **no dependency on `temper-core`**. Pure substrate. The cohesion
  question (does the substrate model fit Temper-the-product?) becomes a
  deliberate future integration rather than upfront entanglement.
- **Surface boundary** — no HTTP routes, no MCP tools, no CLI commands
  in v1. The library is exercised through tests only. The question of
  what a substrate-facing surface looks like belongs in a later phase.

The cohesion test ("does the model survive contact with the codebase?")
is answered by code review and by writing the projection function, not
by sharing tables.

## Tables (schema: `event_substrate`)

All UUIDv7 primary keys. All `created_at` columns are `timestamptz NOT
NULL DEFAULT now()`. Foreign keys are `NOT NULL` unless stated otherwise.

### `entities`

A thing that can emit events. No typology (no agent/human/integration
distinction in v1).

| column | type | notes |
|---|---|---|
| `id` | `uuid` | PK, UUIDv7 |
| `profile_id` | `uuid` | FK → `profiles.id`, NOT NULL |
| `name` | `text` | NOT NULL. Not source-system attribution — a label that makes the emitter-as-entity legible when reading the data. |
| `created_at` | `timestamptz` | default `now()` |

### `profiles`

Aggregator above entities (has-many). Profiles are discardable when no
entity references them.

| column | type | notes |
|---|---|---|
| `id` | `uuid` | PK, UUIDv7 |
| `name` | `text` | NOT NULL. Auto-default profile (created at `create_entity` time) is named `default profile for <entity_name>`; can be renamed later. |
| `created_at` | `timestamptz` | default `now()` |

The has-many lives on `entities.profile_id`. Re-parenting an entity to a
different profile is a regular UPDATE on `entities.profile_id`;
discarding a profile is a DELETE that errors if any entity still
references it (deferred to API-layer guard rather than ON DELETE
RESTRICT for clearer error messages; see Errors below).

### `topics`

FQDN-namespaced subscribable identifier. Hierarchical via `parent_id`.

| column | type | notes |
|---|---|---|
| `id` | `uuid` | PK, UUIDv7 |
| `fqdn` | `text` | NOT NULL, UNIQUE. Dotted FQDN (`event_substrate.bootstrap`, `org.team.epd.code`). |
| `parent_id` | `uuid` | FK → `topics.id`, nullable. Top-level topics have no parent. |
| `created_at` | `timestamptz` | default `now()` |

No governance machinery, no aliases, no supersession in v1. The
hierarchy is structural metadata; subscription semantics are deferred.

### `scopes`

Visibility/precedence label with porosity declared at create-time.

| column | type | notes |
|---|---|---|
| `id` | `uuid` | PK, UUIDv7 |
| `name` | `text` | NOT NULL, UNIQUE |
| `porosity` | `event_substrate.porosity` (enum: `access`, `attention`) | NOT NULL, **no default**. Forces explicit declaration at write-time per the 2026-05-17 fail-closed rule: access ≠ attention, the failure direction flips, they cannot share a continuous dial. |
| `created_at` | `timestamptz` | default `now()` |

The Postgres enum type `event_substrate.porosity` is created by the
schema migration.

### `event_types`

The closed-per-migration set of event type names. FK target for
`events.event_type_id`. Adding a new event type is a migration that
inserts a row (paired with adding the variant to the Rust `EventType`
enum); deprecating one is a flag-flip rather than a delete, since
historical events must remain resolvable.

| column | type | notes |
|---|---|---|
| `id` | `uuid` | PK, UUIDv7 |
| `name` | `varchar(128)` | NOT NULL, UNIQUE. PascalCase canonical name (e.g. `ConceptCreated`). |
| `description` | `text` | nullable. Free-form prose for the registry. |
| `is_deprecated` | `boolean` | NOT NULL DEFAULT `false`. Lets us retire a type without breaking history. |
| `created_at` | `timestamptz` | default `now()` |

The choice of an `event_types` table over a Postgres enum or a free
`varchar` column: the table gives us referential integrity (no typo
events), a registry queryable from SQL, and a place to hang
type-level metadata (description today, current `schema_uri` /
`schema_version` defaults when the schema-registry primitive lands).
The extra join on read is cheap and the FK is indexed.

v1 seed rows: `ConceptCreated`, `ConceptMutated`.

### `events`

Append-only ledger. The substrate's authoritative store.

| column | type | notes |
|---|---|---|
| `id` | `uuid` | PK, UUIDv7. Caller-supplied via `Uuid::now_v7()` so the id is known before INSERT (correlation roots reference themselves). |
| `event_type_id` | `uuid` | FK → `event_types.id`, NOT NULL. Resolved from the Rust `EventType` enum at the write path (small per-write lookup in v1; future optimization caches at startup). |
| `emitter_entity_id` | `uuid` | FK → `entities.id`, NOT NULL |
| `topic_id` | `uuid` | FK → `topics.id`, NOT NULL |
| `scope_id` | `uuid` | FK → `scopes.id`, NOT NULL. Every event carries its scope — the 2026-05-18 emitter-attached scope label. |
| `payload` | `jsonb` | NOT NULL. Typed struct on the Rust side (`ConceptCreatedPayload`, `ConceptMutatedPayload`), serialized via serde. The "typed structs over inline JSON" rule (CLAUDE.md) is honored by the write path. |
| `metadata` | `jsonb` | NOT NULL DEFAULT `'{}'::jsonb`. Reserved for interpretation-of-payload concerns: `schema_version`, `schema_uri` (remote ref to JSON Schema / OpenAPI 3.1 doc), `serializer`, `payload_content_type`. No shape is enforced by the schema in v1 — the column exists so the schema-registry/upcasting decision (2026-05-17 deferred fence) is addressable later without a migration. |
| `references` | `jsonb` | NOT NULL DEFAULT `'[]'::jsonb`. Array of `{ "kind": "<ReferenceKind>", "event_id": "<uuid>" }`. Event-intrinsic, per the 2026-05-14 design: relationships are structural metadata on the event itself, not in a parallel ledger. GIN-indexed for traversal. |
| `correlation_id` | `uuid` | NOT NULL. The originating-intention id. A root event sets `correlation_id = id`; downstream fan-out events share the root's correlation id, forming one causal tree. |
| `occurred_at` | `timestamptz` | NOT NULL. Caller-supplied — the time the event-thing happened in its originating perspective. For internally-emitted events this equals `recorded_at` at the call site, but the column is independent so future external-ingestion can replay original timestamps faithfully. |
| `recorded_at` | `timestamptz` | NOT NULL DEFAULT `now()`. The time the ledger received the event. |

**Append-only enforcement** — a `BEFORE UPDATE OR DELETE ON
event_substrate.events FOR EACH ROW` trigger raises `'event ledger is
append-only'`. There is no ledger-level path to mutate or remove an
event; supersession and correction are themselves events.

### `concepts`

Materialized projection of concept events. Rebuildable by replay.

| column | type | notes |
|---|---|---|
| `id` | `uuid` | PK, UUIDv7 |
| `current_definition` | `text` | NOT NULL. The 2026-05-18 PM "extractable definition / core characterization". |
| `current_elaboration` | `text` | nullable. Context, examples, constraints. |
| `scope_id` | `uuid` | FK → `scopes.id`, NOT NULL. Inherited from the genesis event. |
| `topic_id` | `uuid` | FK → `topics.id`, NOT NULL. Inherited from the genesis event. |
| `created_by_event_id` | `uuid` | FK → `events.id`, NOT NULL. The genesis event. |
| `last_event_id` | `uuid` | FK → `events.id`, NOT NULL. The most-recent event projected into this row. |
| `latest_event_recorded_at` | `timestamptz` | NOT NULL. Mirrors `events.recorded_at` for the row's `last_event_id`. Allows staleness checks without a join. |

A concept's identity (its `id`) is independent of any event's `id`. The
row is the projection target; it can be rebuilt from the event chain
whose root is `created_by_event_id`.

### Indexes

- `events`: btree on `(topic_id, recorded_at DESC)` — topic-ordered reads.
- `events`: btree on `(event_type_id, recorded_at DESC)` — type-filtered reads.
- `events`: btree on `(emitter_entity_id, recorded_at DESC)` — per-emitter reads.
- `events`: btree on `correlation_id` — causal-tree assembly.
- `events`: GIN on `references` with `jsonb_path_ops` — reverse-reference
  traversal ("which events reference event X").
- `concepts`: btree on `(scope_id, topic_id)` — scope/topic filtering.
- `concepts`: btree on `created_by_event_id` — root-event lookup.
- `concepts`: btree on `last_event_id` — projection-idempotency check.
- `entities`: btree on `profile_id`.

## Seed migration

The schema migration is followed by a seed migration that bootstraps the
substrate so subsequent migrations and tests have valid FKs to reference:

- Two `event_types` rows: `ConceptCreated`, `ConceptMutated`.
- One `public` scope with `porosity = 'access'`.
- One `system` profile.
- One `system-bootstrap` entity in the `system` profile.
- One `event_substrate.bootstrap` topic at the root of the FQDN tree (no
  parent).

Seed rows use deterministic UUIDv7 values inserted via the migration so
test fixtures can reference them by id.

## Crate: `temper-events`

Located at `crates/temper-events/`. Added to the workspace `members`
list. **No dependency on `temper-core`** — clean sibling boundary.

**Dependencies (v1):**
- `sqlx` with `postgres`, `runtime-tokio-rustls`, `uuid`, `chrono`, `json` features
- `uuid` with `v7`, `serde` features
- `serde`, `serde_json`
- `chrono` with `serde`
- `thiserror`
- (dev) `tokio` with `rt-multi-thread`, `macros`

**Feature flags:**
- `test-db` — gates the integration test module (per the existing
  `feedback_test_db_feature_gate_convention` memory rule). Every test
  file with `#[sqlx::test]` carries `#![cfg(feature = "test-db")]`.

**Module shape:**

```
crates/temper-events/
├── Cargo.toml
├── src/
│   ├── lib.rs              # re-exports
│   ├── types/
│   │   ├── mod.rs
│   │   ├── entity.rs       # Entity, Profile
│   │   ├── topic.rs        # Topic
│   │   ├── scope.rs        # Scope, Porosity
│   │   ├── event.rs        # Event, EventType, EventReference, ReferenceKind
│   │   └── concept.rs      # Concept
│   ├── payloads/
│   │   ├── mod.rs
│   │   ├── concept_created.rs   # ConceptCreatedPayload
│   │   └── concept_mutated.rs   # ConceptMutatedPayload
│   ├── ledger.rs           # append_event
│   ├── projection.rs       # project_concept
│   ├── replay.rs           # rebuild_concept
│   ├── entities.rs         # create_entity, move_entity, discard_profile
│   └── errors.rs           # LedgerError
└── tests/
    └── substrate_loop.rs   # all integration tests
```

`ReferenceKind` is a Rust enum: `Supersedes`, `DerivedFrom`. v1 ships
only these two; the variant set will grow with future event types.

## Write path — `append_event`

```rust
pub async fn append_event(
    pool: &PgPool,
    write: EventToWrite,
) -> Result<Event, LedgerError>;

pub struct EventToWrite {
    pub id: Uuid,                       // caller-generated UUIDv7
    pub event_type: EventType,
    pub emitter_entity_id: Uuid,
    pub topic_id: Uuid,
    pub scope_id: Uuid,
    pub payload: serde_json::Value,
    pub metadata: serde_json::Value,
    pub references: Vec<EventReference>,
    pub correlation_id: Uuid,           // for a root event, caller sets this equal to `id`
    pub occurred_at: DateTime<Utc>,
}
```

Callers generate `id` via `Uuid::now_v7()` before the call so that the
correlation root (`correlation_id = id`) can be set in one step without
a second round-trip. A small `EventToWrite::new_root(...)` constructor
that generates the id and aligns `correlation_id` with it will be
provided as ergonomic sugar; the explicit-id form remains the primary
interface so causal-tree construction is visible at the call site.

In one transaction:

1. **Resolve `event_type_id`** — look up the `event_types` row whose
   `name` matches the Rust `EventType` discriminant. The Rust enum and
   the seeded `event_types` rows are kept in sync by convention (every
   new variant ships with a migration that inserts the matching row).
   Missing row is `LedgerError::UnknownEventType` and indicates a
   migration/code drift bug.
2. **Validate FKs** — entity, topic, scope all exist in their respective
   tables. Fail closed with the specific `LedgerError` variant.
3. **Validate references resolve** — every `event_id` in `references`
   must already exist in `events`. Fail with `DanglingReference { kind,
   event_id }`.
4. **Type-specific reference invariants** (Rust-side, not SQL):
   - `ConceptCreated` — references may include any number of
     `DerivedFrom` entries, MUST NOT include `Supersedes`. Returns
     `SupersedesOnGenesis`.
   - `ConceptMutated` — references MUST include exactly one
     `Supersedes` pointing to a prior `ConceptCreated` or `ConceptMutated`
     event whose `id` resolves to a row in `events`. Returns
     `MissingSupersedes` or `MultipleSupersedes`.
5. **INSERT** into `events` using `sqlx::query!()` for compile-time
   verification. The append-only trigger guards subsequent UPDATE/DELETE.
6. Return the persisted `Event`.

`append_event` is the *only* write entry point for events. Direct SQL
inserts from tests or other crates are explicitly disallowed by
convention (the crate exposes no helper for them).

## Projection path — `project_concept`

Called explicitly by the caller after `append_event` returns. Synchronous,
deterministic, no background workers in v1 — projection-as-pure-function
is the property the test suite verifies.

```rust
pub async fn project_concept(
    pool: &PgPool,
    event_id: Uuid,
) -> Result<Concept, LedgerError>;
```

- **`ConceptCreated`**: INSERT into `concepts` a new row with a fresh
  UUIDv7 id, `current_definition` and `current_elaboration` from the
  payload, `scope_id` and `topic_id` inherited from the event,
  `created_by_event_id = event.id`, `last_event_id = event.id`,
  `latest_event_recorded_at = event.recorded_at`. Returns the new row.
- **`ConceptMutated`**: walk back via `Supersedes` references to the
  rooted `ConceptCreated` event, find the concept row whose
  `created_by_event_id` matches that root, UPDATE its fields from the
  mutation payload (each text field optional in the payload — null means
  "no change"), set `last_event_id = event.id` and
  `latest_event_recorded_at = event.recorded_at`. Returns the updated row.

**Idempotency** — if the concept row's `last_event_id` already equals
`event_id`, `project_concept` returns the existing row unchanged. Makes
the function safe to call multiple times, which is what allows the
replay test to compare projection-of-record to rebuild-from-scratch.

## Replay path — `rebuild_concept`

```rust
pub async fn rebuild_concept(
    pool: &PgPool,
    concept_id: Uuid,
) -> Result<Concept, LedgerError>;
```

Given a `concept_id`:

1. Read the concept row, capture `created_by_event_id`.
2. Walk forward from the root: assemble all events whose `Supersedes`
   reference points (transitively) back to the root, ordered by
   `recorded_at`.
3. In a transaction: reset the concept row to its genesis state by
   re-projecting the root event over it, then re-project each mutation
   in order.
4. Return the rebuilt `Concept`.

The replay-purity assertion lives in the test suite: append a chain,
snapshot the `concepts` row, run `rebuild_concept`, assert field-by-field
equality. Divergence means the projection function is not pure — fail
loud.

## Entity / profile operations

```rust
pub async fn create_entity(
    pool: &PgPool,
    name: &str,
) -> Result<(Entity, Profile), LedgerError>;

pub async fn move_entity(
    pool: &PgPool,
    entity_id: Uuid,
    target_profile_id: Uuid,
) -> Result<Entity, LedgerError>;

pub async fn discard_profile(
    pool: &PgPool,
    profile_id: Uuid,
) -> Result<(), LedgerError>;
```

- `create_entity` runs in a single transaction: INSERT a new profile
  with name `"default profile for {name}"`, then INSERT the entity
  referencing it. Returns both rows.
- `move_entity` UPDATEs `entities.profile_id`. Returns the updated entity.
  Does **not** auto-discard the source profile even if it has become
  empty — discard is a separate, deliberate act.
- `discard_profile` errors with `ProfileNotEmpty` if any entity still
  references it; otherwise DELETEs the row.

## Errors — `LedgerError`

```rust
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("unknown entity: {0}")]
    UnknownEntity(Uuid),
    #[error("unknown topic: {0}")]
    UnknownTopic(Uuid),
    #[error("unknown scope: {0}")]
    UnknownScope(Uuid),
    #[error("unknown event type: {0}")]
    UnknownEventType(String),
    #[error("dangling reference: event {event_id} ({kind:?}) does not exist")]
    DanglingReference { event_id: Uuid, kind: ReferenceKind },
    #[error("ConceptMutated must include exactly one Supersedes reference; found none")]
    MissingSupersedes,
    #[error("ConceptMutated must include exactly one Supersedes reference; found multiple")]
    MultipleSupersedes,
    #[error("ConceptCreated must not include a Supersedes reference")]
    SupersedesOnGenesis,
    #[error("concept not found: {0}")]
    ConceptNotFound(Uuid),
    #[error("profile not empty: {0}")]
    ProfileNotEmpty(Uuid),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}
```

No silent-fallback paths. Every variant corresponds to a specific
malformed write or precondition failure.

## Test plan

All tests live in `crates/temper-events/tests/substrate_loop.rs` under
`#![cfg(feature = "test-db")]`. Use `#[sqlx::test]` for isolation per
test.

1. `create_entity_creates_default_profile` — entity-and-profile created
   atomically; profile name is derived from entity name.
2. `append_concept_created_projects_to_concept` — smallest happy-path
   round-trip.
3. `append_concept_mutated_without_supersedes_errors` — fail-closed
   reference invariant.
4. `append_concept_created_with_supersedes_errors` — genesis must not
   supersede.
5. `dangling_reference_errors` — `Supersedes` pointing at a non-existent
   event id.
6. `unknown_scope_errors` — every event must route to a known scope.
7. `unknown_topic_errors` — every event must route to a known topic.
8. `unknown_entity_errors` — every event must attribute to a known entity.
9. `events_table_is_append_only` — direct UPDATE on `events` raises;
   direct DELETE on `events` raises.
10. `mutation_chain_projects_correctly` — Create → Mutate → Mutate;
    final `concepts` row reflects the last mutation; intermediate
    mutations do not leak.
11. `rebuild_concept_equals_projection_of_record` — append a chain,
    capture `concepts` row, run `rebuild_concept`, assert field-by-field
    equality. The replay-purity test.
12. `project_concept_is_idempotent` — calling `project_concept` twice
    with the same `event_id` returns the same row, no double-application.
13. `move_entity_to_other_profile` — re-parenting updates
    `entities.profile_id`; source profile is unaffected.
14. `discard_empty_profile_succeeds` — profile with zero entities is
    deletable.
15. `discard_profile_with_entities_errors` — discard guard.
16. `correlation_id_groups_fan_out` — append two events sharing a
    correlation id; query by correlation id returns both.

`cargo make check` and `cargo nextest run --workspace --features
test-db` both pass before the v1 PR.

## Out of scope (v1)

Called out explicitly so the design doc holds the line — these are
Phase-2+ work, captured in the session arc, **not** dropped:

- Topic governance / aliases / supersession
- Entity typology (agent / human / integration) — note that this is
  deliberately deferred per the user's "we need a table to track
  entities but don't yet need typologies or source systems or
  deduplication" framing
- Identity-source FK on entities (the originating-authority axis from
  the 2026-05-14 substrate-design session)
- Profile merge events / linkage events / source-aware dedup
- Bridges (cross-scope reference event, 2026-05-17)
- Scars-as-typed-events — in v1, a "scar" is a `ConceptMutated` with a
  `correction_reason` field in the payload; promoting it to a typed
  event variant comes later
- Translation events (foundation-routed, 2026-05-18)
- Concept-genesis as a distinguished mediated event-class with its own
  invariants — v1 calls genesis `ConceptCreated` and does not gate the
  mediation property at the substrate
- Concept-side relationships table (typed/directional/weighted/bridged
  edges between materialized concepts, 2026-05-18 PM "first-class
  relationships") — v1 carries references on events only
- Trajectory + confidence metadata on cognition-event types
  (2026-05-18 AM)
- Variance-source / unknown-unknowns-risk signals on events
- Priority-weighted saturation thresholds, dirty-state cascade events
- xid8 watermark + transactional outbox + per-consumer subscription
  cursor (the 2026-05-17 "infrastructure layer — adopt, do not
  reinvent")
- Schema registry / upcasting machinery — the `metadata` jsonb column
  is the placeholder; the registry itself is Phase 2+
- Cluster-discrepancy detector (the 2026-05-18 PM deterministic
  front-end for translation candidates)
- Any HTTP, MCP, or CLI surface — pure library in v1

## Open questions / Follow-on phases

- **Topic seeding policy.** v1 seeds one `event_substrate.bootstrap`
  topic. Tests will create additional topics ad-hoc. The question of
  whether topic creation is itself an event (`TopicCreated`?) is open;
  v1 treats topics as substrate-internal config, not as event payloads.
- **Concept identity vs. event identity.** v1 keeps them independent
  (a concept has its own UUIDv7; the genesis event is referenced via
  `created_by_event_id`). The alternative — concept identity equals
  genesis event identity — is rejected for v1 because it conflates two
  things the model distinguishes elsewhere (the act of creation vs. the
  thing created). Worth re-pressuring once concept-side relationships
  land.
- **Payload typing strictness.** v1 enforces payload shapes in Rust
  (`ConceptCreatedPayload` is a typed struct) but accepts arbitrary
  `serde_json::Value` at the `append_event` boundary. A future schema
  registry could lift this to runtime validation; that's the
  `metadata.schema_uri` hook.
- **What carries the porosity declaration into the projection.** v1
  inherits `scope_id` onto `concepts` rows, which carries porosity
  transitively. Whether projection logic should *check* porosity (e.g.
  refuse to project a concept into a scope it doesn't have visibility
  into) is open — v1 says no, projection is pure record-keeping;
  visibility filtering is a query-time concern.

## Out-of-scope MVP framing

This spec deliberately does **not** address the 2026-05-14
"institutional-memory MVP" framing — the bootstrap-and-learning loop
that drives the cognitive-map of an organization from a real source.
That framing belongs above the substrate; the substrate is the layer
the framing eventually consumes. The risk this spec accepts is that the
v1 substrate is built without an applied workload pressing on it.
Mitigation: the test suite stands in for workload pressure by encoding
the model's invariants as assertions. If the assertions feel forced or
the projection function feels arbitrary, the model has a hole worth
finding before the surface is built on top of it.

## Deferred phase — external-system-event typology

Distinct from the cognition-event work in v1, the substrate's full
design includes a separate typology of events whose origin is an
external system of record (GitHub PR, Linear ticket, Notion doc, Slack
message). These are not just additional event types — they carry their
own write-path semantics that are absent from v1 and worth naming
explicitly so the v1 schema does not assume them away:

- **Raw-deterministic-event-with-external-reference shape.** The
  faithful record of an external change. Payload is a reference-payload
  (pointer + hash, no embedded content) per the 2026-05-17 design:
  "record raw external boundary events faithfully — cheap as
  reference-payloads — and debounce at the projection, never at
  ingestion." The external system stays the source of truth for its own
  state; the substrate stays record-faithful.
- **Two distinct trust layers on attribution.** The integration signs
  for the observation ("this is what GitHub told me at time T"); the
  identity-source is trusted for the attribution within that
  observation ("GitHub says it was user X"). v1's entity/profile model
  flattens this — there's no identity-source FK, no integration entity
  distinct from a human entity. The deferred typology will need both.
- **Sets, thresholds, and priority weighting.** Cognition events are
  not emitted one-per-boundary-event; they're emitted across a
  saturation threshold (the 2026-05-18 morning "process a topic region
  once it has accumulated enough coherent pressure"), modulated by a
  per-event-type priority multiplier (`effective_threshold ≈
  saturation_threshold / priority_multiplier`). Routine boundary events
  batch; an ADR or product-decision boundary forces immediate cognition
  passes. The batch decision is itself an emitted event so the
  batching judgment is auditable.
- **Dirty-state cascade events.** When a high-priority cognition event
  propagates into dependent scopes, downstream maps are not rebuilt
  instantaneously. A dirty-state event makes the propagation window
  visible to consumers, carrying source decision, affected scopes,
  affected topic areas, and severity class. Consumers route themselves
  without a central arbiter.

None of this is in v1. The `event_types` table and the `metadata` jsonb
column on `events` are the seams where this typology will attach: new
rows in `event_types` (`BoundaryEventObserved`, `CognitivePassInitiated`,
`DirtyStateAnnounced`, etc.), payload schemas in `metadata.schema_uri`,
and a separate write path (the routing-decision-as-event from 2026-05-17)
that does not bypass `append_event` but layers atop it.

## Deferred phase — concept-projection consumption pattern

v1 produces `concepts` rows; it does not yet define how Temper-the-product
(or any other knowledge-graph consumer) reads them. That consumption
pattern is itself a design problem and a follow-on phase. The shape of
the question, sketched here so it isn't lost:

- **Read-side abstraction.** Does Temper consume `concepts` directly via
  SQL views, through a `temper-events` client crate, or via a thin
  HTTP/MCP surface in front of the substrate? The 2026-05-18 PM session
  resolved that pgvector / tsvector / graph queries are what make a
  digital cognitive map more than an LLM-wiki — so the read-side likely
  needs to project richer artifacts than `concepts` rows alone (facets,
  embeddings, relationship traversal).
- **Subscription vs. snapshot semantics.** A consumer can subscribe to
  new events (the transactional outbox + per-consumer cursor pattern
  from the 2026-05-17 infrastructure tier) or query current state
  (`concepts` table). Both have a place; the snapshot use case is what
  v1 enables, the subscription use case is what an active consumer
  (e.g. an agent watching for translation candidates) will want.
- **Scope-filtered projection.** A consumer's view of the cognitive map
  is scope-bounded by the consumer's perspective (the 2026-05-18 morning
  agent-inherent scoping). The consumption surface needs a way to
  declare the perspective doing the reading — likely a session-scoped
  filter analogous to the existing Temper `resources_visible_to` /
  `can_modify_resource` pattern.
- **Embedding generation placement.** Currently `temper-ingest` owns
  embedding generation for Temper resources. For substrate concepts,
  whether the embeddings are computed at projection time, at first-read
  time, or in a separate background pass is open — and it interacts
  with the rebuildability primitive (replay must remain deterministic;
  embedding model versions are not).
- **Materialization staleness.** v1's `concepts.latest_event_recorded_at`
  lets a consumer detect a stale row, but does not yet define the
  consumer's recourse — re-read, request re-projection, request rebuild
  from event chain. The recourse design connects to the dirty-state
  cascade primitive above.

Whatever the consumption surface looks like, the substrate's contract
to it is the property v1 verifies in the test suite: every `concepts`
row is rebuildable from the event chain whose root is
`created_by_event_id`. That is the load-bearing invariant the consumer
can rely on.
