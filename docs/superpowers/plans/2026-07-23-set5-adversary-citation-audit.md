# Set 5 — Adversary persona as citation auditor: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Set 3's unwritable pairwise-independence model with a citation-grain audit substrate, and ship the adversary persona that feeds it.

**Architecture:** A new event-sourced `kb_citation_audits` projection records a signed `[-1.0, 1.0]` defensibility verdict per `(citation, auditing act)`. `kb_resource_standing.indep_breadth` splits into `citation_magnitude` (distinct cited sources, monotone) and `citation_quality` (mean signed audit value, unaudited at a `-0.5` prior); `challenge_count` is redefined as audit coverage. The auditor is a new schedule in the steward Eve package running under its **own** registered machine client, dispatched through the existing persona-agnostic `kb_workflow_jobs` queue.

**Tech Stack:** PostgreSQL (sqlx migrations), Rust (temper-substrate / temper-services / temper-workflow / temper-api / temper-mcp / temper-cli), TypeScript (Eve agent, vitest).

**Spec of record:** [`docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md`](../specs/2026-07-23-set5-adversary-citation-audit-design.md). **Read the spec section each task cites — this plan is an index over it, not a replacement for it.**

## Global Constraints

- **Shipped migrations are immutable.** Every change to a shipped SQL object is a NEW additive migration doing DROP+CREATE (or CREATE OR REPLACE where the signature is unchanged). Never edit `20260721000010_evidential_standing_memo.sql`.
- **Migration numbering:** latest on `main` is `20260722000100`. This plan uses `20260723000010`, `20260723000020`, `20260723000030` — gaps left deliberately for concurrent sibling sessions.
- **`--all-features`** for all builds and clippy. `#[expect(lint, reason = "...")]`, never `#[allow]`.
- **Typed structs over `serde_json::json!()`** for any known shape. Params structs past 5 domain args.
- **Auth before writes.** Authorization precedes any mutation, always.
- **No `sqlx::query!()` inline in a surface.** Persistence lives in temper-substrate / temper-services; surfaces dispatch one operations command.
- **After any SQL change:** `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services`, then `cargo make prepare-e2e` (per-crate last).
- **Standing is not truth** (spec §Bedrock). An audit assesses the defensibility a citation confers — never what a source says, never whether a claim is true. Any step that drifts toward the latter is wrong.
- **Per-step grounding tags:** each task declares CONFORM (honor an existing constraint — cite it), EXTEND (build beyond an affordance — cite the spec section), or AMEND (deliberately change a shipped thing — cite both).

---

## File Structure

**Migrations (new):**
- `migrations/20260723000010_citation_audits.sql` — audit table, event type, projector, entry function.
- `migrations/20260723000020_standing_citation_components.sql` — the AMEND of Set 3's components: new columns, new component producers, re-thresholded band, rewritten refresh + shape read, retirement of the pairwise objects.
- `migrations/20260723000030_audit_drift_sweep.sql` — the auditor's selection function.

**Rust (modify):**
- `crates/temper-substrate/src/payloads.rs` — `CitationAudited` typed payload.
- `crates/temper-substrate/src/events.rs` — `SeedAction::CitationAudit` arm + `Fired` variant.
- `crates/temper-substrate/src/writes.rs` — `CitationAuditParams` + `record_citation_audit[_with]`.
- `crates/temper-substrate/src/readback/mod.rs` — `StandingShapeRow` field change; new `pub` visibility predicate.
- `crates/temper-core/src/types/standing.rs` — `StandingShape` wire fields.
- `crates/temper-core/src/types/citation_audit.rs` — **new**: the audit request wire type.
- `crates/temper-workflow/src/operations/commands.rs` + `backend.rs` — `RecordCitationAudit` command + trait method.
- `crates/temper-services/src/authz/audit_gate.rs` — **new**: `AuditAuthority`.
- `crates/temper-services/src/services/citation_audit_service.rs` — **new**.
- `crates/temper-services/src/services/auditor_service.rs` — **new**: sweep + dispatch.
- `crates/temper-services/src/backend/db_backend.rs` — trait impl + refresh wiring.
- `crates/temper-api/src/handlers/citation_audits.rs`, `handlers/auditor.rs` — **new**.
- `crates/temper-mcp/src/tools/citation_audits.rs` — **new**.
- `crates/temper-cli/src/commands/resource.rs` — evidence renderer.

**TypeScript (new):**
- `packages/agent-workflows/steward/agent/schedules/auditor.ts`
- `packages/agent-workflows/steward/agent/channels/auditor-worker.ts`
- `packages/agent-workflows/steward/tests/auditor.test.ts`

---

# Phase A — Substrate

## Task 1: The citation-audit table, event, and write path (SQL)

**Grounding tag:** EXTEND — spec §4.1 chose an event-sourced projection over a provenance column; the grain argument (`kb_edges` CHECK admits only `kb_resources`/`kb_cogmaps` at `migrations/20260624000001_canonical_schema.sql:630,632`) forces it.

