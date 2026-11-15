//! Conditional body saves through `DbBackend::update_resource`: a `PATCH` that carries
//! `expect.body` (or `If-Match`) lands only if the resource's readable body is still exactly the
//! bytes its token names, judged under the resource's writer lock after the caller's read
//! standing, and otherwise writes nothing.
//!
//! Every witness drives the backend command the `PATCH` door builds, and reads tokens from the
//! reads the doors serve (`/content` and `?sections=body`), so a token here is one a caller can
//! hold. The HTTP header rules and the `ETag` are witnessed in temper-api.
#![cfg(feature = "test-db")]

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sqlx::PgPool;

use temper_core::error::TemperError;
use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_core::types::precondition::{PreconditionFailure, PreconditionTier, ResourceExpect};
use temper_core::types::resource_view::{ResourceSection, SectionSet};
use temper_services::backend::{substrate_read, DbBackend};
use temper_services::test_support;
use temper_workflow::operations::{
    Backend, BodyUpdate, CreateResource, ShowResource, Surface, UpdateResource,
};
use temper_workflow::types::managed_meta::ManagedMeta;
use uuid::Uuid;

// ── fixtures ─────────────────────────────────────────────────────────────────────────────────────

/// Seed a substrate profile, its emitters, and a profile-owned context. Each test-target crate
/// keeps its own copy (`meta_precondition_test.rs`).
async fn seed_profile(pool: &PgPool, email: &str) -> (Uuid, Uuid) {
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
         VALUES ($1,'kb_profiles',$2,'drafts','drafts')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

fn backend(pool: &PgPool, profile: Uuid) -> DbBackend {
    DbBackend::new(pool.clone(), ProfileId::from(profile))
}

/// A research resource in `context` with `body`.
async fn create(pool: &PgPool, owner: Uuid, context: Uuid, body: &str) -> ResourceId {
    backend(pool, owner)
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: "draft".to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: "draft".to_string(),
            body: Some(BodyUpdate::new(body)),
            managed_meta: ManagedMeta::default(),
            open_meta: Some(json!({"lease": "mine"})),
            goal: None,
            origin_uri: Some(format!("test://draft/{}", Uuid::now_v7())),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create")
        .value
        .id
}

/// Three sections, so the body lands as three blocks.
const THREE: &str =
    "# One\n\nfirst section\n\n# Two\n\nsecond section\n\n# Three\n\nthird section\n";

/// The `PATCH` a whole-body save builds: `content` and nothing else, guarded by `token`.
fn save(resource: ResourceId, content: &str, token: Option<&str>) -> UpdateResource {
    UpdateResource {
        resource,
        title: None,
        slug: None,
        body: Some(BodyUpdate::new(content)),
        managed_meta: None,
        open_meta: None,
        open_meta_add: None,
        goal: None,
        move_to: None,
        context_ref: None,
        expect: token.map(|t| ResourceExpect {
            body: Some(t.to_string()),
            ..Default::default()
        }),
        if_match: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

/// The same save, guarded by an `If-Match` header value instead.
fn save_if_match(resource: ResourceId, content: &str, header: &str) -> UpdateResource {
    UpdateResource {
        if_match: Some(header.to_string()),
        ..save(resource, content, None)
    }
}

/// `GET /api/resources/{id}/content`: the markdown and its token.
async fn content(pool: &PgPool, reader: Uuid, resource: ResourceId) -> (String, String) {
    let read = substrate_read::get_content_select(pool, ProfileId::from(reader), resource)
        .await
        .expect("read content");
    let token = read.body_token.expect("a /content read answers a token");
    (read.markdown, token)
}

/// `GET /api/resources/{id}?sections=body`: the view's content and token.
async fn view_body(pool: &PgPool, reader: Uuid, resource: ResourceId) -> (String, String) {
    let sections: SectionSet = [ResourceSection::Body].into_iter().collect();
    let view = substrate_read::show_view_select(pool, ProfileId::from(reader), resource, &sections)
        .await
        .expect("read the view");
    (
        view.content.expect("content"),
        view.body_token.expect("the body section answers a token"),
    )
}

async fn token_of(pool: &PgPool, reader: Uuid, resource: ResourceId) -> String {
    content(pool, reader, resource).await.1
}

fn failure(answer: Result<impl std::fmt::Debug, TemperError>) -> PreconditionFailure {
    match answer {
        Err(TemperError::PreconditionFailed(f)) => f,
        other => panic!("expected PreconditionFailed, got {other:?}"),
    }
}

fn bad_request(answer: Result<impl std::fmt::Debug, TemperError>) -> String {
    match answer {
        Err(TemperError::BadRequest(m)) => m,
        other => panic!("expected BadRequest, got {other:?}"),
    }
}

/// Every event on the ledger (each test has its own database).
async fn events(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .expect("count events")
}

/// The event types recorded after the first `after` events, in order.
async fn event_types_since(pool: &PgPool, after: i64) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT t.name FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          ORDER BY e.id OFFSET $1",
    )
    .bind(after)
    .fetch_all(pool)
    .await
    .expect("event types")
}

/// The body entry of a refusal, asserting it is shaped as one: tier `body`, no key.
fn body_entry(f: &PreconditionFailure) -> (Value, Value) {
    let entry = f
        .failed
        .iter()
        .find(|e| e.tier == PreconditionTier::Body)
        .expect("a body entry");
    assert_eq!(entry.key, None, "a body entry carries no key");
    (entry.expected.clone(), entry.current.clone())
}

async fn grant_context_write(pool: &PgPool, context: Uuid, grantee: Uuid, by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, true, $3)",
    )
    .bind(context)
    .bind(grantee)
    .bind(by)
    .execute(pool)
    .await
    .expect("grant context write");
}

