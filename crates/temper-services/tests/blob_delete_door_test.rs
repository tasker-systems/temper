//! Integration test — the delete act's ruled door (`DELETE /api/blobs/{id}` through
//! `blob_service::delete_blob`), per the delete-act design ruled 2026-09-06
//! (`specs/2026-09-06-delete-act-design.md`, tasks
//! `01a08ba3-f977-7960-af41-9c3ae732716f` / goal
//! `01a07684-8baf-7a72-a6aa-8549a7635f04`).
//!
//! The witnesses pin the acceptance criteria INSIDE the build, each at the layer it bites:
//! the two custody arms strike (and nothing else does), the ledger tells the story in
//! exactly one `blob_deleted`, the strike folds no edge, the refusal faces speak the
//! assigned `blob_delete:` vocabulary, the relate door narrows to `kb_resources`, and the
//! byte fate rides the fence (a released strike seeds exactly one queue row; a failing
//! provider delete still leaves the act committed).
//!
//! Named remainder (declared, not silent): the prod first-fire is the doors task's
//! exercise, not a CI witness's.
#![cfg(feature = "test-db")]

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use sha2::Digest as _;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use temper_core::types::authorship::{ActContext, ActInput};
use temper_core::types::blob::BlobRelationAssertRequest;
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::ProfileId;
use temper_services::config::{BlobConfig, BlobCredentialMode};
use temper_services::services::blob_service;
use temper_substrate::blob_store::{BlobStore, InMemoryBlobStore, PutReceipt};
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::writes::{self, CreateParams};
use temper_workflow::operations::Surface;

// ── fixtures ────────────────────────────────────────────────────────────────────────

/// Seed a substrate profile + the emitter entities the write path resolves + a
/// profile-owned context (the `blob_surface_emitter_test` shape).
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (Uuid, Uuid, String) {
    let profile_id = Uuid::now_v7();
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
    let context_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id, handle)
}

/// A team + a team-owned context, with `members` joined DIRECTLY at the given roles —
/// the home arm's raw material (`kb_contexts.owner_table='kb_teams'`).
async fn seed_team_context(pool: &PgPool, slug: &str, members: &[(Uuid, &str)]) -> (Uuid, Uuid) {
    let team_id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1,$2,$3)")
        .bind(team_id)
        .bind(slug)
        .bind(slug)
        .execute(pool)
        .await
        .expect("seed team");
    for (profile, role) in members {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1,$2,$3::team_role)",
        )
        .bind(team_id)
        .bind(profile)
        .bind(role)
        .execute(pool)
        .await
        .expect("seed team member");
    }
    let context_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_teams',$2,$3,$3)",
    )
    .bind(context_id)
    .bind(team_id)
    .bind(slug)
    .execute(pool)
    .await
    .expect("seed team context");
    (team_id, context_id)
}

/// The caller's emitter entity id (`<handle>@web`) — `writes::create_resource_with` needs
/// an `EntityId`, and the profile fixture pre-seeds the trio.
async fn emitter_of(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_entities WHERE name = $1")
        .bind(format!("{handle}@web"))
        .fetch_one(pool)
        .await
        .expect("seeded web emitter entity")
}

