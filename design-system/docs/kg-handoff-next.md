# Knowledge graph — production handoff, session 2

**Written:** 2026-04-20, after the PR 1–5 Cytoscape bundle landed on `main`.
**Supersedes as entry point:** `kg-handoff.md` (that's the original design brief; this is the "pick up from here" note).
**Prototype reference (unchanged):** `design-system/ui_kits/app/{KnowledgeGraph,ResourcePeek,graph-data,graph-content}.jsx`.

---

## What's on `main`

The five-bundle program from `kg-handoff.md` landed as six commits:

```
PR 5  feat(graph): zoom tiers + label culling
PR 4  feat(graph): hover gradient + emphasis classes
PR 3  feat(graph): breadcrumb traversal (peekNodeId → peekTrail[])
PR 2  feat(graph): right-docked ResourcePeek panel
  +   fix(setup-claude-web): install postgresql-N-pgvector in native fallback
PR 1  feat(graph): Cytoscape renderer swap + session-aware subgraph contract
```

In use-facing terms:

- `/vault/[owner]/[context]/graph` renders a `cytoscape.js` + `cytoscape-fcose` force layout with typeset (word-as-node) rendering. D3 is gone.
- Sessions are no longer graph nodes. Each remaining node carries `session_count` and shows a `⌊N⌋` glyph when ≥ 1.
- Clicking a node opens a 420 px right-docked peek: doctype marker, title (italic for aggregators), session glyph, neighbors list (participants first, then aggregators), metadata block, and `OPEN RESOURCE →`.
- Neighbor row-click drills deeper, appending to a breadcrumb trail; trail collapses `first › … › penult › current` at 5+ depth; clicking a crumb slices back and recenters the camera 380 ms.
- Hovering any node lifts its incident edges in the source hue at 1.1 px opacity 1; non-neighbors fade to 0.35 opacity; non-incident edges drop to 3 % alpha; 180 ms transitions.
- Below 0.5 zoom, participant labels cull to colored tick marks; above 1.2 zoom, dated nodes show date strips under their labels. Mid band is steady state.

**Test surface:** 78/78 vitest, 15/15 `graph_subgraph` integration scenarios, svelte-check clean, clippy `--all-targets --all-features` clean.

---

## Architecture map

### Server

- `crates/temper-core/src/types/graph.rs` — `GraphNode` fields: `id / slug / title / doc_type / aggregator / edge_count / session_count`. `is_aggregator(DocType) -> bool` is the classification helper.
- `crates/temper-api/src/services/graph_service.rs` — `aggregator_subgraph()` excludes `DocType::Session` and computes `session_count` via a correlated subquery. Two round-trips: node query, then edge query bound to the resolved id set.
- `crates/temper-api/tests/graph_subgraph_test.rs` — the integration fixture harness.
- `scripts/seed-graph-fixtures.sql` — 10 scenarios (happy path, tier-3 reach, diamond overlap, cross-owner leak, etc).

### UI pure modules (`packages/temper-ui/src/lib/graph/`)

| Module | Role |
|---|---|
| `derive.ts` | `label` / `dateStrip` / `fullTitle` rules from slug + title |
| `elements.ts` | `GraphNode[]` + `GraphEdge[]` → Cytoscape `ElementDefinition[]` |
| `layout.ts` | fcose config (prototype-pinned tunings) |
| `styling.ts` | Cytoscape stylesheet, palette, emphasis + tier selectors |
| `peek.ts` | Neighbors list derivation (participants → aggregators) |
| `trail.ts` | Breadcrumb resolution + collapse (threshold = 5) |
| `tiers.ts` | Zoom-tier classifier (< 0.5 / 0.5–1.2 / > 1.2) |
| `navigation.ts` | `resourceHref(owner, context, node)` |
| `adjacency.ts` | Symmetric neighbor index (utility, used by peek-side code) |

### UI components (`packages/temper-ui/src/lib/components/graph/`)

- `KnowledgeGraph.svelte` — the only place Cytoscape runtime lives. Mounts `cy`, registers `fcose`, wires `tap` / `mouseover` / `mouseout` / `zoom` handlers. Owns `peekTrail` state.
- `ResourcePeek.svelte` — the right-docked panel. Pure view — no `cy` access, consumes nodes/edges/trail as props.

### Route

`packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/graph/` — thin `+page.server.ts` loads the subgraph; `+page.svelte` passes it into `KnowledgeGraph`.

---

## Deferred work, ordered by readiness

### 1. Body-preview excerpts in the peek

*Decision already made (session 1): option 1 — skip in the bundle, ship as a focused follow-up.*

**Where to add the field:** `GraphNode` in `temper-core/src/types/graph.rs` — `pub excerpt: Option<String>`.

**Where to compute it:** `graph_service::aggregator_subgraph`. First-paragraph-≤-280-chars from the resource body. Confirm where body text lives (likely a chunk join or `kb_resources.body` directly — `resource_service.rs` is the reference for body fetching elsewhere).

**Where to render it:** `ResourcePeek.svelte`, below the metadata rows. Prototype reference: `design-system/ui_kits/app/ResourcePeek.jsx:342-356`. Mono-cap `EXCERPT` label, then parchment serif 14 px / 1.6 line-height paragraph.

**Don't forget:** regenerate TS types (`cargo make generate-ts-types`) and the sqlx cache (`cargo sqlx prepare --workspace -- --all-targets`). Sessions are already excluded from nodes so no extra filter is needed.

### 2. Task stage tags in the detail zoom tier

*Decision already made (session 1): stubbed; revisit after PR 5. Stage lives in managed_meta — task frontmatter `status` — not on the resource row.*

**Where to add the field:** `GraphNode.stage: Option<String>`. Populated only for `DocType::Task` rows via a join on the managed-meta table.

**Where to render it:** `styling.ts` gets a new rule, something like `node.tier-detail.type-task[stage]`, and `elements.ts` surfaces `stage` in the node data. Visual treatment: small mono caps 8.5 px, faded, below the existing label-plus-date rendering. May require a third multiline variant in `labelWithDate` (e.g. `labelWithDateAndStage`).

**Don't forget:** re-run `edge_count_reflects_total_not_subgraph` and `session_count_reflects_incident_sessions` after the node SQL changes — the row shape drifts.

### 3. Visual chrome parity (prototype has three pieces we haven't ported)

Small but visible. All live on `design-system/preview/kg-scene-v2.html` as the canonical visual spec.

- **`VIEW` toggle at top-left** — `structural` / `meta-doc`. In our codebase it's a stub until the Jaccard work lands. Add it as a `<ModeToggle>` component that renders inside the graph container but is wired to a no-op selector for now.
- **Standalone legend at top-right** — doctype color swatches + `⌊N⌋ SESSIONS · ANNOTATION, NOT EDGE` marker. Pure presentational.
- **`CONTEXT {name}` watermark at bottom-left** — faint italic serif centre-weight, 88 px. Pure presentational, reads the context from the page data.

None are required for the graph's core function — they're polish that moves the production view closer to the prototype's settled visual language.

### 4. PR 6 — Jaccard meta-doc mode toggle

This is *not* ready to start. Per the original `kg-handoff.md`:

> Deferred until the structural mode has shipped and been used in anger for ~2 weeks; until we've seen which aggregators users actually click into (inform which emergent-edge view is most valuable); and until a decision on precompute vs on-the-fly Jaccard.

When it's time, the scoping doc goes in `docs/superpowers/specs/`. Rough shape: server computes aggregator-to-aggregator edges by shared-member Jaccard similarity, exposed via a `mode` query param on `/api/graph/subgraph` or a new endpoint; client adds a `mode: 'structural' | 'meta-doc'` prop to `KnowledgeGraph.svelte` and swaps elements accordingly.

---

## Contracts and gotchas worth re-surfacing

**Session-exclusion is load-bearing.** The server drops `DocType::Session` from the node set before returning and computes `session_count` per remaining node. If a future change re-exposes sessions as nodes, two fixture tests will fail: `sessions_excluded_from_nodes_and_edges`, `session_count_reflects_incident_sessions`. Don't regress that without a fresh R11 review.

**`aggregator` is server-derived.** `is_aggregator(DocType)` in `temper-core/src/types/graph.rs` classifies `Goal | Concept | Decision` as aggregators. A new aggregator doctype means updating that function **and** `aggregator_flag_set_correctly`.

**Route shape.** `/vault/[owner]/[context]/graph` is canonical. `kg-handoff.md` informally wrote `/vault/<ctx>/graph` — treat `<ctx>` as shorthand for `[owner]/[context]`. Do not move to a context-only route without a cross-context plan.

**Transition durations.** Cytoscape's `transition-duration` is per-element, not per-property. We use one value (180 ms, `EMPHASIS_TRANSITION_MS`) for both emphasis and zoom-tier fades. That's a 40 ms deviation from `kg-handoff.md`'s 220 ms zoom-fade spec; documented inline in `styling.ts`. If you need per-property durations, you're building a second stylesheet.

**cytoscape-fcose ships no types.** The only `@ts-expect-error` in the UI is at the `import fcose from 'cytoscape-fcose'` line in `KnowledgeGraph.svelte`. Leave it.

**Dev environment.** Postgres runs native on port **5432** in web sandboxes (not Docker/5437). `setup-claude-web-full.sh` now installs `postgresql-16-pgvector` automatically (commit `d17435c`). Override in tests: `DATABASE_URL=postgresql://temper:temper@localhost:5432/temper_development cargo nextest run …`.

---

## Verification commands

Before committing anything in the next session:

```bash
# UI gate
cd packages/temper-ui && bun run test && bun run check && bun run build

# Server gate
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check

# Integration (requires running Postgres)
DATABASE_URL=postgresql://temper:temper@localhost:5432/temper_development \
  cargo nextest run -p temper-api --features test-db --test graph_subgraph_test
```

Expected at this handoff: UI 78/78, integration 15/15, clippy clean, fmt clean.

---

## One-line to orient a fresh session

> Read `design-system/docs/kg-handoff.md` for the design brief, then this file for where production is, then pick up at deferred item #1 or #2 — both are ½-session pieces of work that cleanly add to the shipped bundle.
