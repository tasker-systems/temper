# Design spec: An anchor says how much of itself there is, and whether it has ever been clustered

**Status:** design ruled 2026-08-23 (Pete), not implemented. Precedes an implementation plan.
**Task:** `01a02ebd-c153-7d22-acb6-d9fdec1b0f16` — an `enables` task.
**Goal:** Every question to Temper is answered by a situated act — `019fbdb9-f287-79c0-aab6-efa0b1de12c8`
**Grounded against:** `origin/main` at `c115336f`, branch `jct/alpha-dominance-probe` at `0ac90dee`
**Sibling task, downstream:** `01a02ebe-a3d2-7ad2-81df-541924c00e36` (survey says what it did) depends
on this for the anchor-level facts.

---

## 1. What is wrong, in one sentence

`anchor_shape` returns a bare list of regions, so an anchor that has never been clustered and an
anchor whose every region is invisible to this caller are byte-identical `[]`, and the CLI help text
ships an explanation that is true for only one of them.

`crates/temper-cli/src/cli.rs:1079`:

> Empty means the context has not materialized regions yet — run `context materialize`.

That is one of at least four causes and the surface cannot tell them apart.

## 2. The read, as it exists

`anchor_shape` (`migrations/20260713000050_region_visible_member_count.sql:79-135`) is the single read
behind every shape surface. It returns six per-region columns and no anchor-level fact:

```sql
CREATE OR REPLACE FUNCTION anchor_shape(
    p_anchor_table  text, p_anchor_id uuid,
    p_principal_kind text, p_principal_id uuid,
    p_lens          uuid DEFAULT NULL)
RETURNS TABLE(region_id uuid, lens_id uuid, salience double precision,
              content_cohesion double precision, label text, member_count integer)
```

Everything downstream is a pass-through. `anchor_shape_select`
(`crates/temper-services/src/backend/substrate_read.rs:1306-1327`) is a pure row mapping;
`readback::anchor_shape` (`crates/temper-substrate/src/readback/mod.rs:1052-1085`) is a `query!` over
the six columns; both HTTP handlers return `Json<Vec<CogmapRegionRow>>`
(`crates/temper-api/src/handlers/contexts.rs:256`, `crates/temper-api/src/handlers/cognitive_maps.rs:197`);
both MCP views `serde_json::to_string_pretty(&rows)`
(`crates/temper-mcp/src/tools/cognitive_maps.rs:53`, `:626`).

**There is no LIMIT and no pagination anywhere on this path.** The handlers, MCP views and CLI pass
only `lens`.

### 2.1 A premise in the task body that disk does not support `[corrected 2026-08-23]`

The task body's first item asks for "the region population this principal can see — the denominator",
and its first acceptance criterion asks that two principals with different reach receive different
populations.

**On this door that criterion is already met, by `regions.len()`.** The member gate at
`migrations/20260713000050_region_visible_member_count.sql:125` —

```sql
AND (p_principal_kind = 'cogmap' OR seen.visible_members > 0)
```

— already drops every region a caller can see nothing in, and the read is unbounded, so the row count
*is* the visible population. A `population` field equal to `regions.len()` would be a redundant value
with nothing linking it to the thing it must agree with.

The denominator is genuinely missing on the **survey** path, which cuts to a funnel width — but survey
runs on `wayfind_scope_ids` / `top_regions` / `__temper_ungated_survey`, not on `anchor_shape`. That
is the sibling task's surface, not this one.

**Ruling `[2026-08-23, Pete]`: `population` is carried, and defined as all-lenses.** It is always
`>= regions.len()`. Without a `lens` the two are equal; with one, `population` is not derivable by
the caller and may exceed the row count — though it need not, since an anchor whose visible regions
all sit under the requested lens returns them all, and that is the ordinary case for an anchor
materializing a single lens. Equality is therefore not evidence that no lens was applied. What the
field buys is the case where it *does* exceed: a real denominator on this door rather than a
restatement, and a fraction for the lens-narrowed read to report.