/// A resource homed in `home`, through the REAL create path (the erasure fixture's shape,
/// minus the embedding — the relate door's peer gate only asks readability).
async fn seed_resource(pool: &PgPool, home: Uuid, owner: Uuid, emitter: Uuid, title: &str) -> Uuid {
    let prose = "prose";
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &format!("test://{title}"),
            body: prose,
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: Some(vec![IncomingChunk {
                chunk_index: 0,
                content_hash: format!("{:x}", sha2::Sha256::digest(prose)),
                content: prose.to_string(),
                embedding: vec![0.1; 768],
                embedded_with: Some("model-sha-1".to_string()),
                header_path: String::new(),
                heading_depth: 0,
            }]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("seed resource through the create path");
    sqlx::query_scalar("SELECT id FROM kb_resources WHERE origin_uri = $1")
        .bind(format!("test://{title}"))
        .fetch_one(pool)
        .await
        .expect("created resource row")
}

fn blob_cfg() -> BlobConfig {
    BlobConfig {
        store_id: "store_test".to_string(),
        read_write_token: Some("vercel_rw_test_store_test".to_string()),
        credential_mode: BlobCredentialMode::Token,
        oidc_token_source: Arc::new(|| None),
        max_bytes: 1 << 20,
        allowlist: vec!["image/png".to_string()],
        single_request_max_bytes: 64 * 1024,
    }
}

async fn commit_blob(
    pool: &PgPool,
    store: &InMemoryBlobStore,
    home: Uuid,
    caller: Uuid,
    bytes: &[u8],
) -> temper_services::services::blob_service::BlobCommitOutcome {
    blob_service::commit_blob(
        pool,
        store,
        &blob_cfg(),
        blob_service::BlobCommitCommand {
            caller: ProfileId::from(caller),
            home_table: Some("kb_contexts".to_string()),
            home_id: Some(home.to_string()),
            content_type: "image/png".to_string(),
            bytes: Bytes::copy_from_slice(bytes),
            surface: Surface::ApiHttp,
        },
    )
    .await
    .expect("commit through the service")
}

async fn relate(
    pool: &PgPool,
    caller: Uuid,
    blob: Uuid,
    peer_table: &str,
    peer_id: Uuid,
) -> Result<temper_core::types::blob::BlobRelationAck, temper_services::error::ApiError> {
    blob_service::relate_blob(
        pool,
        ProfileId::from(caller),
        temper_core::types::ids::BlobId::from(blob),
        &BlobRelationAssertRequest {
            direction: temper_core::types::blob::BlobRelationDirection::BlobAsSource,
            peer_table: peer_table.to_string(),
            peer_id,
            edge_kind: EdgeKind::Express,
            polarity: Polarity::Forward,
            label: "figure_of".to_string(),
            weight: 1.0,
            act: ActInput::default(),
        },
        ActContext::default(),
        Surface::ApiHttp,
    )
    .await
}

async fn delete_blob(
    pool: &PgPool,
    store: &dyn BlobStore,
    caller: Uuid,
    blob: Uuid,
) -> Result<temper_core::types::blob::BlobDeleteAck, temper_services::error::ApiError> {
    blob_service::delete_blob(
        pool,
        ProfileId::from(caller),
        temper_core::types::ids::BlobId::from(blob),
        store,
        ActContext::default(),
        Surface::ApiHttp,
    )
    .await
}

/// The struck row: `(pathname, content_type, content_bytes, content_hash, home_table,
/// owner_profile_id)` — the D5.2 emptied shape is the witness's business.
async fn row_of(
    pool: &PgPool,
    blob: Uuid,
) -> (
    Option<String>,
    Option<String>,
    Option<i64>,
    String,
    String,
    Uuid,
) {
    let row = sqlx::query(
        "SELECT blob_pathname, content_type, content_bytes, content_hash, home_table, \
         owner_profile_id FROM kb_blobs WHERE id = $1",
    )
    .bind(blob)
    .fetch_one(pool)
    .await
    .expect("struck row");
    (
        row.get(0),
        row.get(1),
        row.get(2),
        row.get(3),
        row.get(4),
        row.get(5),
    )
}

/// How many `blob_deleted` events the ledger carries for this blob — the whole story of a
/// strike is ONE of these, and a second would be a violation, never a retry.
async fn blob_deleted_count(pool: &PgPool, blob: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e \
         JOIN kb_event_types t ON t.id = e.event_type_id \
         WHERE t.name = 'blob_deleted' AND e.payload->>'blob_id' = $1",
    )
    .bind(blob.to_string())
    .fetch_one(pool)
    .await
    .expect("blob_deleted count")
}

/// The fence's queue state for a pathname: `(pending_count, any_count)`.
async fn fence_rows(pool: &PgPool, pathname: &str) -> (i64, i64) {
    let any: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_erasure_blob_deletes WHERE pathname = $1")
            .bind(pathname)
            .fetch_one(pool)
            .await
            .expect("fence rows");
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_erasure_blob_deletes \
         WHERE pathname = $1 AND status = 'pending'",
    )
    .bind(pathname)
    .fetch_one(pool)
    .await
    .expect("pending fence rows");
    (pending, any)
}

