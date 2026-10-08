#![cfg(feature = "test-db")]
//! The read of an ERASED resource (a husk: `kb_resources.erased_at` set) answers `410` under
//! `RESOURCE_ERASED`, but only to a caller who holds standing on it (resource erasure spec D6/D7;
//! `resource_husk_held_by`, migration `20260930000060`). Every other caller keeps the `404` an
//! unknown id gets, so the `410` tells a caller without standing nothing. Standing is decided at
//! read time (D6), so who holds it can grow after the act; that is ruled, not witnessed here.
//!
//! Witnessed on the three doors that read a resource, `GET /api/resources/{id}`,
//! `GET /api/resources/{id}/content` and `GET /api/resources/{id}/meta`, and on the block read
//! `GET /api/resources/{id}/blocks/{block_id}`, whose home-resource miss classifies the same way:
//!
//! * the owner of a husk gets `410` + `RESOURCE_ERASED`, and the body names the id and nothing
//!   else — neither the original title nor the body text;
//! * a direct read-grant holder gets the same `410`;
//! * a member of the resource's home context without a grant gets a `404` whose status and body
//!   equal an unknown id's `404`, once each body's own id is replaced by a placeholder;
//! * the owner of a tombstone (soft-deleted through the real delete door, never erased) gets
//!   `404`, not `410`.
//!
//! Every state is made by a real door: the resource by `POST /api/ingest`, the grant by
//! `POST /api/resources/{id}/grants`, the husk by the operator door
//! `POST /api/admin/resources/erasure`, the tombstone by `DELETE /api/resources/{id}`. Nothing
//! writes `is_active` or `erased_at` by hand. The team, its context and the memberships are
//! fixture rows, as in `context_team_owned_resource_visibility_test.rs`.

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};

const TITLE: &str = "Husk Read Subject Title";
const BODY: &str = "Prose the erasure act must take off the wire entirely.";

/// The code a husk read travels under — the one constant producer and consumer share.
const RESOURCE_ERASED: &str = temper_core::error::RESOURCE_ERASED_CODE;

/// An already-embedded chunk carrying the REAL chunker hash, so the test-db tier needs no ONNX
/// (the pattern in `soft_delete_read_floor_test.rs`).
fn chunk(content: &str) -> PackedChunk {
    let c = &temper_ingest::chunk::chunk_markdown(content)[0];
    PackedChunk {
        chunk_index: 0,
        header_path: c.header_path.clone(),
        heading_depth: c.heading_depth,
        content: c.content.clone(),
        content_hash: c.content_hash.clone(),
        embedding: vec![0.5; 768],
        embedded_with: None,
    }
}

struct Caller {
    token: String,
    profile: Uuid,
}

/// A fully provisioned profile (approved standing, `<handle>@web` emitter, own context) and its JWT.
async fn caller(pool: &PgPool, label: &str) -> (Caller, Uuid) {
    let email = format!("husk-{label}-{}@example.com", Uuid::new_v4());
    let (profile, own_context) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);
    (Caller { token, profile }, own_context)
}

/// A team that owns a context, with each profile a `member` (an authoring role, so the owner may
/// write into it; every member reads it). Returns the context id.
async fn team_context(pool: &PgPool, members: &[Uuid]) -> Uuid {
    let team = Uuid::now_v7();
    let slug = format!("husk-team-{}", &team.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $2)")
        .bind(team)
        .bind(&slug)
        .execute(pool)
        .await
        .expect("insert team");
    let context = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_teams', $2, 'husk-home', 'husk-home')",
    )
    .bind(context)
    .bind(team)
    .execute(pool)
    .await
    .expect("insert team-owned context");
    for member in members {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
        )
        .bind(team)
        .bind(member)
        .execute(pool)
        .await
        .expect("add team member");
    }
    context
}

/// A resource with a real body, made through `POST /api/ingest` by `owner` into `context`.
async fn ingest(app: &common::TestApp, owner: &Caller, context: Uuid) -> Uuid {
    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        title: TITLE.to_string(),
        origin_uri: format!("test://husk-read-{}", Uuid::new_v4()),
        context_ref: context.to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        content: BODY.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk(BODY)]).expect("pack")),
        goal: None,
        act: Default::default(),
        sources: Vec::new(),
    };
    let resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {}", owner.token))
        .json(&payload)
        .send()
        .await
        .expect("ingest request");
    assert_eq!(resp.status().as_u16(), 200, "the owner ingests");
    let created: Value = resp.json().await.expect("ingest JSON");
    Uuid::parse_str(created["id"].as_str().expect("id field")).expect("resource id")
}

