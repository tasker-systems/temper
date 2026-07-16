# Vault resource view — rebuild

**Date:** 2026-07-16
**Status:** design, approved
**Scope:** `packages/temper-ui` (all but one line), `crates/temper-substrate` (one line)

## The problem, as the data states it

The vault resource page renders a body and five hardcoded chips. It was built when `task`
and `session` were most of what existed, and the schema has grown out from under it. Three
things are actually wrong, and only one of them is the one we expected.

### It is not doc-type gating

The suspicion going in was that a doc type outside the fourteen named `temper-workflow`
types fails to render. It doesn't. The route never reads `params.doc_type`:

```ts
// vault/[owner]/[context]/[doc_type]/[ident]/+page.server.ts:10-16
const id = parseRef(params.ident);
resource = await apiGet<ResourceRow>(`/api/resources/${id}`, accessToken);
```

Resolution is trailing-UUID-only; the pretty segments are presentation. There is no
whitelist, no param matcher (`src/params/` does not exist), and no per-type branching
anywhere on the vault path. Production carries four doc types with no Rust schema —
`kernel_landmark` (22), `cogmap_charter` (4), `resource` (4), `build` (1) — and every one
of them would render fine if it were reachable.

### It is homing

**533 of 2,330 active resources — 23% — have no vault URL at all.** The route shape
presumes a context home, and the href builder concedes it:

```ts
// vault-url.ts:67-70
export function resourceHref(row: ResourceRow): string | null {
	if (!row.context_owner_ref || !row.context_slug) return null;
	return `/vault/${row.context_owner_ref}/…`;
}
```

`VaultGrid` lists those rows and silently no-ops on click. The stranded doc types are
exactly the knowledge ones:

| fact | concept | commitment | decision | memory | concern | principle | kernel_landmark | theme | question | domain |
|---|---|---|---|---|---|---|---|---|---|---|
| 114 | 102 | 75 | 59 | 44 | 40 | 24 | 22 | 17 | 16 | 7 |

"A doctype that doesn't match a named type won't render" is the right symptom read off the
wrong cause. Every one of those is cogmap-homed. The correlation is not a coincidence —
cogmaps home distilled nodes, and distilled nodes are what the knowledge doc types are.

### Both meta tiers arrive and are typed away

A repo-wide grep for `managed_meta|open_meta` outside `types/generated/` returns zero hits.
There is no frontmatter rendering anywhere. But the reason is subtler than "the page didn't
ask for it" — it asks, and throws the answer away at the type boundary:

```ts
// +page.server.ts:10-16
resource = await apiGet<ResourceRow>(`/api/resources/${id}`, accessToken);
//                     ^^^^^^^^^^^ the server sent ResourceDetail
```

`GET /api/resources/{id}` returns **`ResourceDetail` — the row plus both tiers**
(`show_detail_select` → `get_meta_select` → `readback::meta`). The tiers are on the wire on
a call the page already makes; annotating the response as `ResourceRow` discards them
silently, because excess-property checks don't apply to a type assertion on a fetch result.

> **Do not reach for `/content`'s tiers.** `ContentResponse` declares
> `managed_meta: ManagedMeta | null` and `open_meta: JsonValue | null`, and
> `get_content_select` hardcodes **both to `None`** (`substrate_read.rs:292-297`). They are
> dead fields that have never carried a value. The type says they might; the server says
> they never do. This is worth a separate cleanup — a nullable field that is structurally
> always null is a trap for the next reader, and it already cost this design a wrong turn.

## What we're building

One home-agnostic resource page that renders every resource, shows its full property set as
a document masthead, and carries an Atlas-convention rail of event history and edges.

## Decisions

### D1 — One home-agnostic route

Add `/vault/r/<ref>`, resolving by UUID alone. The existing context route becomes a 303
alias to it.

This follows the invariant already in CLAUDE.md: *addressing is by ref*, resolution is
trailing-UUID-only. The pretty segments were always presentation — the route just never
admitted it. Home stops being a routing precondition and becomes a rendered fact (a chip).

