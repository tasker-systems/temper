#![cfg(feature = "test-db")]
//! **F-1** — edge authorship authorizes both endpoints and the container the edge lands in.
//!
//! Before this, `assert`/`retype`/`reweight`/`fold` gated only `can_modify_resource(source)`. The
//! target endpoint was never authorized, so a caller who could modify A could attach A→B to any B
//! by id — including resources they cannot read. And the two goal-edge projections (create and
//! update) went through the shared assert helper, which gated nothing at all.
//!
//! The rule, decided 2026-07-25: **creating an edge requires write on the source side and read on
//! the target**, and because an edge is *homed* in a context or cogmap — the source side's home —
//! it requires write on that container too. Three clauses, all in
//! `assert_edge_from_source_home`:
//!
//!   1. `can_modify_resource(source)` — retained for the soft-delete floor it carries
//!   2. container-write on the edge's home
//!   3. `endpoint_readable_by_profile(target)`
//!
//! Clause 2 subsumes clause 1's non-tombstone arms, which is exactly why
//! `asserting_from_a_tombstoned_source_is_refused` exists: it is the one case that proves clause 1
//! is still load-bearing, and it would pass vacuously if the tombstone floor were dropped.
//!
//! The mutation verbs (retype/reweight/fold) carry clauses 1 and 2 via `check_edge_mutable`, so that
//! changing an edge is never easier than creating it.

mod common;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::graph;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, EdgeId, ProfileId, ResourceId};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{
    AssertRelationship, Backend, CreateResource, RetypeRelationship, Surface,
};
use temper_workflow::types::managed_meta::ManagedMeta;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn create_cmd(context: Uuid, slug: &str) -> CreateResource {
    CreateResource {
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home: HomeAnchor::Context(ContextId::from(context)),
        title: format!("F-1 {slug}"),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: Some(format!("test://f1-{slug}-{}", Uuid::new_v4())),
        chunks_packed: None,
        content_hash: None,
        goal: None,
        act: Default::default(),
        origin: Surface::ApiHttp,
    }
}

fn assert_cmd(src: Uuid, tgt: Uuid) -> AssertRelationship {
    AssertRelationship {
        source: ResourceId::from(src),
        target: ResourceId::from(tgt),
        edge_kind: graph::EdgeKind::LeadsTo,
        polarity: graph::Polarity::Forward,
        label: String::new(),
        weight: 1.0,
        act: Default::default(),
        origin: Surface::ApiHttp,
    }
}

/// A profile with its own context, plus one resource homed there.
async fn profile_with_resource(pool: &PgPool, tag: &str) -> (Uuid, Uuid, Uuid) {
    let email = format!("f1-{tag}-{}@example.com", Uuid::new_v4());
    let (profile, context) = common::fixtures::create_test_profile_with_context(pool, &email).await;
    let created = DbBackend::new(pool.clone(), ProfileId::from(profile))
        .create_resource(create_cmd(context, tag))
        .await
        .expect("create own resource");
    (profile, context, Uuid::from(created.value.id))
}

/// A direct profile-anchored WRITE grant on a single resource — the delegation shape that satisfies
/// `can_modify_resource` WITHOUT conferring any authority over the resource's home container.
async fn grant_resource_write(pool: &PgPool, resource: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, \
            can_read, can_write, can_delete, can_grant, granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_profiles', $2, true, true, false, false, $2)",
    )
    .bind(resource)
    .bind(profile)
    .execute(pool)
    .await
    .expect("seed a direct resource write grant");
}

// ─── clause 3: the target must be readable ───────────────────────────────────

/// The F-1 vector itself: a caller with full authority over A attaches A→B where B is a resource in
/// a stranger's private context. The positive control (a readable target, same caller, same source)
/// is what proves the refusal is the target check rather than anything about A.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn edge_to_an_unreadable_target_is_refused(pool: PgPool) {
    let (author, own_context, source) = profile_with_resource(&pool, "author").await;
    let (_stranger, _stranger_ctx, secret) = profile_with_resource(&pool, "stranger").await;

    // Non-vacuity: the author genuinely cannot see the stranger's resource.
    let visible: bool =
        sqlx::query_scalar("SELECT endpoint_readable_by_profile($1, 'kb_resources', $2)")
            .bind(author)
            .bind(secret)
            .fetch_one(&pool)
            .await
            .expect("endpoint predicate");
    assert!(!visible, "the stranger's resource must not be readable");

    let backend = DbBackend::new(pool.clone(), ProfileId::from(author));
    let denied = backend
        .assert_relationship(assert_cmd(source, secret))
        .await;
    assert!(
        matches!(denied, Err(TemperError::NotFound(_))),
        "an edge to an unreadable target must be refused as NotFound (never an existence oracle), \
         got {denied:?}"
    );

    // Positive control: a target the author CAN read, from the same source, succeeds.
    let readable = DbBackend::new(pool.clone(), ProfileId::from(author))
        .create_resource(create_cmd(own_context, "sibling"))
        .await
        .expect("create sibling");
    backend
        .assert_relationship(assert_cmd(source, Uuid::from(readable.value.id)))
        .await
        .expect("an edge to a readable target must succeed");
}

