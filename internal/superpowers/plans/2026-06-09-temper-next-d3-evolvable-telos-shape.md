# temper-next D3: evolvable telos shape — block-role property + generic resource-block reads

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the telos-charter's block kinds (statement / question / framing) a `kb_properties` `block_role` property rather than a positional convention, and demote the per-cogmap reads to generic property-filtered resource-block reads — so framing never leaks into the questions projection (code-review finding #1 from D2).

**Architecture:** Block role is an open string property (`property_key='block_role'`, `owner_table='kb_content_blocks'`), stamped by the shared `_persist_resource_blocks` path from a `role` field on the block JSONB. Reads go through a new generic `resource_blocks(resource, principal, p_role)` (access-gated, role-joined, provenance-aggregated) + the existing `resource_body_text`; `cogmap_questions`/`cogmap_charter` retire in favor of a tiny `cogmap_telos` resolver. Roles are derived structurally in Rust (`TelosDef::block_specs`) from which charter field the prose came from — the YAML authoring shape is unchanged.

**Tech Stack:** Rust (temper-next crate), PostgreSQL artifact (`schema-artifact/*.sql`, `temper_next` namespace), sqlx, bge-768 embeddings (ONNX, `artifact-tests` feature), serde_yaml.

**Design source (read before implementing):** `docs/superpowers/specs/2026-06-09-temper-next-d3-evolvable-telos-shape-design.md`. Carry these invariants verbatim:
- *"blocks are addressable but not findable — `block` is a reference/provenance kind only, never a graph-edge target."*
- *"a `kb_resources` record is the relationship-vertex because it is the atom that is self-sufficient for its content; a content-block is not self-sufficient."*
- The block level captures **attribution** (provenance), **not** trajectory; fold is soft-delete/visibility, **not** decay.

**Grounding (per implementation-grounding.md):** Each task is tagged CONFORM / EXTEND / AMEND. Anything the plan does not cite by `file:line`, verify on disk (or run psql) before using — do not trust because it "sounds right." If a step cannot be grounded and is not a sanctioned EXTEND/AMEND, STOP and report BLOCKED.

---

## File map

- `crates/temper-next/src/scenario/model.rs` — `TelosDef::block_proses` → `block_specs` (role-bearing). Modify.
- `crates/temper-next/src/content.rs` — `PreparedBlock.role`; `prepare_block`/`prepare_blocks` take role. Modify.
- `crates/temper-next/src/scenario/loader.rs` — thread `(role, prose)` specs through both charter + ordinary-resource paths. Modify.
- `crates/temper-next/tests/cogmap_genesis_charter.rs` — update block construction to the new API. Modify.
- `crates/temper-next/tests/charter_block_roles.rs` — NEW: framing-leak-closed acceptance. Create.
- `schema-artifact/01_schema.sql` — `kb_properties.owner_table` CHECK += `'kb_content_blocks'`. Modify (`:399`).
- `schema-artifact/02_functions.sql` — role stamping in `_persist_resource_blocks`; new `resource_blocks` + `cogmap_telos`; retire `cogmap_questions` + `cogmap_charter`. Modify.
- `schema-artifact/03_seed.sql` — add `role` to the legacy charter JSONB so the demo read still shows rows. Modify (`~:190-208`).
- `schema-artifact/04_scenarios.sql` — rewrite the `cogmap_charter`/`cogmap_questions` demo call sites. Modify (`:61-79`).

`cogmap_regulation` (`02_functions.sql:297`) is **out of scope — do not touch**. `04b_region_suite.sql` does not reference the retired functions (verified) — leave it.

---

## Task 1: Rust — role-bearing charter blocks (model + content + loader)

**Tag:** EXTEND (spec D3 "evolvable content-blocks, designed"; builds on `model.rs:68-85`, `content.rs:28-83`, `loader.rs:64-92`).

**Files:**
- Modify: `crates/temper-next/src/scenario/model.rs:68-85` (and its unit test `:344-371`)
- Modify: `crates/temper-next/src/content.rs:30-34, 46-83` (and its unit test `:123-141`)
- Modify: `crates/temper-next/src/scenario/loader.rs:68-70, 92`
- Modify: `crates/temper-next/tests/cogmap_genesis_charter.rs:64-67`

