#![cfg(feature = "artifact-tests")]
//! Edge-owned properties — the write path for `kb_properties.owner_table = 'kb_edges'`.
//!
//! `migrations/20260624000001_canonical_schema.sql:656` has admitted an edge owner since the
//! canonical schema ("§4a edges carry facets"), and nothing ever wrote one: production on
//! 2026-07-27 held 10,692 resource-owned properties, 37 block-owned, and **zero** edge-owned.
//! Migration `20260727000030` builds the path.
//!
//! These tests pin the three things that were genuinely unknown because the path had never run,
//! rather than the things that merely looked risky:
//!
//! 1. **Where the event anchors.** The blocker was never the projector — it was that both
//!    wrappers resolved the anchor from `kb_resource_homes`, treating the owner id as a resource
//!    id. An edge has no row there.
//! 2. **What folding an edge does to the properties it owns.** Measured before the migration:
//!    nothing. `edge_folded = t, property_folded = f` — a live citation on a retracted link.
//! 3. **Whether replay reproduces an edge-owned property**, given the property's owner must exist
//!    before the property row. Ordering between `relationship_asserted` and `property_asserted`
//!    was unverified for this owner kind.
//!
//! Harness + seeding helpers follow the per-file convention of this suite (`citation_audits.rs`,
//! `evidential_standing.rs`, `write_path_mutations.rs` each carry their own copy).

mod common;

use temper_core::types::property_owner::PropertyOwner;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, EdgeId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

// ── fixtures ──────────────────────────────────────────────────────────────────────────────────

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

async fn make_home(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

async fn make_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    title: &str,
    uri: &str,
) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: uri,
            body: "seed body",
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

/// Assert a `leads_to`/`advances` edge — the real relation the clause citation will qualify.
async fn assert_edge(
    pool: &sqlx::PgPool,
    src: ResourceId,
    tgt: ResourceId,
    home: ContextId,
    emitter: EntityId,
) -> EdgeId {
    let mut tx = pool.begin().await.unwrap();
    let id = fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src,
            tgt,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("advances"),
            weight: 1.0,
            home: EdgeHome::Context(home),
            emitter,
        },
    )
    .await
    .unwrap()
    .relationship()
    .unwrap();
    tx.commit().await.unwrap();
    id
}

/// The live (non-folded) properties an edge owns, as `(key, value)` in key order.
async fn edge_properties(pool: &sqlx::PgPool, edge: EdgeId) -> Vec<(String, serde_json::Value)> {
    sqlx::query_as(
        "SELECT property_key, property_value FROM kb_properties
          WHERE owner_table='kb_edges' AND owner_id=$1 AND NOT is_folded
          ORDER BY property_key",
    )
    .bind(edge.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// A world with one `advances` edge between two resources, homed in a context.
async fn world(pool: &sqlx::PgPool, slug: &str) -> (EntityId, ContextId, EdgeId) {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let home = make_home(pool, owner, slug).await;
    let task = make_resource(
        pool,
        owner,
        emitter,
        home,
        "a task",
        &format!("kb://{slug}/task"),
    )
    .await;
    let goal = make_resource(
        pool,
        owner,
        emitter,
        home,
        "a goal",
        &format!("kb://{slug}/goal"),
    )
    .await;
    let edge = assert_edge(pool, task, goal, home, emitter).await;
    (emitter, home, edge)
}

// ── 1. the write path ─────────────────────────────────────────────────────────────────────────

/// A facet can be set on an edge, and the property row records the edge as its owner.
///
/// This is the whole point of the migration: before it, `facet_set` raised
/// `"resource <edge-id> has no home to anchor the property event"` because it looked the owner up
/// in `kb_resource_homes`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_facet_can_be_set_on_an_edge(pool: sqlx::PgPool) {
    let (emitter, _home, edge) = world(&pool, "edge-facet-write").await;

    let values = serde_json::json!({ "clause": "decomposition-preserves-rigor" });
    writes::set_facet(&pool, PropertyOwner::edge(edge), &values, 1.0, emitter)
        .await
        .expect("a facet can be set on an edge");

    let props = edge_properties(&pool, edge).await;
    assert_eq!(
        props,
        vec![("facet".to_string(), values)],
        "the edge owns exactly the facet just written"
    );
}

/// The event anchors on the **edge's own home**, not on either endpoint's home.
///
/// An edge carries `home_anchor_table`/`home_anchor_id`, and `relationship_fold` already anchors
/// its event from exactly those columns. If a future change reverted to resolving the anchor from
/// an endpoint resource, this catches it: the two coincide in the simple case, so the assertion is
/// written against the edge row's own columns rather than against a hardcoded id.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_event_anchors_on_the_edges_own_home(pool: sqlx::PgPool) {
    let (emitter, _home, edge) = world(&pool, "edge-facet-anchor").await;

    writes::set_facet(
        &pool,
        PropertyOwner::edge(edge),
        &serde_json::json!({ "clause": "witnesses-discriminate" }),
        1.0,
        emitter,
    )
    .await
    .unwrap();

    let (event_anchor_table, event_anchor_id, edge_home_table, edge_home_id): (
        String,
        Uuid,
        String,
        Uuid,
    ) = sqlx::query_as(
        "SELECT ev.producing_anchor_table, ev.producing_anchor_id,
                e.home_anchor_table, e.home_anchor_id
           FROM kb_properties p
           JOIN kb_events ev ON ev.id = p.asserted_by_event_id
           JOIN kb_edges e ON e.id = p.owner_id
          WHERE p.owner_table = 'kb_edges' AND p.owner_id = $1",
    )
    .bind(edge.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        (event_anchor_table, event_anchor_id),
        (edge_home_table, edge_home_id),
        "the property event must anchor on the edge's own home"
    );
}

