# Graph Atlas Beat 2b — Node Content Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Atlas Tier-2 nodes real content — a body excerpt in the TrailRail (N1), a richer hover card (N2), and click-to-expand event-payload history (N3).

**Architecture:** Two additive migrations extend the shipped Chunk-B reads (`graph_atlas_nodes` gains a first-chunk excerpt source; `element_trail_edge`/`_node` gain the event `payload` + a humanized actor). New optional wire fields on `AtlasNode` / `ElementEvent` flow through the existing page load to the UI. UI adds an excerpt block + enriched, expandable history to `TrailRail`, a new hover card, and three pure client models. Committed `/dev/atlas` fixtures grow synthetic versions of the new fields so the render harness exercises the new UI.

**Tech Stack:** Rust (sqlx, Axum, temper-core/temper-services), PostgreSQL, ts-rs codegen, SvelteKit 5 + TypeScript, Vitest.

**Spec:** `docs/superpowers/specs/2026-07-05-atlas-beat2b-node-content-design.md`

## Global Constraints

- **Shipped migrations are immutable.** Never edit `migrations/20260703130000_graph_atlas_chunk_b_reads.sql` (or any applied migration) — it breaks prod sqlx checksum validation. Add NEW migrations only. Latest existing timestamp is `20260705000002`; this plan adds `20260706000001` and `20260706000002`.
- **`RETURNS TABLE` changes require `DROP FUNCTION` + `CREATE FUNCTION`**, not `CREATE OR REPLACE` (Postgres forbids changing a function's OUT columns via replace). Both these functions are called only from Rust (no SQL dependents), so DROP is safe.
- **Cross-crate type changes are one atomic commit.** A change spanning `temper-core` (type) → `temper-services` (mapping) → generated `.ts` must land together — the pre-commit hook gates whole-workspace clippy. Commit all regenerated ts-rs output, even incidental.
- **ts-rs regen:** `cargo make generate-ts-types` after any `temper-core` type change. **sqlx caches:** the `element_trail` path uses the `query_as!` **macro** → regenerate with `cargo make prepare-services` (and `cargo make prepare-e2e` if e2e test macros touch the changed SQL). The `neighborhood_slice` path uses **runtime** `query_as::<_, tuple>` → no macro cache, but tests run against a fresh migrated DB.
- **All builds/clippy use `--all-features`.** Lint suppression uses `#[expect(..., reason = "...")]`, never `#[allow]`.
- **`DATABASE_URL`** for local dev = `postgresql://temper:temper@localhost:5437/temper_development`. Reset the dev DB after adding a migration: `cargo make db-reset` (or `sqlx migrate run`).
- **Never emit raw ANSI in CLI/UI** (n/a here) and **never `serde_json::json!()` for known-shape data** — but the event `payload` is genuinely schemaless jsonb, so `serde_json::Value` passthrough is correct here.
- After a temper-cli/e2e change, remember the fresh-binary macOS nextest `--list` hang — run a single e2e target via plain `cargo test --test <name>` rather than nextest list-all.

---

## File Structure

**Backend (Task 1 — N1 excerpt):**
- Create `migrations/20260706000001_atlas_node_excerpt_read.sql` — DROP+CREATE `graph_atlas_nodes` with a `first_chunk` column.
- Modify `crates/temper-core/src/types/graph_atlas.rs` — add `AtlasNode.excerpt`.
- Modify `crates/temper-services/src/services/graph_service.rs:321-342` — select + map `first_chunk` → `excerpt`.
- Regen `packages/temper-ui/src/lib/types/generated/graph_atlas.ts`.
- Test `tests/e2e/tests/graph_atlas_slice_sql_test.rs` (extend).

**Backend (Task 2 — N3 payload/actor):**
- Create `migrations/20260706000002_element_trail_payload_actor.sql` — DROP+CREATE `element_trail_edge`/`_node` with `payload` + `actor_name`.
- Modify `crates/temper-core/src/types/element_trail.rs` — add `ElementEvent.payload` + `.actor_name`.
- Modify `crates/temper-services/src/services/event_service.rs` — carry + trim payload, carry actor_name.
- Regen `packages/temper-ui/src/lib/types/generated/element_trail.ts`.
- Test `tests/e2e/tests/element_trail_sql_test.rs` (extend).

**UI (Tasks 3–6):**
- Create `packages/temper-ui/src/lib/graph/atlas/relativeTime.ts` (+ `.test.ts`).
- Create `packages/temper-ui/src/lib/graph/atlas/eventSummary.ts` (+ `.test.ts`).
- Create `packages/temper-ui/src/lib/graph/atlas/payloadRows.ts` (+ `.test.ts`).
- Modify `packages/temper-ui/scripts/sanitize-atlas-fixtures.mjs` + regenerate `static/dev/atlas-fixtures.json`; extend `src/lib/graph/atlas/fixtures.test.ts`.
- Modify `packages/temper-ui/src/lib/graph/atlas/trail.ts` (TrailRow gains `actorName`, `payload`).
- Modify `packages/temper-ui/src/lib/components/graph/atlas/TrailRail.svelte` (excerpt block + expandable history).
- Create `packages/temper-ui/src/lib/components/graph/atlas/marks/NodeHoverCard.svelte`; modify `marks/NodeChip.svelte` + `TierNeighborhood.svelte`.

---

## Task 1 — N1: body excerpt on the R4 node projection

**Files:**
- Create: `migrations/20260706000001_atlas_node_excerpt_read.sql`
- Modify: `crates/temper-core/src/types/graph_atlas.rs:30-39`
- Modify: `crates/temper-services/src/services/graph_service.rs:321-342`
- Regen: `packages/temper-ui/src/lib/types/generated/graph_atlas.ts`
- Test: `tests/e2e/tests/graph_atlas_slice_sql_test.rs`

**Interfaces:**
- Produces: `AtlasNode.excerpt: Option<String>` (TS: `excerpt: string | null`), populated only on the R4 (`neighborhood_slice`) path via the existing `compute_excerpt` (`graph_service.rs:44`).

- [ ] **Step 1: Write the new migration**

Create `migrations/20260706000001_atlas_node_excerpt_read.sql`:

```sql
-- Beat 2b N1: give the R4 node projection a first-body-chunk column so the
-- neighborhood slice can derive a body excerpt (via compute_excerpt in Rust).
-- Additive over the shipped 20260703130000 graph_atlas_nodes. RETURNS TABLE gains
-- a column, so DROP + CREATE (CREATE OR REPLACE cannot change OUT columns).
-- graph_atlas_nodes is called only from Rust (neighborhood_slice); no SQL dependents.
DROP FUNCTION graph_atlas_nodes(uuid, uuid, uuid[]);

CREATE FUNCTION graph_atlas_nodes(
    p_profile uuid, p_team uuid, p_ids uuid[]
) RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int, first_chunk text)
LANGUAGE sql STABLE AS $$
    WITH scope AS (
        SELECT resource_id AS id FROM resources_in_team_scope(p_profile, p_team)
    ),
    ids AS (SELECT DISTINCT unnest(p_ids) AS id),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type' AND NOT p.is_folded
    )
    SELECT r.id, r.title, d.dt AS doc_type, h.home,
           COALESCE(deg.degree, 0) AS degree,
           (SELECT cc.content FROM kb_chunks ch
              JOIN kb_content_blocks b ON b.id = ch.block_id
              JOIN kb_chunk_content cc ON cc.chunk_id = ch.id
             WHERE ch.resource_id = r.id AND ch.is_current AND NOT b.is_folded
             ORDER BY b.seq, ch.chunk_index LIMIT 1) AS first_chunk
    FROM ids
    JOIN scope s   ON s.id = ids.id
    JOIN kb_resources r ON r.id = ids.id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT CASE WHEN bool_or(h2.anchor_table = 'kb_cogmaps') THEN 'cogmap' ELSE 'context' END AS home
        FROM kb_resource_homes h2 WHERE h2.resource_id = r.id
    ) h ON true
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true;
$$;
```

- [ ] **Step 2: Add `excerpt` to `AtlasNode`**

In `crates/temper-core/src/types/graph_atlas.rs`, extend the struct (after `salience`, `:38`):

```rust
pub struct AtlasNode {
    pub id: Uuid,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    pub home: NodeHome,
    pub degree: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salience: Option<f64>,
    /// First-paragraph body preview (≤280 chars, word-boundary truncated), from the
    /// R4 slice's `first_chunk` via `compute_excerpt`. `None` when the node has no
    /// body, or on any read that doesn't source a first chunk. Renders as the
    /// EXCERPT block in the TrailRail and the hover-card snippet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}
```

- [ ] **Step 3: Select + map `first_chunk` → `excerpt` in `neighborhood_slice`**

In `crates/temper-services/src/services/graph_service.rs`, change the node query (`:321-342`):

```rust
    let nodes: Vec<AtlasNode> =
        sqlx::query_as::<_, (Uuid, String, Option<String>, String, i32, Option<String>)>(
            "SELECT id, title, doc_type, home, degree, first_chunk FROM graph_atlas_nodes($1, $2, $3)",
        )
        .bind(profile_id.as_uuid())
        .bind(team_id)
        .bind(&node_ids)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(id, title, doc_type, home, degree, first_chunk)| AtlasNode {
            id,
            title,
            doc_type,
            home: if home == "cogmap" {
                NodeHome::Cogmap
            } else {
                NodeHome::Context
            },
            degree,
            salience: None, // neighborhood-tier salience deferred (no per-node source yet)
            excerpt: first_chunk.as_deref().and_then(compute_excerpt),
        })
        .collect();
```

- [ ] **Step 4: Write the failing SQL test**

In `tests/e2e/tests/graph_atlas_slice_sql_test.rs`, add a test that a neighborhood node with body text carries a truncated excerpt and a body-less node carries `None`. Match the existing harness/fixtures in that file for team + resource setup (read the file's existing helpers first; reuse its seed pattern). Skeleton:

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn neighborhood_node_carries_body_excerpt(pool: PgPool) {
    // ... existing-harness setup: a team, a profile, two resources with an edge
    // between them so both land in the induced subgraph; give ONE a multi-paragraph
    // body (kb_content_blocks + kb_chunks + kb_chunk_content), leave the OTHER bodiless.
    let sub = neighborhood_slice(&pool, profile, team, SliceRequest {
        seeds: vec![with_body],
        depth: 1,
        edge_kinds: vec![],
    }).await.unwrap();

    let n_body = sub.nodes.iter().find(|n| n.id == with_body).unwrap();
    assert!(n_body.excerpt.as_deref().unwrap().starts_with("First paragraph"));
    let n_bare = sub.nodes.iter().find(|n| n.id == no_body).unwrap();
    assert_eq!(n_bare.excerpt, None);
}
```

- [ ] **Step 5: Reset DB, run the test to verify it fails, then passes**

```bash
cargo make db-reset
cargo test -p temper-e2e --features test-db --test graph_atlas_slice_sql_test neighborhood_node_carries_body_excerpt -- --nocapture
```
Expected: FAIL before Steps 1–3 land (field/column missing), PASS after. (Use plain `cargo test --test`, not nextest, to avoid the fresh-binary `--list` hang.)

- [ ] **Step 6: Regenerate TS types and verify UI compiles**

```bash
cargo make generate-ts-types
cd packages/temper-ui && bun run check
```
Expected: `graph_atlas.ts` `AtlasNode` gains `excerpt: string | null`; `bun run check` = 0 errors.

- [ ] **Step 7: Gate + commit (atomic, cross-crate)**

```bash
cargo make check
git add migrations/20260706000001_atlas_node_excerpt_read.sql \
        crates/temper-core/src/types/graph_atlas.rs \
        crates/temper-services/src/services/graph_service.rs \
        tests/e2e/tests/graph_atlas_slice_sql_test.rs \
        packages/temper-ui/src/lib/types/generated/graph_atlas.ts
git commit -m "feat(atlas): body excerpt on R4 node slice (Beat 2b N1)"
```

---

## Task 2 — N3: event payload + humanized actor on the R5 trail

**Files:**
- Create: `migrations/20260706000002_element_trail_payload_actor.sql`
- Modify: `crates/temper-core/src/types/element_trail.rs:27-38`
- Modify: `crates/temper-services/src/services/event_service.rs:42-113`
- Regen: `packages/temper-ui/src/lib/types/generated/element_trail.ts`
- Test: `tests/e2e/tests/element_trail_sql_test.rs`

**Interfaces:**
- Produces: `ElementEvent.payload: serde_json::Value` (TS: `payload: JsonValue`) and `ElementEvent.actor_name: String` (TS: `actor_name: string`). `resource_created` payloads have their heavy inline `blocks` array stripped.

- [ ] **Step 1: Write the new migration**

Create `migrations/20260706000002_element_trail_payload_actor.sql`:

```sql
-- Beat 2b N3: expose the replay-sufficient kb_events.payload and a humanized actor
-- name on the R5 element trail. Additive over the shipped 20260703130000 trail
-- functions. RETURNS TABLE gains columns → DROP + CREATE. Both functions are called
-- only from Rust (event_service::element_trail); no SQL dependents. emitter_entity_id
-- is a NOT NULL FK to kb_entities, so the actor JOIN never drops rows.
DROP FUNCTION element_trail_edge(uuid, uuid);
CREATE FUNCTION element_trail_edge(
    p_profile uuid, p_edge uuid
) RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamptz,
                metadata jsonb, payload jsonb, actor_name text)
LANGUAGE sql STABLE AS $$
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata, ev.payload, en.name
    FROM kb_edges edg
    JOIN kb_events ev ON (ev.payload ->> 'edge_id')::uuid = edg.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    JOIN kb_entities en ON en.id = ev.emitter_entity_id
    WHERE edg.id = p_edge
      AND anchor_readable_by_profile(p_profile, edg.home_anchor_table, edg.home_anchor_id)
      AND endpoint_readable_by_profile(p_profile, edg.source_table, edg.source_id)
      AND endpoint_readable_by_profile(p_profile, edg.target_table, edg.target_id)
    ORDER BY ev.id;
$$;

DROP FUNCTION element_trail_node(uuid, uuid);
CREATE FUNCTION element_trail_node(
    p_profile uuid, p_resource uuid
) RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamptz,
                metadata jsonb, payload jsonb, actor_name text)
