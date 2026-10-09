#![cfg(feature = "test-db")]
//! A write to an ERASED resource (a husk: `kb_resources.erased_at` set) answers `410` under
//! `RESOURCE_ERASED` to a caller who holds standing on it, and the door's uniform `403` to everyone
//! else (resource erasure spec D13, F4; the write floor, `backend::write_floor`). The population is
//! the read side's — `resource_husk_held_by`, migration `20260930000060` — so a write never answers
//! an erasure to a caller the read would not.
//!
//! Witnessed on every resource-row write door that floors on `can_modify_resource` (an edge
//! assert floors its SOURCE), plus the grant door, whose every authority arm refuses a dead
//! subject — driven from one table ([`doors`]) so a new door is one row:
//!
//! * the owner of a husk gets `410` + `RESOURCE_ERASED`;
//! * a direct read-grant holder (who could never write it) gets the same `410`;
//! * a caller with no standing gets `403`, never `410`;
//! * the owner of a tombstone (soft-deleted through the real delete door, never erased) gets
//!   `403`, never `410`.
//!
//! Plus the goal-set partial write: a PATCH that changes the title AND links a goal the caller
//! may not link is refused whole — the title does not land.
//!
//! Plus the reblock batch (`POST /api/resources/reblock`), which floors per candidate and never
//! answers an erasure inside its `200`: an addressed erased id answers `404` (the candidate read
//! filters `is_active`), and a candidate erased between that read and its floor is a `denied` row.
//!
//! Plus the doors whose answer on an erased id is not the table's shape, each pinned on its own:
//!
//! * the edge-mutate doors (retype, reweight, fold, edge facet set and retract) on an edge that
//!   touched an erased resource answer `404` to everyone — the erasure act folded the edge, and
//!   the gate's `NOT is_folded` lookup refuses it before any clause runs; on an edge whose SOURCE
//!   is merely deleted (a delete folds nothing) they answer its owner the floor's `403`;
//! * an edge assert whose TARGET is erased answers the source's owner `404`, never `410` — the
//!   erased classification reaches only the resource the caller would modify;
//! * blob relate refuses an erased or deleted resource peer `404` (the peer read floor);
//! * single reassign answers the owner of a husk `410` and a read-grant holder `403` (the
//!   authority gate — owner or admin reach — runs first); a team reassign skips a husk and a
//!   tombstone instead of failing the run;
//! * a system admin is refused a grant on a tombstone (`403`) and answered `410` on a husk they
//!   hold;
//! * the owner is refused grant administration on a dead resource: revoke on a tombstone `403`,
//!   on a husk `410`, and an all-false grant on a tombstone `403` (the owner's derived `grant`
//!   arm floors on liveness, migration `20261002000010`);
//! * `remove_member`'s residual warning and the team handoff count live resources only;
//! * a create that replays its idempotency key onto a resource since erased or deleted: a
//!   segmented begin answers the owner `410` (erased) or `403` (deleted) from the ingestion
//!   record's floor; a one-shot create (`POST /api/ingest`, `POST /api/resources`) answers `410`
//!   (erased) or `404` (deleted) from its readback. The key is owner-scoped, so only the owner can
//!   replay it.
//!
//! Plus the races (resource erasure spec D13: "a write that races the act either lands before it
//! or refuses after it"), each with the act held open on R and the write shown waiting on a row
//! lock (`pg_stat_activity`) before the act commits: an edge assert into R answers `404` and lands
//! no edge; a blob relate onto R answers `404` and lands no edge; the owner's grant on R answers
//! `410` and lands no grant row; the owner's annotate of R — a door with no pool fast-fail —
//! answers `410` and lands no annotation; a goal-set naming R as the goal locks R before it writes
//! anything and answers `404`. And the delete door, driven for real, waits on a writer's floor
//! lock and completes once the writer commits.
//!
//! Every state is made by a real door: the resource by `POST /api/ingest`, the grant by
//! `POST /api/resources/{id}/grants`, the husk by the operator door
//! `POST /api/admin/resources/erasure`, the tombstone by `DELETE /api/resources/{id}`. Nothing
//! writes `is_active` or `erased_at` by hand. The team, its context and the memberships are
//! fixture rows, as in `resource_husk_read_test.rs`, whose helpers these are.

mod common;

use reqwest::Method;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ingest::{
    pack_chunks, AppendBlockPayload, FinalizePayload, IngestPayload, PackedChunk, SegmentedBegin,
};

const TITLE: &str = "Husk Write Subject Title";
const BODY: &str = "Prose the write floor must refuse to touch once erased.";
/// The segment the append door is offered.
const SEGMENT: &str = "\n## Appended\n\nA segment the write floor must refuse to land.";

/// The code a husk answer travels under — the one constant producer and consumer share.
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
    let email = format!("husk-write-{label}-{}@example.com", Uuid::new_v4());
    let (profile, own_context) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);
    (Caller { token, profile }, own_context)
}

/// A team that owns a context, with each profile a `member` (an authoring role, so the owner may
/// write into it; every member reads it). Returns the context id.
async fn team_context(pool: &PgPool, members: &[Uuid]) -> Uuid {
    let team = Uuid::now_v7();
    let slug = format!("husk-write-team-{}", &team.simple().to_string()[..8]);
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
    ingest_typed(app, owner, context, "research").await
}

/// [`ingest`], as `doc_type`: a goal edge folds only onto a `goal`-typed target.
async fn ingest_typed(
    app: &common::TestApp,
    owner: &Caller,
    context: Uuid,
    doc_type: &str,
) -> Uuid {
    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        title: TITLE.to_string(),
        origin_uri: format!("test://husk-write-{}", Uuid::new_v4()),
        context_ref: context.to_string(),
        home_cogmap_id: None,
        doc_type_name: doc_type.to_string(),
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

/// One write door under test: how to address it for one resource, with a minimal valid body.
struct Door {
    name: &'static str,
    method: Method,
    path: String,
    body: Option<Value>,
}

/// Every resource-row write door that floors on `can_modify_resource` inside its transaction.
/// Each body is valid for a live resource the caller may modify, so a refusal below is the
/// floor's, not a shape error's. The two segmented-ingest doors' bodies are well-formed (the
/// append's `content_hash` is the segment's real sha256 and it carries its own chunks); the
/// resource here is `complete`, not `in_progress`, but both doors floor before the op is asked
/// anything about its state.
fn doors(resource: Uuid) -> Vec<Door> {
    vec![
        Door {
            name: "PATCH /api/resources/{id}",
            method: Method::PATCH,
            path: format!("/api/resources/{resource}"),
            body: Some(json!({ "title": "A title the floor must refuse" })),
        },
        Door {
            name: "PUT /api/resources/{id}/meta",
            method: Method::PUT,
            path: format!("/api/resources/{resource}/meta"),
            body: Some(json!({
                "resource_id": resource,
                "managed_meta": {},
                "open_meta": { "note": "refused" },
                "managed_hash": "",
                "open_hash": "",
            })),
        },
        Door {
            name: "PUT /api/ingest/{id}",
            method: Method::PUT,
            path: format!("/api/ingest/{resource}"),
            body: Some(
                serde_json::to_value(IngestPayload {
                    idempotency_key: None,
                    segmented: None,
                    title: TITLE.to_string(),
                    // `ingest::update` reads none of the identity fields (title, origin_uri,
                    // context_ref, doc_type_name); they are required by the payload type only.
                    origin_uri: format!("test://husk-write-update-{resource}"),
                    context_ref: String::new(),
                    home_cogmap_id: None,
                    doc_type_name: "research".to_string(),
                    content_hash: None,
                    // Empty content ⇒ no body revise; the meta half is the write.
                    content: String::new(),
                    metadata: None,
                    managed_meta: None,
                    open_meta: Some(json!({ "note": "refused" })),
                    chunks_packed: None,
                    goal: None,
                    act: Default::default(),
                    sources: Vec::new(),
                })
                .expect("ingest update body"),
            ),
        },
        Door {
            name: "DELETE /api/resources/{id}",
            method: Method::DELETE,
            path: format!("/api/resources/{resource}"),
            body: None,
        },
        Door {
            name: "POST /api/resources/{id}/provenance",
            method: Method::POST,
            path: format!("/api/resources/{resource}/provenance"),
            body: Some(json!({
                "sources": [{ "kind": "remote", "value": "https://example.com/husk-write" }],
            })),
        },
        Door {
            name: "POST /api/resources/{id}/artifacts",
            method: Method::POST,
            path: format!("/api/resources/{resource}/artifacts"),
            body: Some(json!({
                "kind": "husk-write-probe",
                "intent": "current",
                "content": { "probe": true },
            })),
        },
        Door {
            name: "POST /api/facets (resource owner)",
            method: Method::POST,
            path: "/api/facets".to_string(),
            body: Some(json!({
                "resource": resource,
                "values": { "summary": "refused" },
                "weight": 1.0,
            })),
        },
        Door {
            name: "POST /api/resources/{id}/blocks",
            method: Method::POST,
            path: format!("/api/resources/{resource}/blocks"),
            body: Some(
                serde_json::to_value(AppendBlockPayload {
                    seq: 1,
                    content: SEGMENT.to_string(),
                    content_hash: temper_core::hash::sha256_hex(SEGMENT.as_bytes()),
                    chunks_packed: Some(pack_chunks(&[chunk(SEGMENT)]).expect("pack")),
                    sources: Vec::new(),
                })
                .expect("append body"),
            ),
        },
        Door {
            name: "POST /api/resources/{id}/finalize",
            method: Method::POST,
            path: format!("/api/resources/{resource}/finalize"),
            body: Some(
                serde_json::to_value(FinalizePayload {
                    expected_blocks: 1,
                    expected_body_hash: String::new(),
                    expected_content_hash: None,
                })
                .expect("finalize body"),
            ),
        },
        // The erased resource as the edge's SOURCE: the floor the door runs at the head of the
        // edge write's transaction answers before the target is read. A self-edge keeps the body
        // valid for a live resource (kb_edges carries no self-loop constraint).
        Door {
            name: "POST /api/relationships (erased source)",
            method: Method::POST,
            path: "/api/relationships".to_string(),
            body: Some(json!({
                "source": resource,
                "target": resource,
                "edge_kind": "leads_to",
                "polarity": "forward",
                "label": "husk-write-probe",
                "weight": 1.0,
            })),
        },
        // The resource as a grant SUBJECT. Every arm refuses a dead subject (`can()`'s
        // subject-liveness floor on the explicit branch, `20260902000010`; the owner's derived
        // `grant` arm, `20261002000010`; the admin arm's own floor), and so does the door's
        // in-transaction subject floor; the door classifies the refusal. `kb_access_grants`
        // carries no FK on `principal_id`, and the refusal precedes the insert.
        Door {
            name: "POST /api/resources/{id}/grants",
            method: Method::POST,
            path: format!("/api/resources/{resource}/grants"),
            body: Some(json!({
                "principal_table": "kb_profiles",
                "principal_id": Uuid::now_v7(),
                "can_read": true,
                "can_write": false,
                "can_delete": false,
                "can_grant": false,
            })),
        },
        // The revoke verb of the same door: one authority (`GrantAuthority`) gates both verbs,
        // its refusal is classified by the same `erased_or_refused`, and the same in-transaction
        // subject floor runs before the delete. Revocation is not attenuated, so only the
        // authority arm or the floor can refuse it. An absent grant row would be a no-op `200`,
        // so an admission here would read `200`, never a false refusal.
        Door {
            name: "DELETE /api/resources/{id}/grants",
            method: Method::DELETE,
            path: format!("/api/resources/{resource}/grants"),
            body: Some(json!({
                "principal_table": "kb_profiles",
                "principal_id": Uuid::now_v7(),
            })),
        },
    ]
}

/// Send `door` as `who`: the status and the parsed body (`Null` when the body is not JSON).
async fn send(app: &common::TestApp, who: &Caller, door: &Door) -> (u16, Value) {
    let mut req = app
        .client
        .request(door.method.clone(), app.url(&door.path))
        .header("Authorization", format!("Bearer {}", who.token));
    if let Some(body) = &door.body {
        req = req.json(body);
    }
    let resp = req.send().await.expect("door request");
    let status = resp.status().as_u16();
    let text = resp.text().await.expect("door body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// Assert every door answers `who` with `410` under `RESOURCE_ERASED`, the message naming the id.
async fn assert_every_door_410(app: &common::TestApp, who: &Caller, resource: Uuid, label: &str) {
    for door in doors(resource) {
        let (status, body) = send(app, who, &door).await;
        assert_eq!(
            status, 410,
            "{label}: {} answers 410; body: {body}",
            door.name
        );
        assert_eq!(
            body["error"]["code"], RESOURCE_ERASED,
            "{label}: {} travels under RESOURCE_ERASED; body: {body}",
            door.name
        );
        assert_eq!(
            body["error"]["message"],
            format!("resource {resource} was erased"),
            "{label}: {} — the fixed message names the id and nothing else",
            door.name
        );
    }
}

/// Assert every door answers `who` with `403`, and never under `RESOURCE_ERASED`.
async fn assert_every_door_403(app: &common::TestApp, who: &Caller, resource: Uuid, label: &str) {
    for door in doors(resource) {
        let (status, body) = send(app, who, &door).await;
        assert_eq!(
            status, 403,
            "{label}: {} answers 403; body: {body}",
            door.name
        );
        assert_ne!(
            body["error"]["code"], RESOURCE_ERASED,
            "{label}: {} must never answer RESOURCE_ERASED; body: {body}",
            door.name
        );
    }
}

/// The resource's title as `who` reads it through `GET /api/resources/{id}`.
async fn title_of(app: &common::TestApp, who: &Caller, resource: Uuid) -> String {
    let resp = app
        .client
        .get(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {}", who.token))
        .send()
        .await
        .expect("read request");
    assert_eq!(resp.status().as_u16(), 200, "the owner reads the resource");
    let view: Value = resp.json().await.expect("view JSON");
    view["title"].as_str().expect("title field").to_owned()
}

// ── WITNESS: the owner of a husk gets 410 RESOURCE_ERASED on every write door ─────────────────

/// FAILS IF any write door answers the owner of an erased resource with anything but `410` under
/// `RESOURCE_ERASED`. The bite, per door: restore that door's `self.check_can_modify_next(..)`
/// pre-check (a bare `can_modify_resource` that renders every deny `Forbidden`) ahead of its
/// floor — or, for the update doors, delete the `modify_floor_fast_fail` call, whose absence lets
/// the visibility-gated `native_resource_identity` / readback answer the husk `404` first. For
/// `POST /api/relationships`, restore `self.check_can_modify_next(src_next)` ahead of the
/// transaction in `assert_relationship`. For the grant door, return the refusal unclassified in
/// `access_service::grant_capability` (drop its `erased_or_refused` call); for its revoke verb, the
/// same in `access_service::revoke_capability`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_owner_of_an_erased_resource_gets_410_on_every_write_door(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, _) = caller(&app.pool, "owner").await;
    let home = team_context(&app.pool, &[owner.profile]).await;
    let resource = ingest(&app, &owner, home).await;

    erase(&app, resource).await;

    assert_every_door_410(&app, &owner, resource, "owner").await;
}

// ── WITNESS: a direct read-grant holder gets the same 410 ─────────────────────────────────────

/// FAILS IF a direct `can_read` grantee of an erased resource — a caller who could never have
/// written it — gets anything but the husk `410` on any write door (ruling 1: the write's
/// population is the read's). The bite: in `write_floor::erased_or_forbidden`, answer
/// `Forbidden` unconditionally, or key the classification on `can_modify_resource` standing. The
/// grantee is not a member of the home context, so the grant is its only reach.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_read_grant_holder_of_an_erased_resource_gets_410_on_every_write_door(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, _) = caller(&app.pool, "owner").await;
    let (grantee, _) = caller(&app.pool, "grantee").await;
    let home = team_context(&app.pool, &[owner.profile]).await;
    let resource = ingest(&app, &owner, home).await;
    grant_read(&app, &owner, resource, grantee.profile).await;

    erase(&app, resource).await;

    assert_every_door_410(&app, &grantee, resource, "read-grant holder").await;
}

// ── WITNESS: a caller with no standing gets 403, never 410 ────────────────────────────────────

/// The oracle check. FAILS IF a caller with no standing on an erased resource gets `410` (or
/// anything but `403`) from any write door — the `410` would confirm an erasure to someone the read
/// side answers `404`. The bite: in `write_floor::erased_or_forbidden`, answer
/// `ResourceErased(resource)` whenever the row's `erased_at` is set, without asking
/// `resource_husk_held_by`. The owner's `410` in the same world shows the resource IS a husk.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_caller_with_no_standing_gets_403_on_every_write_door(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, _) = caller(&app.pool, "owner").await;
    let (stranger, _) = caller(&app.pool, "stranger").await;
    let home = team_context(&app.pool, &[owner.profile]).await;
    let resource = ingest(&app, &owner, home).await;

    erase(&app, resource).await;

    assert_every_door_403(&app, &stranger, resource, "no standing").await;
    assert_every_door_410(&app, &owner, resource, "owner, same world").await;
}

// ── WITNESS: the owner of a tombstone gets 403, never 410 ─────────────────────────────────────

/// FAILS IF a soft-deleted resource that was never erased answers its owner `410` (or anything
/// but `403`) on any write door. The bite: classify on `is_active` rather than `erased_at` — e.g.
/// in `write_floor::erased_or_forbidden`, answer `ResourceErased` whenever `can_modify_resource`
/// is false for the resource's owner. The probe pins the fixture: the delete door left a tombstone
/// (`is_active` false) with `erased_at` NULL.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_owner_of_a_tombstone_gets_403_on_every_write_door(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let resource = ingest(&app, &owner, own_context).await;

    delete(&app, &owner, resource).await;

    let (is_active, erased): (bool, bool) =
        sqlx::query_as("SELECT is_active, erased_at IS NOT NULL FROM kb_resources WHERE id = $1")
            .bind(resource)
            .fetch_one(&app.pool)
            .await
            .expect("tombstone probe");
    assert!(!is_active, "precondition: the delete door made a tombstone");
    assert!(!erased, "precondition: a tombstone is not an erasure");

    assert_every_door_403(&app, &owner, resource, "tombstone owner").await;
}

