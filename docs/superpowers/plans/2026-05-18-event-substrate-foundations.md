# Event Substrate Foundations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the v1 event substrate — Postgres schema in a new `event_substrate` namespace, a new `temper-events` crate with no dependency on `temper-core`, and a write/project/replay loop verified by 16 integration tests.

**Architecture:** Append-only `events` ledger keyed by UUIDv7; `concepts` table as materialized projection; one write entry point (`append_event`) with explicit invariant validation; deterministic projection (`project_concept`); rebuildable replay (`rebuild_concept`). Entity/profile aggregator pattern with auto-default-profile-on-entity-creation. All tables in a separate Postgres schema (`event_substrate.*`) so the substrate is droppable without affecting production-Temper data.

**Tech Stack:** Rust 2021, `sqlx 0.8` (postgres / runtime-tokio-rustls / chrono / json / uuid / macros / migrate), `uuid 1` (v7), `chrono 0.4`, `serde 1`, `serde_json 1`, `thiserror 2`. Test runner: `cargo nextest`. Migrations applied via `sqlx::migrate!` macro and `#[sqlx::test(migrator = ...)]`.

**Spec:** `docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md`

---

## Standing rules for every task

Every implementer subagent dispatched against this plan MUST follow these
rules. Violations are escalation triggers — STOP and report rather than
soften the contract.

1. **`#![cfg(feature = "test-db")]`** at the top of every test file
   containing `#[sqlx::test]`. Required by the project's test-db gate
   convention; missing it causes CI unit-tests to fail in ~10ms per
   test with no DB available.
2. **Run `cargo make check` before staging.** The pre-commit hook is a
   backstop, not the first line. Failing check on files this task did
   not touch is itself a scope-creep signal — surface it, don't fix it
   inline.
3. **After any SQL change** (new migration, new `sqlx::query!()` call,
   new `sqlx::query_as!()`), regenerate the offline cache:
   `SQLX_OFFLINE=false DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx prepare --workspace -- --all-features`.
   Commit the resulting `.sqlx/` file changes alongside the SQL change.
4. **Pair filtered tests with a full-suite regression guard.** Every
   task ends with a full `cargo nextest run --workspace --features
   test-db` (not just the single test the task wrote). The single-test
   run is a development tool; the full run is the verification step.
5. **Do not trust nextest's "Summary" line.** With `--no-fail-fast` the
   last summary line is meaningless. Check exit code or grep for
   `error: test run failed` / `FAIL [` in output.
6. **Typed structs over `serde_json::json!()`.** The spec calls this
   out and the project's CLAUDE.md enforces it. Payloads have typed
   structs (`ConceptCreatedPayload`, `ConceptMutatedPayload`) —
   serialize them with `serde_json::to_value`, never construct them by
   hand from `json!()`.
7. **Match on enums, not strings.** `ReferenceKind`, `EventType`,
   `Porosity` are closed sets in v1 — exhaustive `match` statements,
   no string comparisons.
8. **If a test requires softening a contract to pass, STOP and
   escalate.** Do not catch-and-ignore errors, do not change a
   `Result<T, E>` to swallow a variant, do not relax an invariant to
   make a red test green. Report BLOCKED with the conflict and let the
   human decide.

**Database setup precondition:** Docker Postgres must be running on
port 5437. Verify with `docker ps | grep postgres` before any task
that runs tests. If absent, run `cargo make docker-up` from the repo
root.

---

## File structure

This is the complete file inventory the plan produces. Every file is
listed; if it isn't here, it isn't created.

### Migrations
- Create: `migrations/20260518000001_event_substrate_schema.sql`
- Create: `migrations/20260518000002_event_substrate_seed.sql`

### Crate: `crates/temper-events/`
- Create: `crates/temper-events/Cargo.toml`
- Create: `crates/temper-events/src/lib.rs`
- Create: `crates/temper-events/src/errors.rs`
- Create: `crates/temper-events/src/types/mod.rs`
- Create: `crates/temper-events/src/types/entity.rs`
- Create: `crates/temper-events/src/types/topic.rs`
- Create: `crates/temper-events/src/types/scope.rs`
- Create: `crates/temper-events/src/types/event.rs`
- Create: `crates/temper-events/src/types/concept.rs`
- Create: `crates/temper-events/src/payloads/mod.rs`
- Create: `crates/temper-events/src/payloads/concept_created.rs`
- Create: `crates/temper-events/src/payloads/concept_mutated.rs`
- Create: `crates/temper-events/src/entities.rs`
- Create: `crates/temper-events/src/ledger.rs`
- Create: `crates/temper-events/src/projection.rs`
- Create: `crates/temper-events/src/replay.rs`
- Create: `crates/temper-events/tests/substrate_loop.rs`

### Generated
- Create: `crates/temper-events/.sqlx/query-*.json` (via `cargo sqlx prepare`)

### Workspace touch points
The root `Cargo.toml` already declares `members = ["crates/*", "tests/e2e"]`, so the new crate is auto-included. No edits to the root manifest.

---

## Task 1: Scaffold the `temper-events` crate

**Files:**
- Create: `crates/temper-events/Cargo.toml`
- Create: `crates/temper-events/src/lib.rs`

- [ ] **Step 1: Create the Cargo.toml**

```toml
[package]
name = "temper-events"
version = "0.1.0"
edition = "2021"
description = "Event-sourced substrate foundations: append-only ledger, scoped events, concept projection"

[features]
default = []
test-db = []

[dependencies]
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sqlx = { version = "0.8", features = [
  "chrono",
  "json",
  "macros",
  "migrate",
  "postgres",
  "runtime-tokio-rustls",
  "uuid",
] }
thiserror = "2"
uuid = { version = "1", features = ["serde", "v7"] }

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

- [ ] **Step 2: Create the lib.rs with the MIGRATOR**

```rust
//! Event-sourced substrate foundations.
//!
//! See `docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md`.

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
```

- [ ] **Step 3: Verify the crate compiles**

Run: `cargo build -p temper-events`
Expected: clean build, no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-events/Cargo.toml crates/temper-events/src/lib.rs
git commit -m "Scaffold temper-events crate"
```

---

## Task 2: Schema migration

**Files:**
- Create: `migrations/20260518000001_event_substrate_schema.sql`

- [ ] **Step 1: Write the migration**

```sql
-- Event substrate v1 schema.
-- See docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md.

CREATE SCHEMA event_substrate;

CREATE TYPE event_substrate.porosity AS ENUM ('access', 'attention');

CREATE TABLE event_substrate.profiles (
    id          uuid PRIMARY KEY,
    name        text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE event_substrate.entities (
    id          uuid PRIMARY KEY,
    profile_id  uuid NOT NULL REFERENCES event_substrate.profiles(id),
    name        text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX entities_profile_id_idx
    ON event_substrate.entities(profile_id);

CREATE TABLE event_substrate.topics (
    id          uuid PRIMARY KEY,
    fqdn        text NOT NULL UNIQUE,
    parent_id   uuid REFERENCES event_substrate.topics(id),
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE event_substrate.scopes (
    id          uuid PRIMARY KEY,
    name        text NOT NULL UNIQUE,
    porosity    event_substrate.porosity NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE event_substrate.event_types (
    id              uuid PRIMARY KEY,
    name            varchar(128) NOT NULL UNIQUE,
    description     text,
    is_deprecated   boolean NOT NULL DEFAULT false,
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE event_substrate.events (
    id                   uuid PRIMARY KEY,
    event_type_id        uuid NOT NULL REFERENCES event_substrate.event_types(id),
    emitter_entity_id    uuid NOT NULL REFERENCES event_substrate.entities(id),
    topic_id             uuid NOT NULL REFERENCES event_substrate.topics(id),
    scope_id             uuid NOT NULL REFERENCES event_substrate.scopes(id),
    payload              jsonb NOT NULL,
    metadata             jsonb NOT NULL DEFAULT '{}'::jsonb,
    "references"         jsonb NOT NULL DEFAULT '[]'::jsonb,
    correlation_id       uuid NOT NULL,
    occurred_at          timestamptz NOT NULL,
    recorded_at          timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX events_topic_recorded_idx
    ON event_substrate.events(topic_id, recorded_at DESC);
CREATE INDEX events_event_type_recorded_idx
    ON event_substrate.events(event_type_id, recorded_at DESC);
CREATE INDEX events_emitter_recorded_idx
    ON event_substrate.events(emitter_entity_id, recorded_at DESC);
CREATE INDEX events_correlation_idx
    ON event_substrate.events(correlation_id);
CREATE INDEX events_references_gin_idx
    ON event_substrate.events USING gin ("references" jsonb_path_ops);

-- Append-only enforcement.
CREATE OR REPLACE FUNCTION event_substrate.events_append_only()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'event ledger is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER events_no_update_or_delete
    BEFORE UPDATE OR DELETE ON event_substrate.events
    FOR EACH ROW EXECUTE FUNCTION event_substrate.events_append_only();

CREATE TABLE event_substrate.concepts (
    id                        uuid PRIMARY KEY,
    current_definition        text NOT NULL,
    current_elaboration       text,
    scope_id                  uuid NOT NULL REFERENCES event_substrate.scopes(id),
    topic_id                  uuid NOT NULL REFERENCES event_substrate.topics(id),
    created_by_event_id       uuid NOT NULL REFERENCES event_substrate.events(id),
    last_event_id             uuid NOT NULL REFERENCES event_substrate.events(id),
    latest_event_recorded_at  timestamptz NOT NULL
);

CREATE INDEX concepts_scope_topic_idx
    ON event_substrate.concepts(scope_id, topic_id);
CREATE INDEX concepts_created_by_event_idx
    ON event_substrate.concepts(created_by_event_id);
CREATE INDEX concepts_last_event_idx
    ON event_substrate.concepts(last_event_id);
```

