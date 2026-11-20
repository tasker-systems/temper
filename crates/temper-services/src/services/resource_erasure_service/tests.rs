//! Service witnesses. Every test runs on a fresh database migrated by
//! `temper_substrate::MIGRATOR` — no `reset_schema` — so every event type a migration
//! registered is present (the substrate suite's re-registration trap does not apply).
use bytes::Bytes;
use sha2::Digest as _;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::property_owner::PropertyOwner;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::blob_store::{
    blob_pathname, BlobHead, ByteStream, InMemoryBlobStore, PutReceipt,
};
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::ContextId;
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, AssertParams, CommitBlobParams, CreateMode, CreateParams};

use super::*;
use crate::services::erasure_fence_service;
use crate::test_support;

/// The raise literals' distinctive parts: no response or error may carry any of them.
const RAISE_FRAGMENTS: &[&str] = &[
    "resource_erasure_execute:",
    "blob_delete:",
    "related-blob remainder; strike refused",
    "missing or already folded",
    "the ledger carries its emptying",
    "map-grain erasure is filed task",
    "already erased",
];

/// How a [`BadDeleteStore`]'s provider delete misbehaves.
#[derive(Debug, Clone, Copy)]
enum BadDelete {
    Fails,
    Hangs,
}

/// A provider whose delete errors or never returns, over a working in-memory store — the
/// post-commit release's two bad days. Everything but `delete` passes through.
#[derive(Debug)]
struct BadDeleteStore {
    inner: InMemoryBlobStore,
    mode: BadDelete,
}

#[async_trait::async_trait]
impl BlobStore for BadDeleteStore {
    async fn exists(&self, pathname: &str) -> anyhow::Result<bool> {
        self.inner.exists(pathname).await
    }
    async fn put(
        &self,
        pathname: &str,
        content_type: &str,
        body: Bytes,
        cache_control_max_age: u32,
    ) -> anyhow::Result<PutReceipt> {
        self.inner
            .put(pathname, content_type, body, cache_control_max_age)
            .await
    }
    async fn get(&self, pathname: &str, consistent: bool) -> anyhow::Result<ByteStream> {
        self.inner.get(pathname, consistent).await
    }
    async fn head(&self, pathname: &str) -> anyhow::Result<Option<BlobHead>> {
        self.inner.head(pathname).await
    }
    async fn delete(&self, _pathnames: &[&str]) -> anyhow::Result<()> {
        match self.mode {
            BadDelete::Fails => anyhow::bail!("provider unavailable (the witness's outage)"),
            BadDelete::Hangs => std::future::pending::<anyhow::Result<()>>().await,
        }
    }
}

/// A principal: profile, its `<handle>@web` emitter entity, and a personal context.
struct Principal {
    profile: ProfileId,
    handle: String,
    emitter: EntityId,
    home: ContextId,
}

/// The handle is the FULL id: two UUIDv7s minted in one millisecond share leading bytes, so
/// a truncated handle collides on `kb_profiles_handle_key` (the template's rule).
async fn principal(pool: &PgPool) -> Principal {
    let id = Uuid::now_v7();
    let handle = format!("user-{id}");
    crate::test_support::profile::insert_with_id(pool, id, &handle).await;
    let emitter = crate::test_support::entity::insert(pool, id, &format!("{handle}@web")).await;
    let home = crate::test_support::context::insert_personal(pool, id, "home", "Home").await;
    Principal {
        profile: ProfileId::from(id),
        handle,
        emitter: EntityId::from(emitter),
        home: ContextId::from(home),
    }
}

/// An operator's sealed proof, minted through the real gate after `grant_governance`.
async fn operator(pool: &PgPool) -> SystemAdmin {
    let op = principal(pool).await;
    test_support::grant_governance(pool, op.profile.uuid()).await;
    test_support::system_admin_proof_for(pool, op.profile.uuid()).await
}

/// A resource created through the REAL create path, homed in `owner`'s context.
async fn resource_with_mode(
    pool: &PgPool,
    owner: &Principal,
    title: &str,
    mode: CreateMode,
) -> ResourceId {
    let origin = format!("test://{title}-{}", Uuid::now_v7());
    writes::create_resource_with_mode(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &origin,
            body: "body under erasure",
            doc_type: "research",
            home: AnchorRef::context(owner.home),
            owner: owner.profile,
            originator: owner.profile,
            emitter: owner.emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
        mode,
    )
    .await
    .expect("create resource through the real path")
}

async fn resource(pool: &PgPool, owner: &Principal, title: &str) -> ResourceId {
    resource_with_mode(pool, owner, title, CreateMode::default()).await
}