- [ ] **Step 1: Replace the model unit test with a role-asserting one**

In `crates/temper-next/src/scenario/model.rs`, replace the `telos_block_proses_orders_statement_questions_framing` test (`:344-371`) with:

```rust
    #[test]
    fn telos_block_specs_tags_statement_questions_framing() {
        let telos = TelosDef {
            title: "T".into(),
            statement: "The statement.".into(),
            questions: vec![
                QuestionDef { question: "Q1?".into(), context: "C1.".into() },
                QuestionDef { question: "Q2?".into(), context: String::new() },
            ],
            framing: vec!["Framing one.".into()],
        };
        let specs = telos.block_specs();
        assert_eq!(
            specs,
            vec![
                ("statement", "The statement.".to_string()),
                ("question", "Q1?\n\nC1.".to_string()), // question + context joined
                ("question", "Q2?".to_string()),        // empty context ⇒ bare question
                ("framing", "Framing one.".to_string()),
            ]
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p temper-next telos_block_specs_tags_statement_questions_framing`
Expected: FAIL to compile — `no method named block_specs`.

- [ ] **Step 3: Replace `block_proses` with `block_specs`**

In `crates/temper-next/src/scenario/model.rs`, replace the `block_proses` method (`:68-85`) with:

```rust
    /// The charter flattened to its ordered `(role, prose)` block specs for
    /// `content::prepare_blocks`: block-0 is the statement (role `"statement"`), then each question
    /// (role `"question"`, `question + "\n\n" + context`, or just the question when context is empty),
    /// then the framing blocks (role `"framing"`). Positional by index — `seq` is assigned downstream;
    /// `role` is the `block_role` property the persist path stamps so reads distinguish the kinds.
    pub fn block_specs(&self) -> Vec<(&'static str, String)> {
        let mut specs = Vec::with_capacity(1 + self.questions.len() + self.framing.len());
        specs.push(("statement", self.statement.clone()));
        for q in &self.questions {
            let prose = if q.context.is_empty() {
                q.question.clone()
            } else {
                format!("{}\n\n{}", q.question, q.context)
            };
            specs.push(("question", prose));
        }
        for f in &self.framing {
            specs.push(("framing", f.clone()));
        }
        specs
    }
```

Also update the `TelosDef` doc comment (`:53-56`): change "Distinguished by `seq` positionally — no `block_kind` column" to "Each block's kind is stamped as a `block_role` property (`statement`/`question`/`framing`) by the persist path — see `block_specs`."

- [ ] **Step 4: Run the model test to verify it passes**

Run: `cargo nextest run -p temper-next telos_block_specs_tags_statement_questions_framing`
Expected: PASS.

- [ ] **Step 5: Add `role` to `PreparedBlock` and thread it through `prepare_block`/`prepare_blocks`**

In `crates/temper-next/src/content.rs`, change `PreparedBlock` (`:30-34`):

```rust
/// One content-block (seq-ordered within its resource) and its ordered chunks. Blocks carry **no**
/// prose of their own (content-block-primitive β) — text lives only in the chunks. `role` is the
/// block's `block_role` (`"statement"`/`"question"`/`"framing"` for a charter; `None` for an ordinary
/// resource body); when present the persist path stamps it as a `block_role` property. Serialized as
/// `null` when `None`.
#[derive(Debug, Clone, Serialize)]
pub struct PreparedBlock {
    pub seq: i32,
    pub role: Option<String>,
    pub chunks: Vec<PreparedChunk>,
}
```

Change `prepare_block` (`:46-72`) signature + return:

```rust
/// Prepare one block: chunk its prose, then embed every chunk in a single batched ONNX call.
pub fn prepare_block(seq: i32, role: Option<&str>, prose: &str) -> Result<PreparedBlock> {
```

and the returned struct literal (`:71`):

```rust
    Ok(PreparedBlock { seq, role: role.map(str::to_owned), chunks })
```

Change `prepare_blocks` (`:74-83`):

