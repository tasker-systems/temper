# Admin-Event Sink Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make administration event-sourced — a queryable, firewalled admin ledger on `kb_events`, proven end-to-end by the grant chokepoint.

**Architecture:** Admin events ride `kb_events` with a **both-NULL producing anchor** (the cognition firewall — every region producer, the steward delta, and materialize attribution scope by anchor, so NULL-anchored events are structurally invisible to them). Because that firewall also hides admin events from every *reader*, the read path is `kb_events."references"` — a GIN-indexed, never-written column of typed provenance pointers, orthogonal to the anchor. Writers are SQL-resident: each admin act becomes a plpgsql function doing event-append + projection in one transaction, mirroring `facet_set`/`relationship_assert`.

**Tech Stack:** Rust (temper-substrate, temper-services, temper-api, temper-cli, temper-mcp), PostgreSQL 18 + plpgsql, sqlx, axum, rmcp, clap.

**Spec:** [`docs/superpowers/specs/2026-07-16-admin-event-sink-design.md`](../specs/2026-07-16-admin-event-sink-design.md)

**Scope:** Spec §9 steps 1–4. Step 5 (the remaining ~18 acts) is a **separate plan**, written once this plan's pattern exists. This plan delivers a queryable ledger and one proven writer pair.

## Global Constraints

- **Additive-only on `main`.** Every schema change is a forward migration. `main` auto-deploys; a big-bang change is never acceptable.
- **Never edit a shipped migration** — sqlx checksum-locks applied migrations.
- **Migrations use `uuid_generate_v7()`**, never native `uuidv7()` (breaks Neon PG17).
- **`CREATE OR REPLACE FUNCTION` cannot add a parameter.** A new param needs `DROP FUNCTION` + `CREATE`, which is a **write outage across deploy skew**. Get signatures right the first time.
- **The anchor rule:** authority acts are NULL-anchored, always. Never pass a producing anchor to an admin `_event_append`.
- **NULL anchor means "no cognition home", NOT "admin"** — `lens_created` is already in that bucket. Never write a reader that infers admin-ness from anchor nullity. Discriminate by event type or by `references`.
- **Admin payload key ban:** no admin payload may spell a key `resource_id`, `block_id`, `edge_id`, or `owner:{table,id}`. `element_trail_node`/`element_trail_edge` match on payload key shape with no type filter and are gated only by `resources_visible_to` — a violation leaks authority records to any reader of the resource. Use `subject_table`/`subject_id` and carry identity in `references`.
- **Typed structs over `serde_json::json!()`** for anything with a known shape.
- **SQL macros** (`sqlx::query!`) for production queries; regenerate caches after SQL changes (see Task 8).
- **Run `cargo make check` before every commit.**
- **Do not run migrations against prod and do not merge PRs.** Stop at "PR up + CI green + summary".

## Dependency (not a task in this plan)

Task `019f6b06-c48f-7a81-a238-cdd6b131f3dc` — *"Legacy profiles have no emitter entities"* — must be **applied to prod before Task 8's first event fires**. `resolve_emitter` is `fetch_one` with no lazy creation, and two approved prod users have zero emitters. It ships independently; this plan assumes emitters exist.

## File Structure

| File | Responsibility |
|---|---|
| `crates/temper-substrate/src/payloads.rs` (modify) | `AnchorTable` gains `Connections`/`MachineClients`; new `EventRef`/`RefRel`/`AdminGrantPayload` types |
| `crates/temper-substrate/src/events.rs` (modify) | `EventKind` variants + `SeedAction` arms for admin acts |
| `crates/temper-substrate/src/replay.rs` (modify) | `kb_access_grants` joins `INPUT_TABLES` |
| `crates/temper-services/src/services/admin_ledger_service.rs` (create) | The read surface: query by subject, query by actor, authz gate |
| `crates/temper-services/src/services/access_service.rs` (modify) | `insert_grant`/`delete_grant` become wrappers over the SQL fns; `delete_grant` gains an actor |
| `crates/temper-api/src/handlers/admin_ledger.rs` (create) | HTTP transport for the read surface |
| `crates/temper-cli/src/commands/admin_ledger.rs` (create) | `temper admin ledger` |
| `crates/temper-mcp/src/tools/admin_ledger.rs` (create) | MCP parity |
| `migrations/20260716000010_admin_event_types.sql` (create) | Event-type seeds + payload schemas |
| `migrations/20260716000020_admin_ledger_epoch.sql` (create) | The epoch marker |
| `migrations/20260716000030_admin_grant_fns.sql` (create) | `_admin_grant_created` / `_admin_grant_revoked` + projectors |

---

### Task 1: The `references` contract — typed shape + `AnchorTable` extension

`kb_events."references"` is `JSONB NOT NULL DEFAULT '[]'`, GIN-indexed (`idx_kb_events_references … USING GIN ("references" jsonb_path_ops)`), documented as `[{rel, target:{kind,id}}]`, and **never written in 9,835 events**. The `rel` vocabulary (`supersedes|derived_from|touches`) lives in a **comment, not a CHECK** — extending it needs no migration.

`AnchorTable` already has `Teams` and `Profiles` but lacks the admin subjects.

**Files:**
- Modify: `crates/temper-substrate/src/payloads.rs:31-46` (AnchorTable), and append new types
- Test: `crates/temper-substrate/src/payloads.rs` (inline `#[cfg(test)]` module — follow the file's existing convention)

**Interfaces:**
- Produces: `AnchorTable::{Connections, MachineClients}`; `RefRel::{Supersedes, DerivedFrom, Touches, Subject, Principal}`; `EventRef { rel: RefRel, target: AnchorRef }`; `EventRef::subject(AnchorRef) -> EventRef`; `EventRef::principal(AnchorRef) -> EventRef`.
- Consumes: existing `AnchorRef { table: AnchorTable, id: Uuid }` (`payloads.rs:59-62`).

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)]` module in `crates/temper-substrate/src/payloads.rs`:

```rust
#[test]
fn event_ref_serializes_to_the_documented_references_shape() {
    let team = Uuid::parse_str("019f6055-6aea-7aa2-a133-61552dd3d7e4").unwrap();
    let refs = vec![
        EventRef::subject(AnchorRef { table: AnchorTable::Connections, id: team }),
        EventRef::principal(AnchorRef { table: AnchorTable::Teams, id: team }),
    ];
    let json = serde_json::to_value(&refs).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            {"rel": "subject",   "target": {"kind": "kb_connections", "id": team}},
            {"rel": "principal", "target": {"kind": "kb_teams",       "id": team}},
        ]),
        "references must match the column's documented [{{rel, target:{{kind,id}}}}] shape"
    );
    let back: Vec<EventRef> = serde_json::from_value(json).unwrap();
    assert_eq!(back, refs, "references must round-trip");
}

#[test]
fn machine_clients_anchor_serializes_as_the_ddl_spells_it() {
    let j = serde_json::to_value(AnchorTable::MachineClients).unwrap();
    assert_eq!(j, serde_json::json!("kb_machine_clients"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-substrate -E 'test(event_ref_serializes)'`
Expected: FAIL — `cannot find type EventRef`, `no variant Connections`.

- [ ] **Step 3: Write minimal implementation**

Add the two variants to `AnchorTable` (`payloads.rs:31`), keeping the DDL-exact rename convention:

```rust
    #[serde(rename = "kb_connections")]
    Connections,
    #[serde(rename = "kb_machine_clients")]
    MachineClients,
```

Then append the reference types:

```rust
/// The `rel` vocabulary of `kb_events."references"`. The first three are the column's
/// original documented set; `Subject`/`Principal` are the admin-ledger extension (spec
/// 2026-07-16 §5). The vocabulary lives in a column COMMENT, not a CHECK, so this enum is
/// the only enforcement — keep it exhaustive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefRel {
    #[serde(rename = "supersedes")]
    Supersedes,
    #[serde(rename = "derived_from")]
    DerivedFrom,
    #[serde(rename = "touches")]
    Touches,
    /// What the act was performed ON (the grant's subject, the machine provisioned).
    #[serde(rename = "subject")]
    Subject,
    /// WHO the act was performed FOR (the team granted, the profile promoted).
    #[serde(rename = "principal")]
    Principal,
}

/// One typed provenance pointer in `kb_events."references"`.
///
/// This is the admin ledger's ONLY read path. Admin events are NULL-anchored (the cognition
/// firewall), which makes them invisible to every anchor-scoped reader — so identity must live
/// here, where the GIN index (`idx_kb_events_references`) can find it and no cognition reader
/// looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRef {
    pub rel: RefRel,
    /// Spelled `target` with a `kind` field to match the column's documented shape; `AnchorRef`
    /// serializes its table as `kind` via the wrapper below.
    pub target: RefTarget,
}

/// `AnchorRef`'s wire shape inside `references`: `{kind, id}` rather than `{table, id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefTarget {
    pub kind: AnchorTable,
    pub id: Uuid,
}