/// A live blob committed through the REAL path (its bytes pre-registered in `store`), with a
/// relation edge from it to each of `related`. Returns the blob and its pathname.
async fn related_blob(
    pool: &PgPool,
    store: &InMemoryBlobStore,
    owner: &Principal,
    related: &[ResourceId],
) -> (BlobId, String) {
    let bytes = format!("blob bytes {}", Uuid::now_v7());
    let hash = hex::encode(sha2::Sha256::digest(bytes.as_bytes()));
    let pathname = blob_pathname(&hash);
    store.insert(pathname.clone());
    let blob = writes::commit_blob(
        pool,
        store,
        CommitBlobParams {
            id: BlobId::from(Uuid::now_v7()),
            home: AnchorRef::context(owner.home),
            owner: owner.profile,
            originator: None,
            content_hash: hash,
            content_type: "image/png".to_owned(),
            content_bytes: bytes.len() as i64,
            max_bytes: 10 * 1024 * 1024,
            allowlist: &["image/png".to_owned()][..],
            emitter: owner.emitter,
        },
    )
    .await
    .expect("the blob commits through the real path");
    for r in related {
        let mut conn = pool.acquire().await.expect("acquire");
        fire(
            &mut conn,
            SeedAction::RelationshipAssert {
                src: AnchorRef::blob(blob),
                tgt: AnchorRef::resource(*r),
                kind: EdgeKind::Contains,
                polarity: EdgePolarity::Forward,
                label: Some("attached"),
                weight: 1.0,
                home: EdgeHome::Context(owner.home),
                emitter: owner.emitter,
            },
        )
        .await
        .expect("the blob relation asserts through the real path");
    }
    (blob, pathname)
}

fn request(resource: ResourceId, blobs: &[BlobId]) -> ResourceErasureRequest<'_> {
    ResourceErasureRequest {
        resource,
        also_strike_blobs: blobs,
        surface: Surface::ApiHttp,
    }
}

async fn execute(
    pool: &PgPool,
    admin: &SystemAdmin,
    resource: ResourceId,
) -> ApiResult<ResourceErasureOutcome> {
    execute_resource_erasure(pool, None, admin, request(resource, &[])).await
}

async fn events_of(pool: &PgPool, kind: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = $1",
    )
    .bind(kind)
    .fetch_one(pool)
    .await
    .expect("count events")
}

async fn erased_at(pool: &PgPool, resource: ResourceId) -> Option<chrono::DateTime<chrono::Utc>> {
    sqlx::query_scalar("SELECT erased_at FROM kb_resources WHERE id = $1")
        .bind(resource.uuid())
        .fetch_one(pool)
        .await
        .expect("resource row")
}

fn completed(outcome: ResourceErasureOutcome) -> ResourceErasureCompletion {
    match outcome {
        ResourceErasureOutcome::Completed(c) => c,
        other => panic!("the act must complete, got {other:?}"),
    }
}

fn refused(outcome: ResourceErasureOutcome) -> ResourceErasureRefusal {
    match outcome {
        ResourceErasureOutcome::Refused(r) => r,
        other => panic!("the act must be refused, got {other:?}"),
    }
}

fn assert_no_raise_literal(text: &str) {
    for fragment in RAISE_FRAGMENTS {
        assert!(
            !text.contains(fragment),
            "a response carries raise text {fragment:?}: {text}"
        );
    }
}

/// ── WITNESS: the charter refusal ────────────────────────────────────────────────────────
/// FAILS IF a charter resource is not refused-and-recorded, or its detail is not the fixed
/// map-grain text.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_charter_resource_is_refused_recorded_with_the_fixed_detail(pool: PgPool) {
    bootseed::seed_system(&pool).await.expect("boot seed");
    let owner = principal(&pool).await;
    let op = operator(&pool).await;
    let telos = {
        let mut conn = pool.acquire().await.expect("acquire");
        fire(
            &mut conn,
            SeedAction::CogmapGenesis {
                name: "charter-map",
                telos_title: "charter telos",
                charter: &[],
                cogmap_id: None,
                telos_resource_id: None,
                owner: owner.profile,
                emitter: owner.emitter,
            },
        )
        .await
        .expect("cogmap genesis")
        .cogmap_genesis()
        .expect("genesis mints the telos")
        .1
    };

    let refusal = refused(execute(&pool, &op, telos).await.expect("the door answers"));
    assert_eq!(
        refusal.reason,
        ResourceErasureRefusalReason::CharterResource
    );
    assert_eq!(
        refusal.detail,
        Some(ResourceErasureRefusalDetail::MapGrainErasureTask)
    );

    let (reason, detail, correlation): (String, String, Uuid) = sqlx::query_as(
        "SELECT payload->>'reason', payload->>'detail', correlation_id FROM kb_events \
          WHERE id = $1",
    )
    .bind(refusal.event_id)
    .fetch_one(&pool)
    .await
    .expect("the refusal is recorded");
    assert_eq!(reason, "charter_resource");
    assert_eq!(detail, MAP_GRAIN_ERASURE_DETAIL);
    assert_eq!(correlation, refusal.request_reference);
    assert!(erased_at(&pool, telos).await.is_none());
    assert_eq!(events_of(&pool, "resource_erased").await, 0);
}