> SvelteKit gives static segments priority over dynamic ones, so `/vault/r/[ident]` wins
> over `/vault/[owner]/[context]`. It's not ambiguous anyway: owner refs are `@handle` or
> `+team-slug`, so a bare `r` can never be one.

### D2 — Properties render as one flat set, ordered by convention

The managed/open split is not a storage fact. Both tiers are a **read-time projection** of
one flat `kb_properties` store, re-split by a Rust `const` (`keys.rs:42`, ten keys). A
generic property view is closer to the truth than the two-tier DTO is.

Ordering is a UI-side table, not a schema fetch:

1. `doc_type` first
2. the ten managed `temper-*` keys, fixed order
3. everything else, alphabetical

No schema fetch. `describe_doc_type` is MCP-only and would need a new HTTP route, and it
buys enum/description decoration we don't need yet — while a schemaless type
(`kernel_landmark`, `cogmap_charter`) needs the fallback path regardless. Same code path for
every doc type, schema'd or not, which is the property that makes "all resources render"
true by construction rather than by enumeration.

### D3 — Promote `palette.ts` to tokens

There are three visual dialects, not two:

| | Atlas | Vault | `app.css` / design-system |
|---|---|---|---|
| Styling | scoped `<style>`, hardcoded hex, `px` | inline Tailwind `zinc-*` | `--quiet-*`, `.t-*`, `.ed-*` |
| Serif | Georgia | — | Source Serif 4 |
| Body | `#c9d1d9` | `zinc-300` | `--parchment` |

Atlas bypasses the app's own token system, and `palette.ts` already claims to be the sole
source of doc-type hues ("*This module is the ONLY place Atlas hues are defined*"). Make
that true: emit the hues as CSS custom properties alongside `--quiet-*`, and have both Atlas
and vault consume one layer. Atlas's Georgia/`#c9d1d9` become `--font-serif`/`--parchment`.

The alternative — lifting Atlas's scoped styles verbatim — is faster and entrenches the third
dialect on two surfaces instead of one.

### D4 — Layout: editorial masthead

Properties sit under the title, full width, before the prose. The rail holds history and
edges only.

Properties are identity, so they belong with the title; the rail is for what *happened to*
the resource. A 340px rail is where a property set goes to be ignored. Rejected: Atlas-parity
(everything in the rail — optimizes the Atlas→vault trip at the expense of the page's actual
job, which is reading the body) and dual-rail (right only if resources routinely carried more
properties than they do; the median is ~5).

### D5 — Non-scalar values: scalar inline, structured expands

Property values are arbitrary JSONB. A scalar renders as one row; a non-scalar collapses to a
type summary (`⌄ {5 keys}`, `› [3]`) and expands to a nested sub-table. One key = one row,
always.

This argues *against* the obvious reuse. `flattenPayload` (`payloadRows.ts`) already renders
arbitrary JSON as dot-paths, is tested, and exists for precisely this problem — but it
flattens `facet` into five sibling rows sitting at the same visual level as `date` and `tags`,
so the property set stops reading as the resource's key set. The recursive renderer keeps the
document reading; the History rail can adopt it later, at which point the reuse returns from
the other direction.

This is not a margin case: `facet` is on 291 resources, **all cogmap-homed**, and facets are
always objects. This treatment *is* the property view for the population D1 rescues.

### D6 — The rail: history, edges, home

Both endpoints already ship. No backend work.

| Section | Source | Notes |
|---|---|---|
| History | `GET /api/graph/elements/node/{id}/trail` → `EventTrail` | `trail.ts` + `eventSummary.ts` are pure and tested |
| Edges | `GET /api/resources/{id}/edges` → `GraphEdgeRow[]` | Already peer-denormalized — no subgraph load |
| Home | `ResourceRow` | Context *or* cogmap — D1's fix made visible |

Edges show `weight` and `polarity`, not just peer + kind: they're carried, and an author who
set a weight meant it.

Empty sections vanish, following Atlas — except History, which says "No recorded history."

**Lineage is out.** `GET /api/resources/{id}/lineage` exists, but `derived_from` is itself an
edge kind, so it would render twice.