LANGUAGE sql STABLE AS $$
    WITH ev_ids AS (
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload ->> 'resource_id')::uuid = p_resource
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE ev.payload -> 'owner' ->> 'table' = 'kb_resources'
           AND (ev.payload -> 'owner' ->> 'id')::uuid = p_resource
        UNION
        SELECT ev.id FROM kb_events ev
         JOIN kb_content_blocks b ON b.id = (ev.payload ->> 'block_id')::uuid
        WHERE b.resource_id = p_resource
    )
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata, ev.payload, en.name
    FROM ev_ids
    JOIN kb_events ev ON ev.id = ev_ids.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    JOIN kb_entities en ON en.id = ev.emitter_entity_id
    WHERE EXISTS (
        SELECT 1 FROM resources_visible_to(p_profile) v WHERE v.resource_id = p_resource
    )
    ORDER BY ev.id;
$$;
```

- [ ] **Step 2: Add `payload` + `actor_name` to `ElementEvent`**

In `crates/temper-core/src/types/element_trail.rs`, extend the struct (after `confidence`, `:37`):

```rust
pub struct ElementEvent {
    pub event_id: Uuid,
    /// Canonical event-type name (kb_event_types.name), e.g. "relationship_asserted".
    pub kind: String,
    /// The authoring agent entity (kb_events.emitter_entity_id).
    pub actor_entity_id: Uuid,
    /// Humanized actor name (kb_entities.name for the emitter entity).
    pub actor_name: String,
    /// ISO-8601 emission time (kb_events.occurred_at).
    pub occurred_at: String,
    /// ConfidenceBand from event metadata, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    /// The event's replay-sufficient payload (kb_events.payload). Schemaless per
    /// event-type; the UI renders it as an expandable key/value block. `resource_created`
    /// has its heavy inline `blocks` array stripped server-side.
    pub payload: serde_json::Value,
}
```

- [ ] **Step 3: Carry payload + actor_name (with trim) through `event_service`**

In `crates/temper-services/src/services/event_service.rs`, extend `ElementEventRow` (`:42`):

```rust
struct ElementEventRow {
    event_id: Uuid,
    kind: String,
    actor_entity_id: Uuid,
    actor_name: String,
    occurred_at: chrono::DateTime<chrono::Utc>,
    metadata: serde_json::Value,
    payload: serde_json::Value,
}
```

Update BOTH `query_as!` selects (`:70-73` and `:83-86`) to add the two columns, e.g. for the edge branch:

```rust
                r#"SELECT event_id AS "event_id!", kind AS "kind!",
                          actor_entity_id AS "actor_entity_id!", actor_name AS "actor_name!",
                          occurred_at AS "occurred_at!", metadata AS "metadata!",
                          payload AS "payload!"
                     FROM element_trail_edge($1, $2)"#,