async fn hold_writer_lock(
    pool: &PgPool,
    resource: ResourceId,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut held = pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1 FOR NO KEY UPDATE")
        .bind(Uuid::from(resource))
        .execute(&mut *held)
        .await
        .expect("take the writer lock");
    held
}

/// Poll until `n` backends wait on R's writer lock (a row-lock wait on `lock_resource_for_write`'s
/// statement), never on an advisory lock. As in `meta_precondition_test.rs`.
async fn writers_wait_on_the_row_lock(pool: &PgPool, n: i64) {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let advisory: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock' \
                AND wait_event = 'advisory'",
        )
        .fetch_one(pool)
        .await
        .expect("poll advisory waits");
        assert_eq!(advisory, 0, "a writer waited on an advisory lock");
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock' \
                AND wait_event IN ('tuple', 'transactionid') \
                AND query ILIKE '%FROM kb_resources WHERE id = $1 FOR NO KEY UPDATE%'",
        )
        .fetch_one(pool)
        .await
        .expect("poll row-lock waits");
        if waiting >= n {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{waiting} of {n} writers waited on the writer lock within 4 seconds"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// A map in the admin-only regime with a telos owned by someone else, and a system admin who may
/// therefore modify the telos but cannot read it. As in `meta_precondition_test.rs`; the standing
/// is asserted.
async fn modify_but_not_read(pool: &PgPool) -> (Uuid, ResourceId) {
    let (admin, _) = seed_profile(pool, "regime-admin@example.com").await;
    test_support::approved_admin(pool, admin).await;
    let (owner, _) = seed_profile(pool, "regime-owner@example.com").await;
    let (cogmap, telos) = birth_map(pool, owner).await;
    let team = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, 'regime-gate', 'regime-gate')")
        .bind(team)
        .execute(pool)
        .await
        .expect("seed the gating team");
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .expect("bind the map");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'regime-gate' WHERE id = 1")
        .execute(pool)
        .await
        .expect("point the gate at it");
    let (modify, read): (bool, bool) = sqlx::query_as(
        "SELECT can_modify_resource($1, $2), \
                EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(admin)
    .bind(Uuid::from(telos))
    .fetch_one(pool)
    .await
    .expect("standing");
    assert!(
        modify && !read,
        "the fixture is modify-but-not-read: modify {modify}, read {read}"
    );
    (admin, telos)
}

/// A cognitive map owned by `owner`, born with a telos. Returns `(cogmap, telos)`.
async fn birth_map(pool: &PgPool, owner: Uuid) -> (Uuid, ResourceId) {
    let cogmap = Uuid::now_v7();
    let telos = Uuid::now_v7();
    sqlx::query(
        "SELECT cogmap_genesis($1, '{}'::jsonb, \
             (SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
               WHERE p.handle = 'system' AND e.name = 'system'))",
    )
    .bind(json!({
        "cogmap_id": cogmap,
        "name": format!("map-{cogmap}"),
        "owner_profile_id": owner,
        "telos": {"resource_id": telos, "title": "telos",
                  "origin_uri": format!("temper://test/{cogmap}/telos"), "blocks": []},
    }))
    .execute(pool)
    .await
    .expect("birth the map");
    (cogmap, ResourceId::from(telos))
}

/// The owner's `@cli` emitter entity, for the substrate writes the backend has no door for here.
async fn emitter(pool: &PgPool, owner: Uuid) -> temper_core::types::ids::EntityId {
    let profile = temper_substrate::writes::resolve_profile(pool, owner)
        .await
        .expect("profile");
    temper_substrate::writes::resolve_emitter(pool, profile, "cli")
        .await
        .expect("emitter")
}

// ── the guard ────────────────────────────────────────────────────────────────────────────────────

/// **A stale body save never lands.** A save guarded by the token of the body it read lands; a
/// second save guarded by that same, now stale, token is refused `412` and writes nothing. The
/// refusal's body entry carries no key, `expected` is the token sent and `current` is the token
/// `/content` answers now.
///
/// FAILS IF the body guard is skipped, judged against anything but the current readable bytes, or
/// a refusal writes.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_stale_body_save_never_lands(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "stale@example.com").await;
    let resource = create(&pool, owner, context, "# Draft\n\nfirst words\n").await;
    let be = backend(&pool, owner);
    let (_, read) = content(&pool, owner, resource).await;

    let landed = be
        .update_resource(save(resource, "# Draft\n\nmine\n", Some(&read)))
        .await
        .expect("a fresh token lands")
        .value;
    let (now, current) = content(&pool, owner, resource).await;
    assert_eq!(now, "# Draft\n\nmine\n");
    assert_eq!(landed.body_token.as_deref(), Some(current.as_str()));

    let before = events(&pool).await;
    let refused = failure(
        be.update_resource(save(resource, "# Draft\n\ntheirs\n", Some(&read)))
            .await,
    );
    assert_eq!(refused.failed.len(), 1);
    assert_eq!(body_entry(&refused), (json!(read), json!(current)));
    assert_eq!(refused.message(), "1 precondition failed: body");
    assert_eq!(events(&pool).await, before, "a refusal writes nothing");
    assert_eq!(content(&pool, owner, resource).await.0, "# Draft\n\nmine\n");
}

