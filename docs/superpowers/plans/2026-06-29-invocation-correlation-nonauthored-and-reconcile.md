# Invocation correlation on non-authored writes + facet_set/reconcile authorship

> **For agentic workers:** REQUIRED SUB-SKILLS: `superpowers:test-driven-development` per chunk and `hybrid-execution` (Variant A inline, targeted subagents). Steps use checkbox (`- [ ]`) tracking. This plan is grounded per `~/.claude/skills/temper/guidance/implementation-grounding.md` — every step carries a **CONFORM / EXTEND / AMEND** tag with a cited `file:line` anchor; treat the cited anchors as the only pre-grounded facts and re-verify anything uncited. Gate each chunk on `cargo make check` + the chunk's tests; full workspace + e2e at PR-prep; `/code-review` (CQ-* lens) before merge.

**Task:** `invocation-correlation-on-non-authored-writes---facet-set-reconcile-authorship-019f10c5-95ef-7a41-a9d1-ebe8cc03c03c` (temper context, mode `build`, effort `large`). The **Follow-on** explicitly tracked in the parent plan `docs/superpowers/plans/2026-06-28-act-authorship-invocation-surface.md:121-123` (merged as **PR #202**, `git log c665a058`). The parent vertical (Chunks A–D) threaded authorship + invocation through the **three authored acts that already had `fire_with`-threaded SQL** (`resource_create`, `relationship_assert`, `relationship_fold`) with **zero new migrations**. This task closes the remaining gap so a run's act-list is **complete**.

## Goal

After this task, **every** event a run can fire carries its `invocation_id` (and optional authorship), so `invocation_show` answers *"what did this run author, and with what confidence?"* with **no gaps**:

1. **Non-authored write correlation** — `update` / `delete` / `retype` / `reweight` fire with the default `EventContext` today, so they never appear in a run's act-list. A single `temper resource update` fans out to **up to four** sub-events (`writes.rs:226-273`: `block_mutate`, `property_set`, `resource_update`, `resource_rehome`); for **complete** correlation (the resolved scope decision, 2026-06-29) every one must carry the run's `invocation_id`.
2. **facet_set / reconcile authorship** — `reconcile_cognitive_map` **already mints an invocation envelope** (`db_backend.rs:1228` `open_invocation_in_tx`, closed at `:1250`), but every mutation inside `reconcile_apply` fires with `EventContext::default()`, so a reconcile run's act-list is **empty**. Thread the already-minted `inv` (+ caller-supplied authorship) into every act `reconcile_apply` fires.

## Resolved decisions (with the user, 2026-06-29)

1. **Scope = COMPLETE.** Thread correlation through every event a non-authored write **and** the reconcile loop can fire — not just the headline `resource_update`/`resource_delete`/`retype`/`reweight`. Rationale: the task's stated goal is *"a run's act-list is COMPLETE"*; a body-only or rehome-only update that didn't correlate would re-open the exact gap this task closes.
2. **Process = plan-first, then `hybrid-execution`** (Variant A inline, targeted subagents), TDD per chunk, `cargo make check` gate per commit, `/code-review` at end-of-branch.
3. **Reconcile authorship grain = one ActContext for the whole run.** Reconcile's internal acts all carry `{ invocation: Some(inv), authorship: cmd.act.authorship }` — the run's own server-minted envelope, plus optional caller authorship. **Out of scope:** linking the reconcile's envelope to an *outer* caller invocation as `parent` (reconcile opens `parent: None` today — `db_backend.rs:1234`).

## Load-bearing invariants (travel verbatim — GD-4)

- *"Authorship rides `kb_events.metadata`, NOT the payload — invisible to projections (and thus affinity math) by construction, and survives replay verbatim."* (`temper-core/src/types/authorship.rs:11-13`; 06-18 plan §arch)
- *"Auth before writes — authorization checks go before any mutations."* (CLAUDE.md Code Quality Rules)
- **Orthogonality (load-bearing, from parent plan A4):** the write's own authz is unconditional and unchanged; the invocation check is **correlation-integrity only** — it fires *only when* `invocation` is `Some`, exists solely to stop an act *claiming* a run it can't see, and is **additive to, never a substitute for, authn/authz.** Reuse the existing `check_act_invocation` helper (`db_backend.rs:693`) verbatim — do not author a second gate.
- *"Full MCP+API+CLI surface parity is ALWAYS the intention — one shared logic layer, MCP-first ok but API+CLI same vertical."* (memory `feedback_full_surface_parity_always`)
- **Append-only migration / byte-identical drift rule:** extend SQL functions via a **new** forward migration; **never** edit a born migration (`20260624000002_canonical_functions.sql`, `20260629000001_cogmap_charter_set.sql`). (CLAUDE.md "additive-only-on-`main`"; memory `project_ws2_access_scoping_and_edge_home_gap` drift-guard.)

## Grounding evidence (quoted — GD-1)

**The shared carrier is fully built (parent PR #202).**
- `temper-core/src/types/authorship.rs` — `ActContext { invocation: Option<InvocationId>, authorship: Option<AgentAuthorship> }`, `ActInput` (flat surface fields) + `ActInput::into_act_context`, `ConfidenceBand`, `AgentAuthorship`. **CONFORM** — reuse as-is.
- `temper-substrate/src/events.rs:418-424` — `fire_with(conn, action, ctx)`; `ctx_meta = ctx.metadata_json()?`, `ctx_inv = ctx.invocation_uuid()`. `fire()` (`:411`) delegates with `EventContext::default()`.
- `temper-substrate/src/events.rs:487-494` (`resource_create`), `:521-527` (`relationship_assert`), `:547-553` (`facet_set`) — the **template**: authored arms bind `ctx_meta, ctx_inv` as `$4,$5` / `$3,$4`.

**The non-authored arms ignore ctx (the severed links).**
- `events.rs:744-756` `ResourceDelete` → `resource_delete($1,$2)`; `:759-778` `ResourceUpdate` → `resource_update($1,$2)`; `:781-798` `ResourceRehome` → `resource_rehome($1,$2)`; `:801-820` `RelationshipRetype` → `relationship_retype($1,$2)`; `:823-840` `RelationshipReweight` → `relationship_reweight($1,$2)`. `BlockMutate` and `PropertySet` arms similarly call 3-/2-arg SQL. **All drop `ctx_meta`/`ctx_inv`.**

**The SQL functions were born without the metadata params.**
- `migrations/20260624000002_canonical_functions.sql` — `_event_append(...)` (`:765-787`) **already accepts** `p_metadata jsonb DEFAULT '{}'`, `p_invocation uuid DEFAULT NULL` and writes them to `kb_events.metadata`/`invocation_id` (`:781-784`) — **CONFORM, the sink is ready.** But the callers don't pass them: `resource_delete(p_payload, p_emitter)` (`:1064`), `resource_update` (`:1093`), `resource_rehome` (`:1122`), `property_set` (`:1156`), `relationship_retype` (`:1186`), `relationship_reweight` (`:1210`), `block_mutate(p_payload, p_content, p_emitter)` (`:957`). Plus `cogmap_charter_set(p_payload, p_content, p_emitter)` (`migrations/20260629000001_cogmap_charter_set.sql:54`).
- `facet_set` (`:889-901`) **already** has `p_metadata`/`p_invocation` and forwards them — **no migration**; the gap is purely that `writes::set_facet_in_tx` uses `fire()` not `fire_with()`.

**The writes layer — `_with` template + the fns to thread.**
- Template: `writes.rs:113,120` (`create_resource` / `create_resource_with`), `:549,554` (`assert_relationship` / `_with`), `:628,633` (`fold_relationship` / `_with`) — base delegates `EventContext::default()`, `_with` takes `ctx: EventContext`.
- To thread (each currently `fire(...)`): `update_resource_in_tx:194` (fires the 4 sub-events `:226,238,252,265`), `delete_resource_in_tx:289`, `retype_relationship:580`, `reweight_relationship:602`, `set_facet_in_tx:407`, `set_property_in_tx:444`, `mutate_block:467`, `create_kernel_resource_in_tx:336`, `assert_kernel_edge_in_tx:511`, `set_charter_in_tx:371`.

**The command + backend layer.**
- `temper-workflow/src/operations/commands.rs` — `act: ActContext` present on `CreateResource:59`, `AssertRelationship:148`, `FoldRelationship:179`; **absent** on `UpdateResource:87`, `DeleteResource:111`, `RetypeRelationship:155`, `ReweightRelationship:164`, `ReconcileCognitiveMap:208`. **EXTEND** — add `act: ActContext` (default empty) to these five.
- `db_backend.rs:693` `check_act_invocation(...)` — the reusable correlation-integrity gate (404 unreadable/absent, 409 non-open). Used by the authored methods at `:741,:1068,:1171`. **CONFORM** — reuse for the four non-authored methods.
- Non-authored DbBackend methods that need the gate + `_with` call: `update_resource:859` (calls `writes::update_resource:967`), `delete_resource:988` (`:999`), `retype_relationship:1115` (`:1130`), `reweight_relationship:1142` (`:1156`).
- `reconcile_apply:359` fires, in order: `create_kernel_resource_in_tx:443`, `set_property_in_tx:462`, `set_facet_in_tx:476`, `update_resource_in_tx:490`, `assert_kernel_edge_in_tx:557`, `delete_resource_in_tx:587`, `fold_relationship_in_tx:609`, `set_charter_in_tx:666` — **every one must take the run ctx.**

**Readback already exposes per-act invocation + authorship (parent Chunk D).** `invocation_show`'s act records carry `invocation_id` + decoded `metadata` authorship — **CONFORM, no readback change.** Once stamped, the new acts surface automatically.

---

## Chunk 1 — Migration + substrate SQL binding (the severed SQL links)

New append-only forward migration extends the 8 non-authored/charter SQL functions; events.rs binds ctx into their arms; regenerate the `temper_next` `.sqlx` cache.

**1.1 — New migration `migrations/20260629000002_nonauthored_act_correlation.sql`.**
- **AMEND** (disk: `20260624000002_canonical_functions.sql` born without params; spec: this plan §Resolved-1 + append-only rule). For each of `resource_update`, `resource_delete`, `resource_rehome`, `property_set`, `relationship_retype`, `relationship_reweight` (2-arg) and `block_mutate`, `cogmap_charter_set` (3-arg): `DROP FUNCTION <fn>(<existing sig>); CREATE FUNCTION <fn>(<existing params>, p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL) ...` with the body identical to the born version except the inner `_event_append(...)` call gains `p_metadata => p_metadata, p_invocation => p_invocation` (mirror `resource_create:752-754`). **Copy each born body verbatim** from the cited lines — re-`Read` them at implement time; do not paraphrase the projection logic.
  - ⚠️ Postgres: adding a parameter changes the function's identity, so `CREATE OR REPLACE` would create an *overload* (leaving the old arity callable and ambiguous). Use `DROP FUNCTION ... ; CREATE FUNCTION ...`. These fns have no view/trigger dependents (called only from Rust), so a plain `DROP` (no `CASCADE`) succeeds — verify with `psql` (GD-2).
- **Gate (GD-2, executable):** apply the migration to the dev DB (`cargo make docker-up` already running) and confirm via `psql` that each function now has the 2 extra params and that a hand-call writes `metadata`/`invocation_id` into `kb_events`.

**1.2 — events.rs arms bind ctx.**
- **EXTEND** (`events.rs`): in the `ResourceDelete:748`, `ResourceUpdate:770`, `ResourceRehome:790`, `RelationshipRetype:812`, `RelationshipReweight:832`, `BlockMutate:698` (SQL `block_mutate($1,$2,$3)` at `:711`), `PropertySet:586` (SQL `property_set($1,$2)` at `:601` — the `PropertySet` arm, **not** the legacy `PropertyAssert` 2-arg `facet_set` arm at `:560`), and `CharterSet:722` (SQL `cogmap_charter_set($1,$2,$3)` at `:733`) arms, append `, ctx_meta, ctx_inv` to the `sqlx::query_scalar!`/`query!` call and the SQL arg list (`$N`). Mirror `resource_create:487-494`. After this, the now-unused-warning on `ctx_meta`/`ctx_inv` for these arms disappears; a plain `fire()` caller passes `EventContext::default()` → `{}` / `NULL` → byte-identical prior behavior (regression-safe).
- **Gate:** `cargo make prepare-next` (the `temper_next` per-crate `.sqlx` cache — these are `!`-macro queries whose arg count changed); `cargo build -p temper-substrate`.

**1.3 — substrate stamp test (TDD).**
- **EXTEND** test (`temper-substrate/tests/`, mirror `invocation_readback.rs:68-82`): for each non-authored action fired via `fire_with(…, EventContext { invocation: Some(i), authorship: Some(a) })`, assert the resulting `kb_events` row carries `invocation_id = i` and `metadata` = the authorship JSON; and a `fire()` (default) call leaves `invocation_id IS NULL`, `metadata = '{}'`.
- **Gate:** `cargo nextest run -p temper-substrate --features artifact-tests`.

**Validation gate (Chunk 1 done):** every non-authored/charter event kind can be stamped at the SQL+substrate layer; default path byte-identical; `.sqlx` cache regenerated.

---

## Chunk 2 — substrate writes: `EventContext` threading

Give the writes-layer fns a way to *supply* a ctx. Mirror the `_with` convention; thread ctx into the `_in_tx` workhorses (so reconcile and the public wrappers share one path).

**2.1 — thread `ctx: EventContext` into the `_in_tx` workhorses.**
- **EXTEND** (`writes.rs`, spec: parent A2 pattern `:120-145`): add a `ctx: EventContext` param to `update_resource_in_tx:194`, `delete_resource_in_tx:289`, `set_facet_in_tx:407`, `set_property_in_tx:444`, `create_kernel_resource_in_tx:336`, `assert_kernel_edge_in_tx:511`, `set_charter_in_tx:371`, and the `mutate_block:467` body path; switch each inner `fire(…)` → `fire_with(…, ctx.clone())` (clone — an `_in_tx` fires multiple events; `EventContext` is `Clone`). For the top-level public fns `retype_relationship:580` / `reweight_relationship:602` (no `_in_tx` split), add the ctx param directly.

**2.2 — public `_with` wrappers + default-preserving bases.**
- **EXTEND**: add `update_resource_with` / `delete_resource_with` / `retype_relationship_with` / `reweight_relationship_with` / `set_facet_with` / `set_property_with` (+ `mutate_block_with` if it has a non-reconcile caller) taking `ctx: EventContext`; keep the existing public fn as the `EventContext::default()` delegate. Mirror `create_resource:110-118`.
- **Gate:** `cargo build -p temper-substrate`; `cargo nextest run -p temper-substrate --features artifact-tests` (existing writes tests still green — default delegation preserves behavior).

**Validation gate (Chunk 2 done):** every write path can carry a ctx; all default-path callers unchanged; compiles clean.

---

## Chunk 3 — command layer + DbBackend (correlation-integrity + reconcile threading)

**3.1 — commands carry `act`.**
- **EXTEND** (`commands.rs`): add `pub act: ActContext` (default empty via `#[serde(default)]` if the struct is `Deserialize`-constructed) to `UpdateResource:87`, `DeleteResource:111`, `RetypeRelationship:155`, `ReweightRelationship:164`, `ReconcileCognitiveMap:208`. `Backend` trait signatures unchanged (methods take the whole `cmd`).

**3.2 — four non-authored DbBackend methods: gate + `_with`.**
- **CONFORM** (`db_backend.rs`, reuse `check_act_invocation:693` + the orthogonality invariant): in `update_resource:859`, `delete_resource:988`, `retype_relationship:1115`, `reweight_relationship:1142`, after the existing write authz and **before** the write, call `self.check_act_invocation(cmd.act.invocation).await?`; build `EventContext { invocation: cmd.act.invocation, authorship: cmd.act.authorship }`; call the `writes::*_with(...)` variant. Authorship may ride with `invocation: None` (no gate in that branch — the helper already no-ops on `None`).
- **Tests** (`temper-api --features test-db`): (a) each act under an open invocation stamps `invocation_id` + `metadata`; (b) invocation whose cogmap the caller can't read → 404; (c) closed invocation → 409; (d) `None` act → unchanged (regression). Mirror the authored-method tests added in parent Chunk A.

**3.3 — reconcile threads its own envelope into every internal act.**
- **CONFORM/EXTEND** (`db_backend.rs:1202` reconcile_cognitive_map + `reconcile_apply:359`): the envelope `inv` is already minted (`:1228`). Build `let run_ctx = EventContext { invocation: Some(inv), authorship: cmd.act.authorship.clone() };` and pass it down to `reconcile_apply`, which forwards it to **every** `writes::*_in_tx` call it makes (`:443,462,476,490,557,587,609,666`). Use the `*_with`/ctx-param variants from Chunk 2. (Note `inv` is currently created after `reconcile_apply`'s preflight but the apply call is at `:1244` — `inv` at `:1228` precedes it, so ordering is fine.)
- **Test** (`temper-api --features test-db`): reconcile a small manifest, then assert the `kb_events` rows it produced (facet_set, resource_update sub-events, edge asserts, charter) all carry `invocation_id = inv`.
- **Gate:** `cargo make check` + the new tests.

**Validation gate (Chunk 3 done):** backend proves stamp + both auth rejections + `None` regression for the four writes; reconcile's internal acts all carry the run's `inv`.

---

## Chunk 4 — MCP surface (agent-first parity)

- **EXTEND** (`temper-mcp/src/tools/`, mirror parent Chunk B): add the flat `ActInput` fields (`invocation_id` + `reasoning`/`confidence`/`rationale`/`persona`/`model`, schemars-derived from temper-core) to the update / delete / retype / reweight tool inputs, and to the admin reconcile tool input. Build `ActContext` via `ActInput::into_act_context` and set `cmd.act`.
- **e2e** (`tests/e2e`): `invocation_open → update_resource(under it) → invocation_show` shows the update's acts (incl. a body-only update's `block_mutate`) carrying authorship; same for delete/retype/reweight; reconcile run's act-list is non-empty. Mirror PR #198/#202 e2e harness.
- **Gate:** `cargo make test-e2e` (+ `test-e2e-embed` if the path touches embed — body updates do).

---

## Chunk 5 — HTTP API + temper-client + CLI parity

- **EXTEND** (mirror parent Chunk C): wire DTOs for update/delete/retype/reweight + admin reconcile gain optional `invocation_id` + authorship; handlers build `ActContext` onto the command; `temper-client` forwards the fields; CLI gains the optional flags (`--invocation`, `--confidence`, `--reasoning`, `--rationale`, `--persona`, `--model`) on `resource update`/`resource delete`/`edge retype`/`edge reweight` + the admin reconcile command — available to any caller, default `None` (parent Decision 3).
- **Gate:** `cargo make test-e2e` across API + CLI; `cargo make check`.

---

## Chunk 6 — e2e acceptance + projection-invisibility regression

- **EXTEND**: round-trip acceptance — *"every act a run fires (incl. body-only update, rehome, retype, reweight, and every reconcile mutation) is queryable by `invocation_id` and carries its authorship in metadata, invisible to affinity math."*
- **Acceptance invariant test (GD-4):** author the new act kinds with authorship, then assert the region/affinity projection over the cogmap is byte-identical to the same corpus without authorship (reuse the parent Chunk D projection-invisibility harness).
- **Gate:** `cargo make test-all` + `cargo make test-e2e`; `/code-review` (CQ-* lens) on the branch; regenerate any caches (`cargo make prepare-api` if test SQL added).

---

## Migration / cache ritual (don't skip — memory `project_sqlx_per_crate_cache_for_feature_gated_tests`)

- After Chunk 1 SQL: `cargo make prepare-next` (substrate `temper_next` cache).
- After new migrations, before integration tests see them: `cargo clean -p temper-api` (memory `project_sqlx_migrate_macro_stale_cache`).
- After Chunk 3/6 test SQL in temper-api / e2e: `cargo make prepare-api` / `cargo make prepare-e2e`.
- `cargo make check` runs `SQLX_OFFLINE=true` — it is the honest probe of the committed caches.

## Out of scope (track separately if wanted)

- Linking a reconcile envelope to an *outer* caller invocation as `parent` (reconcile is top-level `parent: None`).
- The legacy `PropertyAssert` 2-arg `facet_set($1,$2)` arm (`events.rs:560`) — not on a correlated write path; left as-is.
