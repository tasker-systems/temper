# Surface per-act agent-authorship + invocation_id through the write surfaces

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:test-driven-development` per step and `hybrid-execution` (Variant A inline, targeted subagents) to execute. Steps use checkbox (`- [ ]`) tracking. This plan is grounded per `~/.claude/skills/temper/guidance/implementation-grounding.md` — every step carries a **CONFORM / EXTEND / AMEND** tag with a cited `file:line` anchor; treat the cited anchors as the only pre-grounded facts and re-verify anything uncited.

**Task:** `surface-per-act-agent-authorship---invocation-id-threading-through-write-surfaces-019f0e28-1750-7490-919f-5e51c92c8391` (temper context, mode `plan`, effort `large`). The **act-level half** of the agent-invocation accountability grain. The **run-level envelope half** is Task R, merged as **PR #198** (`git log`: `378877ba Merge … jct/invocation-envelope-surface`).

## Goal

Reconnect the severed vertical from the caller surfaces (MCP / HTTP API / CLI) down to the substrate's `fire_with` / `EventContext`, so each individual **authored act** is (a) attributable to the **invocation** it ran under (`kb_events.invocation_id`) and (b) carries graded-confidence **agent authorship** (`kb_events.metadata`). Without this, the envelope (PR #198) is a hollow shell: it records that a run launched and ended, but no act carries the `invocation_id`, so *"what did this run author, and with what confidence?"* is unanswerable.

## Architecture (the spine)

The substrate end is **already built** (06-18 plan, now in `temper-substrate`). The gap is everything above the SQL:

```
MCP input / HTTP DTO / CLI flags        ← Chunk B / C: gain optional authorship + invocation
  → temper-workflow command struct       ← Chunk A: carry Option<ActContext>
  → DbBackend write method                ← Chunk A: per-act auth check + ActContext → EventContext
  → temper-substrate writes::*(…, ctx)    ← Chunk A: add ctx param; fire() → fire_with()  ⟵ THE SEVERED LINK
  → fire_with(conn, action, ctx)          ← DONE (events.rs:395)
  → SQL resource_create(…, p_meta, p_inv) ← DONE (4 authored-act fns thread metadata + invocation)
  → kb_events.metadata + kb_events.invocation_id