```rust
/// Prepare an ordered run of blocks (`seq` = position). Each spec is `(role, prose)`: the charter
/// passes `[(Some("statement"), …), (Some("question"), …), …, (Some("framing"), …)]`; an ordinary
/// resource passes its single body as one roleless block `[(None, body)]`. A block whose prose exceeds
/// one 510-token window yields >1 chunk — real multi-chunk-per-block.
pub fn prepare_blocks(specs: &[(Option<&str>, &str)]) -> Result<Vec<PreparedBlock>> {
    specs
        .iter()
        .enumerate()
        .map(|(i, (role, prose))| prepare_block(i as i32, *role, prose))
        .collect()
}
```

- [ ] **Step 6: Update the content serialization unit test for `role`**

In `crates/temper-next/src/content.rs`, in `prepared_block_serializes_to_expected_jsonb_shape` (`:124-141`), add `role` to the literal and assert it:

```rust
        let block = PreparedBlock {
            seq: 2,
            role: Some("question".into()),
            chunks: vec![PreparedChunk {
                chunk_index: 0,
                content_hash: "ab".repeat(32),
                content: "hi".into(),
                embedding: vec![0.1, 0.2, 0.3],
            }],
        };
        let v = serde_json::to_value([&block]).unwrap();
        assert_eq!(v[0]["seq"], 2);
        assert_eq!(v[0]["role"], "question");
        assert_eq!(v[0]["chunks"][0]["chunk_index"], 0);
```

- [ ] **Step 7: Thread roles through the loader (both paths)**

In `crates/temper-next/src/scenario/loader.rs`, replace the charter prep (`:68-70`):

```rust
    let charter_specs = s.cogmap.telos.block_specs();
    let charter_refs: Vec<(Option<&str>, &str)> = charter_specs
        .iter()
        .map(|(role, prose)| (Some(*role), prose.as_str()))
        .collect();
    let charter_blocks = crate::content::prepare_blocks(&charter_refs)?;
```

and the ordinary-resource prep (`:92`):

```rust
        let blocks = crate::content::prepare_blocks(&[(None, r.body.as_str())])?;
```

- [ ] **Step 8: Update the artifact test's block construction to the new API**

In `crates/temper-next/tests/cogmap_genesis_charter.rs` (`:64-67`), replace:

```rust
    // Rust-side: flatten → chunk → embed, exactly as the loader does.
    let specs = telos.block_specs();
    let refs: Vec<(Option<&str>, &str)> =
        specs.iter().map(|(role, prose)| (Some(*role), prose.as_str())).collect();
    let blocks = content::prepare_blocks(&refs).unwrap();
```

(Leave the rest of that test unchanged — the SQL ignores the new `role` JSONB key until Task 3, so its existing assertions stay green.)

- [ ] **Step 9: Verify the crate compiles (incl. the artifact-tests target) and pure tests pass**

Run: `cargo nextest run -p temper-next` then `cargo check -p temper-next --features artifact-tests`
Expected: tests PASS; both compile clean.

- [ ] **Step 10: `cargo make check`, then commit**

```bash
cargo make check
git add crates/temper-next/src/scenario/model.rs crates/temper-next/src/content.rs \
        crates/temper-next/src/scenario/loader.rs crates/temper-next/tests/cogmap_genesis_charter.rs
git commit -m "temper-next D3: role-bearing charter blocks (block_specs + PreparedBlock.role)"
```

---

## Task 2: Write the failing framing-leak-closed acceptance test

**Tag:** EXTEND (the regression gate the spec §7 mandates). This test fails until Task 3 ships the SQL.

**Files:**
- Create: `crates/temper-next/tests/charter_block_roles.rs`

- [ ] **Step 1: Write the test**

Create `crates/temper-next/tests/charter_block_roles.rs`:

