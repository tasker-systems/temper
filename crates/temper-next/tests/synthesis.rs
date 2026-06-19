#![cfg(feature = "artifact-tests")]
//! Synthesis-from-state, resource pass (WS6 §8/§2/§1c): `synthesis::run` over the prod-shape fixture
//! regenerates one `temper_next.kb_resources` row per ACTIVE production resource, each homed in its
//! remapped context (`('kb_contexts', ctx)`) carrying its current originator/owner, with a single
//! up-front content block (seq 0) whose chunks carry the production chunk-set verbatim — content,
//! sha256 content_hash, header_path/heading_depth, and a non-NULL bge-768 embedding.
//!
//! Runs on its own ephemeral DB via `#[sqlx::test(migrator = ...)]` (the full migration chain incl. the
//! additive `temper_next` install is applied; `public` is migrated-but-empty). The prod-shape fixture
//! seeds `public.*` only; synthesis writes `temper_next.*`. NOT in the write-path nextest group.

mod common;

use common::fixture_ids;
use sqlx::Row;
use uuid::Uuid;

#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn synthesizes_resources_homes_and_single_block(pool: sqlx::PgPool) {
    common::seed_prod_shape_fixture(&pool).await;

    let report = temper_next::synthesis::run(&pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect("synthesis::run");
    assert_eq!(
        report.resources, 4,
        "4 active resources synthesized (R4 soft-deleted excluded, §0 active-only)"
    );

    // One kb_resources row per active resource (4); titles carried.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM temper_next.kb_resources")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 4, "one kb_resources row per active resource");

    // Exactly one content block (seq 0) per synthesized resource (§8 single block).
    let blocks: i64 = sqlx::query_scalar("SELECT count(*) FROM temper_next.kb_content_blocks")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(blocks, 4, "exactly one content block per resource");
    let max_seq: i32 =
        sqlx::query_scalar("SELECT coalesce(max(seq), -1) FROM temper_next.kb_content_blocks")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(max_seq, 0, "the single block is at seq 0");

    // R2 (task): title carried, homed at its REMAPPED context, distinct originator≠owner preserved
    // (the §2 "carrying its current originator/owner" — proves the COALESCE in _project_resource_created).
    let row = sqlx::query(
        "SELECT r.title, h.anchor_table, h.anchor_id, \
                ow.handle AS owner_handle, orig.handle AS originator_handle \
         FROM temper_next.kb_resources r \
         JOIN temper_next.kb_resource_homes h ON h.resource_id = r.id \
         JOIN temper_next.kb_profiles ow ON ow.id = h.owner_profile_id \
         JOIN temper_next.kb_profiles orig ON orig.id = h.originator_profile_id \
         WHERE r.origin_uri = 'temper://fixture/task-doc'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("title"), "Task Doc");
    assert_eq!(row.get::<String, _>("anchor_table"), "kb_contexts");
    assert_eq!(
        row.get::<String, _>("owner_handle"),
        "fixture-owner",
        "home owner carried verbatim"
    );
    assert_eq!(
        row.get::<String, _>("originator_handle"),
        "fixture-originator",
        "home originator carried distinct from owner (COALESCE, not collapsed)"
    );
    // The anchor is the synthesized temper_next context, whose id is the production id PRESERVED
    // verbatim (id-continuity: contexts carry their prod id through, same as profiles/resources).
    let anchor: Uuid = row.get("anchor_id");
    assert_eq!(
        anchor,
        fixture_ids::CONTEXT_ONE,
        "home anchors the synthesized context, which preserves the production context id verbatim"
    );
    let anchor_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM temper_next.kb_contexts WHERE id = $1)")
            .bind(anchor)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        anchor_exists,
        "home anchor resolves to a synthesized context"
    );

    // R2's chunk-set carried verbatim: chunk 1 carries the heading metadata (header_path/heading_depth);
    // chunk 0 carries the empty heading. content_hash + content + embedding all verbatim.
    let headed = sqlx::query(
        "SELECT c.content_hash, c.header_path, c.heading_depth, cc.content, \
                (c.embedding IS NOT NULL) AS has_emb \
         FROM temper_next.kb_chunks c \
         JOIN temper_next.kb_chunk_content cc ON cc.chunk_id = c.id \
         JOIN temper_next.kb_resources r ON r.id = c.resource_id \
         WHERE r.origin_uri = 'temper://fixture/task-doc' AND c.chunk_index = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(headed.get::<String, _>("content_hash"), "hash-r2-c1");
    assert_eq!(headed.get::<String, _>("header_path"), "Intro > Goals");
    assert_eq!(headed.get::<i16, _>("heading_depth"), 2);
    assert_eq!(
        headed.get::<String, _>("content"),
        "Task goals section body."
    );
    assert!(
        headed.get::<bool, _>("has_emb"),
        "bge-768 embedding carried non-NULL (§8 carry-as-is)"
    );

    let c0 = sqlx::query(
        "SELECT c.content_hash, c.header_path, c.heading_depth, cc.content \
         FROM temper_next.kb_chunks c \
         JOIN temper_next.kb_chunk_content cc ON cc.chunk_id = c.id \
         JOIN temper_next.kb_resources r ON r.id = c.resource_id \
         WHERE r.origin_uri = 'temper://fixture/task-doc' AND c.chunk_index = 0",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(c0.get::<String, _>("content_hash"), "hash-r2-c0");
    assert_eq!(c0.get::<String, _>("header_path"), "");
    assert_eq!(c0.get::<i16, _>("heading_depth"), 0);
    assert_eq!(c0.get::<String, _>("content"), "Task intro paragraph.");

    // R5 homes in the team-owned context (anchor resolves to a synthesized context) — proves the
    // resource pass remaps every referenced context, not just the profile-owned ones.
    let r5_anchor_ok: bool = sqlx::query_scalar(
        "SELECT EXISTS( \
           SELECT 1 FROM temper_next.kb_resources r \
           JOIN temper_next.kb_resource_homes h ON h.resource_id = r.id \
           JOIN temper_next.kb_contexts ctx ON ctx.id = h.anchor_id \
           WHERE r.origin_uri = 'temper://fixture/team-doc' \
             AND h.anchor_table = 'kb_contexts' AND ctx.owner_table = 'kb_teams')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        r5_anchor_ok,
        "the team-context-homed resource anchors at a synthesized team-owned context"
    );
}

/// Property pass (WS6 §7 manifest-key fate table): after `synthesis::run`, each surviving manifest key
/// of the task fixture resource (R2) is reconciled per the §7 fates — workflow/provenance managed keys
/// and every `open_meta` key become `kb_properties` rows verbatim, while title/slug/id/context die,
/// `temper-goal` is held back for the edge pass, and `temper-type` reconciles to the `doc_type` column.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn synthesizes_properties_from_manifest_keys(pool: sqlx::PgPool) {
    common::seed_prod_shape_fixture(&pool).await;

    temper_next::synthesis::run(&pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect("synthesis::run");

    // Every kb_properties row owned by R2 (the task), keyed by property_key with its value as text
    // (`#>> '{}'` extracts the JSON scalar — a managed value like "doing" reads back as `doing`).
    let rows = sqlx::query(
        "SELECT p.property_key, p.property_value #>> '{}' AS value_text \
         FROM temper_next.kb_properties p \
         JOIN temper_next.kb_resources r ON r.id = p.owner_id \
         WHERE p.owner_table = 'kb_resources' AND r.origin_uri = 'temper://fixture/task-doc'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let mut props: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for row in &rows {
        props.insert(row.get("property_key"), row.get("value_text"));
    }

    // Workflow managed keys → kb_properties rows, values verbatim (§7 Property fate).
    assert_eq!(
        props.get("temper-stage").map(String::as_str),
        Some("doing"),
        "temper-stage became a property with its value verbatim"
    );
    assert_eq!(
        props.get("temper-mode").map(String::as_str),
        Some("build"),
        "temper-mode became a property with its value verbatim"
    );
    assert_eq!(
        props.get("temper-effort").map(String::as_str),
        Some("M"),
        "temper-effort became a property with its value verbatim"
    );

    // Every open_meta key → property verbatim (§7).
    assert_eq!(
        props.get("custom-key").map(String::as_str),
        Some("custom-value"),
        "open_meta key carried verbatim"
    );
    assert_eq!(
        props.get("another-key").map(String::as_str),
        Some("another-value"),
        "open_meta key carried verbatim"
    );

    // Identity/derived keys die — never a property (title is a column, slug render-time, id the row id,
    // context derives from the home row).
    for died in ["temper-title", "temper-slug", "temper-id", "temper-context"] {
        assert!(
            !props.contains_key(died),
            "{died} must die (§7), not become a property"
        );
    }

    // temper-goal is an EDGE (synthesized by the edge pass, Task 8), never a property.
    assert!(
        !props.contains_key("temper-goal"),
        "temper-goal is an edge, not a property (§7)"
    );

    // temper-type reconciles against the authoritative doctype column — the column wins, the stray
    // dies: no `temper-type` property, but the `doc_type` property (from the resource pass) carries.
    assert!(
        !props.contains_key("temper-type"),
        "temper-type reconciles to the doc_type column (§7); no property"
    );
    assert_eq!(
        props.get("doc_type").map(String::as_str),
        Some("task"),
        "doc_type property (from resource_create) carries the authoritative doctype"
    );
}

/// Regression — WS6 flip Neon-branch rehearsal finding: a manifest key carried in BOTH `managed_meta`
/// (Property-fated) and `open_meta` with the SAME value must synthesize as ONE property, not violate
/// `uq_kb_properties_active` (whose active grain is `owner + property_key + property_value`).
/// Production has this: four learning-maths resources carry a `date` key in both manifests with an
/// equal value. The property pass fires from both sources, so it must dedup identical `(key, value)`
/// pairs per resource — while still letting DISTINCT values for a repeated key both fire (multi-valued
/// keys are preserved). The shared fixture is left untouched; the duplicate is injected into R2 here.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn synthesizes_dedups_duplicate_property_across_managed_and_open(pool: sqlx::PgPool) {
    common::seed_prod_shape_fixture(&pool).await;

    // Carry the same (key, value) in BOTH manifest blobs of R2 (the task) — the production shape that
    // collides on `uq_kb_properties_active` when the property pass fires each source separately.
    sqlx::query(
        "UPDATE public.kb_resource_manifests \
         SET managed_meta = managed_meta || '{\"dup-key\": \"dup-value\"}'::jsonb, \
             open_meta    = open_meta    || '{\"dup-key\": \"dup-value\"}'::jsonb \
         WHERE resource_id = $1",
    )
    .bind(fixture_ids::RESOURCE_TASK)
    .execute(&pool)
    .await
    .unwrap();

    temper_next::synthesis::run(&pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect(
            "synthesis must dedup the duplicate (key,value), not error on uq_kb_properties_active",
        );

    // Exactly one active property row for the deduped pair.
    let dup_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM temper_next.kb_properties p \
         JOIN temper_next.kb_resources r ON r.id = p.owner_id \
         WHERE p.owner_table = 'kb_resources' AND r.origin_uri = 'temper://fixture/task-doc' \
           AND p.property_key = 'dup-key' AND NOT p.is_folded",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        dup_rows, 1,
        "a (key, value) carried in both managed_meta and open_meta synthesizes as one property"
    );
}