/// The goal-edge projection inherits clause 3. `--goal` takes a bare id that nothing else on the
/// create path authorizes, so before F-1 a caller could link their resource to a goal they cannot
/// see — and the resulting edge is visible to anyone who can see both ends.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn goal_edge_on_create_is_refused_for_an_unreadable_goal(pool: PgPool) {
    let email = format!("f1-goal-{}@example.com", Uuid::new_v4());
    let (author, own_context) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let (_stranger, _sctx, secret_goal) = profile_with_resource(&pool, "goalowner").await;

    let mut cmd = create_cmd(own_context, "with-goal");
    cmd.goal = Some(ResourceId::from(secret_goal));

    let denied = DbBackend::new(pool.clone(), ProfileId::from(author))
        .create_resource(cmd)
        .await;
    assert!(
        matches!(denied, Err(TemperError::NotFound(_))),
        "creating with a goal the caller cannot read must be refused, got {denied:?}"
    );
}

// ─── clause 2: container-write on the edge's home ────────────────────────────

/// A direct `can_write` grant on the source satisfies `can_modify_resource` but confers nothing over
/// the source's home. Under the decided rule an edge is an object homed in that container, so
/// authoring one requires container authority — the same rule F-2 established for placing a
/// resource. This is the case that distinguishes the new gate from the old one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn edge_from_a_source_without_container_write_is_refused(pool: PgPool) {
    let (_owner, owner_context, source) = profile_with_resource(&pool, "owner").await;
    let (delegate, _dctx, target) = profile_with_resource(&pool, "delegate").await;

    grant_resource_write(&pool, source, delegate).await;
    grant_resource_write(&pool, target, delegate).await;

    // Non-vacuity: the delegate really may modify the source, and really may NOT author its home.
    let can_modify: bool = sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(delegate)
        .bind(source)
        .fetch_one(&pool)
        .await
        .expect("modify predicate");
    let can_author: bool = sqlx::query_scalar("SELECT context_authorable_by_profile($1, $2)")
        .bind(delegate)
        .bind(owner_context)
        .fetch_one(&pool)
        .await
        .expect("authorable predicate");
    assert!(
        can_modify,
        "the delegate must satisfy the OLD gate — otherwise this test proves nothing new"
    );
    assert!(
        !can_author,
        "the delegate must not be authorable on the home"
    );

    let denied = DbBackend::new(pool.clone(), ProfileId::from(delegate))
        .assert_relationship(assert_cmd(source, target))
        .await;
    assert!(
        matches!(denied, Err(TemperError::Forbidden)),
        "asserting an edge into a container the caller cannot author must be Forbidden, got {denied:?}"
    );
}

// ─── clause 1: the tombstone floor container-write does NOT carry ────────────

/// Why clause 1 survives clause 2 subsuming it. `context_authorable_by_profile` says nothing about
/// whether the SOURCE is alive; only `can_modify_resource` carries the soft-delete WRITE floor. Drop
/// clause 1 as "redundant" and this is what breaks: edges sprouting from tombstones.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn asserting_from_a_tombstoned_source_is_refused(pool: PgPool) {
    let (author, own_context, source) = profile_with_resource(&pool, "tomb").await;
    let target = DbBackend::new(pool.clone(), ProfileId::from(author))
        .create_resource(create_cmd(own_context, "tomb-target"))
        .await
        .expect("create target");
    let target = Uuid::from(target.value.id);

    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(source)
        .execute(&pool)
        .await
        .expect("tombstone the source");

    // The author still authors the home — so ONLY clause 1 can refuse this.
    let can_author: bool = sqlx::query_scalar("SELECT context_authorable_by_profile($1, $2)")
        .bind(author)
        .bind(own_context)
        .fetch_one(&pool)
        .await
        .expect("authorable predicate");
    assert!(
        can_author,
        "the author must still author the home, so the refusal below isolates the tombstone floor"
    );

    let denied = DbBackend::new(pool.clone(), ProfileId::from(author))
        .assert_relationship(assert_cmd(source, target))
        .await;
    assert!(
        matches!(denied, Err(TemperError::Forbidden)),
        "an edge out of a soft-deleted source must be refused, got {denied:?}"
    );
}

// ─── mutation verbs are no weaker than creation ──────────────────────────────

/// Retyping an edge must not be easier than asserting it. Same delegate as the container-write case:
/// they may modify the source, may not author the home, and so may neither create the edge nor
/// change the meaning of one that exists.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retype_requires_container_write_like_assert(pool: PgPool) {
    let (owner, owner_context, source) = profile_with_resource(&pool, "retype-owner").await;
    let target = DbBackend::new(pool.clone(), ProfileId::from(owner))
        .create_resource(create_cmd(owner_context, "retype-target"))
        .await
        .expect("create target");
    let target = Uuid::from(target.value.id);

    // The owner asserts the edge legitimately.
    let edge: EdgeId = DbBackend::new(pool.clone(), ProfileId::from(owner))
        .assert_relationship(assert_cmd(source, target))
        .await
        .expect("owner asserts the edge")
        .value;

    // A delegate with resource-level write on the source, but no authority over the home.
    let demail = format!("f1-retyper-{}@example.com", Uuid::new_v4());
    let delegate = common::fixtures::create_test_profile(&pool, &demail).await;
    grant_resource_write(&pool, source, delegate).await;

    let denied = DbBackend::new(pool.clone(), ProfileId::from(delegate))
        .retype_relationship(RetypeRelationship {
            edge_handle: edge,
            edge_kind: graph::EdgeKind::Near,
            polarity: graph::Polarity::Forward,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await;
    assert!(
        matches!(denied, Err(TemperError::Forbidden)),
        "retype must carry the same container-write clause as assert, got {denied:?}"
    );

    // Positive control: the owner, who authors the home, may retype.
    DbBackend::new(pool.clone(), ProfileId::from(owner))
        .retype_relationship(RetypeRelationship {
            edge_handle: edge,
            edge_kind: graph::EdgeKind::Near,
            polarity: graph::Polarity::Forward,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("the home's author may retype");
}