```rust
#![cfg(feature = "artifact-tests")]
//! Deliverable-3 acceptance: charter blocks carry a `block_role` property, and the generic
//! `resource_blocks` read filters by role — so framing never leaks into the questions projection
//! (code-review finding #1 from D2). Resets the artifact, ONNX-dependent, serialized via the
//! temper-next-write group.
mod common;

use temper_next::scenario::bootseed;
use temper_next::scenario::model::{QuestionDef, TelosDef};
use temper_next::{content, substrate};
use uuid::Uuid;

async fn seed_actor(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name, system_access) \
         VALUES ('owner', 'Owner', 'approved'::system_access) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, 'agent#1', '{}'::jsonb) RETURNING id",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .unwrap();
    (profile, entity)
}

#[tokio::test]
async fn framing_never_projects_as_a_question() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = seed_actor(&pool).await;

    // statement + 2 questions + 1 framing block. The framing prose carries a marker string we assert
    // never appears in the questions projection.
    let telos = TelosDef {
        title: "Onboarding charter".into(),
        statement: "Help a new EPD engineer reach first-merge confidence in week one.".into(),
        questions: vec![
            QuestionDef { question: "What transfers?".into(), context: String::new() },
            QuestionDef { question: "Smallest real change?".into(), context: String::new() },
        ],
        framing: vec!["This map coordinates with the schema-migration initiative.".into()],
    };
    let specs = telos.block_specs();
    let refs: Vec<(Option<&str>, &str)> =
        specs.iter().map(|(role, prose)| (Some(*role), prose.as_str())).collect();
    let blocks = content::prepare_blocks(&refs).unwrap();
    let charter_json = serde_json::to_value(&blocks).unwrap();

    let (cogmap, telos_resource): (Uuid, Uuid) =
        sqlx::query_as("SELECT cogmap_id, telos_resource_id FROM cogmap_genesis($1,$2,$3,$4,$5)")
            .bind("onboarding-cogmap")
            .bind("Onboarding charter")
            .bind(charter_json)
            .bind(owner)
            .bind(emitter)
            .fetch_one(&pool)
            .await
            .unwrap();

    // every block carries a block_role property, in seq order: statement, question, question, framing
    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT p.property_value #>> '{}' FROM kb_properties p \
         JOIN kb_content_blocks b ON b.id = p.owner_id \
         WHERE p.owner_table='kb_content_blocks' AND p.property_key='block_role' \
           AND b.resource_id = $1 ORDER BY b.seq",
    )
    .bind(telos_resource)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(roles, vec!["statement", "question", "question", "framing"]);

    // the questions projection returns exactly the two questions and NEVER the framing block
    // (principal = the cogmap itself; map-home-confers makes the telos readable, no team wiring).
    let q_rows: Vec<String> = sqlx::query_scalar(
        "SELECT body_text FROM resource_blocks($1, 'cogmap', $2, 'question') ORDER BY seq",
    )
    .bind(telos_resource)
    .bind(cogmap)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(q_rows.len(), 2, "exactly the two questions");
    assert!(
        q_rows.iter().all(|t| !t.contains("schema-migration initiative")),
        "framing prose must not leak into the questions projection, got {q_rows:?}"
    );

    // the framing projection returns exactly the framing block
    let f_rows: Vec<String> = sqlx::query_scalar(
        "SELECT body_text FROM resource_blocks($1, 'cogmap', $2, 'framing') ORDER BY seq",
    )
    .bind(telos_resource)
    .bind(cogmap)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(f_rows.len(), 1);
    assert!(f_rows[0].contains("schema-migration initiative"));

    // unfiltered returns all four blocks
    let all_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM resource_blocks($1, 'cogmap', $2, NULL)")
            .bind(telos_resource)
            .bind(cogmap)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(all_rows, 4);
}
```

- [ ] **Step 2: Run it to verify it fails for the right reason**

Run: `cargo nextest run -p temper-next --features artifact-tests framing_never_projects_as_a_question`
Expected: FAIL — Postgres error `function resource_blocks(...) does not exist` (the SQL is not in place yet). If it fails to *compile* instead, fix the test; if it fails because `block_role` rows are absent, that is also expected pre-Task-3.

- [ ] **Step 3: Commit the failing test**

```bash
git add crates/temper-next/tests/charter_block_roles.rs
git commit -m "temper-next D3: failing test — framing must never project as a question"
```

---

## Task 3: SQL artifact — owner-table AMEND, role stamping, generic reads, retire cogmap_questions/charter