/// **Two concurrent saves of one read, two owners: exactly one lands.** A side transaction holds
/// R's writer lock; A and B each save over the body they both read, guarded by its token, and both
/// are seen waiting on that row lock before it is released. One answers 200; the other answers 412
/// with `current` equal to the winner's token, and the winner's bytes are what is stored.
///
/// FAILS IF the body guard is judged before `lock_resource_for_write` (on the pool, or after
/// `begin_floored` alone): both savers then read the same body and both land.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn two_concurrent_saves_of_one_read_exactly_one_lands(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "race-owner@example.com").await;
    let (other, _) = seed_profile(&pool, "race-other@example.com").await;
    grant_context_write(&pool, context, other, owner).await;
    let resource = create(&pool, owner, context, "# Doc\n\nshared\n").await;
    let read = token_of(&pool, owner, resource).await;

    let held = hold_writer_lock(&pool, resource).await;
    let spawn = |who: Uuid, body: &'static str| {
        let pool = pool.clone();
        let read = read.clone();
        tokio::spawn(async move {
            (
                body,
                backend(&pool, who)
                    .update_resource(save(resource, body, Some(&read)))
                    .await,
            )
        })
    };
    let a = spawn(owner, "# Doc\n\nowner's\n");
    let b = spawn(other, "# Doc\n\nother's\n");
    writers_wait_on_the_row_lock(&pool, 2).await;
    held.commit().await.expect("release");
    let answers = [a.await.unwrap(), b.await.unwrap()];
    let landed: Vec<&str> = answers
        .iter()
        .filter(|(_, r)| r.is_ok())
        .map(|(body, _)| *body)
        .collect();
    assert_eq!(landed.len(), 1, "exactly one save lands: {answers:?}");
    let winner = landed[0];
    let (_, refused) = answers.into_iter().find(|(b, _)| *b != winner).unwrap();
    assert_eq!(
        body_entry(&failure(refused)),
        (json!(read), json!(temper_core::body_token(winner)))
    );
    assert_eq!(content(&pool, owner, resource).await.0, winner);
}

/// **All or nothing, across the tiers.** One request guards a meta key and the body; it lands only
/// if both hold, and a refusal lists every failed expectation, meta before body.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn all_or_nothing_across_meta_and_body(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "both@example.com").await;
    let resource = create(&pool, owner, context, "# Doc\n\nv1\n").await;
    let be = backend(&pool, owner);
    let token = token_of(&pool, owner, resource).await;

    let both = |meta: Value, body: &str| ResourceExpect {
        open: serde_json::from_value(meta).unwrap(),
        body: Some(body.to_string()),
        ..Default::default()
    };
    let guarded = |e: ResourceExpect| UpdateResource {
        expect: Some(e),
        open_meta: Some(json!({"lease": "next"})),
        ..save(resource, "# Doc\n\nv2\n", None)
    };
    let before = events(&pool).await;

    // The body holds, the meta guard does not: nothing lands.
    let f = failure(
        be.update_resource(guarded(both(json!({"lease": "theirs"}), &token)))
            .await,
    );
    assert_eq!(f.failed.len(), 1);
    assert_eq!(f.failed[0].tier, PreconditionTier::Open);
    // The meta guard holds, the body does not: nothing lands.
    let f = failure(
        be.update_resource(guarded(both(json!({"lease": "mine"}), "bt1.stale")))
            .await,
    );
    assert_eq!(f.failed.len(), 1);
    assert_eq!(body_entry(&f).0, json!("bt1.stale"));
    // Neither holds: both are named, open before body.
    let f = failure(
        be.update_resource(guarded(both(json!({"lease": "theirs"}), "bt1.stale")))
            .await,
    );
    let tiers: Vec<_> = f.failed.iter().map(|e| e.tier).collect();
    assert_eq!(tiers, vec![PreconditionTier::Open, PreconditionTier::Body]);
    assert_eq!(f.message(), "2 preconditions failed: open.lease, body");
    assert_eq!(events(&pool).await, before, "no refusal wrote");
    assert_eq!(content(&pool, owner, resource).await.0, "# Doc\n\nv1\n");

    // Both hold: everything lands.
    be.update_resource(guarded(both(json!({"lease": "mine"}), &token)))
        .await
        .expect("both hold");
    assert_eq!(content(&pool, owner, resource).await.0, "# Doc\n\nv2\n");
}

