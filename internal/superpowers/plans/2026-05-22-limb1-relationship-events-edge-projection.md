# Limb 1 — Relationship Events & Edge Projection — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `kb_resource_edges` a rebuildable projection of an append-only relationship-event stream — bringing knowledge-graph edges under the same event-ledger discipline limb 0 gave the ledger.

**Architecture:** Six `relationship_*` event types are appended to the `kb_events` ledger. A projection service applies each event's edge delta to `kb_resource_edges` *within the same transaction* as the append, so the default projection is never stale. A full-rebuild op replays the ledger. Edges gain a four-type structural `edge_kind` enum (SSTorytime ST-types) + `polarity` sign + mandatory free-text `label`. Explicit writes flow through new operations commands on `DbBackend`; the existing frontmatter edge-extraction path is rewired to emit events instead of upserting directly.

**Tech Stack:** Rust (temper-core, temper-events, temper-api, temper-cli, temper-mcp), PostgreSQL 18, sqlx compile-time macros, axum, rmcp (MCP), cargo-nextest.

**Spec:** `docs/superpowers/specs/2026-05-22-limb1-relationship-events-edge-projection-design.md`

---

## Critical execution notes

- **The schema cutover (Task 7) has no intermediate green state.** Renaming `kb_resource_edges` columns and dropping the `edge_type` enum breaks every `sqlx::query!` macro that references them the instant the migration runs against the dev DB. Tasks 1–6 are all *additive* and stay green individually. Task 7 bundles the breaking migration with every Rust repair it forces, and commits once. This mirrors limb 0's "all code first, then one consolidated verify+commit" (see `docs/superpowers/specs/2026-05-21-event-ledger-unification-design.md`).
- **`DATABASE_URL` must point at a migrated dev DB** for sqlx macros to compile. After Task 5's and Task 7's migrations, run `cargo sqlx prepare --workspace -- --all-features` and commit the `.sqlx/` changes (see `feedback_sql_query_patterns` / the CLAUDE.md SQL section). Per-crate prepare may be needed for feature-gated test queries — see `project_sqlx_per_crate_cache_for_feature_gated_tests`.
- **Run `cargo make fix && cargo make check` before every commit.** The pre-commit hook is a backstop, not the first line.
- **Test scope per task:** focused test + the touched crate's suite (`cargo nextest run -p <crate> --features test-db`). Full-workspace nextest belongs at PR-prep only.
- **`cargo clean -p temper-api` after Task 5 and Task 7** — `sqlx::migrate!()` caches stale migration state otherwise (see `project_sqlx_migrate_macro_stale_cache`).

---

## File structure

**temper-core** (`crates/temper-core/src/types/`)
- `graph.rs` — *modify*: replace `EdgeType` with `EdgeKind` + `Polarity`; add `edge_type_legacy_mapping`; update `ResolvedEdge`, the `Graph*Row` types, `GraphEdge`.
- `relationship_events.rs` — *create*: the six typed relationship-event payload structs.

**temper-events** (`crates/temper-events/src/`)
- `types/event.rs` — *modify*: add six `EventType` variants + canonical names; add `EventToWrite::new_correlated`.
- `ledger.rs` — *modify*: add `append_event_tx` (transaction-accepting); add match arms for the six variants.

**temper-core operations** (`crates/temper-core/src/operations/`)
- `commands.rs` — *modify*: add `AssertRelationship`, `RetypeRelationship`, `ReweightRelationship`, `FoldRelationship`.
- `events.rs` — *modify*: add `DbRelationshipAsserted` / `…Retyped` / `…Reweighted` / `…Folded` `DomainEvent` variants.
- `mod.rs` — *modify*: re-export the new commands.

**temper-api**
- `services/relationship_service.rs` — *create*: `apply_relationship_event`, `rebuild_edge_projection`, `reproject_pending_for_resource`, the four write functions.
- `services/edge_service.rs` — *modify*: rewire `upsert_edges`/`defer_edges`/`reconcile_edges`/`resolve_deferred_edges` to emit events; drop `kb_deferred_edges` usage.
- `services/graph_service.rs`, `services/ingest_service.rs`, `services/resource_service.rs`, `services/meta_service.rs` — *modify*: column/enum renames; rewired edge-extraction calls.
- `backend/db_backend.rs` — *modify*: add four inherent relationship-write methods.
- `handlers/edges.rs` — *modify*: add `assert` / `retype` / `reweight` / `fold` handlers.
- `routes.rs` — *modify*: register the four new routes.
- `migrations/` — *create*: two migration files (Task 5 additive, Task 7 breaking).

**temper-cli** (`crates/temper-cli/src/`)
- `commands/graph.rs` — *modify*: add `edge assert/retype/reweight/fold` subcommands.

**temper-mcp** (`crates/temper-mcp/src/tools/`)
- `relationships.rs` — *create*: four MCP tools.
- `mod.rs` — *modify*: register the module.

---

## Task 1: `EdgeKind` and `Polarity` enums in temper-core

**Files:**
- Modify: `crates/temper-core/src/types/graph.rs`

The structural typing layer. `EdgeKind` carries traversal algebra; `Polarity` carries direction. `EdgeType` (the legacy flat enum) stays *for now* — Task 7 removes it. A `legacy_mapping` helper encodes the 8→4 table the migration and frontmatter rewire both use.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/temper-core/src/types/graph.rs`:

```rust
    // ── EdgeKind / Polarity ─────────────────────────────────────────────

    #[test]
    fn edge_kind_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&EdgeKind::LeadsTo).unwrap(), "\"leads_to\"");
        assert_eq!(serde_json::to_string(&EdgeKind::Near).unwrap(), "\"near\"");
    }

    #[test]
    fn polarity_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&Polarity::Inverse).unwrap(), "\"inverse\"");
    }

    #[test]
    fn legacy_mapping_covers_all_seven_edge_types() {
        // Every legacy EdgeType maps to a (kind, polarity, label).
        for et in [
            EdgeType::RelatesTo, EdgeType::Extends, EdgeType::DependsOn,
            EdgeType::References, EdgeType::ParentOf, EdgeType::PrecededBy,
            EdgeType::DerivedFrom,
        ] {
            let (kind, polarity, label) = et.legacy_mapping();
            assert!(!label.is_empty());
            // depends_on / extends / preceded_by / derived_from are inverse leads_to
            if matches!(et, EdgeType::DependsOn | EdgeType::Extends
                          | EdgeType::PrecededBy | EdgeType::DerivedFrom) {
                assert_eq!(kind, EdgeKind::LeadsTo);
                assert_eq!(polarity, Polarity::Inverse);
            }
        }
        assert_eq!(EdgeType::ParentOf.legacy_mapping().0, EdgeKind::Contains);
        assert_eq!(EdgeType::RelatesTo.legacy_mapping().0, EdgeKind::Near);
        assert_eq!(EdgeType::References.legacy_mapping().0, EdgeKind::Near);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-core edge_kind_serializes_snake_case legacy_mapping_covers_all_seven_edge_types polarity_serializes`
Expected: FAIL — `cannot find type EdgeKind` / `Polarity` / `no method legacy_mapping`.

- [ ] **Step 3: Add the enums and mapping**

In `crates/temper-core/src/types/graph.rs`, after the `EdgeType` `Display` impl, add:

```rust
// ─── Structural Edge Typing (SSTorytime four-type taxonomy) ─────────────────

/// Structural edge type — the four Semantic-Spacetime primitives. Each kind
/// carries a distinct traversal algebra:
/// - `Contains`  — transitive (composition / part-of participation)
/// - `LeadsTo`   — antisymmetric, causal/temporal order
/// - `Near`      — symmetric (proximity / similarity)
/// - `Express`   — leaf attribute (has-property)
///
/// Mirrors the Postgres `edge_kind` enum.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "edge_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Express,
    Contains,
    LeadsTo,
    Near,
}

/// Edge direction sign. `source → target` as asserted may run *with* the
/// structural arrow (`Forward`) or *against* it (`Inverse`) — e.g. a
/// `depends_on` edge is asserted source=dependant/target=dependency, but the
/// causal arrow runs dependency→dependant, so it is `Inverse` `LeadsTo`.
/// `Near` is symmetric: always `Forward`.
///
/// Mirrors the Postgres `edge_polarity` enum.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "edge_polarity", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Forward,
    Inverse,
}