/// A store whose delete ALWAYS fails — the post-commit release's honest adversary: the act
/// must still commit, and the fence row must stand in for the bytes.
#[derive(Default)]
struct FailingDeleteStore {
    inner: InMemoryBlobStore,
}

impl std::fmt::Debug for FailingDeleteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FailingDeleteStore")
    }
}

#[async_trait]
impl BlobStore for FailingDeleteStore {
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
    async fn get(
        &self,
        pathname: &str,
        consistent: bool,
    ) -> anyhow::Result<temper_substrate::blob_store::ByteStream> {
        self.inner.get(pathname, consistent).await
    }
    async fn head(
        &self,
        pathname: &str,
    ) -> anyhow::Result<Option<temper_substrate::blob_store::BlobHead>> {
        self.inner.head(pathname).await
    }
    async fn delete(&self, _pathnames: &[&str]) -> anyhow::Result<()> {
        anyhow::bail!("provider outage: the bytes will not go")
    }
}

// ── the witnesses ───────────────────────────────────────────────────────────────────

/// A custodian strikes an ATTACHED blob through the relation arm: the row empties into the
/// D5.2 shape (hash/home/owner kept), exactly one `blob_deleted` fires, the edge is NOT
/// folded, the provider bytes release, and one fence row stands pending.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_custodian_strikes_an_attached_blob_through_the_relation_arm(pool: PgPool) {
    let (owner, ctx, handle) = seed_profile_with_context(&pool, "relation-arm@example.com").await;
    let emitter = emitter_of(&pool, &handle).await;
    let resource = seed_resource(&pool, ctx, owner, emitter, "relation-arm-resource").await;
    let store = InMemoryBlobStore::default();
    let committed = commit_blob(&pool, &store, ctx, owner, b"attached-bytes").await;
    let pathname = temper_substrate::blob_store::blob_pathname(&committed.content_hash);
    let edge = relate(
        &pool,
        owner,
        committed.blob_id.uuid(),
        "kb_resources",
        resource,
    )
    .await
    .expect("relate the blob to its resource");

    let ack = delete_blob(&pool, &store, owner, committed.blob_id.uuid())
        .await
        .expect("the resource-home custodian deletes the attached blob");
    assert!(
        ack.released,
        "the struck row was the last live row on its hash"
    );

    let (pathname_col, ctype, cbytes, hash, home_table, owner_col) =
        row_of(&pool, committed.blob_id.uuid()).await;
    assert_eq!(pathname_col, None, "the pathname is nulled");
    assert_eq!(
        ctype, None,
        "the content type is nulled (the D5.2 emptied marker)"
    );
    assert_eq!(cbytes, None, "the bytes column is nulled");
    assert_eq!(hash, committed.content_hash, "the hash survives");
    assert_eq!(home_table, "kb_contexts", "the home survives");
    assert_eq!(owner_col, owner, "attribution survives a delete");
    assert_eq!(blob_deleted_count(&pool, committed.blob_id.uuid()).await, 1);
    let folded: bool = sqlx::query_scalar("SELECT is_folded FROM kb_edges WHERE id = $1")
        .bind(edge.edge_handle)
        .fetch_one(&pool)
        .await
        .expect("the related edge");
    assert!(
        !folded,
        "the strike folds no edge — the strike performs no edits"
    );
    assert!(!store.contains(&pathname), "the provider bytes released");
    let (pending, any) = fence_rows(&pool, &pathname).await;
    assert_eq!(
        (pending, any),
        (1, 1),
        "exactly one pending fence row seeds"
    );
}

