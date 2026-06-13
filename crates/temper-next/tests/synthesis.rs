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
    // The anchor is the REMAPPED temper_next context id, never the production id.
    let anchor: Uuid = row.get("anchor_id");
    assert_ne!(
        anchor,
        fixture_ids::CONTEXT_ONE,
        "home anchors the remapped context id, not the production one"
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
