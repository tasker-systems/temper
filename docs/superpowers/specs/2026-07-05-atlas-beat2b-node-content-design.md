# Graph Atlas Beat 2b — Node content: excerpt, richer hover, event-payload history

**Date:** 2026-07-05
**Goal:** Graph Atlas (`019f28a1`) · **Task:** C3.1 Beat 2b (`019f2fbe`, node-content slice of the larger C3.1 arc)
**Mode/effort:** plan / large
**Design forks resolved interactively** (visual companion mockups + terminal decisions) — captured under "Decisions".

## Throughline

The Atlas map's deep Tier-2 content "actually looks great," but the node **detail** is thin. The
TrailRail panel shows title / doc-type / context / neighbors / sparse history but **no body
excerpt**, node **hover** shows only the title, and the **event history** is a bare
`Kind · date` list with no way to see what actually changed. This beat makes the node panel and
hover *say what a node is and what happened to it* — three additive reads over the existing
Tier-2 (neighborhood) substrate: a body excerpt, an enriched hover card, and click-to-expand
event payloads. It is **additive-only**: new read columns + wire fields + UI, no access-model or
data-model change.

Scope is deliberately the three node-content items **N1, N2, N3** of task `019f2fbe`. The task's
other clusters (B1–B4 nav bugs, W1 wayfinding, L1–L3 layout, G1–G2 legibility) are separate beats
and are **not touched here**.

## The three items

- **N1 — TrailRail carries a body excerpt.** A server-derived first-paragraph preview (≤280 chars,
  word-boundary truncation) rendered as an `EXCERPT` block **directly under the title** (read-first).
  Neighbors already render (`atlasNeighbors`, shipped in a prior beat) — only the excerpt is new.
- **N2 — Richer node hover.** Replace `NodeChip`'s title-only hover label with a **Standard hover
  card**: doctype pill + edge-count + serif title + a 2-line body snippet + a "click → open in rail"
  hint.
- **N3 — Event history detail.** Enrich each history row (payload-derived summary line + humanized
  actor + relative time) and make each row **click-to-expand** to reveal the event's full payload,
  rendered by one generic key/value walker.

## Architecture

### A. One additive migration — extend the shipped Chunk-B reads

The three SQL functions this beat extends all live in the **shipped, immutable** migration
`migrations/20260703130000_graph_atlas_chunk_b_reads.sql`:
`graph_atlas_nodes` (`:88`), `element_trail_edge` (`:260`), `element_trail_node` (`:280`).
Editing a shipped migration breaks prod sqlx checksum validation, so this beat adds **one new
migration** that `CREATE OR REPLACE`s all three functions (additive-only-on-`main` invariant; the
`migrations/` set continues to reproduce prod from scratch).

**Why one migration, not two:** both reads extend the same Chunk-B surface and ship together as one
PR/beat. A single `…_atlas_node_content_reads.sql` keeps the change reviewable as one unit. (If
review prefers separation, splitting into an excerpt migration + a trail-payload migration is
mechanically equivalent — sqlx applies both.)

#### A1. Excerpt on the R4 node projection (`graph_atlas_nodes`)

- Add a `first_chunk text` output column to `graph_atlas_nodes`, populated by the **same** correlated
  subquery already proven for the legacy path — first body chunk via
  `kb_chunks → kb_content_blocks → kb_chunk_content ORDER BY b.seq, ch.chunk_index LIMIT 1`
  (the legacy `graph_subgraph_nodes` derivation; the machinery consuming it lives at
  `crates/temper-services/src/services/graph_service.rs:168,188`).
- `neighborhood_slice` (`graph_service.rs:262`) builds `AtlasNode`s from
  `SELECT id, title, doc_type, home, degree FROM graph_atlas_nodes(...)` (`:322` → mapped at `:330`).
  Extend the `SELECT` to include `first_chunk` and set
  `excerpt: rec.first_chunk.as_deref().and_then(compute_excerpt)` — `compute_excerpt`
  (`graph_service.rs:44`, `EXCERPT_MAX_CHARS = 280` at `:33`) is already in this file; **zero new
  truncation logic**.
- **Scope: R4 (`graph_atlas_nodes` / Tier-2 neighborhood) only.** `graph_region_members` (R3, Tier-1
  territory interior, `migration :229`) is **not** extended — there is no TrailRail at Tier-1 and the
  hover card is Tier-2-only (see N2). Tier-1 member excerpt is Deferred.

#### A2. Payload + actor on the R5 trail reads (`element_trail_edge`/`_node`)