```

(and the identical column list for `element_trail_node`). Add a trim helper above `element_trail`:

```rust
/// Strip heavy inline fields from a payload before it rides in the trail response.
/// `resource_created` embeds the full `blocks[]` (content) — useless in a trail and
/// potentially large — so drop it; the summary still shows title/doc_type.
fn trim_payload(kind: &str, mut payload: serde_json::Value) -> serde_json::Value {
    if kind == "resource_created" {
        if let Some(obj) = payload.as_object_mut() {
            obj.remove("blocks");
        }
    }
    payload
}
```

Update the map (`:105-111`):

```rust
            ElementEvent {
                event_id: row.event_id,
                kind: row.kind.clone(),
                actor_entity_id: row.actor_entity_id,
                actor_name: row.actor_name,
                occurred_at: row.occurred_at.to_rfc3339(),
                confidence,
                payload: trim_payload(&row.kind, row.payload),
            }
```

- [ ] **Step 4: Write the failing SQL test**

In `tests/e2e/tests/element_trail_sql_test.rs`, add a test (reuse the file's existing seed helpers — read them first) asserting payload + actor_name flow through and blocks are trimmed:

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn node_trail_carries_payload_and_actor(pool: PgPool) {
    // ... existing-harness setup: a profile/entity, a resource with a property_set
    // event (payload {property_key, value, ...}) and a resource_created event.
    let trail = element_trail(&pool, profile, ElementKind::Node, resource).await.unwrap();

    let prop = trail.events.iter().find(|e| e.kind == "property_set").unwrap();
    assert_eq!(prop.payload["property_key"], serde_json::json!("temper-stage"));
    assert!(!prop.actor_name.is_empty());

    let created = trail.events.iter().find(|e| e.kind == "resource_created").unwrap();
    assert!(created.payload.get("blocks").is_none(), "blocks must be trimmed");
}
```