/// Regression — WS6 flip Neon-branch rehearsal finding: `origin_uri` is NOT a unique key in
/// production (CLI/agent-created resources carry an empty `origin_uri`; the real corpus has 166 of
/// 1214). The §8 body-parity gate and the §9 `readback::body` read must therefore key on the preserved
/// resource id, never `origin_uri` — otherwise every empty-`origin_uri` resource reads a concatenation
/// of all of them, the gate false-flags them, and synthesis refuses to complete. Here TWO active
/// resources are forced to share an empty `origin_uri`; synthesis must still pass the gate, and each
/// resource's body must read back as its own distinct content.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn synthesizes_with_nonunique_empty_origin_uri(pool: sqlx::PgPool) {
    common::seed_prod_shape_fixture(&pool).await;

    // R1 (goal) and R3 (decision) now collide on origin_uri='' — the production shape an
    // origin_uri-keyed parity gate / body read silently mishandles.
    sqlx::query("UPDATE public.kb_resources SET origin_uri = '' WHERE id = ANY($1)")
        .bind(vec![
            fixture_ids::RESOURCE_GOAL,
            fixture_ids::RESOURCE_DECISION,
        ])
        .execute(&pool)
        .await
        .unwrap();

    // The §8 gate (run inside synthesis::run) must still pass — matched by preserved id, not the
    // now-ambiguous origin_uri.
    temper_next::synthesis::run(&pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect("synthesis must pass the §8 gate keyed on resource id, not non-unique origin_uri");

    // The §9 body read returns each resource's OWN body, not a concatenation of the empty-origin_uri set.
    let goal_body = temper_next::readback::body(
        &pool,
        fixture_ids::OWNER_PROFILE,
        fixture_ids::RESOURCE_GOAL,
    )
    .await
    .expect("readback::body goal");
    let decision_body = temper_next::readback::body(
        &pool,
        fixture_ids::OWNER_PROFILE,
        fixture_ids::RESOURCE_DECISION,
    )
    .await
    .expect("readback::body decision");
    assert_eq!(
        goal_body, "Goal body text.",
        "empty-origin_uri goal reads its own body"
    );
    assert_eq!(
        decision_body, "Decision body text.",
        "empty-origin_uri decision reads its own body (distinct from the goal's)"
    );
}

