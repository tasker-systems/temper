# Beat 0 — make `survey`'s region disclosure real

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `survey` declares `discloses: vec![Disclosure::Region]` and nothing delivers it. Carry the matched regions and their scores to the caller so the declaration describes the deployed system.

**Architecture:** The SQL already returns `region_id`; the compiler's projection drops it. Add a fifth column to the stage contract — exactly as `via` was added for `follow-from` on 2026-08-14 — carry it on `HitRow`, and aggregate distinct `(region, score)` pairs into `StageTrace`/`StageResult` at the assembler. **No SQL function changes.**

**Tech Stack:** Rust (temper-core wire types, temper-substrate compiler/executor, temper-services assembler), ts-rs + utoipa generated artifacts, `cargo nextest`, `#[sqlx::test]`.

**Spec:** `internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md` §0 (the correction) and §3 (the readout this unblocks). Task: [01a01f21-c2ab-78b0-ada5-e8190d9c0814](./01a01f21-c2ab-78b0-ada5-e8190d9c0814).

## Global Constraints

- **No SQL function change.** `__temper_ungated_survey` already returns `region_id` and `region_score` (`migrations/20260816000020_survey_act.sql`). If a migration seems necessary, **stop** — the premise is wrong and must be re-grounded.
- **The pair rule.** Whatever `StageTrace` carries, `StageResult` carries identically for a returned stage. Stated verbatim in the doc comments of `extent`, `terms_applied`, `refusal`, `input_ids` and `input_unusable`: *"the trace covers every stage and the results only the returned ones, so disagreeing copies would leave a reader unable to tell which was right."* Compute once, read twice.
- **`region_score` is carried raw.** It spans `[-0.57, 1.05]` and is an **OPEN ruling** `[2026-08-14, Pete]`. Do not normalize, clamp, or rescale it. Do not add a `QuantityScale` claim it does not have.
- **A declaration describes the DEPLOYED system.** The rule this whole task enforces; do not widen `discloses` anywhere else while here.
- **`grep`, not `rg`, in this repo.** `rg` mangles identifiers in this tree (renders type names as `n`) — observed 2026-08-20. Every search in this plan uses `grep`.
- **The pre-commit hook gates the WHOLE WORKSPACE, and it is the real definition of "done" for every task here** `[learned by executing Task 1 — 2026-08-20]`. It runs fmt, clippy across every crate, docs, an **OpenAPI drift check**, and — the moment a commit touches `packages/temper-ui/src/lib/types/generated/` — `svelte-check` and `biome` over the UI. Three consequences that shape every task below:
  - **No task may leave the workspace uncompilable**, even if its own crate builds. Adding a field to a wire type breaks every literal constructor of it downstream, and those must be satisfied in the *same* commit.
  - **Every commit that changes a wire type regenerates the derived artifacts in that commit.** Deferring them to a later task cannot work; the hook rejects the commit.
  - **Clippy over a cold workspace exceeds two minutes.** Give commit commands a generous timeout, and do not read a timeout as a failure — check `git log` before retrying.
- Rust edition/toolchain as configured. Run `cargo make check` before any commit that claims completion.

---

### Task 1: The wire type and the two carrier fields  ✅ `fd819fd9`

**Files:**
- Modify: `crates/temper-core/src/types/query/hits.rs` (add `RegionDisclosure`)
- Modify: `crates/temper-core/src/types/query/trace.rs:78-244` (`StageTrace`)
- Modify: `crates/temper-core/src/types/query/envelope.rs` (`StageResult`)
- Test: the `#[cfg(test)]` module already in `crates/temper-core/src/types/query/trace.rs`

**Interfaces:**
- Produces: `RegionDisclosure { region_id: Uuid, region_score: f64 }`; `StageTrace::disclosed_regions: Vec<RegionDisclosure>`; `StageResult::disclosed_regions: Vec<RegionDisclosure>`.
- Consumes: nothing from an earlier task.
- **⚠️ It does NOT compile standing alone** `[corrected — 2026-08-20]`. This plan originally claimed it did. `temper-core` builds, but `temper-services` does not: `query_read.rs` constructs both `StageResult` (`:573`) and `StageTrace` (`:611`) as **literals**, so both stop compiling the moment the field exists. Satisfy them in this same commit with `disclosed_regions: vec![]` — which is the *truthful* value at this point, not a placeholder, because nothing projects a region until Task 2. Say so in a comment; do not write a TODO. `envelope.rs`'s own test fixture `result()` (`:402`) needs the same.