/// **The refusal says what failed, and nothing else.** A body entry's members are exactly `tier`,
/// `expected` and `current`; the message names the tier and never a token.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_body_refusal_carries_tokens_and_nothing_else(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "says@example.com").await;
    let resource = create(&pool, owner, context, "# Doc\n\nbody\n").await;
    let f = failure(
        backend(&pool, owner)
            .update_resource(save(resource, "# Doc\n\nnew\n", Some("bt1.not-it")))
            .await,
    );
    let wire = serde_json::to_value(&f).unwrap();
    let entry = wire["failed"][0].as_object().expect("an entry").clone();
    let mut members: Vec<_> = entry.keys().cloned().collect();
    members.sort();
    assert_eq!(members, vec!["current", "expected", "tier"]);
    assert_eq!(entry["tier"], json!("body"));
    let current = token_of(&pool, owner, resource).await;
    assert_eq!(entry["current"], json!(current));
    let message = f.message();
    assert!(!message.contains("bt1"), "{message}");
}

/// **A landed conditional body save is ordinary**: the same events and the same answer as the same
/// save without a precondition, on a twin resource.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_landed_conditional_body_save_is_ordinary(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "ordinary@example.com").await;
    let guarded = create(&pool, owner, context, THREE).await;
    let plain = create(&pool, owner, context, THREE).await;
    let be = backend(&pool, owner);
    let token = token_of(&pool, owner, guarded).await;
    let edited = THREE.replace("second section", "second, edited");

    let n = events(&pool).await;
    let a = be
        .update_resource(save(guarded, &edited, Some(&token)))
        .await
        .expect("guarded save")
        .value;
    let guarded_events = event_types_since(&pool, n).await;
    let n = events(&pool).await;
    let b = be
        .update_resource(save(plain, &edited, None))
        .await
        .expect("plain save")
        .value;
    assert_eq!(event_types_since(&pool, n).await, guarded_events);
    assert_eq!(a.body_token, b.body_token, "the same answer token");
    assert_eq!(a.bytes_merkle, b.bytes_merkle);
    assert_eq!(a.open_meta, b.open_meta);
}

/// **A hash is never the token.** The view's `body_hash` and `bytes_merkle` describe the same body,
/// but neither is accepted as `expect.body`; only the token is.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_hash_is_never_the_token(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "hash@example.com").await;
    let resource = create(&pool, owner, context, "# Doc\n\nbody\n").await;
    let be = backend(&pool, owner);
    let view = be
        .show_resource(ShowResource {
            resource,
            origin: Surface::ApiHttp,
        })
        .await
        .expect("show")
        .value;
    let mut hashes: Vec<String> = view.body_hash.clone().into_iter().collect();
    hashes.extend(view.bytes_merkle.clone());
    let (markdown, token) = content(&pool, owner, resource).await;
    hashes.push(temper_core::hash::compute_body_hash(&markdown));
    hashes.push(temper_core::hash::sha256_hex(markdown.as_bytes()));
    for hash in hashes {
        assert_ne!(hash, token);
        let f = failure(
            be.update_resource(save(resource, "# Doc\n\nnew\n", Some(&hash)))
                .await,
        );
        assert_eq!(body_entry(&f), (json!(hash), json!(token)));
    }
}

// ── the token moves exactly with the readable bytes ─────────────────────────────────────────────

/// **It moves on every change to the readable bytes**: an edit, a heading-only edit, an append, a
/// charter set, and an erasure that empties the bytes in place. Each time the token `/content`
/// answers is the token of the bytes it answers, and differs from the one before.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_token_moves_on_every_change_to_the_readable_bytes(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "moves@example.com").await;
    let resource = create(&pool, owner, context, THREE).await;
    let be = backend(&pool, owner);
    let mut seen = vec![token_of(&pool, owner, resource).await];
    let mut step = |label: &str, (markdown, token): (String, String)| {
        assert_eq!(token, temper_core::body_token(&markdown), "{label}");
        assert!(!seen.contains(&token), "{label}: the token did not move");
        seen.push(token);
    };

    let edited = THREE.replace("second section", "second section, edited");
    be.update_resource(save(resource, &edited, None))
        .await
        .expect("edit");
    step("an edit", content(&pool, owner, resource).await);

    let heading = edited.replace("# Two", "# Two, renamed");
    be.update_resource(save(resource, &heading, None))
        .await
        .expect("heading-only edit");
    step("a heading-only edit", content(&pool, owner, resource).await);

    let next_seq: i32 = sqlx::query_scalar(
        "SELECT max(seq) + 1 FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded",
    )
    .bind(Uuid::from(resource))
    .fetch_one(&pool)
    .await
    .expect("next seq");
    let block = temper_substrate::content::prepare_block_deferred(
        next_seq,
        None,
        "# Four\n\nappended section\n",
    );
    temper_substrate::writes::append_block(
        &pool,
        temper_substrate::writes::AppendParams {
            resource,
            block: &block,
            sources: vec![],
            emitter: emitter(&pool, owner).await,
        },
    )
    .await
    .expect("append");
    step("an append", content(&pool, owner, resource).await);

    // An erasure that empties the bytes in place: the redaction arms set every revision's stored
    // block bytes and chunk text to '' and leave the live-block set and revisions as they are.
    sqlx::query(
        "UPDATE kb_block_content bc SET content = '', content_hash = encode(sha256(''::bytea), 'hex') \
           FROM kb_content_blocks b \
          WHERE bc.block_revision_id = b.current_revision_id AND b.resource_id = $1",
    )
    .bind(Uuid::from(resource))
    .execute(&pool)
    .await
    .expect("empty the bytes in place");
    sqlx::query(
        "UPDATE kb_chunk_content cc SET content = '' FROM kb_chunks c \
          WHERE cc.chunk_id = c.id AND c.resource_id = $1",
    )
    .bind(Uuid::from(resource))
    .execute(&pool)
    .await
    .expect("empty the chunk text in place");
    step("an erasure in place", content(&pool, owner, resource).await);

    // A charter set, on a map's telos.
    let (cogmap, telos) = birth_map(&pool, owner).await;
    let mut seen_telos = vec![token_of(&pool, owner, telos).await];
    let charter = [temper_substrate::content::prepare_block_deferred(
        0,
        Some("purpose"),
        "# Purpose\n\nwhat this map is for\n",
    )];
    let mut tx = pool.begin().await.expect("begin");
    temper_substrate::writes::set_charter_in_tx(
        &mut tx,
        temper_core::types::ids::CogmapId::from(cogmap),
        &charter,
        emitter(&pool, owner).await,
        Default::default(),
    )
    .await
    .expect("set the charter");
    tx.commit().await.expect("commit");
    let (markdown, token) = content(&pool, owner, telos).await;
    assert_eq!(token, temper_core::body_token(&markdown));
    assert!(
        !seen_telos.contains(&token),
        "a charter set moves the token"
    );
    seen_telos.push(token);
}