/// A custodian strikes an UNATTACHED blob through the home arm (a personal context's
/// owner), with the same one-event, D5.2, released shape.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_custodian_strikes_an_unattached_blob_through_the_home_arm(pool: PgPool) {
    let (owner, ctx, _handle) = seed_profile_with_context(&pool, "home-arm@example.com").await;
    let store = InMemoryBlobStore::default();
    let committed = commit_blob(&pool, &store, ctx, owner, b"unattached-bytes").await;

    let ack = delete_blob(&pool, &store, owner, committed.blob_id.uuid())
        .await
        .expect("the personal context's owner deletes the unattached blob");
    assert!(ack.released);
    assert_eq!(blob_deleted_count(&pool, committed.blob_id.uuid()).await, 1);
    let (pathname_col, ctype, _, _, _, _) = row_of(&pool, committed.blob_id.uuid()).await;
    assert_eq!((pathname_col.is_none(), ctype.is_none()), (true, true));
}

/// The relation arm reads delete standing off the RESOURCE's home owner — so a team OWNER
/// (blob-home custody) still cannot strike a blob attached to a resource whose home they
/// do not own: team-owner custody never confers delete over a resource, and role never
/// widens deletion.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_team_owner_cannot_strike_an_attached_blob_their_custody_does_not_reach(pool: PgPool) {
    let (owner, _pctx, _owner_handle) =
        seed_profile_with_context(&pool, "team-owner@example.com").await;
    let (member, mctx, member_handle) =
        seed_profile_with_context(&pool, "team-member@example.com").await;
    let (_team, tctx) = seed_team_context(
        &pool,
        "doorteam-attached",
        &[(owner, "owner"), (member, "member")],
    )
    .await;
    let member_emitter = emitter_of(&pool, &member_handle).await;
    // The resource's home is the MEMBER's personal context — the team owner holds no
    // delete standing over it.
    let resource = seed_resource(
        &pool,
        mctx,
        member,
        member_emitter,
        "team-owner-refused-resource",
    )
    .await;
    let store = InMemoryBlobStore::default();
    let committed = commit_blob(&pool, &store, tctx, member, b"team-attached-bytes").await;
    relate(
        &pool,
        member,
        committed.blob_id.uuid(),
        "kb_resources",
        resource,
    )
    .await
    .expect("the member attaches the blob to their own resource");

    let err = delete_blob(&pool, &store, owner, committed.blob_id.uuid())
        .await
        .expect_err("the team owner holds no delete standing over the member's resource");
    match err {
        temper_services::error::ApiError::ForbiddenDetail(msg) => {
            assert!(
                msg.starts_with("blob_delete: "),
                "the refusal speaks the assigned vocabulary, got: {msg}"
            );
        }
        other => panic!("expected a ForbiddenDetail custody refusal, got {other:?}"),
    }
    assert_eq!(blob_deleted_count(&pool, committed.blob_id.uuid()).await, 0);
    let (pathname_col, ctype, _, _, _, _) = row_of(&pool, committed.blob_id.uuid()).await;
    assert!(
        pathname_col.is_some() && ctype.is_some(),
        "a refused strike mutates nothing"
    );
    // The same refusal would meet a maintainer: role never widens deletion (the
    // maintainer-widening rejection stands) — asserted through the vocabulary's one voice
    // by the custody predicate, not by a second arm here.

    // The RESOURCE's home owner (the member) CAN strike through the relation arm.
    let ack = delete_blob(&pool, &store, member, committed.blob_id.uuid())
        .await
        .expect("the resource-home custodian strikes");
    assert!(ack.released);
}