/// ── WITNESS 11: a repeat erasure ────────────────────────────────────────────────────────
/// FAILS IF a second execute mints a second `resource_erased`, changes any projection row,
/// or goes unrecorded.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_repeat_erasure_is_refused_already_erased_and_changes_nothing(pool: PgPool) {
    let owner = principal(&pool).await;
    let op = operator(&pool).await;
    let r = resource(&pool, &owner, "repeat").await;
    completed(execute(&pool, &op, r).await.expect("the first act runs"));
    let before = temper_substrate::replay::dump_projections(&pool)
        .await
        .expect("dump");

    let refusal = refused(execute(&pool, &op, r).await.expect("the door answers"));
    assert_eq!(refusal.reason, ResourceErasureRefusalReason::AlreadyErased);
    assert_eq!(refusal.detail, None);

    let after = temper_substrate::replay::dump_projections(&pool)
        .await
        .expect("dump");
    assert_eq!(before, after, "the repeat changed the projection");
    let reason: String =
        sqlx::query_scalar("SELECT payload->>'reason' FROM kb_events WHERE id = $1")
            .bind(refusal.event_id)
            .fetch_one(&pool)
            .await
            .expect("the refusal is recorded");
    assert_eq!(reason, "already_erased");
    assert_eq!(
        events_of(&pool, "resource_erased").await,
        1,
        "exactly one resource_erased"
    );
}

/// ── WITNESS 11: the two former refusals complete ────────────────────────────────────────
/// FAILS IF the service refuses a tombstone (made by the real soft delete) or an in-flight
/// ingest (a segmented create not yet finalized).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_tombstone_and_an_in_flight_ingest_both_complete(pool: PgPool) {
    let owner = principal(&pool).await;
    let op = operator(&pool).await;

    let tombstone = resource(&pool, &owner, "tombstone").await;
    let mut tx = pool.begin().await.expect("begin");
    fire(
        &mut tx,
        SeedAction::ResourceDelete {
            resource: tombstone,
            emitter: owner.emitter,
        },
    )
    .await
    .expect("soft delete through the real path");
    tx.commit().await.expect("commit");
    let active: bool = sqlx::query_scalar("SELECT is_active FROM kb_resources WHERE id = $1")
        .bind(tombstone.uuid())
        .fetch_one(&pool)
        .await
        .expect("row");
    assert!(!active, "the witness needs a real tombstone");

    let in_flight = resource_with_mode(
        &pool,
        &owner,
        "in-flight",
        CreateMode {
            defer: false,
            segmented: true,
        },
    )
    .await;
    let ingest: String = sqlx::query_scalar("SELECT ingest_state FROM kb_resources WHERE id = $1")
        .bind(in_flight.uuid())
        .fetch_one(&pool)
        .await
        .expect("row");
    assert_eq!(
        ingest, "in_progress",
        "the witness needs an ingest in flight"
    );

    completed(
        execute(&pool, &op, tombstone)
            .await
            .expect("the tombstone erases"),
    );
    let flight = completed(
        execute(&pool, &op, in_flight)
            .await
            .expect("the ingest erases"),
    );
    assert!(erased_at(&pool, tombstone).await.is_some());
    assert!(erased_at(&pool, in_flight).await.is_some());
    assert!(
        flight
            .targets
            .iter()
            .any(|t| t.target == "kb_resources.ingest_state"),
        "the record names the ended ingest: {:?}",
        flight.targets
    );
    assert_eq!(events_of(&pool, "resource_erasure_refused").await, 0);
}

/// ── WITNESS: one reference per act ──────────────────────────────────────────────────────
/// FAILS IF two acts share a request reference, or the returned reference is not the one
/// the act's events are correlated by.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn two_acts_get_two_distinct_request_references(pool: PgPool) {
    let owner = principal(&pool).await;
    let op = operator(&pool).await;
    let a = completed(
        execute(&pool, &op, resource(&pool, &owner, "first").await)
            .await
            .expect("first act"),
    );
    let b = completed(
        execute(&pool, &op, resource(&pool, &owner, "second").await)
            .await
            .expect("second act"),
    );
    assert_ne!(a.request_reference, b.request_reference);
    for c in [&a, &b] {
        let correlation: Uuid =
            sqlx::query_scalar("SELECT correlation_id FROM kb_events WHERE id = $1")
                .bind(c.event_id)
                .fetch_one(&pool)
                .await
                .expect("the completion event");
        assert_eq!(correlation, c.request_reference);
    }
}