impl EdgeType {
    /// Map a legacy flat `EdgeType` to its structural `(EdgeKind, Polarity,
    /// label)` triple. The label is the legacy enum's snake_case name —
    /// preserving the human relation vocabulary as free-text. Used by the
    /// schema-cutover migration and the frontmatter edge-extraction rewire.
    pub fn legacy_mapping(self) -> (EdgeKind, Polarity, &'static str) {
        match self {
            Self::ParentOf    => (EdgeKind::Contains, Polarity::Forward, "parent_of"),
            Self::DependsOn   => (EdgeKind::LeadsTo,  Polarity::Inverse, "depends_on"),
            Self::PrecededBy  => (EdgeKind::LeadsTo,  Polarity::Inverse, "preceded_by"),
            Self::DerivedFrom => (EdgeKind::LeadsTo,  Polarity::Inverse, "derived_from"),
            Self::Extends     => (EdgeKind::LeadsTo,  Polarity::Inverse, "extends"),
            Self::RelatesTo   => (EdgeKind::Near,     Polarity::Forward, "relates_to"),
            Self::References  => (EdgeKind::Near,     Polarity::Forward, "references"),
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-core edge_kind legacy_mapping polarity`
Expected: PASS (3 tests).

- [ ] **Step 5: Run the temper-core suite**

Run: `cargo nextest run -p temper-core`
Expected: PASS — no regressions.

- [ ] **Step 6: Commit**

```bash
cargo make fix && cargo make check
git add crates/temper-core/src/types/graph.rs
git commit -m "feat(temper-core): EdgeKind + Polarity structural enums with legacy mapping"
```

---

## Task 2: Relationship-event payload structs in temper-core

**Files:**
- Create: `crates/temper-core/src/types/relationship_events.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

Typed payloads for the six `relationship_*` event types. These are the *structured* shape the projection builder reads (it does not parse raw `serde_json::Value`). `decayed` / `corrected` schemas are defined now (phase-1 requirement) though their projection mechanics are phase 4.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-core/src/types/relationship_events.rs`:

```rust
//! Typed payloads for the `relationship_*` event family — the structured
//! shape the edge-projection builder reads out of `kb_events.payload`.
//!
//! `relationship_asserted` is the lifecycle root: its event id becomes the
//! `correlation_id` shared by every later event for that edge. The projection
//! builder keys on `correlation_id`, not on ledger `references`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::graph::{EdgeKind, Polarity};

/// The target endpoint of an asserted relationship — a resolved resource id,
/// or an unresolved slug (forward reference). A slug target projects no edge
/// until a resource with that slug exists; this replaces `kb_deferred_edges`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum TargetEndpoint {
    Resource(Uuid),
    Slug(String),
}

/// `relationship_asserted` — genesis of a relationship. Topic class: Declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipAsserted {
    pub source_resource_id: Uuid,
    pub target: TargetEndpoint,
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
    pub label: String,
    pub weight: f64,
}

/// `relationship_retyped` — change the structural kind / label. Declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipRetyped {
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
    pub label: String,
}

/// `relationship_reweighted` — change the weight. Declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipReweighted {
    pub weight: f64,
}

/// `relationship_folded` — edge preserved but removed from the default
/// projection. The retraction mechanism: "no longer current, but not wrong".
/// Topic class: Deformation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipFolded {
    /// Optional human note on why the edge was folded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `relationship_decayed` — schema only in phases 1-2; mechanics are phase 4.
/// Topic class: Deformation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipDecayed {
    /// Multiplicative decay factor applied to the edge weight (0.0..1.0).
    pub factor: f64,
}