/// Edge pass (WS6 §4) + minted temper-goal edges (§7/G8): after `synthesis::run`, every active-endpoint
/// `public.kb_resource_edges` row becomes a `temper_next.kb_edges` row with kind/polarity/label/weight
/// carried verbatim and endpoints remapped to the synthesized ids; the edge homes at its SOURCE
/// endpoint's context (§1c); the folded edge synthesizes as an assert+fold pair; and the task's
/// `temper-goal` mints exactly ONE `contains`/`parent_of` goal→task edge — NOT double-created, since
/// that edge is also materialized in `kb_resource_edges` (the dedup proof).
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn synthesizes_edges_and_minted_goal_edges(pool: sqlx::PgPool) {
    common::seed_prod_shape_fixture(&pool).await;

    let report = temper_next::synthesis::run(&pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect("synthesis::run");

    // 3 source edges synthesized; the minted temper-goal edge is deduped against the materialized
    // `contains` R1→R2 row, so `report.edges` counts only the 3 `relationship_asserted` fires.
    assert_eq!(report.edges, 3, "3 source edges; minted goal edge deduped");

    // Total kb_edges = source edges (3) + minted (1) − dedup (1) = 3.
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM temper_next.kb_edges")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 3, "kb_edges count = source edges + minted − dedup");

    // Each edge: endpoints remapped (joinable by origin_uri), both endpoints kb_resources, and the
    // edge homes at the SOURCE resource's home context (§1c — `home_anchor_id == source home anchor`).
    let rows = sqlx::query(
        "SELECT s.origin_uri AS src, t.origin_uri AS tgt, e.edge_kind::text AS kind, \
                e.polarity::text AS polarity, e.label, e.weight, e.is_folded, \
                e.source_table, e.target_table, \
                (e.home_anchor_table = 'kb_contexts' AND e.home_anchor_id = sh.anchor_id) AS home_ok \
         FROM temper_next.kb_edges e \
         JOIN temper_next.kb_resources s ON s.id = e.source_id \
         JOIN temper_next.kb_resources t ON t.id = e.target_id \
         JOIN temper_next.kb_resource_homes sh ON sh.resource_id = e.source_id \
         ORDER BY s.origin_uri, t.origin_uri",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let mut by_pair: std::collections::HashMap<(String, String), &sqlx::postgres::PgRow> =
        std::collections::HashMap::new();
    for r in &rows {
        let src: String = r.get("src");
        let tgt: String = r.get("tgt");
        assert_eq!(r.get::<String, _>("source_table"), "kb_resources");
        assert_eq!(r.get::<String, _>("target_table"), "kb_resources");
        assert!(
            r.get::<bool, _>("home_ok"),
            "edge {src}→{tgt} homes at its source's context (§1c)"
        );
        by_pair.insert((src, tgt), r);
    }

    // contains R1→R2 (parent_of, forward, weight 1.0, not folded) — also R2's materialized goal edge.
    let contains = by_pair
        .get(&(
            "temper://fixture/goal-doc".to_string(),
            "temper://fixture/task-doc".to_string(),
        ))
        .expect("contains R1→R2 edge");
    assert_eq!(contains.get::<String, _>("kind"), "contains");
    assert_eq!(contains.get::<String, _>("polarity"), "forward");
    assert_eq!(
        contains.get::<Option<String>, _>("label"),
        Some("parent_of".to_string())
    );
    assert_eq!(contains.get::<f64, _>("weight"), 1.0);
    assert!(!contains.get::<bool, _>("is_folded"));

    // near R2→R3 (forward, weight 0.5, empty production label → NULL, is_folded=true).
    let near = by_pair
        .get(&(
            "temper://fixture/task-doc".to_string(),
            "temper://fixture/decision-doc".to_string(),
        ))
        .expect("near R2→R3 edge");
    assert_eq!(near.get::<String, _>("kind"), "near");
    assert_eq!(
        near.get::<Option<String>, _>("label"),
        None,
        "empty production label carries as NULL, never an empty string"
    );
    assert!(
        near.get::<bool, _>("is_folded"),
        "the folded edge is is_folded=true"
    );

    // inverse leads_to R3→R1 (polarity carried verbatim, derived_from, weight 0.7).
    let inverse = by_pair
        .get(&(
            "temper://fixture/decision-doc".to_string(),
            "temper://fixture/goal-doc".to_string(),
        ))
        .expect("inverse leads_to R3→R1 edge");
    assert_eq!(inverse.get::<String, _>("kind"), "leads_to");
    assert_eq!(
        inverse.get::<String, _>("polarity"),
        "inverse",
        "inverse polarity carried verbatim (§4)"
    );
    assert_eq!(
        inverse.get::<Option<String>, _>("label"),
        Some("derived_from".to_string())
    );
    assert_eq!(inverse.get::<f64, _>("weight"), 0.7);

    // Exactly ONE contains/parent_of edge R1→R2 — the minted temper-goal edge was deduped, not added.
    let parent_of: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM temper_next.kb_edges e \
         JOIN temper_next.kb_resources s ON s.id = e.source_id \
         JOIN temper_next.kb_resources t ON t.id = e.target_id \
         WHERE e.edge_kind = 'contains' AND e.label = 'parent_of' \
           AND s.origin_uri = 'temper://fixture/goal-doc' \
           AND t.origin_uri = 'temper://fixture/task-doc'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        parent_of, 1,
        "temper-goal edge minted exactly once (deduped against the materialized kb_resource_edges row)"
    );

    // The §4 assert+fold pair: one relationship_asserted per synthesized edge, one relationship_folded
    // for the single folded edge.
    let asserted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM temper_next.kb_events e \
         JOIN temper_next.kb_event_types et ON et.id = e.event_type_id \
         WHERE et.name = 'relationship_asserted'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        asserted, 3,
        "one relationship_asserted per synthesized edge"
    );
    let folded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM temper_next.kb_events e \
         JOIN temper_next.kb_event_types et ON et.id = e.event_type_id \
         WHERE et.name = 'relationship_folded'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        folded, 1,
        "the one folded edge fired a relationship_folded event (the assert+fold pair)"
    );
}

