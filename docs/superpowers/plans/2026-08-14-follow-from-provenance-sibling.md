# `follow-from`'s provenance sibling — implementation plan

**Task:** [Build follow-from's provenance sibling](./01a001b0-10b9-7752-8d7a-770df8dcdb8c)
**Spec — read it, this plan does not replace it:**
[`2026-08-14-follow-from-mechanic-design.md`](../specs/2026-08-14-follow-from-mechanic-design.md)

This is an **index + sequence + grounding evidence** over that spec. Every step cites the section the
implementer must read rather than restating it, and carries a **CONFORM / EXTEND / AMEND** tag. Where
a step is EXTEND or AMEND, the spec section authorizing it is named.

---

## 0. The four rulings this plan is built on

Taken `[2026-08-14, Pete]` before any code. Each is recorded in the spec at the section it settles;
they are listed here only so no step has to be read against a stale draft.

| ruling | consequence for this plan |
|---|---|
| **Depth is definitional, fixed at 2** (§2.1, §5) | `BoundTerm` does **not** grow a variant. `accepts_bound_terms` stays `[Limit]`. No wire change, no openapi/ts-rs churn from this axis. The fragment still takes `p_depth`; the compiler passes the constant. |
| **Three functions**, incumbent re-pointed (§10) | `search_graph_expand` → `query_follow_from` → `__temper_ungated_follow_from`. ONE BODY. **The core must carry `p_gamma`**, because the incumbent's signature has that slot and delegation passes it through. |
| **`p_bound_ids` ships, constraining the whole walk** (§9) | Intermediates included. `accepts_bounds` becomes `[IdKind::Resource]`; the one genuine foreclosure closes. |
| Gates: TDD · `generated-artifacts` skill · `sqlx-query-cache` skill · `/code-review` at the end | Step-level verification below; no completion claim without quoted output. |

---

## 1. Grounding evidence

Gathered first-hand this session against the working tree at `f7b2205a`. **A step below may rely on
these and on the spec's `file:line` citations; anything else is verified at the step.**

### 1.1 The incumbent, in full

`migrations/20260711000030_search_seed_debias_seed_only.sql:23-61`

```sql
CREATE OR REPLACE FUNCTION search_graph_expand(
  p_principal uuid, p_seed_ids uuid[], p_depth int, p_edge_types text[], p_gamma double precision)
RETURNS TABLE (resource_id uuid, graph_score real)
```

Load-bearing details for the re-point:

- `adj` admits an edge only when **both** endpoints are visible (`:37-38`) — the existing
  set-shaped constraint that already binds intermediates, and the model `p_bound_ids` follows.
- `adj` unions both orientations (`:48-50`) — the walk is undirected over a directed graph (spec
  §4.1).
- The one kind filter is `e.edge_kind::text = ANY(p_edge_types)` (`:35-36`). **The label axis has
  never existed** (spec §6).
- The final projection is `SELECT node, MAX(score)::real … WHERE hop > 0 GROUP BY node` (`:57-60`).
  The `hop > 0` and the `MAX` are the act's declared quantity; they do not move.
- `walk` already carries `path uuid[]` (`:41,45`) and uses it for cycle exclusion (`:53`). **The
  parent is `path[array_upper(path,1)-1]`, already in hand** — this is the spike's "the walk
  already carries provenance" finding, and it is why this is a projection change and not a new
  traversal.

### 1.2 The chain to conform to

`migrations/20260808000030_composable_find_family.sql:19-33` (header) states the three-level rule and
why the core takes an array:

> `search_exact(p_principal, …)` → `query_find_exact(…, p_bound_ids)` → `__temper_ungated_find_exact(p_visible_ids, …)`
> … **ONE BODY PER ARM is the invariant the whole chain exists to preserve** … *"two bodies drift,
> and the drift is silent because both keep returning plausible rows"*

And the polarity rule, which this plan must not get backwards —
`20260814000010_find_resources_with.sql:100-105`:

> Every parameter after `p_visible_ids` is an AND-composed narrowing whose NULL narrows **NOTHING**.
> That is the **opposite polarity** from `p_visible_ids`, whose NULL admits nothing.

`p_bound_ids` follows `p_visible_ids`' *sibling* rule, stated at `20260808000030`'s declaration:
**NULL is unbounded and `'{}'` returns zero rows**, so an upstream stage that produced nothing cannot
silently widen into a global walk.

**`unnest`, NEVER `= ANY`, for the id sets** (`20260808000030` header): `= ANY(uuid[])` establishes no
equivalence class and cannot propagate a join condition. The measurement that cleared the array path
compared `JOIN unnest($1::uuid[])`.

### 1.3 The Rust seams

| seam | `file:line` | what it holds today |
|---|---|---|
| act declaration | `registry.rs:325-390` | `served_by: "search_graph_expand"`, `accepts_bounds: vec![]`, `discloses: vec![]`, `door_coverage` **Absent** ×3 |
| the bounded-walk sentence | `registry.rs:328-330` | *"walk from these seeds but **stay inside this set**"* — already the interior reading |
| reachability map | `validate/mod.rs:84-91` | 3 entries, **gated name → ungated core**; membership decides `NotSeparablyReachable` |
| the two refusals to retire | `capability.rs:353-368` | `properties` and `edge_filter`, both **unconditional** |
| the per-act filter check that survives | `capability.rs:415-419` | `edge_filter.is_some() && !decl.accepts_filters.contains(&FilterField::Edge)` |
| emitter | `query_plan.rs:118-120`, `:443-470` | `EMIT_*` constants; `follow-from` falls to the `_` placeholder arm |
| the one call emitter | `query_plan.rs` `emit_ungated_core_call` + `CoreCall` enum `:478-520` | *"the whole security property … is that it is the one place `VISIBLE_IDS` and `PRINCIPAL_BIND` are written"* |
| final select | `query_plan.rs:1022-1053` | shared column list `row_class, stage, id, kind, quantity, produced, unusable` |
| row type | `query_exec.rs:31-37` | `HitRow { stage, id, kind, quantity }` |
| assembler | `query_read.rs:485-491` | `score: h.quantity…`, `located_at: None` |
| hit type | `hits.rs:158-193` | `ResourceHit { resource, scoring, located_at }` |
| disclosure enum | `act.rs:296-298` | *"`InputContribution` used to lead this enum … **returns when a walk carries origin**"* |
| edge filter | `filter.rs:26-31` | `EdgeFilter { edge_kinds, labels }` — the label half has no fragment |

### 1.4 What the incumbent's callers are

`registry.rs:356` and `20260806000010`'s declaration both state it: **nothing outside
temper-substrate's tests calls `search_graph_expand`.** Re-pointing it is therefore a change whose
entire blast radius is `crates/temper-substrate/tests/search_graph_expand.rs` — which is the
argument for doing it (those tests become coverage of the real body) rather than against.

---

## 2. Sequence

Four deliverables. **A is a session; B+C are a session; D is a session.** Each ends green and
committed; none leaves the tree with a half-wired act.

---

### A — the migration and its substrate tests

**Tag: EXTEND** — new functions beside an untouched incumbent — **plus one AMEND** (the re-point).
Authorized by spec §10's three-function ruling and §1's *"adding a differently-shaped function under
a new name is additive, always."*

**A1. `__temper_ungated_follow_from` — the core.** New. Signature carries, in order: the visible set,
the seeds, the walk's two definitional constants, the two edge axes, the bound, the limit.

- `p_visible_ids uuid[]` — NULL admits nothing (§1.2 polarity).
- `p_bound_ids uuid[]` — NULL is unbounded, `'{}'` is empty. **Applied inside `adj`**, exactly where
  visibility is applied, which is what makes it constrain intermediates (spec §9). CONFORM to
  `20260711000030:37-38`; do not add a post-filter on the final `SELECT`, which is the output-only
  reading the ruling refused.
- `p_depth int`, `p_gamma double precision` — present because the incumbent's signature has them and
  delegation passes them through (spec §2.1, §10). **Not caller-facing at the act.**
- `p_edge_kinds text[]` (CONFORM to the incumbent's `p_edge_types` predicate) and `p_labels text[]`
  (**EXTEND** — spec §6: *"the label axis has never existed in the walk"*).
- `p_limit int`.
- Returns the incumbent's two columns **plus `via jsonb`**.

**`via`'s content is settled and is not a design step** — spec §4: `seed_id`, `source_id`,
`target_id`, `edge_kind`, `label`, `polarity`, *as asserted*, no numbers (§3.2), no cap (§5), one
row per node (§3).

**The nullable `label` must be handled and must not be papered over** (spec §4.3): the DDL admits
NULL (`20260624000001:636`), prod has none, and the repo's convention for it is
`COALESCE(label,'')` in the uniqueness index (`:646`). A `p_labels` filter therefore excludes
unlabelled edges — **state that in the `COMMENT`**, per §4.3's *"should be stated rather than
discovered."*

**A2. `query_follow_from` — the gated wrapper.** New. `p_principal` in place of `p_visible_ids`;
computes `ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v)` once and delegates.
CONFORM to `query_find_resources_with` (`20260814000010:235-256`) — including its **no-`CASE`-guard**
reasoning, which applies here for the same reason: this act has no guaranteed-empty input.

**A3. `search_graph_expand` — re-pointed.** **AMEND**, authorized by spec §10. `CREATE OR REPLACE` at
the **byte-identical** signature and return type, body delegating to A2 and projecting the two
incumbent columns.

> ⚠️ **THE TRAP, restated because it is the one that blocks a deploy.** `CREATE OR REPLACE` may not
> change the argument list or the return type. If the delegation cannot be written without changing
> either, **the answer is not to `DROP`** — it is to leave the incumbent alone and fall back to two
> functions, and to say so. Spec §8.1: every local test passes and it fails at deploy on `main`,
> blocking every subsequent deploy on that target until an operator takes a cutover.

**A4. `declare_migration(…, 'additive', …)`.** The class is parsed from the SQL the deploying binary
carries. Three `CREATE FUNCTION`/`CREATE OR REPLACE`, no `DROP`. The description must say what the
functions do and what `p_bound_ids`' polarity is — these `COMMENT`/declaration bodies are the read
surface, per every migration in this family.

**A5. Tests — TDD, written before the bodies.** In
`crates/temper-substrate/tests/search_graph_expand.rs` (neighbours) and a new sibling.

Each of these must **fail against the current state**:

1. **The bound constrains intermediates.** seed → B ∉ bound → C ∈ bound: **C does not return.** This
   is the witness for §9's ruling and the observable difference the spec names.
2. **`'{}'` returns zero rows; NULL is unbounded.** Two assertions, not one.
3. **`via` names the edge as asserted** — an `inverse` `contains` edge reports `polarity` and its own
   `source_id`/`target_id`, not a re-derived direction (§4, §4.2).
4. **Every parent, not the winning path's** — a node reachable by a strong 1-hop and a weak 3-hop
   path reports **both** parents and **one** score (§3.2).
5. **One row per node** (§3).
6. **The label filter excludes unlabelled edges** — asserted, since the DDL admits a NULL prod does
   not have (§4.3).
7. **The re-point is behaviour-preserving**: the incumbent's existing assertions still pass
   unchanged. If any needs editing, that is a finding to report, not a test to adjust.

**A6. Measure** (task's *"measure at build time"*). With-`via` vs without-`via` against the real
function via `pg_stat_statements` (`20260814000020`). Spec §10 says why a hand-written approximation
would measure the approximation. Record the number in the spec's §10 and in the session note.

**Verify A:** `cargo nextest run -p temper-substrate --features artifact-tests --test search_graph_expand`
plus the new target. `cargo clean -p temper-migrate` after adding the migration (CLAUDE.md: `sqlx::migrate!`
embeds `migrations/` at compile time). Read the **`sqlx-query-cache`** skill before touching a query macro.

---

### A′ — **DONE.** Committed `cd6e5152`

The migration, its eleven witnesses, the ungated-gate baseline, and the build-time measurement.
Two things landed that this plan did not predict:

- **`via` costs +20.0%** (315.4 ms against 262.9 ms, depth 2, ~1 ms spread per arm), and the
  **re-pointed incumbent pays it in full for a column it discards** — no pruning through the
  recursive CTE. Both recorded in spec §10.
- **The bound's witness was bite-probed**: flipping to the output-only reading fails exactly one
  test and nothing else.

---

### B0 — widen the stage input **`[ruled — 2026-08-14, Pete]`, blocks B**

**Filed as its own task:** [A stage carries one set](./01a001fd-956c-79b2-acf2-664272f54dbd)

**Tag: AMEND.** Authorized by spec §9's build-time finding.

`ActInvocation.input: Option<StageInput>` → `inputs: Vec<StageInput>`, **at most one per relation**.
Without it a bounded walk is inexpressible — a stage carries one set and one relation, so seeds and
a bound cannot both arrive, and `accepts_bounds: [Resource]` would declare a capability no caller
can reach.

Sites, all verified this session:

| site | `file:line` | what changes |
|---|---|---|
| the field | `envelope.rs:48` | `Vec`, `#[serde(default, skip_serializing_if = "Vec::is_empty")]` |
| shape pass | `shape.rs:318` | iterate the caller sets; **refuse a duplicate relation** |
| capability pass | `capability.rs:196-220`, `:238` | per-input, each against its own relation's list |
| graph edges | `composition.rs:190` | `upstream_names` returns every upstream, not the first |
| compiler | `query_plan.rs` `narrowing_for`, `StageNarrowing` | the enum becomes a struct — **and its "never both" doc comment is the thing being retired, so it must be rewritten rather than left contradicting the type**. The safety property to preserve is that the RELATION picks the slot: a seed must still never land in `p_bound_ids`, and a find act receiving a seed must still error rather than narrow |
| assembler | `query_read.rs:371` | `StageNumbers` goes plural |
| trace | `trace.rs:74` | `input_source` and the relation are singular **on the wire** |

Then generated artifacts (`generated-artifacts` skill), and the `?`-shaped test surface across
`query_plan_compile.rs`, `query_plan_execute.rs`, `query_run_composition_test.rs`,
`query_route_e2e.rs`.

**Only after B0 does `accepts_bounds: vec![IdKind::Resource]` become true rather than aspirational.**

---

### B — the act's declaration and the two refusals

**Tag: AMEND** throughout — every item changes a shipped declaration. Authorized by spec §§2.3, 3.3, 6, 9.

**B1. `registry.rs`** — `served_by` → `query_follow_from` · `accepts_bounds` → `[IdKind::Resource]` ·
`discloses` → `[Disclosure::InputContribution]` · `door_coverage` Absent → **Serves** at CLI and API,
**Absent stays at MCP** (`registry.rs:288-291`: the MCP server exposes no `query` tool, and this act
cannot close that) · `asker_holds` rewritten to say the seed **set** (§2.3) · `orders_by.means` is
where the fixed depth and gamma are documented (§2.1's ruling).

**Do not touch `scoring_revision`.** `registry.rs:349-351`: *"A revision records a change in the
scale or meaning of the quantity."* The body's quantity is unchanged.

**B2. `act.rs`** — `Disclosure::InputContribution` returns, leading the enum as its comment predicts.
**This is a closed enum and its own doc says a third member is a breaking change** (`act.rs:285-288`)
— restoring a removed member is the case that comment anticipated, but confirm the generated
artifacts agree rather than assuming.

**B3. `validate/mod.rs`** — `("query_follow_from", "__temper_ungated_follow_from")` joins
`CALLABLE_FRAGMENTS`, and the doc comment's *"`follow-from` and `survey` are ABSENT"* paragraph
becomes wrong the moment it does. Amend it; leaving it is the defect class this repo keeps paying for.

**B4. `capability.rs` — retire two refusals, ASSERT the survivors.** `:361-368`'s unconditional
`edge_filter` refusal goes; `:415-419`'s per-act check is what remains and must be **witnessed**
(an act without `FilterField::Edge` still refuses). `:353-360`'s `properties` refusal **stays** —
spec §7 is OPEN and property predicates are explicitly **not in scope** (task's *NOT in scope*).

> The task says surviving refusals are **asserted rather than deleted**. A refusal with no test is
> indistinguishable from a refusal that was removed.

**Verify B:** `cargo nextest run -p temper-core --test query_validate_seam` and
`--test act_door_coverage_reachability` (the latter reads `door_coverage` and will move).

---

### C — the compiler, the row, and the hit

**Tag: EXTEND** — a fourth column on a shape that carries three. Authorized by spec §3.

**C1. `query_plan.rs`** — `EMIT_FOLLOW_FROM` constant beside the three · a `CoreCall::Walk` **variant**
(never a second emitter — `:490-500` states why: *"the whole security property … is that it is the
one place `VISIBLE_IDS` and `PRINCIPAL_BIND` are written"*) · a real arm replacing the `_` placeholder
fall-through for this act.

**C2. `via` through the shared column list.** `final_select` (`:1022-1053`) projects one column list
across hit arms, tally arms **and** the empty fallback. Adding `via` means adding it to all three —
the tally's is `NULL::jsonb` **by construction**, which is the same rule already stated there: *"A
tally carries how many, never which."*

**Decide and state, do not discover:** whether every act stage's CTE projects `via` (`NULL::jsonb`
for the non-walk acts, keeping act stages column-uniform) or whether `final_select` emits it
per-stage. Combinator stages project `SELECT id, kind` only (`:996`) and are **not returnable**
(`returns` refuses them as `stage_not_returnable`), so uniformity is only required among act stages.

**C3. `HitRow` gains `via`** (`query_exec.rs:31-37`), and **`ResourceHit` gains `via`**
(`hits.rs:158`). **Typed struct, not `serde_json::Value` on the wire** — CLAUDE.md's first code-quality
rule. Define `ViaEntry` in temper-core beside `EdgeFilter` with the same four derives
(`utoipa` / `ts_rs` / `schemars` / serde). The DB boundary may carry `jsonb`; the contract may not.

**The absent-vs-null question is already ruled and this field inherits the reasoning, not the
answer.** `located_at` is PRESENT-null by F4 (`hits.rs:180-193`) because *"the null unambiguously
means not declared."* `via` is a **collection**, and an empty array is not the same claim as "this
act does not disclose origin". Settle it explicitly against F4's rule and say which it is in the
field's doc comment.

**Verify C:** `cargo nextest run -p temper-substrate --test query_plan_compile`; then the
**`generated-artifacts` skill** — `cargo make openapi` and `cargo make generate-ts-types` both
restale here, and `cargo make check` gates them.

---

### D — end-to-end, and the door

**D1.** A composition through `/api/query` that seeds a walk from a `find-resources-with` stage and
reads `via` off the hits. This is the first witness that `door_coverage: Serves` is **true** rather
than declared.

**D2.** Re-run `cargo make check`; `cd packages/temper-ui && bun run check` if a generated shared
type moved (CLAUDE.md warns `cargo make check` does not cover temper-ui).

**D3.** `/code-review` over the branch.

---

## 3. Declared not in scope

Carried from the task, restated so a later session does not read silence as permission:

- **`survey`** · **the MCP query tool** · **the visibility-hoist strategy**.
- **Edge property predicates** — [EdgeFilter grows property predicates](./01a000c2-033c-7451-8b13-b7aa7469d217),
  which rides *after* this makes an edge filter reachable at all. `capability.rs:353`'s `properties`
  refusal stays for that reason.
- **Spec §7 stays OPEN.** Nothing here rules it.
- **`BoundTerm::Depth`** — refused, not deferred (§2.1). If it ever returns it is additive.

## 4. Risks

- **A3 is the one step that can block a deploy.** Its guard is written into the step: if the
  byte-identical re-point cannot be written, fall back to two functions and report it. Never `DROP`.
- **The `via` column touches a shared column list**, so C2 is the step most likely to break stages
  that have nothing to do with this act. It is sequenced after B so the act is already reachable and
  a failure is attributable.
- **A5's test 7** (the re-point preserves behaviour) is the one that must not be "fixed" by editing
  the assertion. If an incumbent test needs changing, the delegation is wrong.
