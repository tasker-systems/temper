# The staleness clock, gated — and the context composer that was declared missing

Design spec for task
[The staleness clock reports on regions the shape read refuses to name](./01a03636-8077-7ee2-a070-f6766658a41e),
a `witnesses` task under
[Unified visibility semantics for contexts and cognitive maps](./019f5c66-755e-7fc1-bd87-ee2de8e4cd3f).

Grounded against `main` at `49282d95`. Every claim below carries a `file:line` citation or is tagged
as invention. Steps carry **CONFORM / EXTEND / AMEND** so the movable-vs-load-bearing judgment is
auditable up front rather than discovered at invocation.

---

## 0. Scope, and the ruling that set it

Two things were open when this task started. Both are now closed, and this section exists so neither
is re-argued.

**RULED before this spec was written (task body, 2026-08-25):** `anchor_staleness` **requires** a
principal and there is **no ungated variant**. A two-variant split was rejected on evidence, not
taste — the caller inventory is three rows and the only non-principal caller already has a principal
in scope. Reversible later by adding a variant *by name* if a system path ever appears.

**RULED at session start, 2026-08-25:** this work **also** ships `context_analytics`. That is a
deliberate scope increase over the task's filed `medium` effort, taken with the tradeoff stated: it
turns a one-migration security fix into a fix plus a new read surface across SQL, substrate,
services, API, MCP, CLI, generated artifacts and the UI. **The effort is `large` in substance.** It
is recorded here rather than absorbed silently.

### Why the two belong in one change

They are the same asymmetry seen from two ends, and the second is *cheaper* riding along than
separately:

- `anchor_staleness` is **already anchor-generic** (`20260823000020:54`). Once it is gated it answers
  correctly for a context, and the only thing standing between that and a context surface is a
  composer and its wiring.
- `context_analytics`'s composer must pass a principal into `anchor_staleness`. If it is built
  *before* the gate it is built against a signature that is about to change — a second cutover.
- The migration is shape-breaking either way. Adding the composer to it costs one `CREATE`.

---

## 1. The defect, restated with its evidence

`anchor_staleness`'s `touch` CTE (`migrations/20260823000020_anchor_staleness.sql:68-80`) takes
`max(occurred_at)` over a UNION of a regions arm and an edges arm, and applies **no readability
predicate to either**.

The composer gates, but on the **anchor**:
`migrations/20260628000001_cogmap_analytics_read_functions.sql:38-39` is a trailing `WHERE` on
`cogmap_readable_by_profile`. It decides whether the single row appears. It cannot reach the clock's
inputs, because `cogmap_staleness(p_cogmap)` is called at `:37` with no principal — its signature
takes none.

The tell is internal to that one function: at `:35` the sibling `cogmap_regulation(p_cogmap,
p_principal_kind, p_principal_id)` **is** handed the principal. One composed sub-read is
member-gated and the other is not.

Both enumeration doors on the same anchor refuse to name a region the clock reports on:

| door | citation | the rule |
|---|---|---|
| `anchor_shape` | `20260823000010:86-88` | `NOT reg.is_folded` **and** `p_principal_kind = 'cogmap' OR seen.visible_members > 0` |
| `anchor_region_metrics` | `20260713000050:260-268` | same rule, same words, stated as an `EXISTS` |

The edges arm is broader still — it carries **no predicate at all**, where the incumbent read-set
`edges_visible_to` (`20260712000010:295-309`) requires non-folded plus both endpoints visible, and
`element_trail_edge` (`20260719000010:166-168`) requires `anchor_readable_by_profile` **and**
`endpoint_readable_by_profile` on both endpoints.

### Honest bounds — do not oversell this

- `latest_touch` is a `max()`. It leaks **at most one bit per distinct timestamp** — "something under
  this anchor moved at time T" — and never names the row. `is_stale` collapses that to one bit.
- **No differential has been measured.** As with the member-count fix (`20260713000050:41-45`), this
  is structural: it is about what the read is *willing* to say, not what it has said. Do not write a
  reason or a note that implies otherwise.

---

## 2. What must NOT change — the fold arm is correct

**`is_folded` is absent from both arms deliberately, and narrowing them is a silent defect, not a
fix.** `20260823000020:30-32` states it, and the `COMMENT ON FUNCTION` at `:87` and the
`declare_migration` reason at `:113` each restate it independently:

> the folded-inclusive scan and its covering index `idx_kb_edges_home_all` (20260708000008) are
> deliberately untouched: a fold event advances the edge's last_event_id and staleness must keep
> reporting it, which is why that arm carries no `is_folded` predicate. Same for the regions arm.

The upstream index carries the same reason at `20260708000008:12-13`.

The distinction that makes this reconcilable rather than merely inconsistent, from
`internal/agents/key-patterns.md`:

> **Fold is not a visibility predicate.** `resources_visible_to` and `is_active` are *authorization*
> predicates. `is_folded` / `is_current` are *currency* predicates. Only the first class is a
> disclosure question.

**Consequence for the implementer, stated because it is the likeliest way to get this wrong:** the
incumbent read-set `edges_visible_to` mixes both classes in one predicate (`NOT e.is_folded` at
`20260712000010:297`, endpoint visibility at `:302-309`). You may **not** call it wholesale. Take its
*authorization* half — which the schema already exposes under a name, `endpoint_readable_by_profile`
(`20260624000002:292`) — and leave its currency half behind. Say so at the field.

---

## 3. A third fork, ruled during grounding: the gate is the FULL gate

The task's "Do this" says *gate the member-visibility half*. Grounding found that gating **only** the
member half leaves an oracle, so the gate must also carry the anchor disjunction. This is a design
decision made in this spec, not carried in from the task, and here is the argument:

With members and endpoints gated but no anchor gate, a caller who cannot read the anchor gets
`latest_touch = NULL` (both arms contribute nothing). The `COALESCE` at `20260823000020:82` then
collapses `is_stale` to `mat.materialized_at IS NULL` — and the `mat` CTE (`:57-67`) reads the
anchor's watermark **unconditionally**. So a caller who cannot read a **real** anchor still receives
a row carrying that anchor's watermark and an `is_stale` reporting whether it has ever been
materialized.

> **CORRECTED 2026-08-25, after this spec was first published.** The paragraph above originally said
> the denied caller learns this *"for any uuid they invent"*. **That was false, and it is measurably
> false.** `mat` selects `WHERE id = p_anchor_id`, so a uuid naming no anchor produces no `mat` row,
> and the final `FROM mat, touch` cross join yields **zero rows**. Verified against the *incumbent*
> on the live local database: an invented uuid returns 0 rows there today, with no gate at all.
> The real disclosure is narrower and sharper — it is about a **real anchor the caller may not
> read**, which is the case where "this uuid names something that exists" is itself the fact handed
> over. The argument for the full gate is unaffected; only my description of its size was wrong.

The full gate closes it by making **deny and absent the same answer** — zero rows either way, so the
two are indistinguishable. That is the property `anchor_shape` states as *"deny and absent collapse
into ONE arm and disclose neither population nor clock"* (`20260823000010:186`), which it reaches via
the `EXISTS` conjunct at `:65`, argued at `:43-58`. Handing `anchor_staleness` a principal puts it
under the same obligation, so it adopts the same gate.

**One asymmetry worth recording, because it runs the other way.** That `EXISTS` conjunct is
**load-bearing in `anchor_shape` and not here.** `anchor_shape` forces a row via `LEFT JOIN … ON
true` (`20260823000010:181`) to carry its envelope, so a tautological cogmap self-read arm would hand
back `emptiness` and `materialized_at` for an invented uuid. `anchor_staleness` ends in a cross join
against a `mat` CTE that is empty for a non-existent anchor, so it already answers nothing. The
conjunct is carried anyway — one rule in two places must read identically to stay in sync — but the
migration must not claim it closes a hole here, or it repeats the error corrected above one level
down.

**So `anchor_staleness` carries the same two-part gate its two siblings carry:** the anchor
disjunction *and* the member/endpoint rule. On deny it yields **zero rows**, which is what
`cogmap_analytics` already produces on deny (`20260628000001:38-39`) and therefore changes nothing
about that composer's observable behaviour.