**Tag:** AMEND (`01_schema.sql:399` CHECK; retire `02_functions.sql:266-293`) + EXTEND (new functions; `_persist_resource_blocks:459-496`). Authorized by spec §3.1 (owner-table already widened for `kb_edges`), §4 (read-layer full demotion).

**Files:**
- Modify: `schema-artifact/01_schema.sql:399`
- Modify: `schema-artifact/02_functions.sql` (`_persist_resource_blocks`; new functions; drop two)

- [ ] **Step 1: Extend the `owner_table` CHECK**

In `schema-artifact/01_schema.sql:399`, change:

```sql
    owner_table           VARCHAR(64) NOT NULL CHECK (owner_table IN ('kb_resources', 'kb_cogmaps', 'kb_edges')),  -- §4a: edges carry facets
```

to:

```sql
    owner_table           VARCHAR(64) NOT NULL CHECK (owner_table IN ('kb_resources', 'kb_cogmaps', 'kb_edges', 'kb_content_blocks')),  -- §4a edges carry facets; D3 blocks carry block_role
```

- [ ] **Step 2: Stamp the block role in `_persist_resource_blocks`**

In `schema-artifact/02_functions.sql`, in `_persist_resource_blocks`, immediately after the block INSERT (`:461-462`, the `RETURNING id INTO v_block;` line), add:

```sql
        -- D3: stamp the block's role (statement/question/framing) as a block_role property when the
        -- block JSONB carries one. Open string value (no enum); single-label per block. The pair
        -- (owner_table='kb_content_blocks', property_key='block_role') double-segregates it from the
        -- resource-facet lens math (which filters owner_table='kb_resources' AND property_key='facet').
        IF v_block_json ? 'role' AND jsonb_typeof(v_block_json->'role') = 'string' THEN
            INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value,
                                       asserted_by_event_id, last_event_id)
            VALUES ('kb_content_blocks', v_block, 'block_role', v_block_json->'role', p_event, p_event);
        END IF;
```

- [ ] **Step 3: Retire `cogmap_charter` + `cogmap_questions`; add `resource_blocks` + `cogmap_telos`**

In `schema-artifact/02_functions.sql`, delete the entire `CREATE FUNCTION cogmap_charter (...)` block (`:266-273`) and the entire `CREATE FUNCTION cogmap_questions (...)` block (`:279-293`), **including their leading comments**. Leave `cogmap_regulation` (`:297`) untouched. In their place, insert:

```sql
-- Generic per-resource block projection (D3): a resource's non-folded blocks with assembled body
-- text, their block_role, and the provenance-attribution signal. Access-gated via resources_readable_by.
-- p_role NULL ⇒ all blocks; otherwise only blocks whose block_role property equals p_role. The
-- questions / framing / statement reads are all THIS function with a role filter — "kind" is not a
-- per-resource-type concept, but a property-filtered block read is universal (design §2, §4).
-- reinforce_count is a provenance-ATTRIBUTION accretion count (a reinforcement proxy), not a modeled
-- block-level trajectory (design §5).
CREATE FUNCTION resource_blocks(
    p_resource uuid, p_principal_kind text, p_principal_id uuid, p_role text DEFAULT NULL
) RETURNS TABLE(seq int, block_id uuid, body_text text, role text,
                reinforce_count bigint, last_reinforced_at timestamptz) LANGUAGE sql STABLE AS $$
    SELECT b.seq, b.id, block_body_text(b.id),
           rp.property_value #>> '{}',
           count(pr.id) FILTER (WHERE NOT pr.is_corrected),
           max(pr.created) FILTER (WHERE NOT pr.is_corrected)
    FROM kb_content_blocks b
    LEFT JOIN kb_properties rp
           ON rp.owner_table = 'kb_content_blocks' AND rp.owner_id = b.id
          AND rp.property_key = 'block_role' AND NOT rp.is_folded
    LEFT JOIN kb_block_provenance pr ON pr.block_id = b.id
    WHERE b.resource_id = p_resource AND NOT b.is_folded
      AND p_resource IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
      AND (p_role IS NULL OR rp.property_value #>> '{}' = p_role)
    GROUP BY b.seq, b.id, rp.property_value
    ORDER BY b.seq;
$$;

-- The one genuinely cogmap-specific read (D3): resolve a cogmap to its telos-charter resource id (the
-- kb_cogmaps.telos_resource_id FK). Everything else is generic resource-level — resource_body_text for
-- the charter body, resource_blocks(telos, …, p_role) for questions/framing. Retires
-- cogmap_charter/cogmap_questions. (cogmap_regulation is a graph-edge read — left untouched; it may be
-- demoted when the regulation/edge-semantics deliverable lands.)
CREATE FUNCTION cogmap_telos(p_cogmap uuid)
RETURNS uuid LANGUAGE sql STABLE AS $$
    SELECT telos_resource_id FROM kb_cogmaps WHERE id = p_cogmap;
$$;
```