/// **It does not move on anything else**: a meta write, an edge write, or a re-block that keeps the
/// bytes.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_token_holds_across_writes_that_keep_the_bytes(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "holds@example.com").await;
    let resource = create(&pool, owner, context, THREE).await;
    let other = create(&pool, owner, context, "# Other\n\nelsewhere\n").await;
    let be = backend(&pool, owner);
    let token = token_of(&pool, owner, resource).await;

    be.update_resource(UpdateResource {
        body: None,
        open_meta: Some(json!({"lease": "theirs", "tags": ["a"]})),
        ..save(resource, "", None)
    })
    .await
    .expect("a meta write");
    assert_eq!(
        token_of(&pool, owner, resource).await,
        token,
        "a meta write"
    );

    be.assert_relationship(temper_workflow::operations::AssertRelationship {
        source: resource,
        target: other,
        target_table: temper_core::types::relationship_requests::RelationshipTarget::Resource,
        edge_kind: temper_core::types::graph::EdgeKind::Near,
        polarity: temper_core::types::graph::Polarity::Forward,
        label: "near".to_string(),
        weight: 1.0,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    })
    .await
    .expect("an edge write");
    assert_eq!(
        token_of(&pool, owner, resource).await,
        token,
        "an edge write"
    );

    let blocks_before: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded ORDER BY seq",
    )
    .bind(Uuid::from(resource))
    .fetch_all(&pool)
    .await
    .expect("blocks");
    temper_substrate::writes::reblock_resource(
        &pool,
        temper_substrate::writes::ReblockParams {
            resource,
            emitter: emitter(&pool, owner).await,
        },
    )
    .await
    .expect("re-block");
    assert_eq!(token_of(&pool, owner, resource).await, token, "a re-block");
    assert!(!blocks_before.is_empty());
}

/// **The `/content` token is accepted on a resource with a byteless block.** A block whose current
/// revision has no stored bytes makes the body `derived`; the token `/content` answers for that
/// reconstruction guards a save, which then lands.
///
/// FAILS IF the guard hashes anything but the bytes `/content` serves: the write path's own
/// live-block read, for one, has no bytes for that block.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_content_token_guards_a_resource_with_a_byteless_block(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "byteless@example.com").await;
    let resource = create(&pool, owner, context, THREE).await;
    // A legacy block: its current revision carries no stored bytes.
    sqlx::query(
        "DELETE FROM kb_block_content bc USING kb_content_blocks b \
          WHERE bc.block_revision_id = b.current_revision_id AND b.resource_id = $1 \
            AND b.seq = (SELECT min(seq) FROM kb_content_blocks \
                          WHERE resource_id = $1 AND NOT is_folded)",
    )
    .bind(Uuid::from(resource))
    .execute(&pool)
    .await
    .expect("drop one block's bytes");
    let (derived, token) = content(&pool, owner, resource).await;
    assert_ne!(derived, THREE, "the body now reads back derived");
    assert_eq!(view_body(&pool, owner, resource).await.1, token);

    let edited = "# One\n\nrewritten\n";
    let answer = backend(&pool, owner)
        .update_resource(save(resource, edited, Some(&token)))
        .await
        .expect("the /content token holds")
        .value;
    let (now, current) = content(&pool, owner, resource).await;
    assert_eq!(now, edited);
    assert_eq!(answer.body_token.as_deref(), Some(current.as_str()));
}

// ── the answer's token ───────────────────────────────────────────────────────────────────────────