- [ ] **Step 5: Regenerate the sqlx cache (macro), reset DB, run the test**

```bash
cargo make db-reset
cargo make prepare-services   # element_trail uses query_as! — the SELECT changed
cargo make prepare-e2e        # if element_trail_sql_test uses query! macros
cargo test -p temper-e2e --features test-db --test element_trail_sql_test node_trail_carries_payload_and_actor -- --nocapture
```
Expected: PASS after Steps 1–3.

- [ ] **Step 6: Regenerate TS types and verify UI compiles**

```bash
cargo make generate-ts-types
cd packages/temper-ui && bun run check
```
Expected: `element_trail.ts` `ElementEvent` gains `payload: JsonValue` + `actor_name: string`; `bun run check` = 0 errors.

- [ ] **Step 7: Gate + commit (atomic, cross-crate)**

```bash
cargo make check
git add migrations/20260706000002_element_trail_payload_actor.sql \
        crates/temper-core/src/types/element_trail.rs \
        crates/temper-services/src/services/event_service.rs \
        crates/temper-services/.sqlx tests/e2e/.sqlx \
        tests/e2e/tests/element_trail_sql_test.rs \
        packages/temper-ui/src/lib/types/generated/element_trail.ts
git commit -m "feat(atlas): event payload + humanized actor on R5 trail (Beat 2b N3)"
```

---

## Task 3 — Client models: relative time, event summary, payload rows

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/relativeTime.ts` + `.test.ts`
- Create: `packages/temper-ui/src/lib/graph/atlas/eventSummary.ts` + `.test.ts`
- Create: `packages/temper-ui/src/lib/graph/atlas/payloadRows.ts` + `.test.ts`

**Interfaces:**
- Produces: `relativeTime(iso: string, now?: Date): string`; `summarizeEvent(kind: string, payload: unknown, nodesById?: Map<string,{title:string}>): string | null`; `flattenPayload(value: unknown): { key: string; value: string }[]`.
- Consumes (Task 5): all three, from `TrailRail.svelte`.

- [ ] **Step 1: Write failing tests for `relativeTime`**

Create `packages/temper-ui/src/lib/graph/atlas/relativeTime.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { relativeTime } from './relativeTime';