impl From<AnchorRef> for RefTarget {
    fn from(a: AnchorRef) -> Self {
        RefTarget { kind: a.table, id: a.id }
    }
}

impl EventRef {
    /// What the act was performed on.
    pub fn subject(target: impl Into<RefTarget>) -> Self {
        EventRef { rel: RefRel::Subject, target: target.into() }
    }
    /// Who the act was performed for.
    pub fn principal(target: impl Into<RefTarget>) -> Self {
        EventRef { rel: RefRel::Principal, target: target.into() }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-substrate -E 'test(event_ref_serializes) or test(machine_clients_anchor)'`
Expected: PASS (2 tests).

- [ ] **Step 5: Guard the exhaustiveness of the extended AnchorTable**

`AnchorTable` is matched elsewhere. Run the full crate to catch non-exhaustive matches:

Run: `cargo nextest run -p temper-substrate`
Expected: PASS. If a `match` on `AnchorTable` fails to compile, add the two arms — do **not** add a `_ =>` catch-all (a new variant must stay a compile error).

- [ ] **Step 6: Commit**

```bash
cargo make check
git add crates/temper-substrate/src/payloads.rs
git commit -m "feat(admin-ledger): typed references shape + admin AnchorTable variants

kb_events.references has been empty for 9,835 events. It is the admin ledger's
read path: NULL-anchored admin events are invisible to every anchor-scoped
reader by design, so identity lives in references where the GIN index finds it
and no cognition reader looks.

The rel vocabulary is a column comment, not a CHECK, so extending it with
subject/principal needs no migration."
```

---

### Task 2: The read service — query by subject, query by actor

The spec's central inversion: **the read path ships before any writer**, so the writers are built against a known query shape rather than a hypothetical one.

**Files:**
- Create: `crates/temper-services/src/services/admin_ledger_service.rs`
- Modify: `crates/temper-services/src/services/mod.rs` (register the module)
- Test: `crates/temper-services/tests/admin_ledger_test.rs`

**Interfaces:**
- Consumes: `temper_substrate::payloads::{EventRef, RefRel, RefTarget, AnchorTable}` (Task 1).
- Produces:
  - `AdminLedgerEntry { event_id: Uuid, event_type: String, actor_profile_id: Uuid, actor_handle: String, occurred_at: DateTime<Utc>, payload: serde_json::Value, references: Vec<EventRef>, correlation_id: Option<Uuid> }`
  - `list_by_subject(pool: &PgPool, caller: ProfileId, subject: RefTarget, limit: i64, offset: i64) -> ApiResult<Vec<AdminLedgerEntry>>`
  - `list_by_actor(pool: &PgPool, caller: ProfileId, actor: ProfileId, limit: i64, offset: i64) -> ApiResult<Vec<AdminLedgerEntry>>`
  - `ledger_epoch(pool: &PgPool) -> ApiResult<Option<DateTime<Utc>>>`

**Authorization** (spec §5, flagged for review and confirmed): the read gate **mirrors the write gate** — `is_system_admin` OR owner of the owning team, reusing `machine_authz::authorize` semantics. No new predicate.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-services/tests/admin_ledger_test.rs`:

```rust
#![cfg(feature = "test-db")]

use sqlx::PgPool;
use temper_substrate::payloads::{AnchorTable, RefTarget};
use uuid::Uuid;

mod common;

/// Insert a NULL-anchored admin event by hand. Task 8 replaces this with a real fire arm;
/// until then the read surface must be provable against a crafted row.
async fn seed_admin_event(pool: &PgPool, emitter: Uuid, subject: Uuid, principal: Uuid) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO kb_events
               (event_type_id, emitter_entity_id, payload, "references")
           SELECT et.id, $1,
                  jsonb_build_object('subject_table','kb_contexts','subject_id',$2::text),
                  jsonb_build_array(
                    jsonb_build_object('rel','subject',  'target', jsonb_build_object('kind','kb_contexts','id',$2)),
                    jsonb_build_object('rel','principal','target', jsonb_build_object('kind','kb_teams',   'id',$3))
                  )
             FROM kb_event_types et WHERE et.name = 'grant_created'
           RETURNING id"#,
    )
    .bind(emitter).bind(subject).bind(principal)
    .fetch_one(pool).await.expect("seed admin event")
}

#[sqlx::test]
async fn list_by_subject_finds_the_admin_event(pool: PgPool) {
    let f = common::admin_fixture(&pool).await;
    let ev = seed_admin_event(&pool, f.admin_emitter, f.context_id, f.team_id).await;

    let got = temper_services::services::admin_ledger_service::list_by_subject(
        &pool,
        f.admin_profile,
        RefTarget { kind: AnchorTable::Contexts, id: f.context_id },
        50,
        0,
    )
    .await
    .expect("list_by_subject");

    assert_eq!(got.len(), 1, "the seeded grant_created must be found by its subject reference");
    assert_eq!(got[0].event_id, ev);
    assert_eq!(got[0].event_type, "grant_created");
    assert_eq!(got[0].actor_profile_id, f.admin_profile.uuid());
}

#[sqlx::test]
async fn the_admin_event_is_invisible_to_cognition(pool: PgPool) {
    let f = common::admin_fixture(&pool).await;
    seed_admin_event(&pool, f.admin_emitter, f.context_id, f.team_id).await;

    // The firewall: a NULL-anchored event must not be counted by the steward's ingest delta.
    let new_events: i64 = sqlx::query_scalar(
        "SELECT new_events FROM steward_ingest_delta($1, NULL)",
    )
    .bind(f.team_id)
    .fetch_one(&pool)
    .await
    .unwrap_or(0);

    assert_eq!(new_events, 0, "NULL-anchored admin events must not reach the steward delta");
}

#[sqlx::test]
async fn a_non_admin_cannot_read_the_ledger(pool: PgPool) {
    let f = common::admin_fixture(&pool).await;
    seed_admin_event(&pool, f.admin_emitter, f.context_id, f.team_id).await;

    let err = temper_services::services::admin_ledger_service::list_by_subject(
        &pool,
        f.outsider_profile,
        RefTarget { kind: AnchorTable::Contexts, id: f.context_id },
        50,
        0,
    )
    .await
    .expect_err("an outsider must not read the admin ledger");

    assert!(
        matches!(err, temper_services::ApiError::NotFound(_)),
        "reads deny with 404, not 403 (the deny-split invariant); got {err:?}"
    );
}
```

- [ ] **Step 2: Build the fixture**

Add to `crates/temper-services/tests/common/mod.rs` (create if absent, following `crates/temper-api/tests/common/` for the established shape):

```rust
pub struct AdminFixture {
    pub admin_profile: temper_core::types::ids::ProfileId,
    pub admin_emitter: uuid::Uuid,
    pub outsider_profile: temper_core::types::ids::ProfileId,
    pub team_id: uuid::Uuid,
    pub context_id: uuid::Uuid,
}

/// A system-admin with an emitter, an outsider, a team, and a context to grant on.
pub async fn admin_fixture(pool: &sqlx::PgPool) -> AdminFixture {
    // Build via the real service paths, never raw INSERTs, so the fixture cannot
    // drift from production shape (fixtures that fill columns prod leaves empty lie).
    todo!("construct via profile_service::provision_profile_entities + team_service::create_team")
}
```

**This `todo!()` is the one place this plan defers to the implementer** — the fixture must be built from whatever `common/` helpers already exist in `crates/temper-api/tests/common/`. Read those first and reuse. Do **not** hand-INSERT profiles: `provision_profile_entities` is what creates the emitter the ledger reads, and a fixture that skips it will pass while prod 500s.

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run -p temper-services --features test-db --test admin_ledger_test`
Expected: FAIL — `admin_ledger_service` does not exist.

Export `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` and ensure `cargo make docker-up` has run.

- [ ] **Step 4: Write the implementation**

Create `crates/temper-services/src/services/admin_ledger_service.rs`:

```rust
//! The admin ledger's read surface.
//!
//! Admin events are NULL-anchored (spec 2026-07-16 §4) — the cognition firewall. That firewall
//! is structural: every region producer, `steward_ingest_delta`, materialize attribution, and
//! `latest_event_id_for_context` scope by `producing_anchor_table`, so a both-NULL event is
//! invisible to all of them. It is equally invisible to every *reader*, which is why identity
//! lives in `kb_events."references"` (GIN-indexed, and consulted by no cognition reader).
//!
//! Two axes, both index-backed:
//!   - by subject  → `references @> …`      (idx_kb_events_references, jsonb_path_ops)
//!   - by actor    → `emitter_entity_id = …` (idx_kb_events_emitter, (emitter, occurred_at DESC))

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_substrate::payloads::{EventRef, RefTarget};
use uuid::Uuid;

use crate::{ApiError, ApiResult};

#[derive(Debug, Clone, serde::Serialize)]
pub struct AdminLedgerEntry {
    pub event_id: Uuid,
    pub event_type: String,
    pub actor_profile_id: Uuid,
    pub actor_handle: String,
    pub occurred_at: DateTime<Utc>,
    pub payload: serde_json::Value,
    pub references: Vec<EventRef>,
    pub correlation_id: Option<Uuid>,
}

/// Admin event types. The ledger read surface returns ONLY these — never cognition events that
/// happen to share the NULL-anchor bucket (`lens_created` is already in it). Discriminating by
/// anchor nullity would silently absorb system-config events; discriminate by type.
const ADMIN_EVENT_TYPES: &[&str] = &["admin_ledger_opened", "grant_created", "grant_revoked"];

/// The read gate mirrors the write gate: if you could perform the act, you may read the record
/// of it. Reads deny with 404, not 403 (the deny-split invariant).
async fn gate(pool: &PgPool, caller: ProfileId) -> ApiResult<()> {
    let is_admin: bool = sqlx::query_scalar!(
        "SELECT (system_access = 'admin') AS \"is_admin!\" FROM kb_profiles WHERE id = $1",
        caller.uuid()
    )
    .fetch_optional(pool)
    .await?
    .unwrap_or(false);

    if is_admin {
        return Ok(());
    }
    Err(ApiError::NotFound("admin ledger".into()))
}

pub async fn list_by_subject(
    pool: &PgPool,
    caller: ProfileId,
    subject: RefTarget,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<AdminLedgerEntry>> {
    gate(pool, caller).await?;
    let probe = serde_json::json!([{ "target": subject }]);
    fetch(pool, Some(probe), None, limit, offset).await
}

pub async fn list_by_actor(
    pool: &PgPool,
    caller: ProfileId,
    actor: ProfileId,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<AdminLedgerEntry>> {
    gate(pool, caller).await?;
    fetch(pool, None, Some(actor.uuid()), limit, offset).await
}

/// The epoch: admin history begins here. NOT a backfill marker — everything before this is
/// genuinely unrecorded (spec §8), and the surface must say so rather than imply absence.
pub async fn ledger_epoch(pool: &PgPool) -> ApiResult<Option<DateTime<Utc>>> {
    Ok(sqlx::query_scalar!(
        "SELECT e.occurred_at FROM kb_events e
           JOIN kb_event_types t ON t.id = e.event_type_id
          WHERE t.name = 'admin_ledger_opened'
          ORDER BY e.occurred_at ASC LIMIT 1"
    )
    .fetch_optional(pool)
    .await?)
}

async fn fetch(
    pool: &PgPool,
    subject_probe: Option<serde_json::Value>,
    actor: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<AdminLedgerEntry>> {
    // Runtime query_as: the two axes select different predicates. Follows the search_service
    // precedent for dynamic predicates.
    let rows = sqlx::query_as::<_, (Uuid, String, Uuid, String, DateTime<Utc>, serde_json::Value, serde_json::Value, Option<Uuid>)>(
        r#"SELECT e.id, t.name, p.id, p.handle, e.occurred_at, e.payload, e."references", e.correlation_id
             FROM kb_events e
             JOIN kb_event_types t ON t.id = e.event_type_id
             JOIN kb_entities   en ON en.id = e.emitter_entity_id
             JOIN kb_profiles    p ON p.id = en.profile_id
            WHERE t.name = ANY($1)
              AND ($2::jsonb IS NULL OR e."references" @> $2::jsonb)
              AND ($3::uuid  IS NULL OR p.id = $3::uuid)
            ORDER BY e.occurred_at DESC, e.id DESC
            LIMIT $4 OFFSET $5"#,
    )
    .bind(ADMIN_EVENT_TYPES)
    .bind(subject_probe)
    .bind(actor)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|(event_id, event_type, actor_profile_id, actor_handle, occurred_at, payload, refs, correlation_id)| {
            Ok(AdminLedgerEntry {
                event_id,
                event_type,
                actor_profile_id,
                actor_handle,
                occurred_at,
                payload,
                references: serde_json::from_value(refs)
                    .map_err(|e| ApiError::Internal(format!("malformed references on {event_id}: {e}")))?,
                correlation_id,
            })
        })
        .collect()
}
```

Register it in `crates/temper-services/src/services/mod.rs`:

```rust
pub mod admin_ledger_service;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run -p temper-services --features test-db --test admin_ledger_test`
Expected: PASS (3 tests).

If `the_admin_event_is_invisible_to_cognition` fails, **stop** — the firewall is the design's load-bearing claim. Do not adjust the test to pass; report it.

- [ ] **Step 6: Prove the GIN index is used, not a seq scan**

The whole read path rests on `idx_kb_events_references`. A containment query that seq-scans 13k rows is a design failure hiding as a passing test.

Run against the dev DB:

```bash
psql "$DATABASE_URL" -c "EXPLAIN SELECT id FROM kb_events WHERE \"references\" @> '[{\"target\":{\"kind\":\"kb_contexts\",\"id\":\"019f6055-6aea-7aa2-a133-61552dd3d7e4\"}}]'::jsonb;"
```

Expected: a `Bitmap Index Scan on idx_kb_events_references`. If it shows `Seq Scan`, the probe shape does not match `jsonb_path_ops` containment — fix the probe, not the index.

- [ ] **Step 7: Regenerate the sqlx cache and commit**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make check
git add crates/temper-services/src/services/admin_ledger_service.rs \
        crates/temper-services/src/services/mod.rs \
        crates/temper-services/tests/admin_ledger_test.rs \
        crates/temper-services/tests/common/mod.rs \
        .sqlx crates/temper-services/.sqlx
git commit -m "feat(admin-ledger): read surface on kb_events.references

Ships before any writer, deliberately. The NULL anchor that firewalls admin
events from cognition also hides them from every reader, so the read path had
to be designed first or the writers would target a query shape nobody proved.

Two axes, both index-backed: by subject via references @> (GIN), by actor via
(emitter_entity_id, occurred_at DESC). Filters by admin event type rather than
by anchor nullity — lens_created already lives in the NULL bucket."
```

---

### Task 3: The `element_trail` payload-key invariant

`element_trail_node`/`element_trail_edge` (`migrations/20260706000002_element_trail_payload_actor.sql:7-52`) have **no event-type filter**. They match purely on payload key shape and are gated only by `resources_visible_to(p_profile)` (`:47-49`). An admin payload spelling `resource_id` — natural, since a grant with `subject_table='kb_resources'` *is about* a resource — would surface **who was granted access to it** to any reader of that resource.

This lands **before any admin payload exists**, so the invariant is never retrofitted.

**Files:**
- Test: `crates/temper-services/tests/admin_ledger_test.rs` (append)

**Interfaces:**
- Consumes: Task 2's fixture and `seed_admin_event`.

- [ ] **Step 1: Write the failing test**

```rust
/// The banned keys. element_trail_* match on payload KEY SHAPE with no type filter, so an admin
/// payload using any of these leaks an authority record into a cognition read gated only by
/// resources_visible_to. Spec 2026-07-16 §5 makes this a tested invariant, not a convention.
const BANNED_ADMIN_PAYLOAD_KEYS: &[&str] = &["resource_id", "block_id", "edge_id", "owner"];

#[sqlx::test]
async fn no_admin_payload_spells_a_trail_matched_key(pool: PgPool) {
    let bad: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT t.name, k.key
             FROM kb_events e
             JOIN kb_event_types t ON t.id = e.event_type_id
             CROSS JOIN LATERAL jsonb_object_keys(e.payload) AS k(key)
            WHERE t.name = ANY($1) AND k.key = ANY($2)"#,
    )
    .bind(ADMIN_EVENT_TYPES_FOR_TEST)
    .bind(BANNED_ADMIN_PAYLOAD_KEYS)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(
        bad.is_empty(),
        "admin payloads must not spell element_trail-matched keys — these leak authority \
         records to any reader of the resource. Use subject_table/subject_id + references. \
         Offenders: {bad:?}"
    );
}

#[sqlx::test]
async fn an_admin_event_never_appears_in_an_element_trail(pool: PgPool) {
    let f = common::admin_fixture(&pool).await;
    seed_admin_event(&pool, f.admin_emitter, f.context_id, f.team_id).await;

    // element_trail_node over every resource the admin can see must return no admin event.
    let leaked: i64 = sqlx::query_scalar(
        r#"SELECT count(*)
             FROM kb_resources r
             CROSS JOIN LATERAL element_trail_node($1, r.id) AS tr
             JOIN kb_event_types t ON t.name = tr.event_type
            WHERE t.name = ANY($2)"#,
    )
    .bind(f.admin_profile.uuid())
    .bind(ADMIN_EVENT_TYPES_FOR_TEST)
    .fetch_one(&pool)
    .await
    .unwrap_or(0);

    assert_eq!(leaked, 0, "no admin event may surface in a cognition element trail");
}
```

Add near the top of the test file:

```rust
const ADMIN_EVENT_TYPES_FOR_TEST: &[&str] = &["admin_ledger_opened", "grant_created", "grant_revoked"];
```

- [ ] **Step 2: Run tests**

Run: `cargo nextest run -p temper-services --features test-db --test admin_ledger_test -E 'test(payload) or test(element_trail)'`
Expected: PASS immediately — Task 2's `seed_admin_event` already uses `subject_table`/`subject_id`. **A test that passes on first write is correct here**: it is a regression guard against Task 8 and the follow-on plan, not a red-green cycle. If `an_admin_event_never_appears_in_an_element_trail` fails, `element_trail_node`'s signature differs from the assumption — read `migrations/20260706000002_element_trail_payload_actor.sql:7-52` and fix the call, not the assertion.

- [ ] **Step 3: Commit**

```bash
cargo make check
git add crates/temper-services/tests/admin_ledger_test.rs
git commit -m "test(admin-ledger): element_trail payload-key invariant

element_trail_node/_edge match on payload key shape with NO type filter and are
gated only by resources_visible_to. An admin payload spelling resource_id would
surface who was granted access to a resource, to anyone who can read it.

Lands before any admin payload exists so the invariant is never retrofitted."
```

---

### Task 4: Event types + payload schemas + the epoch marker

**Files:**
- Create: `migrations/20260716000010_admin_event_types.sql`
- Create: `migrations/20260716000020_admin_ledger_epoch.sql`

**Interfaces:**
- Produces: `kb_event_types` rows `admin_ledger_opened`, and payload schemas on the pre-existing `grant_created`/`grant_revoked` rows. One `admin_ledger_opened` event.
- Consumes: nothing.

`grant_created`/`grant_revoked` **already exist** in `kb_event_types` (seeded 2026-06-24, `migrations/20260624000003_canonical_seed.sql:51-52`) with NULL `payload_schema` and zero events. They are not dropped — they are this task's types. Their schemas get filled.

- [ ] **Step 1: Write the event-types migration**

Create `migrations/20260716000010_admin_event_types.sql`:

```sql
-- Admin-ledger event types (spec 2026-07-16 §9 step 3).
--
-- `grant_created`/`grant_revoked` were seeded 2026-06-24 and have carried NULL payload_schema and
-- zero events ever since. They are NOT orphans to be dropped: they are this arc's types, and this
-- migration gives them the schemas they never got. NULL payload_schema is legitimate per
-- 20260624000001_canonical_schema.sql:445-447 ("NULL = unregistered/permissive") and 31 of 33 types
-- have one -- so their emptiness never evidenced anything.
--
-- `admin_ledger_opened` is the epoch marker's type. Administration has ~14 acts in prod history and
-- ZERO events; those acts are not reconstructable (kb_teams has no creator column, and the grant
-- upsert overwrites granted_by_profile_id + granted_at). So the ledger declares where it begins
-- rather than synthesizing a past it cannot know.

INSERT INTO kb_event_types (name, payload_schema, schema_version)
SELECT 'admin_ledger_opened', $js${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "AdminLedgerOpened",
  "type": "object",
  "additionalProperties": false,
  "required": ["opened_at", "note"],
  "properties": {
    "opened_at": {"type": "string", "format": "date-time"},
    "note": {"type": "string"}
  }
}$js$::jsonb, 1
WHERE NOT EXISTS (SELECT 1 FROM kb_event_types WHERE name = 'admin_ledger_opened');

-- Fill the two pre-existing types' schemas. Note the payload deliberately spells the subject
-- `subject_table`/`subject_id` and NOT `resource_id`/`owner` -- element_trail_node/_edge match on
-- payload key shape with no type filter, so those keys would leak the grant into any reader's
-- element trail (spec §5, and there is a test).
UPDATE kb_event_types SET payload_schema = $js${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "GrantCreated",
  "type": "object",
  "additionalProperties": false,
  "required": ["subject_table","subject_id","principal_table","principal_id",
               "can_read","can_write","can_delete","can_grant","granted_by"],
  "properties": {
    "subject_table":   {"enum": ["kb_resources","kb_contexts","kb_cogmaps","kb_connections"]},
    "subject_id":      {"type": "string", "format": "uuid"},
    "principal_table": {"enum": ["kb_teams","kb_profiles"]},
    "principal_id":    {"type": "string", "format": "uuid"},
    "can_read":        {"type": "boolean"},
    "can_write":       {"type": "boolean"},
    "can_delete":      {"type": "boolean"},
    "can_grant":       {"type": "boolean"},
    "granted_by":      {"type": "string", "format": "uuid"},
    "previous":        {
      "type": "object",
      "additionalProperties": false,
      "description": "Capabilities before this act, when it replaced an existing grant. Absent on a fresh grant. An upsert that CHANGES capabilities is a real admin act and must not be silently dropped.",
      "required": ["can_read","can_write","can_delete","can_grant"],
      "properties": {
        "can_read":   {"type": "boolean"},
        "can_write":  {"type": "boolean"},
        "can_delete": {"type": "boolean"},
        "can_grant":  {"type": "boolean"}
      }
    }
  }
}$js$::jsonb
WHERE name = 'grant_created';

UPDATE kb_event_types SET payload_schema = $js${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "GrantRevoked",
  "type": "object",
  "additionalProperties": false,
  "required": ["subject_table","subject_id","principal_table","principal_id","revoked_by"],
  "properties": {
    "subject_table":   {"enum": ["kb_resources","kb_contexts","kb_cogmaps","kb_connections"]},
    "subject_id":      {"type": "string", "format": "uuid"},
    "principal_table": {"enum": ["kb_teams","kb_profiles"]},
    "principal_id":    {"type": "string", "format": "uuid"},
    "revoked_by":      {"type": "string", "format": "uuid"}
  }
}$js$::jsonb
WHERE name = 'grant_revoked';
```

- [ ] **Step 2: Write the epoch migration**

Create `migrations/20260716000020_admin_ledger_epoch.sql`:

```sql
-- The admin ledger's epoch (spec 2026-07-16 §8).
--
-- NOT a backfill. ~14 admin acts occurred in prod before any writer existed and 8 of them are
-- permanently unreconstructable: kb_teams has no creator column at all, kb_team_members has no
-- actor, and revoked grants were hard-DELETEd. The 6 that "survive" don't either -- the grant
-- upsert overwrites granted_by_profile_id AND sets granted_at = now(), so those columns are a
-- current snapshot, not history. Synthesizing events from them would mint immortal, append-only
-- rows asserting the wrong actor at a fabricated time.
--
-- A partially-backfilled ledger is WORSE than an honestly-empty one: a reader cannot distinguish
-- "no event" from "predates the writer" from "reconstruction with the wrong actor". An empty
-- ledger with an epoch is unambiguous.
--
-- Emitted by the system actor -- the bare `system` entity, which never resolves through
-- resolve_emitter (20260624000003_canonical_seed.sql). Both-NULL producing anchor: the epoch has
-- no cognition home, and neither will any admin event after it.

INSERT INTO kb_events (event_type_id, emitter_entity_id, payload, "references")
SELECT et.id,
       e.id,
       jsonb_build_object(
         'opened_at', to_jsonb(now()),
         'note', 'Admin ledger opens here. No administrative history exists before this event: '
              || 'the acts happened, but no writer recorded them and their actors are not '
              || 'reconstructable from surviving columns.'
       ),
       '[]'::jsonb
  FROM kb_event_types et
  CROSS JOIN kb_entities e
 WHERE et.name = 'admin_ledger_opened'
   AND e.name  = 'system'
   AND NOT EXISTS (
         SELECT 1 FROM kb_events x JOIN kb_event_types xt ON xt.id = x.event_type_id
          WHERE xt.name = 'admin_ledger_opened'
       );
```

- [ ] **Step 3: Apply and verify locally**

```bash
cargo make docker-up
cargo sqlx migrate run
psql "$DATABASE_URL" -c "SELECT t.name, e.payload->>'opened_at' AS opened_at, e.producing_anchor_table FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id WHERE t.name='admin_ledger_opened';"
```

Expected: exactly one row; `producing_anchor_table` is **NULL**.

- [ ] **Step 4: Verify idempotency**

```bash
psql "$DATABASE_URL" -f migrations/20260716000020_admin_ledger_epoch.sql
psql "$DATABASE_URL" -c "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id WHERE t.name='admin_ledger_opened';"
```

Expected: `1`. A second run inserts nothing.

- [ ] **Step 5: Assert the epoch reads back through the service**

Append to `crates/temper-services/tests/admin_ledger_test.rs`:

```rust
#[sqlx::test]
async fn the_epoch_is_readable_and_null_anchored(pool: PgPool) {
    let epoch = temper_services::services::admin_ledger_service::ledger_epoch(&pool)
        .await
        .expect("ledger_epoch");
    assert!(epoch.is_some(), "the epoch marker must exist after migration");

    let anchored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id
          WHERE t.name='admin_ledger_opened' AND e.producing_anchor_table IS NOT NULL",
    )
    .fetch_one(&pool).await.unwrap();
    assert_eq!(anchored, 0, "the epoch must be NULL-anchored — it has no cognition home");
}
```

Run: `cargo nextest run -p temper-services --features test-db --test admin_ledger_test -E 'test(epoch)'`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo make check
git add migrations/20260716000010_admin_event_types.sql \
        migrations/20260716000020_admin_ledger_epoch.sql \
        crates/temper-services/tests/admin_ledger_test.rs
git commit -m "feat(admin-ledger): event types, payload schemas, and the epoch marker

The epoch is NOT a backfill. 8 of ~14 historical admin acts are permanently
unreconstructable (kb_teams has no creator column; revoked grants were
hard-DELETEd) and the 6 that appear to survive don't -- the grant upsert
overwrites granted_by_profile_id and sets granted_at = now(). Synthesizing from
them would mint immortal append-only rows asserting the wrong actor.

grant_created/grant_revoked are not dropped: they are this arc's types and
finally get the payload schemas they never had."
```

---

### Task 5: The grant chokepoint — SQL fns, projectors, replay ownership

The proving pair. It catches the generic grant path **and** `connection_service::grant_reach`'s bypass (`connection_service.rs:467`, `:486`), which calls `insert_grant` directly — a service-layer sink would miss it. It also exercises replay ownership end-to-end.

**Files:**
- Create: `migrations/20260716000030_admin_grant_fns.sql`
- Modify: `crates/temper-substrate/src/events.rs` (EventKind + SeedAction arms)
- Modify: `crates/temper-substrate/src/replay.rs:88` (INPUT_TABLES)
- Modify: `crates/temper-services/src/services/access_service.rs:128,159`
- Test: `crates/temper-services/tests/admin_ledger_test.rs`

**Interfaces:**
- Consumes: `EventRef`/`RefTarget` (Task 1), the event types (Task 4).
- Produces: `EventKind::{AdminLedgerOpened, GrantCreated, GrantRevoked}`; SQL fns `_admin_grant_created`, `_admin_grant_revoked`; `insert_grant(conn, p: &InsertGrantParams, emitter: EntityId, ctx: EventContext) -> ApiResult<bool>`; `delete_grant(conn, subject_table, subject_id, principal_table, principal_id, revoker: ProfileId, emitter: EntityId, ctx: EventContext) -> ApiResult<bool>`.

> **Signature warning:** `delete_grant` gains three parameters. `CREATE OR REPLACE FUNCTION` cannot add a param, and a mutation-fn signature change is a **write outage across deploy skew** on an auto-deploying `main`. The SQL fns here are **new**, so this is safe — but get them right the first time; widening them later is the expensive case.

- [ ] **Step 1: Write the failing test**

```rust
#[sqlx::test]
async fn granting_writes_an_event_and_the_row(pool: PgPool) {
    let f = common::admin_fixture(&pool).await;

    let outcome = temper_services::services::access_service::grant_capability(
        &pool,
        f.admin_profile,
        common::grant_req(f.context_id, f.team_id),
    )
    .await
    .expect("grant_capability");
    assert!(outcome.granted, "a fresh grant reports granted");

    let entries = temper_services::services::admin_ledger_service::list_by_subject(
        &pool, f.admin_profile,
        RefTarget { kind: AnchorTable::Contexts, id: f.context_id }, 50, 0,
    ).await.unwrap();

    assert_eq!(entries.len(), 1, "the grant must be on the ledger");
    assert_eq!(entries[0].event_type, "grant_created");
    assert_eq!(entries[0].actor_profile_id, f.admin_profile.uuid());
    assert_eq!(entries[0].payload["subject_table"], "kb_contexts");
    assert!(entries[0].payload.get("resource_id").is_none(), "banned key");
}

#[sqlx::test]
async fn revoking_writes_an_event_even_though_the_row_is_deleted(pool: PgPool) {
    let f = common::admin_fixture(&pool).await;
    temper_services::services::access_service::grant_capability(
        &pool, f.admin_profile, common::grant_req(f.context_id, f.team_id)).await.unwrap();
    temper_services::services::access_service::revoke_capability(
        &pool, f.admin_profile, common::revoke_req(f.context_id, f.team_id)).await.unwrap();

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_access_grants WHERE subject_id=$1")
        .bind(f.context_id).fetch_one(&pool).await.unwrap();
    assert_eq!(rows, 0, "revoke still hard-DELETEs the row — the row is the projection");

    let entries = temper_services::services::admin_ledger_service::list_by_subject(
        &pool, f.admin_profile,
        RefTarget { kind: AnchorTable::Contexts, id: f.context_id }, 50, 0,
    ).await.unwrap();

    assert_eq!(entries.len(), 2, "the ledger keeps BOTH acts — this is the whole point");
    assert_eq!(entries[0].event_type, "grant_revoked", "newest first");
    assert_eq!(entries[1].event_type, "grant_created");
}

#[sqlx::test]
async fn the_connection_grant_reach_bypass_is_also_on_the_ledger(pool: PgPool) {
    // connection_service::grant_reach calls access_service::insert_grant DIRECTLY, bypassing
    // grant_capability (connection_service.rs:467). A service-layer sink would miss it; the
    // chokepoint must not.
    let f = common::connection_fixture(&pool).await;

    temper_services::services::connection_service::grant_reach(
        &pool, f.admin_profile, f.connection_id, f.team_id, None,
    ).await.expect("grant_reach");

    let entries = temper_services::services::admin_ledger_service::list_by_subject(
        &pool, f.admin_profile,
        RefTarget { kind: AnchorTable::Connections, id: f.connection_id }, 50, 0,
    ).await.unwrap();

    assert_eq!(entries.len(), 1, "grant_reach's bypass must still reach the ledger");
    assert_eq!(entries[0].payload["subject_table"], "kb_connections");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-services --features test-db --test admin_ledger_test -E 'test(granting) or test(revoking) or test(bypass)'`
Expected: FAIL — no events written.

- [ ] **Step 3: Write the SQL functions**

Create `migrations/20260716000030_admin_grant_fns.sql`:

```sql
-- The grant chokepoint, SQL-resident (spec 2026-07-16 §7).
--
-- WHY SQL AND NOT RUST: cognition events are not fired from Rust alongside a Rust write --
-- fire() dispatches a SeedAction to a SQL function that appends the event AND projects, in one
-- txn (_event_append, canonical_functions.sql:765). Admin acts follow the same shape. A
-- Rust service-layer sink would also MISS connection_service::grant_reach, which bypasses
-- grant_capability and calls insert_grant directly (connection_service.rs:467).
--
-- BOTH-NULL PRODUCING ANCHOR, always. A grant is an authority act; it has no cognition home even
-- when its subject IS a context. Anchoring it would put it in front of every region producer and
-- break the "governance is traceable, but it isn't knowledge" boundary.
--
-- The payload spells the subject `subject_table`/`subject_id`, NEVER `resource_id`/`owner`:
-- element_trail_node/_edge match on payload key shape with no type filter and are gated only by
-- resources_visible_to, so those keys would leak the grant to any reader of the resource.

CREATE FUNCTION _admin_grant_created(
    p_emitter         uuid,
    p_subject_table   text,
    p_subject_id      uuid,
    p_principal_table text,
    p_principal_id    uuid,
    p_can_read        boolean,
    p_can_write       boolean,
    p_can_delete      boolean,
    p_can_grant       boolean,
    p_granted_by      uuid,
    p_correlation     uuid DEFAULT NULL
) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE
    v_prev     jsonb := NULL;
    v_inserted boolean;
    v_payload  jsonb;
BEGIN
    -- Capture the prior capabilities BEFORE the upsert overwrites them. An upsert that changes
    -- capabilities returns inserted = false, so keying emission on that bool alone would silently
    -- drop a real authority change. The event carries before/after instead.
    SELECT jsonb_build_object('can_read', can_read, 'can_write', can_write,
                              'can_delete', can_delete, 'can_grant', can_grant)
      INTO v_prev
      FROM kb_access_grants
     WHERE subject_table = p_subject_table AND subject_id = p_subject_id
       AND principal_table = p_principal_table AND principal_id = p_principal_id;

    INSERT INTO kb_access_grants
        (subject_table, subject_id, principal_table, principal_id,
         can_read, can_write, can_delete, can_grant, granted_by_profile_id)
    VALUES (p_subject_table, p_subject_id, p_principal_table, p_principal_id,
            p_can_read, p_can_write, p_can_delete, p_can_grant, p_granted_by)
    ON CONFLICT (subject_table, subject_id, principal_table, principal_id)
    DO UPDATE SET can_read = EXCLUDED.can_read, can_write = EXCLUDED.can_write,
                  can_delete = EXCLUDED.can_delete, can_grant = EXCLUDED.can_grant,
                  granted_by_profile_id = EXCLUDED.granted_by_profile_id, granted_at = now()
    RETURNING (xmax = 0) INTO v_inserted;

    v_payload := jsonb_build_object(
        'subject_table', p_subject_table, 'subject_id', p_subject_id,
        'principal_table', p_principal_table, 'principal_id', p_principal_id,
        'can_read', p_can_read, 'can_write', p_can_write,
        'can_delete', p_can_delete, 'can_grant', p_can_grant,
        'granted_by', p_granted_by);
    IF v_prev IS NOT NULL THEN
        v_payload := v_payload || jsonb_build_object('previous', v_prev);
    END IF;

    PERFORM _event_append(
        'grant_created', p_emitter, NULL, NULL, v_payload,
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',  'target', jsonb_build_object('kind', p_subject_table,   'id', p_subject_id)),
            jsonb_build_object('rel','principal','target', jsonb_build_object('kind', p_principal_table, 'id', p_principal_id))),
        p_correlation => p_correlation);

    RETURN v_inserted;
END;
$$;

CREATE FUNCTION _admin_grant_revoked(
    p_emitter         uuid,
    p_subject_table   text,
    p_subject_id      uuid,
    p_principal_table text,
    p_principal_id    uuid,
    p_revoked_by      uuid,
    p_correlation     uuid DEFAULT NULL
) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE
    v_deleted boolean := false;
BEGIN
    DELETE FROM kb_access_grants
     WHERE subject_table = p_subject_table AND subject_id = p_subject_id
       AND principal_table = p_principal_table AND principal_id = p_principal_id;
    GET DIAGNOSTICS v_deleted = ROW_COUNT;
    v_deleted := (v_deleted::int > 0);

    -- Emit only when something was actually revoked: a no-op revoke is not an admin act, and the
    -- ledger is append-only -- a spurious row can never be corrected, only quarantined.
    IF v_deleted THEN
        PERFORM _event_append(
            'grant_revoked', p_emitter, NULL, NULL,
            jsonb_build_object(
                'subject_table', p_subject_table, 'subject_id', p_subject_id,
                'principal_table', p_principal_table, 'principal_id', p_principal_id,
                'revoked_by', p_revoked_by),
            p_references => jsonb_build_array(
                jsonb_build_object('rel','subject',  'target', jsonb_build_object('kind', p_subject_table,   'id', p_subject_id)),
                jsonb_build_object('rel','principal','target', jsonb_build_object('kind', p_principal_table, 'id', p_principal_id))),
            p_correlation => p_correlation);
    END IF;

    RETURN v_deleted;
END;
$$;

COMMENT ON FUNCTION _admin_grant_created IS
  'Grant upsert + grant_created event, one txn. Both-NULL producing anchor: a grant is an authority act with no cognition home, even when its subject is a context. Carries `previous` when it replaced an existing grant -- an upsert that changes capabilities returns inserted=false, so the bool alone would drop a real authority change.';

COMMENT ON FUNCTION _admin_grant_revoked IS
  'Grant DELETE + grant_revoked event, one txn. The DELETE stays: the row is the current-state projection, the ledger is the temporal record (access spec §3.7). Emits only when a row was actually deleted -- kb_events is append-only and a spurious event is immortal.';
```

- [ ] **Step 4: Add the EventKind variants and projectors**

In `crates/temper-substrate/src/events.rs`, add to `EventKind`, `as_canonical_name`, and `from_canonical_name` (all three — `from_canonical_name` is documented as the exact inverse):

```rust
    AdminLedgerOpened,
    GrantCreated,
    GrantRevoked,
```

```rust
            EventKind::AdminLedgerOpened => "admin_ledger_opened",
            EventKind::GrantCreated => "grant_created",
            EventKind::GrantRevoked => "grant_revoked",
```

```rust
            "admin_ledger_opened" => EventKind::AdminLedgerOpened,
            "grant_created" => EventKind::GrantCreated,
            "grant_revoked" => EventKind::GrantRevoked,
```

**This is mandatory, not optional.** `replay::replay` (`replay.rs:332-345`) walks every `kb_events` row and does `EventKind::from_canonical_name(&name)` with a `?` — an unknown type is a **hard replay failure**. The moment Task 4's epoch event exists, replay breaks without these variants.

- [ ] **Step 5: Give `kb_access_grants` to replay as an input table**

In `crates/temper-substrate/src/replay.rs`, add to `INPUT_TABLES` (the list at `:88`), after `"kb_team_contexts"`:

```rust
    "kb_access_grants",
```

`INPUT_TABLES` is copied verbatim into the replay namespace, and the projectors are **idempotent re-apply** — the shape `context_reassigned` already uses. Replay walks `ORDER BY e.id` (UUIDv7, time-sortable), so `grant_created` re-applies and a later `grant_revoked` deletes: net state correct.

**Not `PROJECTION_DUMPS`** — that would make replay rebuild grants from events and diff against live, and the 5 pre-epoch grants have no events, so it would report them as spurious forever.

- [ ] **Step 6: Wire the service layer to the SQL fns**

In `crates/temper-services/src/services/access_service.rs`, replace `insert_grant`'s body (`:128`) and `delete_grant`'s (`:159`) with calls to the SQL fns. `insert_grant` keeps its `ApiResult<bool>` contract, so `grant_capability`'s `GrantOutcome { granted }` is unchanged:

```rust
/// Raw grant upsert + `grant_created` event, one txn. **No authorization** — every caller must
/// gate first (unchanged from before; the event records what happened, and it only happens after
/// the caller's gate).
pub async fn insert_grant(
    conn: &mut sqlx::PgConnection,
    p: &InsertGrantParams,
    emitter: EntityId,
    correlation: Option<CorrelationId>,
) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT _admin_grant_created($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) AS "inserted!""#,
        emitter.uuid(),
        p.subject_table,
        p.subject_id,
        p.principal_table,
        p.principal_id,
        p.can_read,
        p.can_write,
        p.can_delete,
        p.can_grant,
        p.granted_by_profile_id,
        correlation.map(CorrelationId::uuid),
    )
    .fetch_one(&mut *conn)
    .await?)
}
```

`delete_grant` gains `revoker`, `emitter`, and `correlation` and returns whether a row was removed:

```rust
pub async fn delete_grant(
    conn: &mut sqlx::PgConnection,
    subject_table: &str,
    subject_id: Uuid,
    principal_table: &str,
    principal_id: Uuid,
    revoker: ProfileId,
    emitter: EntityId,
    correlation: Option<CorrelationId>,
) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT _admin_grant_revoked($1,$2,$3,$4,$5,$6,$7) AS "deleted!""#,
        emitter.uuid(),
        subject_table,
        subject_id,
        principal_table,
        principal_id,
        revoker.uuid(),
        correlation.map(CorrelationId::uuid),
    )
    .fetch_one(&mut *conn)
    .await?)
}
```

Update the four callers. `grant_capability`/`revoke_capability` (`:184`, `:213`) resolve an emitter from their `caller` via `temper_substrate::writes::resolve_emitter(pool, caller, "web")` — the shape `context_service::reassign` uses (`context_service.rs:549`) — and pass `None` for correlation.

`connection_service::grant_reach` (`:467`, `:486`) passes `Some(correlation)`: **it mints one `CorrelationId` and threads it to both** the affirmation and the grant, because the two are one act in one txn (`:449-450` — "never affirmation-without-grant or grant-without-affirmation"). `_event_append` defaults `p_correlation` to `COALESCE(p_correlation, v_ev)`, so each event would otherwise **self-root** and the fusion would be lost.

`connection_service::revoke_reach` (`:542`) passes the caller as `revoker`.

- [ ] **Step 7: Run the tests**

Run: `cargo nextest run -p temper-services --features test-db --test admin_ledger_test`
Expected: PASS (all 8 tests).

- [ ] **Step 8: Prove replay still works**

The single highest-risk regression: admin events now exist and `replay` walks every row.

Run: `cargo make test-artifacts`
Expected: PASS. If replay fails with `no projector for event type admin_ledger_opened`, Step 4 was skipped or incomplete.

- [ ] **Step 9: Full DB-backed suite**

Run: `cargo make test-db && cargo make test-e2e`
Expected: PASS. Trust the **exit code**, not any per-binary Summary line.

- [ ] **Step 10: Regenerate sqlx caches and commit**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-api
cargo make prepare-e2e
cargo make check
git add migrations/20260716000030_admin_grant_fns.sql \
        crates/temper-substrate/src/events.rs \
        crates/temper-substrate/src/replay.rs \
        crates/temper-services/src/services/access_service.rs \
        crates/temper-services/src/services/connection_service.rs \
        crates/temper-services/tests/admin_ledger_test.rs \
        .sqlx crates/temper-services/.sqlx crates/temper-api/.sqlx tests/e2e/.sqlx
git commit -m "feat(admin-ledger): grant chokepoint — the first event-sourced admin act

The proving pair. Installed at insert_grant/delete_grant rather than
grant_capability because connection_service::grant_reach bypasses the latter
and calls insert_grant directly — a service-layer sink would have missed it on
day one, which is precisely how five surfaces came to decline admin-as-events.

kb_access_grants joins INPUT_TABLES with idempotent re-apply projectors (not
PROJECTION_DUMPS: the 5 pre-epoch grants have no events and would diff as
spurious forever). The hard DELETE stays — the row is the projection, the
ledger is the temporal record, which is what access spec §3.7 said in June.

grant_created carries `previous` capabilities: an upsert that CHANGES
capabilities returns inserted=false, so the bool alone would drop a real
authority change."
```