**⚠️ `StageTrace` derives `Eq`, and `f64` is not `Eq`** `[found by executing — 2026-08-20]`. Adding `Vec<RegionDisclosure>` therefore breaks the derive, and the break **cascades** to `CompositionTrace`, which holds `Vec<StageTrace>`. Drop `Eq` from both, keep `PartialEq`. This is safe and was checked rather than assumed: `grep` for `StageTrace` in a `HashSet`/`BTreeSet`/map-key position returns nothing, and `StageResult` never derived `PartialEq` or `Eq` at all — so the two carriers were already asymmetric. Leave a note at each dropped derive; an unexplained missing `Eq` invites a future reader to restore it.

**Why a list and not an `Option`:** the same reasoning `ResourceHit::via` records — *"A collection has no such claim to carry: `[]` and absent would both mean 'no provenance', so a null adds a third spelling of one fact."* Empty means no regions disclosed; the act's `discloses` already says whether to expect any.

- [x] **Step 1: Write the failing test**

In `crates/temper-core/src/types/query/trace.rs`, inside the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn a_disclosed_region_carries_its_id_and_its_raw_score() {
    let d = RegionDisclosure {
        region_id: Uuid::nil(),
        // Deliberately outside [0,1]: `region_score` spans [-0.57, 1.05] and the carrier
        // must not clamp it. The open ruling [2026-08-14] is about whether the BLEND is
        // right, not about whether the number survives transport.
        region_score: 1.05,
    };
    let j = serde_json::to_value(&d).unwrap();
    assert_eq!(j["region_score"], serde_json::json!(1.05));
    let back: RegionDisclosure = serde_json::from_value(j).unwrap();
    assert_eq!(back.region_score, 1.05);
}

#[test]
fn disclosed_regions_defaults_empty_on_a_trace() {
    let t = sample_trace();
    assert!(t.disclosed_regions.is_empty());
}
```

If `sample_trace()` does not already exist in that module, use whatever constructor the neighbouring tests use — check with `grep -n "fn sample_trace\|StageTrace {" crates/temper-core/src/types/query/trace.rs`.

- [x] **Step 2: Run the test and watch it fail**

```bash
cargo nextest run -p temper-core a_disclosed_region_carries_its_id
```

Expected: FAIL — `cannot find type RegionDisclosure`.

- [x] **Step 3: Add the type**

In `crates/temper-core/src/types/query/hits.rs`, beside `RegionHit`, matching that file's existing derive stack (check it with `grep -n "derive" crates/temper-core/src/types/query/hits.rs | head -3` and copy — it will involve `Serialize`, `Deserialize`, and `cfg_attr` gates for `ts-rs`, `utoipa` and `schemars`):

```rust
/// One region a `survey` stage matched, with the score it matched at.
///
/// **Trace disclosure, never a row.** `survey` produces RESOURCES — the ⟨3⟩ redesign moved regions
/// out of the output precisely so a caller could not draw them as though the reader had authored
/// them. This carries which groupings answered, for a caller that wants to say *why these*, and it
/// deliberately has no per-resource mapping: that would put a region back on the row shape.
///
/// **`region_score` is raw and NOT in `[0,1]`.** It is `0.4·sal_norm + 0.6·query_cos + 0.05·prior`,
/// measured spanning `[-0.57, 1.05]`, and whether the `sal_norm` term belongs at all is an OPEN
/// ruling `[2026-08-14, Pete]`. Transporting it unchanged is what lets that ruling stay open; a
/// carrier that normalized it would silently settle the question it is not entitled to settle.
pub struct RegionDisclosure {
    pub region_id: Uuid,
    pub region_score: f64,
}
```

- [x] **Step 4: Add the field to both carriers**

In `trace.rs`, after `pub narrowed_by: Vec<NarrowedBy>,`:

```rust
    /// Which regions a `survey` stage matched, and at what score. Empty for every other act.
    ///
    /// **The pair rule**: [`super::envelope::StageResult::disclosed_regions`] carries the same
    /// value, for the same reason as `extent`, `terms_applied` and the input numbers — the trace
    /// covers every stage and the results only the returned ones, so disagreeing copies would
    /// leave a reader unable to tell which was right.
    #[cfg_attr(feature = "typescript", ts(optional_fields = false))]
    #[serde(default)]
    pub disclosed_regions: Vec<RegionDisclosure>,