/// `owner` grants `grantee` read on `resource` through the real grant door.
async fn grant_read(app: &common::TestApp, owner: &Caller, resource: Uuid, grantee: Uuid) {
    let resp = app
        .client
        .post(app.url(&format!("/api/resources/{resource}/grants")))
        .header("Authorization", format!("Bearer {}", owner.token))
        .json(&json!({
            "principal_table": "kb_profiles",
            "principal_id": grantee,
            "can_read": true,
            "can_write": false,
            "can_delete": false,
            "can_grant": false,
        }))
        .send()
        .await
        .expect("grant request");
    assert_eq!(resp.status().as_u16(), 200, "the owner may grant read");
}

/// Erase `resource` through the operator door, as a fresh instance operator.
async fn erase(app: &common::TestApp, resource: Uuid) {
    let (operator, _) = caller(&app.pool, "operator").await;
    common::fixtures::make_test_admin(&app.pool, operator.profile).await;
    let resp = app
        .client
        .post(app.url("/api/admin/resources/erasure"))
        .header("Authorization", format!("Bearer {}", operator.token))
        .json(&json!({ "resource": resource }))
        .send()
        .await
        .expect("erasure request");
    assert_eq!(resp.status().as_u16(), 200, "the operator's act answers");
    let body: Value = resp.json().await.expect("the tagged outcome");
    assert_eq!(body["status"], "completed", "the act completed: {body}");
}

/// Soft-delete `resource` through the real delete door.
async fn delete(app: &common::TestApp, owner: &Caller, resource: Uuid) {
    let resp = app
        .client
        .delete(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {}", owner.token))
        .send()
        .await
        .expect("delete request");
    assert_eq!(resp.status().as_u16(), 200, "the owner deletes");
}

/// The three reads under test, for one resource. `/meta` composes from the same
/// `show_view_select` as the show door, so it inherits the husk answer; listing it here puts it
/// under every witness below.
fn reads(resource: Uuid) -> [String; 3] {
    [
        format!("/api/resources/{resource}"),
        format!("/api/resources/{resource}/content"),
        format!("/api/resources/{resource}/meta"),
    ]
}

/// `GET path` as `who`: the status and the body text exactly as the client receives it.
async fn get(app: &common::TestApp, who: &Caller, path: &str) -> (u16, String) {
    let resp = app
        .client
        .get(app.url(path))
        .header("Authorization", format!("Bearer {}", who.token))
        .send()
        .await
        .expect("read request");
    let status = resp.status().as_u16();
    (status, resp.text().await.expect("read body"))
}

/// Assert `who` gets the husk answer on every read: `410`, `RESOURCE_ERASED`, a body whose only
/// field is the error with exactly `code` and `message`, a message naming the id alone, and none
/// of the original title or body text anywhere in the bytes.
async fn assert_husk_410(app: &common::TestApp, who: &Caller, resource: Uuid, label: &str) {
    for path in reads(resource) {
        assert_husk_410_at(app, who, resource, &path, label).await;
    }
}

/// [`assert_husk_410`] for one `path` reading `resource`.
async fn assert_husk_410_at(
    app: &common::TestApp,
    who: &Caller,
    resource: Uuid,
    path: &str,
    label: &str,
) {
    let (status, text) = get(app, who, path).await;
    assert_eq!(status, 410, "{label}: {path} answers 410; body: {text}");
    let body: Value = serde_json::from_str(&text).expect("410 body is JSON");
    assert_eq!(body["error"]["code"], RESOURCE_ERASED, "{label}: {path}");
    assert_eq!(
        body["error"]["message"],
        format!("resource {resource} was erased"),
        "{label}: {path} — the fixed message names the id and nothing else"
    );
    let mut keys: Vec<&str> = body["error"]
        .as_object()
        .expect("error object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["code", "message"],
        "{label}: {path} — no details ride the 410"
    );
    assert_eq!(
        body.as_object().expect("body object").len(),
        1,
        "{label}: {path}"
    );
    assert!(
        !text.contains(TITLE),
        "{label}: {path} leaked the title: {text}"
    );
    assert!(
        !text.contains(BODY),
        "{label}: {path} leaked the body: {text}"
    );
}

/// Assert `who` reads the live resource on every door — the precondition that makes a later
/// `404` or `410` about the erasure, not about a caller the fixture never admitted.
async fn assert_reads_live(app: &common::TestApp, who: &Caller, resource: Uuid, label: &str) {
    for path in reads(resource) {
        let (status, text) = get(app, who, &path).await;
        assert_eq!(
            status, 200,
            "{label}: reads {path} while live; body: {text}"
        );
    }
}

/// Assert `who` gets, on every read of `resource`, exactly the `404` an unknown id gets: the
/// same status and the same body bytes once each body's own id is replaced by `<id>`.
async fn assert_unknown_id_404(app: &common::TestApp, who: &Caller, resource: Uuid, label: &str) {
    let unknown = Uuid::now_v7();
    for (path, unknown_path) in reads(resource).into_iter().zip(reads(unknown)) {
        assert_unknown_id_404_at(app, who, (resource, &path), (unknown, &unknown_path), label)
            .await;
    }
}

/// [`assert_unknown_id_404`] for one door: `path` reads `resource`, `unknown_path` reads the
/// never-existing `unknown` through the same door.
async fn assert_unknown_id_404_at(
    app: &common::TestApp,
    who: &Caller,
    (resource, path): (Uuid, &str),
    (unknown, unknown_path): (Uuid, &str),
    label: &str,
) {
    let (status, text) = get(app, who, path).await;
    let (unknown_status, unknown_text) = get(app, who, unknown_path).await;
    assert_eq!(
        unknown_status, 404,
        "an unknown id is 404 on {unknown_path}"
    );
    assert_eq!(status, unknown_status, "{label}: {path}; body: {text}");
    assert_eq!(
        text.replace(&resource.to_string(), "<id>"),
        unknown_text.replace(&unknown.to_string(), "<id>"),
        "{label}: {path} must read exactly as an unknown id's 404"
    );
}

// ── WITNESS: the owner of a husk gets 410 RESOURCE_ERASED on every read, and nothing else ──────

/// FAILS IF any read answers the owner of an erased resource with anything but `410` under
/// `RESOURCE_ERASED` (the bite: dropping the husk check from the miss path returns the `404`), or
/// if the body carries anything beyond the id — the original title, the body text, or any
/// `details`. The live read first shows the body text on the wire, so its absence afterwards is
/// the erasure's work and not a body that never carried it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_owner_of_an_erased_resource_gets_410_resource_erased_on_every_read(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, _) = caller(&app.pool, "owner").await;
    let home = team_context(&app.pool, &[owner.profile]).await;
    let resource = ingest(&app, &owner, home).await;

    assert_reads_live(&app, &owner, resource, "owner").await;
    let (_, live_content) = get(&app, &owner, &format!("/api/resources/{resource}/content")).await;
    assert!(
        live_content.contains(BODY),
        "precondition: the live content carries the body text: {live_content}"
    );

    erase(&app, resource).await;

    assert_husk_410(&app, &owner, resource, "owner").await;
}