- Both functions already `JOIN kb_events ev` and already `SELECT ev.metadata` (for `confidence`).
  Add **`ev.payload`** (already-materialized jsonb on the same row — `kb_events.payload`,
  `migrations/20260624000001_canonical_schema.sql:475`, "replay-sufficient, per-event-type") to both
  `RETURNS TABLE` signatures and both `SELECT`s. No extra query, no N+1.
- Add a humanized actor: `JOIN kb_entities en ON en.id = ev.emitter_entity_id` (optionally
  `LEFT JOIN kb_profiles p ON p.id = en.profile_id`) and return `en.name` (fallback
  `p.display_name`/`handle`) as `actor_name`. The visibility gate stays inside these functions
  (unchanged).

### B. Wire types (temper-core → ts-rs regen)

- `AtlasNode` (`crates/temper-core/src/types/graph_atlas.rs:30`; fields today `id, title, doc_type,
  home, degree, salience`) gains **`pub excerpt: Option<String>`**. Regenerate `graph_atlas.ts`.
- `ElementEvent` (`crates/temper-core/src/types/element_trail.rs:27`; fields today `event_id, kind,
  actor_entity_id, occurred_at, confidence`) gains **`pub payload: serde_json::Value`** and
  **`pub actor_name: String`**. Regenerate `element_trail.ts`.
- All regenerated ts-rs output is committed (even incidental unrelated regen), per repo convention.

### C. Service mapping (temper-services)

- `neighborhood_slice` — as A1 (set `excerpt`). Re-run `cargo make prepare-services` (moved/new SQL
  in a test-adjacent crate needs the per-crate `.sqlx` cache), then `prepare-api` and `prepare-e2e`
  as touched.
- `event_service::element_trail` (`crates/temper-services/src/services/event_service.rs:60`;
  `ElementEventRow` at `:42`) — carry `payload` + `actor_name` through the row → `ElementEvent`
  mapping. **Trim heavy payloads:** for `resource_created` (payload carries an inline `blocks[]`
  with content — `crates/temper-substrate/src/payloads.rs:269`), strip block *content* before
  returning (keep a block count / titles) so the trail response stays light. Other kinds
  (`property_set` → `{property_key, value, weight}` at `payloads.rs:316`; `relationship_asserted` →
  `{source, target, edge_kind, polarity, label, weight}` at `:287`; reweight/retype/fold) are small
  and pass through whole.

### D. UI (temper-ui)

All three consumers already receive their data through the existing page load
(`src/routes/(app)/graph/[owner]/+page.server.ts`) — no new client read wiring beyond the new fields.