- [ ] **Step 4: Confirm the artifact loads (executable grounding)**

Run:
```bash
psql "$DATABASE_URL" -q -v ON_ERROR_STOP=1 \
  -f schema-artifact/01_schema.sql -f schema-artifact/02_functions.sql \
  -c "SELECT proname FROM pg_proc WHERE proname IN ('resource_blocks','cogmap_telos','cogmap_charter','cogmap_questions') ORDER BY proname;"
```
Expected: clean load; the SELECT lists exactly `cogmap_telos` and `resource_blocks` (the two retired functions are gone).

- [ ] **Step 5: Run the acceptance test — it should now pass**

Run: `cargo nextest run -p temper-next --features artifact-tests framing_never_projects_as_a_question`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add schema-artifact/01_schema.sql schema-artifact/02_functions.sql
git commit -m "temper-next D3: block_role property + generic resource_blocks/cogmap_telos; retire cogmap_questions/charter"
```

---

## Task 4: Legacy ripple, sqlx cache, and the full regression gate

**Tag:** AMEND (`04_scenarios.sql` call sites reference dropped functions) + CONFORM (`03_seed.sql` charter gains roles to match the new persist contract; `.sqlx` ritual).

**Files:**
- Modify: `schema-artifact/04_scenarios.sql:61-79`
- Modify: `schema-artifact/03_seed.sql:~190-208`

- [ ] **Step 1: Add `role` to the legacy charter JSONB in `03_seed.sql`**

The legacy seed builds its charter JSONB programmatically with an `ord` ordinal (`schema-artifact/03_seed.sql:~190-200`). In the `jsonb_build_object(...)` that assembles each charter block, add a `role` key so the legacy-seeded charter also stamps `block_role` (statement for the block-0 telos statement, question for the rest):

```sql
                'seq', ord,
                'role', CASE WHEN ord = 0 THEN 'statement' ELSE 'question' END,
                'chunks', ...   -- (leave the existing chunks expression unchanged)