## 3. The shape of the answer

### 3.1 The wire break is forced, and is accepted `[ruled — 2026-08-23, Pete]`

An empty array has nowhere to carry why it is empty. Satisfying the task at all means the response
stops being an array. This is a **breaking wire change** on `GET /api/contexts/{id}/shape` and
`GET /api/cognitive-maps/{id}/shape`, and it is taken deliberately, without a deprecation window and
without a compatibility route — the alternative (an additive second door carrying the anchor facts)
was rejected because a caller who meets `[]` at `context shape` would still get no cause there, which
fails the task's acceptance criterion that the distinction hold *at every door that serves shape*.

### 3.2 The type

New in `crates/temper-core/src/types/cognitive_maps.rs`, beside `CogmapRegionRow`, carrying the same
derive stack that type already carries at `:42-46` (`ts_rs::TS` with
`export_to = "cognitive_maps.ts"`, `utoipa::ToSchema`, `schemars::JsonSchema`, `Serialize`,
`Deserialize`, `PartialEq`, `Debug`, `Clone`):

```rust
pub struct AnchorShape {
    pub regions: Vec<CogmapRegionRow>,
    pub population: i32,
    pub emptiness: Option<ShapeEmptiness>,
    pub materialized_at: Option<DateTime<Utc>>,
}

pub enum ShapeEmptiness {
    NothingVisible,
    NeverClustered,
    UnreadableOrAbsent,
    LensNarrowed,
}
```

`CogmapRegionRow` is **untouched**. Nothing about a region changes; only what wraps it.

`DateTime<Utc>` matches `CogmapStaleness` (`cognitive_maps.rs:118-121`), which already carries
`materialized_at: Option<DateTime<Utc>>` on the wire — so the clock needs no new representation.

**Serde naming `[corrected while planning]`:** serde does **not** snake_case enum variants by
default — without an attribute, `NothingVisible` goes on the wire as `"NothingVisible"`. The enum
carries `#[serde(rename_all = "snake_case")]` **as its own attribute, below the derives**, matching
the house precedent for a ts-rs-exported wire enum at `crates/temper-core/src/types/api.rs:137-142`.

Own-attribute placement is not cosmetic: ts-rs discards an entire `#[serde(...)]` attribute when any
part of it is unsupported, which once silently dropped a `rename` at
`crates/temper-core/src/types/managed_meta.rs:49-55`. Keeping `rename_all` alone in its attribute is
what keeps a later addition from taking it down with it.

### 3.3 The five outcomes, and why two arms deliberately collapse

| Caller's situation | `regions` | `population` | `emptiness` | `materialized_at` |
|---|---|---|---|---|
| Readable, has visible regions | rows | > 0 | `null` | set |
| Readable, clustered, a lens filtered everything out | `[]` | > 0 | `lens_narrowed` | set |
| Readable, clustered, nothing came back (see below) | `[]` | 0 | `nothing_visible` | set |
| Readable, never clustered | `[]` | 0 | `never_clustered` | `null` |
| Cannot read it, **or** it does not exist | `[]` | 0 | `unreadable_or_absent` | `null` |

The last row is one arm on purpose. **Deny and absent must stay indistinguishable from each other**,
and they do. What the arm must not do is disclose a fact about an anchor the caller cannot read — so
it reports neither the population nor the clock, exactly as the task requires: *"the envelope is
absent or empty, not a population of zero that confirms existence."*

This discloses nothing new. Today such a caller receives `[]`, which already means one of
{denied, absent, empty, never-clustered}; tomorrow they receive `unreadable_or_absent`, which narrows
it to {denied, absent} — two states the caller cannot separate and already knew they were in. The
other three arms only ever reach a caller who passed `anchor_readable_by_profile`, for whom
"this anchor exists" is not a disclosure.