const now = new Date('2026-07-06T12:00:00Z');
describe('relativeTime', () => {
	it('renders seconds/minutes/hours/days ago', () => {
		expect(relativeTime('2026-07-06T11:59:30Z', now)).toBe('just now');
		expect(relativeTime('2026-07-06T11:30:00Z', now)).toBe('30m ago');
		expect(relativeTime('2026-07-06T10:00:00Z', now)).toBe('2h ago');
		expect(relativeTime('2026-07-04T12:00:00Z', now)).toBe('2d ago');
	});
	it('falls back to a date for old events', () => {
		expect(relativeTime('2026-05-01T12:00:00Z', now)).toBe('2026-05-01');
	});
});
```

- [ ] **Step 2: Run to verify fail**

Run: `cd packages/temper-ui && bun run test src/lib/graph/atlas/relativeTime.test.ts`
Expected: FAIL (module not found).

- [ ] **Step 3: Implement `relativeTime`**

Create `packages/temper-ui/src/lib/graph/atlas/relativeTime.ts`:

```ts
// relativeTime.ts — humanize an ISO timestamp as "2h ago", falling back to a plain
// date for anything older than a week. Pure; `now` is injectable for tests.
export function relativeTime(iso: string, now: Date = new Date()): string {
	const then = new Date(iso).getTime();
	const secs = Math.round((now.getTime() - then) / 1000);
	if (secs < 45) return 'just now';
	const mins = Math.round(secs / 60);
	if (mins < 60) return `${mins}m ago`;
	const hours = Math.round(mins / 60);
	if (hours < 24) return `${hours}h ago`;
	const days = Math.round(hours / 24);
	if (days <= 7) return `${days}d ago`;
	return iso.slice(0, 10);
}
```

- [ ] **Step 4: Run to verify pass**

Run: `bun run test src/lib/graph/atlas/relativeTime.test.ts`
Expected: PASS.

- [ ] **Step 5: Write failing tests for `flattenPayload`**

Create `packages/temper-ui/src/lib/graph/atlas/payloadRows.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { flattenPayload } from './payloadRows';

describe('flattenPayload', () => {
	it('renders scalar keys as key/value rows', () => {
		expect(flattenPayload({ property_key: 'stage', weight: 1 })).toEqual([
			{ key: 'property_key', value: 'stage' },
			{ key: 'weight', value: '1' }
		]);
	});
	it('dot-paths nested objects', () => {
		expect(flattenPayload({ owner: { table: 'kb_resources', id: 'x' } })).toEqual([
			{ key: 'owner.table', value: 'kb_resources' },
			{ key: 'owner.id', value: 'x' }
		]);
	});
	it('json-encodes arrays and stringifies null', () => {
		expect(flattenPayload({ tags: ['a', 'b'], note: null })).toEqual([
			{ key: 'tags', value: '["a","b"]' },
			{ key: 'note', value: 'null' }
		]);
	});
	it('returns [] for non-objects', () => {
		expect(flattenPayload('nope')).toEqual([]);
		expect(flattenPayload(null)).toEqual([]);
	});
});
```

- [ ] **Step 6: Run to verify fail, then implement**

Run: `bun run test src/lib/graph/atlas/payloadRows.test.ts` → FAIL.

Create `packages/temper-ui/src/lib/graph/atlas/payloadRows.ts`:

```ts
// payloadRows.ts — flatten an event payload (schemaless jsonb) into ordered
// key/value rows for the TrailRail's expandable event detail. Nested objects
// dot-path; arrays and other non-scalars are JSON-encoded. One generic renderer
// for every event type — no per-kind logic.
export interface PayloadRow {
	key: string;
	value: string;
}

export function flattenPayload(value: unknown, prefix = ''): PayloadRow[] {
	if (value === null || typeof value !== 'object' || Array.isArray(value)) {
		return prefix ? [{ key: prefix, value: scalar(value) }] : [];
	}
	const rows: PayloadRow[] = [];
	for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
		const key = prefix ? `${prefix}.${k}` : k;
		if (v !== null && typeof v === 'object' && !Array.isArray(v)) {
			rows.push(...flattenPayload(v, key));
		} else {
			rows.push({ key, value: scalar(v) });
		}
	}
	return rows;
}

function scalar(v: unknown): string {
	if (v === null) return 'null';
	if (typeof v === 'string') return v;
	if (typeof v === 'number' || typeof v === 'boolean') return String(v);
	return JSON.stringify(v);
}
```

Run: `bun run test src/lib/graph/atlas/payloadRows.test.ts` → PASS.

- [ ] **Step 7: Write failing tests for `summarizeEvent`**

Create `packages/temper-ui/src/lib/graph/atlas/eventSummary.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { summarizeEvent } from './eventSummary';

describe('summarizeEvent', () => {
	it('summarizes property_set as key → value', () => {
		expect(summarizeEvent('property_set', { property_key: 'temper-stage', value: 'in-progress' }))
			.toBe('temper-stage → in-progress');
	});
	it('summarizes relationship_asserted with a resolved target title', () => {
		const nodes = new Map([['t1', { title: 'Cutover checklist' }]]);
		expect(
			summarizeEvent(
				'relationship_asserted',
				{ label: 'derived_from', target: { id: 't1' } },
				nodes
			)
		).toBe('derived_from → Cutover checklist');
	});
	it('falls back to the relationship label when the target is unknown', () => {
		expect(summarizeEvent('relationship_asserted', { label: 'part_of', target: { id: 'zzz' } }))
			.toBe('part_of');
	});
	it('returns null for kinds with no useful summary', () => {
		expect(summarizeEvent('resource_created', { title: 'x' })).toBeNull();
	});
	it('never throws on malformed payloads', () => {
		expect(summarizeEvent('property_set', null)).toBeNull();
	});
});
```

- [ ] **Step 8: Run to verify fail, then implement**

Run: `bun run test src/lib/graph/atlas/eventSummary.test.ts` → FAIL.

Create `packages/temper-ui/src/lib/graph/atlas/eventSummary.ts`:

```ts
// eventSummary.ts — a one-line, best-effort summary of an event for the collapsed
// TrailRail history row. Payload-first; relationship summaries resolve a target
// TITLE from the loaded subgraph nodes when present, else fall back to the label.
// Never throws — a malformed/unknown payload yields null (row shows kind + actor only).
export function summarizeEvent(
	kind: string,
	payload: unknown,
	nodesById?: Map<string, { title: string }>
): string | null {
	if (payload === null || typeof payload !== 'object') return null;
	const p = payload as Record<string, unknown>;
	switch (kind) {
		case 'property_set':
		case 'property_asserted': {
			const key = str(p.property_key);
			if (!key) return null;
			const val = 'value' in p ? scalarish(p.value) : null;
			return val === null ? key : `${key} → ${val}`;
		}
		case 'relationship_asserted':
		case 'relationship_retyped':
		case 'relationship_reweighted': {
			const label = str(p.label) ?? str(p.edge_kind);
			const targetId = str((p.target as Record<string, unknown> | undefined)?.id);
			const title = targetId ? nodesById?.get(targetId)?.title : undefined;
			if (label && title) return `${label} → ${title}`;
			return label ?? null;
		}
		default:
			return null;
	}
}