/// **A write's answer token is its own.** A whole-body save answers the token of the body it left;
/// a per-block save answers the token of the WHOLE body, never of the block it sent; a meta-only
/// `PATCH` answers no token. Each answer token equals the one `/content` and `?sections=body`
/// answer next.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_writes_answer_token_is_its_own(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "answer@example.com").await;
    let resource = create(&pool, owner, context, THREE).await;
    let be = backend(&pool, owner);

    let whole = THREE.replace("first section", "first, whole");
    let a = be
        .update_resource(save(resource, &whole, None))
        .await
        .expect("whole-body save")
        .value;
    assert_eq!(a.body_token, Some(temper_core::body_token(&whole)));
    assert_eq!(a.body_token, Some(token_of(&pool, owner, resource).await));
    assert_eq!(
        a.content, None,
        "the answer carries the token, not the body"
    );

    let block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded \
          ORDER BY seq DESC LIMIT 1",
    )
    .bind(Uuid::from(resource))
    .fetch_one(&pool)
    .await
    .expect("last block");
    let piece = "# Three\n\nthird, per block\n";
    let mut per_block = save(resource, piece, None);
    per_block.body.as_mut().unwrap().content_block = Some(block);
    let b = be
        .update_resource(per_block)
        .await
        .expect("per-block save")
        .value;
    let (now, current) = content(&pool, owner, resource).await;
    assert_ne!(now, piece, "the body is more than the block");
    assert_eq!(b.body_token.as_deref(), Some(current.as_str()));
    assert_ne!(b.body_token, Some(temper_core::body_token(piece)));
    assert_eq!(view_body(&pool, owner, resource).await, (now, current));

    let meta_only = be
        .update_resource(UpdateResource {
            body: None,
            open_meta: Some(json!({"lease": "theirs"})),
            ..save(resource, "", None)
        })
        .await
        .expect("meta-only")
        .value;
    assert_eq!(
        meta_only.body_token, None,
        "a meta-only PATCH answers no token"
    );
}

/// **The answer's token is the write's, even with a writer queued behind it.** The second save
/// waits on the first's lock; each answer's token describes the bytes that write left, and the
/// first's is stale by the time the second answers.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_answer_token_never_describes_a_later_write(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "queued@example.com").await;
    let resource = create(&pool, owner, context, "# Doc\n\nv0\n").await;
    let held = hold_writer_lock(&pool, resource).await;
    let spawn = |body: &'static str| {
        let pool = pool.clone();
        tokio::spawn(async move {
            backend(&pool, owner)
                .update_resource(save(resource, body, None))
                .await
                .expect("save")
                .value
        })
    };
    let first = spawn("# Doc\n\nv1\n");
    writers_wait_on_the_row_lock(&pool, 1).await;
    let second = spawn("# Doc\n\nv2\n");
    writers_wait_on_the_row_lock(&pool, 2).await;
    held.commit().await.expect("release");
    let (a, b) = (first.await.unwrap(), second.await.unwrap());
    let tokens = [a.body_token.unwrap(), b.body_token.unwrap()];
    let v1 = temper_core::body_token("# Doc\n\nv1\n");
    let v2 = temper_core::body_token("# Doc\n\nv2\n");
    assert!(
        tokens == [v1.clone(), v2.clone()] || tokens == [v2, v1],
        "each answer names its own bytes"
    );
    let now = token_of(&pool, owner, resource).await;
    assert!(tokens.contains(&now));
}

// ── retried saves ────────────────────────────────────────────────────────────────────────────────

/// **A retried body-only whole-body save that already landed gets `200` and writes nothing.** Its
/// token is stale (it names the body before its own first attempt), but its bytes are the body
/// now. The answer carries the current token. The same retry carrying anything else, a title or a
/// meta change, is refused `412`, as is a per-block save: a no-op answer would drop the rest.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_retried_save_that_landed_is_answered_200_and_writes_nothing(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "retry@example.com").await;
    let resource = create(&pool, owner, context, THREE).await;
    let be = backend(&pool, owner);
    let read = token_of(&pool, owner, resource).await;
    let mine = THREE.replace("third section", "third, mine");

    be.update_resource(save(resource, &mine, Some(&read)))
        .await
        .expect("the first attempt lands");
    let landed = token_of(&pool, owner, resource).await;

    let before = events(&pool).await;
    let retry = be
        .update_resource(save(resource, &mine, Some(&read)))
        .await
        .expect("the retry is answered 200")
        .value;
    assert_eq!(retry.body_token.as_deref(), Some(landed.as_str()));
    assert_eq!(events(&pool).await, before, "the retry wrote nothing");

    // The same through If-Match.
    be.update_resource(save_if_match(resource, &mine, &format!("\"{read}\"")))
        .await
        .expect("the header retry is answered 200");
    assert_eq!(events(&pool).await, before);

    // Anything besides the body: 412.
    let with_title = UpdateResource {
        title: Some("renamed".into()),
        ..save(resource, &mine, Some(&read))
    };
    body_entry(&failure(be.update_resource(with_title).await));
    let with_meta = UpdateResource {
        open_meta: Some(json!({"lease": "next"})),
        ..save(resource, &mine, Some(&read))
    };
    body_entry(&failure(be.update_resource(with_meta).await));
    // A per-block save: 412, even with the block's bytes already in place.
    let block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded \
          ORDER BY seq LIMIT 1",
    )
    .bind(Uuid::from(resource))
    .fetch_one(&pool)
    .await
    .expect("block");
    let mut per_block = save(resource, &mine, Some(&read));
    per_block.body.as_mut().unwrap().content_block = Some(block);
    body_entry(&failure(be.update_resource(per_block).await));
    // Different bytes: 412.
    body_entry(&failure(
        be.update_resource(save(resource, THREE, Some(&read))).await,
    ));
    assert_eq!(events(&pool).await, before, "no refusal wrote");
    assert_eq!(token_of(&pool, owner, resource).await, landed);
}

