#![cfg(all(feature = "test-db", feature = "next-backend"))]

//! WS6 chunk 4c: write round-trip equivalence. The SAME create/update/delete command run through the
//! legacy `DbBackend` (→ `public.*`) and the `NextBackend` (→ `temper_next.*`) produces resources that
//! match at the §9 invariant floor (origin_uri / title / is_active / context_name / doc_type_name /
//! stage / mode / effort / seq). Non-invariants (re-minted ids, `slug`/hashes, timestamps, the
//! caller-relative `owner_handle`, manifest-vs-merkle `body_hash`) are excluded — see the 4b amendment.
//!
//! Backend-level (not HTTP): constructs both backends over one pool and compares their returned
//! `ResourceRow`s. Bodies are omitted (metadata-only) so the legacy path needs no embed pipeline; the
//! body-revise + body-parity paths are proven in temper-next's `write_path_mutations` artifact tests.
//!
//! Local-only: `cargo nextest run -p temper-e2e --features test-db,next-backend`.

mod common;

use temper_core::operations::{
    Backend, CreateResource, DeleteResource, ResourceRef, Surface, UpdateResource,
};
use temper_core::types::ids::ProfileId;
use temper_core::types::managed_meta::ManagedMeta;
use temper_core::types::resource::ResourceRow;

use temper_api::backend::{DbBackend, NextBackend};

const SEED_RESOURCE_ID: &str = "00000000-0000-0000-0099-000000000001";

/// Assert two rows match at the §9 invariant floor (the migration-invariant subset).
fn assert_floor(legacy: &ResourceRow, next: &ResourceRow) {
    assert_eq!(legacy.origin_uri, next.origin_uri, "origin_uri");
    assert_eq!(legacy.title, next.title, "title");
    assert_eq!(legacy.is_active, next.is_active, "is_active");
    assert_eq!(legacy.context_name, next.context_name, "context_name");
    assert_eq!(legacy.doc_type_name, next.doc_type_name, "doc_type_name");
    assert_eq!(legacy.stage, next.stage, "stage");
    assert_eq!(legacy.mode, next.mode, "mode");
    assert_eq!(legacy.effort, next.effort, "effort");
    assert_eq!(legacy.seq, next.seq, "seq");
}