/// ── WITNESS: no raise text reaches a caller ─────────────────────────────────────────────
/// FAILS IF a refusal's rendered detail or an error string carries a raise literal. The
/// scan reads the text a door renders (`detail.as_str()`, `ApiError`'s message), not a
/// `Debug` of the outcome, whose variant names could never match. Not vacuous: the charter
/// and already-erased refusals exist only because the SQL raised, and the two blob 400s are
/// produced only by the classifier's arms over a raised act.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn no_response_or_error_contains_a_raise_literal(pool: PgPool) {
    bootseed::seed_system(&pool).await.expect("boot seed");
    let owner = principal(&pool).await;
    let op = operator(&pool).await;
    let store = InMemoryBlobStore::default();
    let mut texts: Vec<String> = Vec::new();

    // Charter.
    let telos = {
        let mut conn = pool.acquire().await.expect("acquire");
        fire(
            &mut conn,
            SeedAction::CogmapGenesis {
                name: "raise-map",
                telos_title: "raise telos",
                charter: &[],
                cogmap_id: None,
                telos_resource_id: None,
                owner: owner.profile,
                emitter: owner.emitter,
            },
        )
        .await
        .expect("cogmap genesis")
        .cogmap_genesis()
        .expect("telos")
        .1
    };
    let charter = refused(execute(&pool, &op, telos).await.expect("answers"));
    texts.extend(charter.detail.map(|d| d.as_str().to_string()));
    assert_eq!(
        texts.len(),
        1,
        "the charter refusal renders a detail to scan"
    );

    // Already erased, and a blob struck once through a completed act.
    let r1 = resource(&pool, &owner, "raise-r1").await;
    let r2 = resource(&pool, &owner, "raise-r2").await;
    let r3 = resource(&pool, &owner, "raise-r3").await;
    let (blob, _) = related_blob(&pool, &store, &owner, &[r1, r2]).await;
    completed(
        execute_resource_erasure(&pool, Some(&store), &op, request(r1, &[blob]))
            .await
            .expect("r1 erases, striking the blob"),
    );
    let repeat = refused(execute(&pool, &op, r1).await.expect("answers"));
    assert_eq!(repeat.reason, ResourceErasureRefusalReason::AlreadyErased);
    texts.extend(repeat.detail.map(|d| d.as_str().to_string()));
    assert_eq!(
        events_of(&pool, "resource_erasure_refused").await,
        2,
        "both raised"
    );

    // The blob again, through r2 (still related, already struck) and r3 (never related).
    let struck = execute_resource_erasure(&pool, Some(&store), &op, request(r2, &[blob]))
        .await
        .expect_err("an already-struck blob is refused");
    let unrelated = execute_resource_erasure(&pool, Some(&store), &op, request(r3, &[blob]))
        .await
        .expect_err("an unrelated blob is refused");
    for err in [&struck, &unrelated] {
        let ApiError::BadRequest(msg) = err else {
            panic!("a blob refusal is a 400, got {err:?}");
        };
        assert!(msg.contains(&blob.to_string()), "names the blob: {msg}");
        texts.push(err.to_string());
    }
    assert!(erased_at(&pool, r2).await.is_none() && erased_at(&pool, r3).await.is_none());

    // An unknown id, past the gate.
    let unknown = execute(&pool, &op, ResourceId::from(Uuid::now_v7()))
        .await
        .expect_err("an unknown id is not found");
    assert!(matches!(unknown, ApiError::NotFound(_)), "got {unknown:?}");
    texts.push(unknown.to_string());

    for text in &texts {
        assert_no_raise_literal(text);
    }
}