/// Body-text parity gate (WS6 §8): after a full `synthesis::run`, reconstructing each synthesized
/// resource's body from `temper_next` chunks (the production `get_content` algorithm) reproduces the
/// production body byte-for-byte. `run` itself enforces this — it returns `Err` on any mismatch — so a
/// successful run already proves the gate; this asserts the report is non-vacuously clean (all 4 active
/// resources checked, zero mismatches), then proves the gate DETECTS divergence by corrupting one
/// synthesized chunk and re-running the report directly.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn body_parity_gate_passes_and_detects_corruption(pool: sqlx::PgPool) {
    common::seed_prod_shape_fixture(&pool).await;

    // A clean synthesis succeeds — and succeeding IS the parity assertion (run returns Err on mismatch).
    temper_next::synthesis::run(&pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect("synthesis::run (clean fixture → parity holds)");

    // The report is non-vacuously clean: it checked every active resource (4) and flagged none.
    let report = temper_next::synthesis::parity::body_parity_report(&pool)
        .await
        .expect("body_parity_report");
    assert_eq!(
        report.checked, 4,
        "parity gate checked all 4 active resources"
    );
    assert!(
        report.is_clean(),
        "every reconstructed body matches production: {:?}",
        report.mismatches
    );

    // Corrupt exactly one synthesized chunk (team-doc's single chunk) and re-run the report directly:
    // the gate must flag precisely that one resource — proving the pass above is not vacuous.
    let corrupted: u64 = sqlx::query(
        "UPDATE temper_next.kb_chunk_content SET content = 'CORRUPTED BODY' \
         WHERE chunk_id IN ( \
           SELECT c.id FROM temper_next.kb_chunks c \
           JOIN temper_next.kb_resources r ON r.id = c.resource_id \
           WHERE r.origin_uri = 'temper://fixture/team-doc')",
    )
    .execute(&pool)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(corrupted, 1, "team-doc has exactly one chunk to corrupt");

    let report = temper_next::synthesis::parity::body_parity_report(&pool)
        .await
        .expect("body_parity_report after corruption");
    assert_eq!(report.checked, 4, "still checks all 4 resources");
    assert!(!report.is_clean(), "corruption must be detected");
    assert_eq!(report.mismatches.len(), 1, "exactly one resource diverges");
    assert_eq!(
        report.mismatches[0].resource_id,
        fixture_ids::RESOURCE_TEAM,
        "the gate flags precisely the corrupted resource (by its preserved id)"
    );

    // The mismatch carries both bodies for diagnosis — production's intact body vs the corrupted one.
    assert_eq!(
        report.mismatches[0].production_body, "Team body text.",
        "production body reconstructs intact (unaffected by the temper_next corruption)"
    );
    assert_eq!(
        report.mismatches[0].new_body, "CORRUPTED BODY",
        "the new-substrate body reflects the corruption"
    );
}