function str(v: unknown): string | null {
	return typeof v === 'string' && v.length > 0 ? v : null;
}
function scalarish(v: unknown): string | null {
	if (typeof v === 'string') return v;
	if (typeof v === 'number' || typeof v === 'boolean') return String(v);
	return null;
}
```

Run: `bun run test src/lib/graph/atlas/eventSummary.test.ts` → PASS.

- [ ] **Step 9: Gate + commit**

```bash
cd packages/temper-ui && bun run check && bun run test
git add src/lib/graph/atlas/relativeTime.ts src/lib/graph/atlas/relativeTime.test.ts \
        src/lib/graph/atlas/eventSummary.ts src/lib/graph/atlas/eventSummary.test.ts \
        src/lib/graph/atlas/payloadRows.ts src/lib/graph/atlas/payloadRows.test.ts
git commit -m "feat(atlas): TrailRail client models — relativeTime, summarizeEvent, flattenPayload (Beat 2b)"
```

---

## Task 4 — Fixtures: synthesize the new fields for the render harness

**Files:**
- Modify: `packages/temper-ui/scripts/sanitize-atlas-fixtures.mjs`
- Regen: `packages/temper-ui/static/dev/atlas-fixtures.json`
- Modify: `packages/temper-ui/src/lib/graph/atlas/fixtures.test.ts`

**Interfaces:**
- Consumes: `AtlasNode.excerpt`, `ElementEvent.payload`, `ElementEvent.actor_name` (Tasks 1–2 wire types).
- Produces: a committed bundle whose `nodeSelected` neighborhood nodes carry an `excerpt` and whose trail events carry `payload` + `actor_name`, so the harness renders N1/N2/N3 before prod serves them.

- [ ] **Step 1: Extend the sanitizer to synthesize the new fields**

In `packages/temper-ui/scripts/sanitize-atlas-fixtures.mjs`, after the `walk`-based sanitize and before writing each scenario, inject synthetic values where the real capture lacks them (a real capture taken after the backend ships will already have them — only synthesize when absent). Add a post-pass:

```js
// Beat 2b: ensure the harness can render node content even from a pre-backend capture.
// Synthesize an excerpt on neighborhood nodes and a payload + actor_name on trail events
// when a (pre-backend) capture lacks them. Deterministic, personal-data-free.
function ensureNodeContent(view) {
	if (view && view.neighborhood && Array.isArray(view.neighborhood.nodes)) {
		view.neighborhood.nodes.forEach((n, i) => {
			if (n.excerpt === undefined)
				n.excerpt =
					i % 3 === 0
						? null
						: `${synthText('excerpt' + i)} — ${synthText('excerpt-tail' + i)}.`;
		});
	}
	if (view && view.trail && Array.isArray(view.trail.events)) {
		view.trail.events.forEach((e, i) => {
			if (e.actor_name === undefined) e.actor_name = i % 2 === 0 ? 'system' : synthText('actor' + i);
			if (e.payload === undefined)
				e.payload =
					e.kind === 'property_set'
						? { property_key: 'temper-stage', value: 'in-progress', weight: 1 }
						: e.kind === 'relationship_asserted'
							? { label: 'derived_from', target: { id: view.neighborhood?.nodes?.[0]?.id ?? '0' } }
							: { note: synthText('payload' + i) };
		});
	}
	return view;
}
```

Call `ensureNodeContent(sanitized[scenario])` in the scenario loop (after `walk`). Re-run:

```bash
cd packages/temper-ui && node scripts/sanitize-atlas-fixtures.mjs
```

- [ ] **Step 2: Extend the fixtures test**

In `packages/temper-ui/src/lib/graph/atlas/fixtures.test.ts`, add assertions:

```ts
it('nodeSelected neighborhood nodes carry an excerpt field', () => {
	const view = scenario('nodeSelected');
	const withExcerpt = view.neighborhood?.nodes.filter((n) => typeof n.excerpt === 'string') ?? [];
	expect(withExcerpt.length).toBeGreaterThan(0);
});

it('nodeSelected trail events carry payload + actor_name', () => {
	const events = scenario('nodeSelected').trail?.events ?? [];
	expect(events.length).toBeGreaterThan(0);
	for (const e of events) {
		expect(e).toHaveProperty('payload');
		expect(typeof e.actor_name).toBe('string');
	}
});
```

- [ ] **Step 3: Run tests + verify no personal-data regression**

```bash
bun run test src/lib/graph/atlas/fixtures.test.ts
bun run check
```
Expected: PASS (incl. the existing no-leak assertion).

- [ ] **Step 4: Commit**

```bash
git add scripts/sanitize-atlas-fixtures.mjs static/dev/atlas-fixtures.json \
        src/lib/graph/atlas/fixtures.test.ts