/// ── WITNESS: the classifier against the live SQL ────────────────────────────────────────
/// FAILS IF a RAISE in the act or in `blob_delete` is reworded so the classifier no longer
/// recognizes it (it would otherwise degrade to `Internal` silently).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_classifier_matches_the_live_sql_raises(pool: PgPool) {
    bootseed::seed_system(&pool).await.expect("boot seed");
    let owner = principal(&pool).await;
    let store = InMemoryBlobStore::default();
    let raw = |resource: Uuid, blobs: Vec<Uuid>| {
        let pool = pool.clone();
        let emitter = owner.emitter.uuid();
        let actor = owner.profile.uuid();
        async move {
            sqlx::query_scalar::<_, serde_json::Value>(
                "SELECT resource_erasure_execute($1, $2, $3, $4, $5)",
            )
            .bind(resource)
            .bind(actor)
            .bind(emitter)
            .bind(Uuid::now_v7())
            .bind(blobs)
            .fetch_one(&pool)
            .await
        }
    };

    let telos = {
        let mut conn = pool.acquire().await.expect("acquire");
        fire(
            &mut conn,
            SeedAction::CogmapGenesis {
                name: "pin-map",
                telos_title: "pin telos",
                charter: &[],
                cogmap_id: None,
                telos_resource_id: None,
                owner: owner.profile,
                emitter: owner.emitter,
            },
        )
        .await
        .expect("cogmap genesis")
        .cogmap_genesis()
        .expect("telos")
        .1
    };
    let err = raw(telos.uuid(), vec![]).await.expect_err("charter raises");
    assert_eq!(classify_act_error(&err), ActFailure::Charter);

    let r1 = resource(&pool, &owner, "pin-r1").await;
    let r2 = resource(&pool, &owner, "pin-r2").await;
    let r3 = resource(&pool, &owner, "pin-r3").await;
    let (blob, _) = related_blob(&pool, &store, &owner, &[r1, r2]).await;
    raw(r1.uuid(), vec![blob.uuid()]).await.expect("r1 erases");

    let err = raw(r1.uuid(), vec![]).await.expect_err("repeat raises");
    assert_eq!(classify_act_error(&err), ActFailure::AlreadyErased);

    let err = raw(r3.uuid(), vec![blob.uuid()])
        .await
        .expect_err("unrelated raises");
    assert_eq!(
        classify_act_error(&err),
        ActFailure::BlobNotInRemainder(Some(Uuid::from(blob)))
    );

    let err = raw(r2.uuid(), vec![blob.uuid()])
        .await
        .expect_err("struck raises");
    assert_eq!(
        classify_act_error(&err),
        ActFailure::BlobAlreadyStruck(Some(Uuid::from(blob)))
    );

    let err = raw(Uuid::now_v7(), vec![])
        .await
        .expect_err("unknown raises");
    assert_eq!(classify_act_error(&err), ActFailure::NotFound);
}

/// ── WITNESS: the husk short-circuit ─────────────────────────────────────────────────────
/// FAILS IF the survey of an erased resource runs the plan (which still counts husk rows).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_survey_short_circuits_on_a_husk(pool: PgPool) {
    let owner = principal(&pool).await;
    let op = operator(&pool).await;
    let r = resource(&pool, &owner, "husk").await;
    completed(execute(&pool, &op, r).await.expect("erases"));

    let survey = survey_resource_erasure(&pool, &op, r)
        .await
        .expect("answers");
    assert!(survey.already_erased);
    assert_eq!(survey.plan, None, "no plan on a husk");
    assert_eq!(
        survey.completion_fields,
        vec![],
        "an act under the ledger exception leaves nothing to complete"
    );
}

/// ── WITNESS 15 at the doors: the completion pass ───────────────────────────────────────
/// FAILS IF a husk erased under cut 1 cannot be completed through the doors, or if the
/// survey's `completion_fields` is not what the pass then rewrites, or if a completed husk is
/// not refused, on the record, as `already_erased`. The cut-1 husk is made as the cut-1 act
/// made one: the projection-side body under a record naming no `redacted_fields`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_cut1_husk_surveys_its_completion_and_completes_once(pool: PgPool) {
    let owner = principal(&pool).await;
    let op = operator(&pool).await;
    let r = resource(&pool, &owner, "cut-1 husk").await;
    sqlx::query(
        "SELECT _resource_erasure_apply_redaction($1, _event_append('resource_erased', $2, NULL, NULL, \
             jsonb_build_object('subject_table', 'kb_resources', 'subject_id', $1, \
                                'targets', '[]'::jsonb, \
                                'ledger_remainder', resource_erasure_survey_plan($1)->'redacted_fields')))",
    )
    .bind(r.uuid())
    .bind(owner.emitter.uuid())
    .execute(&pool)
    .await
    .expect("a cut-1 husk");

    let survey = survey_resource_erasure(&pool, &op, r)
        .await
        .expect("answers");
    assert!(survey.already_erased);
    assert_eq!(survey.plan, None);
    assert!(
        survey
            .completion_fields
            .iter()
            .any(|f| f.paths.iter().any(|p| p == "title")),
        "the survey names the title the ledger still carries: {:?}",
        survey.completion_fields
    );

    let completion = completed(execute(&pool, &op, r).await.expect("the pass runs"));
    assert_eq!(completion.redacted_fields, survey.completion_fields);
    assert!(completion.folded_edges.is_empty() && completion.targets.is_empty());
    assert_eq!(events_of(&pool, "resource_erased").await, 2);

    let after = survey_resource_erasure(&pool, &op, r)
        .await
        .expect("answers");
    assert_eq!(after.completion_fields, vec![], "nothing is left");
    let refusal = refused(execute(&pool, &op, r).await.expect("the door answers"));
    assert_eq!(refusal.reason, ResourceErasureRefusalReason::AlreadyErased);
    assert_eq!(events_of(&pool, "resource_erased").await, 2);
}