/// The home arm resolves the owning team's OWNER ROLE — and only it: the owner strikes an
/// unattached team-homed blob; a maintainer and a member are refused (the
/// maintainer-widening rejection stands); an unrelated profile is refused.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_home_arm_answers_only_to_the_team_owner_role(pool: PgPool) {
    let (owner, _, _) = seed_profile_with_context(&pool, "home-owner@example.com").await;
    let (maintainer, _, _) = seed_profile_with_context(&pool, "home-maintainer@example.com").await;
    let (member, _, _) = seed_profile_with_context(&pool, "home-member@example.com").await;
    let (unrelated, _, _) = seed_profile_with_context(&pool, "home-unrelated@example.com").await;
    let (_team, tctx) = seed_team_context(
        &pool,
        "doorteam-unattached",
        &[
            (owner, "owner"),
            (maintainer, "maintainer"),
            (member, "member"),
        ],
    )
    .await;
    let store = InMemoryBlobStore::default();
    let committed = commit_blob(&pool, &store, tctx, member, b"team-unattached-bytes").await;

    // Maintainer and member can READ the team context, so their refusals are the custody
    // face (403, the assigned vocabulary); the unrelated profile is NOT a reader, so they
    // meet the read floor's 404 instead — renders-absent, nothing disclosed.
    for (actor, expectation) in [
        (maintainer, "a maintainer is excluded"),
        (member, "a member is excluded"),
    ] {
        let err = delete_blob(&pool, &store, actor, committed.blob_id.uuid())
            .await
            .expect_err(expectation);
        assert!(
            matches!(
                err,
                temper_services::error::ApiError::ForbiddenDetail(ref m) if m.starts_with("blob_delete: ")
            ),
            "the refusal speaks the assigned vocabulary, got {err:?}"
        );
        assert_eq!(
            blob_deleted_count(&pool, committed.blob_id.uuid()).await,
            0,
            "{expectation}; no second act fired"
        );
    }
    {
        let err = delete_blob(&pool, &store, unrelated, committed.blob_id.uuid())
            .await
            .expect_err("an unrelated profile is excluded");
        assert!(
            matches!(err, temper_services::error::ApiError::NotFound(_)),
            "a non-reader reads absent — the same 404 an unknown id gets, got {err:?}"
        );
        assert_eq!(blob_deleted_count(&pool, committed.blob_id.uuid()).await, 0);
    }

    let ack = delete_blob(&pool, &store, owner, committed.blob_id.uuid())
        .await
        .expect("the owning team's owner role is the home arm's custodian");
    assert!(ack.released);
}

/// ADMIN STANDING CONFERS NO CUSTODY (the ruled trap): a profile that satisfies the real
/// `is_system_admin` predicate — and, as a team member, can even READ and COMMIT the
/// blob — is refused by the strike gate. The gate never consults admin standing; custody
/// is relation/home-derived alone.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn admin_standing_confers_no_custody(pool: PgPool) {
    let (admin, _, _) = seed_profile_with_context(&pool, "admin-no-custody@example.com").await;
    temper_services::test_support::approved_admin(&pool, admin).await;
    assert!(
        temper_services::services::access_service::is_system_admin(&pool, ProfileId::from(admin))
            .await
            .expect("is_system_admin resolves"),
        "the fixture must be a REAL system admin for this witness to bite"
    );
    // The ADMIN is a plain member of the team, so they can commit (container-write) and
    // read — the refusal they meet is the custody face, not the read floor's 404. The
    // team's OWNER is the only custodian.
    let (team_owner, _, _) =
        seed_profile_with_context(&pool, "admin-witness-owner@example.com").await;
    let (_team, tctx) = seed_team_context(
        &pool,
        "admindoorteam",
        &[(admin, "member"), (team_owner, "owner")],
    )
    .await;
    let store = InMemoryBlobStore::default();
    let committed = commit_blob(&pool, &store, tctx, admin, b"admin-refused-bytes").await;

    let err = delete_blob(&pool, &store, admin, committed.blob_id.uuid())
        .await
        .expect_err("admin standing is not custody — even for a reader and committer");
    assert!(
        matches!(
            err,
            temper_services::error::ApiError::ForbiddenDetail(ref m) if m.starts_with("blob_delete: ")
        ),
        "the refusal speaks the assigned vocabulary, got {err:?}"
    );
    assert_eq!(blob_deleted_count(&pool, committed.blob_id.uuid()).await, 0);
    let (_, ctype, _, _, _, _) = row_of(&pool, committed.blob_id.uuid()).await;
    assert!(ctype.is_some(), "a refused strike mutates nothing");

    // Sanity: the team's owner role strikes what the admin's standing could not.
    let ack = delete_blob(&pool, &store, team_owner, committed.blob_id.uuid())
        .await
        .expect("the owning team's owner role is the custodian");
    assert!(ack.released);
}