- [ ] **Step 2: Apply migration against the dev database**

Verify Postgres is up: `docker ps | grep postgres` — expected to show the `temper-postgres` container on port 5437.

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  sqlx migrate run --source migrations
```
Expected: `Applied 20260518000001/migrate event_substrate_schema` (and any other pending migrations).

- [ ] **Step 3: Verify the schema exists**

Run:
```bash
psql postgresql://temper:temper@localhost:5437/temper_development \
  -c "\dn event_substrate" \
  -c "\dt event_substrate.*"
```
Expected output includes the `event_substrate` schema and seven tables: `profiles`, `entities`, `topics`, `scopes`, `event_types`, `events`, `concepts`.

- [ ] **Step 4: Verify the append-only trigger raises**

Run:
```bash
psql postgresql://temper:temper@localhost:5437/temper_development \
  -c "BEGIN; INSERT INTO event_substrate.profiles (id, name) VALUES (gen_random_uuid(), 'trigger-test'); UPDATE event_substrate.profiles SET name='x' WHERE name='trigger-test'; ROLLBACK;"
```
Expected: profile insert succeeds; subsequent attempt at `UPDATE event_substrate.events` (run separately below) raises. Test the trigger directly:
```bash
psql postgresql://temper:temper@localhost:5437/temper_development \
  -c "UPDATE event_substrate.events SET payload='{}' WHERE id = gen_random_uuid();"
```
Expected error: `ERROR: event ledger is append-only`.

- [ ] **Step 5: Run `cargo make check`**

Run: `cargo make check`
Expected: PASS. (No new Rust code yet; this verifies the migration did not corrupt anything sqlx-cached.)

- [ ] **Step 6: Commit**

```bash
git add migrations/20260518000001_event_substrate_schema.sql
git commit -m "Add event_substrate schema migration"
```

---

## Task 3: Seed migration

**Files:**
- Create: `migrations/20260518000002_event_substrate_seed.sql`

The seed uses deterministic UUIDv7 values so test fixtures can reference
them by id. The UUIDs are hand-rolled (timestamp bytes + zero suffix) so
they sort correctly under the v7 ordering convention.

- [ ] **Step 1: Write the seed migration**

```sql
-- Event substrate v1 bootstrap rows.
-- Deterministic UUIDv7 values: timestamp 2026-05-18T00:00:00Z (ms = 1779062400000
-- = 0x019E3D6F2300) followed by version nibble 7 and zero-padded random bytes.
-- Each id increments the last 4 hex chars so they remain v7-sortable.

INSERT INTO event_substrate.event_types (id, name, description) VALUES
    ('019e3d6f-2300-7000-8000-000000000001', 'ConceptCreated',
     'Genesis event for a concept; emits initial characterization, scope, topic, and optional derived-from references.'),
    ('019e3d6f-2300-7000-8000-000000000002', 'ConceptMutated',
     'Refinement or correction of an existing concept; must reference the prior event via a single Supersedes reference.');

INSERT INTO event_substrate.scopes (id, name, porosity) VALUES
    ('019e3d6f-2300-7000-8000-000000000010', 'public', 'access');

INSERT INTO event_substrate.profiles (id, name) VALUES
    ('019e3d6f-2300-7000-8000-000000000020', 'system');

INSERT INTO event_substrate.entities (id, profile_id, name) VALUES
    ('019e3d6f-2300-7000-8000-000000000030',
     '019e3d6f-2300-7000-8000-000000000020',
     'system-bootstrap');

INSERT INTO event_substrate.topics (id, fqdn) VALUES
    ('019e3d6f-2300-7000-8000-000000000040', 'event_substrate.bootstrap');
```

- [ ] **Step 2: Apply the seed**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  sqlx migrate run --source migrations
```
Expected: `Applied 20260518000002/migrate event_substrate_seed`.

- [ ] **Step 3: Verify rows landed**

Run:
```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "
SELECT 'event_types' AS rel, count(*) FROM event_substrate.event_types
UNION ALL SELECT 'scopes', count(*) FROM event_substrate.scopes
UNION ALL SELECT 'profiles', count(*) FROM event_substrate.profiles
UNION ALL SELECT 'entities', count(*) FROM event_substrate.entities
UNION ALL SELECT 'topics', count(*) FROM event_substrate.topics;"
```
Expected counts: event_types=2, scopes=1, profiles=1, entities=1, topics=1.

- [ ] **Step 4: Run `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add migrations/20260518000002_event_substrate_seed.sql
git commit -m "Seed event_substrate bootstrap rows"
```

---

## Task 4: Error type, type module scaffold, Entity/Profile types

This is a "compile-only" task — no behavior, no test. The first test
(in Task 5) drives use of these types. Keep them tight; do not add
fields the spec does not require.

**Files:**
- Create: `crates/temper-events/src/errors.rs`
- Create: `crates/temper-events/src/types/mod.rs`
- Create: `crates/temper-events/src/types/entity.rs`
- Modify: `crates/temper-events/src/lib.rs`

- [ ] **Step 1: Write `errors.rs`**

```rust
use uuid::Uuid;

use crate::types::event::ReferenceKind;

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

- [ ] **Step 2: Write `types/entity.rs`**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Profile {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Entity {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}
```

- [ ] **Step 3: Write `types/mod.rs`** (placeholder for now; populated in Task 8)

```rust
pub mod entity;
pub mod event;

pub use entity::{Entity, Profile};
```

- [ ] **Step 4: Write a minimal `types/event.rs`** with just `ReferenceKind` so `errors.rs` compiles

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceKind {
    Supersedes,
    DerivedFrom,
}
```

- [ ] **Step 5: Update `lib.rs`** to wire modules and re-exports

```rust
//! Event-sourced substrate foundations.
//!
//! See `docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md`.

pub mod errors;
pub mod types;

pub use errors::LedgerError;
pub use types::{Entity, Profile};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
```

- [ ] **Step 6: Verify the crate compiles**

Run: `cargo build -p temper-events`
Expected: clean build.

- [ ] **Step 7: Run `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-events/src/errors.rs \
        crates/temper-events/src/types/mod.rs \
        crates/temper-events/src/types/entity.rs \
        crates/temper-events/src/types/event.rs \
        crates/temper-events/src/lib.rs
git commit -m "Add errors and Entity/Profile types for temper-events"
```

---

## Task 5: TDD `create_entity` (entity-and-profile created atomically)

**Files:**
- Create: `crates/temper-events/tests/substrate_loop.rs`
- Create: `crates/temper-events/src/entities.rs`
- Modify: `crates/temper-events/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/temper-events/tests/substrate_loop.rs`:

```rust
#![cfg(feature = "test-db")]