// ── WITNESS: a direct read-grant holder gets the same 410 ─────────────────────────────────────

/// FAILS IF a direct `can_read` grantee of an erased resource gets anything but the husk `410`
/// (the bite: a `resource_husk_held_by` without its direct-grant arm answers this caller `404`).
/// The grantee is not a member of the home context, so the grant is its only reach.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_direct_grant_holder_of_an_erased_resource_gets_410(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, _) = caller(&app.pool, "owner").await;
    let (grantee, _) = caller(&app.pool, "grantee").await;
    let home = team_context(&app.pool, &[owner.profile]).await;
    let resource = ingest(&app, &owner, home).await;
    grant_read(&app, &owner, resource, grantee.profile).await;

    assert_reads_live(&app, &grantee, resource, "grantee").await;

    erase(&app, resource).await;

    assert_husk_410(&app, &grantee, resource, "grantee").await;
}

// ── WITNESS: a context member without a grant gets the unknown id's 404, byte for byte ────────

/// The oracle check. FAILS IF a member of the resource's home context, holding no grant, can tell
/// the erased resource from an id that never existed — by status or by one byte of body once each
/// body's own id is normalized (the bite: a husk check keyed on "erased" alone, or one that kept
/// `resources_visible_to`'s context arm, answers this caller `410`). The member reads the live
/// resource first, so the later `404` is the husk rule's work, not a caller the fixture never
/// admitted. The owner's `410` in the same world shows the resource IS a husk.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_context_member_without_a_grant_gets_the_unknown_id_404(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, _) = caller(&app.pool, "owner").await;
    let (member, _) = caller(&app.pool, "member").await;
    let home = team_context(&app.pool, &[owner.profile, member.profile]).await;
    let resource = ingest(&app, &owner, home).await;

    assert_reads_live(&app, &member, resource, "context member").await;

    erase(&app, resource).await;

    assert_unknown_id_404(&app, &member, resource, "context member").await;
    assert_husk_410(&app, &owner, resource, "owner, same world").await;
}

// ── WITNESS: the owner of a tombstone gets 404, not 410 ───────────────────────────────────────