// ── WITNESS: a refused goal-set rolls the whole update back ───────────────────────────────────

/// FAILS IF a PATCH that changes the title AND links a goal the caller may not link is refused
/// while the title change lands anyway (a partial write). The goal belongs to a stranger, in the
/// stranger's own context, so the owner cannot read it: the edge's target clause
/// (`check_endpoint_readable_in_tx`) refuses it `404`. The bite: in `update_resource`, move
/// `tx.commit()` up to directly after `writes::update_resource_in_tx(..)` and run the goal match on
/// a fresh transaction — the shape before this change, which commits the title and then refuses.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_refused_goal_set_leaves_the_title_unchanged(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let (stranger, stranger_context) = caller(&app.pool, "stranger").await;
    let resource = ingest(&app, &owner, own_context).await;
    let goal = ingest(&app, &stranger, stranger_context).await;

    assert_eq!(
        title_of(&app, &owner, resource).await,
        TITLE,
        "precondition: the resource carries the ingested title"
    );

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {}", owner.token))
        .json(&json!({ "title": "A title that must not land", "goal": goal }))
        .send()
        .await
        .expect("patch request");
    let status = resp.status().as_u16();
    let body = resp.text().await.expect("patch body");
    assert_eq!(
        status, 404,
        "a goal the caller cannot read is refused as absent; body: {body}"
    );

    assert_eq!(
        title_of(&app, &owner, resource).await,
        TITLE,
        "the refused goal-set rolled the whole update back — the title did not land"
    );
}

// ── WITNESS: reblock addressed at an erased id answers 410 to a holder, 404 to everyone else ──

/// FAILS IF `POST /api/resources/reblock` with `scope = resource` addressed at an erased resource
/// answers its owner (a husk holder) anything but `410 RESOURCE_ERASED`, or answers a caller with
/// no standing anything but the leak-safe `404` (ruled 2026-09-30: every write door gives a holder
/// the read side's 410). The resource arm's candidate read filters `is_active`, so the erased id
/// misses there and the miss is classified through `resource_husk_held_by`. The bites: answer the
/// miss with `NotFound` unconditionally (the owner assertion fails), or with `ResourceErased`
/// unconditionally (the stranger assertion fails).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reblock_addressed_at_an_erased_resource_answers_410_to_a_holder_and_404_otherwise(
    pool: PgPool,
) {
    let app = common::setup_test_app(pool).await;
    let (owner, _) = caller(&app.pool, "owner").await;
    let (stranger, _) = caller(&app.pool, "stranger").await;
    let home = team_context(&app.pool, &[owner.profile]).await;
    let resource = ingest(&app, &owner, home).await;

    erase(&app, resource).await;

    for (who, expected_status, label) in
        [(&owner, 410u16, "owner"), (&stranger, 404u16, "stranger")]
    {
        let resp = app
            .client
            .post(app.url("/api/resources/reblock"))
            .header("Authorization", format!("Bearer {}", who.token))
            .json(&json!({ "scope": { "resource": resource }, "dry_run": false }))
            .send()
            .await
            .expect("reblock request");
        let status = resp.status().as_u16();
        let body: Value =
            serde_json::from_str(&resp.text().await.expect("reblock body")).unwrap_or(Value::Null);
        assert_eq!(
            status, expected_status,
            "{label}: reblock of an erased id; body: {body}"
        );
        if expected_status == 410 {
            assert_eq!(
                body["error"]["code"], RESOURCE_ERASED,
                "{label}; body: {body}"
            );
        } else {
            assert_ne!(
                body["error"]["code"], RESOURCE_ERASED,
                "{label}; body: {body}"
            );
        }
    }
}

// ── The race choreography: the act held open, a write waiting on its lock ─────────────────────

/// The erasure act on `resource`, executed and NOT committed: the returned transaction holds
/// `FOR UPDATE` on R's row until the caller commits it. The act is called as the SQL function the
/// operator door calls (`resource_erasure_execute`), because the door commits its own transaction
/// and a race needs one held open; its `SystemAdmin` gate is the door's, not the function's. The
/// choreography is `resource_erasure_act.rs`'s.
async fn hold_the_act(
    app: &common::TestApp,
    resource: Uuid,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let (operator, _) = caller(&app.pool, "operator").await;
    let operator_emitter: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
          WHERE e.profile_id = $1 AND e.name = p.handle || '@web'",
    )
    .bind(operator.profile)
    .fetch_one(&app.pool)
    .await
    .expect("the operator's web emitter");
    let mut act = app.pool.begin().await.expect("begin the act");
    sqlx::query("SELECT resource_erasure_execute($1, $2, $3, $4, '{}'::uuid[])")
        .bind(resource)
        .bind(operator.profile)
        .bind(operator_emitter)
        .bind(Uuid::now_v7())
        .execute(&mut *act)
        .await
        .expect("the act runs inside its open transaction");
    act
}

