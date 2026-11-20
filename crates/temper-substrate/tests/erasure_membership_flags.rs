#![cfg(feature = "test-db")]
//! Membership implies the flag (erasure estate-membership spec D7.3; migrations
//! 20261201000010_erasure_records_schema.sql and 20261201000030_erasure_record_projectors.sql).
//!
//! [`VIOLATIONS`] is the check. It returns one row per violation of five conjuncts, each over the
//! whole database (every `sqlx::test` gets its own):
//!
//!   * `item_missing`: a `kb_erasure_members` row, of any member type and any status, whose item
//!     is not a row of its table. The three flag conjuncts below join to the item, so without this
//!     one a member whose item does not exist would satisfy them by vanishing from the join;
//!   * `resource_flag`: a row of `kb_resources` whose resource is active;
//!   * `context_flag`: a row of `kb_contexts` whose context is active;
//!   * `edge_flag`: a row of `kb_edges` whose edge is not folded;
//!   * `erased_has_row`: a resource with `erased_at` set and no `erased` member row.
//!
//! Not the reverse: ordinary deletes, retirements and folds set the same flags with no member row.
//! Cogmaps carry no conjunct: `kb_cogmaps` has no `is_active` column, so there is no flag to check.
//!
//! The world maps a context, a resource homed in it and an estate-homed edge through an appended
//! `erasure_declared`, and erases one more resource through the resource act, which records its
//! legacy erasure. The check finds nothing there. Each probe then breaks one conjunct alone, by a
//! write no projector makes (a member row inserted without its flag, or `erased_at` set with no
//! record; no trigger refuses either), and asserts the check reports exactly
//! that conjunct on exactly that item.

mod common;

use sqlx::PgPool;
use temper_core::types::ids::EntityId;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EdgeId, ProfileId, ResourceId};
use temper_substrate::payloads::{
    AnchorRef, EdgePolarity, ErasureDeclared, ErasureKind, ErasureMapEntry, ErasureMemberRef,
    ErasureMemberTable,
};
use temper_substrate::writes::{self, AssertParams, CreateParams};
use uuid::Uuid;

/// A manifest digest of the right shape. The projectors never interpret it.
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Every violation of D7.3, as (conjunct, item id). Empty when membership implies the flag.
const VIOLATIONS: &str = "
    SELECT 'item_missing'::text AS conjunct, m.member_id AS item
      FROM kb_erasure_members m
     WHERE (CASE m.member_table
                WHEN 'kb_resources' THEN EXISTS (SELECT 1 FROM kb_resources x WHERE x.id = m.member_id)
                WHEN 'kb_contexts'  THEN EXISTS (SELECT 1 FROM kb_contexts x  WHERE x.id = m.member_id)
                WHEN 'kb_cogmaps'   THEN EXISTS (SELECT 1 FROM kb_cogmaps x   WHERE x.id = m.member_id)
                WHEN 'kb_blobs'     THEN EXISTS (SELECT 1 FROM kb_blobs x     WHERE x.id = m.member_id)
                WHEN 'kb_edges'     THEN EXISTS (SELECT 1 FROM kb_edges x     WHERE x.id = m.member_id)
            END) IS NOT TRUE
    UNION ALL
    SELECT 'resource_flag', m.member_id
      FROM kb_erasure_members m
      JOIN kb_resources r ON r.id = m.member_id
     WHERE m.member_table = 'kb_resources' AND r.is_active
    UNION ALL
    SELECT 'context_flag', m.member_id
      FROM kb_erasure_members m
      JOIN kb_contexts c ON c.id = m.member_id
     WHERE m.member_table = 'kb_contexts' AND c.is_active
    UNION ALL
    SELECT 'edge_flag', m.member_id
      FROM kb_erasure_members m
      JOIN kb_edges e ON e.id = m.member_id
     WHERE m.member_table = 'kb_edges' AND NOT e.is_folded
    UNION ALL
    SELECT 'erased_has_row', r.id
      FROM kb_resources r
     WHERE r.erased_at IS NOT NULL
       AND NOT EXISTS (SELECT 1 FROM kb_erasure_members m
                        WHERE m.member_table = 'kb_resources'
                          AND m.member_id = r.id
                          AND m.status = 'erased')
     ORDER BY 1, 2";

async fn violations(pool: &PgPool) -> Vec<(String, Uuid)> {
    sqlx::query_as(VIOLATIONS)
        .fetch_all(pool)
        .await
        .expect("the D7.3 check runs")
}

