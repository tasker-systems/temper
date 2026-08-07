#![cfg(feature = "test-db")]
//! `/api/resources` answers in **one** shape.
//!
//! The list endpoint used to be modelled as `oneOf<ResourceListResponse, ResourceMetaListResponse>`
//! and selected between them on `?meta_only=true`: two envelopes over two row types. It answers in
//! `ResourceListResponse` unconditionally now, over `ResourceView` rows, and `?sections=` varies
//! which *parts* of that one shape are filled.
//!
//! Three witnesses, per the plan (`docs/superpowers/plans/2026-08-06-resource-view-convergence.md`,
//! Task 7). The third is the convergence witness — and its bite-check is on record as **failing to
//! bite** the property Task 1 owns: re-adding a hoisted `stage` column to `ResourceView` leaves it
//! green, because both sides gain the field, and only `no_workflow_field_is_hoisted` catches it.
//! Neither test is sufficient alone; that is why both exist.

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_services::backend::{substrate_read, DbBackend};
use temper_workflow::operations::{Backend, CreateResource, ShowResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;
use temper_workflow::types::resource::ResourceListParams;

mod common;

/// A single pre-chunked, pre-embedded segment (bring-your-own-vectors path) — ONNX-free, so a
/// seeded resource has real body bytes without the `test-embed` gate. Mirrors
/// `temper-services/tests/resource_view_sections_test.rs`'s `one_chunk_packed`.
fn one_chunk_packed(text: &str, hash_seed: &str) -> String {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: text.to_owned(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.1_f32; 768],
        embedded_with: None,
    };
    pack_chunks(&[chunk]).expect("pack chunk")
}

/// Seed a profile-owned context and `n` `task` resources in it, each carrying all three tiers:
/// a body, a managed key (`temper-stage`) and an open key. A section test whose subject has
/// nothing to serve cannot tell "not requested" from "empty".
async fn seed(pool: &PgPool, email: &str, n: usize) -> (DbBackend, ProfileId, Vec<ResourceId>) {
    let (profile, context) = common::fixtures::create_test_profile_with_context(pool, email).await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let slug = format!("view-row-{i:02}");
        let created = backend
            .create_resource(CreateResource {
                idempotency_key: None,
                slug: slug.clone(),
                doctype: "task".to_string(),
                home: HomeAnchor::Context(ContextId::from(context)),
                title: format!("View Row {i:02}"),
                body: None,
                managed_meta: ManagedMeta {
                    stage: Some("in-progress".to_string()),
                    ..ManagedMeta::default()
                },
                open_meta: Some(serde_json::json!({ "marker": "VIEW" })),
                goal: None,
                origin_uri: Some(format!("test://{slug}")),
                chunks_packed: Some(one_chunk_packed("the body has bytes", "aa")),
                content_hash: None,
                act: ActContext::default(),
                origin: Surface::ApiHttp,
            })
            .await
            .expect("create")
            .value;
        ids.push(created.id);
    }
    (backend, ProfileId::from(profile), ids)
}