**Files:**
- Create: `migrations/20260723000010_citation_audits.sql`
- Test: `crates/temper-substrate/tests/citation_audits.rs`

**Interfaces:**
- Produces: table `kb_citation_audits(id, block_id, source_kind, source_id, value, reason, audited_by_event_id, is_superseded, created)`; event type `citation_audited`; `_project_citation_audited(p_event uuid, p_payload jsonb) RETURNS uuid`; entry `citation_audit(p_payload jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL, p_correlation uuid DEFAULT NULL) RETURNS uuid`.

- [ ] **Step 1: Read the pattern you are copying.** Read `migrations/20260710000001_block_provenance_annotate.sql` end to end. It is the closest incumbent: a payload-only event with no content sidecar, registering an event type, a `_project_*` half, and an entry function that resolves the home anchor from `kb_resource_homes` and calls `_event_append`. Your migration mirrors its shape exactly. Note especially `:55-57` (the empty-input guard) and `:58-62` (anchor resolution, which raises when the resource has no home).

- [ ] **Step 2: Write the migration.**

Requirements, each of which the tests below pin:

- `kb_citation_audits` columns: `id uuid PK DEFAULT uuid_generate_v7()`, `block_id uuid NOT NULL REFERENCES kb_content_blocks(id) ON DELETE CASCADE`, `source_kind provenance_source_kind NOT NULL`, `source_id uuid NOT NULL`, `value double precision NOT NULL`, `reason text`, `audited_by_event_id uuid NOT NULL REFERENCES kb_events(id)`, `is_superseded boolean NOT NULL DEFAULT false`, `created timestamptz NOT NULL DEFAULT now()`.
- A `CHECK (value >= -1.0 AND value <= 1.0)`. The signed range is spec §3.3 and must be enforced in the schema, not only in Rust — the SQL entry function is callable from the scenario DSL and from replay.
- Index `ON kb_citation_audits(block_id, source_kind, source_id) WHERE NOT is_superseded` — this is the shape the standing components read (latest live audit per citation).
- `_project_citation_audited` must **supersede prior live audits for the same citation** before inserting: `UPDATE kb_citation_audits SET is_superseded = true WHERE block_id = … AND source_kind = … AND source_id = … AND NOT is_superseded`. Superseding rather than deleting is spec §4.1 ("superseded audits remain as history").
- The projector must be a **pure function of `(p_event, p_payload)`** and idempotent under replay — the same requirement `20260710000001:12-14` states for `_project_block_annotated`. Re-firing the same event must not create a second row: key the insert on `audited_by_event_id` with `ON CONFLICT DO NOTHING` and add `UNIQUE (audited_by_event_id, block_id, source_kind, source_id)`.
- The entry function raises if the block does not exist, and resolves the event's anchor from `kb_resource_homes` for the block's resource — copy `block_annotate`'s resolution at `20260710000001:51-62` rather than restating it.
- Register the event type with `category = 'domain'` (spec §4.3): `INSERT INTO kb_event_types (name, payload_schema, schema_version) VALUES ('citation_audited', NULL, 1) ON CONFLICT (name) DO NOTHING;`. Permissive NULL schema matches the sibling at `20260710000001:20-22`; the typed payload is validated Rust-side.

- [ ] **Step 3: Write the failing tests.**

Create `crates/temper-substrate/tests/citation_audits.rs`, modelled on `crates/temper-substrate/tests/evidential_standing.rs` (read it first for the `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` harness and its fixture helpers). Tests:

1. `citation_audit_inserts_a_live_row` — fire once, assert one row with `is_superseded = false` and the value round-tripped.
2. `citation_audit_supersedes_the_prior_audit_of_the_same_citation` — fire twice with different values; assert exactly one live row carrying the second value, and the first row still present with `is_superseded = true`.
3. `citation_audit_rejects_a_value_outside_the_signed_range` — `1.5` raises.
4. `citation_audit_is_idempotent_under_replay` — project the same event id twice; assert one row.
5. `citation_audit_raises_for_an_unknown_block`.

- [ ] **Step 4: Run the tests, confirm they fail for the right reason.**

```bash
cargo make docker-up
cargo nextest run -p temper-substrate --features artifact-tests --test citation_audits
```
Expected: failures naming the missing relation/function — **not** a migrator error. A migrator error means the migration does not parse; fix that first.

- [ ] **Step 5: Make them pass, then regenerate the SQL caches.**