/// A resource-owned facet is unaffected — the anchor rule's `kb_resources` arm is the incumbent
/// path, carried verbatim. Without this, a regression in the shared resolver would only surface in
/// the edge tests, and the 10,692 rows that matter would break silently.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_resource_owned_facet_still_works(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "resource-facet-regression").await;
    let r = make_resource(&pool, owner, emitter, home, "a thing", "kb://rfr/thing").await;

    let values = serde_json::json!({ "layer": "concept" });
    writes::set_facet(&pool, PropertyOwner::resource(r), &values, 1.0, emitter)
        .await
        .expect("the incumbent resource path is unchanged");

    let stored: serde_json::Value = sqlx::query_scalar(
        "SELECT property_value FROM kb_properties
          WHERE owner_table='kb_resources' AND owner_id=$1 AND property_key='facet'
            AND NOT is_folded",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, values);
}

/// An owner kind with no write path refuses, rather than silently anchoring nowhere.
///
/// `kb_cogmaps` is admitted by the `kb_properties` CHECK but has never had a property and has no
/// caller. The resolver raises instead of carrying a speculative branch.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_owner_kind_with_no_write_path_refuses(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "unsupported-owner").await;

    let payload = serde_json::json!({
        "property_id": Uuid::now_v7(),
        "owner": { "table": "kb_cogmaps", "id": home.uuid() },
        "property_key": "facet",
        "value": { "x": 1 },
        "weight": 1.0,
    });
    let err = sqlx::query_scalar::<_, Uuid>("SELECT facet_set($1,$2)")
        .bind(&payload)
        .bind(emitter.uuid())
        .fetch_one(&pool)
        .await
        .expect_err("a cogmap owner has no anchor rule");

    let msg = err.to_string();
    assert!(
        msg.contains("has no anchor rule"),
        "the refusal must name the missing rule, not fail obscurely: {msg}"
    );
}

// ── 2. fold semantics ─────────────────────────────────────────────────────────────────────────

/// Folding an edge folds the properties it owns.
///
/// **This test bites against the pre-migration behaviour**, which was measured, not assumed:
/// `_project_relationship_folded` updated `kb_edges` only, leaving `edge_folded = t` beside
/// `property_folded = f`. A clause citation surviving the retraction of the link it qualifies is
/// exactly the divergence edge-owned facets exist to make unrepresentable.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn folding_an_edge_folds_the_properties_it_owns(pool: sqlx::PgPool) {
    let (emitter, _home, edge) = world(&pool, "edge-facet-fold").await;

    writes::set_facet(
        &pool,
        PropertyOwner::edge(edge),
        &serde_json::json!({ "clause": "no-clause-is-uncovered-silently" }),
        1.0,
        emitter,
    )
    .await
    .unwrap();
    assert_eq!(
        edge_properties(&pool, edge).await.len(),
        1,
        "precondition: the edge owns a live property before the fold"
    );

    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::RelationshipFold {
            edge,
            reason: Some("the task no longer advances this goal"),
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    assert!(
        edge_properties(&pool, edge).await.is_empty(),
        "a folded edge must not leave live properties behind"
    );
}