/// Send `method path` as `who` with a JSON body, on its own task: the handle yields the status
/// and the parsed body (`Null` when the body is not JSON).
fn spawn_request(
    app: &common::TestApp,
    who: &Caller,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> tokio::task::JoinHandle<(u16, Value)> {
    let client = app.client.clone();
    let url = app.url(path);
    let token = who.token.clone();
    tokio::spawn(async move {
        let mut req = client
            .request(method, url)
            .header("Authorization", format!("Bearer {token}"));
        if let Some(body) = &body {
            req = req.json(body);
        }
        let resp = req.send().await.expect("raced request");
        let status = resp.status().as_u16();
        let text = resp.text().await.expect("raced body");
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    })
}

/// Poll — every 50ms, for up to 10 seconds — until a backend in this test's database is blocked
/// on a lock (`pg_stat_activity.wait_event_type = 'Lock'`; a row-lock wait shows as a wait on
/// the holder's transaction id), and return that backend's pid. Panics if `request` finishes first
/// (it never waited on the held row) or no backend waits within the deadline. The test's own
/// held transaction is idle, never waiting, so the one waiter is the request's.
async fn a_backend_waits_on_a_lock<T>(
    pool: &PgPool,
    request: &tokio::task::JoinHandle<T>,
    what: &str,
) -> i32 {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        assert!(
            !request.is_finished(),
            "{what} completed while the row was held — it did not wait on the row lock"
        );
        let waiter: Option<i32> = sqlx::query_scalar(
            "SELECT pid FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock' \
              LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .expect("poll pg_stat_activity");
        if let Some(pid) = waiter {
            return pid;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}: no backend waited on a lock within 10 seconds"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Send `method path` as `who` while `held` holds the row, assert the request is blocked on a lock
/// ([`a_backend_waits_on_a_lock`]) and has not answered, then commit `held` and return the
/// request's answer: the status and the parsed body (`Null` when the body is not JSON).
async fn raced_by_the_act(
    app: &common::TestApp,
    held: sqlx::Transaction<'static, sqlx::Postgres>,
    who: &Caller,
    method: Method,
    path: String,
    body: Value,
) -> (u16, Value) {
    let request = spawn_request(app, who, method, &path, Some(body));
    a_backend_waits_on_a_lock(&app.pool, &request, &path).await;
    held.commit().await.expect("commit the held transaction");
    request.await.expect("the raced request must not panic")
}

/// Is `resource` an erased husk?
async fn is_erased(pool: &PgPool, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT erased_at IS NOT NULL FROM kb_resources WHERE id = $1")
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("husk probe")
}

// ── WITNESS: a candidate erased under the batch is a denied row, never a 410 ──────────────────

/// FAILS IF a reblock candidate that is erased between the candidate read and its write floor
/// surfaces as anything but a `denied` row in a `200` receipt. No scope can ENUMERATE an erased
/// resource (every candidate read filters `is_active`), so the floor's erased classification
/// reaches the batch only through this race — and it must read `denied`, exactly as a `Forbidden`
/// does, never a `410` inside a `200` and never an `error` row.
///
/// The choreography is [`hold_the_act`] and [`raced_by_the_act`]: the act runs uncommitted in its
/// own transaction (holding `FOR UPDATE` on R), the reblock starts and its floor waits on R's row
/// lock, then the act commits.
///
/// The bite: in `DbBackend::reblock_candidate`, narrow the floor's refusal arm to
/// `Err(TemperError::Forbidden)` — the `ResourceErased` classification then falls to `row_error`
/// and the row reads `error`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_candidate_erased_under_the_batch_reads_denied(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, _) = caller(&app.pool, "owner").await;
    let home = team_context(&app.pool, &[owner.profile]).await;
    let resource = ingest(&app, &owner, home).await;

    // The act, executed and NOT committed: it holds FOR UPDATE on R's row.
    let act = hold_the_act(&app, resource).await;

    // The reblock: R is still live to its candidate read (the act has not committed), so R is a
    // candidate; its floor's FOR KEY SHARE then waits on the act's FOR UPDATE.
    let (status, receipt) = raced_by_the_act(
        &app,
        act,
        &owner,
        Method::POST,
        "/api/resources/reblock".to_string(),
        json!({ "scope": { "resource": resource }, "dry_run": false }),
    )
    .await;

    assert!(
        is_erased(&app.pool, resource).await,
        "precondition: the act committed an erasure"
    );

    assert_eq!(
        status, 200,
        "the batch answers its receipt; body: {receipt}"
    );
    let outcomes = receipt["outcomes"].as_array().expect("outcomes array");
    assert_eq!(
        outcomes.len(),
        1,
        "R was the one candidate; receipt: {receipt}"
    );
    assert_eq!(
        outcomes[0]["resource"],
        json!(resource),
        "the row is R's; receipt: {receipt}"
    );
    assert_eq!(
        outcomes[0]["outcome"], "denied",
        "an erased candidate is the denied row a forbidden one is; receipt: {receipt}"
    );
    assert_eq!(receipt["summary"]["declined"], 1, "receipt: {receipt}");
    assert_eq!(receipt["summary"]["error"], 0, "receipt: {receipt}");
}

// ── Doors whose answer on an erased id is not the table's shape ───────────────────────────────

/// `method path` as `who`, with an optional JSON body: the status and the parsed body.
async fn call(
    app: &common::TestApp,
    who: &Caller,
    method: Method,
    path: String,
    body: Option<Value>,
) -> (u16, Value) {
    let door = Door {
        name: "ad hoc",
        method,
        path,
        body,
    };
    send(app, who, &door).await
}

/// `owner` asserts `source → target` through `POST /api/relationships`; returns the edge handle.
async fn assert_edge(app: &common::TestApp, owner: &Caller, source: Uuid, target: Uuid) -> Uuid {
    let (status, body) = call(
        app,
        owner,
        Method::POST,
        "/api/relationships".to_string(),
        Some(json!({
            "source": source,
            "target": target,
            "edge_kind": "leads_to",
            "polarity": "forward",
            "label": "husk-edge",
            "weight": 1.0,
        })),
    )
    .await;
    assert_eq!(status, 200, "the owner asserts the edge; body: {body}");
    Uuid::parse_str(body["edge_handle"].as_str().expect("edge_handle")).expect("edge uuid")
}

/// The five doors that mutate an existing edge, addressed at `edge`. The retract's property id is
/// a fresh uuid: the edge gate runs before the row is looked up.
fn edge_mutate_doors(edge: Uuid) -> Vec<Door> {
    vec![
        Door {
            name: "POST /api/relationships/{h}/retype",
            method: Method::POST,
            path: format!("/api/relationships/{edge}/retype"),
            body: Some(json!({ "edge_kind": "near", "polarity": "forward" })),
        },
        Door {
            name: "POST /api/relationships/{h}/reweight",
            method: Method::POST,
            path: format!("/api/relationships/{edge}/reweight"),
            body: Some(json!({ "weight": 0.5 })),
        },
        Door {
            name: "POST /api/relationships/{h}/fold",
            method: Method::POST,
            path: format!("/api/relationships/{edge}/fold"),
            body: Some(json!({ "reason": "husk-write probe" })),
        },
        Door {
            name: "POST /api/relationships/{h}/facets",
            method: Method::POST,
            path: format!("/api/relationships/{edge}/facets"),
            body: Some(json!({ "values": { "summary": "refused" } })),
        },
        Door {
            name: "DELETE /api/relationships/{h}/facets/{pid}",
            method: Method::DELETE,
            path: format!("/api/relationships/{edge}/facets/{}", Uuid::now_v7()),
            body: None,
        },
    ]
}

/// Is `edge` folded?
async fn edge_folded(pool: &PgPool, edge: Uuid) -> bool {
    sqlx::query_scalar("SELECT is_folded FROM kb_edges WHERE id = $1")
        .bind(edge)
        .fetch_one(pool)
        .await
        .expect("edge row")
}

// ── WITNESS: an edge touching an erased resource was folded by the act — its doors answer 404 ──

/// The erasure act folds every live edge touching the erased resource, in either direction
/// (`resource_erasure_execute`'s per-edge fold loop, latest body `20260930000070`). So the
/// edge-mutate doors never reach the source's write floor for such an edge: `check_edge_mutable`'s
/// `NOT is_folded` lookup answers `404` first — to the owner of the husk as to anyone. That is NOT
/// the table's `410`, and it is the honest answer: the door addresses the EDGE, and the edge is
/// gone. Pinned for an edge whose SOURCE was erased and one whose TARGET was.
///
/// FAILS IF the act stops folding the edges of an erased resource, or the edge gate stops
/// refusing a folded edge. The bite: drop `AND NOT is_folded` from `check_edge_mutable`'s lookup —
/// the out-edge then reaches the source floor and answers the owner `410`, and the in-edge
/// reaches its live source's floor and the doors answer `200`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_edge_touching_an_erased_resource_was_folded_and_its_doors_answer_404(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let erased = ingest(&app, &owner, own_context).await;
    let live = ingest(&app, &owner, own_context).await;
    let out_edge = assert_edge(&app, &owner, erased, live).await;
    let in_edge = assert_edge(&app, &owner, live, erased).await;

    erase(&app, erased).await;

    for (edge, label) in [(out_edge, "erased source"), (in_edge, "erased target")] {
        assert!(
            edge_folded(&app.pool, edge).await,
            "precondition ({label}): the act folded the edge"
        );
        for door in edge_mutate_doors(edge) {
            let (status, body) = send(&app, &owner, &door).await;
            assert_eq!(
                status, 404,
                "{label}: {} on a folded edge answers 404; body: {body}",
                door.name
            );
            assert_ne!(
                body["error"]["code"], RESOURCE_ERASED,
                "{label}: {} never answers RESOURCE_ERASED; body: {body}",
                door.name
            );
        }
    }
}

// ── WITNESS: an edge whose source is deleted answers its owner the source floor's 403 ─────────

/// A soft delete folds no edge (`_project_resource_deleted` flips `is_active` only), so an edge out
/// of a tombstone stays live and the edge-mutate doors reach `check_edge_mutable`'s source clause:
/// the write floor, inside the edge write's transaction, which refuses a tombstone `403` — never
/// `410`. The owner still has container-write on the home and can read the live target, so the
/// floor is the only clause that refuses.
///
/// FAILS IF the edge doors admit a write out of a tombstoned source, or classify it erased. The
/// bite: in `check_edge_mutable`, make the `"kb_resources"` arm admit (replace its
/// `write_floor::modify_floor_in_tx(..)` with `{}`) — retype, reweight and fold then answer `200`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_edge_out_of_a_deleted_resource_answers_its_owner_403(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let source = ingest(&app, &owner, own_context).await;
    let target = ingest(&app, &owner, own_context).await;
    let edge = assert_edge(&app, &owner, source, target).await;

    delete(&app, &owner, source).await;
    assert!(
        !edge_folded(&app.pool, edge).await,
        "precondition: a delete folds no edge"
    );

    for door in edge_mutate_doors(edge) {
        let (status, body) = send(&app, &owner, &door).await;
        assert_eq!(
            status, 403,
            "{}: an edge out of a tombstone is refused by the source floor; body: {body}",
            door.name
        );
        assert_ne!(
            body["error"]["code"], RESOURCE_ERASED,
            "{}: a tombstone is never an erasure; body: {body}",
            door.name
        );
    }
}

// ── WITNESS: an erased TARGET stays 404 for the source's owner ────────────────────────────────

/// The erased classification reaches only the resource the caller would MODIFY — the edge's
/// source. The owner of a live source asserting an edge into an erased resource they held gets the
/// target read floor's `404` (`check_endpoint_readable_in_tx`), never `410`: they are not writing
/// the target. The read door shows the same caller IS a holder of that husk (`410` on `GET`).
///
/// FAILS IF the target clause classifies an erased target. The bite: in
/// `assert_edge_from_source_home_in_tx`, replace `check_endpoint_readable_in_tx(.., tgt_table,
/// edge.tgt)` with `write_floor::modify_floor_in_tx(&mut *conn, self.profile_id,
/// ResourceId::from(edge.tgt))` — the husk's holder then gets `410`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_edge_into_an_erased_target_answers_its_holder_404(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let source = ingest(&app, &owner, own_context).await;
    let target = ingest(&app, &owner, own_context).await;

    erase(&app, target).await;

    let (read_status, read_body) = call(
        &app,
        &owner,
        Method::GET,
        format!("/api/resources/{target}"),
        None,
    )
    .await;
    assert_eq!(
        read_status, 410,
        "precondition: the owner holds the husk; body: {read_body}"
    );

    let (status, body) = call(
        &app,
        &owner,
        Method::POST,
        "/api/relationships".to_string(),
        Some(json!({
            "source": source,
            "target": target,
            "edge_kind": "leads_to",
            "polarity": "forward",
            "label": "into-a-husk",
            "weight": 1.0,
        })),
    )
    .await;
    assert_eq!(
        status, 404,
        "an erased target is the target floor's 404; body: {body}"
    );
    assert_ne!(
        body["error"]["code"], RESOURCE_ERASED,
        "the target is not the caller's write; body: {body}"
    );
}

// ── WITNESS: blob relate refuses an erased or deleted resource peer ───────────────────────────

/// A blob app: the in-memory provider and a test-sized blob config, as `blob_handler_test.rs`.
async fn blob_app(pool: PgPool) -> common::TestApp {
    use std::sync::Arc;
    use temper_services::config::{BlobConfig, BlobCredentialMode};
    use temper_substrate::blob_store::InMemoryBlobStore;
    common::setup_test_app_with_state(pool, move |state| {
        state.blob_store = Some(Arc::new(InMemoryBlobStore::default()));
        let mut config = (*state.config).clone();
        config.blob = Some(BlobConfig {
            store_id: "store_test".to_string(),
            read_write_token: Some("vercel_rw_test_store_test".to_string()),
            credential_mode: BlobCredentialMode::Token,
            oidc_token_source: Arc::new(|| None),
            max_bytes: 1 << 20,
            allowlist: vec!["image/png".to_string()],
            single_request_max_bytes: 64 * 1024,
        });
        state.config = Arc::new(config);
    })
    .await
}

/// `owner` commits a blob homed in `context` through `POST /api/blobs`; returns its id.
async fn commit_blob(app: &common::TestApp, owner: &Caller, context: Uuid) -> Uuid {
    let part = reqwest::multipart::Part::bytes(b"husk-peer".to_vec())
        .file_name("figure.png")
        .mime_str("image/png")
        .expect("mime");
    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("home_table", "kb_contexts".to_string())
        .text("home_id", context.to_string());
    let resp = app
        .client
        .post(app.url("/api/blobs"))
        .header("Authorization", format!("Bearer {}", owner.token))
        .multipart(form)
        .send()
        .await
        .expect("blob commit request");
    assert_eq!(resp.status().as_u16(), 200, "the owner commits a blob");
    let committed: Value = resp.json().await.expect("blob JSON");
    Uuid::parse_str(committed["blob_id"].as_str().expect("blob_id")).expect("uuid")
}

