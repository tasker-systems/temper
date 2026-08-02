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
//!
//! **Amended 2026-07-27 — `check_edge_mutable` now carries clause 3 and a tombstone floor too.**
//! It omitted the target clause by design, on the reasoning that retract/reweight "discloses nothing
//! new about the target". That was true of those verbs and stopped being true when `set_facet`
//! joined them: a facet is caller-authored content that other principals read as authoritative, so
//! a caller with source-write and container-write but no read on the target could author a
//! qualifier on a link into a resource they cannot see. An adversarial pass measured it on a live
//! database — one principal, one edge, `GET` 404 and `POST` 200. The same pass found no `is_folded`
//! floor, so a facet could be written to an already-retracted edge and land where no read surface
//! could see it.
//!
//! Both are fixed in `check_edge_mutable` rather than in the facet verb alone, so all four verbs
//! answer to one definition — the gate's own reason for existing.

mod common;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::graph;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, EdgeId, ProfileId, ResourceId};
use temper_core::types::property_owner::PropertyOwner;
use temper_services::backend::DbBackend;
use temper_workflow::operations::{
    AssertRelationship, Backend, CreateResource, FoldRelationship, RetypeRelationship, SetFacet,
    Surface,
};
use temper_workflow::types::managed_meta::ManagedMeta;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn create_cmd(context: Uuid, slug: &str) -> CreateResource {
    CreateResource {
        resource_id: None,
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

/// A direct profile-anchored WRITE grant on a CONTEXT — satisfies `context_authorable_by_profile`
/// via its `profile_explicit_grant` arm, without making the grantee owner or team member. The
/// container-side twin of [`grant_resource_write`].
async fn grant_context_write(pool: &PgPool, context: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, \
            can_read, can_write, can_delete, can_grant, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, true, false, false, $2)",
    )
    .bind(context)
    .bind(profile)
    .execute(pool)
    .await
    .expect("seed a direct context write grant");
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

// ─── clause 3 on the MUTATION verbs, and the tombstone floor ─────────────────

/// **The vector an adversarial pass measured on a live database, 2026-07-27.**
///
/// A delegate holds write on the edge's source *and* on its home container — clauses 1 and 2, which
/// is everything `check_edge_mutable` used to ask. The edge's TARGET sits in the owner's private
/// context, which the delegate cannot read. Before clause 3 reached the mutation gate, that
/// delegate could write a facet onto the link: caller-authored JSON, on an edge pointing at a
/// resource they have no standing to see, which the target's owner then reads back as authoritative.
///
/// The positive control matters as much as the refusal. The same delegate, same source, same home,
/// with a target it *can* read, must succeed — otherwise this test would pass just as well if
/// clauses 1 or 2 were what refused, and it would be pinning the wrong thing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn faceting_an_edge_requires_reading_its_target(pool: PgPool) {
    let (owner, owner_context, source) = profile_with_resource(&pool, "facet-tgt-owner").await;

    // A second, PRIVATE context of the owner's, holding the unreadable target.
    let private_email = format!("f1-facet-priv-{}@example.com", Uuid::new_v4());
    let (_p2, private_context) =
        common::fixtures::create_test_profile_with_context(&pool, &private_email).await;
    let hidden_target = DbBackend::new(pool.clone(), ProfileId::from(_p2))
        .create_resource(create_cmd(private_context, "facet-hidden-target"))
        .await
        .expect("create the hidden target")
        .value
        .id;
    let hidden_target = Uuid::from(hidden_target);

    // A readable target, for the positive control.
    let open_target = DbBackend::new(pool.clone(), ProfileId::from(owner))
        .create_resource(create_cmd(owner_context, "facet-open-target"))
        .await
        .expect("create the readable target")
        .value
        .id;
    let open_target = Uuid::from(open_target);

    // The edges are asserted by principals who legitimately see both endpoints, so the state under
    // test is reachable rather than manufactured: the hidden edge is asserted by the target's own
    // owner after being granted write on the source and its home.
    let delegate_email = format!("f1-facet-delegate-{}@example.com", Uuid::new_v4());
    let delegate = common::fixtures::create_test_profile(&pool, &delegate_email).await;
    grant_resource_write(&pool, source, delegate).await;
    grant_context_write(&pool, owner_context, delegate).await;

    grant_resource_write(&pool, source, _p2).await;
    grant_context_write(&pool, owner_context, _p2).await;
    let hidden_edge: EdgeId = DbBackend::new(pool.clone(), ProfileId::from(_p2))
        .assert_relationship(assert_cmd(source, hidden_target))
        .await
        .expect("the target's owner may assert the edge — it reads both endpoints")
        .value;

    let open_edge: EdgeId = DbBackend::new(pool.clone(), ProfileId::from(owner))
        .assert_relationship(assert_cmd(source, open_target))
        .await
        .expect("owner asserts the readable edge")
        .value;

    // Precondition: the delegate genuinely passes clauses 1 and 2 — otherwise the refusal below
    // would prove nothing about clause 3.
    let source_writable: Option<bool> = sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(delegate)
        .bind(source)
        .fetch_one(&pool)
        .await
        .expect("source clause");
    let home_authorable: Option<bool> =
        sqlx::query_scalar("SELECT context_authorable_by_profile($1, $2)")
            .bind(delegate)
            .bind(owner_context)
            .fetch_one(&pool)
            .await
            .expect("container clause");
    let target_readable: Option<bool> =
        sqlx::query_scalar("SELECT endpoint_readable_by_profile($1, 'kb_resources', $2)")
            .bind(delegate)
            .bind(hidden_target)
            .fetch_one(&pool)
            .await
            .expect("target clause");
    assert_eq!(
        (
            source_writable.unwrap_or(false),
            home_authorable.unwrap_or(false),
            target_readable.unwrap_or(false)
        ),
        (true, true, false),
        "the fixture must isolate clause 3: clauses 1 and 2 hold, the target is unreadable"
    );

    // The refusal. NotFound, never Forbidden — confirming the edge exists would tell the delegate
    // about a resource they cannot read.
    let denied = DbBackend::new(pool.clone(), ProfileId::from(delegate))
        .set_facet(SetFacet {
            owner: PropertyOwner::edge(hidden_edge),
            values: serde_json::json!({"clause": "planted"}),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await;
    assert!(
        matches!(denied, Err(TemperError::NotFound(_))),
        "faceting an edge whose target is unreadable must be NotFound (never an existence oracle), \
         got {denied:?}"
    );
    let planted: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_properties WHERE owner_table = 'kb_edges'")
            .fetch_one(&pool)
            .await
            .expect("count edge properties");
    assert_eq!(planted, 0, "auth-before-writes: the refusal wrote nothing");

    // Positive control: same delegate, same source, same home — a readable target succeeds.
    DbBackend::new(pool.clone(), ProfileId::from(delegate))
        .set_facet(SetFacet {
            owner: PropertyOwner::edge(open_edge),
            values: serde_json::json!({"clause": "legitimate"}),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("a readable target is faceted by the same delegate — clause 3 is what refused");
}

/// The mutation verbs share the gate, so retype inherits clause 3 with no separate wiring.
///
/// Without this, clause 3 could be quietly re-scoped to the facet verb alone and the suite would
/// stay green while `retype` regained the hole.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retype_inherits_the_target_clause(pool: PgPool) {
    let (_owner, owner_context, source) = profile_with_resource(&pool, "retype-tgt-owner").await;

    let stranger_email = format!("f1-retype-tgt-{}@example.com", Uuid::new_v4());
    let (stranger, stranger_context) =
        common::fixtures::create_test_profile_with_context(&pool, &stranger_email).await;
    let hidden = DbBackend::new(pool.clone(), ProfileId::from(stranger))
        .create_resource(create_cmd(stranger_context, "retype-hidden"))
        .await
        .expect("create the hidden target")
        .value
        .id;

    grant_resource_write(&pool, source, stranger).await;
    grant_context_write(&pool, owner_context, stranger).await;
    let edge: EdgeId = DbBackend::new(pool.clone(), ProfileId::from(stranger))
        .assert_relationship(assert_cmd(source, Uuid::from(hidden)))
        .await
        .expect("the target's owner asserts it")
        .value;

    let delegate_email = format!("f1-retype-del-{}@example.com", Uuid::new_v4());
    let delegate = common::fixtures::create_test_profile(&pool, &delegate_email).await;
    grant_resource_write(&pool, source, delegate).await;
    grant_context_write(&pool, owner_context, delegate).await;

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
        matches!(denied, Err(TemperError::NotFound(_))),
        "retype must carry the same target clause as assert, got {denied:?}"
    );
}

/// A folded edge is unmutable on every axis — the edge's tombstone floor.
///
/// Mirrors the rule `can_modify_resource` already states for resources: *"a tombstone is
/// unmodifiable on every axis."* Without it, a facet lands live on a retracted link where no read
/// surface can ever see it, which falsifies the read service's own stated invariant that a live row
/// always belongs to a live edge.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_folded_edge_is_unmutable(pool: PgPool) {
    let (owner, owner_context, source) = profile_with_resource(&pool, "fold-floor-owner").await;
    let target = DbBackend::new(pool.clone(), ProfileId::from(owner))
        .create_resource(create_cmd(owner_context, "fold-floor-target"))
        .await
        .expect("create target")
        .value
        .id;

    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));
    let edge: EdgeId = backend
        .assert_relationship(assert_cmd(source, Uuid::from(target)))
        .await
        .expect("assert")
        .value;

    // Faceting works while the edge is live — the precondition that makes the refusal meaningful.
    backend
        .set_facet(SetFacet {
            owner: PropertyOwner::edge(edge),
            values: serde_json::json!({"clause": "while-live"}),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("a live edge is faceted by its owner");

    backend
        .fold_relationship(FoldRelationship {
            edge_handle: edge,
            reason: Some("retracted".to_string()),
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("owner folds the edge");

    // Post-fold: the same owner, with every clause satisfied, is refused on the tombstone floor.
    let denied = backend
        .set_facet(SetFacet {
            owner: PropertyOwner::edge(edge),
            values: serde_json::json!({"clause": "after-the-fold"}),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await;
    assert!(
        matches!(denied, Err(TemperError::NotFound(_))),
        "a folded edge must be unmutable, got {denied:?}"
    );

    // No orphan: the only property is the pre-fold one, and the cascade folded it.
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties WHERE owner_table = 'kb_edges' AND NOT is_folded",
    )
    .fetch_one(&pool)
    .await
    .expect("count live edge properties");
    assert_eq!(
        live, 0,
        "no live property may survive on a folded edge — neither the cascaded one nor a new one"
    );

    // And folding twice is now NotFound rather than a silent second fold.
    let refolded = backend
        .fold_relationship(FoldRelationship {
            edge_handle: edge,
            reason: None,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await;
    assert!(
        matches!(refolded, Err(TemperError::NotFound(_))),
        "re-folding a folded edge must be NotFound, got {refolded:?}"
    );
}