fn create_cmd(origin_uri: &str) -> CreateResource {
    CreateResource {
        slug: "rt-doc".into(),
        doctype: "research".into(),
        context: "temper".into(),
        title: "RT Doc".into(),
        body: None,
        managed_meta: ManagedMeta {
            stage: Some("backlog".into()),
            mode: Some("build".into()),
            effort: Some("M".into()),
            ..Default::default()
        },
        open_meta: None,
        origin_uri: Some(origin_uri.into()),
        chunks_packed: None,
        content_hash: None,
        origin: Surface::CliCloud,
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_update_delete_roundtrip_next_equals_legacy(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let profile = ProfileId::from(uuid::Uuid::parse_str(common::SYSTEM_PROFILE_ID).unwrap());

    // Manifest for the seed resource so synthesis carries it (Pete = its owner = SYSTEM_PROFILE, who
    // the per-surface emitters bind to). Force the home context owner to SYSTEM_PROFILE so the
    // synthesized "temper" context is owned by the caller's synthesized profile (resolve_context match).
    sqlx::query(
        "INSERT INTO kb_resource_manifests (resource_id, managed_meta, open_meta) \
         VALUES ($1::uuid, '{}'::jsonb, '{}'::jsonb) ON CONFLICT (resource_id) DO NOTHING",
    )
    .bind(SEED_RESOURCE_ID)
    .execute(&app.pool)
    .await
    .expect("seed manifest");
    sqlx::query(
        "UPDATE kb_contexts SET kb_owner_table='kb_profiles', kb_owner_id=$1::uuid \
         WHERE id=$2::uuid",
    )
    .bind(common::SYSTEM_PROFILE_ID)
    .bind(common::TEMPER_CONTEXT_ID)
    .execute(&app.pool)
    .await
    .expect("own temper context");

    temper_next::synthesis::run(&app.pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect("synthesis::run");

    let legacy = DbBackend::new(app.pool.clone(), profile, "dev".into(), Surface::CliCloud);
    let next = NextBackend::new(app.pool.clone(), profile);

    // ── create ──
    let row_l = legacy
        .create_resource(create_cmd("test://rt-doc"))
        .await
        .expect("legacy create")
        .value;
    let row_n = next
        .create_resource(create_cmd("test://rt-doc"))
        .await
        .expect("next create")
        .value;
    assert_floor(&row_l, &row_n);
    assert_eq!(row_n.stage.as_deref(), Some("backlog"), "next create stage");
    assert_eq!(row_n.doc_type_name, "research", "next create doc_type");

    // ── update (title + stage), addressed by the public id (ResolvedIds maps to the next row) ──
    let upd = |id: temper_core::types::ids::ResourceId| UpdateResource {
        resource: ResourceRef::Uuid { id },
        body: None,
        managed_meta: Some(ManagedMeta {
            title: Some("RT Doc v2".into()),
            stage: Some("done".into()),
            ..Default::default()
        }),
        open_meta: None,
        move_to: None,
        origin: Surface::CliCloud,
    };
    let upd_l = legacy
        .update_resource(upd(row_l.id))
        .await
        .expect("legacy update")
        .value;
    let upd_n = next
        .update_resource(upd(row_l.id))
        .await
        .expect("next update")
        .value;
    assert_floor(&upd_l, &upd_n);
    assert_eq!(upd_n.title, "RT Doc v2", "next update title");
    assert_eq!(
        upd_n.stage.as_deref(),
        Some("done"),
        "next update stage superseded"
    );

    // ── delete (soft) ──
    let del = |id: temper_core::types::ids::ResourceId| DeleteResource {
        resource: ResourceRef::Uuid { id },
        force: false,
        origin: Surface::CliCloud,
    };
    // next first: it addresses the target by the public id via ResolvedIds, which filters is_active —
    // so resolve BEFORE legacy soft-deletes the public row. (Moot in the real gated world: one backend
    // is active at a time, so both deletes never run against one DB.)
    next.delete_resource(del(row_l.id))
        .await
        .expect("next delete");
    legacy
        .delete_resource(del(row_l.id))
        .await
        .expect("legacy delete");

    let legacy_active: bool =
        sqlx::query_scalar("SELECT is_active FROM public.kb_resources WHERE id=$1")
            .bind(*row_l.id)
            .fetch_one(&app.pool)
            .await
            .expect("legacy is_active");
    let next_active: bool =
        sqlx::query_scalar("SELECT is_active FROM temper_next.kb_resources WHERE id=$1")
            .bind(*row_n.id)
            .fetch_one(&app.pool)
            .await
            .expect("next is_active");
    assert!(!legacy_active, "legacy soft-deleted");
    assert!(!next_active, "next soft-deleted");
}

// ── relationship round-trip ─────────────────────────────────────────────────────

use temper_core::operations::{
    AssertRelationship, FoldRelationship, RetypeRelationship, ReweightRelationship,
};
use temper_core::types::graph::{EdgeKind, Polarity};

/// (edge_kind, polarity, label, weight, is_folded) for an edge — the state the §9 graph floor asserts.
type EdgeState = (String, String, Option<String>, f64, bool);

async fn legacy_edge(pool: &sqlx::PgPool, src: uuid::Uuid, tgt: uuid::Uuid) -> EdgeState {
    sqlx::query_as(
        "SELECT edge_kind::text, polarity::text, label, weight, is_folded \
         FROM public.kb_resource_edges \
         WHERE source_resource_id=$1 AND target_resource_id=$2 ORDER BY created DESC LIMIT 1",
    )
    .bind(src)
    .bind(tgt)
    .fetch_one(pool)
    .await
    .expect("legacy edge")
}

async fn next_edge(pool: &sqlx::PgPool, edge_id: uuid::Uuid) -> EdgeState {
    sqlx::query_as(
        "SELECT edge_kind::text, polarity::text, label, weight, is_folded \
         FROM temper_next.kb_edges WHERE id=$1",
    )
    .bind(edge_id)
    .fetch_one(pool)
    .await
    .expect("next edge")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn relationship_roundtrip_next_equals_legacy(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let profile = ProfileId::from(uuid::Uuid::parse_str(common::SYSTEM_PROFILE_ID).unwrap());
    sqlx::query(
        "UPDATE kb_contexts SET kb_owner_table='kb_profiles', kb_owner_id=$1::uuid WHERE id=$2::uuid",
    )
    .bind(common::SYSTEM_PROFILE_ID)
    .bind(common::TEMPER_CONTEXT_ID)
    .execute(&app.pool)
    .await
    .expect("own temper context");

    let legacy = DbBackend::new(app.pool.clone(), profile, "dev".into(), Surface::CliCloud);
    let next = NextBackend::new(app.pool.clone(), profile);

    // Two endpoint resources (slugged), created in public, then synthesized into temper_next.
    let mut a_cmd = create_cmd("test://edge-a");
    a_cmd.slug = "edge-a".into();
    a_cmd.title = "Edge A".into();
    let mut b_cmd = create_cmd("test://edge-b");
    b_cmd.slug = "edge-b".into();
    b_cmd.title = "Edge B".into();
    let a = legacy.create_resource(a_cmd).await.expect("create A").value;
    let b = legacy.create_resource(b_cmd).await.expect("create B").value;
    temper_next::synthesis::run(&app.pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect("synthesis::run");

    let assert_cmd = || AssertRelationship {
        source: ResourceRef::Uuid { id: a.id },
        target_slug: "edge-b".into(),
        edge_kind: EdgeKind::LeadsTo,
        polarity: Polarity::Forward,
        label: "operationalized_by".into(),
        weight: 1.0,
        origin: Surface::CliCloud,
    };

    // ── assert ──
    let corr_l = legacy
        .assert_relationship(assert_cmd())
        .await
        .expect("legacy assert")
        .value;
    let edge_n = next
        .assert_relationship(assert_cmd())
        .await
        .expect("next assert")
        .value;
    assert_eq!(
        legacy_edge(&app.pool, *a.id, *b.id).await,
        next_edge(&app.pool, edge_n).await,
        "edge state after assert"
    );

    // ── retype (kind LeadsTo → Contains) ──
    legacy
        .retype_relationship(RetypeRelationship {
            correlation_id: corr_l,
            edge_kind: EdgeKind::Contains,
            polarity: Polarity::Forward,
            origin: Surface::CliCloud,
        })
        .await
        .expect("legacy retype");
    next.retype_relationship(RetypeRelationship {
        correlation_id: edge_n,
        edge_kind: EdgeKind::Contains,
        polarity: Polarity::Forward,
        origin: Surface::CliCloud,
    })
    .await
    .expect("next retype");
    assert_eq!(
        legacy_edge(&app.pool, *a.id, *b.id).await,
        next_edge(&app.pool, edge_n).await,
        "edge state after retype"
    );

    // ── reweight ──
    legacy
        .reweight_relationship(ReweightRelationship {
            correlation_id: corr_l,
            weight: 2.5,
            origin: Surface::CliCloud,
        })
        .await
        .expect("legacy reweight");
    next.reweight_relationship(ReweightRelationship {
        correlation_id: edge_n,
        weight: 2.5,
        origin: Surface::CliCloud,
    })
    .await
    .expect("next reweight");
    assert_eq!(
        legacy_edge(&app.pool, *a.id, *b.id).await,
        next_edge(&app.pool, edge_n).await,
        "edge state after reweight"
    );

    // ── fold ──
    legacy
        .fold_relationship(FoldRelationship {
            correlation_id: corr_l,
            reason: None,
            origin: Surface::CliCloud,
        })
        .await
        .expect("legacy fold");
    next.fold_relationship(FoldRelationship {
        correlation_id: edge_n,
        reason: None,
        origin: Surface::CliCloud,
    })
    .await
    .expect("next fold");
    let (_, _, _, _, l_folded) = legacy_edge(&app.pool, *a.id, *b.id).await;
    let (_, _, _, _, n_folded) = next_edge(&app.pool, edge_n).await;
    assert!(l_folded, "legacy edge folded");
    assert!(n_folded, "next edge folded");
}
