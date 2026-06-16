#![cfg(feature = "artifact-tests")]
//! WS6 chunk 3 — parity-read harness. Each test ports one production read to `temper_next.*` and
//! asserts identical output for the same logical query over the synthesized prod-shape fixture.
//! Isolated ephemeral DB per test via `#[sqlx::test(migrator = ...)]` (NOT the psql-reset write-path
//! group). Runtime schema-qualified reads throughout (same discipline as `synthesis::source`).
mod common;

use std::collections::BTreeMap;

use common::fixture_ids;
use serde_json::{Map, Value};
use temper_next::readback::{self, ResolvedIds};

/// Smoke test: the chunk-3 harness composes. Seed the prod-shape fixture into `public.*`, synthesize
/// into `temper_next.*`, then build the `old↔new` id bimap by `origin_uri`. The synthesized id set is
/// the 4 ACTIVE fixture resources (R4 `temper://fixture/deleted-doc` is excluded, §0 active-only), and
/// the bimap round-trips for a known fixture resource.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn parity_harness_setup_synthesizes(pool: sqlx::PgPool) {
    common::seed_and_synthesize(&pool).await;

    let ids = ResolvedIds::load(&pool).await.expect("ResolvedIds::load");

    let new_ids: Vec<_> = ids.new_ids().collect();
    assert!(!new_ids.is_empty(), "synthesized id set is non-empty");
    assert_eq!(
        ids.len(),
        4,
        "4 active fixture resources synthesized (R4 deleted-doc excluded, §0 active-only)"
    );

    // The bimap round-trips for a known fixture resource (R2, the task).
    let new = ids
        .to_new(fixture_ids::RESOURCE_TASK)
        .expect("R2 (task) has a synthesized id");
    assert_eq!(
        ids.to_old(new),
        Some(fixture_ids::RESOURCE_TASK),
        "old→new→old round-trips for R2"
    );
    assert_eq!(
        ids.origin_uri_for_new(new),
        Some("temper://fixture/task-doc"),
        "the synthesized id resolves back to R2's origin_uri"
    );
}

/// WS2 — synthesis preserves production profile ids verbatim (spec D2 principal-mapping). The
/// prod-shape fixture's owner profile (`fixture_ids::OWNER_PROFILE`) must keep its id in
/// `temper_next` so `resources_visible_to(prod_profile)` resolves directly with no read-time bimap.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn synthesis_preserves_production_profile_ids(pool: sqlx::PgPool) {
    common::seed_and_synthesize(&pool).await;

    let preserved: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM temper_next.kb_profiles WHERE id = $1")
            .bind(fixture_ids::OWNER_PROFILE)
            .fetch_optional(&pool)
            .await
            .expect("query temper_next.kb_profiles by preserved id");

    assert_eq!(
        preserved,
        Some(fixture_ids::OWNER_PROFILE),
        "synthesis must preserve the production owner profile id verbatim (not re-mint it)"
    );
}

/// The projected list-row tuple compared across the two read paths, keyed by `origin_uri`.
type ListProjection = (
    String,         // title
    String,         // doc_type
    Option<String>, // stage
    Option<String>, // mode
    Option<String>, // effort
);

/// §9 — `list` parity. Seed + synthesize the prod-shape fixture, then assert `readback::list` over
/// `temper_next.*` returns the SAME rows with the SAME projected fields as production's `list_visible`
/// over `public.*` for the owner profile P1 (which owns all 4 active fixture resources, so the
/// filterless call returns exactly those 4).
///
/// Compared as a SET keyed by `origin_uri` (a verbatim-carried UNIQUE key), NOT in order: ordered-by-
/// `updated` parity is deliberately NOT asserted. Synthesis sources `kb_resources.updated` from the
/// genesis event's `occurred_at`, which is `now()` = transaction-start time, constant within the single
/// synthesis transaction — so every synthesized row shares one identical `updated` and `ORDER BY updated
/// DESC` is a non-deterministic tie over `temper_next`. The migration-time floor is the row SET + its
/// projected fields, not absolute recency ordering.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn list_parity(pool: sqlx::PgPool) {
    use temper_api::services::resource_service;
    use temper_core::types::resource::ResourceListParams;

    common::seed_and_synthesize(&pool).await;

    // Production read: list_visible(P1). P1 owns all 4 active fixture resources, so resources_visible_to
    // returns exactly those 4. A generous limit (>4) defeats any default page size.
    let params = ResourceListParams {
        limit: Some(100),
        ..Default::default()
    };
    let prod = resource_service::list_visible(&pool, fixture_ids::OWNER_PROFILE, params)
        .await
        .expect("production list_visible");

    let prod_by_uri: BTreeMap<String, ListProjection> = prod
        .rows
        .into_iter()
        .map(|r| {
            (
                r.origin_uri,
                (r.title, r.doc_type_name, r.stage, r.mode, r.effort),
            )
        })
        .collect();

    // Readback over temper_next.*.
    let next_by_uri: BTreeMap<String, ListProjection> = readback::list(&pool)
        .await
        .expect("readback::list")
        .into_iter()
        .map(|r| {
            (
                r.origin_uri,
                (r.title, r.doc_type, r.stage, r.mode, r.effort),
            )
        })
        .collect();

    assert_eq!(
        prod_by_uri.len(),
        4,
        "production returns the 4 active resources"
    );
    assert_eq!(
        next_by_uri.len(),
        4,
        "readback returns the 4 active resources"
    );
    assert_eq!(
        prod_by_uri, next_by_uri,
        "readback::list matches production list_visible row-set + projected fields (keyed by origin_uri)"
    );

    // Spot-check the workflow-field projection: R2 (task) carries stage/mode/effort verbatim...
    assert_eq!(
        next_by_uri.get("temper://fixture/task-doc"),
        Some(&(
            "Task Doc".to_string(),
            "task".to_string(),
            Some("doing".to_string()),
            Some("build".to_string()),
            Some("M".to_string()),
        )),
        "R2 projects its workflow keys verbatim"
    );
    // ...while R1 (goal-doc, a concept with no workflow keys) projects them all as None.
    assert_eq!(
        next_by_uri.get("temper://fixture/goal-doc"),
        Some(&(
            "Goal Doc".to_string(),
            "concept".to_string(),
            None,
            None,
            None,
        )),
        "R1 carries no workflow keys, so stage/mode/effort are None (LEFT-JOIN absent, not dropped)"
    );
}