/// FAILS IF a soft-deleted resource that was never erased answers its owner `410` (the bite: a
/// husk test on `is_active` rather than `erased_at` would), or answers anything but the unknown
/// id's `404`. The read-only probe pins the fixture: the delete door left a tombstone
/// (`is_active` false) with `erased_at` NULL.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_owner_of_a_tombstone_gets_404_not_410(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let resource = ingest(&app, &owner, own_context).await;

    assert_reads_live(&app, &owner, resource, "owner").await;

    delete(&app, &owner, resource).await;

    let (is_active, erased): (bool, bool) =
        sqlx::query_as("SELECT is_active, erased_at IS NOT NULL FROM kb_resources WHERE id = $1")
            .bind(resource)
            .fetch_one(&app.pool)
            .await
            .expect("tombstone probe");
    assert!(!is_active, "precondition: the delete door made a tombstone");
    assert!(!erased, "precondition: a tombstone is not an erasure");

    assert_unknown_id_404(&app, &owner, resource, "tombstone owner").await;
}

// ── the block read: `GET /api/resources/{id}/blocks/{block_id}` ───────────────────────────────

/// The resource's first content block, read before any erasure or delete — a read-only probe of
/// what the ingest door wrote, so the block address under test is a real one.
async fn first_block(pool: &PgPool, resource: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 ORDER BY seq LIMIT 1",
    )
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("the ingested resource has a content block")
}

fn block_path(resource: Uuid, block: Uuid) -> String {
    format!("/api/resources/{resource}/blocks/{block}")
}

/// Assert `who` reads `block` live on its home `resource` — the precondition that makes a later
/// `404` or `410` about the erasure or the delete, not about an address that never resolved.
async fn assert_block_live(
    app: &common::TestApp,
    who: &Caller,
    resource: Uuid,
    block: Uuid,
    label: &str,
) {
    let (status, text) = get(app, who, &block_path(resource, block)).await;
    assert_eq!(status, 200, "{label}: the live block reads; body: {text}");
    let body: Value = serde_json::from_str(&text).expect("block read is JSON");
    assert_eq!(body["state"], "live", "{label}: {text}");
}

/// FAILS IF the block read answers the owner of an erased home resource with anything but the
/// husk `410` under `RESOURCE_ERASED` and the error envelope (the bite: `block_read_select`
/// mapping its `NotVisible` miss straight to `NotFound`, without `erased_or`, answers `404`).
/// The body is the error envelope, never a folded `BlockRead` — `assert_husk_410_at` admits a
/// body with exactly one `error` key and no `state`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_owner_of_an_erased_resource_gets_410_on_the_block_read(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, _) = caller(&app.pool, "owner").await;
    let home = team_context(&app.pool, &[owner.profile]).await;
    let resource = ingest(&app, &owner, home).await;
    let block = first_block(&app.pool, resource).await;

    assert_block_live(&app, &owner, resource, block, "owner").await;

    erase(&app, resource).await;

    assert_husk_410_at(
        &app,
        &owner,
        resource,
        &block_path(resource, block),
        "owner",
    )
    .await;
}

/// The block read's oracle check. FAILS IF a home-context member holding no grant can tell the
/// erased home resource from one that never existed through the block read — by status or by one
/// byte of body once each body's own resource id is normalized (the bite: classifying the miss on
/// "erased" alone, not on `resource_husk_held_by`, answers this caller `410`). The owner's `410`
/// in the same world shows the resource IS a husk.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_non_holder_gets_the_unknown_id_404_on_the_block_read(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, _) = caller(&app.pool, "owner").await;
    let (member, _) = caller(&app.pool, "member").await;
    let home = team_context(&app.pool, &[owner.profile, member.profile]).await;
    let resource = ingest(&app, &owner, home).await;
    let block = first_block(&app.pool, resource).await;

    assert_block_live(&app, &member, resource, block, "context member").await;

    erase(&app, resource).await;

    let unknown = Uuid::now_v7();
    assert_unknown_id_404_at(
        &app,
        &member,
        (resource, &block_path(resource, block)),
        (unknown, &block_path(unknown, block)),
        "context member",
    )
    .await;
    assert_husk_410_at(
        &app,
        &owner,
        resource,
        &block_path(resource, block),
        "owner, same world",
    )
    .await;
}

/// FAILS IF the block read answers the owner of a tombstone (soft-deleted, never erased) `410`
/// (the bite: a husk test on `is_active` rather than `erased_at`), or anything but the unknown
/// id's `404`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_owner_of_a_tombstone_gets_404_on_the_block_read(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let resource = ingest(&app, &owner, own_context).await;
    let block = first_block(&app.pool, resource).await;

    assert_block_live(&app, &owner, resource, block, "owner").await;

    delete(&app, &owner, resource).await;

    let unknown = Uuid::now_v7();
    assert_unknown_id_404_at(
        &app,
        &owner,
        (resource, &block_path(resource, block)),
        (unknown, &block_path(unknown, block)),
        "tombstone owner",
    )
    .await;
}