**`nothing_visible` carries two causes, and there will not be a fifth arm for them.** It is reached
by a readable, clustered anchor whose `population` is 0, and that happens for two different reasons:
the materialize formed **no regions at all**, or it formed regions and **every one of them is
member-gated away from this caller**. The arm does not say which, and no future arm may.

Splitting them is not a documentation nicety, it is a disclosure. "There are regions here, you just
cannot see into any of them" is a statement about resources beyond the caller's reach, and the
member gate exists to refuse exactly that class of statement —
`migrations/20260713000050_region_visible_member_count.sql:137`: *"a caller is never told how many
resources they cannot read."* A fifth variant would route around the gate rather than serve it, and
would leak the same fact the `member_count` blur and the drop-a-region-with-no-visible-members rule
(the `p_principal_kind = 'cogmap' OR seen.visible_members > 0` predicate in `anchor_shape`'s
`regs` CTE) each spend a predicate to withhold. **Rejected, permanently, not deferred.**

The formation cause is not exotic — it is routine, which is why this is written down. `materialize`
stamps the watermark before it knows whether anything clustered, so a first materialize of a small
anchor that forms nothing lands here immediately. The `cogmap` self-read arm makes the reachability
unambiguous: that predicate's left disjunct exempts the self-read arm from the member gate entirely,
so nothing can be hidden from it — and a zero-region anchor with a watermark still reports
`nothing_visible` there, by the precedence in §4.2.

The cost of the collapse is that the arm cannot be read as an access diagnosis, and every surface
that names it must say so. That is what those doors are for: `ShapeEmptiness::NothingVisible`'s doc
comment carries this argument, and the CLI, client and skill surfaces each carry the operational
half — *check the materialize before suspecting a grant*.

**`lens_narrowed` is a fourth arm added during design review `[ruled — 2026-08-23, Pete]`.** It falls
out of §2.1's ruling: once `population` is all-lenses, a lens-filtered read can return `regions: []`
with `population: 12` — an empty answer with no stated cause, which is the precise failure this task
exists to end. Naming it costs one variant and closes the hole rather than leaving it to inference.

### 3.4 Deny posture does not travel

`materialize_delta` denies with `NotFound` (`crates/temper-services/src/services/materialize_service.rs:47`):

```rust
.ok_or_else(|| ApiError::NotFound("cognitive map not found or not readable".to_string()))?
```

`anchor_shape` denies with zero rows, never an error
(`crates/temper-services/src/backend/substrate_read.rs:1301-1305`,
`crates/temper-substrate/src/readback/mod.rs:1038-1041`). Both are non-oracular; they are **not the
same rule**. §6 widens `materialize_delta`, and that widening must not import its 404 posture into the
shape envelope, nor the envelope's empty-arm posture into it.

## 4. One SQL function, one `vis` expansion

### 4.1 Why one function and not two

The house pattern would put a gated `anchor_shape_envelope` beside `anchor_shape`, the way
`cogmap_analytics` sits beside `cogmap_shape` (`migrations/20260628000001_cogmap_analytics_read_functions.sql:63-77`).
It is rejected here on cost: the envelope's population needs the same visible set the regions need,
and a sibling function expands `resources_visible_to` a second time per shape read — doubling the
part this function's own comment singles out as the expensive one
(`migrations/20260713000050_region_visible_member_count.sql:51`, and the `MATERIALIZED` hoist at `:95`
that exists precisely to pay it once). A standing backlog task, `019fddc6-aace-7db0-a14d-5c610bc6506b`,
exists to measure that cost.

One function also means **one gate evaluation**, so the envelope and the rows can never disagree.

### 4.2 The structure

Migration `20260823000010_anchor_shape_envelope.sql` — numbered above `origin/main`'s highest,
`20260822000030_data_artifact_shape_reads.sql`.

Postgres cannot `CREATE OR REPLACE` across a return-type change, so this is `DROP FUNCTION` then
`CREATE` — a **non-additive** migration, taken deliberately.

Structural sketch (**not** a finished body — the implementation writes it against the existing
function, which it must mirror clause for clause):

