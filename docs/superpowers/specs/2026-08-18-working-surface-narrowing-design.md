# The working surface can narrow — design

Task: [The working surface can narrow, and the nav can be read — the table exposes the filters the API already serves](./01a01522-16e4-7a70-85cc-c69d849427e5)
Goal: [Work is located, followed, asked about, and changed from the web surface — by a reader who operates no agent](./01a01520-3ea7-7893-8669-64039f3ae5ba)

Witnesses the goal clause `work-is-locatable-without-a-question`. The nav arm
(`membership-is-legible-at-both-levels`) is deliberately a **separate deliverable** and is not
designed here.

---

## 1. What is actually on disk

Evidence first; the proposal below cites it. Every claim in this section was read at authoring,
not carried from the prior spec.

### The read door

`ResourceListParams` (`crates/temper-workflow/src/types/resource.rs:64`) accepts
`kb_doc_type_id · context_ref · doc_type_name · owner · q · stage · status · goal · tags ·
cogmap_ids · sort · order · limit · offset · sections`.

`ResourceFacets` (`crates/temper-workflow/src/types/resource.rs:143-145`) is a single field:

```rust
pub struct ResourceFacets {
    pub doc_type: std::collections::HashMap<String, i64>,
}
```

`ResourceListResponse::new` (`resource.rs:189-213`) derives `returned` from the page itself and
`truncated` as `offset + returned < total`.

### How the page is actually cut

`filtered_visible_page` (`crates/temper-services/src/backend/substrate_read.rs:122`) builds one
SQL statement, and **three properties of it drive this entire design**:

1. **There is no SQL `LIMIT`.** `fetch_all` returns *every* matching row; `total`, the facet
   histogram, and the page slice are all computed in Rust afterwards
   (`substrate_read.rs:308-328`). The row walk that builds the histogram already touches
   everything.
2. **The `wp` pivot is already joined.** `LEFT JOIN kb_resource_workflow_props wp` is present for
   the `stage`/`status` filters and the `stage`/`seq` sorts, but the SELECT list asks only for
   `r.id` and `doc_type_name`.
3. **`doc_type_name` is filtered in SQL, before the histogram is built**
   (`AND ($3::text IS NULL OR dt.property_value #>> '{}' = $3)`). This is the sole cause of the
   defect in §2.

The query is assembled with `format!` and executed through runtime `sqlx::query()`, documented in
place as "the documented runtime-`query` exception (dynamic ORDER clause), not a static macro."
**No `.sqlx` cache regeneration is required for anything in this design.**

### The mismatched-filter behaviour

`stage` is task-only and `status` is goal-only, but **the door does not refuse a mismatch**.
`substrate_read.rs:235` is a plain `AND ($4::text IS NULL OR wp.stage = $4)`, so
`doc_type_name=research&stage=done` returns zero rows. `status` is validated against the goal
schema's enum (`validate_goal_status`, `schema.rs:367`), but that validates the *value*, never the
*pairing*. **The refusal has to live in the UI or it does not exist.**

### The surface

- `VaultGrid.svelte:34-40` — five hardcoded columns; no filter of any kind.
- `VaultGrid.svelte:56-63` — already reads `stage`/`seq` out of `managed_meta` under their
  canonical `temper-*` names.
- `VaultGrid.svelte:73` — re-derives `hasNext` as `offset + limit < total` rather than reading the
  envelope's own `truncated`.
- `FacetChips.svelte:11` — single-active, `doc_type_name` only.
- `vault/search/+page.server.ts:21-32` — synthesizes `truncated: false` **on a failed fetch**, with
  a comment stating that a consumer reading `truncated` would turn it into a lie.
- The three mounting pages are 27, 24 and 29 lines of near-identical wrapper.

### Incumbents this design must not fork

| Concept | Incumbent | Consequence |
|---|---|---|
| URL route building | `lib/vault-url.ts:69` (`resourceHref`) | Do not fork. Untouched here. |
| URL filter state | `lib/graph/atlas/nav.ts:158-181` — pure `parse*`/`build*Url` over `withParams` | Extend the idiom; do not invent a second one. |
| Managed-key render order | `lib/properties.ts:21` (`MANAGED_KEY_ORDER`), mirroring `keys.rs:42` | Column order defers to it. |
| Multi-value query params | `tags` and `cogmap_ids` are CSV, because the list GET rides serde_urlencoded which cannot encode sequences | Multi-select `doc_type_name` conforms to CSV. |
| Per-kind field vocabulary | `crates/temper-workflow/schemas/{task,goal}.schema.json` | The source of truth for the drift guard. **Not** `schema.rs:830/843`, which are test fixtures. |

---

## 2. The defect that redirected this design

**Facets collapse.** Because `doc_type_name` is applied in SQL before the histogram is built, the
histogram describes the *already-doc-type-filtered* set. Selecting a chip therefore leaves exactly
one chip on screen. The reader can toggle it off to recover, but cannot see or reach the
alternatives, and a lone `task 412` reads as though `task` is the only kind there is — which is
`no-partial-view-reads-as-complete` in the facet dimension.