/// ── WITNESS: another principal's edge is named, with its author ─────────────────────────
/// FAILS IF an edge (or an edge-owned property) another principal asserted goes unnamed, or
/// the owner's own edge — asserted from a SECOND surface entity — is misnamed as another
/// principal's (the profile-to-profile comparison).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_survey_names_another_principals_edge_and_its_author(pool: PgPool) {
    let owner = principal(&pool).await;
    let other = principal(&pool).await;
    let op = operator(&pool).await;
    let r = resource(&pool, &owner, "authored").await;
    let s = resource(&pool, &other, "other-notes").await;
    let t = resource(&pool, &owner, "owner-notes").await;

    let foreign = writes::assert_relationship(
        &pool,
        AssertParams {
            src: s,
            tgt: r,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("another principal's words"),
            weight: 1.0,
            home: other.home,
            emitter: other.emitter,
        },
    )
    .await
    .expect("the other principal's edge");
    let foreign_prop = writes::assert_keyed_property_with(
        &pool,
        PropertyOwner::edge(foreign),
        "note",
        &serde_json::json!("their text"),
        1.0,
        other.emitter,
        EventContext::default(),
    )
    .await
    .expect("the other principal's edge property");

    // The owner's own edge, from the owner's `@cli` entity: a second surface, same principal.
    let owner_cli: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id",
    )
    .bind(owner.profile.uuid())
    .bind(format!("{}@cli", owner.handle))
    .fetch_one(&pool)
    .await
    .expect("owner cli entity");
    let own = writes::assert_relationship(
        &pool,
        AssertParams {
            src: r,
            tgt: t,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("the owner's words"),
            weight: 1.0,
            home: owner.home,
            emitter: EntityId::from(owner_cli),
        },
    )
    .await
    .expect("the owner's edge");

    let plan = survey_resource_erasure(&pool, &op, r)
        .await
        .expect("answers")
        .plan
        .expect("a live resource has a plan");
    assert!(plan.edges.contains(&foreign) && plan.edges.contains(&own));
    assert_eq!(
        plan.other_author_edges,
        vec![OtherAuthorEdge {
            edge_id: foreign,
            author: other.profile,
            folded: false,
        }],
        "only the other principal's edge is named"
    );
    assert_eq!(
        plan.other_author_edge_properties,
        vec![OtherAuthorEdgeProperty {
            property_id: foreign_prop,
            edge_id: foreign,
            author: other.profile,
            folded: false,
        }]
    );
}

/// ── WITNESS: a blob's co-link holder is named ───────────────────────────────────────────
/// FAILS IF the survey does not name the other resource linking a related blob, or names
/// the surveyed resource itself as a holder.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_survey_names_a_blobs_co_link_holder(pool: PgPool) {
    let owner = principal(&pool).await;
    let op = operator(&pool).await;
    let store = InMemoryBlobStore::default();
    let r = resource(&pool, &owner, "co-link-r").await;
    let holder = resource(&pool, &owner, "co-link-holder").await;
    let (shared, _) = related_blob(&pool, &store, &owner, &[r, holder]).await;
    let (alone, _) = related_blob(&pool, &store, &owner, &[r]).await;

    let plan = survey_resource_erasure(&pool, &op, r)
        .await
        .expect("answers")
        .plan
        .expect("a plan");
    let mut links = plan.blob_co_links;
    links.sort_by_key(|l| l.blob_id.uuid());
    let mut expected = vec![
        BlobCoLinks {
            blob_id: shared,
            holders: vec![holder],
        },
        BlobCoLinks {
            blob_id: alone,
            holders: vec![],
        },
    ];
    expected.sort_by_key(|l| l.blob_id.uuid());
    assert_eq!(links, expected);
}