/// The §7-died (and relocated) managed keys: never reappear in a readback's reconstructed managed map.
/// `temper-title`/`temper-slug`/`temper-id`/`temper-context` DIE (the column/render/row-id/home row
/// carry that state authoritatively); `temper-goal` becomes an EDGE; `temper-type` reconciles to the
/// `doc_type` column. Production's manifest still carries all six, so the expected managed set strips
/// them before comparing against readback.
const DROPPED_MANAGED_KEYS: &[&str] = &[
    "temper-title",
    "temper-slug",
    "temper-id",
    "temper-context",
    "temper-goal",
    "temper-type",
];

/// Coerce a serialized `ManagedMeta` (or `open_meta`) `Value` into a JSON object map. Both production
/// shapes are always JSON objects; a non-object is a contract break we want to surface loudly.
fn as_object(v: Value, what: &str) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        other => panic!("expected {what} to be a JSON object, got {other:?}"),
    }
}

/// §9 — `show` + `get_meta` parity. §7 dissolves the production manifest into `kb_resources` columns,
/// the home row, the `doc_type` property, edges, and `kb_properties`; readback reconstructs the
/// managed/open split from `kb_properties` using the inverse fate set (`MANAGED_PROPERTY_KEYS`). The
/// §7-died keys (title/slug/id/context) are gone by construction — the column/home/id carry that state
/// authoritatively — and the relocated keys (`temper-goal` → edge, `temper-type` → `doc_type` column)
/// are absent too. So parity holds modulo `DROPPED_MANAGED_KEYS`: production managed, minus those six,
/// equals readback managed; production `open_meta` equals readback open verbatim.
///
/// The "show" auth gate is `resource_service::get_visible`, already invoked inside `get_meta` — the
/// meta reconstruction IS the show+meta parity (no separate `readback::show` is built).
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn show_and_meta_parity(pool: sqlx::PgPool) {
    use temper_api::services::meta_service;
    use temper_core::types::ids::{ProfileId, ResourceId};

    common::seed_and_synthesize(&pool).await;

    let ids = ResolvedIds::load(&pool).await.expect("ResolvedIds::load");
    assert_eq!(ids.len(), 4, "4 active fixture resources synthesized");

    // Keyed by origin_uri so the per-resource spot-checks can find R1/R2 regardless of iteration order.
    let mut rb_by_uri: BTreeMap<String, readback::ReconstructedMeta> = BTreeMap::new();

    for new_id in ids.new_ids() {
        let origin_uri = ids
            .origin_uri_for_new(new_id)
            .expect("synthesized id has an origin_uri")
            .to_string();
        let old_id = ids
            .to_old(new_id)
            .expect("synthesized id maps back to prod");

        // Production read (auth-gated by get_visible inside get_meta) for the owner profile P1.
        let prod = meta_service::get_meta(
            &pool,
            ProfileId::from(fixture_ids::OWNER_PROFILE),
            ResourceId::from(old_id),
        )
        .await
        .expect("production get_meta");

        let prod_managed_raw = as_object(
            serde_json::to_value(prod.managed_meta.expect("prod managed_meta present"))
                .expect("serialize managed_meta"),
            "managed_meta",
        );
        let prod_open = as_object(prod.open_meta.expect("prod open_meta present"), "open_meta");

        // Production managed minus the §7-died/relocated keys = what readback should reconstruct.
        let mut expected_managed = prod_managed_raw.clone();
        for k in DROPPED_MANAGED_KEYS {
            expected_managed.remove(*k);
        }

        let rb = readback::meta(&pool, new_id).await.expect("readback::meta");

        assert_eq!(
            rb.managed, expected_managed,
            "{origin_uri}: surviving managed keys + values match production minus the §7-dropped set"
        );
        assert_eq!(
            rb.open, prod_open,
            "{origin_uri}: open keys carry verbatim, matching production open_meta"
        );

        // The §7 guarantee: died keys never reappear in the reconstructed managed map.
        for died in ["temper-title", "temper-slug", "temper-id", "temper-context"] {
            assert!(
                !rb.managed.contains_key(died),
                "{origin_uri}: §7-died key {died} must be absent from readback managed"
            );
        }

        // doc_type is the successor to production's temper-type when present, and always equals the
        // authoritative doctype synthesis stamped (cross-checked against itself + temper-type).
        if let Some(prod_type) = prod_managed_raw.get("temper-type") {
            assert_eq!(
                Value::String(rb.doc_type.clone()),
                *prod_type,
                "{origin_uri}: readback doc_type == production temper-type"
            );
        }

        rb_by_uri.insert(origin_uri, rb);
    }

    // Spot-check R2 (task): the rich case — three surviving managed workflow keys, two open keys, task.
    let r2 = rb_by_uri
        .get("temper://fixture/task-doc")
        .expect("R2 reconstructed");
    let expected_r2_managed: Map<String, Value> = [
        ("temper-stage", "doing"),
        ("temper-mode", "build"),
        ("temper-effort", "M"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
    .collect();
    assert_eq!(
        r2.managed, expected_r2_managed,
        "R2 managed = exactly the three surviving workflow keys, values verbatim"
    );
    let expected_r2_open: Map<String, Value> = [
        ("custom-key", "custom-value"),
        ("another-key", "another-value"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
    .collect();
    assert_eq!(
        r2.open, expected_r2_open,
        "R2 open = its two open_meta keys verbatim"
    );
    assert_eq!(
        r2.doc_type, "task",
        "R2 doc_type is the reconciled task doctype"
    );

    // Spot-check R1 (goal-doc, a concept): no surviving managed keys, no open keys, doctype concept.
    let r1 = rb_by_uri
        .get("temper://fixture/goal-doc")
        .expect("R1 reconstructed");
    assert!(
        r1.managed.is_empty(),
        "R1 carries only died managed keys → empty managed"
    );
    assert!(r1.open.is_empty(), "R1 has no open_meta keys");
    assert_eq!(r1.doc_type, "concept", "R1 doc_type is concept");
}

/// §9 — body-reconstruction read parity (closes the §9 body read floor). For every active fixture
/// resource, the markdown `readback::body` reconstructs from `temper_next` chunks must equal the body
/// production's `get_content` serves today (`ContentResponse.markdown`).
///
/// This read-surface check OVERLAPS the §8 synthesis body-parity gate (`synthesis::parity`) — by design:
/// it exercises the SAME `reconstruct_body` + `new_substrate_chunks` algorithm, but reached as a read
/// (resource_id → body) rather than as the cutover gate's two-source comparison. Sharing one assembler
/// is the point (CONFORM, no second body path); the new floor is that the read surface itself round-trips.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn body_read_parity(pool: sqlx::PgPool) {
    use temper_api::services::resource_service;

    common::seed_and_synthesize(&pool).await;

    let ids = ResolvedIds::load(&pool).await.expect("ResolvedIds::load");
    assert_eq!(ids.len(), 4, "4 active fixture resources synthesized");

    for new_id in ids.new_ids() {
        let origin_uri = ids
            .origin_uri_for_new(new_id)
            .expect("synthesized id has an origin_uri")
            .to_string();
        let old_id = ids
            .to_old(new_id)
            .expect("synthesized id maps back to prod");

        // Production body: get_content's assembled markdown (auth-gated for owner P1).
        let prod = resource_service::get_content(&pool, fixture_ids::OWNER_PROFILE, old_id)
            .await
            .expect("production get_content")
            .markdown;

        // Readback body: reconstructed from temper_next chunks via the shared §8 assembler.
        let rb = readback::body(&pool, new_id).await.expect("readback::body");

        assert_eq!(
            rb, prod,
            "{origin_uri}: readback::body == production get_content markdown"
        );
    }

    // Spot-assert R2 (task-doc): the non-vacuous multi-chunk + heading case — an unheaded preamble
    // chunk followed by a depth-2 headed chunk, joined with a blank line.
    let r2_new = ids
        .to_new(fixture_ids::RESOURCE_TASK)
        .expect("R2 (task) has a synthesized id");
    let r2_body = readback::body(&pool, r2_new)
        .await
        .expect("readback::body R2");
    assert_eq!(
        r2_body, "Task intro paragraph.\n\n## Goals\n\nTask goals section body.",
        "R2 reconstructs the preamble + depth-2 heading case verbatim"
    );
}

/// §9 — FTS search parity. For a fixed query set, `readback::fts_search` over `temper_next.*` (a
/// tsvector REBUILT per §9 — title weight-A, body weight-B, body = RAW current-chunk content
/// space-joined) must find the SAME resources as production FTS (`search_service::search`, FTS-only)
/// over the synthesized prod-shape fixture.
///
/// The parity floor is the MATCHING SET (a `BTreeSet` of origin_uris), NOT the ordered list, for three
/// independent reasons:
///   1. Production's tsvector is `setweight(title,'A') || setweight(slug,'A') || setweight(body,'B')`
///      (`rebuild_resource_search_vector`, migration 20260405000001) — slug is weight-A. §7 dissolved
///      slug, so §9 explicitly REBUILDS FTS title-only weight-A. Production can rank a slug match at A;
///      readback structurally cannot. Absolute `ts_rank` values and the order among equal-weight
///      matches therefore legitimately differ — they are NOT a migration invariant.
///   2. `plainto_tsquery` is AND-semantics, so multi-term queries narrow; with this tiny fixture there
///      is no deterministic multi-result ordering to assert against.
/// (This mirrors `list_parity`, which likewise compares a set, not `updated`-ordering.) `fts_search`
/// still ORDERs by `ts_rank DESC` to stay faithful to production; the TEST compares results as sets.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn fts_parity(pool: sqlx::PgPool) {
    use std::collections::BTreeSet;

    use temper_api::services::search_service;
    use temper_core::types::api::SearchParams;

    common::seed_and_synthesize(&pool).await;

    // Each query → the set of origin_uris it should find (same set both sides). "body" is in every
    // active resource's body text, so it finds all 4. "task" hits R2's title + body only. "goal" hits
    // R1's title "Goal Doc" AND R2's body "Task goals section body." — the english config STEMS
    // "goals" → "goal", so the body match pulls R2 in on BOTH sides (production via its body@B,
    // readback via the same raw-chunk body@B); the parity floor (prod set == readback set) is what
    // matters, and it holds for the stemmed two-hit case too.
    let cases: &[(&str, &[&str])] = &[
        (
            "goal",
            &["temper://fixture/goal-doc", "temper://fixture/task-doc"],
        ),
        ("task", &["temper://fixture/task-doc"]),
        (
            "body",
            &[
                "temper://fixture/goal-doc",
                "temper://fixture/task-doc",
                "temper://fixture/decision-doc",
                "temper://fixture/team-doc",
            ],
        ),
    ];

    for (query, expected) in cases {
        // Production FTS-only: query set, no embedding → compute_weights = (1.0, 0.0). graph_expand
        // off routes through unified_search (FTS path). search_config "english" matches readback's
        // 'english'. A generous limit defeats the default page size on the "body" (4-hit) case.
        let params = SearchParams {
            query: Some((*query).to_string()),
            embedding: None,
            search_config: "english".to_string(),
            graph_expand: false,
            limit: Some(50),
            ..Default::default()
        };
        let prod_set: BTreeSet<String> =
            search_service::search(&pool, fixture_ids::OWNER_PROFILE, params)
                .await
                .expect("production FTS search")
                .into_iter()
                .map(|r| r.origin_uri)
                .collect();

        let rb_set: BTreeSet<String> = readback::fts_search(&pool, query)
            .await
            .expect("readback::fts_search")
            .into_iter()
            .collect();

        let expected_set: BTreeSet<String> = expected.iter().map(|s| (*s).to_string()).collect();

        // The load-bearing parity floor: production FTS and readback find the SAME resources.
        assert_eq!(
            prod_set, rb_set,
            "production FTS and readback::fts_search find the SAME resources for {query:?}"
        );
        // ...and that shared set is the expected, non-vacuous anchor.
        assert_eq!(
            prod_set, expected_set,
            "production FTS for {query:?} finds the expected resource set"
        );
        assert_eq!(
            rb_set, expected_set,
            "readback::fts_search for {query:?} finds the expected resource set"
        );
    }

    // Non-vacuous spot-check: "body" appears in every active body, so both sides find exactly 4.
    let body_hits = readback::fts_search(&pool, "body")
        .await
        .expect("readback::fts_search body");
    assert_eq!(
        body_hits.len(),
        4,
        "\"body\" is in every active body text → exactly 4 hits (non-vacuous)"
    );
}

/// §9 — vector search parity. Unlike FTS (where production's slug@A weight makes only SET parity an
/// invariant), embeddings carry VERBATIM through synthesis (§8), so production vector search and
/// readback must agree on the EXACT ORDERED list. readback mirrors production's `vec_hits`: per
/// resource the best (MIN cosine-distance) current chunk decides rank, results ascend by that
/// distance.
///
/// The fixture embeddings are 0.01 in every dimension except dimension 1, which carries a distinct
/// per-chunk discriminating value (DISTINCT DIRECTIONS — cosine distance is magnitude-invariant, so
/// colinear vectors would be a total tie). A query that loads dimension 1 (`q[0] = 1.0`, rest 0.01)
/// orders the resources strictly by their best chunk's dimension-1 value: R5(0.5) < R3(0.3) <
/// R2(0.25 via its CLOSER chunk1) < R1(0.1) by ascending cosine distance. The R2 case is non-vacuous
/// proof that per-resource MIN distance picks the closer chunk (chunk1@0.25, not chunk0@0.2).
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn vector_parity(pool: sqlx::PgPool) {
    use temper_api::services::search_service;
    use temper_core::types::api::SearchParams;

    common::seed_and_synthesize(&pool).await;

    // Query embedding: dimension 1 (0-indexed dim 0) carries the full weight, rest tiny — so the rank
    // is decided by each resource's best chunk's dimension-1 value.
    let mut q = vec![0.01_f32; 768];
    q[0] = 1.0;

    // Production vector-only search: embedding present, query None → compute_weights = (0.0, 1.0).
    // graph_expand off routes through unified_search (vec_hits). Results come back ordered by
    // combined_score DESC == ascending MIN cosine distance. Collect origin_uris IN RETURNED ORDER.
    let params = SearchParams {
        query: None,
        embedding: Some(q.clone()),
        search_config: "english".to_string(),
        graph_expand: false,
        limit: Some(50),
        ..Default::default()
    };
    let prod_order: Vec<String> = search_service::search(&pool, fixture_ids::OWNER_PROFILE, params)
        .await
        .expect("production vector search")
        .into_iter()
        .map(|r| r.origin_uri)
        .collect();

    let rb_order: Vec<String> = readback::vector_search(&pool, &q)
        .await
        .expect("readback::vector_search");

    // The load-bearing invariant: embeddings carry verbatim, so the ORDERED lists match bit-for-bit
    // (contrast fts_parity, where slug@A makes only the SET an invariant).
    assert_eq!(
        prod_order, rb_order,
        "production vector search and readback::vector_search agree on the EXACT ORDER"
    );

    // ...and that shared order is the expected, non-vacuous anchor: per-resource MIN distance picks
    // R2's closer chunk1 (@0.25), placing R2 ahead of R1 (@0.1) but behind R3 (@0.3) and R5 (@0.5).
    let expected = vec![
        "temper://fixture/team-doc".to_string(),
        "temper://fixture/decision-doc".to_string(),
        "temper://fixture/task-doc".to_string(),
        "temper://fixture/goal-doc".to_string(),
    ];
    assert_eq!(
        rb_order, expected,
        "vector ranking orders resources by their best chunk's cosine distance (R5<R3<R2<R1)"
    );
}

/// One 1-hop neighbor tuple compared across the two read paths:
/// `(neighbor_origin_uri, edge_kind, polarity, label)`. `label` is `Option<String>`: temper_next
/// carries an empty production label as `NULL`, so the production oracle normalizes its NOT-NULL
/// `label` column the same way (empty string → `None`) before comparing as a set.
type NeighborTuple = (String, String, String, Option<String>);

/// §9 — graph-neighbors read parity (the LAST chunk-3 read floor). For a fixture resource, the 1-hop
/// neighbor set `readback::neighbors` reads over `temper_next.kb_edges` must equal production's 1-hop
/// neighbor set over `public.kb_resource_edges` — same `NOT is_folded` gate, same
/// `(edge_kind, polarity, label)` projection.
///
/// PRODUCTION ORACLE: a DIRECT symmetric edge query over `public.kb_resource_edges`, NOT
/// `graph_service::aggregator_subgraph`. `aggregator_subgraph` is subgraph-over-a-node-set (you pass it
/// a node set; it returns the edges AMONG them), so using it as a 1-hop neighbor oracle would be
/// circular. The faithful oracle is the symmetric direct read (seed as source OR as target), with the
/// SAME table + `NOT is_folded` gate + `edge_kind`/`polarity`/`label` projection that
/// `aggregator_subgraph` itself uses (graph_service.rs:185-205, `FROM kb_resource_edges ... AND NOT
/// is_folded`).
///
/// Two cases anchor the floor:
///   - R2 (task-doc): the load-bearing case — it touches the minted/deduped `parent_of` edge (E1) AND
///     the FOLDED `near` edge (E2). The folded edge must be EXCLUDED on both sides (decision-doc must
///     NOT appear in R2's neighbor set).
///   - R1 (goal-doc): multi-neighbor + inverse-polarity coverage (forward `contains`→task-doc AND
///     inverse `leads_to`→decision-doc), polarity/label carried verbatim.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn graph_parity(pool: sqlx::PgPool) {
    use std::collections::BTreeSet;

    use sqlx::Row as _;
    use uuid::Uuid;

    common::seed_and_synthesize(&pool).await;

    let ids = ResolvedIds::load(&pool).await.expect("ResolvedIds::load");

    // Production oracle: the symmetric direct 1-hop neighbor read over public.kb_resource_edges (both
    // directions, NOT is_folded), joining to kb_resources for the OTHER endpoint's origin_uri. The
    // NOT-NULL production `label` is normalized empty→None to match temper_next's empty→NULL carry.
    let prod_neighbors = |old_id: Uuid| {
        let pool = pool.clone();
        async move {
            let rows = sqlx::query(
                "SELECT t.origin_uri AS origin_uri, e.edge_kind::text AS edge_kind, \
                        e.polarity::text AS polarity, e.label \
                   FROM public.kb_resource_edges e \
                   JOIN public.kb_resources t ON t.id = e.target_resource_id AND t.is_active \
                  WHERE e.source_resource_id = $1 AND NOT e.is_folded \
                 UNION ALL \
                 SELECT s.origin_uri AS origin_uri, e.edge_kind::text AS edge_kind, \
                        e.polarity::text AS polarity, e.label \
                   FROM public.kb_resource_edges e \
                   JOIN public.kb_resources s ON s.id = e.source_resource_id AND s.is_active \
                  WHERE e.target_resource_id = $1 AND NOT e.is_folded",
            )
            .bind(old_id)
            .fetch_all(&pool)
            .await
            .expect("production neighbor query");

            rows.iter()
                .map(|r| {
                    let label: String = r.get("label");
                    (
                        r.get::<String, _>("origin_uri"),
                        r.get::<String, _>("edge_kind"),
                        r.get::<String, _>("polarity"),
                        (!label.is_empty()).then_some(label),
                    )
                })
                .collect::<BTreeSet<NeighborTuple>>()
        }
    };

    let readback_neighbors = |new_id: Uuid| {
        let pool = pool.clone();
        async move {
            readback::neighbors(&pool, new_id)
                .await
                .expect("readback::neighbors")
                .into_iter()
                .map(|n| (n.origin_uri, n.edge_kind, n.polarity, n.label))
                .collect::<BTreeSet<NeighborTuple>>()
        }
    };

    // --- R2 (task-doc): minted parent_of included, folded near→decision-doc excluded. ---
    let r2_new = ids
        .to_new(fixture_ids::RESOURCE_TASK)
        .expect("R2 (task) synthesized");
    let r2_prod = prod_neighbors(fixture_ids::RESOURCE_TASK).await;
    let r2_rb = readback_neighbors(r2_new).await;

    let r2_expected: BTreeSet<NeighborTuple> = [(
        "temper://fixture/goal-doc".to_string(),
        "contains".to_string(),
        "forward".to_string(),
        Some("parent_of".to_string()),
    )]
    .into_iter()
    .collect();

    assert_eq!(
        r2_prod, r2_rb,
        "R2: readback neighbors == production neighbors (symmetric direct edge read, NOT is_folded)"
    );
    assert_eq!(
        r2_rb, r2_expected,
        "R2: only the minted/deduped parent_of edge; the folded near→decision-doc edge is excluded"
    );
    assert!(
        !r2_rb
            .iter()
            .any(|(uri, ..)| uri == "temper://fixture/decision-doc"),
        "R2: the FOLDED near→decision-doc edge must NOT appear (excluded both sides)"
    );

    // --- R1 (goal-doc): multi-neighbor + inverse polarity. ---
    let r1_new = ids
        .to_new(fixture_ids::RESOURCE_GOAL)
        .expect("R1 (goal) synthesized");
    let r1_prod = prod_neighbors(fixture_ids::RESOURCE_GOAL).await;
    let r1_rb = readback_neighbors(r1_new).await;

    let r1_expected: BTreeSet<NeighborTuple> = [
        (
            "temper://fixture/task-doc".to_string(),
            "contains".to_string(),
            "forward".to_string(),
            Some("parent_of".to_string()),
        ),
        (
            "temper://fixture/decision-doc".to_string(),
            "leads_to".to_string(),
            "inverse".to_string(),
            Some("derived_from".to_string()),
        ),
    ]
    .into_iter()
    .collect();

    assert_eq!(
        r1_prod, r1_rb,
        "R1: readback neighbors == production neighbors (forward contains + inverse leads_to)"
    );
    assert_eq!(
        r1_rb, r1_expected,
        "R1: forward contains→task-doc (parent_of) + inverse leads_to→decision-doc (derived_from)"
    );
}

/// §9 — full-row (`show`/`by_uri`) parity at the INVARIANT-FIELD subset. The non-invariant fields
/// (re-minted ids, §7-dissolved slug/hashes, synthesis-collapsed timestamps) are deliberately NOT
/// compared — see the 4b spec's parity-floor amendment.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn resource_row_parity(pool: sqlx::PgPool) {
    use temper_api::services::resource_service;

    common::seed_and_synthesize(&pool).await;
    let ids = ResolvedIds::load(&pool).await.expect("ResolvedIds::load");
    assert_eq!(ids.len(), 4, "4 active fixture resources synthesized");

    for new_id in ids.new_ids() {
        let origin_uri = ids
            .origin_uri_for_new(new_id)
            .expect("origin_uri")
            .to_string();
        let old_id = ids.to_old(new_id).expect("maps back to prod");

        let rb = readback::resource_row(&pool, new_id)
            .await
            .expect("readback::resource_row");
        // Production oracle: get_visible returns the full ResourceRow for the owner profile.
        let prod = resource_service::get_visible(&pool, fixture_ids::OWNER_PROFILE, old_id)
            .await
            .expect("prod get_visible");

        assert_eq!(rb.origin_uri, prod.origin_uri, "{origin_uri}: origin_uri");
        assert_eq!(rb.title, prod.title, "{origin_uri}: title");
        assert_eq!(rb.is_active, prod.is_active, "{origin_uri}: is_active");
        assert_eq!(
            rb.context_name, prod.context_name,
            "{origin_uri}: context_name"
        );
        assert_eq!(
            rb.doc_type_name, prod.doc_type_name,
            "{origin_uri}: doc_type_name"
        );
        // owner_handle is NOT asserted: production projects it caller-relative ("@me" when
        // owner == caller); the substrate carries the raw handle. It is a render-time decoration,
        // non-invariant — see the 4b spec parity-floor amendment.
        assert_eq!(rb.stage, prod.stage, "{origin_uri}: stage");
        assert_eq!(rb.mode, prod.mode, "{origin_uri}: mode");
        assert_eq!(rb.effort, prod.effort, "{origin_uri}: effort");
        assert_eq!(rb.seq, prod.seq, "{origin_uri}: seq");
        // body_hash is NOT asserted: production stores the manifest hash while temper_next recomputes
        // a merkle over chunks (different algorithms) — the §8 gate compares reconstructed body TEXT,
        // not the hash columns (proven separately by `body_read_parity`). Non-invariant.
    }
}