```bash
cargo nextest run -p temper-substrate --features artifact-tests --test citation_audits
cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 6: Commit.**

```bash
git add migrations/20260723000010_citation_audits.sql crates/temper-substrate/tests/citation_audits.rs .sqlx
git commit -m "Task 1: citation audits are events, because a citation is not an edge"
```

---

## Task 2: Split standing into magnitude and quality (SQL)

**Grounding tag:** AMEND — of `kb_resource_standing`, `standing_band`, `refresh_resource_standing`, `resource_standing_shape` (all `migrations/20260721000010_evidential_standing_memo.sql`). Authorized by spec §3.1/§3.4. Safe because `resource_independence_breadth` is provably a constant today (`…memo.sql:141-146` with no writer for `'independent-of'`), so there is no data to migrate and no behavior to preserve.

**Files:**
- Create: `migrations/20260723000020_standing_citation_components.sql`
- Test: `crates/temper-substrate/tests/evidential_standing.rs` (extend)

**Interfaces:**
- Consumes: `kb_citation_audits` (Task 1).
- Produces: `resource_citation_magnitude(p_finding uuid) RETURNS int`; `resource_citation_quality(p_finding uuid) RETURNS double precision`; `resource_audit_coverage(p_finding uuid) RETURNS int`; `standing_band(p_citation_magnitude int, p_citation_quality double precision, p_audit_coverage int, p_contradiction_balance double precision, p_freshness double precision) RETURNS text`; `resource_standing_shape(p_finding uuid, p_principal_kind text, p_principal_id uuid)` returning `(finding_id, citation_magnitude int, citation_quality double precision, audit_coverage int, contradiction_balance double precision, freshness double precision, r_parent double precision, band text)`.

- [ ] **Step 1: Read what you are amending.** Read `migrations/20260721000010_evidential_standing_memo.sql` in full, and spec §3.1's component-mapping table. The mapping is not negotiable and every column's fate is specified there:
  - `indep_breadth` → replaced by `citation_magnitude` + `citation_quality`
  - `adversarial_survival` → **dropped**, subsumed into `citation_quality`
  - `challenge_count` → **kept integral**, redefined as audit coverage
  - `contradiction_balance`, `freshness`, `r_parent` → unchanged

- [ ] **Step 2: Write the migration.**

- `ALTER TABLE kb_resource_standing` — add `citation_magnitude int NOT NULL DEFAULT 0`, `citation_quality double precision NOT NULL DEFAULT -0.5`, `audit_coverage int NOT NULL DEFAULT 0`; drop `indep_breadth` and `adversarial_survival`. (`challenge_count` is superseded by the clearer `audit_coverage` name; drop it too rather than leaving a column whose name lies about its meaning.)
- `resource_citation_magnitude` — count of **distinct** `p.source_id` over the finding's live, uncorrected provenance. Copy the join shape verbatim from `resource_bases` (`…memo.sql:104-111`) — same `NOT p.is_corrected` / `NOT b.is_folded` filters, same `source_kind = 'resource'` restriction. **CONFORM: do not restate those filters differently.**
- `resource_audit_coverage` — count of distinct sources from that same set having a live (`NOT is_superseded`) row in `kb_citation_audits`.
- `resource_citation_quality` — the **mean** over distinct sources of: the live audit's `value` when one exists, else `-0.5`. Returns `0.0` when the finding has no bases at all (no citations means no quality claim, not a maximally-suspect one). **The mean is the whole point** — spec §3.2: a sum makes ten unaudited citations score worse than one, which is the perverse gradient this design exists to avoid.
- `DROP FUNCTION standing_band(double precision, int, double precision, double precision, double precision);` then CREATE the new signature. Thresholds: `near-canonical` requires `citation_magnitude >= 3 AND citation_quality > 0.5 AND audit_coverage >= 3 AND contradiction_balance > 1.0`; `reinforced` requires `citation_magnitude >= 2 AND citation_quality > 0.0 AND contradiction_balance >= 0.0`; else `provisional`. **The `citation_quality > 0` conjunct on both upper bands is load-bearing** (spec §3.1): it is what stops a high-magnitude monoculture from reaching a high band.
- `CREATE OR REPLACE FUNCTION refresh_resource_standing` — same signature, so REPLACE is correct. Drop the `PERFORM refresh_independence_pairs(...)` call and the `resource_adversarial_survival` SELECT; UPSERT the new component set.
- `DROP FUNCTION resource_standing_shape(uuid, text, uuid);` then CREATE with the new `RETURNS TABLE` (a changed return type requires DROP+CREATE). **Keep the `gated` CTE over `resources_readable_by` byte-for-byte** (`…memo.sql:239-242`) — that gate is the read's leak-safety and delegates verbatim to the canonical visibility predicate. Do not "simplify" it.
- `DROP FUNCTION resource_independence_breadth(uuid); DROP FUNCTION refresh_independence_pairs(uuid); DROP FUNCTION resource_adversarial_survival(uuid); DROP TABLE kb_independence_pairs;`

  > **Deploy-skew warning (CONFORM):** `DROP FUNCTION` is non-additive and breaks migrate-ahead-of-deploy — an old binary calling a dropped function fails. These drops are safe **only** because no Rust code calls them once Task 4 lands. Sequence the deploy accordingly: this migration and Task 4's binary ship together. Say so in the migration header comment.

- [ ] **Step 3: Write the failing tests** in `crates/temper-substrate/tests/evidential_standing.rs`:

1. `unaudited_citations_hold_quality_at_the_prior` — a finding with **one** unaudited citation and a finding with **ten** unaudited citations both read `citation_quality == -0.5`. *This is the perverse-gradient regression named in spec §3.2 — it is the single most important test in this plan.*
2. `citation_magnitude_counts_distinct_sources_not_provenance_rows` — ten citations of ONE source yields `citation_magnitude == 1` and `r_parent == 10.0`. (Spec §3.1: collapsing these reintroduces the actor-count fallacy.)
3. `a_positive_audit_lifts_quality_off_the_prior`.
4. `a_negative_audit_drives_quality_below_the_prior`.
5. `audit_coverage_counts_audited_distinct_sources`.
6. `high_magnitude_with_negative_quality_stays_provisional` — the Landmesser case: magnitude 5, quality negative, band is `provisional`.
7. `standing_shape_returns_none_for_an_unreadable_finding` — extend the existing gate test to the new return shape.

- [ ] **Step 4: Run, watch them fail, implement, run again.**

```bash
cargo nextest run -p temper-substrate --features artifact-tests --test evidential_standing
```

- [ ] **Step 5: Regenerate caches and commit.**

```bash
cargo sqlx prepare --workspace -- --all-features
git add migrations/20260723000020_standing_citation_components.sql crates/temper-substrate/tests/evidential_standing.rs .sqlx
git commit -m "Task 2: magnitude and quality are two numbers, because one number merges the crowd with the maverick"
```

---

## Task 3: The auditor's selection sweep (SQL)

**Grounding tag:** EXTEND — spec §6.3. Modelled on the steward's drift sweep (`crates/temper-services/src/services/steward_service.rs:70` `drift_sweep`, `:101` `candidate_cogmaps`).

**Files:**
- Create: `migrations/20260723000030_audit_drift_sweep.sql`
- Test: `crates/temper-substrate/tests/citation_audits.rs` (extend)

**Interfaces:**
- Produces: `audit_drift_sweep(p_limit int) RETURNS TABLE(cogmap_id uuid, finding_id uuid, uncovered int)`.

- [ ] **Step 1: Write the migration.** Select findings where `resource_citation_magnitude(r.id) > 0 AND resource_audit_coverage(r.id) < resource_citation_magnitude(r.id)`, joined to their cogmap home via `kb_resource_homes` where `anchor_table = 'kb_cogmaps'`, ordered by `uncovered DESC`, limited to `p_limit`.

  **CONFORM — scope boundary (spec §6.2):** cogmap-homed findings only. `kb_workflow_jobs.cogmap_id` is `NOT NULL REFERENCES kb_cogmaps(id)` (`migrations/20260705000001_workflow_jobs.sql:20`), so a context-homed finding has nowhere to enqueue. The `kb_resource_homes` join **is** that boundary — do not widen it here.

  **Coverage, not quality, is the predicate** (spec §6.3). A quality-based sweep drops a partially-audited finding out of the queue after a single citation is weighed.

- [ ] **Step 2: Write the failing tests.**

1. `sweep_returns_a_finding_with_unaudited_citations`
2. `sweep_omits_a_fully_audited_finding`
3. `sweep_omits_a_finding_with_no_citations`
4. `sweep_omits_a_context_homed_finding` — the §6.2 boundary, pinned so a later widening is a deliberate act rather than an accident.
5. `sweep_orders_by_uncovered_descending`

- [ ] **Step 3: Run, implement, run, regenerate, commit.**

```bash
cargo nextest run -p temper-substrate --features artifact-tests --test citation_audits
cargo sqlx prepare --workspace -- --all-features
git add migrations/20260723000030_audit_drift_sweep.sql crates/temper-substrate/tests/citation_audits.rs .sqlx
git commit -m "Task 3: the auditor's queue is coverage, not quality"
```

---

## Task 4: Rust write path and readback

**Grounding tag:** CONFORM — to the authored-write stack: `payloads.rs` typed payload → `events.rs` `SeedAction` arm → `writes.rs` `Params` + `fn` + `fn_with`. The template to copy is `assert_relationship_with` (`crates/temper-substrate/src/writes.rs:914-957`) and the `SeedAction::BlockAnnotate` arm (`crates/temper-substrate/src/events.rs:950-973`).

**Files:**
- Modify: `crates/temper-substrate/src/payloads.rs`, `src/events.rs`, `src/writes.rs`, `src/readback/mod.rs`
- Test: `crates/temper-substrate/tests/citation_audits.rs` (extend)

**Interfaces:**
- Produces:
  - `payloads::CitationAudited { block_id: Uuid, source_kind: String, source_id: Uuid, value: f64, reason: Option<String> }`
  - `writes::CitationAuditParams { block: BlockId, source_kind: ProvenanceSourceKind, source: Uuid, value: f64, reason: Option<&str>, emitter: EntityId }`
  - `writes::record_citation_audit(pool, p) -> Result<Uuid>` and `record_citation_audit_with(pool, p, ctx: EventContext) -> Result<Uuid>`
  - `readback::is_resource_visible(pool, principal: ProfileId, resource: ResourceId) -> Result<bool>` (a `pub` wrapper; the existing `ensure_visible` at `readback/mod.rs:143` is private)
  - `readback::StandingShapeRow` with fields `finding_id, citation_magnitude: i32, citation_quality: f64, audit_coverage: i32, contradiction_balance: f64, freshness: f64, r_parent: f64, band: String`

- [ ] **Step 1: Add the typed payload** to `payloads.rs`, following the neighbouring payload structs. It must round-trip through `verify_ledger_roundtrip` like its siblings — check how the existing payloads register for that check and do the same.

- [ ] **Step 2: Add the `SeedAction::CitationAudit` arm** to `events.rs`, copying `SeedAction::BlockAnnotate` (`:950-973`) — build the typed payload, `sqlx::query_scalar!("SELECT citation_audit($1,$2,$3,$4,$5)", …)` with `ctx_meta` / `ctx_inv` / `ctx_corr` threaded exactly as that arm does, `.context("citation_audit returned null")`.

- [ ] **Step 3: Add `writes::record_citation_audit[_with]`**, copying `assert_relationship_with`'s shape: `begin_scoped(pool)` → `fire_with(&mut tx, SeedAction::CitationAudit{…}, ctx)` → `tx.commit()`.

- [ ] **Step 4: Update `readback`.** Change `StandingShapeRow`'s fields and the `SELECT` column list in `resource_standing` (`readback/mod.rs:900-945`) to the new shape. Add `pub async fn is_resource_visible` wrapping the same SQL predicate `ensure_visible` uses at `:152` — **call it, do not restate it**; extract the shared query so there is exactly one spelling.

- [ ] **Step 5: Write the failing tests**, then implement, then run:

1. `record_citation_audit_writes_a_live_row_and_returns_its_id`
2. `record_citation_audit_with_stamps_the_invocation_and_authorship` — assert `kb_events.invocation_id` is set and `kb_events.metadata` carries the authorship. *This pins spec §4.2: the auditor's own confidence lands in metadata, never the payload.*
3. `standing_row_reads_the_new_components`

```bash
cargo nextest run -p temper-substrate --features artifact-tests --test citation_audits
```

- [ ] **Step 6: Commit.**

```bash
git add crates/temper-substrate/src crates/temper-substrate/tests/citation_audits.rs
git commit -m "Task 4: the Rust half of the audit write, and the readback's new shape"
```

---

## Task 5: The command, the backend, and the refresh clock

**Grounding tag:** CONFORM — to the operations-command dispatch model (CLAUDE.md: "Surfaces build a backend per request and dispatch one operations command per inbound call") and to `tick_resource_standing`'s never-fail-the-write policy (`crates/temper-services/src/backend/db_backend.rs:1147-1185`).

**Files:**
- Modify: `crates/temper-workflow/src/operations/commands.rs`, `operations/backend.rs`, `crates/temper-services/src/backend/db_backend.rs`
- Test: `crates/temper-services/tests/standing_clock_test.rs` (extend)

**Interfaces:**
- Produces: `RecordCitationAudit { block: BlockId, source_kind: String, source: Uuid, value: f64, reason: Option<String>, act: ActContext, origin: Surface }`; `Backend::record_citation_audit(&self, cmd: RecordCitationAudit) -> Result<CommandOutput<Uuid>, TemperError>`.

- [ ] **Step 1: Add the command struct** to `commands.rs`, matching `AssertRelationship`'s shape (`:207-219`) — including the `#[serde(default, skip_serializing_if = "ActContext::is_empty")] pub act: ActContext` and `pub origin: Surface` fields. Add a round-trip unit test beside `assert_relationship_command_round_trips` (`:434`).