/// ── WITNESS: the post-commit byte release ───────────────────────────────────────────────
/// FAILS IF a released strike's bytes survive the call (the release did not run after the
/// commit), or if the fence, deriving the same delete from the `resource_erased` payload,
/// fails it or leaves it outstanding — the provider delete of an absent object is a no-op.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_listed_blob_strike_releases_its_bytes_and_the_fence_has_nothing_left(pool: PgPool) {
    let owner = principal(&pool).await;
    let op = operator(&pool).await;
    let store = InMemoryBlobStore::default();
    let r = resource(&pool, &owner, "strike").await;
    let (blob, pathname) = related_blob(&pool, &store, &owner, &[r]).await;
    assert!(
        store.contains(&pathname),
        "the witness needs the bytes present"
    );

    let completion = completed(
        execute_resource_erasure(&pool, Some(&store), &op, request(r, &[blob]))
            .await
            .expect("the act completes"),
    );
    assert_eq!(
        completion.blob_strikes,
        vec![BlobStrikeOutcome {
            blob_id: Uuid::from(blob),
            released: true,
        }]
    );
    assert!(
        !store.contains(&pathname),
        "the bytes are released after the commit, before any fence tick"
    );

    let summary = erasure_fence_service::drain(&pool, &store)
        .await
        .expect("the fence drains");
    assert_eq!(summary.seeded, 1, "the fence derives the same delete");
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.unparseable_verdicts, 0);
    let outstanding: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_erasure_blob_deletes \
          WHERE status IN ('pending', 'in_progress', 'waiting_for_retry', 'dead')",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(outstanding, 0, "nothing left to drain");
    let again = erasure_fence_service::drain(&pool, &store)
        .await
        .expect("drains");
    assert_eq!(again.claimed, 0);
}

/// ── WITNESS: the fence seeds from a `resource_erased` payload, unassisted ────────────────
/// The act runs with NO store, so nothing releases the bytes after the commit: the only
/// thing that can delete them is the fence deriving the delete from the `resource_erased`
/// event's `targets`.
/// FAILS IF the fence's seed scan drops `'resource_erased'` from its event-type `IN (...)`
/// (nothing seeds, `seeded` is 0, the bytes stay), or if it seeds under any event other than
/// the act's own, or if the drain does not delete the pathname.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_fence_seeds_a_resource_erased_strike_and_the_drain_deletes_the_bytes(pool: PgPool) {
    let owner = principal(&pool).await;
    let op = operator(&pool).await;
    let store = InMemoryBlobStore::default();
    let r = resource(&pool, &owner, "fence-seed").await;
    let (blob, pathname) = related_blob(&pool, &store, &owner, &[r]).await;

    let completion = completed(
        execute_resource_erasure(&pool, None, &op, request(r, &[blob]))
            .await
            .expect("the act completes"),
    );
    assert!(
        store.contains(&pathname),
        "no store was passed, so nothing released the bytes after the commit"
    );

    let summary = erasure_fence_service::drain(&pool, &store)
        .await
        .expect("the fence drains");
    assert_eq!(summary.seeded, 1, "seeded from the resource_erased payload");
    assert_eq!(summary.unparseable_verdicts, 0);
    assert_eq!(summary.deleted, 1, "the drain struck the pathname");
    assert!(!store.contains(&pathname), "the bytes are gone");

    let seeded_under: Vec<Uuid> = sqlx::query_scalar(
        "SELECT erasure_event_id FROM kb_erasure_blob_deletes WHERE pathname = $1",
    )
    .bind(&pathname)
    .fetch_all(&pool)
    .await
    .expect("fence rows");
    assert_eq!(
        seeded_under,
        vec![completion.event_id],
        "one queue row, keyed by the resource_erased event"
    );
}

/// ── WITNESS: a remainder-only related blob never seeds ──────────────────────────────────
/// Two related blobs; the operator lists neither. Both stay in `remainder` (D8), so the
/// payload's `targets` carry no blob and the fence has nothing to derive.
/// FAILS IF the act strikes an unlisted blob: the remainder would stop naming it, `targets`
/// would carry its verdict, and the fence would seed and delete its bytes.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_unlisted_related_blob_stays_in_the_remainder_and_is_never_seeded(pool: PgPool) {
    let owner = principal(&pool).await;
    let op = operator(&pool).await;
    let store = InMemoryBlobStore::default();
    let r = resource(&pool, &owner, "fence-remainder").await;
    let (first_blob, first) = related_blob(&pool, &store, &owner, &[r]).await;
    let (second_blob, second) = related_blob(&pool, &store, &owner, &[r]).await;

    let completion = completed(
        execute_resource_erasure(&pool, None, &op, request(r, &[]))
            .await
            .expect("the act completes"),
    );
    let mut named = related_blob_ids(&completion.remainder).expect("the plan's shape");
    named.sort();
    let mut both = vec![Uuid::from(first_blob), Uuid::from(second_blob)];
    both.sort();
    assert_eq!(
        named, both,
        "the remainder's kb_blobs entries are exactly the two related blobs"
    );
    assert!(
        completion.targets.iter().all(|t| t.target != "kb_blobs"),
        "an unlisted blob is never a target"
    );

    let summary = erasure_fence_service::drain(&pool, &store)
        .await
        .expect("the fence drains");
    assert_eq!(summary.seeded, 0, "nothing derives from the remainder");
    assert_eq!(summary.claimed, 0);
    assert_eq!(summary.deleted, 0);
    assert!(store.contains(&first) && store.contains(&second));
}