/// §9 — the wiring proof (spec proof gate 1): drive `show` through temper-api's `NextBackend` (the
/// trait layer the HTTP show path uses) rather than `readback::*` directly, and assert it reconstructs
/// the same invariant fields as `readback::resource_row`. Confirms the wiring layer preserves the §9
/// floor, not just the underlying SQL. Gated on `next-backend` (temper-api's NextBackend, a dev-dep
/// feature): `cargo nextest run -p temper-next --features artifact-tests,next-backend`.
#[cfg(feature = "next-backend")]
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn show_through_next_backend_preserves_invariants(pool: sqlx::PgPool) {
    use temper_api::backend::NextBackend;
    use temper_core::operations::{Backend, ResourceRef, ShowResource, Surface};
    use temper_core::types::ids::{ProfileId, ResourceId};

    common::seed_and_synthesize(&pool).await;
    let ids = ResolvedIds::load(&pool).await.expect("ResolvedIds::load");
    let backend = NextBackend::new(pool.clone(), ProfileId::from(fixture_ids::OWNER_PROFILE));

    for new_id in ids.new_ids() {
        let old_id = ids.to_old(new_id).expect("maps back to prod");
        let out = backend
            .show_resource(ShowResource {
                resource: ResourceRef::Uuid {
                    id: ResourceId::from(old_id),
                },
                origin: Surface::ApiHttp,
            })
            .await
            .expect("NextBackend::show_resource");

        // The wiring must reproduce readback::resource_row's invariant fields.
        let direct = readback::resource_row(&pool, new_id)
            .await
            .expect("readback::resource_row");
        assert_eq!(
            out.value.origin_uri, direct.origin_uri,
            "origin_uri via NextBackend"
        );
        assert_eq!(out.value.title, direct.title, "title via NextBackend");
        assert_eq!(
            out.value.doc_type_name, direct.doc_type_name,
            "doc_type_name via NextBackend"
        );
        assert_eq!(
            out.value.context_name, direct.context_name,
            "context_name via NextBackend"
        );
        assert_eq!(out.value.stage, direct.stage, "stage via NextBackend");
        assert_eq!(out.value.mode, direct.mode, "mode via NextBackend");
        assert_eq!(out.value.effort, direct.effort, "effort via NextBackend");
        assert_eq!(out.value.seq, direct.seq, "seq via NextBackend");
        // Non-invariant fields are present but best-effort: slug/hashes None.
        assert!(out.value.slug.is_none(), "slug §7-dissolved");
        assert!(
            out.value.managed_hash.is_none() && out.value.open_hash.is_none(),
            "manifest hashes §7-dissolved"
        );
    }
}