/// The blob's relation count, folded or not — any edge touching `blob` at all.
async fn blob_edges(pool: &PgPool, blob: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_edges \
          WHERE (source_table = 'kb_blobs' AND source_id = $1) \
             OR (target_table = 'kb_blobs' AND target_id = $1)",
    )
    .bind(blob)
    .fetch_one(pool)
    .await
    .expect("edge count")
}

/// Blob relate keeps its authority on the BLOB's home by design (no `can_modify` on the peer). Its
/// peer gate is `endpoint_readable_by_profile`, whose `kb_resources` arm is `resources_visible_to`
/// and joins `kb_resources.is_active` — so an erased peer (`is_active` false with the husk) and a
/// deleted one are refused `404`, in both directions, to the owner who held them.
///
/// FAILS IF relate admits an edge onto a dead resource peer. The bite: delete
/// `check_peer_readable(&mut tx, caller, &peer).await?;` from `blob_service::relate_blob`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn blob_relate_refuses_an_erased_or_deleted_resource_peer(pool: PgPool) {
    let app = blob_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let erased = ingest(&app, &owner, own_context).await;
    let deleted = ingest(&app, &owner, own_context).await;
    let blob = commit_blob(&app, &owner, own_context).await;

    erase(&app, erased).await;
    delete(&app, &owner, deleted).await;

    for (peer, label) in [(erased, "erased peer"), (deleted, "deleted peer")] {
        for direction in ["blob_as_source", "blob_as_target"] {
            let (status, body) = call(
                &app,
                &owner,
                Method::POST,
                format!("/api/blobs/{blob}/relations"),
                Some(json!({
                    "direction": direction,
                    "peer_table": "kb_resources",
                    "peer_id": peer,
                    "edge_kind": "express",
                    "polarity": "forward",
                    "label": "husk-peer",
                    "weight": 1.0,
                })),
            )
            .await;
            assert_eq!(
                status, 404,
                "{label}, {direction}: refused as absent; body: {body}"
            );
            assert_ne!(
                body["error"]["code"], RESOURCE_ERASED,
                "{label}, {direction}: the peer is not the caller's write; body: {body}"
            );
        }
    }
    assert_eq!(
        blob_edges(&app.pool, blob).await,
        0,
        "no relation landed on a dead peer"
    );
}

// ── WITNESS: single reassign — owner of a husk 410, everyone else 403, nothing moves ─────────

/// `resource_husk_held_by(profile, resource)`, read directly.
async fn holds_husk(pool: &PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT resource_husk_held_by($1, $2)")
        .bind(profile)
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("husk probe")
}

/// The resource's home owner.
async fn home_owner(pool: &PgPool, resource: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT owner_profile_id FROM kb_resource_homes WHERE resource_id = $1")
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("home row")
}

/// `POST /api/resources/{id}/reassign` as `who`, to `to`.
async fn reassign(app: &common::TestApp, who: &Caller, resource: Uuid, to: Uuid) -> (u16, Value) {
    call(
        app,
        who,
        Method::POST,
        format!("/api/resources/{resource}/reassign"),
        Some(json!({ "to_profile_id": to })),
    )
    .await
}

/// Reassign's population differs from the table's, by design (plan 2c, controller ruling 5): its
/// AUTHORITY stays owner-or-admin-reach and runs first, reading only the home; the liveness floor
/// runs after it, inside the reassign's transaction. So:
///
/// * the owner of a husk passes the authority gate and the floor refuses it: `410`;
/// * a read-grant holder of the husk is not its owner and has no admin reach: the authority gate's
///   `403` — the husk is never confirmed to someone the gate already refuses;
/// * a stranger: `403`.
///
/// Nothing moves: the home owner is unchanged, and so is `resource_husk_held_by`'s population.
///
/// FAILS IF reassign moves, or answers anything but the above for, an erased resource. The bite:
/// delete the `write_floor::liveness_floor_in_tx(..)` call in `reassign_service::reassign_resource`
/// — the owner then gets `200` and the husk's home moves to the stranger.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reassign_of_an_erased_resource_answers_its_owner_410_and_moves_nothing(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, _) = caller(&app.pool, "owner").await;
    let (grantee, _) = caller(&app.pool, "grantee").await;
    let (stranger, _) = caller(&app.pool, "stranger").await;
    let home = team_context(&app.pool, &[owner.profile]).await;
    let resource = ingest(&app, &owner, home).await;
    grant_read(&app, &owner, resource, grantee.profile).await;

    erase(&app, resource).await;

    let (status, body) = reassign(&app, &owner, resource, stranger.profile).await;
    assert_eq!(
        status, 410,
        "owner: the floor refuses the husk; body: {body}"
    );
    assert_eq!(
        body["error"]["code"], RESOURCE_ERASED,
        "owner; body: {body}"
    );

    for (who, label) in [(&grantee, "read-grant holder"), (&stranger, "stranger")] {
        let (status, body) = reassign(&app, who, resource, who.profile).await;
        assert_eq!(
            status, 403,
            "{label}: the authority gate refuses first; body: {body}"
        );
        assert_ne!(
            body["error"]["code"], RESOURCE_ERASED,
            "{label}; body: {body}"
        );
    }

    assert_eq!(
        home_owner(&app.pool, resource).await,
        owner.profile,
        "the husk's home did not move"
    );
    assert!(
        holds_husk(&app.pool, owner.profile, resource).await,
        "the owner still holds the husk"
    );
    assert!(
        holds_husk(&app.pool, grantee.profile, resource).await,
        "the read-grant holder still holds the husk"
    );
    assert!(
        !holds_husk(&app.pool, stranger.profile, resource).await,
        "the would-be recipient gained no standing"
    );
}

/// FAILS IF reassign moves a soft-deleted resource, or answers its owner anything but `403`
/// (never `410` — a tombstone is not an erasure). The bite: delete the
/// `write_floor::liveness_floor_in_tx(..)` call in `reassign_service::reassign_resource` — the
/// owner then gets `200` and the tombstone's home moves.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reassign_of_a_deleted_resource_answers_its_owner_403(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let (recipient, _) = caller(&app.pool, "recipient").await;
    let resource = ingest(&app, &owner, own_context).await;

    delete(&app, &owner, resource).await;

    let (status, body) = reassign(&app, &owner, resource, recipient.profile).await;
    assert_eq!(status, 403, "a tombstone fails liveness; body: {body}");
    assert_ne!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");
    assert_eq!(
        home_owner(&app.pool, resource).await,
        owner.profile,
        "the tombstone's home did not move"
    );
}

// ── WITNESS: team reassign skips a husk and a tombstone; the run completes ────────────────────

/// A team holding the given `(profile, role)` memberships. Returns the team id.
async fn team_with(pool: &PgPool, members: &[(Uuid, &str)]) -> Uuid {
    let team = Uuid::now_v7();
    let slug = format!("husk-reassign-{}", &team.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $2)")
        .bind(team)
        .bind(&slug)
        .execute(pool)
        .await
        .expect("insert team");
    for (profile, role) in members {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             VALUES ($1, $2, $3::team_role)",
        )
        .bind(team)
        .bind(*profile)
        .bind(*role)
        .execute(pool)
        .await
        .expect("add team member");
    }
    team
}

/// A departing member's tombstone and husk are kept out of the bulk run twice over:
/// `team_scoped_owned` (the run's scope) enumerates live resources only, and the liveness floor in
/// the run's transaction skips one that died after the scope read. The run answers `200`, moves
/// the live resource, leaves the dead ones with their owner, and returns only the moved id.
/// (`BulkReassignAck` has no slot to count a skip; the skip is visible as the id's absence.)
///
/// FAILS IF a dead resource is moved, or fails the whole run. Each layer alone holds this test, so
/// the bite takes both: drop `JOIN kb_resources r ON r.id = h.resource_id AND r.is_active` from
/// `team_scoped_owned` AND, in `reassign_service::reassign_team_resources`, turn the floor's
/// refusal arm into `return Err(..)` (the run answers `403`/`410` and nothing moves) or delete the
/// floor call (the husk and the tombstone move to the recipient). The scope half alone is bitten
/// by `remove_member_and_team_reassign_leave_out_a_husk_and_a_tombstone`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_reassign_skips_an_erased_and_a_deleted_resource(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (admin, _) = caller(&app.pool, "admin").await;
    let (from, from_context) = caller(&app.pool, "departing").await;
    let (to, _) = caller(&app.pool, "recipient").await;
    let team = team_with(
        &app.pool,
        &[
            (admin.profile, "owner"),
            (from.profile, "member"),
            (to.profile, "member"),
        ],
    )
    .await;
    sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(from_context)
        .bind(team)
        .execute(&app.pool)
        .await
        .expect("share the departing member's context to the team");

    let live = ingest(&app, &from, from_context).await;
    let husk = ingest(&app, &from, from_context).await;
    let tombstone = ingest(&app, &from, from_context).await;
    erase(&app, husk).await;
    delete(&app, &from, tombstone).await;

    let (status, body) = call(
        &app,
        &admin,
        Method::POST,
        format!("/api/teams/{team}/reassign"),
        Some(json!({ "from_profile_id": from.profile, "to_profile_id": to.profile })),
    )
    .await;
    assert_eq!(status, 200, "the run completes; body: {body}");
    assert_eq!(
        body["resource_ids"],
        json!([live]),
        "only the live resource was reassigned; body: {body}"
    );

    assert_eq!(
        home_owner(&app.pool, live).await,
        to.profile,
        "the live one moved"
    );
    assert_eq!(
        home_owner(&app.pool, husk).await,
        from.profile,
        "the husk did not move"
    );
    assert_eq!(
        home_owner(&app.pool, tombstone).await,
        from.profile,
        "the tombstone did not move"
    );
}

// ── WITNESS: a system admin is refused on a tombstone and answered 410 on a husk they hold ────

/// `who` grants read on `resource` to a fresh principal id: the status and body.
async fn grant_as(app: &common::TestApp, who: &Caller, resource: Uuid) -> (u16, Value) {
    call(
        app,
        who,
        Method::POST,
        format!("/api/resources/{resource}/grants"),
        Some(json!({
            "principal_table": "kb_profiles",
            "principal_id": Uuid::now_v7(),
            "can_read": true,
            "can_write": false,
            "can_delete": false,
            "can_grant": false,
        })),
    )
    .await
}

/// `GrantAuthority::resolve`'s system-admin arm used to return before any subject-liveness check,
/// so an admin was admitted to administer grants on a tombstone or a husk that every other arm
/// refuses (`can()`'s floor, migration `20260902000010`). The admin arm now carries the same
/// liveness floor for a resource subject:
///
/// * an admin who is NOT a holder: `403` on a tombstone and on a husk (grant and revoke alike —
///   one authority gates both verbs);
/// * an admin who owns the husk: the grant door's classification, `410`.
///
/// FAILS IF a system admin is admitted on a dead resource. The bite: in `GrantAuthority::resolve`,
/// return `GrantAuthority::SystemAdmin` unconditionally in the admin arm — the outsider admin then
/// gets `200` on the tombstone and the husk, and the owner-admin `200` on their husk.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_system_admin_is_refused_on_a_dead_resource(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner_admin, owner_context) = caller(&app.pool, "owner-admin").await;
    common::fixtures::make_test_admin(&app.pool, owner_admin.profile).await;
    let (outsider_admin, _) = caller(&app.pool, "outsider-admin").await;
    common::fixtures::make_test_admin(&app.pool, outsider_admin.profile).await;
    let (other, other_context) = caller(&app.pool, "other").await;

    let husk = ingest(&app, &owner_admin, owner_context).await;
    let tombstone = ingest(&app, &other, other_context).await;
    let live = ingest(&app, &other, other_context).await;

    let (status, body) = grant_as(&app, &outsider_admin, live).await;
    assert_eq!(
        status, 200,
        "precondition: the admin arm admits a live resource it does not own; body: {body}"
    );

    erase(&app, husk).await;
    delete(&app, &other, tombstone).await;

    let (status, body) = grant_as(&app, &owner_admin, husk).await;
    assert_eq!(status, 410, "owner-admin on their husk; body: {body}");
    assert_eq!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");

    for (resource, label) in [(husk, "husk"), (tombstone, "tombstone")] {
        let (status, body) = grant_as(&app, &outsider_admin, resource).await;
        assert_eq!(
            status, 403,
            "outsider admin grant on a {label}; body: {body}"
        );
        assert_ne!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");

        let (status, body) = call(
            &app,
            &outsider_admin,
            Method::DELETE,
            format!("/api/resources/{resource}/grants"),
            Some(json!({
                "principal_table": "kb_profiles",
                "principal_id": Uuid::now_v7(),
            })),
        )
        .await;
        assert_eq!(
            status, 403,
            "outsider admin revoke on a {label}; body: {body}"
        );
        assert_ne!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");
    }
}