/// Already-struck and unknown read the SAME 404 and fire NO second event — the
/// renders-absent posture: a distinguishable answer would make the door a struck-row
/// existence oracle.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn already_struck_and_unknown_read_the_same_404_with_no_second_event(pool: PgPool) {
    let (owner, ctx, _) = seed_profile_with_context(&pool, "no-second-event@example.com").await;
    let store = InMemoryBlobStore::default();
    let committed = commit_blob(&pool, &store, ctx, owner, b"struck-once-bytes").await;
    delete_blob(&pool, &store, owner, committed.blob_id.uuid())
        .await
        .expect("the first strike");
    assert_eq!(blob_deleted_count(&pool, committed.blob_id.uuid()).await, 1);

    let again = delete_blob(&pool, &store, owner, committed.blob_id.uuid())
        .await
        .expect_err("an already-struck blob reads absent");
    let unknown = delete_blob(&pool, &store, owner, Uuid::now_v7())
        .await
        .expect_err("an unknown id reads absent");
    assert!(
        matches!(again, temper_services::error::ApiError::NotFound(_)),
        "already-struck is 404, got {again:?}"
    );
    assert!(
        matches!(unknown, temper_services::error::ApiError::NotFound(_)),
        "unknown is the same 404, got {unknown:?}"
    );
    assert_eq!(
        blob_deleted_count(&pool, committed.blob_id.uuid()).await,
        1,
        "no second event — the one blob_deleted is the whole story"
    );
}

/// The relate door narrows blob-relation peers to `kb_resources` (ruled): a cogmap or blob
/// peer is refused in the `blob_relate:` vocabulary — no delete standing resolves over
/// such a peer, so the edge would pin its row permanently.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_relate_door_refuses_non_kb_resources_blob_peers(pool: PgPool) {
    let (owner, ctx, _handle) = seed_profile_with_context(&pool, "narrow-relate@example.com").await;
    let store = InMemoryBlobStore::default();
    let committed = commit_blob(&pool, &store, ctx, owner, b"narrow-bytes").await;
    let peer_blob = commit_blob(&pool, &store, ctx, owner, b"narrow-peer-bytes").await;

    for peer_table in ["kb_cogmaps", "kb_blobs"] {
        let peer_id = if peer_table == "kb_blobs" {
            peer_blob.blob_id.uuid()
        } else {
            Uuid::now_v7()
        };
        let err = relate(&pool, owner, committed.blob_id.uuid(), peer_table, peer_id)
            .await
            .expect_err("a non-kb_resources peer is refused");
        match err {
            temper_services::error::ApiError::BadRequest(msg) => {
                assert!(
                    msg.starts_with("blob_relate: ") && msg.contains("kb_resources"),
                    "the narrowing refusal names the vocabulary and the admitted peer, got: {msg}"
                );
            }
            other => panic!("expected the relate narrowing refusal, got {other:?}"),
        }
    }
}

/// A released strike seeds exactly ONE fence row; a HELD strike (another live home still
/// references the hash) seeds none and keeps the bytes — the refcount bounds the byte
/// fate, and the queue never deletes another home's object.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_held_strike_seeds_no_fence_row_and_keeps_the_bytes(pool: PgPool) {
    let (owner_a, ctx_a, _) = seed_profile_with_context(&pool, "held-a@example.com").await;
    let (owner_b, ctx_b, _) = seed_profile_with_context(&pool, "held-b@example.com").await;
    let store = InMemoryBlobStore::default();
    let a = commit_blob(&pool, &store, ctx_a, owner_a, b"shared-bytes").await;
    let b = commit_blob(&pool, &store, ctx_b, owner_b, b"shared-bytes").await;
    let pathname = temper_substrate::blob_store::blob_pathname(&a.content_hash);

    let ack = delete_blob(&pool, &store, owner_a, a.blob_id.uuid())
        .await
        .expect("A's custodian strikes A's row");
    assert!(
        !ack.released,
        "another live home still references the hash — the bytes were never this act's"
    );
    assert!(
        store.contains(&pathname),
        "the bytes stay: B's live row holds them"
    );
    let (pending, any) = fence_rows(&pool, &pathname).await;
    assert_eq!((pending, any), (0, 0), "a held strike seeds no queue row");
    assert_eq!(blob_deleted_count(&pool, a.blob_id.uuid()).await, 1);

    // B's row is untouched and live; B's custodian striking it now releases the bytes.
    let ack = delete_blob(&pool, &store, owner_b, b.blob_id.uuid())
        .await
        .expect("B's custodian strikes B's row");
    assert!(ack.released);
    assert!(
        !store.contains(&pathname),
        "the last live row's strike releases"
    );
}

