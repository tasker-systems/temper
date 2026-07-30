#![cfg(feature = "test-db")]
//! The resource facet read — goal `019fafd9-a978-7860-ae39-23958b4471b8`.
//!
//! Witnesses for four of the register's clauses. Each names the clause it answers, because a witness
//! whose clause is only implied is unauditable later.
//!
//! **The first test fails non-vacuously against the state before this read existed**, and that is
//! the point of writing it as a differential rather than as a bare "the read returns rows". The
//! register's grounding says the read "has never run, on any door"; production's `get_meta` in fact
//! collapses every non-folded `kb_properties` row into a map keyed by `property_key`
//! (`temper-substrate/src/readback/mod.rs:264-300`), so `facet` already ships inside `open_meta` —
//! newest-wins, weight dropped, siblings hidden. So the pre-existing surface is not silent, it is
//! *confidently wrong*, and the witness that matters asserts the disagreement between the two
//! surfaces rather than merely the presence of the new one.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::authorship::ActContext;
use temper_core::types::cognitive_maps::GrantCapabilityRequest;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_core::types::property_owner::PropertyOwner;
use temper_services::backend::DbBackend;
use temper_services::error::ApiError;
use temper_services::services::{access_service, facet_service};
use temper_workflow::operations::{Backend, CreateResource, SetFacet, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

mod common;

// ── fixtures ─────────────────────────────────────────────────────────────────────

async fn owner_with_context(pool: &PgPool, email: &str) -> (DbBackend, ProfileId, ContextId) {
    let (profile, context) = common::fixtures::create_test_profile_with_context(pool, email).await;
    (
        DbBackend::new(pool.clone(), ProfileId::from(profile)),
        ProfileId::from(profile),
        ContextId::from(context),
    )
}

fn create_cmd(context: ContextId, slug: &str) -> CreateResource {
    CreateResource {
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home: HomeAnchor::Context(context),
        title: format!("Facet read {slug}"),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: Some(format!("test://facet-read-{slug}")),
        chunks_packed: None,
        content_hash: None,
        goal: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

async fn assert_facet(
    backend: &DbBackend,
    resource: ResourceId,
    values: serde_json::Value,
    weight: f64,
) {
    backend
        .set_facet(SetFacet {
            owner: PropertyOwner::resource(resource),
            values,
            weight,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("facet set");
}

// ── the concealment witness ──────────────────────────────────────────────────────

/// Clause `the-read-does-not-conceal-whether-a-key-was-asserted-twice`, and with it
/// `an-asserted-facet-is-discoverable-by-anyone-who-may-read-its-owner`.
///
/// Re-asserting one logical facet supersedes it **per inner key**, and both surfaces agree.
///
/// This test previously asserted the opposite — that two asserts leave two live rows for the same
/// key, and that `get_meta` disclosed only one of them. Both halves changed under
/// `019f6d08`'s inner-key grain, and deliberately:
///
/// - the write now folds the prior mark for each key it names, so one logical facet is one live row
///   (the accumulation this task existed to end);
/// - `get_meta` now MERGES the marks rather than surfacing the newest row, so it reports the whole
///   set instead of an arbitrary member of it.
///
/// What survives verbatim is the property that made the original test worth writing: the two
/// surfaces are checked **against each other**, so a change to one that silently diverges from the
/// other fails here rather than in production.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn re_asserting_a_facet_supersedes_per_key_and_both_surfaces_agree(pool: PgPool) {
    let (backend, owner, context) = owner_with_context(&pool, "facet-read-owner@example.com").await;
    let resource = backend
        .create_resource(create_cmd(context, "twice-asserted"))
        .await
        .expect("create")
        .value
        .id;

    assert_facet(
        &backend,
        resource,
        serde_json::json!({"node_label": "concern", "status": "open"}),
        0.85,
    )
    .await;
    assert_facet(
        &backend,
        resource,
        serde_json::json!({"node_label": "concern", "status": "resolved"}),
        1.0,
    )
    .await;

    let facets = facet_service::list_resource_facets(&pool, owner, resource)
        .await
        .expect("facet read");

    // Two marks, not four and not two-of-one-key: `node_label` and `status` were each asserted
    // twice, and each supersedes in place.
    assert_eq!(
        facets.len(),
        2,
        "one live row per inner key, not one per assert: {facets:#?}"
    );
    assert!(
        facets.iter().all(|f| f.property_key == "facet"),
        "the read is scoped to the facet key: {facets:#?}"
    );

    let mark = |key: &str| {
        facets
            .iter()
            .find(|f| f.value.get(key).is_some())
            .unwrap_or_else(|| panic!("no live mark for {key}: {facets:#?}"))
    };
    assert_eq!(
        mark("status").value["status"],
        "resolved",
        "the superseding value wins: {facets:#?}"
    );
    assert_eq!(mark("node_label").value["node_label"], "concern");

    // Weight is what the region producer clusters on (`substrate.rs:110-122` → `expand_facets`),
    // and it is exactly what `open_meta` drops. Both marks carry the SECOND assert's weight,
    // because both keys were named by it.
    assert_eq!(mark("status").weight, 1.0);
    assert_eq!(mark("node_label").weight, 1.0);

    // Attribution: the author is recoverable without a second round trip.
    assert_eq!(facets[0].authored_by_profile_id, Some(*owner));
    assert!(facets[0].authored_by_handle.is_some());

    // The superseded mark is folded, not deleted — the trail survives the supersession.
    let folded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE property_key = 'facet' AND is_folded AND owner_id = $1",
    )
    .bind(Uuid::from(resource))
    .fetch_one(&pool)
    .await
    .expect("folded count");
    assert_eq!(folded, 2, "both first-assert marks are folded, not dropped");

    // The other half of the differential. `get_meta` now MERGES the marks, so it agrees with the
    // facet read about the set — where it used to disclose one row and hide the rest.
    let meta = temper_services::backend::substrate_read::get_meta_select(&pool, owner, resource)
        .await
        .expect("get_meta");
    let open = meta.open_meta.expect("open_meta present");
    assert_eq!(
        open["facet"],
        serde_json::json!({"node_label": "concern", "status": "resolved"}),
        "get_meta merges every live mark rather than surfacing one row: {open:#?}"
    );
    assert!(
        open["facet"].get("weight").is_none(),
        "it still carries no weight — the facet read remains the faithful door: {open:#?}"
    );
}

// ── the empty case is not a refusal ──────────────────────────────────────────────

/// The register left this open: *"whether a resource that exists and is readable but carries zero
/// facets is answerably distinct from one that does not exist. It should be — but that is a clause
/// about the read's shape, and it belongs to the build."* Decided here, in the build: it is distinct.
/// `200 []` for readable-and-empty, `404` for unreadable-or-absent — CONFORMing to
/// `citation_audit_service::list_citation_audits`, which spends its whole denial dialect on the same
/// distinction. It is not an oracle, because the empty list is only reachable by a caller who has
/// already passed the readability gate.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_readable_resource_with_no_facets_answers_empty_not_missing(pool: PgPool) {
    let (backend, owner, context) = owner_with_context(&pool, "facet-read-empty@example.com").await;
    let resource = backend
        .create_resource(create_cmd(context, "no-facets"))
        .await
        .expect("create")
        .value
        .id;

    let facets = facet_service::list_resource_facets(&pool, owner, resource)
        .await
        .expect("readable-and-empty is a success, not a refusal");
    assert!(facets.is_empty(), "no facets asserted: {facets:#?}");
}

// ── the refusal face ─────────────────────────────────────────────────────────────

/// Clause `a-facet-read-never-widens-who-can-see-what`, negative face: the refusal is not an
/// existence oracle. A resource that exists but is unreadable, and an id that was never minted, must
/// be **indistinguishable** — same variant, same message. Asserting the messages are equal is the
/// part that matters; a test that only checked both are `NotFound` would pass while the two carried
/// distinguishable prose.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unreadable_and_never_existed_refuse_identically(pool: PgPool) {
    let (owner_backend, _owner, context) =
        owner_with_context(&pool, "facet-read-hidden@example.com").await;
    let resource = owner_backend
        .create_resource(create_cmd(context, "hidden"))
        .await
        .expect("create")
        .value
        .id;
    assert_facet(
        &owner_backend,
        resource,
        serde_json::json!({"node_label": "fact"}),
        1.0,
    )
    .await;

    let stranger = ProfileId::from(
        common::fixtures::create_test_profile(&pool, "facet-read-stranger@example.com").await,
    );

    let hidden = facet_service::list_resource_facets(&pool, stranger, resource)
        .await
        .expect_err("a resource the caller cannot read must refuse");
    let absent =
        facet_service::list_resource_facets(&pool, stranger, ResourceId::from(Uuid::now_v7()))
            .await
            .expect_err("an id that never existed must refuse");

    match (&hidden, &absent) {
        (ApiError::NotFound(a), ApiError::NotFound(b)) => assert_eq!(
            a, b,
            "the two refusals must be byte-identical, or the diff is an existence oracle"
        ),
        other => panic!("both arms must render NotFound, got {other:?}"),
    }
}

// ── reading is not a licence to write ────────────────────────────────────────────

/// Clause `reading-a-facet-never-becomes-a-licence-to-assert-one`, and the equivalence the register
/// flagged as *most likely to be wrong* — that read reach alone suffices for this read. A principal
/// holding `can_read` and nothing else reads the facets, and is still refused the write. The two
/// halves are asserted in one test on purpose: they are the same grant, and splitting them would let
/// one pass while the other silently regressed.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_read_grant_reads_facets_and_still_cannot_assert_one(pool: PgPool) {
    let (owner_backend, owner, context) =
        owner_with_context(&pool, "facet-read-granter@example.com").await;
    let resource = owner_backend
        .create_resource(create_cmd(context, "granted"))
        .await
        .expect("create")
        .value
        .id;
    assert_facet(
        &owner_backend,
        resource,
        serde_json::json!({"node_label": "decision", "status": "settled"}),
        0.7,
    )
    .await;

    let reader =
        common::fixtures::create_test_profile(&pool, "facet-read-reader@example.com").await;
    let admin = common::fixtures::create_test_profile(&pool, "facet-read-admin@example.com").await;
    common::fixtures::make_test_admin(&pool, admin).await;

    access_service::grant_capability(
        &pool,
        ProfileId::from(admin),
        &GrantCapabilityRequest {
            subject_table: "kb_resources".into(),
            subject_id: Uuid::from(resource),
            principal_table: "kb_profiles".into(),
            principal_id: reader,
            can_read: true,
            can_write: false,
            can_delete: false,
            can_grant: false,
        },
    )
    .await
    .expect("admin grants read");

    let reader_id = ProfileId::from(reader);

    // Read reach is sufficient for the read.
    let facets = facet_service::list_resource_facets(&pool, reader_id, resource)
        .await
        .expect("a read grant may read facets");
    // Two marks, because the two-key assert is two rows since the inner-key grain landed
    // (`019f6d08`). This test's subject is REACH, not grain — it asserts on every row so it stays
    // about access even if the fixture's key count changes again.
    assert_eq!(facets.len(), 2, "{facets:#?}");
    assert!(
        facets.iter().all(|f| f.weight == 0.7),
        "weight survives the read on every mark: {facets:#?}"
    );
    assert!(
        facets
            .iter()
            .all(|f| f.authored_by_profile_id == Some(*owner)),
        "every row names its author, not its reader: {facets:#?}"
    );

    // And insufficient for the write. The read did not become a licence.
    //
    // Pinned to `Forbidden` specifically, matching `set_facet_test`'s non-owner arm — an assertion
    // that accepted any error variant would pass if the write started failing for an unrelated
    // reason, which is the shape of a test that is green for the wrong reason. The row count is the
    // half that cannot weaken silently: auth-before-write means nothing was written, not that a
    // write was written and then rejected.
    let reader_backend = DbBackend::new(pool.clone(), reader_id);
    let denied = reader_backend
        .set_facet(SetFacet {
            owner: PropertyOwner::resource(resource),
            values: serde_json::json!({"node_label": "planted"}),
            weight: 1.0,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await;
    assert!(
        matches!(denied, Err(TemperError::Forbidden)),
        "read reach must not authorize a facet assert: {denied:?}"
    );

    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties
          WHERE owner_table = 'kb_resources' AND owner_id = $1
            AND property_key = 'facet' AND NOT is_folded",
    )
    .bind(Uuid::from(resource))
    .fetch_one(&pool)
    .await
    .expect("count live facets");
    assert_eq!(
        live, 2,
        "the denied assert must have written nothing — the owner's two marks are all that remain"
    );
}