---

### Task 6: API + CLI + MCP parity

Full surface parity is always intended. The read surface is useless if only Rust can reach it.

**Files:**
- Create: `crates/temper-api/src/handlers/admin_ledger.rs`
- Modify: `crates/temper-api/src/routes.rs`, `crates/temper-api/src/handlers/mod.rs`
- Create: `crates/temper-cli/src/commands/admin_ledger.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs`, the `admin` subcommand tree
- Create: `crates/temper-mcp/src/tools/admin_ledger.rs`
- Modify: `crates/temper-mcp/src/tools/mod.rs`
- Test: `tests/e2e/tests/admin_ledger_e2e.rs`

**Interfaces:**
- Consumes: `admin_ledger_service::{list_by_subject, list_by_actor, ledger_epoch, AdminLedgerEntry}` (Task 2).
- Produces: `GET /api/admin/ledger?subject_kind=&subject_id=&actor=&limit=&offset=`; `temper admin ledger --subject <kind>:<uuid> | --actor <ref>`; MCP tool `admin_ledger`.

- [ ] **Step 1: Write the failing e2e test**

Create `tests/e2e/tests/admin_ledger_e2e.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

/// At the production caller's level: a real server, a real grant over HTTP, read back over HTTP.
#[tokio::test]
async fn a_grant_made_over_http_is_readable_on_the_ledger_over_http() {
    let h = common::harness().await;
    let admin = h.admin_client().await;

    let ctx = admin.create_context("audit-me").await.expect("create context");
    admin.grant_context_to_team(ctx.id, h.team_id, "read").await.expect("grant");

    let ledger = admin
        .get_admin_ledger_by_subject("kb_contexts", ctx.id)
        .await
        .expect("read ledger");

    assert_eq!(ledger.entries.len(), 1, "the grant must be on the ledger");
    assert_eq!(ledger.entries[0].event_type, "grant_created");
    assert!(ledger.epoch.is_some(), "the response must carry the epoch");
}

#[tokio::test]
async fn a_non_admin_gets_404_from_the_ledger() {
    let h = common::harness().await;
    let outsider = h.outsider_client().await;
    let err = outsider
        .get_admin_ledger_by_subject("kb_contexts", h.some_context_id)
        .await
        .expect_err("outsider must not read the ledger");
    assert_eq!(err.status(), Some(404), "reads deny with 404, not 403");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo make test-e2e`