use sqlx::PgPool;
use temper_events::{create_entity, MIGRATOR};

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_entity_creates_default_profile(pool: PgPool) {
    let (entity, profile) = create_entity(&pool, "alice").await.expect("create_entity");

    assert_eq!(entity.name, "alice");
    assert_eq!(entity.profile_id, profile.id);
    assert_eq!(profile.name, "default profile for alice");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-events --features test-db create_entity_creates_default_profile`
Expected: FAIL — `temper_events::create_entity` is not in scope (function does not exist).

- [ ] **Step 3: Write `entities.rs`**

Create `crates/temper-events/src/entities.rs`:

```rust
use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::LedgerError;
use crate::types::{Entity, Profile};

pub async fn create_entity(
    pool: &PgPool,
    name: &str,
) -> Result<(Entity, Profile), LedgerError> {
    let mut tx = pool.begin().await?;

    let profile_id = Uuid::now_v7();
    let profile_name = format!("default profile for {name}");
    let profile = sqlx::query_as!(
        Profile,
        r#"
        INSERT INTO event_substrate.profiles (id, name)
        VALUES ($1, $2)
        RETURNING id, name, created_at
        "#,
        profile_id,
        profile_name,
    )
    .fetch_one(&mut *tx)
    .await?;

    let entity_id = Uuid::now_v7();
    let entity = sqlx::query_as!(
        Entity,
        r#"
        INSERT INTO event_substrate.entities (id, profile_id, name)
        VALUES ($1, $2, $3)
        RETURNING id, profile_id, name, created_at
        "#,
        entity_id,
        profile.id,
        name,
    )
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok((entity, profile))
}
```

- [ ] **Step 4: Re-export from `lib.rs`**

Edit `crates/temper-events/src/lib.rs` — replace the existing contents with:

```rust
//! Event-sourced substrate foundations.
//!
//! See `docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md`.

pub mod entities;
pub mod errors;
pub mod types;

pub use entities::create_entity;
pub use errors::LedgerError;
pub use types::{Entity, Profile};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
```

- [ ] **Step 5: Regenerate the sqlx offline cache**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```
Expected: `query data written to .sqlx in the workspace root`.

- [ ] **Step 6: Run the focused test to verify it passes**

Run: `cargo nextest run -p temper-events --features test-db create_entity_creates_default_profile`
Expected: PASS.

- [ ] **Step 7: Run the full-suite regression guard**

Run: `cargo nextest run --workspace --features test-db`
Expected: PASS for every test. Check exit code is 0; do not rely on the "Summary" line.

- [ ] **Step 8: Run `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-events/tests/substrate_loop.rs \
        crates/temper-events/src/entities.rs \
        crates/temper-events/src/lib.rs \
        crates/temper-events/.sqlx/
git commit -m "Add create_entity with auto-default-profile"
```

---

## Task 6: TDD `move_entity`

**Files:**
- Modify: `crates/temper-events/tests/substrate_loop.rs`
- Modify: `crates/temper-events/src/entities.rs`
- Modify: `crates/temper-events/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-events/tests/substrate_loop.rs`:

```rust
#[sqlx::test(migrator = "MIGRATOR")]
async fn move_entity_to_other_profile(pool: PgPool) {
    use temper_events::{move_entity};

    let (entity, source_profile) = create_entity(&pool, "alice").await.unwrap();
    let (_, target_profile) = create_entity(&pool, "bob").await.unwrap();

    let moved = move_entity(&pool, entity.id, target_profile.id).await.unwrap();
    assert_eq!(moved.profile_id, target_profile.id);

    // Source profile still exists, just unreferenced.
    let source_still_present: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM event_substrate.profiles WHERE id = $1)",
        source_profile.id,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(false);
    assert!(source_still_present, "move_entity must not auto-discard source profile");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-events --features test-db move_entity_to_other_profile`
Expected: FAIL — `move_entity` is not in scope.

- [ ] **Step 3: Implement `move_entity` in `entities.rs`**

Append to `crates/temper-events/src/entities.rs`:

```rust
pub async fn move_entity(
    pool: &PgPool,
    entity_id: Uuid,
    target_profile_id: Uuid,
) -> Result<Entity, LedgerError> {
    let entity = sqlx::query_as!(
        Entity,
        r#"
        UPDATE event_substrate.entities
           SET profile_id = $2
         WHERE id = $1
        RETURNING id, profile_id, name, created_at
        "#,
        entity_id,
        target_profile_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(LedgerError::UnknownEntity(entity_id))?;

    Ok(entity)
}
```

- [ ] **Step 4: Re-export from `lib.rs`**

In `crates/temper-events/src/lib.rs`, change the entities re-export:

```rust
pub use entities::{create_entity, move_entity};
```

- [ ] **Step 5: Regenerate sqlx cache**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 6: Run the focused test**

Run: `cargo nextest run -p temper-events --features test-db move_entity_to_other_profile`
Expected: PASS.

- [ ] **Step 7: Full-suite regression guard**

Run: `cargo nextest run --workspace --features test-db`
Expected: PASS.

- [ ] **Step 8: `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-events/tests/substrate_loop.rs \
        crates/temper-events/src/entities.rs \
        crates/temper-events/src/lib.rs \
        crates/temper-events/.sqlx/
git commit -m "Add move_entity (re-parent without auto-discard)"
```

---

## Task 7: TDD `discard_profile` (happy + sad paths)

**Files:**
- Modify: `crates/temper-events/tests/substrate_loop.rs`
- Modify: `crates/temper-events/src/entities.rs`
- Modify: `crates/temper-events/src/lib.rs`

- [ ] **Step 1: Write the two failing tests**

Append to `crates/temper-events/tests/substrate_loop.rs`:

```rust
#[sqlx::test(migrator = "MIGRATOR")]
async fn discard_empty_profile_succeeds(pool: PgPool) {
    use temper_events::{discard_profile, move_entity};

    let (entity, source_profile) = create_entity(&pool, "alice").await.unwrap();
    let (_, target_profile) = create_entity(&pool, "bob").await.unwrap();
    move_entity(&pool, entity.id, target_profile.id).await.unwrap();

    discard_profile(&pool, source_profile.id).await.unwrap();

    let still_present: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM event_substrate.profiles WHERE id = $1)",
        source_profile.id,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(false);
    assert!(!still_present);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn discard_profile_with_entities_errors(pool: PgPool) {
    use temper_events::{discard_profile, LedgerError};

    let (_, profile) = create_entity(&pool, "alice").await.unwrap();
    let err = discard_profile(&pool, profile.id).await.unwrap_err();
    assert!(matches!(err, LedgerError::ProfileNotEmpty(id) if id == profile.id));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run -p temper-events --features test-db discard_`
Expected: FAIL — `discard_profile` is not in scope.

- [ ] **Step 3: Implement `discard_profile`**

Append to `crates/temper-events/src/entities.rs`:

```rust
pub async fn discard_profile(
    pool: &PgPool,
    profile_id: Uuid,
) -> Result<(), LedgerError> {
    let mut tx = pool.begin().await?;

    let referencing_count: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM event_substrate.entities WHERE profile_id = $1",
        profile_id,
    )
    .fetch_one(&mut *tx)
    .await?
    .unwrap_or(0);

    if referencing_count > 0 {
        return Err(LedgerError::ProfileNotEmpty(profile_id));
    }

    let result = sqlx::query!(
        "DELETE FROM event_substrate.profiles WHERE id = $1",
        profile_id,
    )
    .execute(&mut *tx)
    .await?;

    if result.rows_affected() == 0 {
        return Err(LedgerError::ProfileNotEmpty(profile_id));
    }

    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 4: Re-export from `lib.rs`**

```rust
pub use entities::{create_entity, discard_profile, move_entity};
```

- [ ] **Step 5: Regenerate sqlx cache**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 6: Run the focused tests**

Run: `cargo nextest run -p temper-events --features test-db discard_`
Expected: PASS.

- [ ] **Step 7: Full-suite regression guard**

Run: `cargo nextest run --workspace --features test-db`
Expected: PASS.

- [ ] **Step 8: `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-events/tests/substrate_loop.rs \
        crates/temper-events/src/entities.rs \
        crates/temper-events/src/lib.rs \
        crates/temper-events/.sqlx/
git commit -m "Add discard_profile with not-empty guard"
```

---

## Task 8: Event-related types and payloads (compile-only scaffolding)

This task lands all remaining types so the first `append_event` test in
Task 9 only has to introduce one new function, not a wall of new types.
No tests; the next task drives the types through use.

**Files:**
- Modify: `crates/temper-events/src/types/event.rs`
- Create: `crates/temper-events/src/types/topic.rs`
- Create: `crates/temper-events/src/types/scope.rs`
- Create: `crates/temper-events/src/types/concept.rs`
- Create: `crates/temper-events/src/payloads/mod.rs`
- Create: `crates/temper-events/src/payloads/concept_created.rs`
- Create: `crates/temper-events/src/payloads/concept_mutated.rs`
- Modify: `crates/temper-events/src/types/mod.rs`
- Modify: `crates/temper-events/src/lib.rs`

- [ ] **Step 1: Write `types/topic.rs`**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Topic {
    pub id: Uuid,
    pub fqdn: String,
    pub parent_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}
```

- [ ] **Step 2: Write `types/scope.rs`** (mapping to the Postgres enum)

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "porosity", rename_all = "lowercase")]
pub enum Porosity {
    Access,
    Attention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Scope {
    pub id: Uuid,
    pub name: String,
    pub porosity: Porosity,
    pub created_at: DateTime<Utc>,
}
```

Note: `#[sqlx(type_name = "porosity")]` maps to the Postgres enum without
its schema qualifier; sqlx resolves the type from the current `search_path`.
The dev `search_path` includes `public` only by default, so the
`event_substrate.porosity` enum needs an explicit qualifier in queries
(handled by casting columns: `porosity AS "porosity!: Porosity"`).
Document this in a doc comment on `Porosity`.

- [ ] **Step 3: Write `types/event.rs`** (expanding the existing file)

Replace the contents of `crates/temper-events/src/types/event.rs` with:

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
    pub emitter_entity_id: Uuid,
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
    pub emitter_entity_id: Uuid,
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
        emitter_entity_id: Uuid,
        topic_id: Uuid,
        scope_id: Uuid,
        payload: serde_json::Value,
        occurred_at: DateTime<Utc>,
    ) -> Self {
        let id = Uuid::now_v7();
        Self {
            id,
            event_type,
            emitter_entity_id,
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

- [ ] **Step 4: Write `types/concept.rs`**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Concept {
    pub id: Uuid,
    pub current_definition: String,
    pub current_elaboration: Option<String>,
    pub scope_id: Uuid,
    pub topic_id: Uuid,
    pub created_by_event_id: Uuid,
    pub last_event_id: Uuid,
    pub latest_event_recorded_at: DateTime<Utc>,
}
```

- [ ] **Step 5: Write `payloads/mod.rs`**

```rust
pub mod concept_created;
pub mod concept_mutated;

pub use concept_created::ConceptCreatedPayload;
pub use concept_mutated::ConceptMutatedPayload;
```

- [ ] **Step 6: Write `payloads/concept_created.rs`**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConceptCreatedPayload {
    pub definition: String,
    pub elaboration: Option<String>,
}
```

- [ ] **Step 7: Write `payloads/concept_mutated.rs`**

```rust
use serde::{Deserialize, Serialize};

/// Each field is optional: `None` means "no change on this field."
/// `Some("")` is a deliberate update to an empty string and is preserved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConceptMutatedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elaboration: Option<String>,
}
```

- [ ] **Step 8: Update `types/mod.rs`**

```rust
pub mod concept;
pub mod entity;
pub mod event;
pub mod scope;
pub mod topic;

pub use concept::Concept;
pub use entity::{Entity, Profile};
pub use event::{Event, EventReference, EventToWrite, EventType, ReferenceKind};
pub use scope::{Porosity, Scope};
pub use topic::Topic;
```

- [ ] **Step 9: Update `lib.rs`** to expose the new types + payloads module

Replace `crates/temper-events/src/lib.rs` with:

```rust
//! Event-sourced substrate foundations.
//!
//! See `docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md`.

pub mod entities;
pub mod errors;
pub mod payloads;
pub mod types;

pub use entities::{create_entity, discard_profile, move_entity};
pub use errors::LedgerError;
pub use payloads::{ConceptCreatedPayload, ConceptMutatedPayload};
pub use types::{
    Concept, Entity, Event, EventReference, EventToWrite, EventType, Porosity, Profile,
    ReferenceKind, Scope, Topic,
};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
```

- [ ] **Step 10: Verify compilation**

Run: `cargo build -p temper-events`
Expected: clean build.

- [ ] **Step 11: `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 12: Commit**

```bash
git add crates/temper-events/src/types/ \
        crates/temper-events/src/payloads/ \
        crates/temper-events/src/lib.rs
git commit -m "Add event/topic/scope/concept types and concept payloads"
```

---

## Task 9: TDD `append_event` minimal happy path (ConceptCreated, no FK validation)

This task lands the simplest possible write path. FK validation
(distinguishing `UnknownEntity` from a generic SQL error) lands in Task
10; reference invariants land in Task 11. Keeping each invariant as a
separate test cycle isolates the change that introduced it.

**Files:**
- Modify: `crates/temper-events/tests/substrate_loop.rs`
- Create: `crates/temper-events/src/ledger.rs`
- Modify: `crates/temper-events/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-events/tests/substrate_loop.rs`:

```rust
use chrono::Utc;
use serde_json::json;
use temper_events::{
    append_event, EventToWrite, EventType,
};

const PUBLIC_SCOPE_ID: uuid::Uuid =
    uuid::uuid!("019e3d6f-2300-7000-8000-000000000010");
const SYSTEM_ENTITY_ID: uuid::Uuid =
    uuid::uuid!("019e3d6f-2300-7000-8000-000000000030");
const BOOTSTRAP_TOPIC_ID: uuid::Uuid =
    uuid::uuid!("019e3d6f-2300-7000-8000-000000000040");

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_concept_created_writes_to_ledger(pool: PgPool) {
    let payload = json!({
        "definition": "the digital cognitive map artifact model",
        "elaboration": "events + richly-related resources; markdown is one projection",
    });
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        payload.clone(),
        Utc::now(),
    );
    let event = append_event(&pool, write.clone()).await.expect("append_event");

    assert_eq!(event.id, write.id);
    assert_eq!(event.correlation_id, write.id);
    assert_eq!(event.emitter_entity_id, SYSTEM_ENTITY_ID);
    assert_eq!(event.payload, payload);

    // The row is in the ledger.
    let row_count: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM event_substrate.events WHERE id = $1",
        write.id,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(0);
    assert_eq!(row_count, 1);
}
```

Note: `uuid::uuid!` macro requires `uuid = { features = ["macros"] }`.
If it isn't already enabled, add it to dev-dependencies for this crate:

In `crates/temper-events/Cargo.toml`, change the `[dev-dependencies]`
section to:

```toml
[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
uuid = { version = "1", features = ["macros", "serde", "v7"] }
```

(The dep is inherited from `[dependencies]` but with the `macros`
feature only added in dev — that's fine; sqlx already pulls uuid as
well.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-events --features test-db append_concept_created_writes_to_ledger`
Expected: FAIL — `append_event` not in scope.

- [ ] **Step 3: Write `ledger.rs`**

Create `crates/temper-events/src/ledger.rs`:

```rust
use serde_json::Value;
use sqlx::PgPool;

use crate::errors::LedgerError;
use crate::types::event::{Event, EventToWrite};

pub async fn append_event(
    pool: &PgPool,
    write: EventToWrite,
) -> Result<Event, LedgerError> {
    let event_type_name = write.event_type.as_canonical_name();

    let event_type_id: uuid::Uuid = sqlx::query_scalar!(
        "SELECT id FROM event_substrate.event_types WHERE name = $1",
        event_type_name,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| LedgerError::UnknownEventType(event_type_name.to_string()))?;

    let references_json: Value = serde_json::to_value(&write.references)
        .expect("EventReference serialization is infallible");

    let event = sqlx::query_as!(
        Event,
        r#"
        INSERT INTO event_substrate.events (
            id, event_type_id, emitter_entity_id, topic_id, scope_id,
            payload, metadata, "references", correlation_id, occurred_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING
            id,
            event_type_id,
            emitter_entity_id,
            topic_id,
            scope_id,
            payload,
            metadata,
            "references",
            correlation_id,
            occurred_at,
            recorded_at
        "#,
        write.id,
        event_type_id,
        write.emitter_entity_id,
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

- [ ] **Step 4: Wire into `lib.rs`**

Replace `crates/temper-events/src/lib.rs` with:

```rust
//! Event-sourced substrate foundations.
//!
//! See `docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md`.

pub mod entities;
pub mod errors;
pub mod ledger;
pub mod payloads;
pub mod types;

pub use entities::{create_entity, discard_profile, move_entity};
pub use errors::LedgerError;
pub use ledger::append_event;
pub use payloads::{ConceptCreatedPayload, ConceptMutatedPayload};
pub use types::{
    Concept, Entity, Event, EventReference, EventToWrite, EventType, Porosity, Profile,
    ReferenceKind, Scope, Topic,
};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
```

- [ ] **Step 5: Regenerate sqlx cache**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 6: Run the focused test**

Run: `cargo nextest run -p temper-events --features test-db append_concept_created_writes_to_ledger`
Expected: PASS.

- [ ] **Step 7: Full-suite regression guard**

Run: `cargo nextest run --workspace --features test-db`
Expected: PASS.

- [ ] **Step 8: `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-events/tests/substrate_loop.rs \
        crates/temper-events/src/ledger.rs \
        crates/temper-events/src/lib.rs \
        crates/temper-events/Cargo.toml \
        crates/temper-events/.sqlx/
git commit -m "Add append_event happy path with event_type_id resolution"
```

---

## Task 10: TDD explicit FK validation (UnknownEntity / Topic / Scope)

The happy path passes by relying on Postgres FK errors surfacing as
`sqlx::Error`. This task lifts those checks into the `LedgerError`
variants so callers can program against typed errors.

**Files:**
- Modify: `crates/temper-events/tests/substrate_loop.rs`
- Modify: `crates/temper-events/src/ledger.rs`

- [ ] **Step 1: Write the three failing tests**

Append to `crates/temper-events/tests/substrate_loop.rs`:

```rust
use temper_events::LedgerError;
use uuid::Uuid;

#[sqlx::test(migrator = "MIGRATOR")]
async fn unknown_entity_errors(pool: PgPool) {
    let bogus = Uuid::now_v7();
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        bogus,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({"definition": "x"}),
        Utc::now(),
    );
    let err = append_event(&pool, write).await.unwrap_err();
    assert!(matches!(err, LedgerError::UnknownEntity(id) if id == bogus));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn unknown_topic_errors(pool: PgPool) {
    let bogus = Uuid::now_v7();
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        bogus,
        PUBLIC_SCOPE_ID,
        json!({"definition": "x"}),
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
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        bogus,
        json!({"definition": "x"}),
        Utc::now(),
    );
    let err = append_event(&pool, write).await.unwrap_err();
    assert!(matches!(err, LedgerError::UnknownScope(id) if id == bogus));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run -p temper-events --features test-db unknown_`
Expected: FAIL — the tests run but receive `LedgerError::Database(...)`
instead of the typed variants.

- [ ] **Step 3: Add explicit lookups to `ledger.rs`**

In `crates/temper-events/src/ledger.rs`, immediately after the
`event_type_id` lookup and before the INSERT, add:

```rust
    // Validate FKs explicitly so callers see typed errors instead of
    // raw Postgres foreign-key violations.
    let entity_exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM event_substrate.entities WHERE id = $1)",
        write.emitter_entity_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if !entity_exists {
        return Err(LedgerError::UnknownEntity(write.emitter_entity_id));
    }

    let topic_exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM event_substrate.topics WHERE id = $1)",
        write.topic_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if !topic_exists {
        return Err(LedgerError::UnknownTopic(write.topic_id));
    }

    let scope_exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM event_substrate.scopes WHERE id = $1)",
        write.scope_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if !scope_exists {
        return Err(LedgerError::UnknownScope(write.scope_id));
    }
