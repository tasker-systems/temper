# Set 5 — Adversary persona as citation auditor: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Set 3's unwritable pairwise-independence model with an append-only citation-audit substrate, and ship the adversary persona that feeds it.

**Architecture:** A new **append-only** `kb_citation_audits` event projection records a signed `[-1.0, 1.0]` defensibility verdict per audit act — no supersession, no mutation. `kb_resource_standing.indep_breadth` splits into three axes: `citation_magnitude` (distinct **live** cited sources, monotone), `audit_coverage` (distinct sources with ≥1 audit, monotone), and `citation_quality` (mean over the **audited subset only** of each source's **decay-weighted** audit value, recomputed fresh from the trail). Skepticism toward unevaluated evidence lives in a coverage-ratio band gate, not in a poisoned mean. The auditor is a new schedule in the steward Eve package under its **own** registered machine client, dispatched through the existing persona-agnostic `kb_workflow_jobs` queue.

**Tech Stack:** PostgreSQL (sqlx migrations), Rust (temper-substrate / temper-core / temper-workflow / temper-services / temper-api / temper-mcp / temper-client / temper-cli), TypeScript (Eve agent, vitest).

**Spec of record:** [`docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md`](../specs/2026-07-23-set5-adversary-citation-audit-design.md). **Read the spec section each task cites — this plan is an index over it, not a replacement for it.** This plan was rewritten after a three-lens adversarial review; the fixes that review earned are folded into the tasks below and are called out with **[review]**.

## Global Constraints

- **Shipped migrations are immutable.** Every change to a shipped SQL object is a NEW additive migration (DROP+CREATE, or CREATE OR REPLACE where the signature is unchanged). Never edit `20260721000010_evidential_standing_memo.sql`.
- **Migration numbering:** latest on `main` is `20260722000100`. This plan uses `20260723000010/20/30`; gaps left for concurrent siblings. Re-verify no `20260723*` exists before applying (`git ls-tree origin/main migrations/`).
- **`--all-features`** for builds and clippy. `#[expect(lint, reason=…)]`, never `#[allow]`.
- **Typed structs over `serde_json::json!()`**. Params structs past 5 domain args. **Use `temper_core::types::provenance::ProvenanceSource`** for a citation's source — it is the existing `{kind,value}` tagged sum (`provenance.rs:35`). There is **no** `ProvenanceSourceKind` type; do not invent one.
- **Auth before writes**, and the authorization subject travels *inside* the sealed proof.
- **No `sqlx::query!()` inline in a surface.** Persistence lives in temper-substrate / temper-services; surfaces dispatch one operations command.
- **A new emittable event needs an `EventKind` variant and a `replay.rs` arm** or `replay()` hard-fails at runtime against any DB that ever recorded one (this is the shipped `resource_finalized` bug — `events.rs:68-71`). This is Task 2 and is not optional.
- **Regeneration is deferred to the final task.** Per-task work runs scoped tests (which need no regen); the router-drift gates (`openapi-*`, `ts-rs-drift`) are only satisfied by the final regen after *all* route/type changes (Tasks 8 and 13). Do **not** run full `cargo make check` mid-plan expecting green — run the scoped `cargo nextest`/`bun test` the task names.
- **After any SQL change:** `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services`, then `cargo make prepare-e2e` (per-crate last).
- **Standing is not truth** (spec §Bedrock). An audit assesses the defensibility a citation confers — never what a source says, never whether a claim is true.
- **Per-step grounding tags:** CONFORM (cite the constraint), EXTEND (cite the spec section), AMEND (cite both).

---

## File Structure

**Migrations (new):**
- `20260723000010_citation_audits.sql` — append-only audit table, event type (`category='domain'`), projector, entry function.
- `20260723000020_standing_citation_components.sql` — the AMEND of Set 3's components: new columns, new producers (magnitude/coverage/quality with per-source collapse + decay + liveness), re-thresholded band with `disputed` arm, rewritten refresh + shape read, retirement of the pairwise objects.
- `20260723000030_audit_drift_sweep.sql` — the auditor's **principal-scoped** selection function.

**Rust:**
- `temper-substrate/src/{payloads,events,writes,replay}.rs`, `readback/mod.rs`
- `temper-core/src/types/standing.rs`, new `citation_audit.rs`
- `temper-workflow/src/operations/{commands,backend}.rs`
- `temper-services/src/authz/audit_gate.rs` (new), `services/{citation_audit_service,auditor_service}.rs` (new), `backend/db_backend.rs`
- `temper-cli/src/cloud_backend/backend.rs` (both `Backend` impls), `src/commands/resource.rs`
- `temper-client/src/resources.rs`
- `temper-api/src/handlers/{citation_audits,auditor}.rs` (new), `routes.rs`
- `temper-mcp/src/tools/citation_audits.rs` (new)

**TypeScript:**
- `packages/agent-workflows/steward/agent/schedules/auditor.ts`, `channels/auditor-worker.ts`, `tests/auditor.test.ts`

**Dependency order (NOT the numbering — the numbering is reading order):** 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 11 → 12 → 13 → 10/14. Task 6 consumes Task 4; Task 7 consumes Task 6. Each task's Interfaces block names what it consumes.

---

# Phase A — Substrate

## Task 1: The append-only citation-audit table, event, and write path (SQL)

**Grounding tag:** EXTEND — spec §4.1 (append-only, no supersession). The grain argument (a citation is `(block, source)`; `kb_edges` admits only `kb_resources`/`kb_cogmaps` endpoints, `canonical_schema.sql:630,632`) forces an event, not an edge.

**Files:** Create `migrations/20260723000010_citation_audits.sql`; Test `crates/temper-substrate/tests/citation_audits.rs`

**Interfaces:**
- Produces: table `kb_citation_audits(id, block_id, source_kind, source_id, value, reason, audited_by_event_id, created)`; event type `citation_audited` (`category='domain'`); `_project_citation_audited(p_event uuid, p_payload jsonb) RETURNS uuid` (returns the **audit** id); entry `citation_audit(p_payload jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL, p_correlation uuid DEFAULT NULL) RETURNS uuid`.

- [ ] **Step 1: Read the pattern.** Read `migrations/20260710000001_block_provenance_annotate.sql` end to end (payload-only event, anchor resolved from `kb_resource_homes`, `:55-62`). Read `migrations/20260720000020_principal_standing_events.sql:26` for the **post-firewall** event-type registration idiom.

- [ ] **Step 2: Write the migration.**

  - **Table (append-only, no supersession — [review]):** `id uuid PK DEFAULT uuid_generate_v7()`, `block_id uuid NOT NULL REFERENCES kb_content_blocks(id) ON DELETE CASCADE`, `source_kind provenance_source_kind NOT NULL`, `source_id uuid NOT NULL`, `value double precision NOT NULL CHECK (value >= -1.0 AND value <= 1.0)`, `reason text`, `audited_by_event_id uuid NOT NULL REFERENCES kb_events(id)`, `created timestamptz NOT NULL DEFAULT now()`. **No `is_superseded` column** — the trail is immutable and read via decay aggregation (Task 3), never latest-wins.
  - **Idempotency, the only uniqueness:** `UNIQUE (audited_by_event_id)`. Replay re-projects the same event; `ON CONFLICT (audited_by_event_id) DO NOTHING` makes that a no-op. There is deliberately **no** unique index on `(block_id, source_kind, source_id)` — multiple audits of one citation are the whole point (§4.1).
  - **[review] Reject non-`'resource'` source_kind in the entry function** with a `RAISE` naming the reason: standing reads only resource-kind bases (`…memo.sql:110`), so an audit on a `remote`/`event` citation would be a silent no-op the auditor cannot detect (spec §6.2). `IF (p_payload->>'source_kind') <> 'resource' THEN RAISE EXCEPTION 'citation_audit: only resource-kind citations are auditable (got %)', p_payload->>'source_kind'; END IF;`
  - **[review] The projector returns the AUDIT id, and survives the ON CONFLICT no-op.** The `block_annotate` sibling returns the *block* id; do not copy that. `INSERT … ON CONFLICT (audited_by_event_id) DO NOTHING RETURNING id` yields no row on the replay path, so fall back: `SELECT id INTO v_audit FROM kb_citation_audits WHERE audited_by_event_id = p_event; RETURN v_audit;`
  - Entry function: raise if the block does not exist; resolve the event anchor from `kb_resource_homes` for the block's resource (copy `20260710000001:51-62`); `_event_append('citation_audited', …)`; `PERFORM _project_citation_audited(v_ev, p_payload)` and return **its** result.
  - **[review] Registration spells `category` — a bare `(name, payload_schema, schema_version)` INSERT aborts at apply time** because `kb_event_types.category` is `NOT NULL` with no default (`20260719000010:98`, deliberately, `:74-81`): `INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES ('citation_audited', NULL, 1, 'domain') ON CONFLICT (name) DO NOTHING;`

- [ ] **Step 3: Write the failing tests** (`crates/temper-substrate/tests/citation_audits.rs`, modelled on `tests/evidential_standing.rs` — read it for the `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` harness under `#![cfg(feature = "artifact-tests")]`):

  1. `citation_audit_inserts_a_row_and_returns_its_audit_id` — the returned uuid is a `kb_citation_audits.id`, not the block id.
  2. `two_audits_of_one_citation_both_persist` — fire twice, different values; assert **two** rows (append-only, no supersession).
  3. `citation_audit_rejects_a_value_outside_the_signed_range` — `1.5` raises.
  4. `citation_audit_rejects_a_remote_source_kind` — a `remote` citation raises.
  5. `citation_audit_is_idempotent_under_replay` — projecting the same event id twice yields one row and returns its id both times.
  6. `citation_audit_raises_for_an_unknown_block`.

- [ ] **Step 4: Run, confirm failure is missing-relation not migrator-parse, implement, run green.**

```bash
cargo make docker-up
cargo nextest run -p temper-substrate --features artifact-tests --test citation_audits
```

- [ ] **Step 5: Regenerate caches, commit.**

```bash
cargo sqlx prepare --workspace -- --all-features
git add migrations/20260723000010_citation_audits.sql crates/temper-substrate/tests/citation_audits.rs .sqlx
git commit -m "Task 1: citation audits are append-only events, because the ledger is immutable"
```

---

## Task 2: The Rust write path AND the replay arm

**Grounding tag:** CONFORM — the authored-write stack (`payloads.rs` → `events.rs` `SeedAction` → `writes.rs` `Params`+`fn`+`fn_with`), the `assert_relationship_with` template (`writes.rs:933-957`), the `SeedAction::BlockAnnotate` arm (`events.rs:950-973`), and the `EventKind`/`replay.rs` registration (`events.rs:38,103,143`; `replay.rs:201,491`).

**Files:** Modify `crates/temper-substrate/src/{payloads,events,writes,replay}.rs`; Test `crates/temper-substrate/tests/citation_audits.rs` (extend), `tests/replay_roundtrip.rs` (extend)

**Interfaces:**
- Produces:
  - `payloads::CitationAudited { block_id: Uuid, source: ProvenanceSource, value: f64, reason: Option<String> }` (uses `temper_substrate::payloads::ProvenanceSource`, re-exported from temper-core — see `tests/evidential_standing.rs:16`). The projector reads `source_kind`/`source_id` from the serialized `{kind,value}` shape.
  - `EventKind::CitationAudited`
  - `Fired::CitationAudit(Uuid)` + accessor `fn citation_audit(self) -> Result<Uuid>` beside `relationship()` (`events.rs:490`)
  - `writes::CitationAuditParams<'a> { block: BlockId, source: ProvenanceSource, value: f64, reason: Option<&'a str>, emitter: EntityId }`
  - `writes::record_citation_audit(pool, p) -> Result<Uuid>` and `record_citation_audit_with(pool, p, ctx: EventContext) -> Result<Uuid>`

- [ ] **Step 1: Add `EventKind::CitationAudited`** — variant at `events.rs:61`-area, `as_canonical_name` arm `=> "citation_audited"` (`:122`-area), `from_canonical_name` arm (`:162`-area). **[review] Do NOT add it to `TYPED_EVENT_NAMES`** — like `BlockProvenanceAnnotated` (`payloads.rs:842-847`) it is registered by a post-canonical-seed migration with a NULL `payload_schema` and gets no committed JSON-Schema snapshot; `tests/bootseed.rs:95` and `payload_schema.rs:48` assert set-equality against `TYPED_EVENT_NAMES` and will break if you touch it.

- [ ] **Step 2: Add the `replay.rs` arm** — include `CitationAudited` in the projector-dispatch (`replay.rs:201`-area group) and add `EventKind::CitationAudited => { sqlx::query("SELECT _project_citation_audited($1,$2)")… }` beside `BlockProvenanceAnnotated` (`replay.rs:491`). **[review] Without this, `replay()` compiles clean and hard-fails at runtime against any DB that recorded an audit** — the `resource_finalized` bug (`events.rs:68-71`).

- [ ] **Step 3: Add the payload** (`payloads.rs`) with a `verify_ledger_roundtrip` arm (`payloads.rs:1120`-area), following `BlockProvenanceAnnotated`'s permissive posture.

- [ ] **Step 4: Add `SeedAction::CitationAudit` + `Fired::CitationAudit`** (`events.rs`), copying `BlockAnnotate` (`:950-973`): build the typed payload, `sqlx::query_scalar!("SELECT citation_audit($1,$2,$3,$4,$5)", serde_json::to_value(&payload)?, emitter.uuid(), ctx_meta, ctx_inv, ctx_corr)`, `.context("citation_audit returned null")`, return `Fired::CitationAudit(id)`. Add the `Fired::citation_audit()` accessor.

- [ ] **Step 5: Add `writes::record_citation_audit[_with]`** (`writes.rs`), copying `assert_relationship_with`: `begin_scoped` → `fire_with(&mut tx, SeedAction::CitationAudit{…}, ctx)?.citation_audit()?` → `commit`.

- [ ] **Step 6: Tests.**
  1. `record_citation_audit_writes_a_row_and_returns_its_audit_id` (in `citation_audits.rs`).
  2. `record_citation_audit_with_stamps_the_invocation_and_authorship` — assert `kb_events.invocation_id` set and authorship in `kb_events.metadata`, **not** the payload (§4.2).
  3. `replay_reprojects_a_citation_audit` (in `replay_roundtrip.rs`) — fire an audit into the fixture, replay, assert the row is reprojected and no "no projector" error.

```bash
cargo nextest run -p temper-substrate --features artifact-tests --test citation_audits --test replay_roundtrip
```

- [ ] **Step 7: Commit.**

```bash
git add crates/temper-substrate/src crates/temper-substrate/tests
git commit -m "Task 2: the Rust write path and — the part that is not optional — the replay arm"
```

---

## Task 3: Split standing into magnitude, coverage, and decay-weighted quality (SQL)

**Grounding tag:** AMEND — `kb_resource_standing`, `standing_band`, `refresh_resource_standing`, `resource_standing_shape` (all `20260721000010`). Authorized by spec §3.1/§3.4. Safe because `resource_independence_breadth` is a constant today (no production writer for `'independent-of'`).

**Files:** Create `migrations/20260723000020_standing_citation_components.sql`; Test `crates/temper-substrate/tests/evidential_standing.rs` (extend **and prune** — [review])

**Interfaces:**
- Consumes: `kb_citation_audits` (Task 1).
- Produces: `resource_citation_magnitude(uuid) RETURNS int`; `resource_audit_coverage(uuid) RETURNS int`; `resource_citation_quality(uuid) RETURNS double precision`; `standing_band(p_citation_magnitude int, p_audit_coverage int, p_citation_quality double precision, p_contradiction_balance double precision, p_freshness double precision) RETURNS text`; `resource_standing_shape(uuid, text, uuid)` returning `(finding_id, citation_magnitude int, audit_coverage int, citation_quality double precision, contradiction_balance double precision, freshness double precision, r_parent double precision, band text)`.

- [ ] **Step 1: Read** `20260721000010` in full and spec §3.1 (the component-mapping table, the three axes, the per-source-collapse rule, the liveness rule, the band arms).

- [ ] **Step 2: Write the migration.**

  - `ALTER TABLE kb_resource_standing` — add `citation_magnitude int NOT NULL DEFAULT 0`, `audit_coverage int NOT NULL DEFAULT 0`, `citation_quality double precision NOT NULL DEFAULT 0`; drop `indep_breadth`, `adversarial_survival`, `challenge_count`.
  - **`resource_citation_magnitude`** — count of **distinct live** resource-kind sources. Start from `resource_bases`' join shape (`…memo.sql:104-111`: `NOT p.is_corrected`, `NOT b.is_folded`, `source_kind='resource'`) but **[review] add the liveness join** the spec requires: `JOIN kb_resources src ON src.id = p.source_id AND src.is_active`. The word "live" in "distinct live sources" is load-bearing (spec §3.1) — Set 3's `resource_bases` did not carry it.
  - **`resource_audit_coverage`** — count of distinct such sources having ≥1 row in `kb_citation_audits` for this finding's blocks.
  - **`resource_citation_quality`** — **[review] two-stage aggregate** (spec §3.1): inner — per distinct source, the **decay-weighted mean** of that source's audit values across all its citing blocks; outer — the mean over distinct **audited** sources of those per-source values. Return `0.0` when `audit_coverage = 0`. **Decay** weights each audit by recency toward `now()`, mirroring `resource_freshness`'s half-life shape (`…memo.sql:63-75`) — e.g. `weight = pow(0.5, age_days / half_life_days)`; the per-source value is `sum(weight*value)/sum(weight)`. A naive `LEFT JOIN` of audits onto provenance double-weights a multi-block source — do the inner aggregate first (spec §3.1's echo warning).
  - **`standing_band`** — DROP the old `(double precision,int,double precision,double precision,double precision)` signature, CREATE the new `(int,int,double precision,double precision,double precision)` with **[review] four arms** (spec §3.1), coverage-ratio-gated:
    - `near-canonical`: `citation_magnitude >= 3 AND audit_coverage::float / citation_magnitude >= 0.75 AND citation_quality > 0.5 AND contradiction_balance >= 0.0`
    - `reinforced`: `citation_magnitude >= 2 AND audit_coverage::float / citation_magnitude >= 0.5 AND citation_quality > 0.0 AND contradiction_balance >= 0.0`
    - `disputed`: `audit_coverage > 0 AND citation_quality < 0.0`
    - else `provisional` (includes every unaudited finding: coverage 0 ⇒ ratio 0 ⇒ falls through to here).

    Guard the ratio against `magnitude = 0` (an unaudited or citation-less finding must land `provisional`, never divide-by-zero). Thresholds are tunable defaults this set owns.
  - **`refresh_resource_standing`** — CREATE OR REPLACE (signature unchanged). Drop the `refresh_independence_pairs` and `resource_adversarial_survival` calls; UPSERT the three new components + unchanged `contradiction_balance`/`freshness`/`r_parent`.
  - **`resource_standing_shape`** — DROP+CREATE (return type changed). **[review] Keep the `gated` CTE over `resources_readable_by` byte-for-byte** (`…memo.sql:239-242`) — it is the read's leak-safety.
  - **Retire the pairwise objects:** `DROP FUNCTION resource_independence_breadth(uuid); DROP FUNCTION refresh_independence_pairs(uuid); DROP FUNCTION resource_adversarial_survival(uuid); DROP TABLE kb_independence_pairs;`

    > **[review] Non-additive migration — header comment must say so.** This drops three columns and four functions, breaking the `additive-only-on-main` invariant that makes `main` auto-deploy safe. The DROPs are correct only because Task 7's binary (which stops calling them) ships in the same deploy. Task 14 adds the `DEPLOYING.md` cutover entry.

- [ ] **Step 3: Prune the orphaned tests, then write the new ones.** **[review]** Four existing tests in `evidential_standing.rs` call dropped objects and must be **deleted** (not left to fail): `silence_default_is_correlated` (`:204`), `affirmed_independence_raises_breadth` (`:227`, the only writer of `'independent-of'`), `zero_challenges_is_not_survival` (`:287`), `band_is_read_time_over_components` (`:311`, passes the old 5-arg `standing_band` signature). Then add:

  1. `magnitude_counts_distinct_live_sources_not_provenance_rows` — ten citations of ONE source → `magnitude=1, r_parent=10`; **and** a soft-deleted source stops counting (`magnitude` drops after `is_active=false`). **[review]**
  2. `a_source_cited_by_two_blocks_counts_once_in_quality` — audit the same source on two blocks with `+1.0` and `-1.0`; assert the source contributes **one** per-source value, not two votes. **[review]**
  3. `adding_unaudited_citations_does_not_demote` — a finding audited-positive stays its band when four more unaudited sources are added (the perverse-gradient regression — spec §3.2). **[review]**
  4. `unaudited_finding_is_provisional_regardless_of_magnitude` — magnitude 5, coverage 0 → `provisional`.
  5. `a_negative_audit_yields_disputed` — coverage>0, quality<0 → `disputed`. **[review]**
  6. `recent_audit_outweighs_older_opposite` — an old `-1.0` then a recent `+1.0` yields positive quality, but both rows persist (decay + append-only — spec §4.1).
  7. `standing_shape_returns_none_for_an_unreadable_finding` — the gate, at the new return shape.
  8. **[review]** `which_band_the_shipped_write_paths_reach` — build a finding using ONLY audit writes (no `supports` edge, since nothing writes those) and assert the maximum band reachable. If `near-canonical` requires `contradiction_balance` and nothing writes it, this test **documents** that ceiling as a fact rather than leaving a silent dead threshold (spec §9).

- [ ] **Step 4: Run, implement, run, regenerate, commit.**

```bash
cargo nextest run -p temper-substrate --features artifact-tests --test evidential_standing
cargo sqlx prepare --workspace -- --all-features
git add migrations/20260723000020_standing_citation_components.sql crates/temper-substrate/tests/evidential_standing.rs .sqlx
git commit -m "Task 3: three axes, decay-weighted quality, and the pairwise model retired with its tests"
```

---

## Task 4: Readback shape + a shared visibility predicate

**Grounding tag:** CONFORM — `readback/mod.rs`'s `resource_standing` (`:900-945`) and `ensure_visible` (`:143-160`).

**Files:** Modify `crates/temper-substrate/src/readback/mod.rs`; Test in-file.

**Interfaces:**
- Produces: `StandingShapeRow { finding_id, citation_magnitude: i32, audit_coverage: i32, citation_quality: f64, contradiction_balance: f64, freshness: f64, r_parent: f64, band: String }`; `pub async fn is_resource_visible(pool, principal: ProfileId, resource: ResourceId) -> Result<bool>`.

- [ ] **Step 1:** Update `StandingShapeRow` fields and the `SELECT` column list in `resource_standing` (`:926-928`, `:935-944`) to the new shape.

- [ ] **Step 2: Extract and expose the visibility predicate.** `ensure_visible` (`:143`) inlines `SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)` (`:152`). Extract that one query into `pub async fn is_resource_visible(pool, principal, resource) -> Result<bool>` and have `ensure_visible` call it — **[review] one spelling, so Task 6's gate and this read cannot drift.** (Note: `resource_standing_shape`'s gate uses `resources_readable_by('profile', …)`, which *delegates to* `resources_visible_to` for the profile kind — `20260712000010:419` — so the two are equivalent for the profile principal the auditor uses; calling `resources_visible_to` directly is the incumbent Rust-callable predicate.)

- [ ] **Step 3: Test** `is_resource_visible_true_for_a_readable_finding` / `_false_for_an_unreadable_one`, then commit.

```bash
cargo nextest run -p temper-substrate --features artifact-tests --test <the readback test target>
git add crates/temper-substrate/src/readback
git commit -m "Task 4: the readback's new shape, and one visibility predicate the gate can share"
```

---

## Task 5: The auditor's principal-scoped selection sweep (SQL)

**Grounding tag:** EXTEND — spec §6.3. Modelled on `steward_drift_sweep(p_principal, p_threshold)` (`migrations/20260705000002_steward_drift_sweep.sql:19`, routed through `steward_candidate_cogmaps`).

**Files:** Create `migrations/20260723000030_audit_drift_sweep.sql`; Test `crates/temper-substrate/tests/citation_audits.rs` (extend)

**Interfaces:**
- Produces: `audit_drift_sweep(p_principal uuid, p_limit int) RETURNS TABLE(cogmap_id uuid, finding_id uuid, uncovered int)`.

- [ ] **Step 1: Write the migration.** **[review] The signature takes the principal first** and gates through the same readability every read uses — `steward_candidate_cogmaps(p_principal)` or a `resources_visible_to(p_principal)` join. An ungated sweep is a cross-tenant enumeration oracle that defeats Task 6's entire `NotFound` posture. Select findings where `resource_citation_magnitude(r.id) > 0 AND resource_audit_coverage(r.id) < resource_citation_magnitude(r.id)`, **[review] and `r.is_active AND r.ingest_state = 'complete'`** (spec §3.1 — a deleted or half-uploaded finding must not head the queue forever), joined to their cogmap home (`kb_resource_homes` where `anchor_table='kb_cogmaps'` — this join **is** the §6.2 cogmap-only boundary), ordered by `uncovered DESC`, limited to `p_limit`.

- [ ] **Step 2: Tests.**
  1. `sweep_returns_a_finding_with_uncovered_citations`
  2. `sweep_omits_a_fully_covered_finding`
  3. `sweep_omits_a_finding_with_no_resource_citations`
  4. `sweep_omits_a_context_homed_finding` (the §6.2 boundary)
  5. `sweep_omits_a_deleted_or_in_progress_finding` **[review]**
  6. `sweep_omits_a_finding_the_principal_cannot_read` **[review]**
  7. `sweep_orders_by_uncovered_descending`

  > **[review] Known first-cut limitation to note in the migration header, not fix:** because coverage is monotone (append-only), a readable live finding whose remaining sources the auditor *declines* to verdict stays uncovered and re-heads the queue each tick. The resource-kind + liveness + readability filters remove the common causes (remote/deleted/unreadable sources); a terminal "cannot assess" verdict or a per-finding backoff is the real fix and is deferred to the reaper pass (spec §6.3).

- [ ] **Step 3: Run, implement, run, regenerate, commit.**

```bash
cargo nextest run -p temper-substrate --features artifact-tests --test citation_audits
cargo sqlx prepare --workspace -- --all-features
git add migrations/20260723000030_audit_drift_sweep.sql crates/temper-substrate/tests/citation_audits.rs .sqlx
git commit -m "Task 5: the auditor's queue is coverage, principal-scoped, and live-only"
```

---

# Phase B — Surfaces

## Task 6: `AuditAuthority` — readability, minus self-audit

**Grounding tag:** EXTEND — spec §7 (discharged here). CONFORM — the `ScopedAuthority` trait + sealed proof (`crates/temper-services/src/authz/mod.rs:54-133`), the `NotFound`-dialect impls (`authz/read_gates.rs`).

**Files:** Create `crates/temper-services/src/authz/audit_gate.rs`; Modify `authz/mod.rs` (module decl). **[review] Tests live in an in-file `#[cfg(all(test, feature="test-db"))] mod tests`** — the authority types are `pub(crate)` and an integration test under `tests/` cannot name them (the pattern is `steward_service.rs:111`).

**Interfaces:**
- Produces: `pub(crate) enum AuditAuthority { Auditor, Author, Unreadable }` with `impl ScopedAuthority for AuditAuthority { type Subject = ResourceId; }`; a helper to resolve a block's owning finding.

- [ ] **Step 1: Read** `authz/mod.rs:54-133` and `authz/read_gates.rs` in full — match the structure and the comment discipline (each impl says *why* its dialect is what it is).

- [ ] **Step 2: Implement.** **[review] Five decisions (spec §7 lists five; the earlier plan dropped #5):**
  1. **Subject = the finding derived from the target block** — the caller does not name it. The service (Task 8) resolves `SELECT resource_id FROM kb_content_blocks WHERE id=$block` and authorizes *that*, so a caller cannot authorize over a readable finding while writing onto a block of an unreadable one (the transposition the sealed proof stops, `authz/mod.rs:110-117`).
  2. **`resolve` calls `readback::is_resource_visible`** (Task 4) — *"SQL predicates are authoritative — call them, do not restate them"* (`authz/mod.rs:65-66`).
  3. **Two denial arms:** `Unreadable` (not visible) **and** `Author` — **[review] the citer may not audit their own work** (spec §7). `Author` is when the caller `can_modify_resource(finding)` (the cheap sufficient proxy for "authored the citation"). Without this, readability alone lets the citer self-grade and dodge the queue, defeating the adversarial premise.
  4. **`denial()` = `ApiError::NotFound`** — the evidence *read* over this subject is already leak-safe by zero-rows→404 (`…memo.sql:239-242`), so the *write* refuses in the same dialect rather than becoming an existence oracle. Comment it, `read_gates.rs:53-59` style. (`is_denial` matches both `Unreadable` and `Author`.)
  5. **Machine reach — grounded, not assumed.** Read `authz/machine.rs`; confirm what grant rows Task 12's `provision --team <ref>:member` creates and that `resources_visible_to(<auditor machine profile>)` returns the corpus. State the finding in a doc comment. The failure this guards is silent: everything builds and every audit 404s in prod.

- [ ] **Step 3: Tests** (in-file `mod tests`):
  1. `a_reader_who_is_not_the_author_may_audit` — the whole point (spec §7).
  2. `the_author_of_the_finding_is_refused` **[review]** — self-audit denial.
  3. `a_principal_who_cannot_read_is_refused`.
  4. `both_denials_render_not_found` — assert the variant, so a later consistency pass cannot convert it to a leak.

- [ ] **Step 4: Run, implement, commit.**

```bash
cargo nextest run -p temper-services --features test-db --lib authz::audit_gate
git add crates/temper-services/src/authz
git commit -m "Task 6: an auditor may audit what it can read but not what it wrote"
```

---

## Task 7: The command, all three backends, the invocation gate, and the refresh clock

**Grounding tag:** CONFORM — the operations-command model; `tick_resource_standing`'s never-fail-the-write policy (`db_backend.rs:1147-1185`); `check_act_invocation` (`db_backend.rs:1196`, called at `:1808`); the three `Backend` impls.

**Files:** Modify `crates/temper-workflow/src/operations/{commands,backend}.rs`, `crates/temper-services/src/backend/db_backend.rs`, **[review]** `crates/temper-cli/src/cloud_backend/backend.rs` (both impl blocks); Test `crates/temper-services/tests/standing_clock_test.rs` (extend)

**Interfaces:**
- Produces: `RecordCitationAudit { block: BlockId, source: ProvenanceSource, value: f64, reason: Option<String>, act: ActContext, origin: Surface }`; `Backend::record_citation_audit(&self, cmd) -> Result<CommandOutput<Uuid>, TemperError>`.

- [ ] **Step 1: Add the command** (`commands.rs`) matching `AssertRelationship` (`:207-219`) incl. `act`/`origin`; add a round-trip unit test beside `assert_relationship_command_round_trips` (`:434`).

- [ ] **Step 2: Add the trait method** (`operations/backend.rs`, beside `assert_relationship` `:102`).

- [ ] **Step 3: Implement on `DbBackend`**, in order:
  1. **Authorize.** Resolve the block's finding, `authorize::<AuditAuthority>(&pool, profile, finding)` (Task 6). Authorization lives **here** (or in the Task 8 service — pick one and state it); the backend does not double-gate.
  2. **[review] `self.check_act_invocation(cmd.act.invocation).await?`** — the correlation-integrity gate every authored write runs (`db_backend.rs:1808`), additive to authz. The auditor runs inside invocations, so an audit onto a closed/unreadable envelope must 409/404.
  3. `writes::record_citation_audit_with`.
  4. Resolve the finding and `self.tick_resource_standing(finding)` — **CONFORM** its log-and-swallow policy (`:1151-1154`), never fail the write.

- [ ] **Step 4: [review] Implement on both `CloudBackend` blocks** (`temper-cli/src/cloud_backend/backend.rs:74` embed, `:505` non-embed): the embed arm POSTs to `/api/resources/{id}/citation-audits` via the Task 8 client method; the non-embed arm returns the standard cloud-mode error. Verify with `cargo check -p temper-cli --no-default-features` **and** `--all-features`.

- [ ] **Step 5: [review] Replace the `TODO(Set 5)` doc comment** (`db_backend.rs:1165-1172`) with the accurate note (spec §3.4): the independence-edge staleness it warned about is gone (breadth reads citations now), **but** `contradiction_balance` is unchanged and still reads edges, so the memo *column* is stale after edge writes — harmless only because `resource_standing_shape` recomputes live; any future direct memo-column reader must first wire an edge-incident refresh for `contradiction_balance`.

- [ ] **Step 6: [review] Extend the clock test with a fixture that can move quality.** The existing `standing_clock_test.rs` fixture has no provenance, so quality is the no-bases `0.0` both sides (`:118-120`). Build a source resource + annotate the finding's block with a resource-kind citation, then `recording_an_audit_moves_quality_off_zero` — assert `citation_quality` both before (`0.0`) and after (`>0`), so the assertion cannot pass vacuously against the column default.

- [ ] **Step 7: Run, regenerate per-crate cache, commit.**

```bash
cargo nextest run -p temper-services --features test-db --test standing_clock_test
cargo check -p temper-cli --no-default-features && cargo check -p temper-cli --all-features
cargo make prepare-services
git add crates/temper-workflow crates/temper-services crates/temper-cli
git commit -m "Task 7: authorize, gate the envelope, write, tick — across all three backends"
```

---

## Task 8: The API surface, the wire types, and the client method

**Grounding tag:** CONFORM — `handlers/evidence.rs` (the Set 3 sibling), `routes!` mounting, `temper-client`'s `resources().evidence()`.

**Files:** Create `crates/temper-core/src/types/citation_audit.rs`, `crates/temper-api/src/handlers/citation_audits.rs`, `crates/temper-services/src/services/citation_audit_service.rs`; Modify `temper-core/src/types/{standing,mod}.rs`, `temper-services/src/services/evidential_standing_service.rs`, `temper-client/src/resources.rs`, `temper-api/src/{routes,openapi}.rs`; Test `crates/temper-api/tests/citation_audit_handler_test.rs`, **[review]** modify `crates/temper-api/tests/evidence_handler_test.rs`

**Interfaces:**
- Produces: `POST /api/resources/{id}/citation-audits` taking `CitationAuditRequest { block_id: Uuid, source: ProvenanceSource, value: f64, reason: Option<String> }`, returning the audit id. `StandingShape` gains `citation_magnitude: i32`, `audit_coverage: i32`, `citation_quality: f64`; loses `indep_breadth`, `adversarial_survival`, `challenge_count`. `temper_client … resources().record_citation_audit(id, req)`.

- [ ] **Step 1: Update `StandingShape`** (`temper-core/src/types/standing.rs`) to the new fields, keeping the derive stack and the shape-primary framing; update `evidential_standing_service::resource_evidence`'s mapping and `StandingShapeRow`→`StandingShape` field names.

- [ ] **Step 2: [review] Fix the restaled Set 3 test.** `crates/temper-api/tests/evidence_handler_test.rs:33,39,40` assert `challenge_count`/`indep_breadth`/`adversarial_survival`; rewrite to the new fields. (The e2e's exact-field-set assertion is Task 11.)

- [ ] **Step 3: Add `CitationAuditRequest`** in `citation_audit.rs` with the same derive stack as `StandingShape` (ts-rs export, `utoipa::ToSchema` under `web-api`, `schemars` under `mcp`). Use `ProvenanceSource` for the source. Register the ts-rs export tree.

- [ ] **Step 4: Service + handler.** The service authorizes via `authorize::<AuditAuthority>` (resolving the finding from `request.block_id` — Task 6/7) and dispatches `RecordCitationAudit` through the backend. **[review] 404 if the resolved finding ≠ path `{id}`.** Handler is thin. Never call persistence from the handler.

- [ ] **Step 5: Mount via `routes!`** (copy `handlers::evidence` mounting) so it enters the OpenAPI contract.

- [ ] **Step 6: [review] Add the client method** `record_citation_audit(resource_id, CitationAuditRequest)` in `temper-client/src/resources.rs` beside `evidence` (`:196`) — Task 7's CloudBackend arm and Task 11's e2e both need it.

- [ ] **Step 7: Tests** (`citation_audit_handler_test.rs`, modelled on `evidence_handler_test.rs`):
  1. `posting_an_audit_returns_the_audit_id`
  2. `posting_an_audit_without_auth_returns_401`
  3. `posting_on_an_unreadable_finding_returns_404`
  4. `posting_as_the_author_returns_404` **[review]** (self-audit, surface-level)
  5. `posting_a_block_of_another_finding_returns_404` **[review]** (transposition)
  6. `posting_an_out_of_range_value_returns_400`

```bash
cargo nextest run -p temper-api --features test-db --test citation_audit_handler_test --test evidence_handler_test
```
> **Gotcha:** never a bare `cargo nextest run -p temper-api` (no `--test`) — it hangs at list enumeration on the bin target.

- [ ] **Step 8: Commit.**

```bash
git add crates/temper-core crates/temper-services crates/temper-api crates/temper-client
git commit -m "Task 8: the audit write surface, its wire type, and the client that reaches it"
```

---

## Task 9: The MCP tool

**Grounding tag:** CONFORM — `crates/temper-mcp/src/tools/relationships.rs` (an authored graph write with an `ActContext`).

**Files:** Create `crates/temper-mcp/src/tools/citation_audits.rs`; Modify `tools/mod.rs`

- [ ] **Step 1: Read `tools/relationships.rs`** — match its param struct, authorship threading, error mapping.

- [ ] **Step 2: Add `record_citation_audit`** (the auditor agent calls this): params carry `block_id`, `source: ProvenanceSource`, `value`, `reason`, and the standard act/authorship fields. **[review] `ProvenanceSource` already derives `schemars(inline)`** (`provenance.rs:33`) so it reaches the tool-use layer with a visible shape; confirm any local enum params do too.

- [ ] **Step 3: Test, run, commit.**

```bash
cargo nextest run -p temper-mcp <the tool test>
git add crates/temper-mcp
git commit -m "Task 9: the MCP tool the auditor calls"
```

---

## Task 11: The CLI evidence renderer + e2e

**Grounding tag:** CONFORM — the `temper resource evidence` action shipped in Set 3 (`crates/temper-cli/src/commands/resource.rs:1449`).

**Files:** Modify `crates/temper-cli/src/commands/resource.rs`; **[review]** `tests/e2e/tests/resource_evidence_test.rs`

- [ ] **Step 1: [review] Confirm the renderer needs no per-field change.** It serializes the whole `StandingShape` (`resource.rs:1464-1465`), so the new fields render automatically; the band is already carried WITH the shape (spec §1.1). Verify, don't rewrite.

- [ ] **Step 2: [review] Rewrite the e2e's field assertions (replace, not extend).** `resource_evidence_test.rs:96-98,135,139` assert the retired names by string literal — an **exact set**, not a subset. Replace with `citation_magnitude`/`audit_coverage`/`citation_quality`. Then extend: create a source resource, annotate the finding's block with a **resource-kind** citation (not the fixture's `https://…` remote source, which would give magnitude 0), record an audit via the client (Task 8), and assert `citation_quality` off `0.0` and `audit_coverage` incremented.

- [ ] **Step 3: Run.**

```bash
cargo build --bin temper           # e2e spawns the built binary; nextest does NOT rebuild it
cargo make test-e2e
cargo make prepare-e2e
```

- [ ] **Step 4: Commit.**

```bash
git add crates/temper-cli tests/e2e
git commit -m "Task 11: evidence renders the new shape, end to end through the CLI"
```

---

# Phase C — The persona

## Task 12: Provision the auditor's machine principal

**Grounding tag:** CONFORM — spec §5.2; `profile_service.rs:230-241` (lookup-or-401, no JIT).

**Files:** Modify `DEPLOYING.md`

- [ ] **Step 1: Document the provisioning**, beside the steward's setup:

```bash
temper admin machine provision --client-id <auditor-client-id> --label "citation auditor" --team <team-ref>:member
```

- [ ] **Step 2: Write the rationale** (not just the command): the auditor **must not** share the steward's `client_id` — one credential is one `emitter_entity_id`, and the ledger would be unable to tell an audit from the citation it audits, collapsing "assessed by another party" into "asserted by the same party wearing a label" (spec §5.2). **[review] Cross-reference Task 6 decision 5:** the `--team … :member` reach must actually make `resources_visible_to(<auditor profile>)` return the corpus, or every audit 404s.

- [ ] **Step 3: Commit.**

---

## Task 13: The auditor dispatch command + schedule

**Grounding tag:** CONFORM — `schedules/steward.ts` (correlation-id threading, `temperFetch` auth ordering); **[review]** the steward dispatch is a **`Backend` command**, not a bare service fn (`handlers/steward.rs:189-193` builds `DbBackend` and calls `steward_dispatch_tick`).

**Files:** Create `packages/agent-workflows/steward/agent/schedules/auditor.ts`, `channels/auditor-worker.ts`, `tests/auditor.test.ts`, `crates/temper-api/src/handlers/auditor.rs`, `crates/temper-services/src/services/auditor_service.rs`; Modify `operations/{commands,backend}.rs`, `db_backend.rs`, `temper-cli/.../cloud_backend/backend.rs`, `routes.rs`

- [ ] **Step 1: Read `schedules/steward.ts` in full** — the header documents why the handler is code-driven, how the correlation id threads cron → dispatch → session, and why the fan-out is over the workflow not the target.

- [ ] **Step 2: [review] Add an `AuditorDispatchTick { cap, correlation, origin }` command + `Backend::auditor_dispatch_tick`** on all three backends, mirroring `StewardDispatchTick` (`commands.rs`, `handlers/steward.rs:189`). The endpoint `POST /api/auditor/dispatch` runs `audit_drift_sweep(principal, cap)` (Task 5) → enqueue → claim. Header `x-auditor-correlation-id`, parsed leniently (malformed warns and self-roots, never 400 — `steward.rs:172-183`).

- [ ] **Step 3: [review] Enqueue one job per cogmap, carrying the uncovered finding list.** The single-flight index is `(cogmap_id, persona, dispatch_type)` (`workflow_jobs.sql:43-45`), so per-finding enqueues collide and `ON CONFLICT DO NOTHING` silently drops all but one (spec §6.1). Group the sweep's rows by `cogmap_id`, `workflow_job_enqueue(cogmap, 'auditor', 'citation-audit', jsonb_build_object('findings', <uncovered id list>))`, and the session iterates the list. Assert (Rust test) that N findings in one cogmap produce one job carrying N findings, not one finding.

- [ ] **Step 4: Mount the route, write the schedule.** Its own credential env names; **[review] its own model config — spec §5.3 recommends a different model from the steward** (the one lever against shared trained priors). One isolated session per claimed job.

- [ ] **Step 5: Write the session prompt.** Direct the agent to: read the finding's citations via `get_block_provenance`; pull `element_trail` **only** for the citing acts it weighs (discrete calls, never a bulk pass — spec §8); weigh the citing act's confidence/rationale, related resources, and citation-set size (spec §3.3); emit a signed verdict per **resource-kind** citation via the Task 9 tool with its **own** authorship; and — verbatim — *"assess only whether the source carries the connection claimed; never whether the claim is true, and never what the source says"* (spec §Bedrock, §3.4).

  > **[review] Known thin-input gap (spec §8):** `element_trail`'s DTO lifts only `confidence` from metadata, dropping `rationale`/`persona`/`model` (`element_trail.rs:37-39`), and `element_trail` is REST-only (no MCP tool). First cut accepts the thinner input — the prompt works from `confidence` + the citing act's payload. Widening the DTO / adding the tool is a follow-up, noted here so it is a decision, not a surprise.

- [ ] **Step 6: Test, commit.**

```bash
cargo nextest run -p temper-services --features test-db <auditor dispatch test>
cd packages/agent-workflows/steward && npm install && npm test
git add crates packages/agent-workflows/steward
git commit -m "Task 13: the auditor's tick — one job per cogmap, its own model, its own boundary"
```

---

## Task 10/14: Regenerate artifacts and final verification

*(Numbered last because it must run after every route/type change — Tasks 8 and 13 both touch the router.)*

**Grounding tag:** CONFORM — the four drift gates; the non-additive-migration cutover.

- [ ] **Step 1: Regenerate all downstream artifacts.**

```bash
cargo make openapi           # openapi.json + temper-rb gem (Docker) + temper-ts schema.ts
cargo make generate-ts-types # both ts-rs trees
```

- [ ] **Step 2: Stage, then check.** **[review] The drift gates diff against git — stage before checking** or a correctly-regenerated artifact still reds.

```bash
git add openapi.json clients packages/temper-ui/src/lib/types/generated packages/agent-workflows/mention/agent/generated
cargo make check
```

- [ ] **Step 3: UI check** (`cargo make check` does not cover temper-ui).

```bash
cd packages/temper-ui && bun install && bun run check
```

- [ ] **Step 4: Full local gate.**

```bash
cargo make test
cargo make test-db
cargo build --bin temper && cargo make test-e2e-embed   # -embed, or every test-embed test compiles out
cd packages/agent-workflows/steward && npm test
```

- [ ] **Step 5: Complete and stage the sqlx caches** (workspace, then per-crate):

```bash
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make prepare-e2e
git status --short
```

- [ ] **Step 6: [review] Add the DEPLOYING.md cutover entry.** Task 3's migration is **non-additive** (drops three columns + four functions), breaking `additive-only-on-main`. Add a cutover entry beside the WS6 precedent (`DEPLOYING.md:68`): backup → migrate → deploy binary → verify `GET /api/resources/{id}/evidence`; note every running site (temperkb.io, self-hosted) is cut over individually and that Task 3's migration + Task 7's binary must ship together.

- [ ] **Step 7: Update `CLAUDE.md`** — the evidential-standing entry describes `indep_breadth` and the pairwise model, both gone. Record the three-axis citation model and the append-only audit trail.

- [ ] **Step 8: Merge `origin/main`, re-run `cargo make check`, push, open PR.**

  > A clean auto-merge is not a passing build — a sibling's sealed field / renamed accessor compiles on their branch and breaks yours (PR #519). Re-check after merging. Never hand-merge generated-file conflicts — regenerate from the merged router.

- [ ] **Step 9: Commit the regenerated artifacts + docs.**

```bash
git add -A && git commit -m "Task 14: regenerate artifacts, cutover runbook, and the CLAUDE.md record"
```

---

## Self-Review

**Spec coverage:** §1 debts → Tasks 1,2,3,7. §2 reframe → 1,3. §3.1 three axes + per-source collapse + liveness → 3. §3.2 no-poisoned-mean → 3 (test `adding_unaudited_citations_does_not_demote`). §3.3 signed range → 1,9. §3.4 absorb/retire + honest TODO → 3,7. §4.1 append-only + decay + fresh-recompute → 1,2,3 (tests `two_audits…persist`, `recent_audit_outweighs_older_opposite`). §4.2 payload-vs-metadata → 2 (test). §4.3 refresh + domain category → 1,7. §5.2 principal separation → 12. §5.3 placement + distinct model → 13. §5.4 envelope unchanged → no task, deliberate. §6.1 queue reuse + grain fix → 13. §6.2 cogmap-only + resource-kind → 1,5. §6.3 principal-scoped coverage sweep → 5. §6.4 structural gate → 3 (band) + 6 (self-audit). §7 five decisions → 6. §8 restaling + thin-input gap → 8,10/14,13. §9 not-designed + reachability test → 3 (test `which_band…reach`).

**Review-finding coverage:** all 11 data-model findings, 21 implementability findings, and 13 drift findings have a home task, tagged **[review]** at the fix site. The three that were *dissolved* by the append-only decision (supersede/insert concurrency race, non-unique-live-index, idempotency-vs-supersession contradiction) are gone by construction — Task 1 has no supersession and no live/superseded distinction.

**Ordering:** dependency order (1→2→3→4→5→6→7→8→9→11→12→13→10/14) is stated in File Structure and each cross-task consume is named in an Interfaces block. Task 10/14 is numbered last on purpose.

**Type consistency:** `ProvenanceSource` (never `ProvenanceSourceKind`) across payloads/params/command/request/tool; `citation_magnitude`/`audit_coverage`/`citation_quality` identical in migration, readback, wire type, and tests; `RecordCitationAudit`/`AuditorDispatchTick` command names stable across commands.rs, backend.rs, and all three impls.