```

Drop the `cfg_attr` line if the neighbouring fields in this struct do not use one — match the file, do not introduce a pattern. Add the identical field to `StageResult` in `envelope.rs` with a doc comment pointing back the other way.

- [x] **Step 5: Run the tests and watch them pass**

```bash
cargo nextest run -p temper-core --test-threads 4 disclosed_region
```

Expected: PASS, both tests.

- [x] **Step 6: Commit**

```bash
git add crates/temper-core/src/types/query/
git commit -m "feat(query): carry survey's disclosed regions on the trace and the result"
```

---

### Task 2: Carry the column through the compiler's stage contract  ✅ `c420c96d`

> **Two corrections from executing it** `[2026-08-20]`:
> - **A `survey_stage(name)` fixture ALREADY EXISTS** at `tests/query_plan_compile.rs:1634`, with a cogmap anchor and an embedding. Use it. Writing a second one is an E0428 duplicate-definition error.
> - **Do not assert the union column by counting occurrences.** The per-act CTE bodies name `region` too, so a count over the whole statement passes while an arm is still missing it — the first version of this test did exactly that. Slice per arm with the file's own `hit_arm` / `tally_arm` helpers.

**Files:**
- Modify: `crates/temper-substrate/src/readback/query_plan.rs` — survey arm `:657`, follow-from arm `:614`, the other act arms `:494`, `:527`, `:573`, `:687`, `final_select` `:1517-1545`, empty fallback `:348`
- Test: the `#[cfg(test)]` module in the same file

**Interfaces:**
- Consumes: nothing from Task 1 (SQL layer, no wire types).
- Produces: every emitted stage arm now selects a **fifth** column, `region`, of type `uuid`. Survey emits `region_id`; every other act emits `NULL::uuid AS region`.

**⚠️ Do not overload `via`.** It would type-check and it would be wrong: `via` means *how a walk reached this row*, and a second meaning on one field is the drift this contract keeps removing. `via`'s own arrival is the precedent to copy — `query_plan.rs:609`: *"**`via` crosses into the stage contract as a fourth column.** Every other act emits `NULL::jsonb` for it — see `final_select`, which shares one column list across hit arms, tally arms and the empty fallback."* Do exactly that, one column over.

- [x] **Step 1: Write the failing test**

Find this module's existing emission-test harness first — it exists, because every act arm above is already covered:

```bash
grep -n "#\[test\]" -A 3 crates/temper-substrate/src/readback/query_plan.rs | grep -i "fn .*emit\|fn .*arm\|fn .*select" | head
```

Write the two tests with **that** harness. What they must assert:

```rust
#[test]
fn a_survey_arm_projects_the_region_it_matched() {
    let sql = /* emit a validated single-stage survey composition, this module's way */;
    assert!(
        sql.contains("region_id::uuid AS region"),
        "survey must project its region; got:\n{sql}"
    );
}

#[test]
fn every_arm_of_the_union_carries_the_region_column() {
    // The hit arms, the tally arms and the empty fallback are ONE result set. A column in
    // some arms and not others is not a narrower disclosure — it is a SQL error at runtime,
    // which no compile-time test above can catch.
    let sql = /* the full compiled statement for a two-stage plan, this module's way */;
    let arms = sql.matches("SELECT 'hit'::text").count()
        + sql.matches("SELECT 'tally'::text").count();
    assert!(arms >= 2, "fixture must produce both a hit arm and a tally arm");
    assert_eq!(
        sql.matches("region").count() >= arms,
        true,
        "each arm must name the region column"
    );
}
```

- [x] **Step 2: Run and watch it fail**

```bash
cargo nextest run -p temper-substrate a_survey_arm_projects_its_region
```

Expected: FAIL — the assertion, showing SQL with `NULL::jsonb AS via` and no region column.

- [x] **Step 3: Widen the survey arm**

`query_plan.rs:655-659`, change:

```rust
                 SELECT resource_id AS id, 'resource'::text AS kind, \
                 region_score::double precision AS quantity, NULL::jsonb AS via\n    \
```

to:

```rust
                 SELECT resource_id AS id, 'resource'::text AS kind, \
                 region_score::double precision AS quantity, NULL::jsonb AS via, \
                 region_id::uuid AS region\n    \
```

- [x] **Step 4: Widen every other arm and the shared column list**