fn list_params(
    sections: Option<&str>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> ResourceListParams {
    ResourceListParams {
        doc_type_name: Some("task".to_string()),
        sections: sections.map(str::to_string),
        limit,
        offset,
        ..Default::default()
    }
}

/// The envelope and the row type do not change with `?sections=`.
///
/// This is the property the retired `oneOf` broke: `meta_only=true` returned a *different type*,
/// so a generated client carried a union to discriminate and an agent had two shapes to learn.
/// Asking for a section now adds a key to the same object and changes nothing else — asserted key
/// by key, because "both deserialize into `ResourceListResponse`" is true by construction and
/// would witness nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_returns_one_shape_regardless_of_sections(pool: PgPool) {
    let (_backend, profile, _ids) = seed(&pool, "view-one-shape@example.com", 2).await;

    let default = substrate_read::list_select(&pool, profile, list_params(None, None, None))
        .await
        .expect("list with no sections");
    let with_meta =
        substrate_read::list_select(&pool, profile, list_params(Some("open-meta"), None, None))
            .await
            .expect("list with the open-meta section");

    let d = serde_json::to_value(&default).expect("serialize default page");
    let m = serde_json::to_value(&with_meta).expect("serialize sectioned page");

    let envelope_keys = |v: &serde_json::Value| {
        v.as_object()
            .expect("the list response is an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        envelope_keys(&d),
        envelope_keys(&m),
        "the envelope is the same shape either way — sections vary the ROW's filled parts, \
         never the response type"
    );

    let row_keys = |v: &serde_json::Value| {
        let mut keys = v["rows"][0]
            .as_object()
            .expect("a row is an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        keys
    };
    let default_keys = row_keys(&d);
    let mut sectioned_keys = row_keys(&m);

    assert!(
        sectioned_keys.contains(&"open_meta".to_string()),
        "asking for `open-meta` fills `open_meta`: {m}"
    );
    assert!(
        !default_keys.contains(&"open_meta".to_string()),
        "not asking for it leaves the key absent — absent means NOT REQUESTED: {d}"
    );
    sectioned_keys.retain(|k| k != "open_meta");
    assert_eq!(
        default_keys, sectioned_keys,
        "beyond the requested section the two rows carry exactly the same keys"
    );

    // The managed tier is not a section, so it is present on BOTH — which is what makes
    // dropping the hoisted stage/mode/effort/seq columns lossless.
    for (label, v) in [("default", &d), ("sectioned", &m)] {
        assert_eq!(
            v["rows"][0]["managed_meta"]["temper-stage"], "in-progress",
            "{label} page must carry the always-present managed tier: {v}"
        );
    }
}

/// The paging state rides on the wire, not on the client.
///
/// `returned` and `truncated` used to be injected render-time by the CLI alone, so MCP and
/// raw-HTTP callers never received them — while the shipped agent skill instructs every agent to
/// read all three. The last-page case is the one a naive `total > returned` gets wrong.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_envelope_carries_returned_and_truncated(pool: PgPool) {
    let (_backend, profile, ids) = seed(&pool, "view-paging@example.com", 5).await;
    let total = ids.len() as i64;

    let first = substrate_read::list_select(&pool, profile, list_params(None, Some(2), Some(0)))
        .await
        .expect("first page");
    assert_eq!(first.total, total, "`total` is the FILTERED match count");
    assert_eq!(first.returned, 2, "`returned` is this page's row count");
    assert_eq!(first.limit, Some(2), "the envelope echoes the page size");
    assert_eq!(first.offset, 0);
    assert!(
        first.truncated,
        "3 of 5 matching rows are beyond this page, so the caller may not conclude absence"
    );

    let last = substrate_read::list_select(&pool, profile, list_params(None, Some(2), Some(4)))
        .await
        .expect("last page");
    assert_eq!(last.returned, 1, "the tail page holds the remainder");
    assert_eq!(last.offset, 4);
    assert!(
        !last.truncated,
        "row 5 of 5 is the tail of the set — nothing is hidden even though total > returned"
    );
}

/// **The convergence witness.** `show` and a single-row `list` of the same resource, asked for
/// the same sections and with the body excluded, serialize **byte for byte identically**.
///
/// This is the whole plan's thesis as one assertion: six near-identical projections collapsed
/// into one, so "what does a resource look like?" has a single answer that does not depend on how
/// the resource was reached. Byte-equality (not key-set equality) is deliberate — a field that
/// rendered differently on the two paths would still be a second shape.
///
/// It is **not sufficient on its own**, and that is recorded rather than assumed: re-adding a
/// hoisted `stage` column to `ResourceView` leaves this test green, because both sides gain the
/// column. `no_workflow_field_is_hoisted` (`temper-core/src/types/resource_view.rs`) is what
/// catches that, and this comment is the reason it must not be deleted as redundant.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_and_single_row_list_agree_when_body_excluded(pool: PgPool) {
    let (backend, profile, ids) = seed(&pool, "view-witness@example.com", 1).await;
    let id = ids[0];

    // `show_resource` asks for `open-meta` and not `body`; the list asks for the same, so the
    // comparison is between two reads of the same question rather than two different questions.
    let shown = backend
        .show_resource(ShowResource {
            resource: id,
            origin: Surface::ApiHttp,
        })
        .await
        .expect("show")
        .value;

    let page =
        substrate_read::list_select(&pool, profile, list_params(Some("open-meta"), None, None))
            .await
            .expect("list");
    assert_eq!(page.rows.len(), 1, "the seed holds exactly one task");

    assert!(
        shown.content.is_none() && page.rows[0].content.is_none(),
        "precondition: neither read asked for the body, so neither carries one"
    );
    assert_eq!(
        serde_json::to_string(&shown).expect("serialize show"),
        serde_json::to_string(&page.rows[0]).expect("serialize list row"),
        "a resource has ONE shape: `show` and a single-row `list` of it are byte-identical"
    );
}