/// `relationship_corrected` — the edge was *wrong*; carries a scar.
/// Schema only in phases 1-2; mechanics are phase 4. Topic class: Judgment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipCorrected {
    /// Structured account of the wrongness — the scar.
    pub scar: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asserted_round_trips_with_slug_target() {
        let p = RelationshipAsserted {
            source_resource_id: Uuid::nil(),
            target: TargetEndpoint::Slug("some-goal".into()),
            edge_kind: EdgeKind::Contains,
            polarity: Polarity::Forward,
            label: "parent_of".into(),
            weight: 1.0,
        };
        let v = serde_json::to_value(&p).unwrap();
        let back: RelationshipAsserted = serde_json::from_value(v).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn target_endpoint_resource_round_trips() {
        let t = TargetEndpoint::Resource(Uuid::nil());
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(serde_json::from_value::<TargetEndpoint>(v).unwrap(), t);
    }

    #[test]
    fn folded_reason_is_optional() {
        let v = serde_json::json!({});
        let p: RelationshipFolded = serde_json::from_value(v).unwrap();
        assert!(p.reason.is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-core relationship_events`
Expected: FAIL — module not declared.

- [ ] **Step 3: Declare the module**

In `crates/temper-core/src/types/mod.rs`, add `pub mod relationship_events;` in alphabetical order with the other `pub mod` lines.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-core relationship_events`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
cargo make fix && cargo make check
git add crates/temper-core/src/types/relationship_events.rs crates/temper-core/src/types/mod.rs
git commit -m "feat(temper-core): typed relationship-event payload structs"
```

---

## Task 3: Six `relationship_*` variants in the temper-events `EventType` enum

**Files:**
- Modify: `crates/temper-events/src/types/event.rs`
- Modify: `crates/temper-events/src/ledger.rs`

`EventType` is a closed enum with `as_canonical_name`. `append_event` has an *exhaustive* `match write.event_type` enforcing `Supersedes` invariants — adding variants will not compile until that match gains arms. Relationship events impose no `Supersedes` requirement.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/temper-events/src/types/event.rs` (create the module if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_event_canonical_names_are_snake_case() {
        assert_eq!(EventType::RelationshipAsserted.as_canonical_name(), "relationship_asserted");
        assert_eq!(EventType::RelationshipRetyped.as_canonical_name(), "relationship_retyped");
        assert_eq!(EventType::RelationshipReweighted.as_canonical_name(), "relationship_reweighted");
        assert_eq!(EventType::RelationshipFolded.as_canonical_name(), "relationship_folded");
        assert_eq!(EventType::RelationshipDecayed.as_canonical_name(), "relationship_decayed");
        assert_eq!(EventType::RelationshipCorrected.as_canonical_name(), "relationship_corrected");
    }

    #[test]
    fn new_correlated_keeps_supplied_correlation_id() {
        let corr = Uuid::now_v7();
        let w = EventToWrite::new_correlated(
            EventType::RelationshipRetyped,
            Uuid::nil(), Uuid::nil(), Uuid::nil(),
            serde_json::json!({}),
            corr,
            Utc::now(),
        );
        assert_eq!(w.correlation_id, corr);
        assert_ne!(w.id, corr);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-events relationship_event_canonical new_correlated`
Expected: FAIL — `no variant RelationshipAsserted` / `no function new_correlated`.

- [ ] **Step 3: Add the variants, canonical names, and constructor**

In `crates/temper-events/src/types/event.rs`, extend the `EventType` enum:

```rust
pub enum EventType {
    ConceptCreated,
    ConceptMutated,
    RelationshipAsserted,
    RelationshipRetyped,
    RelationshipReweighted,
    RelationshipFolded,
    RelationshipDecayed,
    RelationshipCorrected,
}
```

Extend `as_canonical_name`:

```rust
            EventType::ConceptCreated => "ConceptCreated",
            EventType::ConceptMutated => "ConceptMutated",
            EventType::RelationshipAsserted => "relationship_asserted",
            EventType::RelationshipRetyped => "relationship_retyped",
            EventType::RelationshipReweighted => "relationship_reweighted",
            EventType::RelationshipFolded => "relationship_folded",
            EventType::RelationshipDecayed => "relationship_decayed",
            EventType::RelationshipCorrected => "relationship_corrected",
```

Add a non-root constructor to `impl EventToWrite` (after `new_root`):

```rust
    /// Construct a non-root event that joins an existing lifecycle: `id` is
    /// fresh, `correlation_id` is the caller-supplied lifecycle root id.
    pub fn new_correlated(
        event_type: EventType,
        emitter_profile_id: Uuid,
        topic_id: Uuid,
        scope_id: Uuid,
        payload: serde_json::Value,
        correlation_id: Uuid,
        occurred_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            event_type,
            emitter_profile_id,
            topic_id,
            scope_id,
            payload,
            metadata: serde_json::json!({}),
            references: Vec::new(),
            correlation_id,
            occurred_at,
        }
    }
```

- [ ] **Step 4: Add match arms in `append_event`**

In `crates/temper-events/src/ledger.rs`, the `match write.event_type { ... }` block enforcing `Supersedes` invariants is now non-exhaustive. Add one arm covering all six relationship variants — they impose no reference invariant:

```rust
        EventType::RelationshipAsserted
        | EventType::RelationshipRetyped
        | EventType::RelationshipReweighted
        | EventType::RelationshipFolded
        | EventType::RelationshipDecayed
        | EventType::RelationshipCorrected => {
            // Relationship lifecycle events impose no Supersedes invariant;
            // intra-lifecycle linkage is carried by correlation_id.
        }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run -p temper-events`
Expected: PASS — new tests pass, no regressions.

- [ ] **Step 6: Commit**

```bash
cargo make fix && cargo make check
git add crates/temper-events/src/types/event.rs crates/temper-events/src/ledger.rs
git commit -m "feat(temper-events): six relationship_* EventType variants + new_correlated"
```

---

## Task 4: Transaction-accepting `append_event_tx`

**Files:**
- Modify: `crates/temper-events/src/ledger.rs`

`append_event` takes `&PgPool` and runs its own implicit transaction. The spec requires "append + project edge delta in **one** transaction", so the relationship service needs to append *inside* a transaction it also uses for the projection write. Extract the body to accept a transaction.

- [ ] **Step 1: Write the failing test**

This is a `test-db` integration test. Add to `crates/temper-events/tests/` — create `crates/temper-events/tests/append_tx_test.rs` (gate it; see `project_test_db_feature_gate_convention`):

```rust
#![cfg(feature = "test-db")]
//! `append_event_tx` appends within a caller-owned transaction.

use temper_events::{append_event_tx, EventToWrite, EventType};

#[sqlx::test(migrations = "../../migrations")]
async fn append_event_tx_commits_with_caller_transaction(pool: sqlx::PgPool) {
    // Seed a profile, topic, scope the FK checks require.
    // (Use the bootstrap topic/public scope seeded by the ledger-unification
    // migration: topic 019e3d6f-2300-7000-8000-000000000040,
    // scope 019e3d6f-2300-7000-8000-000000000010. Insert a profile.)
    let profile_id = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $3)")
        .bind(profile_id)
        .bind(format!("p{}", profile_id.simple()))
        .bind("Test")
        .execute(&pool)
        .await
        .expect("seed profile");

    let topic = uuid::Uuid::parse_str("019e3d6f-2300-7000-8000-000000000040").unwrap();
    let scope = uuid::Uuid::parse_str("019e3d6f-2300-7000-8000-000000000010").unwrap();

    let mut tx = pool.begin().await.unwrap();
    let write = EventToWrite::new_root(
        EventType::RelationshipAsserted,
        profile_id, topic, scope,
        serde_json::json!({"probe": true}),
        chrono::Utc::now(),
    );
    let event = append_event_tx(&mut tx, write).await.expect("append in tx");
    tx.commit().await.unwrap();

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events WHERE id = $1")
        .bind(event.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
```

> ⚠️ Plan/reality check: confirm `kb_profiles` column names (`handle`, `display_name`) against the live schema before relying on this seed — adjust the INSERT to the real columns. The bootstrap topic/scope UUIDs are from migration `20260522000001_event_ledger_unification.sql` and are verified.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-events --features test-db append_event_tx_commits`
Expected: FAIL — `cannot find function append_event_tx`.

- [ ] **Step 3: Refactor `append_event` to delegate to `append_event_tx`**

In `crates/temper-events/src/ledger.rs`, change the signature of the existing function body to take `&mut sqlx::PgConnection` (which both `&mut Transaction` and a pooled connection deref to). Rename the worker to `append_event_tx` accepting `&mut sqlx::PgConnection`; keep `append_event(&PgPool, ...)` as a thin wrapper that acquires a connection. Replace every `.fetch_*(pool)` inside with `.fetch_*(&mut **conn)` (transaction) — use a generic executor instead:

```rust
/// Append within a caller-owned transaction. Every validation query and the
/// final INSERT run on `conn`, so the caller controls commit/rollback — this
/// is what lets a surface append a ledger event and apply its projection
/// delta atomically.
pub async fn append_event_tx(
    conn: &mut sqlx::PgConnection,
    write: EventToWrite,
) -> Result<Event, LedgerError> {
    // ... existing body, with every `.fetch_*(pool)` → `.fetch_*(&mut *conn)` ...
}

/// Append using a pool — acquires a connection and delegates. Unchanged
/// behavior for existing callers.
pub async fn append_event(pool: &PgPool, write: EventToWrite) -> Result<Event, LedgerError> {
    let mut conn = pool.acquire().await?;
    append_event_tx(&mut conn, write).await
}
```

Export `append_event_tx` from `crates/temper-events/src/lib.rs` alongside `append_event`.

> ⚠️ Implementation note: `sqlx::query!` macros accept `impl Executor`. `&mut PgConnection` is an executor; `&mut *conn` where `conn: &mut PgConnection` re-borrows. For a `Transaction`, callers pass `&mut *tx` (which derefs to `PgConnection`). Verify each query compiles — if a query is run twice, the executor is consumed; re-borrow `&mut *conn` per call.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-events --features test-db`
Expected: PASS — new test passes; existing ledger tests still pass via the `append_event` wrapper.

- [ ] **Step 5: Regenerate the SQL cache**

Run: `cargo sqlx prepare -p temper-events -- --features test-db` (per-crate; the queries moved but did not change shape — cache should be stable, but regenerate to be safe).
Expected: no diff, or a benign diff. Commit any `.sqlx/` change.

- [ ] **Step 6: Commit**

```bash
cargo make fix && cargo make check
git add crates/temper-events/src/ledger.rs crates/temper-events/src/lib.rs crates/temper-events/tests/append_tx_test.rs .sqlx/
git commit -m "feat(temper-events): append_event_tx — transaction-accepting append"
```

---

## Task 5: Additive migration — `edge_kind`/`edge_polarity` enums, topics, event-type registry

**Files:**
- Create: `crates/temper-core/.../migrations/20260522100001_relationship_event_taxonomy.sql` → actually `migrations/20260522100001_relationship_event_taxonomy.sql`

This migration is purely *additive* — it creates two enums and seeds registry/topic rows. Nothing references them yet, so the workspace stays green.

- [ ] **Step 1: Write the migration**

Create `migrations/20260522100001_relationship_event_taxonomy.sql`:

```sql
-- Relationship-event taxonomy — phase 1 of limb 1 (edges as event projections).
-- Spec: docs/superpowers/specs/2026-05-22-limb1-relationship-events-edge-projection-design.md
-- This migration is ADDITIVE: new enums + registry/topic rows. The breaking
-- kb_resource_edges cutover is a separate later migration.

-- ─── Structural edge-typing enums ───────────────────────────────────────────
CREATE TYPE edge_kind     AS ENUM ('express', 'contains', 'leads_to', 'near');
CREATE TYPE edge_polarity AS ENUM ('forward', 'inverse');

-- ─── Topic rows for the three framing-schema classes ────────────────────────
-- Deterministic UUIDv7 ids so fixtures can reference them by constant.
INSERT INTO kb_topics (id, fqdn) VALUES
    ('019e3d6f-2300-7000-8000-000000000050', 'declaration'),
    ('019e3d6f-2300-7000-8000-000000000051', 'deformation'),
    ('019e3d6f-2300-7000-8000-000000000052', 'judgment')
ON CONFLICT (fqdn) DO NOTHING;

-- ─── Event-type registry rows ───────────────────────────────────────────────
INSERT INTO kb_event_types (name, description) VALUES
    ('relationship_asserted',   'A knowledge-graph relationship was asserted (genesis).'),
    ('relationship_retyped',    'A relationship''s structural kind or label changed.'),
    ('relationship_reweighted', 'A relationship''s weight changed.'),
    ('relationship_folded',     'A relationship was folded — preserved, off the default projection.'),
    ('relationship_decayed',    'A relationship decayed (phase-4 mechanics).'),
    ('relationship_corrected',  'A relationship was corrected as wrong — carries a scar (phase-4 mechanics).')
ON CONFLICT (name) DO NOTHING;
```

> ⚠️ Plan/reality check: confirm `kb_topics` has columns `(id, fqdn)` and `kb_event_types` has `(name, description)` — verified against `migrations/20260522000001_event_ledger_unification.sql`. The three topic UUIDs continue the `…00050+` block; confirm no collision with existing seed ids.

- [ ] **Step 2: Run the migration against the dev DB**

Run: `cargo make docker-up` then `sqlx migrate run --source migrations` (or `cargo make` equivalent — check `Makefile.toml` for the migrate task).
Expected: migration applies cleanly.

- [ ] **Step 3: Verify the enums and rows exist**

Run:
```bash
psql "$DATABASE_URL" -c "SELECT enumlabel FROM pg_enum JOIN pg_type t ON t.oid = enumtypid WHERE t.typname = 'edge_kind' ORDER BY 1;"
psql "$DATABASE_URL" -c "SELECT name FROM kb_event_types WHERE name LIKE 'relationship_%' ORDER BY 1;"
```
Expected: four `edge_kind` labels; six `relationship_*` rows.

- [ ] **Step 4: Rebuild and confirm the workspace stays green**

Run: `cargo clean -p temper-api && cargo make check`
Expected: PASS — additive migration breaks nothing.

- [ ] **Step 5: Commit**

```bash
git add migrations/20260522100001_relationship_event_taxonomy.sql
git commit -m "feat(db): additive migration — edge_kind/edge_polarity enums + relationship topics"
```

---

## Task 6: `relationship_service` — projection apply + write functions (against the NEW schema, not yet wired)

**Files:**
- Create: `crates/temper-api/src/services/relationship_service.rs`
- Modify: `crates/temper-api/src/services/mod.rs`

> **Sequencing note:** This service writes the *new* `kb_resource_edges` columns, which do not exist until Task 7's migration. To keep TDD honest, write this service file in this task with its **unit-testable pure helpers fully tested here**, but defer the `test-db` integration tests and the `mod.rs` wiring to *after* Task 7. Concretely: this task delivers the pure payload-building / mapping helpers and their unit tests; Task 7 adds the SQL-bearing functions and `test-db` tests once the columns exist. If the subagent finds it cleaner to merge this task into Task 7, that is acceptable — flag it in the task report.

- [ ] **Step 1: Write the failing test (pure helpers only)**

Create `crates/temper-api/src/services/relationship_service.rs`:

```rust
//! Relationship service — appends `relationship_*` events to the ledger and
//! projects their edge deltas into `kb_resource_edges` within one transaction.
//!
//! The ledger is truth; `kb_resource_edges` is a rebuildable projection.
//! `apply_relationship_event` does the incremental delta; `rebuild_edge_projection`
//! replays the whole stream. See the limb-1 design spec.

use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::relationship_events::{RelationshipAsserted, TargetEndpoint};

/// Topic UUIDs seeded by migration 20260522100001.
pub const TOPIC_DECLARATION: &str = "019e3d6f-2300-7000-8000-000000000050";
pub const TOPIC_DEFORMATION: &str = "019e3d6f-2300-7000-8000-000000000051";
pub const TOPIC_JUDGMENT: &str = "019e3d6f-2300-7000-8000-000000000052";

/// Validation: a `near` edge must carry a meaningful label — the mandatory-label
/// rule that stops `near` becoming the new vague catch-all. An empty or
/// whitespace-only label is rejected for every kind.
pub fn validate_assertion_label(kind: EdgeKind, label: &str) -> Result<(), String> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Err("relationship label must be non-empty".to_string());
    }
    let _ = kind; // kind-specific banned-generic-label checks may tighten later
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_label_is_rejected() {
        assert!(validate_assertion_label(EdgeKind::Near, "   ").is_err());
        assert!(validate_assertion_label(EdgeKind::Contains, "").is_err());
    }

    #[test]
    fn non_empty_label_is_accepted() {
        assert!(validate_assertion_label(EdgeKind::LeadsTo, "depends_on").is_ok());
    }
}
```

- [ ] **Step 2: Declare the module**

In `crates/temper-api/src/services/mod.rs`, add `pub mod relationship_service;` in alphabetical order.

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo nextest run -p temper-api validate_assertion_label empty_label non_empty_label`
Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
cargo make fix && cargo make check
git add crates/temper-api/src/services/relationship_service.rs crates/temper-api/src/services/mod.rs
git commit -m "feat(temper-api): relationship_service scaffold + label validation"
```

---

## Task 7: The schema cutover — breaking migration + all forced Rust repairs (single commit)

**Files:**
- Create: `migrations/20260522100002_edges_as_projection.sql`
- Modify: `crates/temper-core/src/types/graph.rs`
- Modify: `crates/temper-api/src/services/edge_service.rs`
- Modify: `crates/temper-api/src/services/graph_service.rs`
- Modify: `crates/temper-api/src/services/relationship_service.rs`
- Modify: any file the compiler flags (handlers, tests, fixtures)

> **This is the no-green-intermediate-state task.** The migration renames `kb_resource_edges` columns, drops the `edge_type` enum, drops `kb_deferred_edges`, and `CREATE OR REPLACE`s the graph SQL functions. Every `sqlx::query!` referencing the old shape breaks at compile time. Do *all* the repair, then verify, then one commit. Treat the compiler error list as the worklist.

- [ ] **Step 1: Write the breaking migration**

Create `migrations/20260522100002_edges_as_projection.sql`:

```sql
-- Edges as projection — phase-2 schema cutover for limb 1.
-- kb_resource_edges becomes a projection of the relationship-event stream.
-- Spec: docs/superpowers/specs/2026-05-22-limb1-relationship-events-edge-projection-design.md

-- ─── 1. Synthesize a genesis relationship_asserted event per existing edge ──
-- Pre-existing edges must become real ledger history, or a full rebuild loses
-- them. emitter = created_by_profile_id; occurred_at = the edge's created time.
-- edge_kind / polarity / label come from the 8->4 legacy mapping; the payload
-- shape matches temper_core::types::relationship_events::RelationshipAsserted.
INSERT INTO kb_events (
    id, event_type_id, profile_id, device_id, topic_id, scope_id,
    payload, metadata, "references", correlation_id, occurred_at, created
)
SELECT
    public.uuid_generate_v7(),
    (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted'),
    e.created_by_profile_id,
    'migration',
    '019e3d6f-2300-7000-8000-000000000050',  -- declaration topic
    '019e3d6f-2300-7000-8000-000000000010',  -- public scope
    jsonb_build_object(
        'source_resource_id', e.source_resource_id,
        'target', jsonb_build_object('kind', 'resource', 'value', e.target_resource_id),
        'edge_kind', m.edge_kind,
        'polarity',  m.polarity,
        'label',     m.label,
        'weight',    e.weight
    ),
    '{}'::jsonb,
    '[]'::jsonb,
    public.uuid_generate_v7(),
    e.created,
    e.created
FROM kb_resource_edges e
CROSS JOIN LATERAL (
    -- 8->4 mapping, matching EdgeType::legacy_mapping() in temper-core.
    SELECT
        CASE e.edge_type
            WHEN 'parent_of'    THEN 'contains'
            WHEN 'tagged_with'  THEN 'express'
            WHEN 'depends_on'   THEN 'leads_to'
            WHEN 'preceded_by'  THEN 'leads_to'
            WHEN 'derived_from' THEN 'leads_to'
            WHEN 'extends'      THEN 'leads_to'
            WHEN 'relates_to'   THEN 'near'
            WHEN 'references'   THEN 'near'
        END AS edge_kind,
        CASE e.edge_type
            WHEN 'depends_on'   THEN 'inverse'
            WHEN 'preceded_by'  THEN 'inverse'
            WHEN 'derived_from' THEN 'inverse'
            WHEN 'extends'      THEN 'inverse'
            ELSE 'forward'
        END AS polarity,
        e.edge_type::text AS label
) m;

-- ─── 2. Evolve kb_resource_edges into the projection shape ──────────────────
ALTER TABLE kb_resource_edges
    ADD COLUMN edge_kind            edge_kind,
    ADD COLUMN polarity             edge_polarity,
    ADD COLUMN label                text,
    ADD COLUMN asserted_by_event_id uuid REFERENCES kb_events(id),
    ADD COLUMN last_event_id        uuid REFERENCES kb_events(id),
    ADD COLUMN is_folded            boolean NOT NULL DEFAULT false;

-- Backfill the new columns from the legacy edge_type before making them NOT NULL.
UPDATE kb_resource_edges e SET
    edge_kind = (CASE edge_type
        WHEN 'parent_of' THEN 'contains' WHEN 'tagged_with' THEN 'express'
        WHEN 'depends_on' THEN 'leads_to' WHEN 'preceded_by' THEN 'leads_to'
        WHEN 'derived_from' THEN 'leads_to' WHEN 'extends' THEN 'leads_to'
        WHEN 'relates_to' THEN 'near' WHEN 'references' THEN 'near' END)::edge_kind,
    polarity = (CASE edge_type
        WHEN 'depends_on' THEN 'inverse' WHEN 'preceded_by' THEN 'inverse'
        WHEN 'derived_from' THEN 'inverse' WHEN 'extends' THEN 'inverse'
        ELSE 'forward' END)::edge_polarity,
    label = edge_type::text;

-- asserted_by_event_id / last_event_id link each surviving edge row to its
-- genesis event. Match on the synthesized payload's source+target.
UPDATE kb_resource_edges e SET
    asserted_by_event_id = ev.id,
    last_event_id        = ev.id
FROM kb_events ev
WHERE ev.event_type_id = (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted')
  AND ev.device_id = 'migration'
  AND (ev.payload->>'source_resource_id')::uuid = e.source_resource_id
  AND (ev.payload->'target'->>'value')::uuid     = e.target_resource_id
  AND ev.payload->>'label'                       = e.edge_type::text;

ALTER TABLE kb_resource_edges
    ALTER COLUMN edge_kind            SET NOT NULL,
    ALTER COLUMN polarity             SET NOT NULL,
    ALTER COLUMN label                SET NOT NULL,
    ALTER COLUMN asserted_by_event_id SET NOT NULL,
    ALTER COLUMN last_event_id        SET NOT NULL;

-- ─── 3. Drop the legacy columns / constraint / enum ─────────────────────────
ALTER TABLE kb_resource_edges DROP CONSTRAINT uq_resource_edge;
ALTER TABLE kb_resource_edges
    DROP COLUMN edge_type,
    DROP COLUMN created_by_profile_id,
    DROP COLUMN metadata;
DROP TYPE edge_type;

ALTER TABLE kb_resource_edges
    ADD CONSTRAINT uq_resource_edge
    UNIQUE (source_resource_id, target_resource_id, edge_kind, label, polarity);

-- ─── 4. Drop kb_deferred_edges (Gate 3 — replaced by slug-target assertions) ─
DROP TABLE kb_deferred_edges;

-- ─── 5. CREATE OR REPLACE the graph functions for the new column shape ──────
-- graph_traverse, graph_neighbors, graph_resource_edges, graph_subgraph_nodes
-- all SELECT/return edge_type. Re-create each returning edge_kind + polarity +
-- label, and excluding folded edges from the default projection
-- (WHERE NOT e.is_folded). The p_edge_types filter argument now filters on
-- edge_kind::text. Copy each function body from its origin migration
-- (20260411000002, 20260420000001, etc.), apply the column swap, and add the
-- `NOT is_folded` predicate to every kb_resource_edges scan.
--
-- (Full function bodies omitted here for brevity — the implementer copies the
--  current definitions from the migrations enumerated in the spec's "Known
--  ripple" section and applies the mechanical swap. Each function must still
--  return its existing row shape EXCEPT edge_type→edge_kind.)
```

> ⚠️ **Plan/reality gap — the implementer must resolve this concretely.** Step 1's migration text leaves the graph-function `CREATE OR REPLACE` bodies as a directive, not literal SQL, because they are long and must be copied verbatim from their origin migrations. Before writing them: `Read` migrations `20260411000002`, `20260411000003`, `20260420000001`, `20260420000003`, `20260420000004` to collect the *current* definitions of `graph_traverse`, `graph_neighbors`, `graph_resource_edges`, `graph_subgraph_nodes` (the latest definition of each wins). Re-emit each with: `edge_type` column → `edge_kind`; add `polarity`, `label` to any RETURNS TABLE that exposed `edge_type`; add `AND NOT e.is_folded` to every `kb_resource_edges` scan; change the `p_edge_types` filter to compare `edge_kind::text`. This is the bulk of the task.

- [ ] **Step 2: Run the migration; expect the workspace to break**

Run: `sqlx migrate run --source migrations && cargo clean -p temper-api`
Then: `cargo check --workspace 2>&1 | tee /tmp/cutover-errors.txt`
Expected: FAIL — a list of compile errors. This list is the worklist for Steps 3–6.

- [ ] **Step 3: Repair `temper-core/src/types/graph.rs`**

- Delete the `EdgeType` enum, its `Display` impl, and `legacy_mapping` *only after* the migration has consumed the mapping — but `legacy_mapping` is also used by the frontmatter rewire (Task 12). **Keep `EdgeType` + `legacy_mapping`** as a pure-Rust mapping table (it no longer derives `sqlx::Type` against a dropped enum — remove the `sqlx::Type` derive and the `#[sqlx(...)]` attribute from `EdgeType`).
- `GraphTraversalRow`, `GraphNeighborRow`, `GraphEdgeRow`, `GraphEdge`: replace the `edge_type: EdgeType` field with `edge_kind: EdgeKind` (+ `polarity: Polarity` and `label: String` on `GraphEdgeRow` / `GraphEdge` where the SQL now returns them — match the function RETURNS TABLE shapes from Step 1).
- `ResolvedEdge`: replace `edge_type: EdgeType` with `edge_kind: EdgeKind`, add `polarity: Polarity`, `label: String`; drop `metadata` (the column is gone).
- `EdgeReconciliation`: rename `deferred` → keep, but it now counts slug-target assertions, not `kb_deferred_edges` rows; add `folded` for retraction count.

- [ ] **Step 4: Repair `edge_service.rs` minimally to compile**

`edge_service.rs` will not compile (column renames, dropped `kb_deferred_edges`, dropped `metadata`). For *this* task, make it compile by the smallest change that preserves behavior shape — the *behavioral* rewire to emit events is Task 12. Concretely: the SQL strings referencing `edge_type` / `metadata` / `kb_deferred_edges` must change. Since Task 12 rewrites these functions wholesale, the pragmatic move is: **in this task, gut `edge_service`'s write functions to `todo!()`-free stubs that compile and are not yet called incorrectly** — but that risks shipping a broken main. Better: **fold Task 12 into this task.** See Step 4-alt.

- [ ] **Step 4-alt (recommended): pull Task 12's rewire into this commit**

Because `edge_service` cannot reach a correct compiling state without emitting events, execute **Task 12's content here** as part of the cutover. The cutover commit then includes the frontmatter rewire. Update the plan checkboxes accordingly and note the merge in the task report. (Task 12 below remains as the detailed spec for that work; it just lands in this commit.)

- [ ] **Step 5: Repair `graph_service.rs`, handlers, and the relationship_service SQL**

- `graph_service.rs`: every `sqlx::query!` selecting `edge_type` → `edge_kind`; struct field maps follow `graph.rs`.
- `handlers/edges.rs` / `handlers/graph.rs`: response construction follows the renamed fields.
- Add the SQL-bearing functions to `relationship_service.rs` now that the columns exist — see Task 8 for their full spec; implement `apply_relationship_event` + `rebuild_edge_projection` here or in Task 8 depending on whether Task 8 is merged. (If keeping Task 8 separate, `relationship_service` only needs to *compile* here — it has no callers yet.)

- [ ] **Step 6: Repair test fixtures and seed scripts**

`grep -rn 'edge_type\|kb_deferred_edges\|created_by_profile_id' crates/ scripts/ tests/ --include='*.rs' --include='*.sql'` and fix every hit: `crates/temper-api/tests/common/fixtures.rs`, `graph_test.rs`, `edge_ingest_test.rs`, `scripts/seed-graph-fixtures.sql`, `scripts/seed-dev-data.sql`. Fixtures that directly `INSERT INTO kb_resource_edges` must supply the new columns (or be rewritten to assert relationship events — preferred, but a direct insert with the new columns is acceptable for a fixture).

- [ ] **Step 7: Regenerate the SQL cache**

Run: `cargo sqlx prepare --workspace -- --all-features` then per-crate prepare for feature-gated test queries (see `project_sqlx_per_crate_cache_for_feature_gated_tests`).

- [ ] **Step 8: Verify green**

Run: `cargo clean -p temper-api && cargo make check && cargo nextest run -p temper-core -p temper-api --features test-db`
Expected: PASS — full compile + tests. Iterate Steps 3–7 until green.

- [ ] **Step 9: Commit (single coordinated commit)**

```bash
git add -A
git commit -m "feat(db): kb_resource_edges becomes a projection of relationship events

Breaking schema cutover — no intermediate green state. Synthesizes genesis
relationship_asserted events for existing edges, evolves kb_resource_edges
to the projection shape (edge_kind/polarity/label/event linkage/is_folded),
drops the edge_type enum and kb_deferred_edges, and rewires the graph SQL
functions + every Rust caller in one commit."
```

---

## Task 8: `apply_relationship_event` + `rebuild_edge_projection`

**Files:**
- Modify: `crates/temper-api/src/services/relationship_service.rs`
- Test: `crates/temper-api/tests/relationship_projection_test.rs`

> If Task 7 already implemented these (Step 5), this task is just the `test-db` tests. Otherwise implement both here.

The projection core. `apply_relationship_event` mutates `kb_resource_edges` from one event; `rebuild_edge_projection` truncates and replays.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/relationship_projection_test.rs`:

```rust
#![cfg(feature = "test-db")]
//! apply_relationship_event projects edges; rebuild reproduces them.

// Test outline (implementer fleshes out fixture seeding via tests/common):
// 1. Seed two resources A, B in a context the profile can modify.
// 2. Append a relationship_asserted (A -leads_to-> B) via relationship_service
//    and assert one kb_resource_edges row exists with edge_kind=leads_to.
// 3. Append relationship_reweighted; assert weight updated, last_event_id bumped.
// 4. Append relationship_folded; assert the row has is_folded=true and is
//    absent from graph_neighbors output.
// 5. Snapshot graph_traverse output, call rebuild_edge_projection, assert the
//    edge set is identical (folded edge present with is_folded=true).
// 6. Assert a slug-target assertion to a non-existent slug projects NO edge.
```

> ⚠️ The implementer writes the concrete test using the existing `tests/common/` harness (see `edge_ingest_test.rs` for the fixture pattern). Each numbered item is one `#[sqlx::test]`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db relationship_projection`
Expected: FAIL — functions missing (or test file empty).

- [ ] **Step 3: Implement the projection functions**

In `relationship_service.rs`:

```rust
/// Apply one relationship event's delta to kb_resource_edges. Runs on the
/// caller's transaction so it commits atomically with the ledger append.
pub async fn apply_relationship_event(
    tx: &mut sqlx::PgConnection,
    event: &temper_events::Event,
    event_type: temper_events::EventType,
) -> ApiResult<()> {
    use temper_events::EventType::*;
    match event_type {
        RelationshipAsserted => { /* resolve TargetEndpoint; upsert edge row
            ON CONFLICT uq_resource_edge DO UPDATE; if slug target unresolved,
            project nothing. */ }
        RelationshipRetyped => { /* UPDATE edge by correlation -> last_event_id */ }
        RelationshipReweighted => { /* UPDATE weight, last_event_id */ }
        RelationshipFolded => { /* UPDATE is_folded = true, last_event_id */ }
        RelationshipDecayed | RelationshipCorrected => { /* phase-4 no-op */ }
        ConceptCreated | ConceptMutated => { /* not a relationship event */ }
    }
    Ok(())
}

/// Truncate kb_resource_edges and replay every relationship event in ledger
/// order. Idempotent — the validation harness for "drop + rebuild = identical".
pub async fn rebuild_edge_projection(tx: &mut sqlx::PgConnection) -> ApiResult<()> {
    sqlx::query!("TRUNCATE kb_resource_edges").execute(&mut *tx).await?;
    // SELECT every relationship_* event ordered by occurred_at, id; for each,
    // call apply_relationship_event.
    Ok(())
}
```

> Edges are keyed for retype/reweight/fold by `correlation_id` → the `kb_resource_edges` row whose `asserted_by_event_id` shares that correlation. Store the correlation linkage so the UPDATE can find the row. The asserted event's `id == correlation_id` (it is the root), so `asserted_by_event_id` *is* the `correlation_id` — UPDATE `WHERE asserted_by_event_id = $correlation`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-api --features test-db relationship_projection`
Expected: PASS.

- [ ] **Step 5: Regenerate SQL cache + commit**

```bash
cargo sqlx prepare -p temper-api -- --features test-db
cargo make fix && cargo make check
git add crates/temper-api/src/services/relationship_service.rs crates/temper-api/tests/relationship_projection_test.rs .sqlx/
git commit -m "feat(temper-api): apply_relationship_event + rebuild_edge_projection"
```

---

## Task 9: Operations commands + `DomainEvent` variants

**Files:**
- Modify: `crates/temper-core/src/operations/commands.rs`
- Modify: `crates/temper-core/src/operations/events.rs`
- Modify: `crates/temper-core/src/operations/mod.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `commands.rs`:

```rust
    #[test]
    fn assert_relationship_command_round_trips() {
        let cmd = AssertRelationship {
            source: ResourceRef::scoped("@me", "temper", "task", "a"),
            target_slug: "b".to_string(),
            edge_kind: temper_core::types::graph::EdgeKind::LeadsTo,
            polarity: temper_core::types::graph::Polarity::Inverse,
            label: "depends_on".to_string(),
            weight: 1.0,
            origin: Surface::ApiHttp,
        };
        let s = serde_json::to_string(&cmd).unwrap();
        assert_eq!(serde_json::from_str::<AssertRelationship>(&s).unwrap(), cmd);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-core assert_relationship_command`
Expected: FAIL — `AssertRelationship` not found.

- [ ] **Step 3: Add the four command structs**

In `commands.rs` add `AssertRelationship`, `RetypeRelationship`, `ReweightRelationship`, `FoldRelationship` — each `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`, carrying a `ResourceRef` source (or an edge identifier for retype/reweight/fold — use `correlation_id: Uuid` to name the edge lifecycle), the relevant fields, and `origin: Surface`. Mirror the shape of `AssertRelationship` above.

In `events.rs` add `DomainEvent` variants: `DbRelationshipAsserted { correlation_id: Uuid }`, `DbRelationshipRetyped { correlation_id: Uuid }`, `DbRelationshipReweighted { correlation_id: Uuid }`, `DbRelationshipFolded { correlation_id: Uuid }`.

In `mod.rs` re-export the four commands.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo make fix && cargo make check
git add crates/temper-core/src/operations/
git commit -m "feat(temper-core): relationship write commands + DomainEvent variants"
```

---

## Task 10: `DbBackend` relationship-write methods

**Files:**
- Modify: `crates/temper-api/src/backend/db_backend.rs`
- Test: `crates/temper-api/tests/relationship_write_test.rs`

Relationship writes are cloud-only — they go on `DbBackend` as **inherent methods**, not on the shared `Backend` trait (`VaultBackend` does not implement them). Each method: auth-check → begin tx → `append_event_tx` → `apply_relationship_event` → commit.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/relationship_write_test.rs` (`#![cfg(feature = "test-db")]`):

```rust
// 1. Seed resources A, B + a profile that can modify A.
// 2. DbBackend::assert_relationship(AssertRelationship{...}) -> CommandOutput
//    with a DbRelationshipAsserted event; assert the edge is projected.
// 3. A profile that CANNOT modify A gets an auth error, no event appended.
// 4. retype / reweight / fold each mutate the projection + append an event.
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db relationship_write`
Expected: FAIL.

- [ ] **Step 3: Implement the four inherent methods**

On `impl DbBackend`:

```rust
    /// Assert a relationship. Auth: emitter must be able to modify the source.
    pub async fn assert_relationship(
        &self,
        cmd: AssertRelationship,
    ) -> Result<CommandOutput<Uuid>, TemperError> {
        // 1. resolve source ResourceRef -> resource_id
        // 2. can_modify_resource(profile_id, source) — else TemperError auth
        // 3. tx = pool.begin()
        // 4. build RelationshipAsserted payload; EventToWrite::new_root(...)
        //    topic = declaration, scope = public
        // 5. event = append_event_tx(&mut *tx, write)
        // 6. apply_relationship_event(&mut *tx, &event, EventType::RelationshipAsserted)
        // 7. tx.commit()
        // 8. Ok(CommandOutput::with_events(event.correlation_id,
        //      vec![DomainEvent::DbRelationshipAsserted { correlation_id: ... }]))
    }
    // retype_relationship / reweight_relationship / fold_relationship — same
    // shape, EventToWrite::new_correlated with the cmd's correlation_id, and
    // the matching EventType + DomainEvent.
```

> ⚠️ Plan/reality check: confirm the auth helper name. The spec says `can_modify_resource`; grep `crates/temper-api/src/services/access_service.rs` for the real signature before wiring (CLAUDE.md profile-scoping rule names `can_modify_resource` as canonical). Auth-before-writes — the check precedes `tx.begin()`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-api --features test-db relationship_write`
Expected: PASS.

- [ ] **Step 5: Regenerate cache + commit**

```bash
cargo sqlx prepare -p temper-api -- --features test-db
cargo make fix && cargo make check
git add crates/temper-api/src/backend/db_backend.rs crates/temper-api/tests/relationship_write_test.rs .sqlx/
git commit -m "feat(temper-api): DbBackend relationship-write methods"
```

---

## Task 11: API handlers + routes

**Files:**
- Modify: `crates/temper-api/src/handlers/edges.rs`
- Modify: `crates/temper-api/src/routes.rs`

Four POST endpoints. Each builds a `DbBackend` per request (the established pattern) and dispatches one command.

- [ ] **Step 1: Write the failing test**

Add an e2e-style handler test (follow the pattern in existing handler tests — check how `handlers/edges.rs::list` is tested, likely via `tests/graph_test.rs`). Assert `POST /api/relationships` with a valid body returns 200 and projects the edge; an unauthorized caller gets 403.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db relationship_handler`
Expected: FAIL.

- [ ] **Step 3: Implement handlers + routes**

In `handlers/edges.rs` add `assert`, `retype`, `reweight`, `fold` handlers — each `#[utoipa::path(post, ...)]`, extracting `AuthUser` + `State<AppState>` + `Json<...>` request body, building a `DbBackend`, calling the inherent method, returning `Json` of the result.

In `routes.rs` after line 61–62 register:

```rust
        .route("/api/relationships", post(handlers::edges::assert))
        .route("/api/relationships/{correlation_id}/retype", post(handlers::edges::retype))
        .route("/api/relationships/{correlation_id}/reweight", post(handlers::edges::reweight))
        .route("/api/relationships/{correlation_id}/fold", post(handlers::edges::fold))
```

Add `post` to the `axum::routing` import. Register the new `#[utoipa::path]` handlers in `openapi.rs` if it enumerates paths explicitly.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-api --features test-db`
Expected: PASS.

- [ ] **Step 5: Regenerate cache + commit**

```bash
cargo sqlx prepare -p temper-api -- --features test-db
cargo make fix && cargo make check
git add crates/temper-api/src/handlers/edges.rs crates/temper-api/src/routes.rs crates/temper-api/src/openapi.rs .sqlx/
git commit -m "feat(temper-api): relationship write endpoints"
```

---

## Task 12: Rewire the frontmatter edge-extraction path to emit events

**Files:**
- Modify: `crates/temper-api/src/services/edge_service.rs`
- Modify: `crates/temper-api/src/services/ingest_service.rs`
- Modify: `crates/temper-api/src/services/resource_service.rs`
- Modify: `crates/temper-api/src/services/meta_service.rs`

> **If Task 7 Step 4-alt was taken, this work already landed in the cutover commit** — in that case this task is verification-only: confirm the three call sites emit events and the tests below pass.

`edge_service` derives edges from frontmatter. Rewire its write side so create-path declarations become `relationship_asserted` events and update-path removals become `relationship_folded` events. `kb_deferred_edges` and `resolve_deferred_edges` are gone — an unresolved slug becomes a slug-`TargetEndpoint` assertion.

- [ ] **Step 1: Write the failing test**

Adapt `crates/temper-api/tests/edge_ingest_test.rs`: after creating a resource with `extends: [other]` frontmatter, assert (a) a `relationship_asserted` event exists in `kb_events`, and (b) the projected edge exists. After an update removing the relation, assert a `relationship_folded` event exists and the edge has `is_folded = true`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db edge_ingest`
Expected: FAIL — events not emitted (direct upsert still in place).

- [ ] **Step 3: Rewire `edge_service`**

- `upsert_edges` → for each `ResolvedEdge`, append a `relationship_asserted` event (via the relationship_service / `append_event`) and project it. A `ResolvedEdge` with an unresolvable target becomes a slug-`TargetEndpoint` assertion — so `resolve_declarations`'s "unresolved" list is no longer special-cased into `defer_edges`; both resolved and unresolved become assertion events (resolved → `TargetEndpoint::Resource`, unresolved → `TargetEndpoint::Slug`).
- `defer_edges` and `resolve_deferred_edges` — **delete**. The slug-target assertion event is the durable record; create-path re-projection (Task 13) replaces deferred resolution.
- `reconcile_edges` — keep the diff, but: additions emit `relationship_asserted`; removals emit `relationship_folded` for the removed edge's `correlation_id` (look it up via `asserted_by_event_id` on the existing row). Unchanged → nothing.
- Map each frontmatter relation field to `(edge_kind, polarity, label)` via `EdgeType::legacy_mapping()` — the frontmatter field name *is* the label.

- [ ] **Step 4: Update the three call sites**

`ingest_service.rs` (~line 542): `extract_and_upsert_edges` keeps its signature; its body now emits events. Remove the separate `resolve_deferred_edges` call (~line 564) — superseded by Task 13's create-path re-projection.
`resource_service.rs` (~line 996) and `meta_service.rs` (~line 275): `reconcile_edges` keeps its signature; body emits events. No call-site signature change needed if the function signatures are preserved.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run -p temper-api --features test-db edge_ingest edge_`
Expected: PASS.

- [ ] **Step 6: Regenerate cache + commit**

```bash
cargo sqlx prepare -p temper-api -- --features test-db
cargo make fix && cargo make check
git add crates/temper-api/src/services/ .sqlx/
git commit -m "feat(temper-api): frontmatter edge extraction emits relationship events"
```

---

## Task 13: Create-path re-projection of pending slug-target assertions

**Files:**
- Modify: `crates/temper-api/src/services/relationship_service.rs`
- Modify: `crates/temper-api/src/services/ingest_service.rs`
- Test: `crates/temper-api/tests/relationship_projection_test.rs`

The event-sourced replacement for `resolve_deferred_edges`: when a resource is created, any prior `relationship_asserted` event whose slug `target` now matches must project its edge.

- [ ] **Step 1: Write the failing test**

Add a `#[sqlx::test]` to `relationship_projection_test.rs`: assert A→`slug:"b"` (no resource `b` yet) — no edge projected. Create resource `b`. Assert the edge now exists.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db reproject_pending`
Expected: FAIL.

- [ ] **Step 3: Implement `reproject_pending_for_resource`**

```rust
/// After a resource is created, project any relationship_asserted event whose
/// slug target now resolves to it. The event-sourced replacement for the
/// retired kb_deferred_edges holding table.
pub async fn reproject_pending_for_resource(
    tx: &mut sqlx::PgConnection,
    new_resource_id: Uuid,
    new_slug: &str,
) -> ApiResult<usize> {
    // SELECT relationship_asserted events where
    //   payload->'target'->>'kind' = 'slug' AND payload->'target'->>'value' = new_slug
    // and no edge row yet exists for that correlation; project each.
}
```

Call it from `ingest_service::ingest` after the resource row is created (replacing the deleted `resolve_deferred_edges` call), on the ingest transaction.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-api --features test-db relationship_projection`
Expected: PASS.

- [ ] **Step 5: Regenerate cache + commit**

```bash
cargo sqlx prepare -p temper-api -- --features test-db
cargo make fix && cargo make check
git add crates/temper-api/src/services/ crates/temper-api/tests/relationship_projection_test.rs .sqlx/
git commit -m "feat(temper-api): create-path re-projection replaces deferred-edge resolution"
```

---

## Task 14: CLI `temper edge` subcommands

**Files:**
- Modify: `crates/temper-cli/src/commands/graph.rs`

Cloud-mode writes — they POST to the API (no vault path, per the spec's non-concerns). Add an `edge` subcommand group: `assert`, `retype`, `reweight`, `fold`.

- [ ] **Step 1: Write the failing test**

Add a unit test asserting the clap parser accepts `temper edge assert --source <ref> --target <slug> --kind leads_to --polarity inverse --label depends_on`. Follow the arg-parsing test pattern in existing `commands/*.rs`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-cli edge_assert_parses`
Expected: FAIL.

- [ ] **Step 3: Implement the subcommands**

Add an `Edge` variant to the graph command enum (or a new top-level `edge` command — match the existing CLI structure; check how `graph.rs` registers subcommands). Each subcommand builds the request body and calls the temper-client API method (Task 11's endpoints). Add the corresponding client methods to `temper-client` if absent — check `crates/temper-client/src/` for the HTTP-call pattern.

> ⚠️ Plan/reality check: `temper-client` may need new methods for the four endpoints. Grep `crates/temper-client/src/` for an existing edge/graph call to copy the pattern; if none, add to the natural module.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo make fix && cargo make check
git add crates/temper-cli/src/commands/graph.rs crates/temper-client/src/
git commit -m "feat(temper-cli): temper edge assert/retype/reweight/fold"
```

---

## Task 15: MCP relationship tools

**Files:**
- Create: `crates/temper-mcp/src/tools/relationships.rs`
- Modify: `crates/temper-mcp/src/tools/mod.rs`

Four MCP tools mirroring the API. MCP tools delegate to temper-api services (CLAUDE.md). Follow `crates/temper-mcp/src/tools/resources.rs` for the tool-definition + param-struct pattern.

- [ ] **Step 1: Write the failing test**

Add a unit test asserting the four tool param structs deserialize from representative JSON. MCP param structs use `schemars::JsonSchema` — the `mcp` feature must be enabled on `temper-core` for `EdgeKind`/`Polarity` (already added in Task 1).

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-mcp relationship_tool`
Expected: FAIL.

- [ ] **Step 3: Implement the tools**

Create `tools/relationships.rs` with four tools (`assert_relationship`, `retype_relationship`, `reweight_relationship`, `fold_relationship`), each delegating to a `DbBackend` relationship method. Register `pub mod relationships;` in `tools/mod.rs` and wire the tools into the MCP server's tool registry (check how `resources.rs` tools are registered).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-mcp`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo make fix && cargo make check
git add crates/temper-mcp/src/tools/
git commit -m "feat(temper-mcp): relationship write tools"
```

---

## Task 16: e2e validation — the headline rebuild invariant

**Files:**
- Create: `tests/e2e/tests/relationship_projection_e2e_test.rs`

The spec's acceptance criterion: drop + rebuild from events = identical traversal.

- [ ] **Step 1: Write the test**

Create `tests/e2e/tests/relationship_projection_e2e_test.rs` (`#![cfg(feature = "test-db")]`):

```rust
// Through the real Axum server:
// 1. Create several resources; assert a graph of relationships via the API.
// 2. Fold one edge; retype another; reweight a third.
// 3. Snapshot graph_traverse + graph_neighbors output for a seed set.
// 4. Call rebuild_edge_projection (via a test-only path or direct service call).
// 5. Assert the post-rebuild traversal output is byte-identical to the snapshot
//    (folded edge present with is_folded, absent from default traversal).
// 6. Migration fidelity: assert pre-existing seeded edges survive a rebuild.
```

Follow the harness in `tests/e2e/tests/common/` and the pattern of `tests/e2e/tests/graph_build_e2e_test.rs` / `projection_pull_test.rs`.

- [ ] **Step 2: Run the test**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db relationship_projection_e2e`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
cargo make fix && cargo make check
git add tests/e2e/tests/relationship_projection_e2e_test.rs
git commit -m "test(e2e): edge projection rebuild yields identical traversal"
```

---

## Task 17: TypeScript type regeneration + temper-cloud sweep

**Files:**
- Modify: `packages/temper-ui/src/lib/types/generated/graph.ts` (regenerated)
- Modify: `packages/temper-cloud/` (if it queries `kb_resource_edges` / `kb_deferred_edges`)

Limb 0's lesson (`feedback`/memory): a schema change touches every language that queries the table — sweep TypeScript too.

- [ ] **Step 1: Sweep temper-cloud for edge-table queries**

Run: `grep -rn 'kb_resource_edges\|kb_deferred_edges\|edge_type' packages/temper-cloud/src/`
Fix any direct SQL referencing the dropped/renamed columns. Expected: likely none (edges are graph-build territory), but verify — do not assume.

- [ ] **Step 2: Regenerate TS types**

Run: `cargo make generate-ts-types`
Expected: `graph.ts` updates — `EdgeType` → `EdgeKind` + `Polarity`, new fields on edge rows.

- [ ] **Step 3: Typecheck**

Run: `cd packages/temper-ui && bun run check` and `cd packages/temper-cloud && bun run typecheck`
Expected: PASS — or fix UI call sites consuming the renamed fields.

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/lib/types/generated/ packages/temper-cloud/
git commit -m "chore: regenerate TS types for edge_kind; sweep temper-cloud"
```

---

## Task 18: PR-prep verification

- [ ] **Step 1: Full workspace test**

Run: `cargo nextest run --workspace --features test-db` then `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed` (the embed features per CLAUDE.md — workspace feature unification, `project_workspace_feature_unification_ort`).
Expected: PASS. Trust the exit code, not nextest's summary line (`feedback_nextest_summary_lies`).

- [ ] **Step 2: Full check**

Run: `cargo make check && cargo make ts-test`
Expected: PASS.

- [ ] **Step 3: Confirm SQL cache committed**

Run: `git status .sqlx/` — expected: clean (all regenerations committed).

- [ ] **Step 4: Open the PR**

```bash
git push -u origin jct/limb1-relationship-events
gh pr create --title "Limb 1 — relationship events + edge projection" --body "..."
```

Scrutinize the **Embed & MCP Round-Trip** CI job — it is the only tier with ONNX runtime and catches workspace-feature-unification surprises.

---

## Self-review notes

- **Spec coverage:** Gates 1–4 → Tasks 1 (EdgeKind/Polarity), 5 (topics/registry), 7 (migration). Event family → Tasks 2–3. Projection (transactional apply + rebuild) → Tasks 4, 8. Write path (commands/backend/handlers) → Tasks 9–11. CLI+MCP → Tasks 14–15. Frontmatter rewire → Task 12. `kb_deferred_edges` retirement → Tasks 7, 12, 13. Migration of existing edges → Task 7. Validation → Tasks 8, 16. TS sweep → Task 17.
- **Known risk — Task 7 size.** The cutover is irreducibly large (no green intermediate state). The plan flags Step 4-alt: pull Task 12 into the cutover commit because `edge_service` cannot compile correctly otherwise. The executor decides at Task 7 whether to merge; either way the work is specified.
- **Plan/reality gaps flagged inline** with ⚠️ markers: `kb_profiles` column names, graph-function bodies to copy verbatim, `can_modify_resource` signature, `temper-client` method existence. The executor must verify each before dispatching/implementing.