Append `, NULL::uuid AS region` to the projections at `:494`, `:527`, `:573`, `:614` (follow-from), `:687`, and to the empty fallback at `:348`.

In `final_select` (`:1517`), the hit arm becomes:

```rust
                "SELECT 'hit'::text AS row_class, '{s}'::text AS stage, id, kind, quantity, via, \
                 region, NULL::bigint AS produced, NULL::bigint AS unusable FROM \"{s}\""
```

the tally arm gains `NULL::uuid AS region,` after its `NULL::jsonb AS via,`, and the zero-arm fallback gains `NULL::uuid AS region,` likewise. **All three, or the UNION will not type.**

- [x] **Step 5: Run and watch it pass**

```bash
cargo nextest run -p temper-substrate --lib readback::query_plan
```

Expected: PASS, including every pre-existing emission test. A pre-existing test asserting an exact SQL string will fail here — that is the correct signal, not a problem; update its expectation to include the new column.

- [x] **Step 6: Commit**

```bash
git add crates/temper-substrate/src/readback/query_plan.rs
git commit -m "feat(query): add region to the stage contract, following via's precedent"
```

---

### Task 3: Carry it on `HitRow`  ✅ `28bc95ca`

**Files:**
- Modify: `crates/temper-substrate/src/readback/query_exec.rs:31-49` (`HitRow`) and the row-mapping site below it
- Test: same file's test module

**Interfaces:**
- Consumes: the `region` column from Task 2.
- Produces: `HitRow.region: Option<Uuid>` — `None` for every act that is not `survey`.

**Why `Option<Uuid>` and not a typed newtype:** `grep -rn "pub struct RegionId" crates/temper-core/src/types/` returns nothing — there is no region newtype in this codebase, and `HitRow.id` is already a bare `Uuid`. Follow the file.

- [x] **Step 1: Write the failing test**

```rust
#[test]
fn a_hit_row_carries_the_region_when_the_column_is_present() {
    let row = hit_row_from_test_columns(/* region = Some(uuid) */);
    assert_eq!(row.region, Some(the_uuid));
}

#[test]
fn a_non_survey_hit_row_carries_no_region() {
    let row = hit_row_from_test_columns(/* region = NULL */);
    assert_eq!(row.region, None);
}
```

- [x] **Step 2: Run and watch it fail**

```bash
cargo nextest run -p temper-substrate a_hit_row_carries_the_region
```

Expected: FAIL — `no field named region`.

- [x] **Step 3: Add the field**

In `query_exec.rs`, after `pub via: Option<serde_json::Value>,`:

```rust
    /// The region a `survey` row came from — the raw `region` column, `None` for every act that is
    /// not a survey.
    ///
    /// Kept as a bare `Uuid` because this crate has no dependency on `temper-core`'s wire types and
    /// must not grow one to carry a column through — the same reasoning `via` records one field up.
    pub region: Option<Uuid>,
```

and read it in the row mapping beside where `via` is read (`grep -n "via" crates/temper-substrate/src/readback/query_exec.rs` to find the site): `region: r.try_get("region").ok().flatten(),` — match the exact accessor style the neighbouring columns use rather than assuming `try_get`.

- [x] **Step 4: Run and watch it pass**

```bash
cargo nextest run -p temper-substrate --lib readback::query_exec
```

- [x] **Step 5: Commit**

```bash
git add crates/temper-substrate/src/readback/query_exec.rs
git commit -m "feat(query): carry the region column on HitRow"
```

---

### Task 4: Aggregate into the trace and the result  ✅ `f8b039dd`

**Files:**
- Modify: `crates/temper-services/src/backend/query_read.rs` — `stage_trace()` `:609`, the `StageResult` construction `:573-587`
- Test: same file's test module (it already has `HitRow`/`TallyRow` fixtures — `query_read.rs:911`)

**Interfaces:**
- Consumes: `HitRow.region` (Task 3), `RegionDisclosure` (Task 1).
- Produces: `disclosed_regions` populated identically on both carriers.

**The aggregation, and why it is `(region, quantity)` distinct pairs:** for a survey row the SQL projects `region_score AS quantity`, so every resource in one region carries that region's score as its quantity. Distinct `(region, quantity)` pairs therefore *are* the matched region set with its scores — no second query, no separate tally.

**Order them by score descending**, matching the act's `orders_by`. Ties break on `region_id` so the output is deterministic — a trace that reorders between identical runs is not a disclosure a reader can diff.