/// A FAILING provider delete leaves the act committed and the fence row pending — the
/// ruled caller-side release is best-effort; retry plus age alerting is the fence's, and
/// a post-commit provider failure is never a door refusal.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_failing_provider_delete_still_leaves_the_act_committed(pool: PgPool) {
    let (owner, ctx, _) = seed_profile_with_context(&pool, "failing-store@example.com").await;
    let store = FailingDeleteStore::default();
    let committed = {
        blob_service::commit_blob(
            &pool,
            &store,
            &blob_cfg(),
            blob_service::BlobCommitCommand {
                caller: ProfileId::from(owner),
                home_table: Some("kb_contexts".to_string()),
                home_id: Some(ctx.to_string()),
                content_type: "image/png".to_string(),
                bytes: Bytes::from_static(b"failing-release-bytes"),
                surface: Surface::ApiHttp,
            },
        )
        .await
        .expect("commit through the service")
    };
    let pathname = temper_substrate::blob_store::blob_pathname(&committed.content_hash);

    let ack = delete_blob(&pool, &store, owner, committed.blob_id.uuid())
        .await
        .expect("the strike commits even though the provider delete will fail");
    assert!(ack.released);
    assert_eq!(blob_deleted_count(&pool, committed.blob_id.uuid()).await, 1);
    let (pending, any) = fence_rows(&pool, &pathname).await;
    assert_eq!(
        (pending, any),
        (1, 1),
        "the fence row stands in for the unreleased bytes"
    );
}

/// A post-delete re-commit of identical bytes mints a FRESH live row and is never refused
/// by, nor deduplicated against, the struck one (`delete-replay-reproduces-absence`'s
/// synchronous face — the emptied row reads ABSENT to get-or-create).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_post_delete_recommit_mints_a_fresh_live_row(pool: PgPool) {
    let (owner, ctx, _) = seed_profile_with_context(&pool, "recommit@example.com").await;
    let store = InMemoryBlobStore::default();
    let first = commit_blob(&pool, &store, ctx, owner, b"recommit-bytes").await;
    delete_blob(&pool, &store, owner, first.blob_id.uuid())
        .await
        .expect("the strike");

    let second = commit_blob(&pool, &store, ctx, owner, b"recommit-bytes").await;
    assert_ne!(
        first.blob_id, second.blob_id,
        "the struck row is never deduplicated against"
    );
    let (_, ctype, _, _, _, _) = row_of(&pool, second.blob_id.uuid()).await;
    assert_eq!(ctype.as_deref(), Some("image/png"), "the fresh row is live");
}

/// A non-custodian who cannot even READ a personal blob gets the read floor's own 404 —
/// renders-absent, nothing disclosed, nothing mutated (the custody refusal's other face,
/// the visibility one, is the team roles' witness above).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_personal_blob_discloses_nothing_to_a_non_reader(pool: PgPool) {
    let (owner, ctx, _) = seed_profile_with_context(&pool, "personal-owner@example.com").await;
    let (other, _, _) = seed_profile_with_context(&pool, "personal-other@example.com").await;
    let store = InMemoryBlobStore::default();
    let committed = commit_blob(&pool, &store, ctx, owner, b"personal-bytes").await;

    let err = delete_blob(&pool, &store, other, committed.blob_id.uuid())
        .await
        .expect_err("another profile's personal blob is not theirs to strike");
    assert!(
        matches!(err, temper_services::error::ApiError::NotFound(_)),
        "a non-reader reads absent — the same 404 an unknown id gets, got {err:?}"
    );
    assert_eq!(blob_deleted_count(&pool, committed.blob_id.uuid()).await, 0);
    let (_, ctype, _, _, _, _) = row_of(&pool, committed.blob_id.uuid()).await;
    assert!(ctype.is_some(), "the refused attempt mutated nothing");
}