// ── WITNESS: an edge assert racing the act into its TARGET waits, then answers 404 ────────────

/// Live, unfolded `source → target` edges.
async fn live_edges(pool: &PgPool, source: Uuid, target: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_edges \
          WHERE source_table = 'kb_resources' AND source_id = $1 \
            AND target_table = 'kb_resources' AND target_id = $2 \
            AND NOT is_folded",
    )
    .bind(source)
    .bind(target)
    .fetch_one(pool)
    .await
    .expect("edge count")
}

/// The owner of a live source S asserts S → R while the act holds R. The target clause
/// (`check_endpoint_readable_in_tx`) locks R `FOR KEY SHARE` before it reads, so the assert waits
/// on the act; once the act commits, the read sees the husk and refuses `404` — the target floor's
/// answer, never `410` — and no edge lands on R.
///
/// FAILS IF the target check is not serialized against the act. The bite: delete the
/// `write_floor::lock_resource_key_share(..)` call in `check_endpoint_readable_in_tx`. The read
/// then admits R on the pre-act snapshot, and the edge write reaches
/// `_project_relationship_asserted`'s `_resource_write_guard` on R, which waits on the act and
/// then RAISEs: the assert answers `500`, not `404`. (No edge lands either way: the guard is the
/// substrate's backstop; the lock is what turns it into the door's own refusal.)
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_edge_assert_racing_the_act_into_its_target_answers_404_and_lands_nothing(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let source = ingest(&app, &owner, own_context).await;
    let target = ingest(&app, &owner, own_context).await;

    let act = hold_the_act(&app, target).await;
    let (status, body) = raced_by_the_act(
        &app,
        act,
        &owner,
        Method::POST,
        "/api/relationships".to_string(),
        json!({
            "source": source,
            "target": target,
            "edge_kind": "leads_to",
            "polarity": "forward",
            "label": "raced-into-a-husk",
            "weight": 1.0,
        }),
    )
    .await;

    assert!(
        is_erased(&app.pool, target).await,
        "precondition: the act committed an erasure"
    );
    assert_eq!(
        status, 404,
        "the target read, after the act, refuses the husk; body: {body}"
    );
    assert_ne!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");
    assert_eq!(
        live_edges(&app.pool, source, target).await,
        0,
        "no live edge landed on the husk"
    );
}

// ── WITNESS: a blob relate racing the act onto its PEER waits, then answers 404 ───────────────

/// The owner relates a blob to R while the act holds R. The peer check and the relation write
/// share one transaction, and the peer is locked `FOR KEY SHARE` before it is read, so the relate
/// waits on the act; once the act commits, the read refuses the husk `404` ("relation peer not
/// found or not readable") and no edge touches the blob.
///
/// FAILS IF the peer check is not serialized against the act. The bite: delete the
/// `crate::backend::write_floor::lock_resource_key_share(..)` call in
/// `blob_service::check_peer_readable`. The read then admits R on the pre-act snapshot, and the
/// relation write reaches `_project_relationship_asserted`'s `_resource_write_guard` on R, which
/// waits on the act and then RAISEs: the relate answers `500`, not `404`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_blob_relate_racing_the_act_onto_its_peer_answers_404_and_lands_nothing(pool: PgPool) {
    let app = blob_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let peer = ingest(&app, &owner, own_context).await;
    let blob = commit_blob(&app, &owner, own_context).await;

    let act = hold_the_act(&app, peer).await;
    let (status, body) = raced_by_the_act(
        &app,
        act,
        &owner,
        Method::POST,
        format!("/api/blobs/{blob}/relations"),
        json!({
            "direction": "blob_as_source",
            "peer_table": "kb_resources",
            "peer_id": peer,
            "edge_kind": "express",
            "polarity": "forward",
            "label": "raced-husk-peer",
            "weight": 1.0,
        }),
    )
    .await;

    assert!(
        is_erased(&app.pool, peer).await,
        "precondition: the act committed an erasure"
    );
    assert_eq!(
        status, 404,
        "the peer read, after the act, refuses the husk; body: {body}"
    );
    assert_ne!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");
    assert_eq!(
        blob_edges(&app.pool, blob).await,
        0,
        "no relation landed on the husk"
    );
}

// ── WITNESS: the owner's grant racing the act waits, then answers 410 and lands no row ────────

/// Grant rows naming `principal` on `subject`.
async fn grant_rows(pool: &PgPool, subject: Uuid, principal: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_access_grants \
          WHERE subject_table = 'kb_resources' AND subject_id = $1 \
            AND principal_table = 'kb_profiles' AND principal_id = $2",
    )
    .bind(subject)
    .bind(principal)
    .fetch_one(pool)
    .await
    .expect("grant row count")
}

/// The owner grants read on R while the act holds R. The authority gate runs on the pool and sees
/// R live (the act has not committed); the door's subject floor then locks R `FOR KEY SHARE` at the
/// head of the grant write's transaction and waits on the act. Once the act commits, the floor
/// sees the husk and classifies it: `410 RESOURCE_ERASED` to its owner. No grant row lands.
///
/// FAILS IF the grant write is not serialized against the act. The bite: delete the
/// `grant_subject_floor_in_tx(&mut tx, caller, subject)` check (and its rollback arm) in
/// `access_service::grant_capability` — nothing in the grant write touches R's row, so the request
/// answers `200` without ever waiting on a lock (`a_backend_waits_on_a_lock` panics) and its row
/// survives the act.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_grant_racing_the_act_answers_its_owner_410_and_lands_no_row(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let resource = ingest(&app, &owner, own_context).await;
    let grantee = Uuid::now_v7();

    let act = hold_the_act(&app, resource).await;
    let (status, body) = raced_by_the_act(
        &app,
        act,
        &owner,
        Method::POST,
        format!("/api/resources/{resource}/grants"),
        json!({
            "principal_table": "kb_profiles",
            "principal_id": grantee,
            "can_read": true,
            "can_write": false,
            "can_delete": false,
            "can_grant": false,
        }),
    )
    .await;

    assert!(
        is_erased(&app.pool, resource).await,
        "precondition: the act committed an erasure"
    );
    assert_eq!(
        status, 410,
        "the subject floor, after the act, refuses the husk; body: {body}"
    );
    assert_eq!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");
    assert_eq!(
        grant_rows(&app.pool, resource, grantee).await,
        0,
        "no grant row landed on the husk"
    );
}

// ── WITNESS: the owner administers no grant on a dead resource ────────────────────────────────

/// `who` revokes `principal`'s grant on `resource` through `DELETE /api/resources/{id}/grants`.
async fn revoke_as(
    app: &common::TestApp,
    who: &Caller,
    resource: Uuid,
    principal: Uuid,
) -> (u16, Value) {
    call(
        app,
        who,
        Method::DELETE,
        format!("/api/resources/{resource}/grants"),
        Some(json!({
            "principal_table": "kb_profiles",
            "principal_id": principal,
        })),
    )
    .await
}

/// The owner's derived `grant` arm (`derived_access_profile`, migration `20261002000010`) answers a
/// live resource only, so the owner may not administer grants on a tombstone or a husk:
///
/// * revoke on a live resource: admitted (`200`) — the precondition that the owner arm is live;
/// * revoke on a tombstone: `403`, never `410`;
/// * revoke on a husk the owner holds: `410 RESOURCE_ERASED`;
/// * an all-false grant on a tombstone: `403`. Attenuation has nothing to check on an all-false
///   request, so only the authority arm (or the subject floor) can refuse it.
///
/// FAILS IF the owner is admitted on a dead resource. Two layers hold each refusal — the derived
/// arm (authority) and the door's in-transaction subject floor — so the bite takes both: revert
/// the `AND EXISTS (SELECT 1 FROM kb_resources r WHERE r.id = p_subject_id AND r.is_active)`
/// conjunct on the `grant` arm in `20261002000010` AND delete the `grant_subject_floor_in_tx(..)`
/// calls in `access_service`: the tombstone revoke and the all-false grant then answer `200`. The
/// arm alone is bitten by `can_subject_liveness_test.rs`'s
/// `the_owners_derived_grant_and_delete_close_on_a_tombstone`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_owner_administers_no_grant_on_a_dead_resource(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let live = ingest(&app, &owner, own_context).await;
    let tombstone = ingest(&app, &owner, own_context).await;
    let husk = ingest(&app, &owner, own_context).await;
    let principal = Uuid::now_v7();

    let (status, body) = revoke_as(&app, &owner, live, principal).await;
    assert_eq!(
        status, 200,
        "precondition: the owner administers grants on a live resource; body: {body}"
    );

    delete(&app, &owner, tombstone).await;
    erase(&app, husk).await;

    let (status, body) = revoke_as(&app, &owner, tombstone, principal).await;
    assert_eq!(status, 403, "owner revoke on a tombstone; body: {body}");
    assert_ne!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");

    let (status, body) = revoke_as(&app, &owner, husk, principal).await;
    assert_eq!(status, 410, "owner revoke on their husk; body: {body}");
    assert_eq!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");

    let (status, body) = call(
        &app,
        &owner,
        Method::POST,
        format!("/api/resources/{tombstone}/grants"),
        Some(json!({
            "principal_table": "kb_profiles",
            "principal_id": principal,
            "can_read": false,
            "can_write": false,
            "can_delete": false,
            "can_grant": false,
        })),
    )
    .await;
    assert_eq!(
        status, 403,
        "owner all-false grant on a tombstone; body: {body}"
    );
    assert_ne!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");
    assert_eq!(
        grant_rows(&app.pool, tombstone, principal).await,
        0,
        "no grant row landed on the tombstone"
    );
}

// ── WITNESS: the residual warning and the team handoff count live resources only ──────────────

/// `remove_member`'s residual warning and the team handoff read one scope,
/// `reassign_service::team_scoped_owned`, which enumerates live resources only: a departing
/// member's husk and tombstone are nothing to hand off. The member owns three resources in a
/// context shared to the team — one live, one erased, one deleted. Removing them answers a
/// residual of exactly the live one; the handoff then moves exactly the live one.
///
/// FAILS IF the scope enumerates a dead resource. The bite: drop
/// `JOIN kb_resources r ON r.id = h.resource_id AND r.is_active` from `team_scoped_owned` — the
/// residual then counts `3`. (The handoff's `resource_ids` stays `[live]` under that bite: its
/// liveness floor skips the dead ones, see `team_reassign_skips_an_erased_and_a_deleted_resource`.)
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn remove_member_and_team_reassign_leave_out_a_husk_and_a_tombstone(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (admin, _) = caller(&app.pool, "admin").await;
    let (from, from_context) = caller(&app.pool, "departing").await;
    let (to, _) = caller(&app.pool, "recipient").await;
    let team = team_with(
        &app.pool,
        &[
            (admin.profile, "owner"),
            (from.profile, "member"),
            (to.profile, "member"),
        ],
    )
    .await;
    sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(from_context)
        .bind(team)
        .execute(&app.pool)
        .await
        .expect("share the departing member's context to the team");

    let live = ingest(&app, &from, from_context).await;
    let husk = ingest(&app, &from, from_context).await;
    let tombstone = ingest(&app, &from, from_context).await;
    erase(&app, husk).await;
    delete(&app, &from, tombstone).await;

    let (status, body) = call(
        &app,
        &admin,
        Method::DELETE,
        format!("/api/teams/{team}/members/{}", from.profile),
        None,
    )
    .await;
    assert_eq!(status, 200, "the admin removes the member; body: {body}");
    assert_eq!(
        body["residual_owned"]["count"], 1,
        "the residual counts the live resource only; body: {body}"
    );
    let contexts = body["residual_owned"]["contexts"]
        .as_array()
        .expect("residual contexts");
    assert_eq!(contexts.len(), 1, "one shared context; body: {body}");
    assert_eq!(
        contexts[0]["count"], 1,
        "its count is the live resource only; body: {body}"
    );

    let (status, body) = call(
        &app,
        &admin,
        Method::POST,
        format!("/api/teams/{team}/reassign"),
        Some(json!({ "from_profile_id": from.profile, "to_profile_id": to.profile })),
    )
    .await;
    assert_eq!(status, 200, "the handoff completes; body: {body}");
    assert_eq!(
        body["resource_ids"],
        json!([live]),
        "the handoff moves the live resource only; body: {body}"
    );
}