**⚠️ One definition, read twice.** Write `disclosed_regions_for(stage, rows)` once and call it from both `stage_trace()` and the `StageResult` construction. This is the pair rule's stated implementation shape — `terms_applied` records it as *"one `applied_terms` DEFINITION rather than two… Computed twice, they would eventually differ, and the difference would be a response claiming a page size that did not run."*

- [x] **Step 1: Write the failing test**

The existing fixture is `fn hit(stage: &str, id: Uuid, q: f64) -> HitRow` at `query_read.rs:1184`, and it constructs `HitRow { stage, id, kind, quantity, via }` **as a literal** — so it stops compiling the moment Task 3 lands. **That compile error is the correct signal, not an obstacle.** Give it the new field and add a sibling that sets a region:

```rust
    fn hit(stage: &str, id: Uuid, q: f64) -> HitRow {
        HitRow {
            stage: stage.to_string(),
            id,
            kind: "resource".to_string(),
            quantity: Some(q),
            via: None,
            region: None,
        }
    }

    /// A survey row: the region it came from, scored at that region's `region_score`.
    fn hit_in_region(stage: &str, id: Uuid, region: Uuid, q: f64) -> HitRow {
        HitRow { region: Some(region), ..hit(stage, id, q) }
    }
```

Build `QueryRows` the way the neighbouring tests do (`query_read.rs:1168` `no_rows()`, and the literals at `:1212` and `:1268`): `QueryRows { hits, tallies, refusals: vec![] }`.

```rust
#[test]
fn a_survey_stage_discloses_each_matched_region_once_ordered_by_score() {
    let (r1, r2) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let rows = QueryRows {
        hits: vec![
            hit_in_region("s1", Uuid::from_u128(10), r1, 0.91),
            hit_in_region("s1", Uuid::from_u128(11), r1, 0.91), // same region, same score
            hit_in_region("s1", Uuid::from_u128(12), r2, 0.44),
        ],
        tallies: vec![tally("s1", 3, 0)],
        refusals: vec![],
    };
    let d = disclosed_regions_for("s1", &rows);
    assert_eq!(d.len(), 2, "one entry per region, not per resource");
    assert_eq!(d[0].region_id, r1);
    assert_eq!(d[0].region_score, 0.91);
    assert_eq!(d[1].region_id, r2);
}

#[test]
fn a_walk_stage_discloses_no_regions() {
    let rows = QueryRows {
        hits: vec![hit("w", Uuid::from_u128(10), 0.7)],
        tallies: vec![tally("w", 1, 0)],
        refusals: vec![],
    };
    assert!(disclosed_regions_for("w", &rows).is_empty());
}

#[test]
fn a_negative_region_score_survives_the_carrier() {
    // `region_score` spans [-0.57, 1.05]. A carrier that clamped would silently settle the
    // OPEN blend ruling [2026-08-14] it is not entitled to settle.
    let rows = QueryRows {
        hits: vec![hit_in_region("s1", Uuid::from_u128(10), Uuid::from_u128(1), -0.57)],
        tallies: vec![tally("s1", 1, 0)],
        refusals: vec![],
    };
    assert_eq!(disclosed_regions_for("s1", &rows)[0].region_score, -0.57);
}
```

The pair rule gets its own assertion, built the way `the_page_a_stage_reports_is_the_clamped_one_the_statement_actually_ran` (`query_read.rs:1203`) builds its plan — `act_node(...)` then `plan(vec![node], vec!["s1"])`, then `assemble`:

```rust
#[test]
fn the_trace_and_the_result_carry_the_same_disclosed_regions() {
    let v = plan(vec![act_node("s1", ActName::Survey, None)], vec!["s1"]);
    let rows = QueryRows {
        hits: vec![hit_in_region("s1", Uuid::from_u128(10), Uuid::from_u128(1), 0.9)],
        tallies: vec![tally("s1", 1, 0)],
        refusals: vec![],
    };
    let res = assemble(&v, &rows, &no_hydration());
    let trace = res.trace.stages.iter().find(|s| s.stage.as_str() == "s1").unwrap();
    let result = res.returned.get("s1").unwrap();
    assert_eq!(result.disclosed_regions, trace.disclosed_regions);
    assert_eq!(result.disclosed_regions.len(), 1);
}
```