// ── the world ────────────────────────────────────────────────────────────────────────────────

async fn fresh(pool: &PgPool) {
    common::reset_schema(pool).await;
    temper_substrate::scenario::bootseed::seed_system(pool)
        .await
        .unwrap();
}

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: EntityId =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), entity)
}

async fn make_context(pool: &PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

async fn make_resource(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    slug: &str,
) -> ResourceId {
    let origin = format!("test://erasure-membership-flags/{slug}");
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: slug,
            origin_uri: &origin,
            body: "body",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap()
}

async fn make_edge(
    pool: &PgPool,
    emitter: EntityId,
    home: ContextId,
    src: ResourceId,
    tgt: ResourceId,
    label: &str,
) -> EdgeId {
    writes::assert_relationship(
        pool,
        AssertParams {
            src,
            tgt,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some(label),
            weight: 1.0,
            home,
            emitter,
        },
    )
    .await
    .unwrap()
}

/// Append an `erasure_declared` under the system actor and project it in the same transaction,
/// as the live path does.
async fn declare(pool: &PgPool, d: &ErasureDeclared) -> Uuid {
    let payload = serde_json::to_value(d).unwrap();
    let (_, emitter) = system_actor(pool).await;
    let mut tx = pool.begin().await.unwrap();
    let event: Uuid = sqlx::query_scalar(
        "SELECT _event_append('erasure_declared', $1, NULL, NULL, $2, p_correlation => $3)",
    )
    .bind(emitter)
    .bind(&payload)
    .bind(d.erasure_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query("SELECT _project_erasure_declared($1, $2)")
        .bind(event)
        .bind(&payload)
        .execute(&mut *tx)
        .await
        .expect("erasure_declared projects");
    tx.commit().await.unwrap();
    event
}

fn pending(
    table: ErasureMemberTable,
    id: Uuid,
    parent: Option<ErasureMemberRef>,
) -> ErasureMapEntry {
    ErasureMapEntry {
        table,
        id,
        parent,
        prior_erasure: None,
        prior_event: None,
    }
}

/// The items of the world. The `free_*` items are in no erasure and carry no flag; each probe
/// maps one of them by hand.
struct World {
    erasure: Uuid,
    context: ContextId,
    resource: ResourceId,
    edge: EdgeId,
    act_erased: ResourceId,
    free_context: ContextId,
    free_resource: ResourceId,
    free_edge: EdgeId,
}

/// A selection erasure mapping a context, a resource homed in it and an edge homed in another
/// context, appended and projected; and one more resource erased by the resource act.
async fn world(pool: &PgPool) -> World {
    fresh(pool).await;
    let (owner, emitter) = system_actor(pool).await;
    let home = make_context(pool, owner, "flags-home").await;
    let context = make_context(pool, owner, "flags-container").await;
    let free_context = make_context(pool, owner, "flags-free-context").await;
    let resource = make_resource(pool, owner, emitter, context, "flags-member").await;
    let src = make_resource(pool, owner, emitter, home, "flags-src").await;
    let tgt = make_resource(pool, owner, emitter, home, "flags-tgt").await;
    let free_resource = make_resource(pool, owner, emitter, home, "flags-free-resource").await;
    let act_erased = make_resource(pool, owner, emitter, home, "flags-act-erased").await;
    let edge = make_edge(pool, emitter, home, src, tgt, "mapped").await;
    let free_edge = make_edge(pool, emitter, home, tgt, src, "free").await;

    let erasure = Uuid::now_v7();
    let parent = ErasureMemberRef {
        table: ErasureMemberTable::KbContexts,
        id: context.uuid(),
    };
    declare(
        pool,
        &ErasureDeclared {
            erasure_id: erasure,
            kind: ErasureKind::Selection,
            subject_table: None,
            subject_id: None,
            request_ref: Some(Uuid::now_v7()),
            manifest_version: Some("1.0.0".to_owned()),
            manifest_digest: Some(DIGEST.to_owned()),
            map: vec![
                pending(ErasureMemberTable::KbContexts, context.uuid(), None),
                pending(
                    ErasureMemberTable::KbResources,
                    resource.uuid(),
                    Some(parent),
                ),
                pending(ErasureMemberTable::KbEdges, edge.uuid(), None),
            ],
            counts: None,
            checksum: None,
            holders: vec![],
            legacy: false,
        },
    )
    .await;

    let (_, operator) = system_actor(pool).await;
    sqlx::query("SELECT resource_erasure_execute($1, $2, $3, $4)")
        .bind(act_erased.uuid())
        .bind(operator)
        .bind(operator)
        .bind(Uuid::now_v7())
        .execute(pool)
        .await
        .expect("the resource act erases");

    World {
        erasure,
        context,
        resource,
        edge,
        act_erased,
        free_context,
        free_resource,
        free_edge,
    }
}

/// Map an item into the world's erasure by hand, as no projector does: a pending row, its flag
/// left as it stands.
async fn map_without_flag(pool: &PgPool, erasure: Uuid, table: &str, id: Uuid) {
    sqlx::query(
        "INSERT INTO kb_erasure_members (erasure_id, member_table, member_id) VALUES ($1, $2, $3)",
    )
    .bind(erasure)
    .bind(table)
    .bind(id)
    .execute(pool)
    .await
    .expect("a member row inserts without its flag");
}

fn only(conjunct: &str, item: Uuid) -> Vec<(String, Uuid)> {
    vec![(conjunct.to_owned(), item)]
}

// ── the witness ──────────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn every_member_carries_its_flag_and_every_erased_resource_has_an_erased_row(pool: PgPool) {
    let w = world(&pool).await;

    // The check is not vacuous: every conjunct has an item to hold.
    let mapped: Vec<(String, Uuid)> = sqlx::query_as(
        "SELECT member_table, member_id FROM kb_erasure_members ORDER BY member_table, member_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for (table, id) in [
        ("kb_contexts", w.context.uuid()),
        ("kb_resources", w.resource.uuid()),
        ("kb_edges", w.edge.uuid()),
        ("kb_resources", w.act_erased.uuid()),
    ] {
        assert!(
            mapped.contains(&(table.to_owned(), id)),
            "setup: {table} {id} is mapped (mapped: {mapped:?})"
        );
    }
    let erased: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM kb_resources WHERE id = $1 AND erased_at IS NOT NULL")
            .bind(w.act_erased.uuid())
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(
        erased,
        Some(w.act_erased.uuid()),
        "setup: the act erased its resource"
    );

    assert_eq!(violations(&pool).await, Vec::<(String, Uuid)>::new());
}

// ── the probes: one conjunct each ────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_resource_member_left_active_is_reported(pool: PgPool) {
    let w = world(&pool).await;
    map_without_flag(&pool, w.erasure, "kb_resources", w.free_resource.uuid()).await;
    assert_eq!(
        violations(&pool).await,
        only("resource_flag", w.free_resource.uuid())
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_context_member_left_active_is_reported(pool: PgPool) {
    let w = world(&pool).await;
    map_without_flag(&pool, w.erasure, "kb_contexts", w.free_context.uuid()).await;
    assert_eq!(
        violations(&pool).await,
        only("context_flag", w.free_context.uuid())
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_member_left_unfolded_is_reported(pool: PgPool) {
    let w = world(&pool).await;
    map_without_flag(&pool, w.erasure, "kb_edges", w.free_edge.uuid()).await;
    assert_eq!(
        violations(&pool).await,
        only("edge_flag", w.free_edge.uuid())
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_erased_resource_with_no_erased_row_is_reported(pool: PgPool) {
    let w = world(&pool).await;
    // Inactive as well, so only the erased-row conjunct can see it: the resource is in no map.
    sqlx::query("UPDATE kb_resources SET erased_at = now(), is_active = false WHERE id = $1")
        .bind(w.free_resource.uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        violations(&pool).await,
        only("erased_has_row", w.free_resource.uuid())
    );
}

/// A member whose item does not exist is reported, for each of the five member types, whatever
/// its status. Each type is mapped in turn, so each step's report adds exactly that member.
///
/// FAILS IF: a member type's missing item is not reported (the flag conjuncts' joins drop it), or
/// is reported under any other conjunct.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_member_whose_item_is_missing_is_reported_for_every_member_type(pool: PgPool) {
    let w = world(&pool).await;
    let mut missing: Vec<Uuid> = Vec::new();
    for table in [
        "kb_resources",
        "kb_contexts",
        "kb_cogmaps",
        "kb_blobs",
        "kb_edges",
    ] {
        let id = Uuid::now_v7();
        map_without_flag(&pool, w.erasure, table, id).await;
        missing.push(id);
        missing.sort();
        assert_eq!(
            violations(&pool).await,
            missing
                .iter()
                .map(|id| ("item_missing".to_owned(), *id))
                .collect::<Vec<_>>(),
            "after mapping a missing {table} item"
        );
    }
}