Expected: FAIL — no such route.

- [ ] **Step 3: Implement the handler**

`crates/temper-api/src/handlers/admin_ledger.rs` — transport only; all logic is in the service (read paths stay service-direct by design). The response DTO carries the epoch alongside the entries, so a reader **never** mistakes an empty list for "nothing happened":

```rust
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct AdminLedgerResponse {
    pub entries: Vec<AdminLedgerEntry>,
    /// When the admin ledger opened. Entries before this do not exist — the acts happened, but
    /// no writer recorded them (spec §8). An empty `entries` with an `epoch` means "nothing since
    /// T", never "nothing ever".
    pub epoch: Option<chrono::DateTime<chrono::Utc>>,
}
```

Register the route in `routes.rs` under `/api/admin/ledger` behind the existing auth middleware.

- [ ] **Step 4: Implement CLI + MCP**

CLI `temper admin ledger --subject kb_contexts:<uuid>` / `--actor <ref>`, routed through `temper-client` over HTTP (the CLI never calls services directly). MCP tool `admin_ledger` with the same two axes, delegating to `admin_ledger_service`.

- [ ] **Step 5: Regenerate the router's artifacts**

OpenAPI, the temper-rb gem, and temper-ts's `schema.ts` are **all products of the router**. A new response DTO stales all three:

```bash
cargo make openapi
git add openapi.json clients/temper-rb/lib/temper/generated clients/temper-ts/src/generated/schema.ts
```

The drift gates compare against **git**, not a fresh build — a correctly regenerated artifact still fails `cargo make check` while unstaged. Stage first, then check.

- [ ] **Step 6: Run everything**

```bash
cargo make check
cargo make test-e2e-embed
```

Expected: PASS. Use `test-e2e-embed`, not `test-e2e` — the latter silently compiles out every `test-embed`-gated test, and CI enables it.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/handlers/admin_ledger.rs crates/temper-api/src/handlers/mod.rs \
        crates/temper-api/src/routes.rs \
        crates/temper-cli/src/commands/admin_ledger.rs crates/temper-cli/src/commands/mod.rs \
        crates/temper-mcp/src/tools/admin_ledger.rs crates/temper-mcp/src/tools/mod.rs \
        tests/e2e/tests/admin_ledger_e2e.rs \
        openapi.json clients/temper-rb/lib/temper/generated clients/temper-ts/src/generated/schema.ts
git commit -m "feat(admin-ledger): API + CLI + MCP read parity

The response carries the epoch alongside the entries so an empty list reads as
'nothing since T', never 'nothing ever' — the distinction a partially-honest
audit log cannot make, and the reason the backfill was withdrawn."
```

---

### Task 7: Amend the published docs

The claim becomes true here, and **not before**. The docs are wrong in a way RETIRE would not have fixed either: they name the wrong *mechanism*.

**Files:**
- Modify: `docs/cognitive-maps/07-operating-temper.md:95-96`
- Modify: `docs/cognitive-maps/07b-governance-and-administration.md` (frontmatter, `:17-18`, `:58-59`, `:70`)
- Modify: `docs/superpowers/specs/2026-07-13-external-systems-as-subscribed-emitters-design.md:467`

- [ ] **Step 1: Fix the mechanism error**

`07-operating-temper.md:96` says admin acts are *"events, with an emitter and **a producing anchor**"*. The anchor is exactly what admin events must **not** have — and the same paragraph's next sentence promises they *"do not participate in cognitive maps"*, which the **NULL** anchor is what delivers. Anyone implementing from this line would have built the leak. Replace with wording that names the real mechanism: an emitter, and **no** producing anchor, which is what keeps them out of maps.

Apply the same fix at `07b:58-59`.

- [ ] **Step 2: Upgrade "firewalled by intent"**

`07b:70` reads *"The two live on the same ledger, firewalled by intent."* That was honest when written. It is now **firewalled by construction** — the NULL anchor is a structural property every region producer, `steward_ingest_delta`, and materialize attribution respect, and Task 2 has a test for it.

- [ ] **Step 3: Scope the "settled" claim honestly**

`07-operating-temper.md:95` and `07b:17-18` claim *every* administrative act is an event. After this plan, **one** is (the grant pair). Scope the claim to what ships, and say the rest is in flight. Do not restore the overclaim — that is what produced this task.

Note `07b`'s lead example is *"creating a team"* — which is **not** in this plan and whose history is unreconstructable (`kb_teams` has no creator column). Pick an example that is true.

- [ ] **Step 4: Fix the emitters spec's false premise**

`2026-07-13-external-systems-as-subscribed-emitters-design.md:467` says *"consistent with the existing admin-event-sourcing shape."* There was no existing shape. Point it at the 2026-07-16 spec.

The `07b` visualization placeholder — admin events flowing into *"a separate channel that does not feed the cognitive maps"* — **survives as-is**. NULL-anchoring implements it faithfully.

- [ ] **Step 5: Commit**

```bash
cargo make check
git add docs/cognitive-maps/07-operating-temper.md \
        docs/cognitive-maps/07b-governance-and-administration.md \
        docs/superpowers/specs/2026-07-13-external-systems-as-subscribed-emitters-design.md