`act_node`, `plan` and the hydration fixture already exist in this module — confirm their exact signatures with `grep -n "fn act_node\|fn plan(\|fn no_rows\|Hydrated {" crates/temper-services/src/backend/query_read.rs` and match them rather than adapting the call above from memory.

- [x] **Step 2: Run and watch it fail**

```bash
cargo nextest run -p temper-services a_survey_stage_discloses_each_matched_region
```

Expected: FAIL — `cannot find function disclosed_regions_for`.

- [x] **Step 3: Write the one definition**

```rust
/// The regions a stage matched, one entry per region rather than per row.
///
/// **One definition, two readers** — `stage_trace` and the `StageResult` construction both call
/// this, so the pair rule holds by construction rather than by discipline. The function is pure;
/// that purity is the property to preserve if it is ever edited.
///
/// A survey row projects `region_score AS quantity`, so every resource in one region carries that
/// region's score. Distinct `(region, quantity)` pairs are therefore the matched set with its
/// scores — no second query, and no snapshot skew against the rows beside it.
fn disclosed_regions_for(stage: &str, rows: &QueryRows) -> Vec<RegionDisclosure> {
    let mut seen: BTreeMap<Uuid, f64> = BTreeMap::new();
    for h in rows.hits.iter().filter(|h| h.stage == stage) {
        if let (Some(region), Some(q)) = (h.region, h.quantity) {
            seen.entry(region).or_insert(q);
        }
    }
    let mut out: Vec<RegionDisclosure> = seen
        .into_iter()
        .map(|(region_id, region_score)| RegionDisclosure { region_id, region_score })
        .collect();
    // Score descending, matching the act's `orders_by`; `region_id` breaks ties so two identical
    // runs cannot disagree about order.
    out.sort_by(|a, b| {
        b.region_score
            .partial_cmp(&a.region_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.region_id.cmp(&b.region_id))
    });
    out
}
```

`rows.hits` is correct — `QueryRows { hits: Vec<HitRow>, tallies: Vec<TallyRow>, refusals: … }`, `query_exec.rs:69`.

- [x] **Step 4: Call it from both carriers**

In the `StageResult` construction (`:573`), after `narrowed_by: narrowed_by(node, rows),`, add `disclosed_regions: disclosed_regions_for(node.name().as_str(), rows),`. Add the identical line in `stage_trace()`.

- [x] **Step 5: Run and watch them pass**

```bash
cargo nextest run -p temper-services --lib backend::query_read
```

- [x] **Step 6: Commit**

```bash
git add crates/temper-services/src/backend/query_read.rs
git commit -m "feat(query): aggregate survey's matched regions onto the trace and the result"
```

---

### Task 5: Verify the derived artifacts, and the snapshots the hook does not check  ✅ `pending commit`

**✅ The rewrite was right.** Executed 2026-08-20: `openapi.json` was already clean (Tasks 1–4 each regenerated it to pass the hook), and **four schema snapshots had drifted unchecked** — `composition_trace`, `query_response`, `stage_result`, `stage_trace`. Regenerate with `UPDATE_SCHEMA=1 cargo nextest run -p temper-core --features mcp --test query_schema`, which the test's own header documents.

**⚠️ Rewritten `[2026-08-20]`. This task used to say "regenerate the artifacts" and was sequenced last. That is impossible:** the pre-commit hook fails any commit whose wire types drift from `openapi.json`, so **Tasks 1–4 each regenerate in their own commit** as a condition of landing at all. What is left here is the part the hook does *not* cover.

**Files:**
- Verify (generated): `packages/temper-ui/src/lib/types/generated/query.ts`, `openapi.json`, `clients/temper-rb/`, `clients/temper-ts/src/generated/schema.ts`
- Verify: the schema snapshots

**REQUIRED SUB-SKILL:** load the `generated-artifacts` skill. It owns which artifact regenerates from what, and this plan deliberately does not restate it — a second copy of that procedure is a second thing to drift.

**The regeneration commands each earlier task needs**, recorded once here so they are not rediscovered four times:

```bash
cargo make openapi              # openapi.json + the temper-rb gem + temper-ts schema.ts
cargo make generate-ts-types    # the ts-rs tree the UI reads
```

- [x] **Step 1: Confirm no artifact drifted**

```bash
cargo make openapi && cargo make generate-ts-types && git status --short
```

Expected: **no modifications**. Anything that changes here is an artifact an earlier task failed to regenerate, which means that task's commit should not have passed the hook — investigate rather than committing the diff.