/// The retry's no-op still meets the ingest-state refusal a whole-body save meets: a resource whose
/// ingest has not completed refuses the save, never answers it as landed.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_retried_save_on_an_incomplete_ingest_is_refused_as_any_save_is(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "ingest@example.com").await;
    let resource = create(&pool, owner, context, THREE).await;
    sqlx::query("UPDATE kb_resources SET ingest_state = 'in_progress' WHERE id = $1")
        .bind(Uuid::from(resource))
        .execute(&pool)
        .await
        .expect("mark the ingest incomplete");
    let (markdown, _) = content(&pool, owner, resource).await;
    let answer = backend(&pool, owner)
        .update_resource(save(resource, &markdown, Some("bt1.stale")))
        .await;
    assert!(
        matches!(answer, Err(ref e) if !matches!(e, TemperError::PreconditionFailed(_))
            && e.to_string().contains("not complete")),
        "the ingest refusal, not a no-op and not a 412: {answer:?}"
    );
}

// ── If-Match ─────────────────────────────────────────────────────────────────────────────────────

/// **`If-Match` is `expect.body`'s synonym, with the header's own rules.** A list matches on any
/// member; one pair of quotes is stripped; a weak tag naming the current body is a `400`, never a
/// `412` a re-reading client would loop on; a weak tag naming another is a `412`; `*` is a `400`;
/// a header that disagrees with `expect.body` is a `400`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn if_match_is_judged_as_expect_body_with_the_headers_rules(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "ifmatch@example.com").await;
    let resource = create(&pool, owner, context, "# Doc\n\nv0\n").await;
    let be = backend(&pool, owner);
    let before = events(&pool).await;
    let t = token_of(&pool, owner, resource).await;

    let msg = bad_request(
        be.update_resource(save_if_match(
            resource,
            "# Doc\n\nx\n",
            &format!("W/\"{t}\""),
        ))
        .await,
    );
    assert!(
        msg.contains("weak entity-tag") && !msg.contains(&t),
        "{msg}"
    );
    let f = failure(
        be.update_resource(save_if_match(resource, "# Doc\n\nx\n", "W/\"bt1.other\""))
            .await,
    );
    assert_eq!(body_entry(&f), (json!("bt1.other"), json!(t)));
    for refused in ["*", "\"open", "", "\"a\" junk"] {
        bad_request(
            be.update_resource(save_if_match(resource, "# Doc\n\nx\n", refused))
                .await,
        );
    }
    let mut both = save(resource, "# Doc\n\nx\n", Some(&t));
    both.if_match = Some("\"bt1.other\"".into());
    let msg = bad_request(be.update_resource(both).await);
    assert!(msg.contains("If-Match and expect.body"), "{msg}");
    let f = failure(
        be.update_resource(save_if_match(
            resource,
            "# Doc\n\nx\n",
            "\"bt1.a\", \"bt1.b\"",
        ))
        .await,
    );
    assert_eq!(body_entry(&f), (json!(["bt1.a", "bt1.b"]), json!(t)));
    assert_eq!(events(&pool).await, before, "nothing above wrote");

    // A list with the current token anywhere in it holds; so does the agreeing pair.
    be.update_resource(save_if_match(
        resource,
        "# Doc\n\nv1\n",
        &format!("\"bt1.a\", \"{t}\""),
    ))
    .await
    .expect("a list matches on any member");
    let t1 = token_of(&pool, owner, resource).await;
    let mut both = save(resource, "# Doc\n\nv2\n", Some(&t1));
    both.if_match = Some(format!("\"{t1}\""));
    be.update_resource(both)
        .await
        .expect("agreeing guards land");
    assert_eq!(content(&pool, owner, resource).await.0, "# Doc\n\nv2\n");
}

// ── read standing first ──────────────────────────────────────────────────────────────────────────

