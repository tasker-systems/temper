//! The read paths answer in `ResourceView`, and a section is served only when it is asked for.
//!
//! Three of these are the conjuncts the convergence rests on
//! (`internal/superpowers/plans/2026-08-06-resource-view-convergence.md`, Task 5): `content` tracks
//! the `body` section, `open_meta` tracks the `open-meta` section, and `managed_meta` tracks
//! neither because it is always present — which is what makes dropping the hoisted
//! `stage`/`mode`/`effort`/`seq` columns lossless.
//!
//! The fourth covers logic this task introduces rather than a clause it inherits: the list path
//! now reads its whole page through one set-shaped readback (`readback::hit_identities`, whose
//! `WHERE r.id = ANY($2)` carries no ORDER BY), so the page's sort order is restored in Rust and
//! needs its own witness.
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_core::types::resource_view::{ResourceSection, SectionSet};
use temper_services::backend::{substrate_read, DbBackend};
use temper_workflow::operations::{Backend, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;
use temper_workflow::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

/// Seed a substrate profile + a profile-owned `temper` context (the minimum the write path's
/// `resolve_emitter` + visibility gate require). Mirrors `open_meta_roundtrip_test.rs`'s and
/// `segmented_backend_test.rs`'s inlined fixture — each test target keeps its own copy so it has
/// no cross-target test-harness dependency.
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (uuid::Uuid, uuid::Uuid) {
    let profile_id = uuid::Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

/// A single pre-chunked, pre-embedded segment (bring-your-own-vectors path) — ONNX-free, so the
/// body section has real bytes to serve without the `test-embed` gate. Mirrors
/// `segmented_backend_test.rs`'s `one_chunk_packed`.
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

/// The body text every seeded resource carries, so a served `content` is recognizable.
const BODY_TEXT: &str = "the body section has bytes";

/// Seed a profile + context + one resource carrying all three tiers: a body (packed chunks), a
/// managed key (`temper-stage`), and an open key. Every test below needs all three present —
/// a section test whose subject has nothing to serve cannot tell "not requested" from "empty".
async fn seed_resource(
    pool: &PgPool,
    email: &str,
    slug: &str,
) -> (ProfileId, temper_core::types::resource_view::ResourceView) {
    let (profile, context) = seed_profile_with_context(pool, email).await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let created = backend
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: slug.to_string(),
            doctype: "task".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: slug.to_string(),
            body: None,
            managed_meta: ManagedMeta {
                stage: Some("in-progress".to_string()),
                ..ManagedMeta::default()
            },
            open_meta: Some(serde_json::json!({"marker": "SECTIONS"})),
            goal: None,
            origin_uri: Some(format!("test://{slug}")),
            chunks_packed: Some(one_chunk_packed(BODY_TEXT, "aa")),
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create")
        .value;
    (ProfileId::from(profile), created)
}

/// `content` is populated only when the `body` section is asked for.
///
/// The positive half is what gives the negative half a bite: a read that never fills `content`
/// would satisfy "absent when not requested" trivially and be wrong.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn section_body_absent_omits_content(pool: PgPool) {
    let (profile, created) = seed_resource(&pool, "sections-body@example.com", "zz-sec-body").await;

    let without =
        substrate_read::show_view_select(&pool, profile, created.id, &SectionSet::default())
            .await
            .expect("show without the body section");
    assert!(
        without.content.is_none(),
        "the body section was not requested, so `content` must be absent — \
         `None` means NOT REQUESTED, never \"empty body\": {:?}",
        without.content
    );

    let with: SectionSet = [ResourceSection::Body].into_iter().collect();
    let with = substrate_read::show_view_select(&pool, profile, created.id, &with)
        .await
        .expect("show with the body section");
    assert!(
        with.content
            .as_deref()
            .is_some_and(|b| b.contains(BODY_TEXT)),
        "the body section WAS requested, so `content` must carry the body: {:?}",
        with.content
    );
}

/// `open_meta` is populated only when the `open-meta` section is asked for.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn section_open_meta_absent_omits_open_meta(pool: PgPool) {
    let (profile, created) = seed_resource(&pool, "sections-open@example.com", "zz-sec-open").await;

    let without =
        substrate_read::show_view_select(&pool, profile, created.id, &SectionSet::default())
            .await
            .expect("show without the open-meta section");
    assert!(
        without.open_meta.is_none(),
        "the open-meta section was not requested, so `open_meta` must be absent: {:?}",
        without.open_meta
    );

    let with: SectionSet = [ResourceSection::OpenMeta].into_iter().collect();
    let with = substrate_read::show_view_select(&pool, profile, created.id, &with)
        .await
        .expect("show with the open-meta section");
    assert_eq!(
        with.open_meta
            .as_ref()
            .and_then(|m| m.get("marker"))
            .cloned(),
        Some(serde_json::json!("SECTIONS")),
        "the open-meta section WAS requested, so the open tier must be carried: {:?}",
        with.open_meta
    );
}