### D7 — Facets render once, newest-wins

**We are not adding a `facets[]` field.** An earlier draft of this design proposed one, on the
premise that 13 resources are richly multi-faceted and the projection collapses them. The data
refuted it: all 13 are stale accumulation.

| Resource | Facet 1 | Facet 2 |
|---|---|---|
| `[question] What doc-type vocabulary…` | `status: open` | `status: resolved` |
| `How should temper-ui's graph…` | `status: open` | `status: answered` |
| `Team read + member lifecycle…` | `status: shipped-draft` | `status: superseded` |
| `SQLA chunk 7…` | `severity, decision_required` (w=0.85) | `resolved: true, resolution: …` (w=1) |
| `Admin-as-events…` (+4 more) | `as_of + as_of_source` (w=1) | `as_of` (w=0.95) |

Every pair is one logical facet asserted twice — an older value and its update. `facet_set`
appends by design (`_project_property_asserted`), so an *update* leaves the superseded row
live. Rendering both as weighted chips would print "open" and "resolved" side by side as if
both were true.

**The live bug this surfaced:** `readback::meta`'s query has no `ORDER BY` (`readback/mod.rs:241-245`)
and map-inserts per row, so which facet survives the collapse is whatever order Postgres
returns. A resolved question can render as open, and differently between two page loads.

The fix is one line:

```sql
 SELECT property_key, property_value
   FROM kb_properties
  WHERE owner_table = 'kb_resources' AND owner_id = $1 AND NOT is_folded
+ ORDER BY created, id
```

Ascending + last-write-wins on the map insert = newest wins, which matches author intent in
all 13 cases. Verified against production: `ORDER BY created` disambiguates every pair with
zero ties; `id` is a uuidv7 tiebreak for same-transaction writes.

Bundled here rather than extracted, per the repo convention: this work surfaced it, and the
property view is unshippable on a non-deterministic read.

**Out of scope, filed separately:** facet supersession itself. `facet_set`'s append semantics
are genuinely right for a multi-valued facet, and these writers used it as an upsert — at read
time the two cases are indistinguishable. That's a substrate/domain fix with real blast radius,
not something the vault page should paper over.

## Architecture

```
routes/(app)/vault/r/[ident]/
  +page.server.ts        parallel: resource, content, trail, edges
  +page.svelte           layout A composition

routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/
  +page.server.ts        → 303 /vault/r/<ident>

lib/components/vault/
  PropertySet.svelte     masthead block; ordering convention (D2)
  PropertyValue.svelte   recursive scalar/structured renderer (D5)
  HomeChip.svelte        context or cogmap (D1/D6)
  EventHistory.svelte    extracted from TrailRail §history
  EdgeList.svelte        GraphEdgeRow[] → peer links

lib/properties.ts        ordering table + managed-key set; pure, unit-tested
```

Each unit is independently testable. `properties.ts` is pure. `PropertyValue` recurses on one
JSON value with no app deps. `EventHistory` takes `EventTrail | null` and nothing else — its
only current coupling in TrailRail is `--hue`, which D3 turns into a token.

`resourceHref` loses its `null` branch and returns `/vault/r/${row.id}` for every row, which
un-breaks `VaultGrid` and `CommandPalette` click-through for the 533.

## Data flow

```
+page.server.ts
  ├─ GET /api/resources/{id}          → ResourceDetail (row + both meta tiers)
  ├─ GET /api/resources/{id}/content  → ContentResponse.markdown  (markdown only)
  ├─ GET /api/graph/elements/node/{id}/trail → EventTrail
  └─ GET /api/resources/{id}/edges    → GraphEdgeRow[]
        ↓  four parallel reads, one await
  mergeProperties(managed_meta, open_meta) → ordered (key, value)[]
        ↓
  PropertySet → PropertyValue*        EventHistory + EdgeList + HomeChip
```

No new endpoint and no extra round-trip: the first two calls are what the page already
makes. The only change is typing the first one honestly.