/// **Read standing first.** A caller who may modify a resource it cannot read gets GET's `404` for
/// any body guard, `expect.body` or `If-Match`, matching or not, and never a `412`; it learns no
/// token. The unlocked pre-check answers it without waiting on a held writer lock.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_modifier_who_cannot_read_gets_404_for_any_body_guard(pool: PgPool) {
    let (admin, telos) = modify_but_not_read(&pool).await;
    let be = backend(&pool, admin);
    let get = be
        .show_resource(ShowResource {
            resource: telos,
            origin: Surface::ApiHttp,
        })
        .await
        .expect_err("the admin cannot read the telos")
        .to_string();
    // The token that would match, computed as the owner would read it.
    let matching = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM kb_profiles WHERE email = 'regime-owner@example.com'",
    )
    .fetch_one(&pool)
    .await
    .expect("owner");
    let matching = token_of(&pool, matching, telos).await;

    let held = hold_writer_lock(&pool, telos).await;
    let guards = [
        save(telos, "# Telos\n\nx\n", Some(&matching)),
        save(telos, "# Telos\n\nx\n", Some("bt1.wrong")),
        save_if_match(telos, "# Telos\n\nx\n", &format!("\"{matching}\"")),
        save_if_match(telos, "# Telos\n\nx\n", "\"bt1.wrong\""),
        save_if_match(telos, "# Telos\n\nx\n", &format!("W/\"{matching}\"")),
    ];
    for cmd in guards {
        let asked = Instant::now();
        let answer = be.update_resource(cmd).await;
        assert!(
            asked.elapsed() < Duration::from_secs(2),
            "the pre-check answered without waiting on the held writer lock"
        );
        match answer {
            Err(e @ TemperError::NotFound(_)) => assert_eq!(e.to_string(), get, "GET's 404"),
            other => panic!("expected GET's 404, got {other:?}"),
        }
    }
    held.commit().await.expect("release");
}

/// **The locked gate decides.** A reader passes the pre-check and waits on the writer lock; its
/// read standing is withdrawn while it waits. Under the lock it gets GET's `404`, never the `412`
/// its stale token would earn, and so never the current token.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn read_standing_lost_while_waiting_answers_404_not_412(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "gate-owner@example.com").await;
    let (guest, _) = seed_profile(&pool, "gate-guest@example.com").await;
    grant_context_write(&pool, context, guest, owner).await;
    let resource = create(&pool, owner, context, "# Doc\n\nowner's\n").await;

    let held = hold_writer_lock(&pool, resource).await;
    let request = {
        let pool = pool.clone();
        tokio::spawn(async move {
            backend(&pool, guest)
                .update_resource(save(resource, "# Doc\n\nguest's\n", Some("bt1.stale")))
                .await
        })
    };
    writers_wait_on_the_row_lock(&pool, 1).await;
    sqlx::query("DELETE FROM kb_access_grants WHERE principal_id = $1")
        .bind(guest)
        .execute(&pool)
        .await
        .expect("withdraw the guest's standing");
    held.commit().await.expect("release");
    match request.await.unwrap() {
        Err(TemperError::NotFound(_)) => {}
        other => panic!("expected the locked gate's 404, got {other:?}"),
    }
    assert_eq!(
        content(&pool, owner, resource).await.0,
        "# Doc\n\nowner's\n"
    );
}

/// **The order of refusals.** The modify floor first; then the body condition's own shape
/// (`If-Match: *`, unparseable, disagreeing with `expect.body`), which reads no state; then
/// "nothing to write"; then read standing; then the precondition.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_order_of_body_refusals(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "order@example.com").await;
    let (stranger, _) = seed_profile(&pool, "order-stranger@example.com").await;
    let resource = create(&pool, owner, context, "# Doc\n\nv0\n").await;

    // A stranger who may not modify gets the floor's answer, whatever the header says.
    for header in ["*", "\"bt1.x\"", "W/\"bt1.x\""] {
        match backend(&pool, stranger)
            .update_resource(save_if_match(resource, "# Doc\n\nx\n", header))
            .await
        {
            Err(TemperError::Forbidden) | Err(TemperError::ForbiddenDetail(_)) => {}
            other => panic!("expected the floor's 403 for {header:?}, got {other:?}"),
        }
    }
    let be = backend(&pool, owner);
    // `expect` with only a body guard on a request that writes nothing: the nothing-to-write 400.
    let nothing = UpdateResource {
        body: None,
        ..save(resource, "", Some("bt1.x"))
    };
    let msg = bad_request(be.update_resource(nothing).await);
    assert!(msg.contains("writes nothing"), "{msg}");
    // An empty `expect`: names no key.
    let empty = UpdateResource {
        expect: Some(ResourceExpect::default()),
        ..save(resource, "# Doc\n\nx\n", None)
    };
    let msg = bad_request(be.update_resource(empty).await);
    assert!(msg.contains("names no key"), "{msg}");
}

// ── the same response ────────────────────────────────────────────────────────────────────────────

/// **The body section's token is of the content beside it**: the view read answers `content` and
/// `body_token` from one assembler call, equal to what `/content` answers for the same state.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_body_section_and_content_agree(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "agree@example.com").await;
    let resource = create(&pool, owner, context, THREE).await;
    let (markdown, token) = view_body(&pool, owner, resource).await;
    assert_eq!(token, temper_core::body_token(&markdown));
    assert_eq!(content(&pool, owner, resource).await, (markdown, token));
    // Without the body section, no token.
    let view = substrate_read::show_view_select(
        &pool,
        ProfileId::from(owner),
        resource,
        &SectionSet::default(),
    )
    .await
    .expect("view");
    assert_eq!(view.body_token, None);
    assert_eq!(view.content, None);
}