```
WITH vis  AS MATERIALIZED (SELECT resource_id FROM resources_visible_to(p_principal_id)),
     gate AS (SELECT <the existing readability disjunction, verbatim from :126-132> AS readable),
     regs AS (SELECT <the existing region select and member gate, verbatim from :99-130>
                 -- WITHOUT the p_lens predicate: regs is ALL lenses
              WHERE ... AND (SELECT readable FROM gate)),
     env  AS (SELECT
                CASE WHEN g.readable THEN (SELECT count(*)::int FROM regs) ELSE 0 END AS population,
                CASE WHEN g.readable THEN <shape_materialized_event_id -> kb_events.occurred_at>
                     ELSE NULL END AS materialized_at,
                <emptiness, per the precedence below> AS emptiness
              FROM gate g)
SELECT env.population, env.emptiness, env.materialized_at, r.*
  FROM env LEFT JOIN (SELECT * FROM regs WHERE p_lens IS NULL OR lens_id = p_lens) r ON true
 ORDER BY r.salience DESC NULLS LAST, r.region_id;
```

The `LEFT JOIN ... ON true` is what makes the empty case able to speak: an anchor with no rows still
yields exactly one row, with `region_id` NULL. Rust reads the envelope from the first row and drops
the NULL-region sentinel.

**`emptiness` precedence, evaluated in this order** `[rule 1 added 2026-08-23, during Task 4]`**:**

1. rows returned under `p_lens` > 0 → `NULL`
2. not readable → `unreadable_or_absent`
3. `shape_materialized_event_id IS NULL` → `never_clustered`
4. `population = 0` → `nothing_visible`
5. otherwise → `lens_narrowed`

Order matters in two places. **Rule 1 guards the field's own contract.** Without it, a readable
anchor holding visible regions that was never materialized returns rows *and* `never_clustered` — a
named cause attached to a non-empty answer, contradicting the column's documented meaning (*"why
`regions` is empty; `None` when it is not"*). Suppressing it there loses nothing: `materialized_at`
is NULL for exactly that anchor, and that is the field which is actually about the clock. An
unreadable anchor never reaches rule 1, having no rows.

**Rule 3 must precede rule 4**, or a never-clustered anchor reports `nothing_visible` and the
distinction this whole task exists to draw collapses back into the bug.

### 4.3 The `cogmap` self-read arm is preserved, as it was argued

`anchor_shape` also serves `p_principal_kind = 'cogmap'`, and
`migrations/20260713000050_region_visible_member_count.sql:60-74` records at length why that arm keeps
today's behaviour exactly — `resources_visible_to` takes a profile, a cogmap id yields the empty set,
and applying the member gate there would be "a blackout, not a fix". That reasoning is unchanged and
binding. For this arm the envelope reports `population` as the count of all non-folded regions (member
gate exempt, matching the rows it returns) and discloses `materialized_at`. **This spec makes no new
semantic call about what a map may count** — the same refusal `:69-72` records.

### 4.4 The stranded wrapper

`cogmap_shape` (`migrations/20260713000010_anchor_orientation_reads.sql:166-181`) is
`SELECT * FROM anchor_shape(...)` with its own six-column `RETURNS TABLE`. Dropping `anchor_shape`
strands it.

**It has no callers.** The only `cogmap_shape(` hits in the tree are its own definition and the
identically-named Rust MCP tool function (`crates/temper-mcp/src/tools/cognitive_maps.rs:34`, `:753`),
which calls `anchor_shape_select` and never the SQL wrapper.

It is nonetheless **recreated, pinned to the six original columns by explicit select-list**, rather
than dropped. Retiring it belongs to the M3 naming retirement its own comment at `:185` names
(*"this name goes away at M3 with the rest of the cogmap_* naming"*), not to this task. Recreating it
costs four lines and keeps a non-additive migration from also being a silent removal.

## 5. What moves