**The response type is hand-written, deliberately.** `ResourceDetail` uses `#[serde(flatten)]`,
which `ts-rs` cannot codegen (noted in `openapi.json`). Rather than restructure the Rust DTO
to suit the generator, compose the generated parts in TypeScript:

```ts
type ResourceDetail = ResourceRow & {
	managed_meta: ManagedMeta | null;
	open_meta: JsonValue | null;
};
```

Both halves stay generated; only the join is by hand. This does not violate *shared types at
boundaries* — no shape is being re-declared, it is being composed.

The merge is client-side and lossless *because* D7 made the projection deterministic — the
two tiers are a re-split of one flat set, so merging them back is a reconstruction, not a guess.
The D7 fix reaches this page precisely because `show_detail_select` calls `get_meta_select`,
which calls `readback::meta` — the same function whose unordered query is the bug.

## Error handling

- **Unknown doc type** — no fallback needed; nothing branches on it. Hue falls back to
  `FALLBACK_HUE` (`palette.ts:47`), already the Atlas behaviour.
- **Trail/edges failure** — the rail sections degrade independently. A 500 on edges must not
  blank the body. Do **not** copy `vault/search/+page.server.ts:15`'s `.catch()` → empty, which
  renders an API 500 as "no results".
- **Missing resource** — 404 passthrough, unchanged.
- **Event row keys** — key on `row.id`, never a composite. `trail.test.ts:46-58` documents the
  `each_key_duplicate` crash that killed the panel when two batch events shared actor+time+kind.

## Testing

| Layer | What |
|---|---|
| Unit (`properties.ts`) | ordering: `doc_type` first, managed in fixed order, open alphabetical; unknown keys sort to open |
| Unit (`PropertyValue`) | scalar → one row; object → summary + expansion; array → indexed; null/empty |
| Unit (Rust) | `readback::meta` returns the newest of two same-key rows — the D7 regression guard |
| Component | a schemaless doc type renders; a cogmap-homed resource renders; empty trail says "No recorded history" |
| e2e | `/vault/r/<uuid>` resolves for both homes; the context route 303s |

The Rust test is the one that matters most — it is the only guard on a bug that was invisible
because it was non-deterministic.

## Risks

- **D3 touches Atlas.** Re-tokenizing scoped styles risks visual regression on a surface that
  is not this design's subject. Mitigate by landing tokens first, additive, with Atlas
  consuming them in a separate commit that changes no rendered value.
- **The 533 become reachable for the first time.** They have never been rendered. Expect
  content surprises (long titles, absent bodies, `cogmap_charter`'s multi-block shape).
- **Local dev has one resource.** The dev DB cannot exercise any of this; `bun run dev` renders
  an empty vault, and pointing at prod fails on auth (the device-flow token doesn't carry
  through the UI's server-side proxy). Component tests carry the load for this PR. The durable
  fix is `/dev/vault` (follow-on 2), mirroring the `/dev/atlas` harness that exists for exactly
  this reason — Vercel previews can't carry Auth0 either.

## Follow-on tasks

1. **Facet supersession** — task `019f6d08-2b55-7ee0-b9ac-1959cf4d736b`. `facet_set`
   append-vs-upsert; the 13 stale rows need a decision (fold on same-identity assert? key
   facet identity on `node_label`?) and a backfill. Carries the full evidence table.
2. **`/dev/vault` harness + local fixture data** — task `019f6d08-8b33-7f30-a438-8487261d5f23`
   (goal: Maintenance). Mirrors `/dev/atlas`, which solves this exact problem for the Atlas.
   This is the answer to the local-dev risk below, and it is not a prerequisite — but every
   future vault change pays the same tax until it lands.
3. **`ContentResponse`'s dead meta fields** — `managed_meta`/`open_meta` are structurally
   always `None`. Drop them, or populate them. Either beats a nullable field that is never
   non-null. Not yet filed.

## Design-system artifact

`design-system/preview/comp-resource-view.html` — the agreed render, at fidelity, with real
content and a doc-type tint switcher (including `kernel_landmark` → `FALLBACK_HUE`).
Implementation grounds there; this document explains why it looks that way.