git commit -m "test(atlas): synthesize excerpt/payload/actor into committed fixtures (Beat 2b)"
```

---

## Task 5 — TrailRail: excerpt block + enriched, expandable history

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/trail.ts`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TrailRail.svelte`

**Interfaces:**
- Consumes: `AtlasNode.excerpt`; `ElementEvent.payload`/`.actor_name`; `relativeTime`, `summarizeEvent`, `flattenPayload` (Task 3).

- [ ] **Step 1: Extend `TrailRow` to carry actor + payload; update `trailModel`**

In `packages/temper-ui/src/lib/graph/atlas/trail.ts`, extend `TrailRow` and the map (keep the existing `id`/`kind`/`occurredAt`/`confidence`; `actor` was the raw entity id — keep it, add the humanized name + payload):

```ts
export interface TrailRow {
	id: string;
	kind: string;
	rawKind: string; // canonical name for summarizeEvent (kind is humanized for display)
	actor: string;
	actorName: string;
	occurredAt: string;
	confidence: string | null;
	payload: unknown;
}
```

In `trailModel`, set `rawKind: e.kind`, `actorName: e.actor_name`, `payload: e.payload` (leave `kind: humanizeKind(e.kind)`).

- [ ] **Step 2: Update `trail.test.ts` if present**

Read `packages/temper-ui/src/lib/graph/trail.test.ts` (note: the atlas trail model may be covered there or not). If it constructs `EventTrail` fixtures, add `payload: {}` and `actor_name: 'system'` to each event so it compiles, and assert `rawKind`/`actorName`/`payload` pass through. Run `bun run test src/lib/graph/trail.test.ts` → PASS.

- [ ] **Step 3: Add the EXCERPT block + expandable history to `TrailRail.svelte`**

In `packages/temper-ui/src/lib/components/graph/atlas/TrailRail.svelte`:

1. Import the models and add per-row expand state:

```ts
	import { relativeTime } from '$lib/graph/atlas/relativeTime';
	import { summarizeEvent } from '$lib/graph/atlas/eventSummary';
	import { flattenPayload } from '$lib/graph/atlas/payloadRows';

	const nodeExcerpt = $derived(node?.excerpt ?? null);
	const nodesById = $derived(
		new Map((subgraph?.nodes ?? []).map((n) => [n.id, { title: n.title }]))
	);
	let openEvent = $state<string | null>(null);