/// `managed_meta` is populated whatever the section set says — it is not a section.
///
/// This is the conjunct that makes dropping the hoisted `stage`/`mode`/`effort`/`seq` columns
/// lossless: a caller who asks for nothing still gets the workflow tier, one level down. Asserted
/// on **both** read paths, because they assemble their views separately and only one of them
/// carrying the managed tier is exactly the drift this collapse exists to end.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn managed_meta_present_regardless_of_sections(pool: PgPool) {
    let (profile, created) =
        seed_resource(&pool, "sections-managed@example.com", "zz-sec-managed").await;

    for sections in [
        SectionSet::default(),
        [ResourceSection::Body].into_iter().collect(),
        [ResourceSection::Body, ResourceSection::OpenMeta]
            .into_iter()
            .collect(),
    ] {
        let view = substrate_read::show_view_select(&pool, profile, created.id, &sections)
            .await
            .expect("show");
        assert_eq!(
            view.managed_meta.stage.as_deref(),
            Some("in-progress"),
            "managed_meta is not a section — it is present for {sections:?}"
        );
    }

    let page = substrate_read::list_views_select(
        &pool,
        profile,
        &ResourceListParams::default(),
        &SectionSet::default(),
    )
    .await
    .expect("list");
    let row = page
        .views
        .iter()
        .find(|v| v.id == created.id)
        .expect("the seeded resource is on the page");
    assert_eq!(
        row.managed_meta.stage.as_deref(),
        Some("in-progress"),
        "the list path carries the managed tier too, with no section asked for"
    );
}

/// The list page comes back in the order the page was cut in.
///
/// New logic this task introduces, so it carries its own witness. `readback::hit_identities`
/// answers `WHERE r.id = ANY($2)` — a SET, with no `ORDER BY` — so the sort
/// `filtered_visible_page` applied survives only because the page is reassembled by walking its
/// ids. A version that trusted the readback's row order would pass by luck and fail by planner.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn list_views_preserve_the_page_sort_order(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "sections-order@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    // Created in an order that is not the sorted order, so a correct result cannot be insertion
    // order either.
    for title in ["zz-order-c", "zz-order-a", "zz-order-b"] {
        backend
            .create_resource(CreateResource {
                idempotency_key: None,
                slug: title.to_string(),
                doctype: "task".to_string(),
                home: HomeAnchor::Context(ContextId::from(context)),
                title: title.to_string(),
                body: None,
                managed_meta: ManagedMeta::default(),
                open_meta: None,
                goal: None,
                origin_uri: Some(format!("test://{title}")),
                chunks_packed: None,
                content_hash: None,
                act: ActContext::default(),
                origin: Surface::ApiHttp,
            })
            .await
            .expect("create");
    }

    let page = substrate_read::list_views_select(
        &pool,
        ProfileId::from(profile),
        &ResourceListParams {
            // Scoped to the seeded context so the page is exactly these three: the visible set
            // otherwise also carries what the migration chain seeds publicly (the L0 kernel telos).
            context_ref: Some(context.to_string()),
            sort: Some(ResourceSortField::Title),
            order: Some(SortOrder::Asc),
            ..ResourceListParams::default()
        },
        &SectionSet::default(),
    )
    .await
    .expect("list sorted by title");

    let titles: Vec<&str> = page.views.iter().map(|v| v.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["zz-order-a", "zz-order-b", "zz-order-c"],
        "the page's ORDER BY must survive the set-shaped batched readback"
    );
}

/// `list_select` refuses `sections=body` at the door, and still serves `open-meta`.
///
/// **This is the server-side half, and it is the half that was missing.** The CLI's
/// `list_section_parser` has refused `body` on `list` since sections shipped, so the rule looked
/// enforced — but `list_select` parsed against `ResourceSection::ALL` and `fill_sections` filled
/// the body for anyone who reached it directly. MCP, raw HTTP and temper-rb all do. With `--all`
/// sending no limit, that was a per-row 2-3 statement body reconstruction over an unbounded page:
/// the last N+1 on the read path, reachable from every door except the one that refused it.
///
/// A surface-level test could not have caught this — the CLI was already correct. The witness has
/// to sit on `list_select`, which is what every door actually calls.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn list_select_refuses_the_body_section(pool: PgPool) {
    let (profile, created) =
        seed_resource(&pool, "sections-nobody@example.com", "zz-sec-nobody").await;

    let err = substrate_read::list_select(
        &pool,
        profile,
        ResourceListParams {
            sections: Some("body".to_string()),
            ..ResourceListParams::default()
        },
    )
    .await
    .expect_err("the list door refuses `body`");

    let msg = err.to_string();
    assert!(
        msg.contains("open-meta"),
        "the refusal names what this door accepts, so a caller recovers from the message \
         alone: {msg}"
    );

    // The narrowing is to `body`, not to sections. `open-meta` must still fill — otherwise this
    // change is a removal of the list door's one working section rather than a refusal of its
    // unbounded one.
    let page = substrate_read::list_select(
        &pool,
        profile,
        ResourceListParams {
            sections: Some("open-meta".to_string()),
            ..ResourceListParams::default()
        },
    )
    .await
    .expect("`open-meta` is still served on the list door");
    let row = page
        .rows
        .iter()
        .find(|v| v.id == created.id)
        .expect("the seeded resource is on the page");
    assert!(
        row.open_meta.is_some(),
        "`open-meta` asked for means the tier is filled, not merely accepted"
    );
    assert!(
        row.content.is_none(),
        "and no body rides along on a list row"
    );
}

/// `show_view_select` still fills the body — the door that legitimately serves it.
///
/// The complement. `list_select`'s narrowing is only correct if `show` is untouched: MCP's
/// `get_resource` reads its body through exactly this call, and a change that took the section
/// away from both doors would satisfy every assertion in the test above while breaking the read
/// that was never the problem.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn show_view_select_still_serves_the_body_section(pool: PgPool) {
    let (profile, created) =
        seed_resource(&pool, "sections-showbody@example.com", "zz-sec-showbody").await;

    let sections: SectionSet = [ResourceSection::Body].into_iter().collect();
    let view = substrate_read::show_view_select(&pool, profile, created.id, &sections)
        .await
        .expect("show serves the body section");
    assert!(
        view.content.is_some(),
        "`show --with body` must still reconstruct the body — this narrowing is per-door"
    );
}