This is pre-existing behaviour, and it invalidates the naive UI-only design: a filter bar built on
a histogram that collapses when used is a filter bar that lies as soon as it works.

---

## 3. Decisions

### D1 — The read door widens. Scope reversed, deliberately.

The task body says "This is UI-only. No API work," with an escape hatch for the multi-select
question. **That framing is superseded.** It was a hypothesis about cost written before the door's
implementation was read, and §1 falsifies it: the server already walks every matching row, and the
`wp` join is already present, so the facet work is nearly free where it belongs — and expensive,
duplicated, and dishonest in the UI.

Recorded per the task's own instruction that a widening is "a decision to record, not a quiet
edit."

### D2 — `doc_type`, `stage` and `status` predicates move from SQL into the Rust filter step.

**AMEND** (`substrate_read.rs:234` doc_type, `:235` stage, `:247` status; authorized by D1).

A dimension that publishes a histogram must exclude its own predicate from that histogram.
Pagination is *already* Rust-side, so this is consistent with the existing implementation rather
than a new pattern. `tags`, `context_ref`, `owner`, `q`, `goal` and `cogmap_ids` **stay in SQL** —
none publishes a facet, so none can be inconsistent with one.

`total` is computed after all filtering and keeps its documented meaning: "the FILTERED match
count — every row the filters admit, before `limit`/`offset`."

### D3 — `ResourceFacets` gains `stage` and `status`.

**EXTEND** (`resource.rs:143-145`; authorized by D1). Additive — existing consumers ignore new
fields. The SELECT gains `wp.stage` and `wp.status`; the join already exists.

### D4 — `doc_type_name` accepts CSV, answering the task's open question (1).

**AMEND** (`resource.rs:64`; authorized by D1). The task offered three resolutions — narrow the
ask, filter within the fetched page, or widen the API. **The API widens.** Filtering within a
bounded page was never available (it is `no-partial-view-reads-as-complete` by construction), and
narrowing the ask was a concession priced against a cost that turned out not to exist: once
doc-type filtering is Rust-side, multi-select is a set membership test.

CSV **CONFORMS** to `tags`/`cogmap_ids` and their documented reasoning. A single value remains
valid, so the change is backward-compatible on the wire.

### D5 — Contract change to record: `doc_type` facets become pre-filter.

The one genuinely breaking-ish change. A CLI or MCP caller that filters by doc-type and reads
`facets.doc_type` will now see the full distribution rather than its own selection. This is the
standard faceted-browse semantic and the more useful one, but it is a changed meaning for an
existing field, not an addition. Consumers: the MCP list tool
(`temper-mcp/src/tools/resources.rs:413,1690`), a synthetic CLI path
(`temper-cli/src/commands/resource.rs`), and the three UI page servers.

### D6 — Procedural revelation, not disabled controls.

A filter the visible kind cannot carry is **absent**, not greyed. Applicability is *"the fully
filtered set is exactly one kind"*.

**Deriving that predicate is subtler than it looks, and D2 is what makes it so.** Once the
`doc_type` histogram is computed *excluding* its own predicate, it no longer describes the fully
filtered set whenever a doc-type is selected — so "exactly one key in `facets.doc_type`" is wrong,
and would have been silently wrong only in the selected case. The correct predicate has two arms:

- **A doc-type selection exists** → the set is one kind iff the selection names exactly one value.
  (With CSV multi-select, two selected values means no revelation.)
- **No doc-type selection** → the histogram *is* the fully filtered set's composition, because
  excluding an absent predicate changes nothing. One key means one kind, so revelation fires when
  some *other* filter incidentally narrows to a single kind.

Both arms live in `vault-filters.ts` and are unit-tested, including the case that distinguishes
them.

Chosen over a permanently-present disabled control because the goal states the reader "is not
expected to hold the system's own vocabulary": a dimmed `Stage — tasks only` teaches the doc-type
model to someone who never asked for it.

### D7 — Columns are lean; `mode` and `effort` are never scanning columns.

`task → [temper-stage]`, `goal → [temper-status]`, everything else `→ []`. When the set is one
kind, the `Type` column drops (every cell would read the same word) and the kind's keys arrive.
Order defers to `MANAGED_KEY_ORDER`.

`temper-mode` and `temper-effort` are deliberately excluded. They are estimates recorded before the
work and revised during it, so a column or sort built on them ranks by a stale prediction while
presenting it as a property of the work. They stay reachable on the resource view through
`mergeProperties`.

### D8 — The `q` control is labelled "title contains".

`substrate_read.rs` implements `q` as `r.title ILIKE '%' || $7 || '%'`. Labelling it "search" is
the live `no-affordance-overstates-what-it-does` failure the goal already names. Costs nothing to
get right. **This does not touch the header control or the command palette** — see §6.

---

## 4. The work

### Arm A — the read door (Rust)