/// The cascade retains history rather than deleting it, and attributes the retirement to the fold.
///
/// `is_folded` IS the retention mechanism — this is what makes the cascade safe to prefer over
/// making every reader join `kb_edges`. The folded row must still exist, and its `last_event_id`
/// must point at the fold event so the trail records which act retired it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_cascade_retains_history_and_attributes_the_fold(pool: sqlx::PgPool) {
    let (emitter, _home, edge) = world(&pool, "edge-facet-fold-history").await;

    writes::set_facet(
        &pool,
        PropertyOwner::edge(edge),
        &serde_json::json!({ "clause": "witnesses-discriminate" }),
        1.0,
        emitter,
    )
    .await
    .unwrap();

    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::RelationshipFold {
            edge,
            reason: None,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (is_folded, shares_last_event): (bool, bool) = sqlx::query_as(
        "SELECT p.is_folded, (p.last_event_id = e.last_event_id)
           FROM kb_properties p JOIN kb_edges e ON e.id = p.owner_id
          WHERE p.owner_table='kb_edges' AND p.owner_id=$1",
    )
    .bind(edge.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(is_folded, "the row is retained, folded — not deleted");
    assert!(
        shares_last_event,
        "the fold event must be recorded as what retired the property"
    );
}

// ── 3. replay ─────────────────────────────────────────────────────────────────────────────────

/// Projecting the property event does not depend on the owning edge already existing.
///
/// This is the ordering question the task raised — *"does replay reproject an edge-owned property
/// correctly, given the property's owner must exist before the property row?"* — asked in the form
/// that can actually fail. The answer is that the premise does not hold: `kb_properties.owner_id`
/// is **polymorphic and therefore carries no foreign key** (the only FKs on the table are
/// `asserted_by_event_id` and `last_event_id`), so the insert is order-independent by construction.
///
/// The test drives the projector with an owner id that does not exist as an edge at all — a
/// stronger condition than any replay ordering could produce. If someone later adds a real FK, or
/// a trigger validating the owner, this fails and tells them the ordering assumption changed.
///
/// **The corollary is the important half**: since nothing structural binds a property to its
/// owner's existence, the fold cascade is the *only* thing keeping an edge's facets from
/// outliving it. That is why the cascade is a projector change and not a reader convention.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn projecting_a_property_does_not_require_its_owner_to_exist(pool: sqlx::PgPool) {
    let (emitter, home, _edge) = world(&pool, "edge-facet-ordering").await;
    let orphan_edge = Uuid::now_v7();

    let payload = serde_json::json!({
        "property_id": Uuid::now_v7(),
        "owner": { "table": "kb_edges", "id": orphan_edge },
        "property_key": "clause",
        "value": "decomposition-preserves-rigor",
        "weight": 1.0,
    });
    // The event is appended against a real anchor (the projector takes the anchor as given; only
    // the *wrapper* resolves it from the owner, and that path is covered above).
    let event_id: Uuid =
        sqlx::query_scalar("SELECT _event_append('property_asserted', $1, 'kb_contexts', $2, $3)")
            .bind(emitter.uuid())
            .bind(home.uuid())
            .bind(&payload)
            .fetch_one(&pool)
            .await
            .unwrap();

    // `uuid[]`, not `uuid`: a facet assert writes one row per inner key, so the projector returns
    // every id it wrote (migration 20260730000010). This payload's key is `clause`, not `facet`, so
    // it still takes the append arm and the array holds exactly one id.
    sqlx::query_scalar::<_, Vec<Uuid>>("SELECT _project_property_asserted($1,$2)")
        .bind(event_id)
        .bind(&payload)
        .fetch_one(&pool)
        .await
        .expect("no FK on owner_id — projection is order-independent");

    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties WHERE owner_table='kb_edges' AND owner_id=$1",
    )
    .bind(orphan_edge)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, 1);
}

/// An edge-owned property is inside the replay-equivalence fingerprint, not beside it.
///
/// `replay.rs` fingerprints `kb_properties` ordered by `(owner_table, owner_id, property_key,
/// property_value)` with the surrogate id masked. That ordering already spans every owner kind, so
/// an edge-owned row participates in the existing equivalence proof rather than needing a new one —
/// but nothing asserted that, and a fingerprint narrowed to `owner_table = 'kb_resources'` would
/// silently stop covering these rows. This pins it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_owned_property_is_inside_the_replay_fingerprint(pool: sqlx::PgPool) {
    let (emitter, _home, edge) = world(&pool, "edge-facet-fingerprint").await;

    let properties_dump = |pool: sqlx::PgPool| async move {
        temper_substrate::replay::dump_projections(&pool)
            .await
            .unwrap()
            .into_iter()
            .find(|(table, _)| table == "kb_properties")
            .expect("kb_properties is a replay-diffed projection")
            .1
    };

    let before = properties_dump(pool.clone()).await;
    writes::set_facet(
        &pool,
        PropertyOwner::edge(edge),
        &serde_json::json!({ "clause": "witnesses-discriminate" }),
        1.0,
        emitter,
    )
    .await
    .unwrap();
    let after = properties_dump(pool.clone()).await;

    assert_ne!(
        before, after,
        "writing an edge-owned property must move the kb_properties replay dump — if it does not, \
         replay equivalence is blind to these rows"
    );
    assert!(
        after.to_string().contains("kb_edges"),
        "the dump must carry the edge owner_table, not just a value delta: {after}"
    );
}