/// ── WITNESS: a committed act never answers as a failure ─────────────────────────────────
/// The provider misbehaves AFTER the act commits: its delete errors, or it never returns.
/// FAILS IF either turns the committed act into an error, or into a call that does not
/// return (the hang arm runs under a 200ms release bound inside a 30s outer limit, so an
/// unbounded release trips the outer bound): the answer must be `Completed`, carrying the
/// minted reference and the event id of the `resource_erased` correlated by it, with the
/// strike labelled from the act's own targets and the bytes left for the fence.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_failed_or_hung_post_commit_release_still_answers_completed(pool: PgPool) {
    let owner = principal(&pool).await;
    let op = operator(&pool).await;
    for mode in [BadDelete::Fails, BadDelete::Hangs] {
        let store = BadDeleteStore {
            inner: InMemoryBlobStore::default(),
            mode,
        };
        let r = resource(&pool, &owner, "bad-release").await;
        let (blob, pathname) = related_blob(&pool, &store.inner, &owner, &[r]).await;

        let outcome = tokio::time::timeout(
            Duration::from_secs(30),
            execute_with_release_timeout(
                &pool,
                Some(&store),
                &op,
                request(r, &[blob]),
                Duration::from_millis(200),
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("{mode:?}: the door must answer, not wait on the provider"))
        .unwrap_or_else(|e| panic!("{mode:?}: a committed act is not an error, got {e:?}"));
        let completion = completed(outcome);

        let (kind, correlation): (String, Uuid) = sqlx::query_as(
            "SELECT t.name, e.correlation_id FROM kb_events e \
               JOIN kb_event_types t ON t.id = e.event_type_id WHERE e.id = $1",
        )
        .bind(completion.event_id)
        .fetch_one(&pool)
        .await
        .expect("the completion's event");
        assert_eq!(kind, "resource_erased", "{mode:?}");
        assert_eq!(correlation, completion.request_reference, "{mode:?}");
        assert_eq!(
            completion.blob_strikes,
            vec![BlobStrikeOutcome {
                blob_id: Uuid::from(blob),
                released: true,
            }],
            "{mode:?}"
        );
        assert!(erased_at(&pool, r).await.is_some(), "{mode:?}");
        assert!(
            store.inner.contains(&pathname),
            "{mode:?}: the failed release left the bytes for the fence"
        );
    }
    assert_eq!(events_of(&pool, "resource_erasure_refused").await, 0);
}

/// ── WITNESS: a list naming one blob twice ───────────────────────────────────────────────
/// FAILS IF an operator's duplicate reaches the act (the SQL would strike the blob, then
/// raise on its second mention and answer the false "struck by an earlier act" 400), or if
/// the 400 appends anything. A caller who is not a system admin never reaches this check:
/// the door answers 404 first (`admin_resource_erasure_surface_test`). The last call — the
/// same blob listed once — completes.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_blob_listed_twice_is_refused_before_the_act(pool: PgPool) {
    let owner = principal(&pool).await;
    let op = operator(&pool).await;
    let store = InMemoryBlobStore::default();
    let r = resource(&pool, &owner, "duplicate").await;
    let (blob, _) = related_blob(&pool, &store, &owner, &[r]).await;

    let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(&pool)
        .await
        .expect("count");
    let err = execute_resource_erasure(&pool, Some(&store), &op, request(r, &[blob, blob]))
        .await
        .expect_err("an operator's duplicate is refused");
    let ApiError::BadRequest(msg) = &err else {
        panic!("a duplicate is a 400, got {err:?}");
    };
    assert!(msg.contains("listed more than once"), "{msg}");
    assert!(msg.contains(&blob.to_string()), "{msg}");
    assert_no_raise_literal(msg);
    let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(
        events_before, events_after,
        "the operator's 400 appended nothing"
    );
    assert!(erased_at(&pool, r).await.is_none());

    completed(
        execute_resource_erasure(&pool, Some(&store), &op, request(r, &[blob]))
            .await
            .expect("listed once, the act completes"),
    );
}