**Beat A — substrate.** The migration; `readback::anchor_shape` returns a typed envelope row instead
of `Vec<CogmapShapeRow>`; substrate-tier tests. Follow
`crates/temper-substrate/tests/cogmap_shape_readback.rs` (`#![cfg(feature = "artifact-tests")]`,
`cargo make test-artifacts`).

**Beat B — services, API, MCP, client.** `anchor_shape_select` → `ApiResult<AnchorShape>`; both HTTP
handlers and their `utoipa::path` response bodies; both MCP views; `temper-client`'s
`contexts().shape()`; the nine `anchor_shape_select` call sites in
`crates/temper-api/tests/context_orientation_test.rs`.

**Beat C — UI and generated artifacts.** `packages/temper-ui/src/lib/server/graph-query.ts` (two
`apiGet<CogmapRegionRow[]>` call sites) and `src/lib/graph/readout.ts`, plus their tests; then
`openapi.json`, the ts-rs tree, `clients/temper-ts/src/generated/schema.ts`, the Ruby gem model, and
the skills projection.

**Beat D — item 3's generalization.** §6.

**Beat E — the claim, and the docs.** Replace `crates/temper-cli/src/cli.rs:1079`. Then one
consolidated review across the branch.

## 6. Item 3: the clock, generalized rather than invented

The task rules item 3 in scope and its body carries the audit; this spec does not re-derive it. Two
surfaces are cogmap-only, and both are generalizations of an already-anchor-generic substrate:

**`cogmap_staleness`** (`migrations/20260624000002_canonical_functions.sql:527-551`). Its `kb_edges`
arm is already anchor-generic (`:544`); only the regions arm is stuck on `cogmap_id` (`:540`).

> **The trap, restated because it is silent.** If the regions arm is left on `cogmap_id` while the
> function is generalized, contexts do not error and do not return nulls. `latest_touch` comes back
> NULL, so `latest_touch > materialized_at` is NULL, and the `COALESCE` at `:549` falls through to
> `materialized_at IS NULL` — **false** for any context that has materialized once. Every context
> would report `is_stale = false`, permanently, and nothing would go red. **Any witness for this arm
> must use a context that has materialized AND been touched since**, or it cannot tell the two apart.

**`materialize_delta`** (`crates/temper-services/src/services/materialize_service.rs:26-31`) —
cogmap-only by signature (`CogmapId`) and by an inline `FROM kb_cogmaps`, not because anything beneath
it lacks the capability: `replay::formation_touched_count_since` already takes a `HomeAnchor`
(`crates/temper-substrate/src/replay.rs:839-843`) and `region_clocks::shape_watermark` already matches
exhaustively on it (`crates/temper-services/src/backend/region_clocks.rs:126-157`). Widen the
signature to `HomeAnchor` and register the context route beside
`crates/temper-api/src/routes.rs:145`.

**The asymmetry this closes:** a user can materialize a context today
(`crates/temper-api/src/routes.rs:113`, and `MaterializeAck` has been anchor-addressed since T8) and
cannot ask when it last happened. T8 generalized the write path; the read path did not follow.

## 7. Out of scope, named rather than done

- **No new CLI verb** for `materialize-delta`, so no `DOCUMENTED_VERBS` change
  (`crates/temper-cli/src/cli.rs:2820`).
- **The context door's missing `analytics` view stays missing.** The task rules that widening
  `materialize_delta` is item 3's business and closing the whole view asymmetry is not.
- **`cogmap_shape` is not retired** — §4.4; that is M3's.
- **No answer-quality claim.** Nothing here measures whether any answer got better; this task builds
  an instrument and witnesses nothing on its own, which is why it is filed `enables`.
- **A fifth arm separating "formed no regions" from "formed regions you cannot see" is REJECTED, not
  deferred** — §3.3. It is the one distinction the member gate exists to refuse, so it is not
  something a later task may pick up.
- **`cogmap_list_rows`' `region_count` is not reused and not fixed.** It is keyed on the vestigial
  `cogmap_id` column (`migrations/20260724000010_cogmap_list_rows.sql:46`), NULL for every context
  region, and is not member-gated. After this lands it will legitimately disagree with the envelope's
  population for the same map. **Say so where a reader meets it** rather than letting it be
  discovered.