// ── WITNESS: a create replaying its idempotency key onto a dead resource ──────────────────────

/// A segmented begin (`POST /api/ingest` with `segmented` set) by `owner` into `context`, carrying
/// `key`: the status and the parsed body. Block 0 is [`BODY`] with its real chunk, so no ONNX.
async fn segmented_begin(
    app: &common::TestApp,
    owner: &Caller,
    context: Uuid,
    key: Uuid,
) -> (u16, Value) {
    let payload = IngestPayload {
        idempotency_key: Some(key),
        segmented: Some(SegmentedBegin {
            total_blocks_hint: Some(2),
            block_budget: 262_144,
            source_hash: None,
        }),
        title: TITLE.to_string(),
        origin_uri: format!("test://husk-write-segmented-{}", Uuid::new_v4()),
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
    call(
        app,
        owner,
        Method::POST,
        "/api/ingest".to_string(),
        Some(serde_json::to_value(payload).expect("segmented begin body")),
    )
    .await
}

/// The resource a segmented begin answered with.
fn begun(status: u16, body: &Value) -> Uuid {
    assert_eq!(
        status, 200,
        "the owner begins a segmented ingest; body: {body}"
    );
    Uuid::parse_str(body["resource_id"].as_str().expect("resource_id")).expect("resource uuid")
}

/// A segmented begin that replays its idempotency key converges on the already-created resource
/// (`create_resource_unread` returns the claimed id without minting) and then writes the
/// ingestion record through its floor — before any read of the resource. So a replay onto a
/// since-deleted resource is the write floor's `403`, and onto a since-erased one the floor's
/// classification: `410 RESOURCE_ERASED` to its owner, a husk holder. The key is owner-scoped
/// (`kb_idempotency_keys`' PK), so no one else can replay it.
///
/// FAILS IF a replay onto a dead resource answers anything else. The bites: in
/// `DbBackend::begin_segmented_ingest`, read the resource back before recording the source
/// (replace `self.create_resource_unread(cmd, true).await?` with
/// `ResourceId::from(self.create_resource_inner(cmd, true).await?.value.id)`) — the deleted replay
/// then answers the readback's `404`; or delete the `write_floor::modify_floor_in_tx(..)` call in
/// `record_ingestion_source` — the deleted replay then answers `200`, writing a record onto a
/// tombstone.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_segmented_replay_onto_a_deleted_resource_answers_403(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let key = Uuid::now_v7();

    let (status, body) = segmented_begin(&app, &owner, own_context, key).await;
    let resource = begun(status, &body);
    delete(&app, &owner, resource).await;

    let (status, body) = segmented_begin(&app, &owner, own_context, key).await;
    assert_eq!(
        status, 403,
        "the replay converges on the tombstone and its record's floor refuses; body: {body}"
    );
    assert_eq!(body["error"]["code"], "FORBIDDEN", "body: {body}");

    let (is_active, erased): (bool, bool) =
        sqlx::query_as("SELECT is_active, erased_at IS NOT NULL FROM kb_resources WHERE id = $1")
            .bind(resource)
            .fetch_one(&app.pool)
            .await
            .expect("tombstone probe");
    assert!(!is_active && !erased, "the resource stayed a tombstone");
}

/// The erased half of [`a_segmented_replay_onto_a_deleted_resource_answers_403`]: the owner's
/// replay onto a since-erased resource answers `410 RESOURCE_ERASED`, the fixed message naming the
/// id. The bite: in `write_floor::erased_or_forbidden`, answer `Forbidden` unconditionally — the
/// replay then answers `403`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_segmented_replay_onto_an_erased_resource_answers_its_owner_410(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let key = Uuid::now_v7();

    let (status, body) = segmented_begin(&app, &owner, own_context, key).await;
    let resource = begun(status, &body);
    erase(&app, resource).await;

    let (status, body) = segmented_begin(&app, &owner, own_context, key).await;
    assert_eq!(
        status, 410,
        "the replay names a husk the owner holds; body: {body}"
    );
    assert_eq!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");
    assert_eq!(
        body["error"]["message"],
        format!("resource {resource} was erased"),
        "the fixed message names the id and nothing else"
    );
}

/// The two one-shot create doors, carrying `key`, by `owner` into `context`: the status and the
/// parsed body. Both are bodiless, so no ONNX.
async fn one_shot_creates(
    app: &common::TestApp,
    owner: &Caller,
    context: Uuid,
    ingest_key: Uuid,
    resources_key: Uuid,
) -> [(&'static str, u16, Value); 2] {
    let ingest = IngestPayload {
        idempotency_key: Some(ingest_key),
        segmented: None,
        title: TITLE.to_string(),
        origin_uri: format!("test://husk-write-oneshot-{}", Uuid::new_v4()),
        context_ref: context.to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        goal: None,
        act: Default::default(),
        sources: Vec::new(),
    };
    let (ingest_status, ingest_body) = call(
        app,
        owner,
        Method::POST,
        "/api/ingest".to_string(),
        Some(serde_json::to_value(ingest).expect("one-shot ingest body")),
    )
    .await;
    let (resources_status, resources_body) = call(
        app,
        owner,
        Method::POST,
        "/api/resources".to_string(),
        Some(json!({
            "kb_context_id": context,
            "doc_type": "research",
            "origin_uri": format!("test://husk-write-create-{}", Uuid::new_v4()),
            "title": TITLE,
            "idempotency_key": resources_key,
        })),
    )
    .await;
    [
        ("POST /api/ingest (one-shot)", ingest_status, ingest_body),
        ("POST /api/resources", resources_status, resources_body),
    ]
}

/// A one-shot create that replays its idempotency key converges on the already-created resource
/// and answers with its readback (`native_resource_view` → `show_view_select`, the read door's
/// classifier): `410 RESOURCE_ERASED` to the owner of a since-erased resource, and the read side's
/// `404` for a since-deleted one (a tombstone is never an erasure). Pinned on both create doors.
///
/// FAILS IF a one-shot replay onto an erased id answers its owner anything but `410`, or onto a
/// tombstone anything but `404`. The bite: in `substrate_read::show_view_select`, return the
/// miss's `not_found` instead of `erased_or(pool, profile_id, .., not_found)` — the erased replays
/// then answer `404`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_one_shot_replay_onto_a_dead_resource_answers_410_erased_and_404_deleted(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;

    for (dead, expected) in [("erased", 410u16), ("deleted", 404u16)] {
        let (ingest_key, resources_key) = (Uuid::now_v7(), Uuid::now_v7());
        let first = one_shot_creates(&app, &owner, own_context, ingest_key, resources_key).await;
        let mut created = Vec::new();
        for (name, status, body) in &first {
            assert_eq!(*status, 200, "{name}: the owner creates; body: {body}");
            let id = Uuid::parse_str(body["id"].as_str().expect("id")).expect("resource uuid");
            match dead {
                "erased" => erase(&app, id).await,
                _ => delete(&app, &owner, id).await,
            }
            created.push(id);
        }

        let replays = one_shot_creates(&app, &owner, own_context, ingest_key, resources_key).await;
        for ((name, status, body), id) in replays.iter().zip(&created) {
            assert_eq!(
                *status, expected,
                "{name}: a replay onto a {dead} resource; body: {body}"
            );
            if expected == 410 {
                assert_eq!(
                    body["error"]["code"], RESOURCE_ERASED,
                    "{name}; body: {body}"
                );
                assert_eq!(
                    body["error"]["message"],
                    format!("resource {id} was erased"),
                    "{name}: the replay converged on the erased id"
                );
            } else {
                assert_ne!(
                    body["error"]["code"], RESOURCE_ERASED,
                    "{name}; body: {body}"
                );
            }
        }
    }
}

// ── WITNESS: a write racing a soft delete waits for it, then is refused 403 ───────────────────

/// Hold a soft delete open on `resource`, as the delete door does it: `FOR UPDATE` on the row,
/// then the `ResourceDelete` seed, inside a transaction this test commits when it chooses.
async fn hold_the_delete(
    app: &common::TestApp,
    owner: &Caller,
    resource: Uuid,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let emitter: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
          WHERE e.profile_id = $1 AND e.name = p.handle || '@web'",
    )
    .bind(owner.profile)
    .fetch_one(&app.pool)
    .await
    .expect("the owner's web emitter");
    let mut tx = app.pool.begin().await.expect("begin the delete");
    sqlx::query("SELECT id FROM kb_resources WHERE id = $1 FOR UPDATE")
        .bind(resource)
        .execute(&mut *tx)
        .await
        .expect("the delete's row lock");
    temper_substrate::writes::delete_resource_in_tx(
        &mut tx,
        resource.into(),
        emitter.into(),
        temper_substrate::events::EventContext::default(),
    )
    .await
    .expect("the tombstone flip inside the open transaction");
    tx
}

/// FAILS IF a write that races a soft delete lands on the tombstone, or answers anything but the
/// write side's uniform `403` (never `410`: a tombstone is not an erasure). The delete door takes
/// `FOR UPDATE` on the row first, so the write floor's `FOR KEY SHARE` waits for the delete to
/// commit and then sees `is_active = false`. The delete here is [`hold_the_delete`], this test's
/// copy of the door's two steps, because a race needs the delete held open and the door commits
/// its own transaction. The bite: drop the `FOR UPDATE` statement from `hold_the_delete`. The
/// write is then admitted by the floor on the pre-delete snapshot, blocks only at its own row
/// UPDATE, lands on the tombstone once the delete commits, and answers `404` from its readback —
/// this test's `403` assertion fails. Dropping the `FOR UPDATE` from `DbBackend::delete_resource`
/// does NOT fail this test (the door is not what holds the row here); the door-driven witness
/// below, `the_delete_door_waits_on_a_writers_floor_then_completes`, is the one that does.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_write_racing_a_soft_delete_waits_for_it_and_answers_403(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let resource = ingest(&app, &owner, own_context).await;

    let delete = hold_the_delete(&app, &owner, resource).await;
    let (status, body) = raced_by_the_act(
        &app,
        delete,
        &owner,
        Method::PATCH,
        format!("/api/resources/{resource}"),
        json!({ "title": "raced-onto-a-tombstone" }),
    )
    .await;

    assert_eq!(
        status, 403,
        "the write after the delete is refused; body: {body}"
    );
    assert_ne!(
        body["error"]["code"], RESOURCE_ERASED,
        "a tombstone is never erased: {body}"
    );
    let title: String = sqlx::query_scalar("SELECT title FROM kb_resources WHERE id = $1")
        .bind(resource)
        .fetch_one(&app.pool)
        .await
        .expect("read the title");
    assert_ne!(
        title, "raced-onto-a-tombstone",
        "nothing landed on the tombstone"
    );
}

// ── WITNESS: the real delete door waits on a writer's floor, then completes ──────────────────

/// Hold a writer's floor on `resource`: `FOR KEY SHARE` on its row — the lock
/// `write_floor::modify_floor_in_tx` takes at the head of every floored write — in a transaction
/// this test commits when it chooses, as a writer that has passed its floor and not yet committed
/// holds it.
async fn hold_a_writers_floor(
    app: &common::TestApp,
    resource: Uuid,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = app.pool.begin().await.expect("begin the writer");
    sqlx::query("SELECT id FROM kb_resources WHERE id = $1 FOR KEY SHARE")
        .bind(resource)
        .execute(&mut *tx)
        .await
        .expect("the writer's floor lock");
    tx
}

/// Is `resource` live (`kb_resources.is_active`)?
async fn is_live(pool: &PgPool, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT is_active FROM kb_resources WHERE id = $1")
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("liveness probe")
}

/// FAILS IF `DELETE /api/resources/{id}` does not serialize against a writer that has passed its
/// floor. The door takes `FOR UPDATE` on the row before its in-transaction floor and its tombstone
/// flip; `FOR UPDATE` conflicts with the writer's `FOR KEY SHARE`, so the delete waits for the
/// writer to commit, and then completes.
///
/// The bite: delete the `SELECT id FROM kb_resources WHERE id = $1 FOR UPDATE` statement in
/// `DbBackend::delete_resource`. The floor's own lock is `FOR KEY SHARE` and the tombstone flip's
/// row update is `FOR NO KEY UPDATE`, neither of which conflicts with the writer's lock, so the
/// delete completes while the writer still holds its floor: `a_backend_waits_on_a_lock` panics
/// ("completed while the row was held").
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_delete_door_waits_on_a_writers_floor_then_completes(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let resource = ingest(&app, &owner, own_context).await;

    let writer = hold_a_writers_floor(&app, resource).await;
    let request = spawn_request(
        &app,
        &owner,
        Method::DELETE,
        &format!("/api/resources/{resource}"),
        None,
    );
    a_backend_waits_on_a_lock(&app.pool, &request, "DELETE /api/resources/{id}").await;
    assert!(
        is_live(&app.pool, resource).await,
        "the delete has not landed while the writer holds its floor"
    );

    writer.commit().await.expect("commit the writer");
    let (status, body) = request.await.expect("the delete request must not panic");
    assert_eq!(
        status, 200,
        "the delete completes once the writer commits; body: {body}"
    );
    assert!(
        !is_live(&app.pool, resource).await,
        "the delete landed: the resource is a tombstone"
    );
}