git commit -m "docs: admin-as-events — name the real mechanism, scope the claim

The docs said admin acts carry 'an emitter and a producing anchor'. The anchor
is exactly what they must NOT have: the NULL anchor is what delivers the
'do not participate in cognitive maps' boundary the same paragraph promises.
A literal implementation of that line would have built the leak.

'Firewalled by intent' becomes 'by construction' — it is now a structural
property with a test. The 'settled' claim is scoped to the grant pair that
actually ships."
```

---

## Follow-on tasks (create in temper, do not build here)

- **The remaining authority tier** (§6): machine provision/rebind/revoke/rotate, connection provision/revoke/attach_credential/grant-reach/affirm, `change_role`, `promote_admin`, `update_system_settings`, cogmap bind/unbind, context share/unshare, join-request review. Its own plan, written against this one's proven pattern. **Thread the actor into `promote_admin` and `update_system_settings`** — they take no `caller` today, which is a plumbing gap, not an auth hole.
- **The principal-lifecycle tier** (§6): team create/delete/add_member/remove_member, invitations, SAML reconcile (whose actor is a system reconciler, not a profile — the actor model must handle it).
- **`kb_teams.created_by`** — additive column so future teams record a creator independent of the sink (§11).
- **Does a live profile-creation path still skip `provision_profile_entities`?** If invitation-accept or SAML creates profiles without it, `019f6b06`'s backfill is a treadmill.

---

## Self-Review

**Spec coverage:**

| Spec section | Task |
|---|---|
| §4 anchor rule (NULL for authority acts) | Task 5 SQL fns; Task 4 epoch; asserted in Tasks 2, 4 |
| §5 firewall is structural | Task 2 Step 5 (`the_admin_event_is_invisible_to_cognition`) |
| §5 read path on `references`, two axes | Tasks 1, 2; index proven in Task 2 Step 6 |
| §5 `element_trail` invariant + test | Task 3 |
| §5 read authorization | Task 2 `gate()` |
| §6 catalogue | Task 5 (grant pair only); rest is follow-on, per scope |
| §7 SQL-resident writers | Task 5 |
| §7 replay ownership (`INPUT_TABLES`) | Task 5 Step 5; verified Step 8 |
| §7 correlation threading | Task 5 Step 6 (`grant_reach`) |
| §7 `EventKind` + projectors or replay breaks | Task 5 Step 4; verified Step 8 |
| §7 emitter prerequisite | Extracted → `019f6b06`; noted as a dependency |
| §8 epoch, no backfill | Task 4 |
| §9 sequencing | Task order (read → invariant → epoch → writer → surfaces → docs) |
| §10 doc amendments | Task 7 |
| §11 open questions | Follow-on list |

**Placeholder scan:** one deliberate `todo!()` in Task 2 Step 2's fixture, explicitly flagged with what to read and why hand-INSERTing profiles would produce a lying fixture. Every other step carries real code or a real command.

**Type consistency:** `RefTarget` is used consistently in Tasks 1, 2, 5. `AdminLedgerEntry` fields match between Task 2's definition and Tasks 5/6's uses. `insert_grant`'s `ApiResult<bool>` contract is preserved so `GrantOutcome { granted }` is unchanged. `ADMIN_EVENT_TYPES` (service) and `ADMIN_EVENT_TYPES_FOR_TEST` (tests) are duplicated deliberately — the test must not import the constant it is guarding, or a wrong constant would pass its own test.