```

## Grounding evidence (quoted — GD-1)

**Substrate is ready; the writes layer never engages it.**
- `crates/temper-substrate/src/events.rs:368-371` — `EventContext { authorship: Option<AgentAuthorship>, invocation: Option<InvocationId> }`.
- `events.rs:388-389` — `fire()` delegates to `fire_with(conn, action, EventContext::default())`.
- `events.rs:464-470, 498-507, 524-530, 662-668` — the four authored-act arms (`ResourceCreate`, `RelationshipAssert`, `FacetSet`, `RelationshipFold`) bind `ctx_meta, ctx_inv` into `resource_create($1..$5)` / `relationship_assert($1..$4)` / `facet_set($1..$4)` / `relationship_fold($1..$4)`.
- `crates/temper-substrate/src/writes.rs:117` — `create_resource` calls `fire(&mut tx, SeedAction::ResourceCreate{…})` — **default ctx, the severed link**. Same at `writes.rs:513` (`assert_relationship`) and the fold path (`fold_relationship_in_tx`).

**The shared-type home.**
- `crates/temper-core/Cargo.toml` — leaf crate (no `temper-*` path deps). `crates/temper-workflow/Cargo.toml` + `crates/temper-substrate/Cargo.toml` both `temper-core = { path = "../temper-core" }`.
- `crates/temper-core/src/types/ids.rs:188` — `InvocationId` **already lives in temper-core** (shared today).
- `crates/temper-substrate/src/payloads.rs:481,492` — `ConfidenceBand` + `AgentAuthorship` live in **temper-substrate** (NOT visible to temper-workflow commands).

**The command + backend layer.**
- `crates/temper-workflow/src/operations/commands.rs` — write commands with an existing `origin: Surface` field on each: `CreateResource:24-54`, `UpdateResource:79-99`, `AssertRelationship:131-139`, `FoldRelationship:162-166` (+ retype/reweight/delete/open/close/reconcile).
- `crates/temper-workflow/src/operations/backend.rs:49-125` — `Backend` trait write methods, each `async fn …(&self, cmd: <Cmd>) -> Result<CommandOutput<_>, TemperError>`. No signature change needed — commands carry the new field.
- `crates/temper-api/src/backend/db_backend.rs:624` (`create_resource`), `:944` (`assert_relationship`) — build `writes::CreateParams` / `writes::AssertParams` with only `emitter`; no `EventContext`.

**The reusable auth predicate (PR #198 template).**
- `migrations/20260624000002_canonical_functions.sql:274` — `anchor_readable_by_profile(p_profile, p_anchor_table, p_anchor_id)`; `'kb_cogmaps' → cogmap_readable_by_profile`.
- `crates/temper-substrate/src/readback/mod.rs:851,911` + `db_backend.rs:1196` — invocation show/list/close already gate on `anchor_readable_by_profile($, 'kb_cogmaps', i.originating_cogmap_id)`.
- `migrations/20260624000001_canonical_schema.sql:512-527` — `kb_invocations.originating_cogmap_id UUID NOT NULL` is the auth key to join through.

**The readback shape.**
- `readback/mod.rs:841-893` — `InvocationShowRow { …, acts: Vec<InvocationActRecord> }`; the acts query (`:861-878`) selects `e.id, et.name, e.emitter_entity_id, e.occurred_at WHERE e.invocation_id = $1` — it **does not yet expose `e.metadata` or `e.invocation_id`**.

**The confirmed gap (per-act stamping is genuinely unsurfaced).**
- `tests/e2e/.../invocation_envelope_e2e.rs:85` (PR #198) — *"per-act domain threading is a separate task."*
- `crates/temper-substrate/tests/invocation_readback.rs:68-82` — the ONLY `fire_with(…, EventContext{ invocation: Some(…) })` caller is a test; no production write path constructs a non-default `EventContext`.

## Load-bearing invariants (travel verbatim — GD-4)

- *"Authorship rides `kb_events.metadata`, NOT the payload — invisible to projections (and thus affinity math) by construction, and survives replay verbatim."* (06-18 plan `docs/superpowers/plans/2026-06-18-invocation-envelope-and-authorship-metadata.md:7`)
- *"Auth before writes — authorization checks go before any mutations."* (CLAUDE.md Code Quality Rules)
- *"Full MCP+API+CLI surface parity is ALWAYS the intention — one shared logic layer, MCP-first ok but API+CLI same vertical."* (memory `feedback_full_surface_parity_always`)
- *"Typed structs over inline JSON; shared types at boundaries live in temper-core with ts-rs derives."* (CLAUDE.md)

## Resolved design decisions (this session, with the user)

1. **Act scope = authored acts now; correlation-on-the-rest is a follow-on.** Surface authorship + invocation on the authored acts the substrate already threads. Of the authored-4, only **`resource_create`, `relationship_assert`, `relationship_fold`** have direct caller-facing write commands; **`facet_set` has no caller surface** (it is driven inside `reconcile_cognitive_map` — `writes.rs:379`), so facet/reconcile authorship is part of the follow-on, not this vertical.
2. **Threading shape = one shared `ActContext` struct** (`{ invocation: Option<InvocationId>, authorship: Option<AgentAuthorship> }`) in temper-core, attached as a single `Option<ActContext>` field per write command, mapping 1:1 to substrate's `EventContext`. One place to run the per-act auth check; honors the params-struct rule.
3. **CLI flags available to any caller.** Agent-driven CLI is the *expected* case, not a *restricted* one — anything an agent can do via the CLI, a human can. The flags are plain optional flags defaulting to `None`; no gating by caller type.

---

## Chunk A — Shared types + substrate writes spine + per-act auth (no caller surface yet)

The load-bearing vertical. After this chunk every layer can carry an `ActContext` and the per-act auth check is enforced, but no surface can *supply* one yet (all default `None`) — proven by backend/substrate tests. PR-sized, one session.

**A1 — Shared wire types in temper-core.**
- **AMEND** `crates/temper-substrate/src/payloads.rs:481,492` → move `ConfidenceBand` + `AgentAuthorship` into `crates/temper-core/src/types/` (e.g. `authorship.rs`), with `serde` + the gated `ts-rs` / `schemars` (`mcp`) / `utoipa` (`web-api`) derives matching the other boundary types. Authorization: CLAUDE.md *"the wire type lives in temper-core with ts-rs derives… both sides share the generated type."* temper-substrate (depends on temper-core) re-uses the temper-core type in `EventContext` + when serializing to `kb_events.metadata`; delete the substrate-local copy.
- **EXTEND** define `ActContext { invocation: Option<InvocationId>, authorship: Option<AgentAuthorship> }` in temper-core. `InvocationId` is already there (`ids.rs:188` — CONFORM).
- Gate: `cargo make generate-ts-types` clean; `cargo build -p temper-core -p temper-substrate`.

**A2 — substrate writes accept `EventContext`.**
- **EXTEND** `writes::create_resource` / `assert_relationship` / `fold_relationship_in_tx` (`writes.rs:110,500,~588`): add a `ctx: EventContext` param to each (or fold into the existing `CreateParams`/`AssertParams` structs — they are already over the 5-param threshold, so a struct field is the right home), and change the inner `fire(…)` → `fire_with(…, ctx)`. `fire_with` exists (`events.rs:395`) — CONFORM. Non-authored writes (`update`/`delete`/`retype`/`reweight`/`property_set`) keep `fire()`.
- Gate: `cargo make prepare-next` is **not** needed (no SQL string change — the SQL fns already take `$4/$5`); `cargo nextest run -p temper-substrate --features artifact-tests`.

**A3 — temper-workflow commands carry `Option<ActContext>`.**
- **EXTEND** `commands.rs` — add `act: Option<ActContext>` to `CreateResource:24-54`, `AssertRelationship:131-139`, `FoldRelationship:162-166`. `Backend` trait signatures unchanged (CONFORM `backend.rs:49-125` — methods take the whole `cmd`).

**A4 — DbBackend: per-act correlation-integrity check + `ActContext` → `EventContext`.**
- **Orthogonality invariant (load-bearing):** the write's **own authz is unconditional and unchanged** — `can_modify_resource` / context-owner resolution / `check_can_modify_next` (`db_backend.rs:954`) gate *every* write regardless of invocation. The invocation check below is **correlation-integrity only**: it fires *only when* `invocation_id` is present, exists solely to stop an act *claiming* a run it can't see, and is **additive to, never a substitute for, authn/authz**. The naive-implementation risk to avoid is the inverse: do NOT treat "valid invocation_id + can read its cogmap" as authorizing the write — both checks run independently, and a caller with no `invocation_id` (human, or MCP-via-agent-proxy) is fully valid.
- **CONFORM (auth-before-write + reuse predicate)** in `db_backend.rs` `create_resource:624` / `assert_relationship:944` / fold: the existing write authz runs as today. *Additionally*, when `cmd.act.invocation` is `Some`, **before** the write, `SELECT originating_cogmap_id, status FROM kb_invocations WHERE id = $invocation`; reject 404 if absent/unreadable via `anchor_readable_by_profile(profile, 'kb_cogmaps', originating_cogmap_id)` (mirror the close-path unified gate `db_backend.rs:1196`), and reject 409 if `status <> 'open'` (mirror `:1207-1211`). Then build `EventContext { authorship: cmd.act.authorship, invocation: cmd.act.invocation }` and pass to `writes::*`. Authorship may ride with `invocation: None` (a one-off attributed act) — no invocation gate in that branch.
- **Tests** (`temper-api --features test-db` + substrate): (a) an act under an open invocation stamps `kb_events.invocation_id` + `metadata.reasoning/confidence`; (b) act with an invocation whose cogmap the caller can't read → 404; (c) act onto a closed invocation → 409; (d) `None` act → unchanged behaviour (regression).
- **Gate:** `cargo make check` + the new tests green.

**Validation gate (Chunk A done):** backend test proves stamp + both auth rejections; `None`-path regression green; nothing in MCP/CLI/API changed yet.

---

## Chunk B — MCP surface (agent-first)

- **EXTEND** the write tool inputs in `crates/temper-mcp/src/tools/resources.rs` (`CreateResourceInput:21-48`) + `relationships.rs` (`AssertRelationshipInput:29-42`, `FoldRelationshipInput:65-71`): add an optional `invocation_id` + flattened authorship (`reasoning`/`confidence`/`rationale`/`persona`/`model`) — reuse the temper-core wire types (schemars-derived, `mcp` feature). Build `ActContext`, set on the command.
- **e2e** (`tests/e2e`, the `invocation_open → create_resource(under it) → invocation_show` flow): the act appears in the run's act list carrying its authorship. Mirror PR #198's e2e harness.
- **Gate:** `cargo make test-e2e` (+ `test-e2e-embed` if the path touches embed).

---

## Chunk C — HTTP API + temper-client + CLI parity

- **EXTEND** wire DTOs: `ResourceCreateRequest` (`crates/temper-workflow/src/types/resource.rs:134-146`), `AssertRelationshipRequest` + `FoldRelationshipRequest` (`crates/temper-core/src/types/relationship_requests.rs`): add optional `invocation_id` + authorship. Handlers (`temper-api/src/handlers/resources.rs:145`, `edges.rs:59,171`) build `ActContext` onto the command (CONFORM — handlers already build commands).
- **EXTEND** `temper-client` resource/edge calls to forward the new fields.
- **EXTEND** CLI (`temper-cli/src/commands/resource.rs`, `edge.rs`): optional flags — `--invocation <ref>`, `--confidence <tentative|probable|confident>`, `--reasoning`, `--persona`, `--model` — available to **any** caller, default `None`. Decision 3.
- **Gate:** `cargo make test-e2e` across API + CLI; `cargo make check`.

---

## Chunk D — Readback exposure + projection-invisibility proof

- **EXTEND** `InvocationActRecord` (`readback/mod.rs:841-893`) + its acts query (`:861-878`): expose `invocation_id` and the `metadata` authorship (decode to the temper-core `AgentAuthorship`). Surface the enriched `invocation_show` through MCP + API + CLI (the read surfaces already exist from PR #198 — CONFORM).
- **Acceptance invariant test (GD-4):** assert authorship in `kb_events.metadata` is **invisible to affinity math / projections** — author an act with authorship, then assert the region/affinity projection over that cogmap is byte-identical to the same corpus without authorship. (Reuse a substrate projection/affinity harness.)
- **Gate:** the round-trip acceptance — *"an act authored under a run is queryable by `invocation_id` and carries its authorship in metadata, invisible to affinity math."*

---

## Follow-on (tracked — NOT this task)

Create a separate build task: **invocation_id correlation on non-authored writes** (`retype` / `reweight` / `update` / `delete`) **+ facet_set / reconcile authorship**. Needs new **append-only forward migrations** extending those SQL functions with `p_metadata DEFAULT '{}'` / `p_invocation DEFAULT NULL` (byte-identical artifact rule — see `migrations/` drift guard), then threading through the `reconcile_cognitive_map` loop (`writes.rs:379`). Gives a complete run act-list. Out of scope here because Decision 1 fixed this vertical to the already-threaded authored acts with zero new migrations.

## Resolved during planning (with the user, 2026-06-28)

- **Authorship is discrete fields on the wire**, not a JSON blob — `--reasoning`, `--confidence`, `--rationale`, `--persona`, `--model`. Decider: discrete flags fold into `--help` guidance; a blob hides the shape. Apply the same discrete shape to the MCP input schema.
- **`confidence` is required iff authorship is supplied.** Substrate `AgentAuthorship.confidence` is non-`Option` (`payloads.rs`). If a caller supplies any authorship field, `confidence` is required; if they supply none, the whole `ActContext.authorship` is `None`. Verify serde behaviour when moving the type to temper-core.
- **Authorship does NOT require an invocation_id.** The `invocation_id` is a *correlation* aid, not an authz mechanism, and never a substitute for existing authn/authz. A human (or MCP-via-agent-proxy) calling the same CLI/API/MCP tools with no `invocation_id` is fully valid; authorship may ride alone as a one-off attributed act. See the A4 orthogonality invariant — the invocation check is correlation-integrity only, additive to the always-on write authz.