// ── WITNESS: a door with no fast-fail, racing the act, answers 410 and lands nothing ─────────

/// `block_provenance_annotated` events in this test's database.
async fn annotations(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'block_provenance_annotated'",
    )
    .fetch_one(pool)
    .await
    .expect("annotation count")
}

/// The owner annotates R (`POST /api/resources/{id}/provenance`) while the act holds R. The door
/// runs no pool fast-fail: its one check is the modify floor at the head of its transaction
/// (`DbBackend::begin_floored`), whose `FOR KEY SHARE` waits on the act's `FOR UPDATE`. Once the act
/// commits, the floor sees the husk and classifies it: `410 RESOURCE_ERASED` to its owner. No
/// annotation lands.
///
/// FAILS IF the annotate's floor is not inside its transaction. The bite: in
/// `DbBackend::annotate_resource`, open the transaction with `self.pool.begin()` instead of
/// `self.begin_floored(..)`. Nothing then answers the floor's `410`: the annotate either completes
/// before the act commits (`a_backend_waits_on_a_lock` panics) or meets the substrate after it
/// (the erased resource's block lookup or its write guard) and answers a non-`410` error — this
/// test's `410` assertion fails. Which of the two is unverified; both fail the test.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_annotate_racing_the_act_answers_its_owner_410_and_lands_nothing(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let resource = ingest(&app, &owner, own_context).await;
    assert_eq!(
        annotations(&app.pool).await,
        0,
        "precondition: the ingest annotated nothing"
    );

    let act = hold_the_act(&app, resource).await;
    let (status, body) = raced_by_the_act(
        &app,
        act,
        &owner,
        Method::POST,
        format!("/api/resources/{resource}/provenance"),
        json!({
            "sources": [{ "kind": "remote", "value": "https://example.com/raced-annotate" }],
        }),
    )
    .await;

    assert!(
        is_erased(&app.pool, resource).await,
        "precondition: the act committed an erasure"
    );
    assert_eq!(
        status, 410,
        "the floor, after the act, refuses the husk; body: {body}"
    );
    assert_eq!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");
    assert_eq!(
        annotations(&app.pool).await,
        0,
        "no annotation landed on the husk"
    );
}

// ── WITNESS: a goal-set locks the goal before it writes anything ──────────────────────────────

/// `RowExclusiveLock`s the backend `pid` holds — one per relation its transaction has written.
async fn row_exclusive_locks(pool: &PgPool, pid: i32) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_locks WHERE pid = $1 AND mode = 'RowExclusiveLock' AND granted",
    )
    .bind(pid)
    .fetch_one(pool)
    .await
    .expect("lock count")
}

/// The owner PATCHes R with a new title AND a goal G while the act holds G. The update floors R,
/// then locks G's row up front (`DbBackend::update_resource`'s lock-order rule: every
/// `kb_resources` row the write touches is locked before any other row lock), so it waits on the
/// act having written nothing. Once the act commits, the goal's target clause reads G erased and
/// refuses it `404` (the endpoint read floor, never `410` — the caller is not writing G), the whole
/// update rolls back, and the title does not land. Never a `500`.
///
/// The deadlock this order prevents (the act holding G's `FOR UPDATE` and then folding edges the
/// update had already locked, while the update waits on G) needs the act mid-flight; a held act
/// has already folded, so the deadlock itself is not deterministically reachable here. What is
/// pinned is the order that prevents it: while the update waits on G, its backend holds no
/// `RowExclusiveLock` — it has written no row anywhere.
///
/// FAILS IF the update writes before it locks the goal. The bite: delete the up-front
/// `self.lock_goal_rows(..)` block in `DbBackend::update_resource`.
/// The update then runs `update_resource_in_tx` (the retitle appends an event and updates R's
/// row) before the target clause's lock waits on G, so its backend holds `RowExclusiveLock`s
/// while it waits and the zero assertion fails.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_goal_set_racing_the_acts_erasure_of_the_goal_locks_it_first_and_answers_404(
    pool: PgPool,
) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let resource = ingest(&app, &owner, own_context).await;
    let goal = ingest(&app, &owner, own_context).await;

    let act = hold_the_act(&app, goal).await;
    let request = spawn_request(
        &app,
        &owner,
        Method::PATCH,
        &format!("/api/resources/{resource}"),
        Some(json!({ "title": "raced-goal-set", "goal": goal })),
    );
    let waiter = a_backend_waits_on_a_lock(&app.pool, &request, "PATCH {goal}").await;
    assert_eq!(
        row_exclusive_locks(&app.pool, waiter).await,
        0,
        "the update waits on the goal's row having written nothing"
    );
    act.commit().await.expect("commit the act");
    let (status, body) = request.await.expect("the raced update must not panic");

    assert!(
        is_erased(&app.pool, goal).await,
        "precondition: the act committed an erasure of the goal"
    );
    assert_eq!(
        status, 404,
        "the goal's target clause, after the act, refuses the husk; body: {body}"
    );
    assert_ne!(body["error"]["code"], RESOURCE_ERASED, "body: {body}");
    assert_eq!(
        title_of(&app, &owner, resource).await,
        TITLE,
        "the refused goal-set rolled the whole update back — the title did not land"
    );
}

// ── WITNESS: a refused caller takes no row lock ───────────────────────────────────────────────

/// Hold `FOR UPDATE` on `resource`'s row in an open transaction — the lock the erasure act takes —
/// without erasing anything. Any request that asks for the row's `FOR KEY SHARE` waits on it.
async fn hold_the_row(pool: &PgPool, resource: Uuid) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut held = pool.begin().await.expect("begin the holder");
    sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1 FOR UPDATE")
        .bind(resource)
        .execute(&mut *held)
        .await
        .expect("hold the row");
    held
}

/// Await `request`, failing if it has not answered within five seconds: a refused caller's request
/// must not be waiting on the held row.
async fn answers_without_waiting(
    request: impl std::future::Future<Output = (u16, Value)>,
    what: &str,
) -> (u16, Value) {
    tokio::time::timeout(std::time::Duration::from_secs(5), request)
        .await
        .unwrap_or_else(|_| panic!("{what}: a refused caller waited on the held row lock"))
}

/// While R's row is held `FOR UPDATE` (the erasure act's lock), a caller with no standing on R is
/// refused at every door that would lock R on its behalf — every floored write door, an edge from
/// its own resource into R, a blob relation onto R, and a goal set naming R — and is refused
/// promptly, never queued behind the held lock (`write_floor`'s "a refused caller takes no lock").
///
/// FAILS IF any of those paths locks R before deciding the caller is refused. The bite: delete the
/// unlocked `modify_admission(..)` call at the head of `write_floor::modify_floor_in_tx` (or the
/// unlocked `endpoint_readable_on(..)` in `check_endpoint_readable_in_tx`, the unlocked
/// `peer_readable_on(..)` in `blob_service::check_peer_readable`, or the one in
/// `DbBackend::lock_goal_rows`) — that request queues for R's `FOR KEY SHARE` behind the held
/// `FOR UPDATE` and does not answer within the deadline.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_refused_caller_answers_without_taking_the_row_lock(pool: PgPool) {
    let app = blob_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let (stranger, stranger_context) = caller(&app.pool, "stranger").await;
    let resource = ingest(&app, &owner, own_context).await;
    let own = ingest(&app, &stranger, stranger_context).await;
    let blob = commit_blob(&app, &stranger, stranger_context).await;

    let held = hold_the_row(&app.pool, resource).await;

    for door in doors(resource) {
        let (status, body) = answers_without_waiting(send(&app, &stranger, &door), door.name).await;
        assert_eq!(
            status, 403,
            "{}: a caller with no standing; body: {body}",
            door.name
        );
    }

    let (status, body) = answers_without_waiting(
        call(
            &app,
            &stranger,
            Method::POST,
            "/api/relationships".to_string(),
            Some(json!({
                "source": own,
                "target": resource,
                "edge_kind": "leads_to",
                "polarity": "forward",
                "label": "into-an-unreadable-target",
                "weight": 1.0,
            })),
        ),
        "edge assert into R",
    )
    .await;
    assert_eq!(status, 404, "the target read refuses; body: {body}");

    let (status, body) = answers_without_waiting(
        call(
            &app,
            &stranger,
            Method::POST,
            format!("/api/blobs/{blob}/relations"),
            Some(json!({
                "direction": "blob_as_source",
                "peer_table": "kb_resources",
                "peer_id": resource,
                "edge_kind": "express",
                "polarity": "forward",
                "label": "onto-an-unreadable-peer",
                "weight": 1.0,
            })),
        ),
        "blob relate onto R",
    )
    .await;
    assert_eq!(status, 404, "the peer read refuses; body: {body}");

    let (status, body) = answers_without_waiting(
        call(
            &app,
            &stranger,
            Method::PATCH,
            format!("/api/resources/{own}"),
            Some(json!({ "title": "goal-set onto R", "goal": resource })),
        ),
        "goal set naming R",
    )
    .await;
    assert_eq!(status, 404, "the goal's read refuses; body: {body}");

    held.rollback().await.expect("release the row");
}

// ── WITNESS: a goal change racing the act on the CURRENT goal folds its edge once ─────────────

/// `relationship_folded` events naming `edge`.
async fn fold_events(pool: &PgPool, edge: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'relationship_folded' AND e.payload->>'edge_id' = $1",
    )
    .bind(edge.to_string())
    .fetch_one(pool)
    .await
    .expect("fold event count")
}

/// The id of `source`'s live goal edge.
async fn goal_edge(pool: &PgPool, source: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM kb_edges \
          WHERE source_table = 'kb_resources' AND source_id = $1 \
            AND edge_kind = 'leads_to' AND NOT is_folded",
    )
    .bind(source)
    .fetch_one(pool)
    .await
    .expect("the goal edge")
}

/// R advances goal G. The owner clears R's goal while the act holds G. The act has folded the
/// edge R → G in its open transaction, so the clear must not fold it a second time: the update
/// locks G's row up front (`DbBackend::lock_goal_rows`, the CURRENT goal's row), waits on the act,
/// and once the act commits reads the edge already folded. One `relationship_folded` for the
/// edge, the act's — never two.
///
/// FAILS IF neither of the two guards holds — the update's lock on the current goal's row, and
/// `fold_goal_edges`' lock-and-recheck of the edge. Either alone keeps the count at 1, so the bite
/// removes both: start `rows` empty in `DbBackend::lock_goal_rows` AND drop `FOR UPDATE OF e` from
/// `fold_goal_edges`. The clear then reads the edge unfolded on its own snapshot, fires its fold,
/// waits in the projector on the act's lock on the edge row, and appends a second
/// `relationship_folded` once the act commits — the count reads 2. The fold's guard alone is
/// pinned by `a_goal_clear_never_folds_an_edge_folded_under_it`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_goal_clear_racing_the_acts_erasure_of_the_goal_folds_the_edge_once(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let resource = ingest(&app, &owner, own_context).await;
    let goal = ingest_typed(&app, &owner, own_context, "goal").await;
    let (status, body) = call(
        &app,
        &owner,
        Method::PATCH,
        format!("/api/resources/{resource}"),
        Some(json!({ "goal": goal })),
    )
    .await;
    assert_eq!(status, 200, "precondition: the goal is set; body: {body}");
    let edge = goal_edge(&app.pool, resource).await;

    let act = hold_the_act(&app, goal).await;
    let request = spawn_request(
        &app,
        &owner,
        Method::PATCH,
        &format!("/api/resources/{resource}"),
        Some(json!({ "clear_goal": true })),
    );
    a_backend_waits_on_a_lock(&app.pool, &request, "PATCH clear_goal").await;
    act.commit().await.expect("commit the act");
    let (status, body) = request.await.expect("the raced clear must not panic");

    assert!(
        is_erased(&app.pool, goal).await,
        "precondition: the act committed an erasure of the goal"
    );
    assert_eq!(status, 200, "the clear lands after the act; body: {body}");
    assert_eq!(
        fold_events(&app.pool, edge).await,
        1,
        "the edge was folded once — by the act — and the clear did not fold it again"
    );
}