/// §9 — the read-selector wiring proof (spec proof gate 2, harness level): drive the service-direct
/// read selector (`list` / `get_content` / `get_meta` / `search`) under both `Legacy` and `Next` over
/// the synthesized fixture, asserting the `Next` arm returns the same invariant data as `Legacy`. This
/// behavior-tests the selector's `Next` reconstruction (full-row list, body, typed-meta assembly, FTS
/// enrichment) end-to-end through the temper-api wiring, not just `readback::*`. Gated on `next-backend`.
#[cfg(feature = "next-backend")]
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn read_selector_next_matches_legacy(pool: sqlx::PgPool) {
    use std::collections::{BTreeMap, BTreeSet};

    use temper_api::backend::read_selector;
    use temper_api::backend::BackendSelection;
    use temper_core::types::api::SearchParams;
    use temper_core::types::ids::{ProfileId, ResourceId};
    use temper_core::types::resource::{ResourceListParams, ResourceListResponse};

    common::seed_and_synthesize(&pool).await;
    let p1 = fixture_ids::OWNER_PROFILE;

    // --- list: invariant projection per origin_uri must match ---
    type Proj = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        bool,
    );
    let project = |r: &ResourceListResponse| -> BTreeMap<String, Proj> {
        r.rows
            .iter()
            .map(|x| {
                (
                    x.origin_uri.clone(),
                    (
                        x.title.clone(),
                        x.doc_type_name.clone(),
                        x.stage.clone(),
                        x.mode.clone(),
                        x.effort.clone(),
                        x.is_active,
                    ),
                )
            })
            .collect()
    };
    let params = ResourceListParams {
        limit: Some(100),
        ..Default::default()
    };
    let leg = read_selector::list_select(BackendSelection::Legacy, &pool, p1, params.clone())
        .await
        .expect("legacy list");
    let nxt = read_selector::list_select(BackendSelection::Next, &pool, p1, params)
        .await
        .expect("next list");
    assert_eq!(
        project(&leg),
        project(&nxt),
        "list_select Next matches Legacy invariant projection (per origin_uri)"
    );
    assert_eq!(nxt.total, 4, "next list total = 4 active fixture resources");

    // --- get_content (R2 task): body markdown must match ---
    let r2 = fixture_ids::RESOURCE_TASK;
    let leg_c = read_selector::get_content_select(BackendSelection::Legacy, &pool, p1, r2)
        .await
        .expect("legacy content");
    let nxt_c = read_selector::get_content_select(BackendSelection::Next, &pool, p1, r2)
        .await
        .expect("next content");
    assert_eq!(
        leg_c.markdown, nxt_c.markdown,
        "get_content_select Next markdown == Legacy"
    );

    // --- get_meta (R2): open_meta carries verbatim; managed reconstructed ---
    let leg_m = read_selector::get_meta_select(
        BackendSelection::Legacy,
        &pool,
        ProfileId::from(p1),
        ResourceId::from(r2),
    )
    .await
    .expect("legacy meta");
    let nxt_m = read_selector::get_meta_select(
        BackendSelection::Next,
        &pool,
        ProfileId::from(p1),
        ResourceId::from(r2),
    )
    .await
    .expect("next meta");
    assert_eq!(
        leg_m.open_meta, nxt_m.open_meta,
        "get_meta_select Next open_meta == Legacy (open keys carry verbatim)"
    );
    assert!(
        nxt_m.managed_meta.is_some(),
        "next get_meta reconstructs managed_meta from kb_properties"
    );

    // --- search: the matching origin_uri SET must match (scores are not invariants) ---
    let sp = SearchParams {
        query: Some("task".to_string()),
        embedding: None,
        search_config: "english".to_string(),
        graph_expand: false,
        limit: Some(50),
        ..Default::default()
    };
    let leg_s: BTreeSet<String> =
        read_selector::search_select(BackendSelection::Legacy, &pool, p1, sp.clone())
            .await
            .expect("legacy search")
            .into_iter()
            .map(|r| r.origin_uri)
            .collect();
    let nxt_s: BTreeSet<String> =
        read_selector::search_select(BackendSelection::Next, &pool, p1, sp)
            .await
            .expect("next search")
            .into_iter()
            .map(|r| r.origin_uri)
            .collect();
    assert_eq!(
        leg_s, nxt_s,
        "search_select Next origin_uri set == Legacy for query 'task'"
    );
}