| Step | Tag | Cites |
|---|---|---|
| Move `doc_type`/`stage`/`status` predicates out of the WHERE into the Rust filter step | AMEND | `substrate_read.rs:235-244`, D1/D2 |
| Compute each of the three histograms over the set filtered by the *other* two (plus all SQL-side filters) | EXTEND | D2 |
| Add `wp.stage`, `wp.status` to the SELECT list | CONFORM | join already present, `substrate_read.rs` |
| Add `stage`/`status` maps to `ResourceFacets` | EXTEND | `resource.rs:143-145`, D3 |
| Parse `doc_type_name` as CSV | CONFORM | `tags`/`cogmap_ids` idiom, D4 |
| Regenerate ts-rs bindings, `openapi.json`, temper-rb, temper-ts | CONFORM | `generated-artifacts` skill |

No migration. No `.sqlx` regeneration (runtime `query()`).

### Arm B — narrowing (Svelte)

New `lib/vault-filters.ts` — pure: `parseFilters(url)`, `buildFilterUrl(base, patch)`, and the
applicability predicate. **EXTEND** of the `nav.ts:158-181` idiom.

New `lib/components/vault/FilterBar.svelte` — renders what the module decides; no logic.

Controls: **title contains** (`q`), **doc-type chips** (now multi-select), **context** (only where
the route does not already fix it), **tags**, and the revealed **stage**/**status**. Filters use
the door's own param names, so the page servers keep forwarding `url.searchParams` untouched —
verified safe: `ResourceListParams` carries no `deny_unknown_fields`, so UI-only URL state can
coexist.

Not built: `owner`, `goal`, `cogmap_ids`. YAGNI.

### Arm C — columns (Svelte)

New `lib/vault-columns.ts` — the kind→keys map and column derivation. **EXTEND** of
`properties.ts:21`.

`VaultGrid.svelte` takes `columns` as a prop instead of the hardcoded const, and reads the
envelope's `returned`/`truncated` instead of re-deriving `hasNext`.

**Drift guard:** a vitest reads `crates/temper-workflow/schemas/{task,goal}.schema.json` and fails
if the kind→keys map disagrees with the schemas. `MANAGED_KEY_ORDER` never got one; this map does.

### Arm D — shared chrome

New `lib/components/vault/VaultBrowser.svelte`: heading + filter bar + chips + grid. The three
pages collapse to thin mounts differing only in what the route fixes.

**Honesty fixes this forces:**

- `vault/search/+page.server.ts:21-32` — the synthetic empty envelope becomes a real error surface.
  Once the grid reads `truncated`, synthesizing `false` on a failed fetch is exactly the lie the
  field exists to prevent, as its own comment predicts.
- Heading caption reads as a filtered count when filters are active.
- Empty results name the filters that produced the emptiness and offer to clear them.

---

## 5. Verification

- Vitest on `vault-filters.ts`, `vault-columns.ts` and the schema drift guard — the incumbent
  pattern (`nav.test.ts`, `vault-url.test.ts`, `sidebar.test.ts` are all pure-module tests; there
  are no component tests to match). Written test-first.
- Rust: unit coverage for the three histograms under combinations of the three predicates,
  including the case that motivated this design — a doc-type filter must not shrink the doc-type
  histogram.
- `cargo make check`, `bun run check`, `bun run biome`.
- Codegen drift gates. Note they clear at different git stages: rb/ts drift on `git add`, ts-rs
  drift only after commit.

**Declared limit, not papered over:** there is no `/dev/vault` harness (only `/dev/atlas`), and
Auth0 callbacks are prod-only, so authed routing cannot be browser-verified on a Vercel preview.
Real verification is post-merge in prod, as with prior beats.

---

## 6. Out of scope

### Rejected — load-bearing, deliberately not doing

- **A second URL authority.** `lib/vault-url.ts` stays the single builder.
- **Full faceted-browse semantics.** Only the three dimensions that publish histograms exclude
  their own predicate. `tags`, `context_ref`, `owner`, `q` keep their SQL predicates. Considered
  and declined as a much larger restructure than the UI arm it would serve.
- **Client-side filtering of a fetched page.** Inside `no-partial-view-reads-as-complete` by
  construction.
- **`mode`/`effort` as columns, sorts or filters.** See D7.

### Deferred — later, not rejected

- **The nav arm** — `membership-is-legible-at-both-levels`. Next session.
- **Real search.** `/vault/search` and the palette both call `/api/resources?q=`, and the header
  reads "Search the vault…" while performing a title match. That is
  `the-question-asked-is-the-question-answered`, its own task. This design does not half-fix it and
  adds no control that worsens the overstatement.
- **Relationship navigation in the reader's terms** — `EdgeList.svelte` renders graph vocabulary.
- **The table⇄graph toggle** — sequenced behind the successor graph surface's route shape being
  named.
- **Facet counts on `tags` or `context`.** Not needed by any control here.

---

## 7. Declared holes

- **No exemplars.** The goal's judged cell — "can a reader who operates no agent get their work
  done here" — still has zero collected exemplars. This design is argued from the code and from the
  register, not from an observed reader. Unchanged by shipping it.
- **D5 has no consumer audit beyond a grep.** The four `ResourceFacets` consumers were located by
  search; none was exercised to confirm nothing depends on the post-filter meaning.