// ── WITNESS: goal patches on one resource serialize, and a fold never folds a folded edge ─────

/// A goal patch on R waits while another transaction holds R's goal-patch lock — the
/// transaction-scoped advisory lock keyed `goal_patch:<R>` that `DbBackend::lock_goal_rows` takes —
/// and lands once it is released. Two goal patches on one resource therefore cannot both read the
/// same current goal edge and both assert their own goal (two live goal edges on one resource).
///
/// FAILS IF goal patches do not serialize. The bite: delete the `pg_advisory_xact_lock` select at
/// the head of `lock_goal_rows`. A goal-only PATCH then takes nothing the held lock conflicts with
/// and completes while it is held.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_goal_patch_waits_on_another_goal_patch_of_the_same_resource(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let resource = ingest(&app, &owner, own_context).await;
    let goal = ingest_typed(&app, &owner, own_context, "goal").await;

    let mut held = app.pool.begin().await.expect("begin the holder");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("goal_patch:{resource}"))
        .execute(&mut *held)
        .await
        .expect("hold the goal-patch lock as another goal patch does");
    let request = spawn_request(
        &app,
        &owner,
        Method::PATCH,
        &format!("/api/resources/{resource}"),
        Some(json!({ "goal": goal })),
    );
    a_backend_waits_on_a_lock(&app.pool, &request, "PATCH goal").await;
    held.rollback().await.expect("release the goal-patch lock");
    let (status, body) = request.await.expect("the goal patch must not panic");
    assert_eq!(
        status, 200,
        "the goal patch lands once released; body: {body}"
    );
}

/// A goal patch and an ordinary update of the same resource both complete; neither answers a
/// deadlock (`40P01`). The other update (held here) takes the resource row `FOR NO KEY UPDATE`
/// first, then folds a property row the goal patch also sets, then writes the resource row: the
/// order every update takes since the head of `update_resource_in_tx` locks the row first (task
/// 01a0fd62-bb17-7442-8b01-11b2c6e01319). The goal patch waits on the row, holding only the
/// goal-patch advisory lock and the goal rows' `FOR KEY SHARE`, none of which the update needs.
///
/// This test first held the property row and THEN wrote the resource row, the order a
/// title-and-meta update took before that fix. That order is gone from every update, and a
/// transaction that still took it would now deadlock with ANY update of the resource, goal patch
/// or not; `temper-substrate/tests/update_lock_order.rs` pins that. Its old bite (taking the
/// source `FOR NO KEY UPDATE` in `DbBackend::lock_goal_rows` instead of the advisory lock) no
/// longer fires: with every update taking the row first, a goal patch holding it early only waits.
/// The advisory lock is kept for its other job, serializing goal patches before they read the
/// current goal edges (`lock_goal_rows`, step 1).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_goal_patch_does_not_deadlock_with_an_update_of_the_same_resource(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let resource = ingest(&app, &owner, own_context).await;
    let goal = ingest_typed(&app, &owner, own_context, "goal").await;
    let (status, body) = call(
        &app,
        &owner,
        Method::PATCH,
        format!("/api/resources/{resource}"),
        Some(json!({ "open_meta": { "note": "first" } })),
    )
    .await;
    assert_eq!(status, 200, "precondition: a live `note`; body: {body}");

    let mut held = app.pool.begin().await.expect("begin the other update");
    sqlx::query("SELECT id FROM kb_resources WHERE id = $1 FOR NO KEY UPDATE")
        .bind(resource)
        .execute(&mut *held)
        .await
        .expect("the other update takes the resource row first");
    sqlx::query(
        "SELECT id FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1 \
            AND property_key = 'note' AND NOT is_folded \
          FOR UPDATE",
    )
    .bind(resource)
    .execute(&mut *held)
    .await
    .expect("hold the `note` row");
    let request = spawn_request(
        &app,
        &owner,
        Method::PATCH,
        &format!("/api/resources/{resource}"),
        Some(json!({ "goal": goal, "open_meta": { "note": "second" } })),
    );
    a_backend_waits_on_a_lock(&app.pool, &request, "PATCH goal + note").await;

    let wrote = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        sqlx::query("UPDATE kb_resources SET title = title WHERE id = $1")
            .bind(resource)
            .execute(&mut *held),
    )
    .await
    .expect("the other update's resource-row write must not hang");
    assert!(
        wrote.is_ok(),
        "the other update's resource-row write must not deadlock with the goal patch: {wrote:?}"
    );
    held.commit().await.expect("commit the other update");

    let (status, body) = request.await.expect("the goal patch must not panic");
    assert_eq!(
        status, 200,
        "the goal patch lands after the other update; body: {body}"
    );
}

/// R's goal edge is folded by another transaction that has not yet committed (it holds the edge
/// row). A goal clear on R waits on that row, and once the other transaction commits it reads the
/// edge folded and does not fold it again: no `relationship_folded` from the clear. Whoever folded
/// it first — the erasure act, or a concurrent write — the clear never appends a second one.
///
/// FAILS IF `fold_goal_edges` reads the edges without locking them. The bite: drop `FOR UPDATE OF
/// e` from its select. The clear then reads the edge unfolded on its own snapshot, fires its fold,
/// waits in the projector on the held row, and appends a `relationship_folded` once released.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_goal_clear_never_folds_an_edge_folded_under_it(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let resource = ingest(&app, &owner, own_context).await;
    let goal = ingest_typed(&app, &owner, own_context, "goal").await;
    let (status, body) = call(
        &app,
        &owner,
        Method::PATCH,
        format!("/api/resources/{resource}"),
        Some(json!({ "goal": goal })),
    )
    .await;
    assert_eq!(status, 200, "precondition: the goal is set; body: {body}");
    let edge = goal_edge(&app.pool, resource).await;

    let mut held = app.pool.begin().await.expect("begin the folder");
    sqlx::query("UPDATE kb_edges SET is_folded = true WHERE id = $1")
        .bind(edge)
        .execute(&mut *held)
        .await
        .expect("fold the edge, uncommitted");
    let request = spawn_request(
        &app,
        &owner,
        Method::PATCH,
        &format!("/api/resources/{resource}"),
        Some(json!({ "clear_goal": true })),
    );
    a_backend_waits_on_a_lock(&app.pool, &request, "PATCH clear_goal").await;
    held.commit().await.expect("commit the fold");
    let (status, body) = request.await.expect("the clear must not panic");

    assert_eq!(status, 200, "the clear lands; body: {body}");
    assert_eq!(
        fold_events(&app.pool, edge).await,
        0,
        "the clear read the edge already folded and appended no fold of its own"
    );
}

// ── WITNESS: no write door waits on a second pool connection while its transaction is open ─────

/// FAILS IF any write door in the table holds its write transaction open while acquiring a SECOND
/// connection from the same pool (hold-and-wait). The app here runs on a pool of exactly ONE
/// connection with a short acquire timeout: a door that resolves its profile or emitter (or runs
/// any other query) on the pool between `begin()` and `commit()` waits on the connection its own
/// transaction holds, times out, and answers `500`. A small pool stalls the same way under a
/// handful of concurrent writes. Every door here is driven by
/// the owner against a fresh live resource; any non-`500` answer (a state refusal on the
/// segmented doors included) proves the door completed on one connection. The bite: in
/// `DbBackend::resolve_actor_in_tx` (or any converted door), resolve on `&self.pool` instead of
/// the transaction — that door answers `500` here.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_write_door_completes_on_a_single_connection_pool(pool: PgPool) {
    let one = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(3))
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("a one-connection pool on the test database");
    let app = common::setup_test_app(one).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;

    let probe = ingest(&app, &owner, own_context).await;
    for door in doors(probe) {
        let resource = ingest(&app, &owner, own_context).await;
        let door = doors(resource)
            .into_iter()
            .find(|d| d.name == door.name)
            .expect("the same door, addressed at a fresh resource");
        let (status, body) = send(&app, &owner, &door).await;
        assert_ne!(
            status, 500,
            "{}: a write door must complete on a one-connection pool (hold-and-wait); body: {body}",
            door.name
        );
    }
}

/// FAILS IF any of the doors outside `doors()` holds its write transaction open while acquiring a
/// second pool connection — the same hold-and-wait the table witness pins, for the edge-mutate
/// doors (`begin_edge_mutation`, the keyed-facet validation), a goal-set `PATCH` (the goal lock,
/// the goal folds and the in-transaction assert), single-resource reassign, and blob relate. Each is
/// driven by the owner on a one-connection pool against live rows, so each must answer `2xx` — a
/// door refusing before its transaction opened would prove nothing. The bite: resolve the emitter on `&self.pool` inside
/// `begin_edge_mutation`'s transaction (or `reassign_resource`'s / `relate_blob`'s) — that door
/// answers `500` here.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_remaining_write_doors_complete_on_a_single_connection_pool(pool: PgPool) {
    let one = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(3))
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("a one-connection pool on the test database");
    let app = blob_app(one).await;
    let (owner, own_context) = caller(&app.pool, "owner").await;
    let (heir, _) = caller(&app.pool, "heir").await;

    let mut probes: Vec<(String, u16, Value)> = Vec::new();

    for door_name in edge_mutate_doors(Uuid::nil()).into_iter().map(|d| d.name) {
        let source = ingest(&app, &owner, own_context).await;
        let target = ingest(&app, &owner, own_context).await;
        let edge = assert_edge(&app, &owner, source, target).await;
        let mut door = edge_mutate_doors(edge)
            .into_iter()
            .find(|d| d.name == door_name)
            .expect("the same door, addressed at a fresh edge");
        if door.method == Method::DELETE {
            // The retract must address a live facet, or it answers 404 before its transaction.
            let (status, ack) = call(
                &app,
                &owner,
                Method::POST,
                format!("/api/relationships/{edge}/facets"),
                Some(json!({ "values": { "summary": "to retract" } })),
            )
            .await;
            assert_eq!(status, 200, "precondition: a facet to retract; body: {ack}");
            let property = ack["property_ids"][0].as_str().expect("a property id");
            door.path = format!("/api/relationships/{edge}/facets/{property}");
        }
        let (status, body) = send(&app, &owner, &door).await;
        probes.push((door.name.to_string(), status, body));
    }

    // A goal REPLACEMENT, so the patch serializes on the source, locks the current goal, folds its
    // edge and asserts the new one — every step of a goal patch, on one connection.
    let resource = ingest(&app, &owner, own_context).await;
    let first = ingest_typed(&app, &owner, own_context, "goal").await;
    let goal = ingest_typed(&app, &owner, own_context, "goal").await;
    let (status, body) = call(
        &app,
        &owner,
        Method::PATCH,
        format!("/api/resources/{resource}"),
        Some(json!({ "goal": first })),
    )
    .await;
    assert_eq!(status, 200, "precondition: a first goal; body: {body}");
    let (status, body) = call(
        &app,
        &owner,
        Method::PATCH,
        format!("/api/resources/{resource}"),
        Some(json!({ "title": "goal replaced on one connection", "goal": goal })),
    )
    .await;
    probes.push((
        "PATCH /api/resources/{id} (goal replaced)".to_string(),
        status,
        body,
    ));

    let resource = ingest(&app, &owner, own_context).await;
    let (status, body) = call(
        &app,
        &owner,
        Method::POST,
        format!("/api/resources/{resource}/reassign"),
        Some(json!({ "to_profile_id": heir.profile })),
    )
    .await;
    probes.push((
        "POST /api/resources/{id}/reassign".to_string(),
        status,
        body,
    ));

    let blob = commit_blob(&app, &owner, own_context).await;
    let peer = ingest(&app, &owner, own_context).await;
    let (status, body) = call(
        &app,
        &owner,
        Method::POST,
        format!("/api/blobs/{blob}/relations"),
        Some(json!({
            "direction": "blob_as_source",
            "peer_table": "kb_resources",
            "peer_id": peer,
            "edge_kind": "express",
            "polarity": "forward",
            "label": "one-connection",
            "weight": 1.0,
        })),
    )
    .await;
    probes.push(("POST /api/blobs/{id}/relations".to_string(), status, body));

    // Every probe is the owner acting on live rows, so each must SUCCEED: a refusal before the
    // door's transaction opens would answer non-500 without exercising the hold-and-wait at all.
    for (name, status, body) in probes {
        assert!(
            (200..300).contains(&status),
            "{name}: a write door must complete on a one-connection pool (hold-and-wait); \
             status {status}, body: {body}"
        );
    }
}