/// End-to-end (WS6 §0): one consolidated assertion that a full `synthesis::run` over the prod-shape
/// fixture realizes the complete per-resource sequence — `resource_created` → `property_asserted` per
/// surviving key (§7) → `relationship_asserted` per edge (§4), folds as assert+fold pairs — and that
/// the run is parity-clean (a successful `run` IS the parity proof: it returns `Err` on any mismatch).
/// The per-slice tests above prove each pass in isolation; this proves the aggregate end state.
///
/// Count arithmetic, derived from `tests/fixtures/prod_shape.sql` (4 active: R1/R2/R3/R5; R4
/// soft-deleted, excluded §0):
/// * `kb_resources` = 4 — one row per active resource.
/// * `kb_content_blocks` = 4 — the §8 single up-front block (seq 0) per active resource.
/// * `kb_properties` = 9 — Property-fated keys via `facet_set` (`property_asserted` events): R2 carries
///   `temper-stage`/`temper-mode`/`temper-effort` (3 managed, §7 Property) + `custom-key`/`another-key`
///   (2 open_meta, always Property) = 5; R1/R3/R5 carry only Die-fated managed keys (title/slug/id) = 0;
///   PLUS the `doc_type` property `_project_resource_created` writes directly (NOT via a
///   `property_asserted` event), one per active resource = 4. Total 5 + 4 = 9.
/// * `kb_edges` = 3 — 3 source `kb_resource_edges` rows survive (all endpoints active) + 1 minted
///   `temper-goal` edge (R2's `temper-goal: goal-doc` → goal R1 → task R2, contains/parent_of) − 1
///   dedup (that minted edge equals the materialized R1→R2 contains/parent_of row) = 3.
/// * event histogram: `resource_created` = 4 (one per active resource); `property_asserted` = 5 (the
///   `facet_set` Property keys only — `doc_type` is a direct insert in the projector, never an event);
///   `relationship_asserted` = 3 (the 3 source edges; the minted goal edge deduped → never fired);
///   `relationship_folded` = 1 (the one folded fixture edge, the near R2→R3 row).
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn synthesizes_fixture_end_to_end(pool: sqlx::PgPool) {
    common::seed_prod_shape_fixture(&pool).await;

    // A successful run is itself the §8 parity-clean proof — `run` returns `Err` on any body mismatch.
    let report = temper_next::synthesis::run(&pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect("synthesis::run (clean fixture → full §0 sequence + parity holds)");

    // The report aggregates each pass: 4 resources, 5 Property-fated property fires, 3 edge fires.
    assert_eq!(report.resources, 4, "4 active resources synthesized");
    assert_eq!(
        report.properties, 5,
        "5 property_asserted fires (Property-fated managed + open_meta keys; doc_type is direct)"
    );
    assert_eq!(
        report.edges, 3,
        "3 edge fires (source edges; minted goal edge deduped)"
    );

    // ── aggregate destination state ──────────────────────────────────────────────────────────────
    let resources: i64 = sqlx::query_scalar("SELECT count(*) FROM temper_next.kb_resources")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(resources, 4, "one kb_resources row per active resource");

    let blocks: i64 = sqlx::query_scalar("SELECT count(*) FROM temper_next.kb_content_blocks")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(blocks, 4, "one §8 single content block per active resource");

    let properties: i64 = sqlx::query_scalar("SELECT count(*) FROM temper_next.kb_properties")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        properties, 9,
        "kb_properties = 5 facet_set Property keys + 4 direct doc_type properties"
    );

    let edges: i64 = sqlx::query_scalar("SELECT count(*) FROM temper_next.kb_edges")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        edges, 3,
        "kb_edges = 3 source edges + 1 minted goal edge − 1 dedup"
    );

    // ── event-type histogram (the §0 per-resource sequence, ledger-side) ──────────────────────────
    let histogram_rows = sqlx::query(
        "SELECT t.name AS name, count(*) AS n \
         FROM temper_next.kb_events e \
         JOIN temper_next.kb_event_types t ON t.id = e.event_type_id \
         GROUP BY t.name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let mut histogram: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for row in &histogram_rows {
        histogram.insert(row.get("name"), row.get("n"));
    }

    assert_eq!(
        histogram.get("resource_created").copied(),
        Some(4),
        "resource_created fired once per active resource"
    );
    assert_eq!(
        histogram.get("property_asserted").copied(),
        Some(5),
        "property_asserted fired per Property-fated key (doc_type is a direct insert, not an event)"
    );
    assert_eq!(
        histogram.get("relationship_asserted").copied(),
        Some(3),
        "relationship_asserted fired per synthesized edge (minted goal edge deduped, not fired)"
    );
    assert_eq!(
        histogram.get("relationship_folded").copied(),
        Some(1),
        "relationship_folded fired for the one folded fixture edge (assert+fold pair)"
    );
}