## 8. Back-compat: what breaks, exhaustively

The break is deliberate (§3.1). The inventory:

- **Two HTTP response bodies** — array to object. No external deprecation window.
- **Two MCP view payloads** — array to object.
- **`temper-client`'s `contexts().shape()`** return type.
- **temper-ui**: `lib/server/graph-query.ts` (two `apiGet` call sites), `lib/graph/readout.ts`, and the
  test files that build `CogmapRegionRow` literals.
- **Five generated artifacts**: `openapi.json`, the ts-rs tree
  (`crates/temper-core/bindings/cognitive_maps.ts` → `packages/temper-ui/src/lib/types/generated/`),
  `clients/temper-ts/src/generated/schema.ts`, the Ruby gem model, the skills projection.
- **Nine `anchor_shape_select` call sites** in `crates/temper-api/tests/context_orientation_test.rs`,
  plus `crates/temper-api/tests/cogmap_shape_handler_test.rs` and
  `tests/e2e/tests/context_orientation_e2e.rs`.

**Gates, and what each one misses:** `cargo make check` runs the drift gates but **does not run
temper-ui** — `cd packages/temper-ui && bun run check` is a separate command and is required. ts-rs
drift only clears after a **commit**, not at `git add`. SQL changes need the **per-crate** `.sqlx`
regeneration, not `--workspace` (`Makefile.toml:112-121`).

## 9. Testing

- **Representative test to follow:** `crates/temper-api/tests/context_orientation_test.rs`
  (`#![cfg(feature = "test-db")]`) — nine `anchor_shape_select` call sites already covering grantee
  reach, stranger denial as empty, lens narrowing and the member gate. Every one of the five outcomes
  in §3.3 has a near-neighbour there already.
- **Fixture to reuse:** `crates/temper-substrate/tests/common/context_fixture.rs` — the shared fixture
  for a real embedded context with materialized regions. Its header records that it was extracted
  rather than copied because it encodes non-obvious formation constraints. Do not fork it.
- **Substrate tier:** `#![cfg(feature = "artifact-tests")]`, `cargo make test-artifacts`.
- **The member-gate witness must use two principals with different reach** reading the same anchor and
  receiving different populations — the task's first acceptance criterion, which §2.1 shows is
  already met by `regions.len()` and which the all-lenses definition now makes non-trivial: assert it
  **under a lens filter**, where `population` and `regions.len()` genuinely differ.
- **The `is_stale` witness must use a context that has materialized AND been touched since** (§6).
- **Test-db modules need the `test-db` cfg gate**; `#[sqlx::test]` modules without it compile into
  tiers that have no database.

## 10. Acceptance, mapped

| Task criterion | Where it is met |
|---|---|
| Both anchor kinds report a member-gated population, demonstrated by two principals with different reach | §4.2 `regs` (inside `vis`); §9 witness, asserted under a lens |
| "Never clustered" distinguishable from "nothing visible here" at every door that serves shape | §3.3 rows 3–4; §3.1 (why the envelope is on shape itself and not a second door) |
| A principal who cannot read the anchor learns nothing new | §3.3 row 5 and its argument |
| `cli.rs:1079`'s claim is either true or replaced | §5 Beat E — replaced |
| Generated artifacts ride along, drift gates green, temper-ui checked separately | §8 |

## 11. Why this is a precondition, beyond this goal `[noted 2026-08-23, Pete]`

A CLI goal is forthcoming to bring the graph-traversal tools built for the UI into general use. Region
materialization visibility is part of that: `survey -> graph expand`-style traversals, both **within a
composition** and **across tool calls**, need to know whether an anchor has anything to traverse and
how much of it there is, rather than inferring it from an empty result. This task is expected to be a
precondition of that goal. It is recorded here so the connection is not re-derived.