```

Read the surrounding block first to place the key correctly inside the existing object literal — do not guess the column list.

- [ ] **Step 2: Rewrite the `04_scenarios.sql` demo call sites**

In `schema-artifact/04_scenarios.sql:61-79`, replace the `cogmap_charter` and `cogmap_questions` calls (the `cogmap_regulation` call between them stays). The charter body becomes `resource_body_text(cogmap_telos(...))`; questions become `resource_blocks(cogmap_telos(...), 'cogmap', <cogmap id>, 'question')`; the nomad gating demo becomes a `resource_blocks(..., 'profile', nomad, NULL)` count:

```sql
\echo '-- charter body (onboarding) via the generic resource read:'
SELECT r.title, resource_body_text(r.id) AS body_text
FROM kb_resources r
WHERE r.id = cogmap_telos((SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap'));

\echo '-- guiding questions (onboarding): role=question blocks, with the provenance-attribution signal:'
SELECT seq, body_text, reinforce_count
FROM resource_blocks(
        cogmap_telos((SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap')),
        'cogmap', (SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap'), 'question')
ORDER BY seq;
```

and the nomad-gating block (`:77-79`):

```sql
\echo '-- gating: nomad (cannot read the map) gets ZERO charter blocks:'
SELECT count(*) AS nomad_charter_rows
FROM resource_blocks(
        cogmap_telos((SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap')),
        'profile', (SELECT id FROM kb_profiles WHERE handle='nomad'), NULL);
```

- [ ] **Step 3: Confirm the legacy chain loads end-to-end**

Run:
```bash
psql "$DATABASE_URL" -q -v ON_ERROR_STOP=1 \
  -f schema-artifact/01_schema.sql -f schema-artifact/02_functions.sql \
  -f schema-artifact/03_seed.sql -f schema-artifact/04_scenarios.sql >/dev/null && echo "legacy chain OK"
```
Expected: `legacy chain OK` (no dangling reference to the retired functions; the questions read returns the seeded question rows; nomad count is 0).

- [ ] **Step 4: Regenerate the temper_next sqlx cache and confirm no surprise drift**

The retired/added functions are not referenced by any non-test `query!` macro (the new functions are exercised only by the runtime-`query_as` acceptance test and by SQL), and `cogmap_genesis`/`resource_create` signatures are unchanged — so the cache should not change. Regenerate to be certain (per CLAUDE.md, per-crate, never `--workspace`):

```bash
cargo make prepare-next
git status --porcelain crates/temper-next/.sqlx
```
Expected: no output from `git status` (cache unchanged). If the cache *did* change, inspect the diff — an unexpected change means a macro you didn't account for; reconcile before committing.

- [ ] **Step 5: Run the full write-path artifact gate**

Run: `cargo nextest run -p temper-next --features artifact-tests`
Expected: PASS — `bootseed`, `scenario_load`, `scenario_roundtrip` (incl. the cross-path membership proof), `cogmap_genesis_charter`, and `charter_block_roles` all green. (Per the empty-bg-log note, trust the exit code / grep for `FAIL [`, not a per-binary summary line.)

- [ ] **Step 6: `cargo make check`, then commit**

```bash
cargo make check
git add schema-artifact/03_seed.sql schema-artifact/04_scenarios.sql crates/temper-next/.sqlx
git commit -m "temper-next D3: legacy charter roles + 04_scenarios generic reads; regen temper_next sqlx cache"
```

---

## Self-Review

**Spec coverage** (against `2026-06-09-temper-next-d3-evolvable-telos-shape-design.md` §7.1):
- owner_table CHECK AMEND → Task 3 Step 1. ✓
- block JSONB carries role + `_persist_resource_blocks` stamps `block_role` → Task 1 (Rust role) + Task 3 Step 2 (SQL stamp). ✓
- generic `resource_blocks` + `cogmap_telos` → Task 3 Step 3. ✓
- retire `cogmap_questions` + `cogmap_charter`; rewrite `04_scenarios.sql` → Task 3 Step 3 + Task 4 Step 2. ✓
- Rust `TelosDef`/`content.rs`/`loader.rs` carry role → Task 1. ✓
- scenario/JsonSchema: **no change needed** — role is derived structurally from the existing `statement`/`questions`/`framing` fields, so the YAML authoring shape and `scenario.schema.json` are unchanged (the spec's "scenario authors a framing block" is realized as the new test's framing block + the unchanged `framing:` field). No JsonSchema-snapshot task is required; if `cargo make check` surfaces a snapshot diff, regenerate it then. ✓ (deviation from §7.1's table row, justified here)
- regenerate `.sqlx` → Task 4 Step 4. ✓
- regression gate (scenario_roundtrip + cross-path proof + framing-exclusion assertion) → Task 2 (test) + Task 4 Step 5. ✓
- `cogmap_regulation` untouched → stated in File map + Task 3 Step 3. ✓

**Out of scope (not in this plan, per design §8):** regulation-lesson half of scar; relational framing neighborhood edges; weighted multi-role; general `p_key/p_value` addressing; public doc/UI name-sweep (separate follow-up task).

**Type consistency:** `block_specs() -> Vec<(&'static str, String)>` (Task 1) ↔ loader maps to `(Some(*role), prose.as_str())` ↔ `prepare_blocks(&[(Option<&str>, &str)])` ↔ `PreparedBlock.role: Option<String>` ↔ JSONB `role` string ↔ SQL `v_block_json->'role'` ↔ `block_role` property ↔ `resource_blocks` `p_role text` / returned `role text`. Consistent end to end.

**Placeholder scan:** none — every code/SQL step shows the literal content; every command has an expected result.