This supersedes the header claim at `20260823000020:50-52` ("No gate here, matching the incumbent…
the gate lives in the composers"). That sentence was true when the function took no principal. The
new migration must say so explicitly rather than leave two migrations disagreeing.

### The fail-open shape that is forbidden

Do **not** express the gate as optional principal parameters where `NULL` means ungated. An empty
scope aggregating to `NULL` and falling open has already bitten this codebase
(`reference: array_agg over empty scope falls open`). Ungated, if it ever returns, is a **name** you
cannot type by accident.

### The overload that must be dropped, not replaced

`cogmap_staleness` gains parameters. In Postgres, adding a parameter **creates an overload** — a
`CREATE OR REPLACE` with a longer argument list leaves the old `cogmap_staleness(uuid)` standing and
**ungated**. That is the "misrouting, not drift" hazard the task body records: same name, same column
names, same `boolean` type, silently no gate. Both `anchor_staleness(text, uuid)` and
`cogmap_staleness(uuid)` must be explicitly **`DROP`ped**.

---

## 4. `context_analytics` — three columns, not five

`cogmap_analytics` returns five columns (`20260628000001:29-30`): `telos_resource_id`,
`materialized_at`, `latest_touch`, `is_stale`, `regulation`.

**A context has nothing to put in two of them, and the UI already says so in as many words.**
`packages/temper-ui/src/lib/graph/analysis.ts:353-362` — the file with the NUL byte, read with
`sed`/`tr -d '\000'`, never with grep:

> The unification's **D6** would port the staleness half of `cogmap_analytics` to contexts, and it
> is unshipped: `kb_contexts.shape_materialized_event_id` exists and is written, but there is no
> context analytics read. A context has a `telos_centroid` and neither a charter resource nor a
> regulation set, so there is nothing to return even in principle for two of the three.

and the string it guards, `CONTEXT_HAS_NO_MAP_READOUT` (`analysis.ts:361-362`):

> a charter, and the concepts that regulate it, belong to a cognitive map — a context has neither, so
> there is **nothing here to report rather than nothing found**.

**That distinction decides the return type.** Returning `telos_resource_id NULL, regulation '[]'`
would say *nothing found* about two things that cannot exist — the precise failure the UI constant
was written to avoid, and what its own docstring calls "faked as a peer field". So:

`context_analytics(p_context uuid, p_principal_kind text, p_principal_id uuid)` returns
**`(materialized_at, latest_touch, is_stale)`** and nothing else.

This is **not** a divergent shape hand-written a second time — it is the same delegating-wrapper
pattern `cogmap_staleness` already uses over `anchor_staleness` (`20260823000020:103-108`), which the
goal endorses and which the task body confirms was *not* the source of the "one rule in four places"
drift. Both composers select from the one gated core.

---

## 5. Surface parity — the exact gap being closed

Measured from `crates/temper-api/src/routes.rs`:

| read | cogmap | context |
|---|---|---|
| `shape` | `:145` | `:111` |
| `region_metrics` | `:148` | `:112` |
| `materialize_delta` | `:146` | `:113` |
| `materialize` | `:147` | `:114` |
| **`analytics`** | **`:149`** | **absent** |

`context_analytics` closes one row of a five-row table. Every other row is already symmetric, so the
wiring has four worked examples to copy rather than a pattern to invent.

---

## 6. The witness must bite — and the scenario suite will not provide it

**A consequence to plan around, not a surprise to discover.** Passing `loaded.owner` at
`runner.rs:486` leaves every existing scenario green, because an owner sees all their own regions.
Good for churn, bad for coverage: **the scenario suite will not exercise the gate at all.** A test
that merely re-runs the current function proves reproducibility, not correctness.

**The reachable bite is soft-delete, and only soft-delete.** `anchor_shape`'s own analysis
(`20260823000010:157-173`) establishes why:

> a readable anchor's regions are built by the materialize projection from that anchor's own homes
> […] and `resources_visible_to` admits every resource homed in a readable anchor
> (`20260807000010:192-222`). So "regions holding members another tenant hid from you" is not a row
> the writer can produce.
>
> What IS reachable […] is STALE membership: members soft-deleted (invisible on every axis,
> `20260807000010:224`) or rehomed away, leaving ghost regions whose `visible_members` is 0.

So the fixture is a **ghost region**, and it needs no second principal — which makes it sharper, not
weaker: the *owner themself* must stop seeing the clock move.

**The shape that separates the working function from the broken one:**

1. Materialize the anchor (sets the watermark).
2. Soft-delete every member of one region (`kb_resources.is_active = false`).
3. Advance that region's `last_event_id` to an event later than the watermark.
4. Read staleness **as the owner**.

`is_stale` is **true** before the fix and **false** after. Under the ungated function the ghost
region's touch is counted; under the gated one it is not, `latest_touch` is `NULL`, and the
`COALESCE` falls to `materialized_at IS NULL` — false, because step 1 materialized.

**The machinery already exists.** `crates/temper-api/tests/context_orientation_test.rs:689-718` (`touch_region_with_a_later_event`) is a
helper that mints an event later than the watermark and advances a region's `last_event_id` — steps
1 and 3, already written and already used by `a_touched_context_reports_stale` (`:759`). The new
fixture is that helper plus the soft-delete.

**A second witness for the edges arm**, same shape: an edge whose endpoint is soft-deleted, touched
after the watermark.

**Bite-probe both.** Revert the gate on a working copy and confirm *that test alone* fails. A guard
green because it never asked the question is the failure mode this repo has hit twice.

> **Not required this time:** a Linux-container run. That lesson came from a **shell** harness
> hitting a BSD/GNU `stat` split; nothing here is shell. Ruled out at session start rather than
> carried as ritual.

---

## 7. The work, as tagged steps

Each step names the incumbent to copy. **No code bodies are authored here** — where a predicate is
needed, the citation *is* the specification, and the implementer reads it from disk.

### Beat A — the migration (`migrations/20260825000010_staleness_member_gate.sql`)

Number chosen as the next slot above `main`'s maximum, `20260823000020`. **Immutable once applied.**

| # | step | tag | grounding |
|---|---|---|---|
| A1 | `DROP FUNCTION anchor_staleness(text, uuid)` — signature change, not replaceable | **AMEND** | `20260823000020:54`; overload hazard §3 |
| A2 | `CREATE FUNCTION anchor_staleness(text, uuid, text, uuid)`, same three return columns | **AMEND** | return set at `20260823000020:55` |
| A3 | gate CTE: carry the anchor disjunction **verbatim** from its sibling, `EXISTS` conjunct included | **CONFORM** | `20260823000010:59-66` |
| A4 | regions arm: add the member rule, same words as both incumbent doors | **CONFORM** | `20260823000010:87-88`; `20260713000050:260-268` |
| A5 | edges arm: add `endpoint_readable_by_profile` on **both** endpoints, plus the cogmap self-read exemption | **CONFORM** | `20260624000002:292`; `20260719000010:166-167` |
| A6 | **leave both arms fold-inclusive**; state at the field why the two halves differ | **CONFORM** | `20260823000020:30-32`; `20260708000008:12-13` |
| A7 | `mat` CTE's inner `UNION ALL` subquery stays byte-identical to `anchor_shape`'s `clock` | **CONFORM** | `20260823000020:59-66` ≡ `20260823000010:95-101` — see note |
| A8 | `DROP FUNCTION cogmap_staleness(uuid)`, then `CREATE` it at `(uuid, text, uuid)` | **AMEND** | wrapper pattern `20260823000020:103-108`; §3 overload |
| A9 | `CREATE OR REPLACE cogmap_analytics` — pass its principal through to the wrapper at `:37` | **AMEND** | `20260628000001:37`; the `:35` tell |
| A10 | `CREATE FUNCTION context_analytics(uuid, text, uuid)` → three columns | **EXTEND** | authorized by §0 ruling + goal D6; shape argued §4 |
| A11 | `COMMENT ON FUNCTION` for each; **correct**, do not restate, `20260823000020:50-52` | **AMEND** | §3 |
| A12 | `declare_migration(..., 'shape-breaking', ...)` — reason states the gate, the preserved fold arm, the dropped overloads, and that **no differential was measured** | **CONFORM** | `20260823000020:110-114` as the model |

> **A7 corrected during Beat A.** This spec originally said the two CTEs are byte-identical. They
> are not, and forcing them to be would be wrong: `anchor_shape`'s `clock` additionally projects
> `a.eid`, which its `never_clustered` arm needs and staleness has no use for. What must stay
> byte-identical is the **inner `UNION ALL` subquery** — the two read the same column off the same
> two tables, and a divergence there would be a divergence between what a shape read calls
> "materialized" and what a staleness read compares against.

> **A citation repair found while grounding A11.** `20260823000020:51` cites the composer gate as
> `20260628000001:77-78`. That file is 40 lines long; the gate is at `:38-39`. The new migration
> repairs the citation rather than propagating it.

### Beat B — Rust

| # | step | tag | grounding |
|---|---|---|---|
| B1 | `runner.rs:486` → pass `loaded.owner` and `'profile'` | **AMEND** | `loader.rs:24` `pub owner: Uuid`; call site is `fetch_one`, so deny would error — the owner never denies |
| B2 | `readback::context_analytics` + `ContextAnalyticsRow`, mirroring its cogmap peer | **CONFORM** | `readback/mod.rs:2085-2137` |
| B3 | `substrate_read::context_analytics_select` | **CONFORM** | `substrate_read.rs:1385-1400` |
| B4 | `handlers::contexts::analytics` + `routes.rs` registration | **CONFORM** | `cognitive_maps.rs:309-336`; `routes.rs:111-114` |
| B5 | MCP `context_analytics` tool | **CONFORM** | `tools/cognitive_maps.rs:106-131` |
| B6 | CLI `temper context analytics` | **CONFORM** | `commands/cogmap.rs:120-128`; `actions/cogmap.rs:91-99` |
| B7 | the orphaned duplicate `.sqlx` entry in `temper-services` for the runner's query becomes **provably** dead once B1 changes the query text | **AMEND** | `crates/temper-services/.sqlx/query-2c7323be…json` vs the caller in `temper-substrate` — verify before removing |

### Beat C — witnesses (§6)

Ghost-region bite for the regions arm, ghost-endpoint bite for the edges arm, `context_analytics`
happy path, and a **deny** test asserting zero rows / `None` rather than an error. Bite-probe each.

### Beat D — UI

| # | step | grounding |
|---|---|---|
| D1 | `graph-query.ts:278-289` — the ternary that skips contexts now calls the context endpoint | the block reads `anchor.kind === 'cogmap' ? … : Promise.resolve(null)` |
| D2 | `graph-query.ts:264-267` — the docstring asserting "there is no context analytics read (D6 is unshipped)" becomes false the moment D1 lands | must be rewritten, not left standing |
| D3 | `analysis.ts:353-362` docstring and `CONTEXT_HAS_NO_MAP_READOUT` — the string says a context shows no "charter, **staleness** or regulation"; staleness is now shipped | charter + regulation remain true; the middle term must go |
| D4 | `AnalysisPage.svelte:205` renders the constant; a context now has a staleness readout to show | `analysis.test.ts:277` asserts the phrase "a context has neither" — still true of two things, so check rather than assume the test breaks |

> **Every file in Beat D is reached through the NUL-byte file or beside it.** `analysis.ts` is
> **skipped silently by `grep` and `rg`**. Sweep it with `sed`/`tr -d '\000'`/`cat`. A grep-based
> sweep will conclude there is no UI consumer, and there are four.

### Beat E — generated artifacts, cutover, PR

Regenerate the `.sqlx` caches, ts-rs bindings, `openapi.json`, `temper-rb` and `temper-ts` — the
drift gates clear at different git stages (rb/ts on `git add`, ts-rs only after **commit**). Then
`cargo make check` and `test-db`.

**The cutover is an operator step and runs through `scripts/migrate-cutover.sh`, never the bare
binary.** It applies the **working tree**, not the commit; a clean tree is not the same as a pushed
one. Editing this migration after it is applied trips the `_sqlx_migrations` checksum.

---

## 8. Out of scope

**Rejected** — argued and declined, do not re-open:

- **An ungated `anchor_staleness` variant.** Ruled 2026-08-25 on a three-row caller inventory (§0).
- **Narrowing either arm to live rows.** §2. This is the point of the function, not a defect in it.
- **`telos_resource_id` / `regulation` on `context_analytics`.** §4 — a context has neither, and
  returning a null peer field says *nothing found* about something that cannot exist.
- **Per-caller metric recomputation.** Ruled out at `20260713000050:227-231` and untouched here: the
  stored metrics ride through as stored. Only *which rows are enumerated* changes.

**Deferred** — real, not addressed here:

- **No differential measurement.** §1. `20260713000050:28-45` set the standard by measuring 0 of 546
  regions diverging before shipping. Not done here, and the migration reason must say so.
- **The remaining `.sqlx` orphans in `temper-services`.** Only the one B1 provably kills is touched.
- **`materialize_delta` is cogmap-only on every surface and typed on `cogmap_id`**, and no CLI
  exposes it for either anchor — the same asymmetry wearing a different hat, per the goal. Untouched.