```

- [ ] **Step 4: Regenerate sqlx cache**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 5: Run the focused tests**

Run: `cargo nextest run -p temper-events --features test-db unknown_`
Expected: PASS.

- [ ] **Step 6: Full-suite regression guard**

Run: `cargo nextest run --workspace --features test-db`
Expected: PASS.

- [ ] **Step 7: `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-events/tests/substrate_loop.rs \
        crates/temper-events/src/ledger.rs \
        crates/temper-events/.sqlx/
git commit -m "Add explicit FK validation to append_event"
```

---

## Task 11: TDD reference invariants (dangling, supersedes-on-genesis, missing/multiple supersedes)

**Files:**
- Modify: `crates/temper-events/tests/substrate_loop.rs`
- Modify: `crates/temper-events/src/ledger.rs`

- [ ] **Step 1: Write the four failing tests**

Append to `crates/temper-events/tests/substrate_loop.rs`:

```rust
use temper_events::{EventReference, ReferenceKind};

#[sqlx::test(migrator = "MIGRATOR")]
async fn dangling_reference_errors(pool: PgPool) {
    let bogus_event = Uuid::now_v7();
    let id = Uuid::now_v7();
    let write = EventToWrite {
        id,
        event_type: EventType::ConceptMutated,
        emitter_entity_id: SYSTEM_ENTITY_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({"definition": "x"}),
        metadata: json!({}),
        references: vec![EventReference {
            kind: ReferenceKind::Supersedes,
            event_id: bogus_event,
        }],
        correlation_id: id,
        occurred_at: Utc::now(),
    };
    let err = append_event(&pool, write).await.unwrap_err();
    assert!(matches!(
        err,
        LedgerError::DanglingReference { event_id, kind: ReferenceKind::Supersedes }
            if event_id == bogus_event
    ));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_concept_created_with_supersedes_errors(pool: PgPool) {
    // First, write a real ConceptCreated so the Supersedes target exists.
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({"definition": "root"}),
        Utc::now(),
    );
    append_event(&pool, root.clone()).await.unwrap();

    let id = Uuid::now_v7();
    let bad = EventToWrite {
        id,
        event_type: EventType::ConceptCreated,
        emitter_entity_id: SYSTEM_ENTITY_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({"definition": "x"}),
        metadata: json!({}),
        references: vec![EventReference {
            kind: ReferenceKind::Supersedes,
            event_id: root.id,
        }],
        correlation_id: id,
        occurred_at: Utc::now(),
    };
    let err = append_event(&pool, bad).await.unwrap_err();
    assert!(matches!(err, LedgerError::SupersedesOnGenesis));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_concept_mutated_without_supersedes_errors(pool: PgPool) {
    let id = Uuid::now_v7();
    let bad = EventToWrite {
        id,
        event_type: EventType::ConceptMutated,
        emitter_entity_id: SYSTEM_ENTITY_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({"definition": "x"}),
        metadata: json!({}),
        references: vec![],
        correlation_id: id,
        occurred_at: Utc::now(),
    };
    let err = append_event(&pool, bad).await.unwrap_err();
    assert!(matches!(err, LedgerError::MissingSupersedes));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_concept_mutated_with_multiple_supersedes_errors(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({"definition": "root"}),
        Utc::now(),
    );
    append_event(&pool, root.clone()).await.unwrap();

    let id = Uuid::now_v7();
    let bad = EventToWrite {
        id,
        event_type: EventType::ConceptMutated,
        emitter_entity_id: SYSTEM_ENTITY_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({"definition": "x"}),
        metadata: json!({}),
        references: vec![
            EventReference { kind: ReferenceKind::Supersedes, event_id: root.id },
            EventReference { kind: ReferenceKind::Supersedes, event_id: root.id },
        ],
        correlation_id: id,
        occurred_at: Utc::now(),
    };
    let err = append_event(&pool, bad).await.unwrap_err();
    assert!(matches!(err, LedgerError::MultipleSupersedes));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run -p temper-events --features test-db reference`
And: `cargo nextest run -p temper-events --features test-db supersedes`
Expected: all FAIL with wrong error variants or panic on `unwrap`.

- [ ] **Step 3: Add reference invariant checks to `ledger.rs`**

In `crates/temper-events/src/ledger.rs`, after the scope FK check and
before the INSERT, add:

```rust
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
            "SELECT EXISTS (SELECT 1 FROM event_substrate.events WHERE id = $1)",
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
```

Add the necessary import at the top of `ledger.rs`:

```rust
use crate::types::event::{Event, EventReference, EventToWrite, EventType, ReferenceKind};
```

(Replace the existing partial import.)

- [ ] **Step 4: Regenerate sqlx cache**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 5: Run the focused tests**

Run: `cargo nextest run -p temper-events --features test-db -E 'test(=dangling_reference_errors) | test(=append_concept_created_with_supersedes_errors) | test(=append_concept_mutated_without_supersedes_errors) | test(=append_concept_mutated_with_multiple_supersedes_errors)'`
Expected: all PASS.

- [ ] **Step 6: Full-suite regression guard**

Run: `cargo nextest run --workspace --features test-db`
Expected: PASS.

- [ ] **Step 7: `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-events/tests/substrate_loop.rs \
        crates/temper-events/src/ledger.rs \
        crates/temper-events/.sqlx/
git commit -m "Add reference invariant validation to append_event"
```

---

## Task 12: TDD append-only trigger verification

The trigger is already in the schema migration; this task adds the test
that verifies it raises. The test exercises the trigger via a raw SQL
UPDATE attempt — it does NOT use `append_event`.

**Files:**
- Modify: `crates/temper-events/tests/substrate_loop.rs`

- [ ] **Step 1: Write the test**

Append to `crates/temper-events/tests/substrate_loop.rs`:

```rust
#[sqlx::test(migrator = "MIGRATOR")]
async fn events_table_is_append_only(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({"definition": "trigger-test"}),
        Utc::now(),
    );
    append_event(&pool, root.clone()).await.unwrap();

    let update_err = sqlx::query!(
        "UPDATE event_substrate.events SET metadata = $1 WHERE id = $2",
        json!({"tampered": true}),
        root.id,
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(
        update_err.to_string().contains("event ledger is append-only"),
        "expected append-only trigger to raise; got: {}",
        update_err
    );

    let delete_err = sqlx::query!(
        "DELETE FROM event_substrate.events WHERE id = $1",
        root.id,
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(
        delete_err.to_string().contains("event ledger is append-only"),
        "expected append-only trigger to raise on DELETE; got: {}",
        delete_err
    );
}
```

- [ ] **Step 2: Regenerate sqlx cache** (the test introduces two new `sqlx::query!()` calls)

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 3: Run the test**

Run: `cargo nextest run -p temper-events --features test-db events_table_is_append_only`
Expected: PASS — the trigger raises with the documented message.

- [ ] **Step 4: Full-suite regression guard**

Run: `cargo nextest run --workspace --features test-db`
Expected: PASS.

- [ ] **Step 5: `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-events/tests/substrate_loop.rs \
        crates/temper-events/.sqlx/
git commit -m "Verify events ledger append-only trigger raises"
```

---

## Task 13: TDD `project_concept` for ConceptCreated

**Files:**
- Modify: `crates/temper-events/tests/substrate_loop.rs`
- Create: `crates/temper-events/src/projection.rs`
- Modify: `crates/temper-events/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-events/tests/substrate_loop.rs`:

```rust
use temper_events::project_concept;

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_concept_created_projects_to_concept(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({
            "definition": "the LLM-wiki is the wrong artifact model",
            "elaboration": "markdown is one lossy projection of a richer substrate",
        }),
        Utc::now(),
    );
    let event = append_event(&pool, root.clone()).await.unwrap();

    let concept = project_concept(&pool, event.id).await.expect("project_concept");

    assert_eq!(concept.current_definition, "the LLM-wiki is the wrong artifact model");
    assert_eq!(
        concept.current_elaboration.as_deref(),
        Some("markdown is one lossy projection of a richer substrate")
    );
    assert_eq!(concept.scope_id, PUBLIC_SCOPE_ID);
    assert_eq!(concept.topic_id, BOOTSTRAP_TOPIC_ID);
    assert_eq!(concept.created_by_event_id, event.id);
    assert_eq!(concept.last_event_id, event.id);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-events --features test-db append_concept_created_projects_to_concept`
Expected: FAIL — `project_concept` not in scope.

- [ ] **Step 3: Write `projection.rs`**

Create `crates/temper-events/src/projection.rs`:

```rust
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::LedgerError;
use crate::payloads::ConceptCreatedPayload;
use crate::types::concept::Concept;
use crate::types::event::{Event, EventType};

pub async fn project_concept(
    pool: &PgPool,
    event_id: Uuid,
) -> Result<Concept, LedgerError> {
    let event = load_event(pool, event_id).await?;
    let event_type = resolve_event_type(pool, event.event_type_id).await?;

    match event_type {
        EventType::ConceptCreated => project_created(pool, &event).await,
        EventType::ConceptMutated => unimplemented!("ConceptMutated projection lands in Task 14"),
    }
}

async fn project_created(pool: &PgPool, event: &Event) -> Result<Concept, LedgerError> {
    let payload: ConceptCreatedPayload = serde_json::from_value(event.payload.clone())
        .map_err(|e| LedgerError::Database(sqlx::Error::Decode(Box::new(e))))?;

    let concept_id = Uuid::now_v7();
    let concept = sqlx::query_as!(
        Concept,
        r#"
        INSERT INTO event_substrate.concepts (
            id, current_definition, current_elaboration,
            scope_id, topic_id,
            created_by_event_id, last_event_id, latest_event_recorded_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING
            id, current_definition, current_elaboration,
            scope_id, topic_id,
            created_by_event_id, last_event_id, latest_event_recorded_at
        "#,
        concept_id,
        payload.definition,
        payload.elaboration,
        event.scope_id,
        event.topic_id,
        event.id,
        event.id,
        event.recorded_at,
    )
    .fetch_one(pool)
    .await?;

    Ok(concept)
}

async fn load_event(pool: &PgPool, event_id: Uuid) -> Result<Event, LedgerError> {
    sqlx::query_as!(
        Event,
        r#"
        SELECT
            id, event_type_id, emitter_entity_id, topic_id, scope_id,
            payload, metadata, "references", correlation_id,
            occurred_at, recorded_at
        FROM event_substrate.events
        WHERE id = $1
        "#,
        event_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(LedgerError::DanglingReference {
        event_id,
        kind: crate::types::event::ReferenceKind::Supersedes,
    })
}

async fn resolve_event_type(
    pool: &PgPool,
    event_type_id: Uuid,
) -> Result<EventType, LedgerError> {
    let name: String = sqlx::query_scalar!(
        "SELECT name FROM event_substrate.event_types WHERE id = $1",
        event_type_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| LedgerError::UnknownEventType(format!("(by id) {event_type_id}")))?;

    match name.as_str() {
        "ConceptCreated" => Ok(EventType::ConceptCreated),
        "ConceptMutated" => Ok(EventType::ConceptMutated),
        other => Err(LedgerError::UnknownEventType(other.to_string())),
    }
}
```

- [ ] **Step 4: Wire into `lib.rs`**

Replace `crates/temper-events/src/lib.rs` with:

```rust
//! Event-sourced substrate foundations.
//!
//! See `docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md`.

pub mod entities;
pub mod errors;
pub mod ledger;
pub mod payloads;
pub mod projection;
pub mod types;

pub use entities::{create_entity, discard_profile, move_entity};
pub use errors::LedgerError;
pub use ledger::append_event;
pub use payloads::{ConceptCreatedPayload, ConceptMutatedPayload};
pub use projection::project_concept;
pub use types::{
    Concept, Entity, Event, EventReference, EventToWrite, EventType, Porosity, Profile,
    ReferenceKind, Scope, Topic,
};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
```

- [ ] **Step 5: Regenerate sqlx cache**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 6: Run the focused test**

Run: `cargo nextest run -p temper-events --features test-db append_concept_created_projects_to_concept`
Expected: PASS.

- [ ] **Step 7: Full-suite regression guard**

Run: `cargo nextest run --workspace --features test-db`
Expected: PASS.

- [ ] **Step 8: `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-events/tests/substrate_loop.rs \
        crates/temper-events/src/projection.rs \
        crates/temper-events/src/lib.rs \
        crates/temper-events/.sqlx/
git commit -m "Add project_concept for ConceptCreated"
```

---

## Task 14: TDD `project_concept` for ConceptMutated (chain test)

**Files:**
- Modify: `crates/temper-events/tests/substrate_loop.rs`
- Modify: `crates/temper-events/src/projection.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-events/tests/substrate_loop.rs`:

```rust
#[sqlx::test(migrator = "MIGRATOR")]
async fn mutation_chain_projects_correctly(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({"definition": "first", "elaboration": "elab1"}),
        Utc::now(),
    );
    let created = append_event(&pool, root.clone()).await.unwrap();
    let concept = project_concept(&pool, created.id).await.unwrap();

    // First mutation: change definition only.
    let m1_id = Uuid::now_v7();
    let m1 = EventToWrite {
        id: m1_id,
        event_type: EventType::ConceptMutated,
        emitter_entity_id: SYSTEM_ENTITY_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({"definition": "second"}),
        metadata: json!({}),
        references: vec![EventReference {
            kind: ReferenceKind::Supersedes,
            event_id: created.id,
        }],
        correlation_id: created.id,
        occurred_at: Utc::now(),
    };
    let m1_event = append_event(&pool, m1).await.unwrap();
    let after_m1 = project_concept(&pool, m1_event.id).await.unwrap();
    assert_eq!(after_m1.id, concept.id);
    assert_eq!(after_m1.current_definition, "second");
    assert_eq!(after_m1.current_elaboration.as_deref(), Some("elab1"));

    // Second mutation: change elaboration only.
    let m2_id = Uuid::now_v7();
    let m2 = EventToWrite {
        id: m2_id,
        event_type: EventType::ConceptMutated,
        emitter_entity_id: SYSTEM_ENTITY_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({"elaboration": "elab2"}),
        metadata: json!({}),
        references: vec![EventReference {
            kind: ReferenceKind::Supersedes,
            event_id: m1_event.id,
        }],
        correlation_id: created.id,
        occurred_at: Utc::now(),
    };
    let m2_event = append_event(&pool, m2).await.unwrap();
    let after_m2 = project_concept(&pool, m2_event.id).await.unwrap();
    assert_eq!(after_m2.current_definition, "second");
    assert_eq!(after_m2.current_elaboration.as_deref(), Some("elab2"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-events --features test-db mutation_chain_projects_correctly`
Expected: FAIL — `unimplemented!("ConceptMutated projection lands in Task 14")` panics.

- [ ] **Step 3: Implement `ConceptMutated` projection**

In `crates/temper-events/src/projection.rs`, replace the
`unimplemented!()` line in `project_concept` and add a new helper:

```rust
        EventType::ConceptMutated => project_mutated(pool, &event).await,
```

Then add the helper at the bottom of the file:

```rust
async fn project_mutated(pool: &PgPool, event: &Event) -> Result<Concept, LedgerError> {
    let payload: crate::payloads::ConceptMutatedPayload =
        serde_json::from_value(event.payload.clone())
            .map_err(|e| LedgerError::Database(sqlx::Error::Decode(Box::new(e))))?;

    let root_event_id = walk_to_root(pool, event.id).await?;

    // Locate the concept row by its genesis event.
    let concept = sqlx::query_as!(
        Concept,
        r#"
        SELECT
            id, current_definition, current_elaboration,
            scope_id, topic_id,
            created_by_event_id, last_event_id, latest_event_recorded_at
        FROM event_substrate.concepts
        WHERE created_by_event_id = $1
        "#,
        root_event_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(LedgerError::ConceptNotFound(root_event_id))?;

    let updated = sqlx::query_as!(
        Concept,
        r#"
        UPDATE event_substrate.concepts
           SET current_definition       = COALESCE($2, current_definition),
               current_elaboration      = CASE WHEN $3::boolean THEN $4 ELSE current_elaboration END,
               last_event_id            = $5,
               latest_event_recorded_at = $6
         WHERE id = $1
        RETURNING
            id, current_definition, current_elaboration,
            scope_id, topic_id,
            created_by_event_id, last_event_id, latest_event_recorded_at
        "#,
        concept.id,
        payload.definition,
        payload.elaboration.is_some(),
        payload.elaboration,
        event.id,
        event.recorded_at,
    )
    .fetch_one(pool)
    .await?;

    Ok(updated)
}

/// Walks `Supersedes` references back from the given event until we reach
/// a `ConceptCreated` event. Returns that genesis event's id.
async fn walk_to_root(pool: &PgPool, event_id: Uuid) -> Result<Uuid, LedgerError> {
    let mut current = event_id;
    loop {
        let event = load_event(pool, current).await?;
        let event_type = resolve_event_type(pool, event.event_type_id).await?;
        if matches!(event_type, EventType::ConceptCreated) {
            return Ok(current);
        }
        let refs: Vec<crate::types::event::EventReference> =
            serde_json::from_value(event.references.clone())
                .map_err(|e| LedgerError::Database(sqlx::Error::Decode(Box::new(e))))?;
        let supersedes = refs
            .into_iter()
            .find(|r| matches!(r.kind, crate::types::event::ReferenceKind::Supersedes))
            .ok_or(LedgerError::MissingSupersedes)?;
        current = supersedes.event_id;
    }
}
```

- [ ] **Step 4: Regenerate sqlx cache**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 5: Run the focused test**

Run: `cargo nextest run -p temper-events --features test-db mutation_chain_projects_correctly`
Expected: PASS.

- [ ] **Step 6: Full-suite regression guard**

Run: `cargo nextest run --workspace --features test-db`
Expected: PASS.

- [ ] **Step 7: `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-events/tests/substrate_loop.rs \
        crates/temper-events/src/projection.rs \
        crates/temper-events/.sqlx/
git commit -m "Add project_concept for ConceptMutated (Supersedes walk + COALESCE update)"
```

---

## Task 15: TDD projection idempotency

**Files:**
- Modify: `crates/temper-events/tests/substrate_loop.rs`
- Modify: `crates/temper-events/src/projection.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-events/tests/substrate_loop.rs`:

```rust
#[sqlx::test(migrator = "MIGRATOR")]
async fn project_concept_is_idempotent(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({"definition": "idempotency-test"}),
        Utc::now(),
    );
    let event = append_event(&pool, root.clone()).await.unwrap();

    let first = project_concept(&pool, event.id).await.unwrap();
    let second = project_concept(&pool, event.id).await.unwrap();

    assert_eq!(first, second);

    // The concepts table should hold exactly one row for this genesis event.
    let count: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM event_substrate.concepts WHERE created_by_event_id = $1",
        event.id,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(0);
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-events --features test-db project_concept_is_idempotent`
Expected: FAIL — second `project_concept` call inserts a second row,
violating the unique-projection invariant.

- [ ] **Step 3: Add the idempotency short-circuit**

In `crates/temper-events/src/projection.rs`, at the top of
`project_concept` (immediately after `let event = load_event(...)?;` and
`let event_type = resolve_event_type(...)?;`), insert the short-circuit:

```rust
    // Idempotency short-circuit: if a concept already has this event as
    // its last_event_id, return it unchanged. Makes the projection function
    // safe to call multiple times.
    let already_projected = sqlx::query_as!(
        Concept,
        r#"
        SELECT
            id, current_definition, current_elaboration,
            scope_id, topic_id,
            created_by_event_id, last_event_id, latest_event_recorded_at
        FROM event_substrate.concepts
        WHERE last_event_id = $1
        "#,
        event_id,
    )
    .fetch_optional(pool)
    .await?;
    if let Some(concept) = already_projected {
        return Ok(concept);
    }
```

- [ ] **Step 4: Regenerate sqlx cache**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 5: Run the focused test**

Run: `cargo nextest run -p temper-events --features test-db project_concept_is_idempotent`
Expected: PASS.

- [ ] **Step 6: Full-suite regression guard**

Run: `cargo nextest run --workspace --features test-db`
Expected: PASS.

- [ ] **Step 7: `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-events/tests/substrate_loop.rs \
        crates/temper-events/src/projection.rs \
        crates/temper-events/.sqlx/
git commit -m "Make project_concept idempotent via last_event_id short-circuit"
```

---

## Task 16: TDD `rebuild_concept` (replay-purity invariant)

This is the load-bearing test of the event-primary primitive. If
`rebuild_concept` and the projection-of-record diverge, the projection
function is not pure — failure here means the model has a hole.

**Files:**
- Modify: `crates/temper-events/tests/substrate_loop.rs`
- Create: `crates/temper-events/src/replay.rs`
- Modify: `crates/temper-events/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-events/tests/substrate_loop.rs`:

```rust
use temper_events::rebuild_concept;

#[sqlx::test(migrator = "MIGRATOR")]
async fn rebuild_concept_equals_projection_of_record(pool: PgPool) {
    // Build a Create -> Mutate -> Mutate chain.
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({"definition": "v0", "elaboration": "e0"}),
        Utc::now(),
    );
    let created = append_event(&pool, root.clone()).await.unwrap();
    project_concept(&pool, created.id).await.unwrap();

    let m1_id = Uuid::now_v7();
    let m1 = EventToWrite {
        id: m1_id,
        event_type: EventType::ConceptMutated,
        emitter_entity_id: SYSTEM_ENTITY_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({"definition": "v1"}),
        metadata: json!({}),
        references: vec![EventReference {
            kind: ReferenceKind::Supersedes,
            event_id: created.id,
        }],
        correlation_id: created.id,
        occurred_at: Utc::now(),
    };
    let m1_event = append_event(&pool, m1).await.unwrap();
    project_concept(&pool, m1_event.id).await.unwrap();

    let m2_id = Uuid::now_v7();
    let m2 = EventToWrite {
        id: m2_id,
        event_type: EventType::ConceptMutated,
        emitter_entity_id: SYSTEM_ENTITY_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({"elaboration": "e2"}),
        metadata: json!({}),
        references: vec![EventReference {
            kind: ReferenceKind::Supersedes,
            event_id: m1_event.id,
        }],
        correlation_id: created.id,
        occurred_at: Utc::now(),
    };
    let m2_event = append_event(&pool, m2).await.unwrap();
    let projection_of_record = project_concept(&pool, m2_event.id).await.unwrap();

    let rebuilt = rebuild_concept(&pool, projection_of_record.id).await.unwrap();

    assert_eq!(rebuilt, projection_of_record);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-events --features test-db rebuild_concept_equals_projection_of_record`
Expected: FAIL — `rebuild_concept` not in scope.

- [ ] **Step 3: Write `replay.rs`**

Create `crates/temper-events/src/replay.rs`:

```rust
use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::LedgerError;
use crate::projection::project_concept;
use crate::types::concept::Concept;
use crate::types::event::EventReference;

pub async fn rebuild_concept(
    pool: &PgPool,
    concept_id: Uuid,
) -> Result<Concept, LedgerError> {
    let concept = sqlx::query_as!(
        Concept,
        r#"
        SELECT
            id, current_definition, current_elaboration,
            scope_id, topic_id,
            created_by_event_id, last_event_id, latest_event_recorded_at
        FROM event_substrate.concepts
        WHERE id = $1
        "#,
        concept_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(LedgerError::ConceptNotFound(concept_id))?;

    let chain = collect_chain(pool, concept.created_by_event_id).await?;

    let mut tx = pool.begin().await?;
    sqlx::query!(
        "DELETE FROM event_substrate.concepts WHERE id = $1",
        concept_id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let mut latest: Option<Concept> = None;
    for event_id in chain {
        latest = Some(project_concept(pool, event_id).await?);
    }

    latest.ok_or(LedgerError::ConceptNotFound(concept_id))
}

/// Walk forward from a genesis event by collecting all events whose
/// `Supersedes` reference points (transitively) back to it, ordered by
/// `recorded_at`.
async fn collect_chain(pool: &PgPool, root_event_id: Uuid) -> Result<Vec<Uuid>, LedgerError> {
    // Recursive CTE traversing forward through Supersedes references.
    let rows = sqlx::query!(
        r#"
        WITH RECURSIVE chain AS (
            SELECT id, recorded_at, "references"
              FROM event_substrate.events
             WHERE id = $1
            UNION ALL
            SELECT e.id, e.recorded_at, e."references"
              FROM event_substrate.events e
              JOIN chain c
                ON e."references" @> jsonb_build_array(
                     jsonb_build_object('kind', 'Supersedes', 'event_id', c.id)
                   )
        )
        SELECT id AS "id!: Uuid", recorded_at AS "recorded_at!: chrono::DateTime<chrono::Utc>"
        FROM chain
        ORDER BY recorded_at ASC, id ASC
        "#,
        root_event_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| r.id).collect())
}
```

Note: the recursive CTE forward-traverses by checking whether each
candidate event's `references` jsonb array `@>` contains a matching
`Supersedes` entry. The GIN index on `references` (jsonb_path_ops) is
exactly what makes this affordable.

- [ ] **Step 4: Wire into `lib.rs`**

Replace `crates/temper-events/src/lib.rs` with:

```rust
//! Event-sourced substrate foundations.
//!
//! See `docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md`.

pub mod entities;
pub mod errors;
pub mod ledger;
pub mod payloads;
pub mod projection;
pub mod replay;
pub mod types;

pub use entities::{create_entity, discard_profile, move_entity};
pub use errors::LedgerError;
pub use ledger::append_event;
pub use payloads::{ConceptCreatedPayload, ConceptMutatedPayload};
pub use projection::project_concept;
pub use replay::rebuild_concept;
pub use types::{
    Concept, Entity, Event, EventReference, EventToWrite, EventType, Porosity, Profile,
    ReferenceKind, Scope, Topic,
};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
```

- [ ] **Step 5: Regenerate sqlx cache**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 6: Run the focused test**

Run: `cargo nextest run -p temper-events --features test-db rebuild_concept_equals_projection_of_record`
Expected: PASS.

If the test fails with a divergence (`assert_eq!` mismatch between
`rebuilt` and `projection_of_record`), STOP. This is the load-bearing
test of the event-primary primitive. Divergence means the projection
function is not pure. Report BLOCKED with the field-by-field diff;
do not paper over the difference.

- [ ] **Step 7: Full-suite regression guard**

Run: `cargo nextest run --workspace --features test-db`
Expected: PASS.

- [ ] **Step 8: `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-events/tests/substrate_loop.rs \
        crates/temper-events/src/replay.rs \
        crates/temper-events/src/lib.rs \
        crates/temper-events/.sqlx/
git commit -m "Add rebuild_concept (replay-purity invariant)"
```

---

## Task 17: TDD `correlation_id` groups fan-out

This is a query-only test; no new code is required. It exists to lock
the correlation_id semantic in as a regression guard.

**Files:**
- Modify: `crates/temper-events/tests/substrate_loop.rs`

- [ ] **Step 1: Write the test**

Append to `crates/temper-events/tests/substrate_loop.rs`:

```rust
#[sqlx::test(migrator = "MIGRATOR")]
async fn correlation_id_groups_fan_out(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({"definition": "fan-out root"}),
        Utc::now(),
    );
    let created = append_event(&pool, root.clone()).await.unwrap();

    // Two mutations sharing the root's correlation_id (fan-out under one intention).
    for label in ["m1", "m2"] {
        let id = Uuid::now_v7();
        let mutation = EventToWrite {
            id,
            event_type: EventType::ConceptMutated,
            emitter_entity_id: SYSTEM_ENTITY_ID,
            topic_id: BOOTSTRAP_TOPIC_ID,
            scope_id: PUBLIC_SCOPE_ID,
            payload: json!({"definition": label}),
            metadata: json!({}),
            references: vec![EventReference {
                kind: ReferenceKind::Supersedes,
                event_id: created.id,
            }],
            correlation_id: created.correlation_id,
            occurred_at: Utc::now(),
        };
        append_event(&pool, mutation).await.unwrap();
    }

    let count: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM event_substrate.events WHERE correlation_id = $1",
        created.correlation_id,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(0);
    assert_eq!(count, 3, "root + 2 mutations all share one correlation_id");
}
```

- [ ] **Step 2: Regenerate sqlx cache** (new `sqlx::query_scalar!`)

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 3: Run the focused test**

Run: `cargo nextest run -p temper-events --features test-db correlation_id_groups_fan_out`
Expected: PASS.

- [ ] **Step 4: Full-suite regression guard**

Run: `cargo nextest run --workspace --features test-db`
Expected: PASS — total test count for temper-events should be 16 by
this point (the spec's test plan plus the multiple-supersedes test
added in Task 11). Confirm with: `cargo nextest list -p temper-events
--features test-db | wc -l`.

- [ ] **Step 5: `cargo make check`**

Run: `cargo make check`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-events/tests/substrate_loop.rs \
        crates/temper-events/.sqlx/
git commit -m "Verify correlation_id groups fan-out events"
```

---

## Task 18: Final regression guard + PR prep

This task does no implementation work; it is the verification gate
before the branch is considered ready for review.

- [ ] **Step 1: Run the entire workspace test suite under both feature sets**

CI runs two distinct feature unifications. Reproduce both locally:

```bash
cargo nextest run --workspace --features test-db
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed
```

Expected: PASS for both. If the embed-gated run is unavailable locally
(no ONNX runtime installed), document that in the PR description and
let CI's Embed job verify.

- [ ] **Step 2: Run `cargo make check`**

Run: `cargo make check`
Expected: PASS — clippy, fmt, docs, machete, biome, tsc all green.

- [ ] **Step 3: Final spec/plan/reality verification**

Confirm by inspection:
- [ ] `crates/temper-events/src/` contains exactly the files listed in the File Structure section above. No stragglers.
- [ ] The `event_substrate` schema in the database matches the migration. Run `\dt event_substrate.*` in psql.
- [ ] No `serde_json::json!()` constructs appear in `crates/temper-events/src/` (only in tests). Grep: `rg 'json!\(' crates/temper-events/src/`. Expected: empty result.
- [ ] No string comparisons on `EventType` / `ReferenceKind` / `Porosity` outside of the canonical-name helpers. Grep: `rg 'as_str\(\) == "Concept' crates/temper-events/src/`. Expected: empty result.
- [ ] Every test file with `#[sqlx::test]` carries `#![cfg(feature = "test-db")]`. Grep: `head -1 crates/temper-events/tests/substrate_loop.rs`. Expected first line: `#![cfg(feature = "test-db")]`.

- [ ] **Step 4: Push the branch and open a PR**

```bash
git push -u origin jct/eventing-foundations
gh pr create --title "Event substrate foundations (temper-events crate + event_substrate schema)" \
  --body "$(cat <<'EOF'
## Summary

- New `event_substrate` Postgres schema: `entities`, `profiles`, `topics`, `scopes`, `event_types`, `events` (append-only via trigger), `concepts` (materialized projection).
- New `temper-events` crate (no dep on `temper-core`): types, payloads, `append_event`, `project_concept`, `rebuild_concept`, entity/profile ops.
- 16 integration tests covering the write-and-project-and-replay loop and every documented invariant.

Implements `docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md`.
Per `docs/superpowers/plans/2026-05-18-event-substrate-foundations.md`.

## Test plan

- [x] Unit + integration tests pass: `cargo nextest run --workspace --features test-db`
- [x] Quality checks pass: `cargo make check`
- [ ] CI's Embed & MCP Round-Trip Tests job confirms test-embed feature unification
- [x] Append-only trigger verified directly via psql
- [x] Replay-purity invariant verified (`rebuild_concept_equals_projection_of_record`)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review checklist (done before plan was published)

- [x] **Spec coverage** — every table in the spec maps to a CREATE in Task 2; every type maps to a struct in Task 4/8; every error variant has a test in Tasks 5/7/10/11/16; the write/project/replay loop is exercised by Tasks 9–17.
- [x] **No placeholders** — every code block is complete; no TODO/TBD/"add validation"; every command shows expected output.
- [x] **Type consistency** — `EventToWrite` fields match across the type definition (Task 8) and the constructor (Task 8) and every test (Tasks 9, 11, 14, 16, 17). `LedgerError` variants defined in Task 4 are referenced in Tasks 5, 7, 10, 11, 16. `ReferenceKind::Supersedes` / `DerivedFrom` are the only variants used throughout.
- [x] **Standing rules** — `#![cfg(feature = "test-db")]` set at the top of `tests/substrate_loop.rs` in Task 5 (the first task that creates it). `cargo sqlx prepare --workspace -- --all-features` appears after every SQL-touching task. Full-suite regression guard appears at the end of every test-bearing task. The "escalate, don't soften" rule is named in the Standing Rules and reinforced in Task 16's replay-purity step.

---

## Execution

**Plan complete and saved to `docs/superpowers/plans/2026-05-18-event-substrate-foundations.md`.** Two execution options:

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