- [x] **Step 2: Confirm the new type reached the UI**

```bash
grep -n "RegionDisclosure\|disclosed_regions" packages/temper-ui/src/lib/types/generated/query.ts
```

Expected: `RegionDisclosure` declared, and `disclosed_regions` present on **both** `StageTrace` and `StageResult`. If it appears on only one, Task 4 missed a carrier — go back.

- [x] **Step 3: Update the schema snapshots — the hook does NOT check these**

This is the substance of the task. `openapi` drift is caught per commit; the schema snapshots are not, so they are the artifact that can silently rot across Tasks 1–4.

```bash
cargo make test-schema-core
cargo make test-schema-substrate
```

Both regenerate a committed artifact. If either reports a diff, the snapshot needs committing — that is the expected outcome here, not a failure.

- [x] **Step 4: Full gate**

```bash
cargo make check
```

- [x] **Step 5: Commit — only if Step 1 or Step 3 actually changed something**

```bash
git add -A
git commit -m "chore(generated): update schema snapshots for the region disclosure"
```

**If nothing changed, this task ends with no commit, and that is the success case.** It means Tasks 1–4 each landed their own artifacts correctly. Do not manufacture a commit to make the task look done.

---

### Task 6: Prove it end to end against a database

**Files:**
- Create or modify: an integration test in `crates/temper-services` (or `temper-api`) under `#[cfg(all(test, feature = "test-db"))]`

**⚠️ `#[sqlx::test]` modules must be gated `cfg(all(test, feature = "test-db"))`** — an ungated module breaks a no-DB `cargo make test`. Verify with a no-DB run before claiming done.

**Why this task exists at all:** Tasks 1–4 each assert against fixtures. None of them proves the SQL column survives the round trip — the exact seam where a projection widening silently fails, because a `UNION` arm mismatch is a runtime error and every unit test above is compile-time.

- [ ] **Step 1: Write the failing test**

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn a_survey_composition_discloses_the_regions_it_matched(pool: PgPool) {
    let v = validated_survey_composition_for_test(/* an anchor with known regions */);
    let res = query_read::run_composition(&pool, principal, &v).await.unwrap();

    let trace = res.trace.stages.iter().find(|s| s.stage.as_str() == "s1").unwrap();
    assert!(
        !trace.disclosed_regions.is_empty(),
        "survey declares Disclosure::Region; the response must carry it"
    );
    // The declaration, now describing the deployed system.
    assert!(trace.disclosed_regions.len() <= 3, "regions_n defaults to 3");

    let result = res.returned.get("s1").unwrap();
    assert_eq!(result.disclosed_regions, trace.disclosed_regions, "pair rule");
}
```

Seed whatever fixture the neighbouring DB tests use; find them with `grep -rn "sqlx::test" crates/temper-services/src | head`.

- [ ] **Step 2: Run against the local database**

```bash
cargo nextest run -p temper-services --features test-db a_survey_composition_discloses
```

Docker Postgres is on port 5437; `DATABASE_URL` per `internal/agents/environment.md`.

- [ ] **Step 3: Confirm the no-DB build is unaffected**

```bash
cargo make test
```

Expected: PASS with the DB-gated module skipped. **If this fails to compile, the `cfg` gate is wrong** — that is the failure this step exists to catch.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-services/
git commit -m "test(query): prove the region disclosure survives the round trip"
```

---

## Out of scope, named so it is not mistaken for oversight

- **Per-resource region attribution.** The registry comment says *"the region each resource came from"* and then *"is trace disclosure, not the primary output"* — this plan takes the second reading, which is where the sentence lands and which keeps regions off the row shape, consistent with the ⟨3⟩ redesign. A caller wanting per-resource attribution is a larger, separate ask.
- **`query_cos` is still not disclosed anywhere, and this is a second gap of the same family.** `survey`'s `orders_by` says *"Resources within a matched region are ranked by the resource's own embedding similarity to the query (`query_cos` at a finer grain)"* — but the projection takes `region_score AS quantity`, so every resource in one region has an identical quantity and the within-region signal reaches no caller. Found while grounding this plan; **not fixed here**, because it is a different disclosure with a different argument, and folding it in would make this task's premise ("no SQL change, one column") false. File it separately if it matters.
- **The `sal_norm` blend ruling** stays open. This carries the number; it settles nothing.