- [ ] **Step 2: Add the trait method** to `operations/backend.rs` beside `assert_relationship` (`:102`).

- [ ] **Step 3: Implement it on `DbBackend`.** The method must, in order:
  1. **Authorize first** (Task 6's gate — this task depends on it; implement Task 6 before this step).
  2. Call `writes::record_citation_audit_with`.
  3. Resolve the audited block's owning resource and call `self.tick_resource_standing(finding)`.

  **CONFORM:** step 3 inherits `tick_resource_standing`'s policy verbatim — log and swallow on failure, never fail the write (`db_backend.rs:1151-1154`). Do not invent a different policy.

- [ ] **Step 4: Delete the obsolete `TODO(Set 5)` doc comment** at `db_backend.rs:1165-1172` and replace it with a note that the edge-incident staleness it warned about is now unreachable, because breadth reads citations rather than independence edges (spec §3.4). **Do not leave the TODO** — a stale warning about a dissolved problem is worse than none.

- [ ] **Step 5: Write the failing test** in `crates/temper-services/tests/standing_clock_test.rs`: `recording_an_audit_refreshes_the_finding_standing_memo` — assert `kb_resource_standing.citation_quality` moved after an audit write, without an explicit refresh call.

- [ ] **Step 6: Run, implement, run, regenerate the per-crate cache, commit.**

```bash
cargo nextest run -p temper-services --features test-db --test standing_clock_test
cargo make prepare-services
git add crates/temper-workflow crates/temper-services
git commit -m "Task 5: one command, authorized then written, then the clock ticks"
```

---

# Phase B — Surfaces

## Task 6: `AuditAuthority` — a write authorized by readability

**Grounding tag:** EXTEND — spec §7, which left this as a required pre-plan grounding obligation. It is discharged here.

**Files:**
- Create: `crates/temper-services/src/authz/audit_gate.rs`
- Modify: `crates/temper-services/src/authz/mod.rs` (module declaration)
- Test: `crates/temper-services/tests/audit_gate_test.rs`

**Interfaces:**
- Produces: `pub(crate) enum AuditAuthority { Readable, None }` with `impl ScopedAuthority for AuditAuthority { type Subject = ResourceId; }`.

- [ ] **Step 1: Read the trait and both existing impls** before writing anything: `crates/temper-services/src/authz/mod.rs:54-133` (the trait, the sealed `Authorized<A>`, and `authorize`) and `crates/temper-services/src/authz/read_gates.rs` in full. Match their structure and their comment discipline — each impl explains *why* its denial dialect is what it is.

- [ ] **Step 2: Implement `AuditAuthority`.** Four decisions, each fixed by the spec and the trait's own doctrine:

  1. **Subject is the finding** (`ResourceId`), so the sealed proof carries it and the act reads the subject from `Authorized::subject()` — never from a parameter alongside it (`authz/mod.rs:110-117`).
  2. **`resolve` calls `readback::is_resource_visible`** (Task 4) — the same predicate the canonical read gate uses. *"SQL predicates are authoritative here — call them, do not restate them"* (`authz/mod.rs:65-66`). A hand-written visibility clause here is the exact drift the policy layer exists to close.
  3. **`is_denial`** matches `AuditAuthority::None`. Denial is a named arm, never an `Err` from inside `resolve` (`authz/mod.rs:69-74`).
  4. **`denial()` returns `ApiError::NotFound`**, not `Forbidden` — and the doc comment must say why: the evidence **read** over this same subject is already leak-safe by returning zero rows → 404 (`…memo.sql:239-242`), so a `Forbidden` on the write would create an existence oracle beside a gate deliberately built to avoid one. Follow `read_gates.rs:53-59`'s comment style.

- [ ] **Step 3: Write the failing tests** in `crates/temper-services/tests/audit_gate_test.rs`:

1. `a_reader_who_is_not_the_owner_may_audit` — **the whole point of the gate.** An auditor that may only assess findings it owns is not an auditor (spec §7).
2. `a_principal_who_cannot_read_the_finding_is_refused`
3. `the_refusal_is_not_found_not_forbidden` — assert the variant, so a later consistency pass cannot silently convert it into an existence leak.

- [ ] **Step 4: Run, implement, run, commit.**

```bash
cargo nextest run -p temper-services --features test-db --test audit_gate_test
git add crates/temper-services/src/authz crates/temper-services/tests/audit_gate_test.rs
git commit -m "Task 6: an auditor that may only audit what it owns is not an auditor"
```

---

## Task 7: The API surface and the wire types

**Grounding tag:** CONFORM — to `handlers/evidence.rs` (the sibling Set 3 read) and the `routes!` mounting that puts a handler in the OpenAPI contract.

**Files:**
- Create: `crates/temper-core/src/types/citation_audit.rs`, `crates/temper-api/src/handlers/citation_audits.rs`
- Modify: `crates/temper-core/src/types/standing.rs`, `crates/temper-core/src/types/mod.rs`, `crates/temper-services/src/services/citation_audit_service.rs` (new), `crates/temper-api/src/routes.rs`, `crates/temper-api/src/openapi.rs`
- Test: `crates/temper-api/tests/citation_audit_handler_test.rs`

**Interfaces:**
- Produces: `POST /api/resources/{id}/citation-audits` taking `CitationAuditRequest { block_id, source_kind, source_id, value, reason }`, returning the audit id. `StandingShape` gains `citation_magnitude: i32`, `citation_quality: f64`, `audit_coverage: i32`; loses `indep_breadth`, `adversarial_survival`, `challenge_count`.

- [ ] **Step 1: Update `StandingShape`** (`crates/temper-core/src/types/standing.rs`) to the new field set, keeping the derive stack and the doc comments' framing (standing is shape-primary; the band is a lossy chip). Update `evidential_standing_service::resource_evidence`'s mapping to match.

- [ ] **Step 2: Add `CitationAuditRequest`** in a new `citation_audit.rs`, with the same derive stack as `StandingShape` (`ts-rs` export, `utoipa::ToSchema` under `web-api`, `schemars` under `mcp`). **Typed struct, not `json!()`.**

- [ ] **Step 3: Add the service and handler.** The service authorizes via `authorize::<AuditAuthority>(…)` (Task 6) and dispatches the `RecordCitationAudit` command through the backend (Task 5). The handler is thin: extract, validate, call, respond. **Never call persistence directly from the handler.**

- [ ] **Step 4: Mount via `routes!`** so it lands in the OpenAPI contract — check how `handlers/evidence.rs` is mounted and do the same.

- [ ] **Step 5: Write the failing tests** in `crates/temper-api/tests/citation_audit_handler_test.rs`, modelled on `crates/temper-api/tests/evidence_handler_test.rs`:

1. `posting_an_audit_returns_the_audit_id`
2. `posting_an_audit_without_auth_returns_401`
3. `posting_an_audit_on_an_unreadable_finding_returns_404` — not 403 (Task 6's dialect, asserted at the surface).
4. `posting_an_out_of_range_value_returns_400`

- [ ] **Step 6: Run, implement, run, commit.**

```bash
cargo nextest run -p temper-api --features test-db --test citation_audit_handler_test
git add crates/temper-core crates/temper-services crates/temper-api
git commit -m "Task 7: the audit write surface"
```

> **Gotcha:** never run a bare `cargo nextest run -p temper-api` with no `--test` filter — it hangs at list enumeration on the bin target (CLAUDE.md).

---

## Task 8: Regenerate every downstream artifact

**Grounding tag:** CONFORM — the four drift gates in `cargo make check`.

**Files:** `openapi.json`, `clients/temper-rb/lib/temper/generated/**`, `clients/temper-ts/src/generated/schema.ts`, `packages/temper-ui/src/lib/types/generated/**`, `packages/agent-workflows/mention/agent/generated/**`

- [ ] **Step 1: Regenerate.**

```bash
cargo make openapi          # openapi.json + temper-rb gem (needs Docker) + temper-ts schema.ts
cargo make generate-ts-types  # both ts-rs trees
```

- [ ] **Step 2: Stage the output, then verify.**

```bash
git add openapi.json clients packages/temper-ui/src/lib/types/generated packages/agent-workflows/mention/agent/generated
cargo make check
```

> **The drift gates compare against git, not against a fresh build.** A correctly regenerated artifact still fails `check` while it sits unstaged, with an error that reads like you forgot to regenerate. Stage first, then check (CLAUDE.md).

- [ ] **Step 3: Check the UI, which `cargo make check` does not cover.**

```bash
cd packages/temper-ui && bun install && bun run check
```

Fix any fixture or helper that the `StandingShape` field change broke.

- [ ] **Step 4: Commit** all regenerated artifacts in one commit.

```bash
git commit -m "Task 8: regenerate the artifacts the component split restaled"
```

---

## Task 9: The CLI evidence renderer

**Grounding tag:** CONFORM — to the existing `temper resource evidence` action shipped in Set 3.

**Files:** Modify `crates/temper-cli/src/commands/resource.rs` and its action; Test: `tests/e2e/tests/` (extend the Set 3 evidence e2e)

- [ ] **Step 1: Update the renderer** to print the new component set. **Carry the band WITH the shape, never instead of it** (spec §1.1 of `019f81e8`) — the existing renderer already does this; preserve it.

- [ ] **Step 2: Extend the e2e** that Set 3's Task 9 shipped: after recording an audit through the real API, assert `temper resource evidence <ref>` shows `citation_quality` off the `-0.5` prior and `audit_coverage` incremented.

- [ ] **Step 3: Run.**

```bash
cargo build --bin temper          # e2e spawns the built binary; nextest does NOT rebuild it
cargo make test-e2e
cargo make prepare-e2e
```

> **Gotcha:** the e2e harness spawns a **stale** `temper` binary unless you build it first (CLAUDE.md / prior sessions).

- [ ] **Step 4: Commit.**

---

## Task 10: The MCP tool

**Grounding tag:** CONFORM — to the MCP tool pattern in `crates/temper-mcp/src/tools/`, which delegates to temper-services (services-direct reads, `DbBackend` writes).

**Files:** Create `crates/temper-mcp/src/tools/citation_audits.rs`; Modify `crates/temper-mcp/src/tools/mod.rs`

- [ ] **Step 1: Read a sibling tool** — `tools/relationships.rs` is the closest analogue (an authored graph write with an `ActContext`). Match its parameter struct, its authorship threading, and its error mapping.

- [ ] **Step 2: Add `record_citation_audit`.** The auditor agent calls this. Its parameters carry the citation key, the signed value, the reason, and the standard act/authorship fields.

  **Enum params must be `#[schemars(inline)]`** — a `$ref` into `$defs` reaches the Anthropic tool-use layer with no type signal and returns `null` (`crates/temper-core/src/types/authorship.rs:32-35`).

- [ ] **Step 3: Test, run, commit.**

---

# Phase C — The persona

## Task 11: Provision the auditor's machine principal

**Grounding tag:** CONFORM — spec §5.2; `crates/temper-services/src/services/profile_service.rs:230-241` (lookup-or-401, no JIT create).

- [ ] **Step 1: Document the provisioning step** in `DEPLOYING.md`, beside the steward's own setup:

```bash
temper admin machine provision --client-id <auditor-client-id> --label "citation auditor" --team <team-ref>:member
```

- [ ] **Step 2: Write the rationale into the doc**, not just the command: the auditor **must not** share the steward's `client_id`, because one credential means one `emitter_entity_id` and the ledger would be unable to tell an audit from the citation it audits — collapsing "assessed by another party" into "asserted by the same party wearing a label" (spec §5.2).

- [ ] **Step 3: Commit.**

---

## Task 12: The auditor schedule

**Grounding tag:** CONFORM — `packages/agent-workflows/steward/agent/schedules/steward.ts` is the template, including its correlation-id threading and its `temperFetch` auth ordering.

**Files:**
- Create: `packages/agent-workflows/steward/agent/schedules/auditor.ts`, `agent/channels/auditor-worker.ts`, `tests/auditor.test.ts`
- Modify: `crates/temper-api/src/handlers/auditor.rs` + `crates/temper-services/src/services/auditor_service.rs` (the dispatch endpoint, mirroring `handlers/steward.rs` / `steward_service.rs`)

- [ ] **Step 1: Read `schedules/steward.ts` in full**, including its long header comment. It documents why the handler is code-driven rather than model-driven, how the correlation id threads cron → dispatch → session, and why the fan-out is over the workflow rather than over an agent's target. The auditor mirrors all of it.

- [ ] **Step 2: Add the dispatch endpoint.** `POST /api/auditor/dispatch` runs sweep (Task 3) → `workflow_job_enqueue(cogmap, 'auditor', 'citation-audit', payload)` → claim, mirroring `steward_service`. **No new queue DDL** — `kb_workflow_jobs` is persona-agnostic by construction (`migrations/20260705000001_workflow_jobs.sql:18-46`), so the auditor is a new `persona` value and nothing more.

- [ ] **Step 3: Write the schedule.** Its own credential env names, its own model config (spec §5.3 recommends a **different** model from the steward — it is the one lever that attacks shared trained priors). One isolated session per claimed job.

- [ ] **Step 4: Write the session prompt.** It must direct the agent to:
  - read the finding and its citations via `get_block_provenance`;
  - pull `element_trail` **only** for the citing acts it decides to weigh — discrete calls, never a bulk first pass (spec §8);
  - weigh the citing act's recorded confidence and rationale, the related resources, and the size of the citation set (spec §3.3);
  - emit a signed verdict per citation via the Task 10 tool, attaching its **own** authorship and confidence;
  - **assess only whether the source carries the connection claimed — never whether the claim is true, and never what the source says** (spec §Bedrock, §3.4). This sentence belongs in the prompt verbatim; it is the persona's boundary.

- [ ] **Step 5: Test.**

```bash
cd packages/agent-workflows/steward && npm install && npm test
```

- [ ] **Step 6: Commit.**

---

## Task 13: Final verification

- [ ] **Step 1: Full local gate.**

```bash
cargo make check
cargo make test
cargo make test-db
cargo build --bin temper && cargo make test-e2e-embed
cd packages/agent-workflows/steward && npm test
cd packages/temper-ui && bun run check
```

> `cargo make test-e2e` alone silently compiles out every `test-embed`-gated test; CI does not. Run the `-embed` variant (CLAUDE.md).

- [ ] **Step 2: Confirm the sqlx caches are complete and staged** — workspace first, then per-crate:

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-e2e
git status --short
```

- [ ] **Step 3: Update `CLAUDE.md`** — the evidential-standing entry describes `indep_breadth` and the pairwise independence model, both of which are gone. Record what replaced them and why.

- [ ] **Step 4: Merge `origin/main`, re-run `cargo make check`, then push and open the PR.**

> A clean auto-merge is not a passing build — a sibling's sealed field or renamed accessor compiles on their branch and breaks yours (prior session, PR #519). Re-check after merging, always. And never hand-merge generated-file conflicts: regenerate them from the merged router.

---

## Self-Review

**Spec coverage:** §1 (inherited debts) → Tasks 2, 5. §2 (reframe) → Tasks 1–2. §3.1 (component split) → Task 2. §3.2 (negative prior) → Task 2 Step 3 test 1. §3.3 (signed range) → Tasks 1, 12. §3.4 (absorb/retire) → Tasks 2, 5. §4.1 (grain, supersession) → Task 1. §4.2 (payload vs metadata) → Task 4 test 2. §4.3 (refresh, domain category) → Tasks 1, 5. §5.2 (principal separation) → Task 11. §5.3 (placement) → Task 12. §5.4 (envelope unchanged) → no task needed, deliberately. §6.1–6.2 (queue reuse, scope boundary) → Tasks 3, 12. §6.3 (coverage predicate) → Task 3. §6.4 (structural promotion gate) → falls out of Task 2's band thresholds; no separate task. §7 (`can_audit_resource`) → Task 6. §8 (restaling) → Tasks 8, 9.

**Known gap, deliberately carried:** spec §8 notes `element_trail`'s DTO drops `rationale`, `persona`, and `model`, and that `element_trail` has no MCP tool. Neither is scheduled above. The auditor can work without them — it has `confidence` plus the citing act's payload — but its judgment is thinner than §3.3 describes. **Decide before Task 12** whether to add a task widening the DTO, or accept the thinner input for a first cut and say so in the prompt.
