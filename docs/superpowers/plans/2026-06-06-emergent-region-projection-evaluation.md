# Emergent Region Projection — Plan 3: The Falsification Evaluation

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development or superpowers:executing-plans. `- [ ]` checkboxes.
>
> **GROUNDING DISCIPLINE:** `~/.claude/skills/temper/guidance/implementation-grounding.md` — GD-1…GD-5.

> ## ✅ RE-GROUNDED 2026-06-06 (post-Plan-1 **and** Plan-2) — READY TO EXECUTE
> Plans 1 and 2 shipped; every forward-reference below was re-verified against live disk (GD-2 executable:
> `psql \d` on `temper_next`, grep over the shipped `temper-next` crate) and the plan corrected inline.
> Verified facts (quoted from live checks):
> - **block-content source reconciled (T1 ↔ Plan-2 T5):** shipped `crates/temper-next/src/embed.rs` reads
>   `kb_content_blocks b JOIN block_text bt ON bt.block_id = b.id … bt.body`. T1's
>   `block_text(block_id UUID PK REFERENCES kb_content_blocks(id), body TEXT NOT NULL)` matches the join +
>   column names **exactly**. ✓ No reconciliation needed — T1 is correct as written.
> - **Seed shape** (`03_seed.sql`): one `DO $seed$ … $seed$` block; `p_dave`, `c_onboarding`, `ev_assert`,
>   `e_agent`, `et_assert`, `ev_region` are in scope. The hand-seeded region is `03_seed.sql:230-239`
>   (`INSERT INTO kb_cogmap_regions … 'first-week confidence'` + its member insert + the
>   `shape_materialized_event_id` UPDATE). T2's removal target is exactly those lines. The telos-default
>   lens (`:224-229`, `RETURNING id INTO v_lens`) and `ev_region` (`:222`, the lens's `asserted_by_event_id`)
>   and the stale-edge block (`:241-247`, drives S6h staleness) all STAY. ✓
> - **Readout columns + functions** all present (verified `\d kb_cogmap_regions` + `\df`): `content_cohesion`,
>   `internal_tension`, `centrality`, `reference_standing`, `telos_alignment`; the 5 `cogmap_region_*`
>   functions; `cogmap_staleness(p_cogmap uuid)`; `cogmap_shape`. ✓
> - **S6h primitives**: `cogmap_staleness(uuid)` function + `relationship_asserted` event type both exist. ✓
> - **`kb_resources` has `origin_uri`** → the T3 suite's `origin_uri` joins are valid. ✓
>
> **Three corrections applied inline below** (search for `RE-GROUNDED FIX`): **(A)** T2 must DECLARE the new
> cast vars (or use nested `DECLARE … BEGIN … END` per concept, like the `b_reg`/`ch_reg` example at
> `03_seed.sql:206`) — `RETURNING id INTO r_pair` needs a declared target. **(B) [load-bearing]** S6f needs
> the harness to materialize a *named* lens, but shipped `substrate.rs` hardcodes `WHERE name='telos-default'`
> and `main.rs` takes only a cogmap arg — so Plan 3 must first EXTEND the harness with a lens-name parameter
> (new **Task 4 Step 0**). **(C)** the pre-existing `r_regulation` concept (`temper://reg/pair`, homed in
> onboarding at `:198-218`) stays a node but has NO `block_text` → the embed job's INNER JOIN skips it → its
> region would have a NULL centroid and break S6c/`materialize`'s "all regions content_cohesion NOT NULL".
> T2 must give the regulation block a `block_text` body (it already has block `b_reg`).
>
> **Note (not a fix):** removing the hand-seeded region (T2) makes `04_scenarios.sql`'s region lines
> (`cogmap_shape`, `cogmap_staleness` @ `:92-99`) show empty/fresh until the `temper-next` binary runs. Those
> lines are illustrative `\echo` SELECTs with **no hard asserts**, so this is expected, not a regression. The
> hard verdicts live in `04b` and run post-binary. `04a_plan1_fixture.sql` is self-contained (own fixture
> cogmap) and untouched.

**Goal:** Turn the artifact into a falsifiable experiment: author the α/β/bridge/tension/isolate cast with content engineered so declared-structure and cosine-structure **disagree**, run the harness, and assert the S6a–h verdicts — proving regions form from the declared graph (not cosine), with the surface↔relational cohesion split observable.

**Architecture:** Enrich `03_seed.sql` (cast + authored content + facets + edges); add a `block_text` eval table the embed job reads; extend the shipped harness with a lens-name parameter (T4 Step 0, for S6f plurality); add `04b_region_suite.sql` (the S6a–h psql verdicts run *after* the `temper-next` binary materializes). Load order becomes **01 → 02 → 03 → `temper-next` binary → 04b**.

**Tech Stack:** SQL/`psql`, the Plan-2 binary, pgvector. Spec §5 (the falsification frame, the 2×2, the cast table, S6a–h).

---

## File Structure

| File | Responsibility |
|------|----------------|
| `schema-artifact/01_schema.sql` | **Modify** — add the `block_text` eval table (block bodies for the embed job) |
| `schema-artifact/03_seed.sql` | **Modify** — replace the hand-seeded region with the enriched α/β/bridge/tension/isolate cast: concepts, homes, authored block content, facets, declared edges, a second lens for S6f |
| `schema-artifact/04b_region_suite.sql` | **Create** — S6a–h verdict queries over the materialized result |
| `schema-artifact/run_eval.sh` | **Create** — the full load → binary → suite runner |
| `crates/temper-next/src/{substrate.rs,main.rs}` | **Modify** (T4 Step 0) — add a lens-name parameter so S6f can materialize `telos-default-propheavy` |

---

## Task 1: The `block_text` eval table (block-content source for the embed job)

**Tag:** EXTEND (NEW eval-only table). The artifact's `kb_content_blocks` carries no body; production stores content per the content-block-primitive spec, but the *evaluation* needs prose to embed. A tiny `block_text` keeps that explicit and eval-scoped.

**Files:** Modify `schema-artifact/01_schema.sql`.

- [ ] **Step 1: Add the table** (after `kb_content_blocks`):
```sql
-- EVAL-ONLY (spec §5b / §6): block bodies the temper-next embed job chunks+embeds. Production stores
-- content per the content-block-primitive spec; this is the artifact's prose source, nothing more.
CREATE TABLE block_text (
    block_id UUID PRIMARY KEY REFERENCES kb_content_blocks(id) ON DELETE CASCADE,
    body     TEXT NOT NULL
);
```
- [ ] **Step 2: Reload + confirm** — `psql "$DB" -q -c '\d temper_next.block_text'` → table exists. Commit:
```bash
git add schema-artifact/01_schema.sql
git commit -m "feat(artifact): block_text eval table (prose source for the embed job, §5b)"
```

---

## Task 2: The enriched cast — authored content engineered for cross-axis disagreement

**Tag:** AMEND `03_seed.sql` (replaces the hand-seeded region; spec §5a/§5b). The **content is the independent variable** — α genuinely similar, β genuinely divergent, solo near-α but standalone — or the falsification cells (S6c/S6d) collapse and prove nothing.

**Files:** Modify `schema-artifact/03_seed.sql`.

- [ ] **Step 1: Remove the hand-seeded region** — delete exactly `03_seed.sql:230-239`: the
  `INSERT INTO kb_cogmap_regions … 'first-week confidence'` row, its `INSERT INTO kb_cogmap_region_members`,
  and the `UPDATE kb_cogmaps SET shape_materialized_event_id = ev_region` line (the harness sets that now).
  The harness produces regions. **KEEP** the telos-default lens (`:224-229`), `ev_region` (`:222` — the lens's
  `asserted_by_event_id`), and the late stale-edge block (`:241-247` — drives S6h). The `reg`/`v_centroid`
  vars become unused; either drop their DECLAREs or leave them (PL/pgSQL tolerates unused vars).

- [ ] **RE-GROUNDED FIX (C): give the existing regulation concept embeddable prose.** `r_regulation`
  (`temper://reg/pair`, homed in onboarding at `:198-218`) stays a clustered node but currently has NO
  `block_text` — the embed job's INNER JOIN `kb_content_blocks b JOIN block_text bt` would skip it, leaving
  its region with a NULL centroid and breaking S6c / `materialize`'s "all regions content_cohesion NOT NULL".
  Its content block `b_reg` already exists (`:206-208`). Add (inside that nested `BEGIN…END`, after `b_reg`):
  ```sql
  INSERT INTO block_text (block_id, body) VALUES (b_reg,
      'Always pair a newcomer with a maintainer on their first PR. The first contribution should be small '
   || 'and made alongside someone who knows the code, so confidence is built safely.');
  ```
  (Its content reads like α — fine; the regulation is express-edged off the charter, not part of the β/solo
  discriminating cells.) The existing `kb_chunk`/`kb_chunk_content` rows for `b_reg` (`:209-212`) collide with
  the embed job's `INSERT … ON CONFLICT (block_id, chunk_index, version) DO UPDATE` and get their `embedding`
  backfilled — consistent, not a duplicate.

- [ ] **Step 2: Author the cast.** For each concept: a `kb_resources` row, a `kb_resource_homes` row
  (`anchor_table='kb_cogmaps', anchor_id=c_onboarding`), one `kb_content_blocks` row, a `block_text` row with
  **genuinely-authored prose**, facets via `kb_properties`, and declared `kb_edges`.
  **RE-GROUNDED FIX (A):** the example below uses `RETURNING id INTO r_pair` / `… INTO b_tmp` — those targets
  must be DECLARE'd. Either add `r_pair r_smallest … b_tmp uuid;` to the seed's top `DECLARE` section, or wrap
  each concept in a nested `DECLARE … BEGIN … END` block (the pattern already used for `b_reg`/`ch_reg` at
  `:206`). A helper pattern (repeat per concept — DRY via a local loop is fine, but show one fully):

```sql
-- α1: pair-on-first-PR (content: early, small, safe, confident contribution)
INSERT INTO kb_resources (title, origin_uri) VALUES ('concept: pair-on-first-PR','temper://c/pair') RETURNING id INTO r_pair;
INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
    VALUES (r_pair,'kb_cogmaps',c_onboarding,p_dave,p_dave);
INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
    VALUES (r_pair,0,ev_assert,ev_assert) RETURNING id INTO b_tmp;
INSERT INTO block_text (block_id, body) VALUES (b_tmp,
    'Pair on the first pull request. A new engineer''s earliest change should be small and made '
 || 'alongside someone who knows the code, so the first contribution builds confidence safely rather '
 || 'than risking a large unfamiliar change. Small, paired, early — that is how confidence starts.');
INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id)
    VALUES ('kb_resources', r_pair, 'facet', '{"phase":"first-week"}'::jsonb, ev_assert, ev_assert);
```

Author the remaining cast with the **same shape**, content tuned to the axis it must land on:

| concept | content theme (authored to be…) | facet | edges |
|---|---|---|---|
| `pair-on-first-PR` (α1) | small/safe/early/confident | `{phase:first-week}` | `near` → α2, α3 |
| `smallest-real-change` (α2) | smallest change that builds confidence | `{phase:first-week}` | `near` → α1 |
| `early-confidence-signal` (α3) | reading early confidence | `{phase:first-week}` | `express` → α1 |
| `staging-rollout` (β1) | environments, promotion between stages | `{topic:deployment}` | `leads_to` → β2 |
| `feature-flags` (β2) | toggles, gating, percentage rollout | `{topic:deployment}` | `leads_to` → β3 |
| `rollback-runbook` (β3) | reverting, incident steps | `{topic:deployment}` | `leads_to` → β4 |
| `oncall-handoff` (β4) | shift handover, escalation paths | `{topic:deployment}` | (sink of the flow) |
| `deploy-confidence-checklist` (bridge) | deployment readiness checklist | `{topic:deployment}` | **no edge** (facet-only) |
| `blue-green` (tension1) | two identical environments, swap traffic | `{topic:deployment}` | `near`+label `contradicts` → big-bang |
| `big-bang-cutover` (tension2) | switch everyone at once | `{topic:deployment}` | (target of the contradicts edge) |
| `solo-retro-note` (isolate) | **content like α** (confidence/retro) | **none** | **none** |

> **Falsification check (GD-1):** β prose MUST read substantively unlike each other (a flags doc ≠ an oncall doc) and unlike α; `solo-retro-note` MUST read like α but carry no edge/facet. If you find yourself writing similar prose for β, STOP — you've collapsed the discriminating cells (spec §5b).

- [ ] **Step 3: A second lens for S6f (plurality)** — seed a `telos-default-propheavy` lens (same as telos-default but `w_prop` high, `w_leads_to` low), so S6f can show a different region-set over the same substrate:
```sql
INSERT INTO kb_cogmap_lenses (cogmap_id, name, selection_kind, w_express, w_contains, w_leads_to, w_near,
    w_prop, s_telos, s_ref, s_central, resolution, asserted_by_event_id)
VALUES (c_onboarding,'telos-default-propheavy','homed',1.0,1.0,0.1,0.3, 1.2,0.5,0.3,0.2,0.5, ev_assert);
```

- [ ] **Step 4: Reload + confirm clean** — `for f in 01 02 03; …` loads with no error; `SELECT count(*) FROM temper_next.block_text` ≥ 11. Commit:
```bash
git add schema-artifact/03_seed.sql
git commit -m "feat(artifact): enriched α/β/bridge/tension/isolate cast with authored falsification content (§5a/§5b)"
```

---

## Task 3: The S6a–h verdict suite

**Tag:** EXTEND (NEW `04b_region_suite.sql`, spec §5d). Runs **after** the `temper-next` binary materializes the telos-default regions.

**Files:** Create `schema-artifact/04b_region_suite.sql`.

- [ ] **Step 1: Write the suite.** Each block prints a labeled verdict. Region membership is resolved by joining `kb_cogmap_region_members` → `kb_resources.origin_uri`.

```sql
SET search_path = temper_next, public;
\echo '======== REGION SUITE (telos-default, post-materialize) ========'

-- helper: region id holding a given concept (by origin_uri), for the telos-default lens
-- (inline as subqueries below)

\echo '== S6a: ≥2 regions; α co-region; β co-region =='
SELECT (SELECT count(*) FROM kb_cogmap_regions r JOIN kb_cogmap_lenses l ON l.id=r.lens_id
          WHERE l.name='telos-default' AND NOT r.is_folded) AS region_count,
       (SELECT m1.region_id = m2.region_id
          FROM kb_cogmap_region_members m1, kb_cogmap_region_members m2
          WHERE m1.member_id=(SELECT id FROM kb_resources WHERE origin_uri='temper://c/pair')
            AND m2.member_id=(SELECT id FROM kb_resources WHERE origin_uri='temper://c/smallest')) AS alpha_together;
-- EXPECT: region_count >= 2, alpha_together = t

\echo '== S6c (HEADLINE): content_cohesion(α) > content_cohesion(β) =='
WITH areg AS (SELECT region_id FROM kb_cogmap_region_members
                WHERE member_id=(SELECT id FROM kb_resources WHERE origin_uri='temper://c/pair')),
     breg AS (SELECT region_id FROM kb_cogmap_region_members
                WHERE member_id=(SELECT id FROM kb_resources WHERE origin_uri='temper://c/staging'))
SELECT round((SELECT content_cohesion FROM kb_cogmap_regions WHERE id=(SELECT region_id FROM areg))::numeric,4) AS alpha_cohesion,
       round((SELECT content_cohesion FROM kb_cogmap_regions WHERE id=(SELECT region_id FROM breg))::numeric,4) AS beta_cohesion,
       (SELECT content_cohesion FROM kb_cogmap_regions WHERE id=(SELECT region_id FROM areg))
        > (SELECT content_cohesion FROM kb_cogmap_regions WHERE id=(SELECT region_id FROM breg)) AS surface_gt_relational;
-- EXPECT: surface_gt_relational = t   (β is coherent yet content-divergent — the relational-surplus region)

\echo '== S6d: solo-retro-note forms its OWN region (not absorbed into α despite content similarity) =='
SELECT (SELECT count(*) FROM kb_cogmap_region_members
          WHERE region_id=(SELECT region_id FROM kb_cogmap_region_members
                             WHERE member_id=(SELECT id FROM kb_resources WHERE origin_uri='temper://c/solo'))) AS solo_region_size;
-- EXPECT: solo_region_size = 1   (cosine did NOT form co-membership)

\echo '== S6e: bridge joins β via facet_overlap alone (no edge) =='
SELECT (SELECT region_id FROM kb_cogmap_region_members
          WHERE member_id=(SELECT id FROM kb_resources WHERE origin_uri='temper://c/checklist'))
     = (SELECT region_id FROM kb_cogmap_region_members
          WHERE member_id=(SELECT id FROM kb_resources WHERE origin_uri='temper://c/staging')) AS bridge_in_beta;
-- EXPECT: bridge_in_beta = t

\echo '== S6g: blue-green & big-bang co-region AND internal_tension > 0 =='
WITH treg AS (SELECT region_id FROM kb_cogmap_region_members
                WHERE member_id=(SELECT id FROM kb_resources WHERE origin_uri='temper://c/bluegreen'))
SELECT (SELECT region_id FROM kb_cogmap_region_members
          WHERE member_id=(SELECT id FROM kb_resources WHERE origin_uri='temper://c/bigbang'))
        = (SELECT region_id FROM treg) AS tension_together,
       (SELECT internal_tension FROM kb_cogmap_regions WHERE id=(SELECT region_id FROM treg)) > 0 AS tension_positive;
-- EXPECT: tension_together = t, tension_positive = t
```

> **GD-1:** the `origin_uri` literals (`temper://c/pair`, `/smallest`, `/staging`, `/solo`, `/checklist`, `/bluegreen`, `/bigbang`) MUST match exactly what Task 2 seeded. Cross-check against `03_seed.sql` before running — a typo silently yields NULL verdicts.

- [ ] **Step 2: Commit** — `git add schema-artifact/04b_region_suite.sql && git commit -m "feat(artifact): S6a–h region falsification suite"`

---

## Task 4: The end-to-end runner + S6b/S6f/S6h

**Tag:** EXTEND (NEW `run_eval.sh`). Ties the load order together and adds the verdicts that need the binary run more than once.

**Files:** Create `schema-artifact/run_eval.sh`; **modify** `crates/temper-next/src/{substrate.rs,main.rs}` (Step 0).

- [ ] **RE-GROUNDED FIX (B) — Step 0: extend the harness to materialize a *named* lens (prerequisite for S6f).**
  Shipped `substrate.rs` hardcodes `WHERE name='telos-default'` and `main.rs` takes only a cogmap arg, so the
  harness cannot materialize `telos-default-propheavy` as S6f requires. Add a lens-name parameter (Tag: EXTEND,
  spec §5c plurality / OQ-2 — same producer, different lens args). Concrete, minimal diff:
  - `substrate::cogmap_by_name` is fine. Change `substrate::load(pool, cogmap)` →
    `load(pool, cogmap, lens_name: &str)` and bind `lens_name` in the existing lens query
    (`WHERE name=$2 AND (cogmap_id=$1 OR cogmap_id IS NULL)`), defaulting callers to `"telos-default"`.
  - `write::materialize_cogmap(pool, cogmap)` → `materialize_cogmap(pool, cogmap, lens_name)`, forwarded to
    `substrate::load`. The fold-prior `WHERE … lens_id=$3` already scopes by the loaded lens's id, so
    re-materializing a *different* lens does not fold the other lens's regions. ✓
  - `main.rs`: read an optional 2nd arg `lens = args().nth(2).unwrap_or("telos-default")`, pass it through.
  - TDD: extend `tests/cluster_determinism.rs`? No — this is DB-shaped. Add the lens-name assertion to the
    (now un-gated) `tests/materialize.rs`, or a focused substrate test that loads `telos-default-propheavy`
    and asserts a different `w_prop`. Keep the pure core (`affinity`/`cluster`) untouched (still lift-ready).
  Run `cargo make check` + the crate suite after this edit, commit it as its own `feat(temper-next):` commit
  before the eval-runner commit.

- [ ] **Step 1: Write the runner:**
```bash
#!/usr/bin/env bash
set -euo pipefail
DB="${DATABASE_URL:-postgresql://temper:temper@localhost:5437/temper_development}"
cd "$(dirname "$0")/.."
for f in 01_schema 02_functions 03_seed; do
  psql "$DB" -q -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql
done
DATABASE_URL="$DB" cargo run -q -p temper-next -- onboarding-cogmap   # embed + materialize (telos-default)
psql "$DB" -q -f schema-artifact/04b_region_suite.sql
```
> RE-GROUNDED: shipped `main.rs` takes the **cogmap name** as arg 1 (default `onboarding-cogmap`) and — after
> Step 0 — an optional **lens name** as arg 2 (default `telos-default`); it runs `embed_all_blocks` then
> `materialize_cogmap`. The runner line above matches. ✓ (Embedding needs the ONNX runtime present — the
> `temper-ingest` `embed` feature, bge-768 — so run on a box with it, matching the Embed CI job.)

- [ ] **Step 2: S6b (reproducibility)** — already a Rust test in Plan 2 (`materialize_is_reproducible_*`); reference it here, and add a shell cross-check: run the binary twice and diff the membership fingerprint table:
```bash
DATABASE_URL="$DB" cargo run -q -p temper-next -- onboarding-cogmap
A=$(psql "$DB" -tAc "SELECT md5(string_agg(member_id::text, ',' ORDER BY region_id, member_id)) FROM temper_next.kb_cogmap_region_members m JOIN temper_next.kb_cogmap_regions r ON r.id=m.region_id WHERE NOT r.is_folded")
DATABASE_URL="$DB" cargo run -q -p temper-next -- onboarding-cogmap
B=$(psql "$DB" -tAc "SELECT md5(string_agg(member_id::text, ',' ORDER BY region_id, member_id)) FROM temper_next.kb_cogmap_region_members m JOIN temper_next.kb_cogmap_regions r ON r.id=m.region_id WHERE NOT r.is_folded")
[ "$A" = "$B" ] && echo "S6b reproducible: PASS" || echo "S6b reproducible: FAIL"
```
> Note: requires `materialize_cogmap` to fold prior live regions (Plan 2 T6 does) so the second run replaces, not appends.

- [ ] **Step 3: S6f (plurality by varied input)** — using the Step-0 lens-name arg, run
  `cargo run -q -p temper-next -- onboarding-cogmap telos-default-propheavy`, then assert its region-set
  differs from telos-default's. Verdict: the bridge concept and a β concept that were split under
  telos-default merge under prop-heavy (or some membership delta), proving same-function-different-args.
  (The fold-prior is lens-scoped — Step 0 — so the two lenses' region sets coexist; compare by `lens_id`.)

- [ ] **Step 4: S6h (functorial update + staleness)** — emit one new `relationship_asserted` edge event linking `solo-retro-note` into α, re-run the binary, assert solo's region membership changed and `cogmap_staleness` reported stale between materializations. (Concrete psql: INSERT the edge + event, check `cogmap_staleness(onboarding) → is_stale=t`, re-materialize, assert solo now co-regions with α.)

- [ ] **Step 5: Commit** — `git add schema-artifact/run_eval.sh && git commit -m "feat(artifact): end-to-end eval runner + S6b/S6f/S6h verdicts"`

---

## Self-Review

**1. Spec coverage (§5):** cast §5a → T2 ✓ · authored-content-as-independent-variable §5b → T2 (with the falsification-check guard) ✓ · lens row §5c → Plan 1 T2 + the S6f second lens ✓ · S6a/c/d/e/g → T3 ✓ · S6b/S6f/S6h → T4 ✓ · the 2×2 (β must-form / solo must-not-merge) → S6c + S6d ✓.
**2. Placeholder scan:** the authored prose is *described by theme + one fully-worked example* (T2) rather than all 11 bodies inlined — this is the one deliberate exception, because the prose is creative content the implementer authors to the falsification spec, not mechanical code; the **guard** (must-be-divergent) is the spec the bodies satisfy. Every SQL verdict is complete and runnable.
**3. Consistency:** `origin_uri` literals shared T2↔T3 (with the GD-1 cross-check). Lens names (`telos-default`, `telos-default-propheavy`) consistent.
**4. Grounding:** RE-GROUNDED 2026-06-06 against live Plan-1 + Plan-2 disk — block-content source reconciled
with the shipped `embed.rs` (T1 ✓), seed shape + exact removal lines verified, all readout columns/functions +
`cogmap_staleness` + `relationship_asserted` confirmed present, `origin_uri` join keys confirmed. Three
corrections applied inline (A: DECLARE cast vars; B: harness lens-name extension for S6f as Task 4 Step 0;
C: `block_text` body for the pre-existing regulation concept) + one note (T2 leaves `04_scenarios` region
lines empty-until-binary, no hard asserts ⇒ expected).

---

**Plan 3 is RE-GROUNDED and READY TO EXECUTE** (verified against live Plan-1 + Plan-2 disk 2026-06-06; three
fixes applied inline — banner). It is the payoff: when S6c (α-cohesion > β-cohesion) and S6d (solo stays
singleton) both pass on genuinely-divergent content, "regions are computable from the declared graph, not
cosine" stops being a claim and becomes a demonstrated, falsifiable result. Execution order:
**Task 4 Step 0 (harness lens-name extension) → T1 → T2 → T3 → T4** — Step 0 first because the eval seed (T2)
references the second lens that S6f materializes. The one creative-authoring exception (the 11 prose bodies,
Self-Review #2) stands: the implementer authors them to the §5b falsification guard.