- **TrailRail** (`src/lib/components/graph/atlas/TrailRail.svelte`):
  - **N1 EXCERPT block** immediately under `<h2 class="title">`, before `NEIGHBORS`. Source:
    the selected node's `AtlasNode.excerpt` (from the loaded `subgraph`). Guard on presence — a bare
    leaf (empty subgraph, falls back to `resourceRow`) shows **no** block (graceful degrade,
    consistent with the "no mapped neighbors yet" state #276 established).
  - **N3 history** rows become: `Kind` + a payload-derived **summary line** + **actor** (`by <name>`)
    + **relative time**; each row is a `<button>`/expandable disclosure that reveals the payload via
    the generic key/value walker. `trailModel` (`src/lib/graph/atlas/trail.ts`) extends `TrailRow`
    with `actorName` and the raw `payload`; the **summary line is computed in the component** via
    `summarizeEvent(kind, payload, subgraph)` — the subgraph (which the component holds, `trailModel`
    does not) is what lets relationship summaries resolve a target *title* rather than a bare id.
- **Hover card** — a new `NodeHoverCard.svelte` mark (or extension of
  `src/lib/components/graph/atlas/marks/NodeChip.svelte`, which today reveals only a truncated
  `<text>` label on `hovered`). Renders the **Standard** layout (doctype pill + `⌷N edges` + serif
  title + 2-line clamped snippet + "click → open in rail" hint), positioned above the node. Consumes
  `AtlasNode` fields already present after B (`excerpt`, `degree`, `doc_type`, `title`, `home`).
- **Client models** (pure, unit-tested, in `src/lib/graph/atlas/`):
  - `summarizeEvent(kind, payload)` → the collapsed summary line. Payload-first
    (`property_set` → `` `${property_key} → ${value}` ``; `relationship_*` → relationship + target
    title *resolved from the loaded subgraph when present, else omitted*). Best-effort, never throws.
  - `relativeTime(iso)` → `"2h ago"` style.
  - a generic **payload → key/value rows** flattener: renders **every** payload key as a `key → value`
    row (nested objects indented), one renderer for all event types — no per-kind filtering. It reads
    cleaner than raw JSON (no braces/quotes noise) but does **not** hide plumbing keys like
    `owner.table`; suppressing specific keys would be per-kind work and is out of scope.
  - the hover-card view-model.

### E. Fixtures & the render harness

The committed `/dev/atlas` fixtures (from the task-1 first commit on this branch) don't yet carry the
new fields. To exercise the new UI in the harness **before** prod serves the fields:

- Extend the sanitizer (`packages/temper-ui/scripts/sanitize-atlas-fixtures.mjs`) / committed bundle
  to **synthesize** `excerpt` on `AtlasNode`, and `payload` + `actor_name` on `ElementEvent`, for the
  relevant scenarios (esp. `nodeSelected`). Keep it personal-data-free.
- Grow `src/lib/graph/atlas/fixtures.test.ts` to assert the new fields are present where expected
  (e.g. `nodeSelected` neighborhood nodes carry an `excerpt`; trail events carry `payload` +
  `actor_name`). The `REQUIRED_KEYS`/`satisfies` top-level gate is unchanged (these are nested).
- After the backend deploys, real fixtures can be re-captured per the README recipe.

## Testing

- **Backend (`#[sqlx::test]`, gated `test-db`):**
  - `graph_atlas_nodes`/`neighborhood_slice`: a node with body → truncated excerpt; a node with no
    body → `excerpt = None`.
  - `element_trail_*`: an event with a payload → `payload` returned intact; `actor_name` resolves via
    the `kb_entities` join; `resource_created` block content is trimmed.
  - Regenerate per-crate `.sqlx` caches: `cargo sqlx prepare --workspace -- --all-features` →
    `prepare-services` → `prepare-api` → `prepare-e2e` (whichever targets the new SQL).
- **e2e (`tests/e2e/`):** assert the excerpt flows through the gated R4 slice and the payload +
  actor_name flow through the gated R5 trail — the reads are visibility-scoped, so verify through the
  real Axum + Postgres path, not just a unit fixture.
- **TS (vitest, node):** `summarizeEvent`, `relativeTime`, the payload key/value flattener, and the
  hover-card view-model — all pure. Plus the extended `fixtures.test.ts` assertions.
- **Harness (browser):** render-verify every scenario in `/dev/atlas` — TrailRail excerpt block,
  expandable history row, and the hover card — against the synthesized fixtures, in light + dark.
- **Gates:** `cargo make check` (fmt/clippy/docs/machete + tsc/biome), `bun run check`,
  `bun run test`, the sqlx caches; browser-verify in prod post-merge (authenticated Atlas can't be
  verified on a Vercel preview).

## Decisions

- **Hover richness = Standard card** (doctype + edge-count + title + 2-line snippet + click hint).
  Rejected: metadata-only (loses the snippet the map's depth earns) and full ResourcePeek-on-hover
  (heavy for a transient hover; duplicates the rail).
- **Excerpt placement = read-first** (directly under the title). Rejected: after-meta (buries the
  node's primary content below its relationships).
- **Bare-leaf excerpt = graceful degrade, one mechanism.** The excerpt rides only in the neighborhood
  subgraph; a truly-isolated selected node shows title/meta/history without an excerpt block. Chosen
  over a supplementary per-node excerpt read (a whole new endpoint for a rare edge-less case).
- **History expansion = generic key/value + inline payload + kept actor line.** Rejected: raw-JSON
  expansion (technical, leaks internal shape) and per-event-type bespoke renderers (prettier, but
  per-kind code for marginal gain). Payload is delivered **inline** in the trail read (no click-time
  round-trip), justified by its replay-sufficiency and the trimming of heavy `resource_created`
  blocks.
- **Trail read extension over a new per-event endpoint.** The payload is a single self-contained
  jsonb already on the rows the trail joins; a standalone `event_id` read would need a re-derived
  visibility gate (`kb_events` has no resource FK) for the same data.

## Deferred (not rejected)

- **Tier-1 territory-member hover snippet** — would need `first_chunk`/excerpt on
  `graph_region_members` too; out of this beat's Tier-2 scope.
- **Lazy/hybrid payload loading** — only if real payloads prove too large despite the `blocks[]`
  trim; the inline approach is the default.
- **"you"-detection on the actor line** — showing `by you` for the session profile needs the client
  to compare the actor's profile to the session; ship the humanized entity/profile name first.

## Out of scope (this beat)

The other C3.1 items on task `019f2fbe`: B1–B4 (nav bugs), W1 (wayfinding), L1–L3 (layout), G1–G2
(legibility). Separate beats.