```

2. Add the EXCERPT block directly under `<h2 class="title">` (read-first), guarded on presence:

```svelte
		{#if isNode && nodeExcerpt}
			<section class="excerpt"><div class="label">EXCERPT</div><p>{nodeExcerpt}</p></section>
		{/if}
```

3. Replace the history row markup (`{#each rows.slice(0, 50)...}`) with enriched, expandable rows:

```svelte
				{#each rows.slice(0, 50) as row (row.id)}
					{@const summary = summarizeEvent(row.rawKind, row.payload, nodesById)}
					<div class="event">
						<button class="event-head" onclick={() => (openEvent = openEvent === row.id ? null : row.id)}>
							<span class="ekind">{row.kind}</span>
							<span class="chev">{openEvent === row.id ? '⌄' : '›'}</span>
						</button>
						{#if summary}<div class="ev-summary">{summary}</div>{/if}
						<div class="ev-meta">by <b>{row.actorName}</b> · {relativeTime(row.occurredAt)}{#if row.confidence} · {row.confidence}{/if}</div>
						{#if openEvent === row.id}
							<dl class="ev-payload">
								{#each flattenPayload(row.payload) as pr (pr.key)}
									<div><dt>{pr.key}</dt><dd>{pr.value}</dd></div>
								{/each}
							</dl>
						{/if}
					</div>
				{/each}
```

4. Add scoped styles for `.excerpt`, `.event-head`, `.chev`, `.ev-summary`, `.ev-meta`, `.ev-payload` matching the panel's existing monospace-label / serif aesthetic (see the mockup `.superpowers/brainstorm/*/content/history-detail.html` and `rail-excerpt.html` for the target look: excerpt = `#aeb7c4` body with a hue left-border; summary = `#aeb7c4`; actor = `#8a929e` with `b` in `#b7c0cd`; payload = monospace `key`/`value` grid).

- [ ] **Step 4: Verify compile + fixtures + render in the harness**

```bash
cd packages/temper-ui && bun run check && bun run test
mv static/dev/atlas-fixtures.local.json /tmp/atlas.local.bak 2>/dev/null || true   # force committed default
bun run dev --port 5199 &
```
Then load `http://localhost:5199/dev/atlas`, scenario `nodeSelected`: confirm the EXCERPT block renders under the title, history rows show summary + `by <actor> · Nh ago`, and clicking a row expands the key/value payload. Check `leafBare` degrades (no excerpt block). Screenshot both. Restore: `mv /tmp/atlas.local.bak static/dev/atlas-fixtures.local.json` and stop the dev server.

- [ ] **Step 5: Commit**

```bash
git add src/lib/graph/atlas/trail.ts src/lib/graph/trail.test.ts \
        src/lib/components/graph/atlas/TrailRail.svelte
git commit -m "feat(atlas): TrailRail excerpt block + expandable event-payload history (Beat 2b N1/N3)"
```

---

## Task 6 — Node hover card (N2)

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/marks/NodeHoverCard.svelte`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/marks/NodeChip.svelte`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierNeighborhood.svelte`

**Interfaces:**
- Consumes: `AtlasNode` fields (`title`, `doc_type`, `degree`, `excerpt`, `home`) — passed from `TierNeighborhood` through `NodeChip`.

- [ ] **Step 1: Pass excerpt + degree into `NodeChip`**

In `TierNeighborhood.svelte`, the `{#each graph.nodes ...}` block already passes `title`/`docType`/`home`. Add `excerpt={n.excerpt ?? null}` and `edges={n.degree}` (confirm the local node view-model carries `excerpt`/`degree` from `AtlasNode`; if it's remapped, thread the two fields through that mapping first).

- [ ] **Step 2: Add the Standard hover card**

Create `NodeHoverCard.svelte` (an SVG `<foreignObject>` positioned above the node, so it renders HTML inside the canvas), rendering the **Standard** layout chosen in brainstorming: doctype pill (hue) + `⌷N edges` + serif title + a 2-line clamped snippet (`excerpt`) + a muted "click → open in rail" hint. Props: `x, y, r, title, docType, edges: number, excerpt: string | null`.

In `NodeChip.svelte`, replace the title-only hover `<text>` (`showLabel` block) with: keep the small label for the anchored (non-hover) case, but when `hovered`, render `<NodeHoverCard .../>` instead. Keep the existing `.atlas-focusable`/focus-ring markup unchanged (don't regress #276's focus rings).

- [ ] **Step 3: Verify compile + render in the harness**

```bash
cd packages/temper-ui && bun run check
# harness as in Task 5 Step 4; hover a neighborhood node in `nodeSelected`
```
Confirm the hover card shows pill + edge-count + title + 2-line snippet + hint, positioned above the node, in light and dark. Screenshot.

- [ ] **Step 4: Commit**

```bash
git add src/lib/components/graph/atlas/marks/NodeHoverCard.svelte \
        src/lib/components/graph/atlas/marks/NodeChip.svelte \
        src/lib/components/graph/atlas/TierNeighborhood.svelte
git commit -m "feat(atlas): Standard node hover card — doctype, edge-count, snippet (Beat 2b N2)"
```

---

## Task 7 — Full-stack gates + branch finish

**Files:** none (verification + PR).

- [ ] **Step 1: Full backend + TS gates**

```bash
cargo make check
cargo make test-e2e            # test-db tier (slice + trail sql tests)
cd packages/temper-ui && bun run check && bun run test
```
Expected: all green. If `cargo make check` reports `relation "..." does not exist`, run `sqlx migrate run` against the dev DB (not a push blocker; CI uses SQLX_OFFLINE).

- [ ] **Step 2: Harness sweep + capture**

Run the harness against the committed default (local override moved aside) and screenshot `nodeSelected` (excerpt + expanded history), `leafBare` (graceful degrade), and a hover card — in light and dark. These are the branch's visual verification.

- [ ] **Step 3: Merge main, push, open PR**

```bash
git fetch origin && git merge origin/main
cargo make check    # re-gate after merge (sibling-PR drift, sqlx)
git push -u origin jct/atlas-beat2b-node-content
gh pr create --title "Atlas Beat 2b — node content: excerpt, hover, event-payload history" --body "..."
```
Note in the PR body: authenticated Atlas can't be verified on a Vercel preview — browser-verify in prod post-merge. The branch also carries the earlier task-1 fixtures commit + the design spec commit.

---

## Self-Review

**Spec coverage:**
- N1 excerpt (read + type + render, read-first placement) → Tasks 1, 5. ✓
- N2 Standard hover card → Task 6. ✓
- N3 payload + humanized actor + expandable key/value + summary + relative time → Tasks 2, 3, 5. ✓
- One-migration decision → refined to **two** migrations (spec sanctioned the split as equivalent; two files avoid local sqlx-checksum friction from editing an unshipped-but-applied migration between tasks). ✓
- `resource_created` blocks trim → Task 2 Step 3. ✓
- Bare-leaf graceful degrade → Task 5 Step 3 (excerpt block guarded on presence). ✓
- Fixtures synthesize new fields → Task 4. ✓
- Testing tiers (sqlx/e2e, TS unit, harness) → Tasks 1/2 (sql tests), 3 (vitest), 5/6/7 (harness). ✓
- Deferred (Tier-1 hover, lazy payload, "you"-detection) → not implemented, by design. ✓

**Placeholder scan:** No TBD/TODO. The two sql-test skeletons say "reuse the file's existing seed helpers" rather than inventing a seed harness that may diverge from the real one — the implementer must read the existing `*_sql_test.rs` setup; this is a deliberate instruction, not a placeholder. Styling in Task 5 Step 3.4 / Task 6 Step 2 references the committed mockups for the exact palette rather than restating hex values a third time.

**Type consistency:** `AtlasNode.excerpt: Option<String>`↔`excerpt: string | null`; `ElementEvent.payload: serde_json::Value`↔`payload: JsonValue`, `.actor_name: String`↔`actor_name: string`; `TrailRow` gains `rawKind`/`actorName`/`payload` (Task 5.1) consumed by `summarizeEvent(row.rawKind, row.payload, nodesById)` and the actor line (Task 5.3). `summarizeEvent`/`relativeTime`/`flattenPayload` signatures (Task 3) match their call sites (Task 5.3). Event-type strings are underscore (`property_set`, `relationship_asserted`, `resource_created`) in both the Rust trim and `summarizeEvent`. ✓
