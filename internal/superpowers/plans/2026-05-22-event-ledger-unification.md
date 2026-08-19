# Event Ledger Unification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify Temper's two event models into one disciplined append-only ledger in the `public` schema — drop the test-only `event_substrate` schema and evolve `kb_events` into a registry-backed ledger — without changing any observable behaviour of the resource-write path.

**Architecture:** A single Postgres migration drops `event_substrate` and evolves `kb_events` (adds `event_type_id` FK + `kb_event_types`/`kb_topics`/`kb_scopes` tables, `references`/`correlation_id`/`topic_id`/`scope_id`/`occurred_at`/`metadata` columns, an append-only trigger). The `insert_event_and_audit` SQL function keeps its exact signature, so all 5 Rust callers and 2 TypeScript callers are untouched. The `temper-events` crate stays a leaf crate and is retargeted from `event_substrate.*` onto the `public` `kb_*` tables; its concept-projection and entity machinery (limb 2's concern) is removed. Three `temper-api` read/write sites that name the dropped `event_type` column are adjusted.

**Tech Stack:** Rust (sqlx 0.8, compile-time-checked macros + runtime queries), PostgreSQL 18 / pgvector, cargo-make, cargo-nextest.

**Spec:** `docs/superpowers/specs/2026-05-21-event-ledger-unification-design.md`

---

## Background the implementer needs

**Two event models exist today.**

- `kb_events` (`public`) — a *log*: `(id, profile_id, device_id, kb_context_id, resource_id, event_type VARCHAR, payload JSONB, created)`. Every resource mutation routes through the `insert_event_and_audit()` SQL function, which atomically writes a `kb_events` row + a `kb_resource_audits` row. No append-only enforcement, no registry, no references/topic/scope.
- `event_substrate.*` (separate schema, `temper-events` crate) — a *disciplined ledger* built by PR 81: append-only trigger, `event_types` FK registry, `references`/Supersedes, `correlation_id`, `occurred_at`/`recorded_at`, topics, scopes. **Never emitted into in production — entirely test-only.**

This plan makes `kb_events` the one disciplined ledger and retires `event_substrate`.

**The non-breaking strategy.** `insert_event_and_audit`'s signature is preserved exactly (still takes `p_event_type VARCHAR`); only its body changes — it resolves the type *name* to an `event_type_id` internally. So the 5 Rust callers (in `ingest_service.rs`, `resource_service.rs`) and 2 TypeScript callers (`temper-cloud`) need **zero changes**. Only code that *names the `event_type` column directly* changes: the 4 read queries in `event_service.rs` and the 2 direct-`INSERT` sites in `context_service.rs` / `access_service.rs`.

**Commit boundary — read this.** Dropping `event_substrate` breaks the `temper-events` crate's compilation until it is retargeted. The project's pre-commit hook runs `clippy` against the **live dev database** (it does not set `SQLX_OFFLINE`). Therefore **Tasks 1–5 are one coupled set and produce a single commit** — do not attempt to commit after Task 1, 2, 3, or 4 individually; the first green commit is Task 5. Tasks 6, 7, 8 each commit normally.

**`.sqlx` cache discipline** (per project memory `project_sqlx_per_crate_cache_for_feature_gated_tests`): `temper-events` has its own `crates/temper-events/.sqlx/`; the rest of the workspace uses the root `.sqlx/`. `cargo sqlx prepare --workspace` is **destructive** to `temper-events`'s feature-gated test queries. Always regenerate `temper-events` per-crate and the workspace separately, as the tasks below specify.

**Dev database:** `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` (must be running: `cargo make docker-up`).

---

## File Structure

**Created:**
- `migrations/20260522000001_event_ledger_unification.sql` — the whole migration.

**Modified (`temper-events` crate):**
- `crates/temper-events/src/types/event.rs` — `emitter_entity_id` → `emitter_profile_id`.
- `crates/temper-events/src/types/mod.rs`, `src/lib.rs` — drop concept/entity/projection/replay/payload re-exports.
- `crates/temper-events/src/errors.rs` — `UnknownEntity` → `UnknownProfile`; drop concept-only variants.
- `crates/temper-events/src/ledger.rs` — retarget all queries from `event_substrate.*` to `public` `kb_*` tables.
- `crates/temper-events/tests/substrate_loop.rs` — rewrite down to the ledger-discipline subset.

**Deleted (`temper-events` crate):**
- `src/entities.rs`, `src/projection.rs`, `src/replay.rs`, `src/payloads/` (whole dir), `src/types/concept.rs`, `src/types/entity.rs`.

**Modified (`temper-api`):**
- `crates/temper-api/src/services/event_service.rs` — 4 read queries join `kb_event_types`.
- `crates/temper-api/src/services/context_service.rs` — direct `INSERT` uses `event_type_id`.
- `crates/temper-api/src/services/access_service.rs` — direct `INSERT` uses `event_type_id`.

---

## Task 1: Schema migration

**Files:**
- Create: `migrations/20260522000001_event_ledger_unification.sql`

- [ ] **Step 1: Confirm the dev database is running and up to date**

Run: `cargo make docker-up && DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx migrate run`
Expected: all existing migrations applied, no error.

- [ ] **Step 2: Confirm the seeded system profile id**

Run: `sed -n '18,21p' migrations/20260330000002_seed.sql`
Expected: an `INSERT INTO kb_profiles (id, display_name)` row. Note its `id` — it is referenced later as the emitter profile in tests. This plan assumes `00000000-0000-0000-0004-000000000001`; if the seed differs, use the real value wherever this plan writes `SYSTEM_PROFILE_ID`.

- [ ] **Step 3: Write the migration file**

Create `migrations/20260522000001_event_ledger_unification.sql`:

```sql
-- Event ledger unification — limb 0 of the event-primary reorientation.
-- Drops the test-only `event_substrate` schema and evolves `kb_events`
-- into one disciplined, append-only, registry-backed ledger in `public`.
-- Spec: docs/superpowers/specs/2026-05-21-event-ledger-unification-design.md

-- ─── 1. Drop the test-only event_substrate schema ───────────────────────────
-- Never emitted into in production; a clean drop, not a data migration.
DROP SCHEMA IF EXISTS event_substrate CASCADE;

-- ─── 2. Ledger taxonomy tables ──────────────────────────────────────────────

CREATE TABLE kb_event_types (
    id            UUID PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    name          VARCHAR(128) NOT NULL UNIQUE,
    description   TEXT,
    is_deprecated BOOLEAN NOT NULL DEFAULT false,
    created       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE kb_topics (
    id        UUID PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    fqdn      TEXT NOT NULL UNIQUE,
    parent_id UUID REFERENCES kb_topics(id),
    created   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TYPE porosity AS ENUM ('access', 'attention');

CREATE TABLE kb_scopes (
    id       UUID PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    name     TEXT NOT NULL UNIQUE,
    porosity porosity NOT NULL,
    created  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ─── 3. Seed the event-type registry ────────────────────────────────────────
-- The event-type names emitted today, plus ConceptCreated/ConceptMutated
-- carried over from event_substrate as harmless registry rows (the concept
-- projection itself is limb 2).
INSERT INTO kb_event_types (name, description) VALUES
    ('resource_created',       'A resource was created.'),
    ('body_updated',           'A resource body was updated.'),
    ('managed_meta_updated',   'A resource''s managed/open frontmatter was updated.'),
    ('resource_deleted',       'A resource was soft-deleted.'),
    ('context_created',        'A context was created.'),
    ('join_request.submitted', 'A team join request was submitted.'),
    ('join_request.approved',  'A team join request was approved.'),
    ('ConceptCreated',         'Genesis event for a concept (projection is limb 2).'),
    ('ConceptMutated',         'Refinement of an existing concept (projection is limb 2).')
ON CONFLICT (name) DO NOTHING;

-- Catch any event-type string present in live data but not enumerated above.
INSERT INTO kb_event_types (name)
SELECT DISTINCT event_type FROM kb_events
ON CONFLICT (name) DO NOTHING;

-- ─── 4. Seed a root topic and a default scope ───────────────────────────────
-- Deterministic UUIDv7 ids so test fixtures can reference them by constant.
INSERT INTO kb_topics (id, fqdn) VALUES
    ('019e3d6f-2300-7000-8000-000000000040', 'temper.bootstrap');
INSERT INTO kb_scopes (id, name, porosity) VALUES
    ('019e3d6f-2300-7000-8000-000000000010', 'public', 'access');

-- ─── 5. Evolve kb_events into the disciplined ledger ────────────────────────

ALTER TABLE kb_events
    ADD COLUMN event_type_id  UUID REFERENCES kb_event_types(id),
    ADD COLUMN topic_id       UUID REFERENCES kb_topics(id),
    ADD COLUMN scope_id       UUID REFERENCES kb_scopes(id),
    ADD COLUMN metadata       JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN "references"   JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN correlation_id UUID,
    ADD COLUMN occurred_at    TIMESTAMPTZ NOT NULL DEFAULT now();

-- Backfill: map the legacy varchar type to its registry id; occurred_at
-- mirrors the recorded time for historical rows.
UPDATE kb_events e
   SET event_type_id = et.id
  FROM kb_event_types et
 WHERE et.name = e.event_type;
UPDATE kb_events SET occurred_at = created;

ALTER TABLE kb_events ALTER COLUMN event_type_id SET NOT NULL;
ALTER TABLE kb_events DROP COLUMN event_type;

CREATE INDEX idx_kb_events_event_type  ON kb_events(event_type_id);
CREATE INDEX idx_kb_events_topic       ON kb_events(topic_id) WHERE topic_id IS NOT NULL;
CREATE INDEX idx_kb_events_correlation ON kb_events(correlation_id) WHERE correlation_id IS NOT NULL;
CREATE INDEX idx_kb_events_references  ON kb_events USING GIN ("references" jsonb_path_ops);

-- ─── 6. Append-only enforcement ─────────────────────────────────────────────
-- Supersession and correction are themselves events; the ledger row is final.
CREATE FUNCTION kb_events_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'event ledger is append-only';
END;
$$;

CREATE TRIGGER kb_events_append_only
    BEFORE UPDATE OR DELETE ON kb_events
    FOR EACH ROW EXECUTE FUNCTION kb_events_append_only();

-- ─── 7. resolve_event_type — insert-or-get against the registry ─────────────
-- Used by insert_event_and_audit and by the direct-INSERT sites. Insert-or-get
-- keeps the resource-write path non-breaking: an unrecognised type name
-- auto-registers rather than erroring. Strict rejection of unknown types is
-- the temper-events `append_event` path's job, not this one's.
CREATE FUNCTION resolve_event_type(p_name VARCHAR) RETURNS UUID
LANGUAGE plpgsql AS $$
DECLARE
    v_id UUID;
BEGIN
    INSERT INTO kb_event_types (name) VALUES (p_name)
    ON CONFLICT (name) DO NOTHING;
    SELECT id INTO v_id FROM kb_event_types WHERE name = p_name;
    RETURN v_id;
END;
$$;

-- ─── 8. Evolve insert_event_and_audit to write event_type_id ────────────────
-- Signature is IDENTICAL to migration 20260521000001 — every Rust and
-- TypeScript caller is untouched. Only the body changes: resolve the type
-- name to an id, and insert event_type_id instead of the dropped column.
DROP FUNCTION IF EXISTS insert_event_and_audit(
    UUID, UUID, VARCHAR, UUID, UUID, VARCHAR, VARCHAR, TEXT, TEXT, TEXT, JSONB
);

CREATE FUNCTION insert_event_and_audit(
    p_event_id       UUID,
    p_profile_id     UUID,
    p_device_id      VARCHAR(64),
    p_context_id     UUID,
    p_resource_id    UUID,
    p_event_type     VARCHAR(64),
    p_action         VARCHAR(64),
    p_body_hash      TEXT,
    p_managed_hash   TEXT,
    p_open_hash      TEXT,
    p_payload_extra  JSONB DEFAULT '{}'::jsonb
) RETURNS TABLE(event_id UUID, audit_id UUID)
LANGUAGE plpgsql AS $$
DECLARE
    v_audit_id      UUID;
    v_event_type_id UUID;
BEGIN
    v_event_type_id := resolve_event_type(p_event_type);

    INSERT INTO kb_events (
        id, profile_id, device_id, kb_context_id, resource_id,
        event_type_id, payload, created
    )
    VALUES (
        p_event_id, p_profile_id, p_device_id, p_context_id, p_resource_id,
        v_event_type_id,
        jsonb_build_object(
            'body_hash', p_body_hash,
            'managed_hash', p_managed_hash,
            'open_hash', p_open_hash
        ) || COALESCE(p_payload_extra, '{}'::jsonb),
        now()
    );

    INSERT INTO kb_resource_audits (
        resource_id, event_id, profile_id, device_id,
        body_hash, managed_hash, open_hash, action
    )
    VALUES (
        p_resource_id, p_event_id, p_profile_id, p_device_id,
        p_body_hash, p_managed_hash, p_open_hash, p_action
    )
    RETURNING id INTO v_audit_id;

    RETURN QUERY SELECT p_event_id, v_audit_id;
END;
$$;
```

- [ ] **Step 4: Apply the migration to the dev database**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx migrate run`
Expected: `Applied 20260522000001/migrate event ledger unification`.

- [ ] **Step 5: Smoke-verify the new schema**

Run:
```bash
PGPASSWORD=temper psql -h localhost -p 5437 -U temper -d temper_development -c \
  "SELECT count(*) AS event_types FROM kb_event_types;
   SELECT to_regclass('event_substrate.events') AS substrate_gone;
   \d kb_events"
```
Expected: `event_types` ≥ 7; `substrate_gone` is NULL (schema dropped); `\d kb_events` shows `event_type_id`, `topic_id`, `scope_id`, `metadata`, `references`, `correlation_id`, `occurred_at` and **no** `event_type` column.

Do **not** commit yet — see "Commit boundary" above. Continue to Task 2.

---

## Task 2: Retarget `temper-events` types, errors, and module wiring

The crate stays a leaf ledger crate. Its concept-projection and entity
machinery is limb 2's concern and is removed now.

**Files:**
- Modify: `crates/temper-events/src/types/event.rs`
- Modify: `crates/temper-events/src/errors.rs`
- Modify: `crates/temper-events/src/types/mod.rs`
- Modify: `crates/temper-events/src/lib.rs`
- Delete: `crates/temper-events/src/entities.rs`, `src/projection.rs`, `src/replay.rs`, `src/types/concept.rs`, `src/types/entity.rs`, and the `src/payloads/` directory.

- [ ] **Step 1: Rename the emitter field in `event.rs`**

In `crates/temper-events/src/types/event.rs`, rename `emitter_entity_id` to `emitter_profile_id` in **both** structs (`Event` and `EventToWrite`) and in `EventToWrite::new_root`'s parameter list and body. The full corrected file:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    ConceptCreated,
    ConceptMutated,
}

impl EventType {
    pub fn as_canonical_name(self) -> &'static str {
        match self {
            EventType::ConceptCreated => "ConceptCreated",
            EventType::ConceptMutated => "ConceptMutated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceKind {
    Supersedes,
    DerivedFrom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventReference {
    pub kind: ReferenceKind,
    pub event_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Event {
    pub id: Uuid,
    pub event_type_id: Uuid,
    pub emitter_profile_id: Uuid,
    pub topic_id: Uuid,
    pub scope_id: Uuid,
    pub payload: serde_json::Value,
    pub metadata: serde_json::Value,
    #[sqlx(rename = "references")]
    pub references: serde_json::Value,
    pub correlation_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct EventToWrite {
    pub id: Uuid,
    pub event_type: EventType,
    pub emitter_profile_id: Uuid,
    pub topic_id: Uuid,
    pub scope_id: Uuid,
    pub payload: serde_json::Value,
    pub metadata: serde_json::Value,
    pub references: Vec<EventReference>,
    pub correlation_id: Uuid,
    pub occurred_at: DateTime<Utc>,
}

impl EventToWrite {
    /// Construct a root event whose `id` and `correlation_id` are equal
    /// and freshly generated.
    pub fn new_root(
        event_type: EventType,
        emitter_profile_id: Uuid,
        topic_id: Uuid,
        scope_id: Uuid,
        payload: serde_json::Value,
        occurred_at: DateTime<Utc>,
    ) -> Self {
        let id = Uuid::now_v7();
        Self {
            id,
            event_type,
            emitter_profile_id,
            topic_id,
            scope_id,
            payload,
            metadata: serde_json::json!({}),
            references: Vec::new(),
            correlation_id: id,
            occurred_at,
        }
    }
}
```

- [ ] **Step 2: Trim `errors.rs`**

In `crates/temper-events/src/errors.rs`, rename `UnknownEntity` to `UnknownProfile` and remove the concept/entity-only variants (`ConceptNotFound`, `ProfileNotEmpty`). Full corrected file:

```rust
use uuid::Uuid;

use crate::types::event::ReferenceKind;

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("unknown profile: {0}")]
    UnknownProfile(Uuid),
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
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}
```

- [ ] **Step 3: Delete the concept-projection and entity modules**

Run:
```bash
git rm crates/temper-events/src/entities.rs \
       crates/temper-events/src/projection.rs \
       crates/temper-events/src/replay.rs \
       crates/temper-events/src/types/concept.rs \
       crates/temper-events/src/types/entity.rs
git rm -r crates/temper-events/src/payloads
```

- [ ] **Step 4: Rewrite `types/mod.rs`**

Replace `crates/temper-events/src/types/mod.rs` with:

```rust
pub mod event;
pub mod scope;
pub mod topic;

pub use event::{Event, EventReference, EventToWrite, EventType, ReferenceKind};
pub use scope::{Porosity, Scope};
pub use topic::Topic;
```

- [ ] **Step 5: Rewrite `lib.rs`**

Replace `crates/temper-events/src/lib.rs` with:

```rust
//! Event-sourced ledger: append-only, scoped, registry-backed.
//!
//! Limb 0 of the event-primary reorientation. See
//! `docs/superpowers/specs/2026-05-21-event-ledger-unification-design.md`.

pub mod errors;
pub mod ledger;
pub mod types;

pub use errors::LedgerError;
pub use ledger::append_event;
pub use types::{
    Event, EventReference, EventToWrite, EventType, Porosity, ReferenceKind, Scope, Topic,
};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
```

`types/scope.rs` and `types/topic.rs` are unchanged — the `Scope`/`Topic`/`Porosity` shapes match the new `kb_scopes`/`kb_topics` tables as written.

Do **not** commit yet — the crate will not compile until Task 3 retargets `ledger.rs`. Continue to Task 3.

---

## Task 3: Retarget `ledger.rs` onto the `public` tables

**Files:**
- Modify: `crates/temper-events/src/ledger.rs`

- [ ] **Step 1: Replace `ledger.rs` in full**

The retargeted `append_event`: registry lookup against `kb_event_types`,
emitter validated against `kb_profiles`, topic/scope against
`kb_topics`/`kb_scopes`, references and the INSERT against `kb_events`.
The emitter error is now `UnknownProfile`. The INSERT supplies a fixed
`device_id` of `'ledger'` (the ledger-appended write path is distinct
from the device-scoped resource-write path) and leaves `kb_context_id` /
`resource_id` NULL. `created` (the recorded-at column) takes its default.

```rust
use serde_json::Value;
use sqlx::PgPool;

use crate::errors::LedgerError;
use crate::types::event::{Event, EventReference, EventToWrite, EventType, ReferenceKind};

pub async fn append_event(pool: &PgPool, write: EventToWrite) -> Result<Event, LedgerError> {
    let event_type_name = write.event_type.as_canonical_name();

    let event_type_id: uuid::Uuid = sqlx::query_scalar!(
        "SELECT id FROM kb_event_types WHERE name = $1",
        event_type_name,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| LedgerError::UnknownEventType(event_type_name.to_string()))?;

    // Validate FKs explicitly so callers see typed errors instead of
    // raw Postgres foreign-key violations.
    let profile_exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM kb_profiles WHERE id = $1)",
        write.emitter_profile_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if !profile_exists {
        return Err(LedgerError::UnknownProfile(write.emitter_profile_id));
    }

    let topic_exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM kb_topics WHERE id = $1)",
        write.topic_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if !topic_exists {
        return Err(LedgerError::UnknownTopic(write.topic_id));
    }

    let scope_exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM kb_scopes WHERE id = $1)",
        write.scope_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if !scope_exists {
        return Err(LedgerError::UnknownScope(write.scope_id));
    }

    // Reference invariants — type-specific.
    let supersedes_refs: Vec<&EventReference> = write
        .references
        .iter()
        .filter(|r| matches!(r.kind, ReferenceKind::Supersedes))
        .collect();

    match write.event_type {
        EventType::ConceptCreated => {
            if !supersedes_refs.is_empty() {
                return Err(LedgerError::SupersedesOnGenesis);
            }
        }
        EventType::ConceptMutated => match supersedes_refs.len() {
            0 => return Err(LedgerError::MissingSupersedes),
            1 => {}
            _ => return Err(LedgerError::MultipleSupersedes),
        },
    }

    // Validate every reference resolves to a real event.
    for reference in &write.references {
        let exists: bool = sqlx::query_scalar!(
            "SELECT EXISTS (SELECT 1 FROM kb_events WHERE id = $1)",
            reference.event_id,
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);
        if !exists {
            return Err(LedgerError::DanglingReference {
                event_id: reference.event_id,
                kind: reference.kind,
            });
        }
    }

    let references_json: Value = serde_json::to_value(&write.references)
        .expect("EventReference serialization is infallible");

    let event = sqlx::query_as!(
        Event,
        r#"
        INSERT INTO kb_events (
            id, event_type_id, profile_id, device_id, topic_id, scope_id,
            payload, metadata, "references", correlation_id, occurred_at
        )
        VALUES ($1, $2, $3, 'ledger', $4, $5, $6, $7, $8, $9, $10)
        RETURNING
            id,
            event_type_id,
            profile_id        AS "emitter_profile_id!",
            topic_id          AS "topic_id!",
            scope_id          AS "scope_id!",
            payload           AS "payload!",
            metadata,
            "references",
            correlation_id    AS "correlation_id!",
            occurred_at,
            created           AS "recorded_at!"
        "#,
        write.id,
        event_type_id,
        write.emitter_profile_id,
        write.topic_id,
        write.scope_id,
        write.payload,
        write.metadata,
        references_json,
        write.correlation_id,
        write.occurred_at,
    )
    .fetch_one(pool)
    .await?;

    Ok(event)
}
```

Note the forced-non-null aliases (`"…!"`): `kb_events.topic_id`,
`scope_id`, `correlation_id`, and `payload` are nullable columns (or, for
`payload`, has a default), but `append_event` always supplies them, so
the `Event` struct's non-`Option` fields are correct — the `!` tells
sqlx to trust that. `created` is aliased to the struct's `recorded_at`.

Do **not** commit yet. Continue to Task 4.

---

## Task 4: Rewrite the ledger-discipline test suite

The old `substrate_loop.rs` tested `create_entity`, `project_concept`,
and `rebuild_concept` — all removed. The surviving suite tests **ledger
discipline only**: append happy-path, FK validation, reference
invariants, the append-only trigger, and `correlation_id` grouping.
Replay-purity returns when limb 2 builds the concept projection.

**Files:**
- Modify (full rewrite): `crates/temper-events/tests/substrate_loop.rs`

- [ ] **Step 1: Replace `substrate_loop.rs` in full**

The emitter is the seeded system profile (confirmed in Task 1 Step 2).
The topic and scope are the deterministic seed ids from the migration.

```rust
#![cfg(feature = "test-db")]

use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use temper_events::{
    append_event, EventReference, EventToWrite, EventType, LedgerError, ReferenceKind, MIGRATOR,
};

// Seeded by migrations/20260330000002_seed.sql (kb_profiles) and
// migrations/20260522000001_event_ledger_unification.sql (topic, scope).
const SYSTEM_PROFILE_ID: Uuid = uuid::uuid!("00000000-0000-0000-0004-000000000001");
const BOOTSTRAP_TOPIC_ID: Uuid = uuid::uuid!("019e3d6f-2300-7000-8000-000000000040");
const PUBLIC_SCOPE_ID: Uuid = uuid::uuid!("019e3d6f-2300-7000-8000-000000000010");

fn mutation(
    id: Uuid,
    supersedes: Vec<Uuid>,
    correlation_id: Uuid,
) -> EventToWrite {
    EventToWrite {
        id,
        event_type: EventType::ConceptMutated,
        emitter_profile_id: SYSTEM_PROFILE_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({ "definition": "x" }),
        metadata: json!({}),
        references: supersedes
            .into_iter()
            .map(|event_id| EventReference {
                kind: ReferenceKind::Supersedes,
                event_id,
            })
            .collect(),
        correlation_id,
        occurred_at: Utc::now(),
    }
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_writes_to_ledger(pool: PgPool) {
    let payload = json!({ "definition": "the disciplined ledger" });
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        payload.clone(),
        Utc::now(),
    );
    let event = append_event(&pool, write.clone()).await.expect("append_event");

    assert_eq!(event.id, write.id);
    assert_eq!(event.correlation_id, write.id);
    assert_eq!(event.emitter_profile_id, SYSTEM_PROFILE_ID);
    assert_eq!(event.payload, payload);

    let row_count: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_events WHERE id = $1",
        write.id,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(0);
    assert_eq!(row_count, 1);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn unknown_profile_errors(pool: PgPool) {
    let bogus = Uuid::now_v7();
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        bogus,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({ "definition": "x" }),
        Utc::now(),
    );
    let err = append_event(&pool, write).await.unwrap_err();
    assert!(matches!(err, LedgerError::UnknownProfile(id) if id == bogus));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn unknown_topic_errors(pool: PgPool) {
    let bogus = Uuid::now_v7();
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        bogus,
        PUBLIC_SCOPE_ID,
        json!({ "definition": "x" }),
        Utc::now(),
    );
    let err = append_event(&pool, write).await.unwrap_err();
    assert!(matches!(err, LedgerError::UnknownTopic(id) if id == bogus));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn unknown_scope_errors(pool: PgPool) {
    let bogus = Uuid::now_v7();
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        BOOTSTRAP_TOPIC_ID,
        bogus,
        json!({ "definition": "x" }),
        Utc::now(),
    );
    let err = append_event(&pool, write).await.unwrap_err();
    assert!(matches!(err, LedgerError::UnknownScope(id) if id == bogus));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn dangling_reference_errors(pool: PgPool) {
    let bogus_event = Uuid::now_v7();
    let id = Uuid::now_v7();
    let err = append_event(&pool, mutation(id, vec![bogus_event], id))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        LedgerError::DanglingReference { event_id, kind: ReferenceKind::Supersedes }
            if event_id == bogus_event
    ));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn concept_created_with_supersedes_errors(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({ "definition": "root" }),
        Utc::now(),
    );
    append_event(&pool, root.clone()).await.unwrap();

    let id = Uuid::now_v7();
    let bad = EventToWrite {
        event_type: EventType::ConceptCreated,
        ..mutation(id, vec![root.id], id)
    };
    let err = append_event(&pool, bad).await.unwrap_err();
    assert!(matches!(err, LedgerError::SupersedesOnGenesis));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn concept_mutated_without_supersedes_errors(pool: PgPool) {
    let id = Uuid::now_v7();
    let err = append_event(&pool, mutation(id, vec![], id))
        .await
        .unwrap_err();
    assert!(matches!(err, LedgerError::MissingSupersedes));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn concept_mutated_with_multiple_supersedes_errors(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({ "definition": "root" }),
        Utc::now(),
    );
    append_event(&pool, root.clone()).await.unwrap();

    let id = Uuid::now_v7();
    let err = append_event(&pool, mutation(id, vec![root.id, root.id], id))
        .await
        .unwrap_err();
    assert!(matches!(err, LedgerError::MultipleSupersedes));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn ledger_is_append_only(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({ "definition": "trigger-test" }),
        Utc::now(),
    );
    append_event(&pool, root.clone()).await.unwrap();

    let update_err = sqlx::query("UPDATE kb_events SET metadata = $1 WHERE id = $2")
        .bind(json!({ "tampered": true }))
        .bind(root.id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        update_err.to_string().contains("event ledger is append-only"),
        "expected append-only trigger on UPDATE; got: {update_err}"
    );

    let delete_err = sqlx::query("DELETE FROM kb_events WHERE id = $1")
        .bind(root.id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        delete_err.to_string().contains("event ledger is append-only"),
        "expected append-only trigger on DELETE; got: {delete_err}"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn correlation_id_groups_fan_out(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({ "definition": "fan-out root" }),
        Utc::now(),
    );
    let created = append_event(&pool, root.clone()).await.unwrap();

    for _ in 0..2 {
        let id = Uuid::now_v7();
        append_event(&pool, mutation(id, vec![created.id], created.correlation_id))
            .await
            .unwrap();
    }

    let count: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_events WHERE correlation_id = $1",
        created.correlation_id,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(0);
    assert_eq!(count, 3, "root + 2 mutations share one correlation_id");
}
```

Note: the `UPDATE`/`DELETE` in `ledger_is_append_only` use runtime
`sqlx::query` (not the `query!` macro) deliberately — the macro would
reject a statement the append-only trigger guarantees fails.

---

## Task 5: Regenerate the `temper-events` cache, verify, and commit the coupled set

**Files:** none (build + cache + commit)

- [ ] **Step 1: Regenerate the `temper-events` per-crate `.sqlx` cache**

`temper-events` queries now hit `public` tables. Regenerate **per-crate**
(never `--workspace` — that destroys feature-gated test entries):

```bash
cd crates/temper-events
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare -- --features test-db --tests
cd ../..
```
Expected: `query data written to .sqlx in the current directory`.

- [ ] **Step 2: Build the crate**

Run: `cargo build -p temper-events`
Expected: compiles clean, no errors.

- [ ] **Step 3: Run the ledger-discipline tests**

Run: `cargo nextest run -p temper-events --features test-db`
Expected: 9 tests, all PASS.

- [ ] **Step 4: Verify the resource-write path is intact**

The migration changed `insert_event_and_audit`'s body. Prove the
non-breaking guarantee against the resource-write services:

Run: `cargo nextest run -p temper-api --features test-db`
Expected: PASS — no test modified, no regression.

- [ ] **Step 5: Quality gate**

Run: `cargo make check`
Expected: Rust fmt + clippy + docs + machete all green. If `machete`
flags a now-unused dependency in `crates/temper-events/Cargo.toml`,
remove it from `[dependencies]` and from the `cargo-machete` `ignored`
list, then re-run.

- [ ] **Step 6: Commit the coupled schema + crate set**

```bash
git add migrations/20260522000001_event_ledger_unification.sql \
        crates/temper-events/ \
        .sqlx/
git commit -m "feat: unify event ledger into public schema, retire event_substrate

Drop the test-only event_substrate schema; evolve kb_events into the
disciplined ledger (append-only trigger, kb_event_types registry,
references / correlation_id / topic_id / scope_id / occurred_at).
Retarget the temper-events crate onto the public kb_* tables and
remove the concept-projection / entity machinery (limb 2's concern).
insert_event_and_audit keeps its signature — resource-write callers
are untouched.

Limb 0 of the event-primary reorientation.
Spec: docs/superpowers/specs/2026-05-21-event-ledger-unification-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```
Note: `git add .sqlx/` here picks up only the `temper-events` crate's own
`crates/temper-events/.sqlx/` (its working dir) — the workspace root
`.sqlx/` is regenerated in Task 8.

---

## Task 6: Adjust `event_service.rs` read queries

The 4 read queries select `e.event_type` — a column that no longer
exists. They must join `kb_event_types` and project `name` under the
same alias so the `EventRow` struct (`event_type: String`, in
`temper-core::types::api`) is unchanged.

**Files:**
- Modify: `crates/temper-api/src/services/event_service.rs:21-114`

- [ ] **Step 1: Update all 4 query variants**

In each of the 4 `sqlx::query_as!(EventRow, ...)` blocks, make two edits:
1. In the `SELECT` list, replace `e.event_type,` with `et.name AS "event_type!",`.
2. After `FROM kb_events e`, add: `JOIN kb_event_types et ON et.id = e.event_type_id`.

For example, the `(Some(rid), Some(etype))` variant's query becomes:

```sql
WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
SELECT e.id, e.profile_id, e.device_id,
       e.kb_context_id as "kb_context_id: Uuid",
       e.resource_id as "resource_id: Uuid",
       et.name AS "event_type!", e.payload as "payload: serde_json::Value", e.created
  FROM kb_events e
  JOIN kb_event_types et ON et.id = e.event_type_id
 WHERE (e.profile_id = $1 OR e.resource_id IN (SELECT resource_id FROM visible))
   AND e.resource_id = $2
   AND et.name        = $3
 ORDER BY e.created DESC
 LIMIT $4 OFFSET $5
```

Apply the same two edits to all 4 variants. In the two variants that
filter by event type (`(Some, Some)` and `(None, Some)`), also change the
`WHERE e.event_type = $N` clause to `WHERE et.name = $N` (shown above for
the first; the `(None, Some)` variant's clause is `AND et.name = $2`).
The `"event_type!"` forced-non-null alias is required: `kb_event_types.name`
reached through a JOIN is inferred nullable by sqlx, but `EventRow.event_type`
is `String`.

- [ ] **Step 2: Build to verify the queries type-check**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo build -p temper-api`
Expected: compiles clean (the macros check live against the migrated DB).

- [ ] **Step 3: Run the event-service / handler tests**

Run: `cargo nextest run -p temper-api --features test-db -E 'test(event)'`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/services/event_service.rs
git commit -m "fix: event_service reads event_type via kb_event_types join

The event_type column was dropped in the ledger unification; read
queries now project kb_event_types.name under the same alias, so the
EventRow shape is unchanged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Adjust the two direct-`INSERT` sites

`context_service.rs` and `access_service.rs` `INSERT INTO kb_events`
naming the dropped `event_type` column. Both switch to `event_type_id`
via the `resolve_event_type()` SQL function.

**Files:**
- Modify: `crates/temper-api/src/services/context_service.rs` (the `INSERT INTO kb_events` in `create_context`)
- Modify: `crates/temper-api/src/services/access_service.rs` (the `INSERT INTO kb_events` in `emit_join_request_event`)

- [ ] **Step 1: Update `context_service.rs`**

Replace the `sqlx::query(...)` INSERT block with:

```rust
    let event_id = EventId::new();
    sqlx::query(
        "INSERT INTO kb_events (id, profile_id, device_id, kb_context_id, event_type_id, payload, created)
         VALUES ($1, $2, $3, $4, resolve_event_type($5), '{}', now())",
    )
    .bind(event_id)
    .bind(profile_id)
    .bind("api")
    .bind(id)
    .bind("context_created")
    .execute(&mut *tx)
    .await?;
```

This is a runtime `sqlx::query` — no `.sqlx` cache entry, no compile-time
column check; correctness is covered by the context-service tests.

- [ ] **Step 2: Update `access_service.rs`**

Replace the `sqlx::query!(...)` INSERT in `emit_join_request_event` with:

```rust
    let _ = sqlx::query!(
        "INSERT INTO kb_events (id, profile_id, device_id, event_type_id, payload, created)
         VALUES ($1, $2, 'system', resolve_event_type($3), $4, now())",
        event_id as EventId,
        profile_id,
        event_type,
        payload_json,
    )
    .execute(pool)
    .await;
```

This is the `query!` macro — it is compile-checked and **cached in the
workspace `.sqlx/`** (regenerated in Task 8).

- [ ] **Step 3: Build**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo build -p temper-api`
Expected: compiles clean.

- [ ] **Step 4: Run the affected service tests**

Run: `cargo nextest run -p temper-api --features test-db -E 'test(context) + test(access) + test(join)'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/context_service.rs \
        crates/temper-api/src/services/access_service.rs
git commit -m "fix: direct kb_events inserts resolve event_type_id

context_created and join-request events now resolve their type name to
an id via resolve_event_type(), matching the unified ledger schema.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Workspace cache regen and full verification

**Files:** none (cache + verification + commit)

- [ ] **Step 1: Regenerate the workspace `.sqlx` cache**

The `access_service.rs` `query!` macro changed. Regenerate the workspace
cache. This does **not** touch `crates/temper-events/.sqlx/` (that crate
has its own cache dir, already committed in Task 5):

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```
Expected: `query data written to .sqlx`.

- [ ] **Step 2: Confirm the temper-events cache survived**

Run: `git status --porcelain crates/temper-events/.sqlx/`
Expected: **no output** — the per-crate cache committed in Task 5 is
untouched. If files show as deleted, `--workspace` clobbered them;
restore with `git checkout crates/temper-events/.sqlx/` and re-run Task 5
Step 1.

- [ ] **Step 3: Full quality gate**

Run: `cargo make check`
Expected: all green.

- [ ] **Step 4: Rust test suites**

Run: `cargo make test && cargo make test-db`
Expected: unit + integration suites green.

- [ ] **Step 5: E2E suite**

Run: `cargo make test-e2e`
Expected: green — the CLI ↔ API ↔ DB resource flows still emit events
and audits correctly through the unified ledger.

- [ ] **Step 6: Commit the cache**

```bash
git add .sqlx/
git commit -m "chore: regenerate workspace sqlx cache for unified event ledger

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 7: Push and open the PR**

```bash
git push -u origin jct/event-ledger-unification
gh pr create --title "Event ledger unification (limb 0)" --body "$(cat <<'EOF'
Unifies Temper's two event models into one disciplined append-only
ledger in the `public` schema. Drops the test-only `event_substrate`
schema; evolves `kb_events` (append-only trigger, `kb_event_types`
registry, `references` / `correlation_id` / `topic_id` / `scope_id` /
`occurred_at`). `insert_event_and_audit` keeps its signature, so every
resource-write caller is untouched. Retargets the `temper-events` crate
onto the public tables.

Limb 0 of the event-primary reorientation.

Spec: `docs/superpowers/specs/2026-05-21-event-ledger-unification-design.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review

**Spec coverage** — checked against `2026-05-21-event-ledger-unification-design.md`:

- Drop `event_substrate` → Task 1 Step 3 §1. ✓
- `kb_event_types` / `kb_topics` / `kb_scopes` → Task 1 §2. ✓
- Evolve `kb_events` (append-only trigger, registry FK, `references`, `correlation_id`, `topic_id`, `scope_id`, `occurred_at`) → Task 1 §5–6. ✓
- Emitter reconciled onto `kb_profiles` → Task 1 (no parallel identity table) + Task 3 (`profile_id` validation). ✓
- Scope + context coexist → `kb_events` keeps `kb_context_id`, gains `scope_id`. ✓
- Non-breaking resource-write path (`insert_event_and_audit` signature preserved) → Task 1 §8, verified Task 5 Step 4 + Task 8 Step 5. ✓
- `temper-events` stays a leaf crate, retargeted → Tasks 2–5. ✓
- Concept projection removed (limb 2) → Task 2 Step 3. ✓
- Transplanted ledger-discipline tests → Task 4. ✓
- `event_substrate.concepts` not rebuilt; `ConceptCreated`/`ConceptMutated` carried as registry rows → Task 1 §3. ✓

**Deliberate addition beyond the spec's enumerated columns:** the `metadata JSONB` column on `kb_events`. The spec's column list was illustrative; `metadata` is part of PR 81's design ("transplants unchanged") and keeping it avoids churning the `Event`/`EventToWrite` structs. Flagged to the user at handoff.

**Placeholder scan:** none — every step has complete SQL/Rust/commands.

**Type consistency:** `emitter_profile_id` used consistently across `event.rs`, `ledger.rs`, `substrate_loop.rs`. `UnknownProfile` consistent across `errors.rs` and the test. `event_type_id` consistent across migration, `ledger.rs`, `event_service.rs`, `context_service.rs`, `access_service.rs`. The `"event_type!"` / `"…!"` forced-non-null aliases are noted where used.

**One open verification for the implementer:** Task 1 Step 2 — confirm the seeded `kb_profiles` id. The plan assumes `00000000-0000-0000-0004-000000000001`; if the seed differs, substitute the real value for `SYSTEM_PROFILE_ID` in Task 4.
