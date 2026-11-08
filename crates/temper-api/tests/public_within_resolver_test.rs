#![cfg(feature = "test-db")]
//! `+team/slug` for a reader who is not a member of the team, and the `reach.via` a single-object
//! read reports.
//!
//! **The world:**
//!
//! ```text
//! org ─ eng ─ eng-web            the readers: people who are members of eng-web
//!           └ eng-sec  (T)       owns ctx-t; opened to eng's subtree by one row (T, eng)
//! ```
//!
//! A person in eng-web reads ctx-t only through public_within. A member of eng-sec reads it by
//! membership. A machine client and a connection agent, both members of eng-web and registered by
//! a reader, never read through public_within (the fence is the principal).
//!
//! Every refusal is compared as the HTTP layer renders it, status and body bytes, against the
//! refusal a stranger to the team received before this change.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_core::{context_ref::parse_context_ref, types::ids::ProfileId};
use temper_services::{error::ApiError, services::context_service};
use uuid::Uuid;

// ─── Fixture helpers ──────────────────────────────────────────────────────────

async fn team(pool: &PgPool, slug: &str, parents: &[Uuid]) -> Uuid {
    let id: Uuid =
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(slug)
            .fetch_one(pool)
            .await
            .expect("team");
    for p in parents {
        link(pool, *p, id).await;
    }
    id
}

async fn link(pool: &PgPool, parent: Uuid, child: Uuid) {
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await
        .expect("link");
}

async fn join(pool: &PgPool, team: Uuid, who: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
    )
    .bind(team)
    .bind(who)
    .bind(role)
    .execute(pool)
    .await
    .expect("join");
}

async fn context(pool: &PgPool, owner_team: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(owner_team)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("context")
}

async fn retire(pool: &PgPool, context_id: Uuid, retired_slug: &str) {
    sqlx::query("UPDATE kb_contexts SET is_active = false, slug = $2 WHERE id = $1")
        .bind(context_id)
        .bind(retired_slug)
        .execute(pool)
        .await
        .expect("retire");
}

/// Write a public_within row directly, as the read-arm tests do.
async fn open(pool: &PgPool, t: Uuid, a: Uuid, by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by, instance_wide, admin_act) \
         VALUES ($1, $2, $3, false, false)",
    )
    .bind(t)
    .bind(a)
    .bind(by)
    .execute(pool)
    .await
    .expect("open public_within row");
}

async fn ctx_reads(pool: &PgPool, who: Uuid, ctx: Uuid) -> bool {
    sqlx::query_scalar("SELECT context_visible_to($1, $2)")
        .bind(who)
        .bind(ctx)
        .fetch_one(pool)
        .await
        .expect("context reads")
}

/// A machine client registered by `registered_by`, with the machine provider's link.
async fn machine(pool: &PgPool, client_id: &str, registered_by: Uuid) -> Uuid {
    let id = common::fixtures::create_test_profile(pool, &format!("{client_id}@example.com")).await;
    sqlx::query("DELETE FROM kb_profile_auth_links WHERE profile_id = $1")
        .bind(id)
        .execute(pool)
        .await
        .expect("drop the person link");
    sqlx::query(
        "INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id) \
         VALUES (uuid_generate_v7(), $1, 'auth0-m2m', $2)",
    )
    .bind(id)
    .bind(format!("{client_id}@clients"))
    .execute(pool)
    .await
    .expect("machine link");
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, $1, $2, $3)",
    )
    .bind(client_id)
    .bind(id)
    .bind(registered_by)
    .execute(pool)
    .await
    .expect("machine client");
    id
}

/// A connection's agent profile: no auth link and no machine-client row.
async fn connection_agent(pool: &PgPool, slug: &str, registered_by: Uuid, home: Uuid) -> Uuid {
    let id = common::fixtures::create_test_profile(pool, &format!("{slug}@example.com")).await;
    sqlx::query("DELETE FROM kb_profile_auth_links WHERE profile_id = $1")
        .bind(id)
        .execute(pool)
        .await
        .expect("drop the person link");
    let entity: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_entities e WHERE e.profile_id = $1 AND e.name LIKE '%@web'",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("agent entity");
    sqlx::query(
        "INSERT INTO kb_connections \
           (provider, slug, name, registered_by_profile_id, profile_id, emitter_entity_id, \
            home_context_id) \
         VALUES ('github', $1, $1, $2, $3, $4, $5)",
    )
    .bind(slug)
    .bind(registered_by)
    .bind(id)
    .bind(entity)
    .bind(home)
    .execute(pool)
    .await
    .expect("connection");
    id
}

struct Person {
    id: Uuid,
    token: String,
}

async fn person(pool: &PgPool, name: &str) -> Person {
    let email = format!("{name}-{}@example.com", Uuid::new_v4().simple());
    let id = common::fixtures::create_test_profile(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{id}"), &email);
    Person { id, token }
}

struct World {
    org: Uuid,
    eng: Uuid,
    eng_web: Uuid,
    /// T.
    eng_sec: Uuid,
    ctx_t: Uuid,
    ctx_web: Uuid,
    /// Owns `ctx-t`'s team membership; opened the row.
    sec_member: Person,
    /// Reads ctx-t only through public_within.
    reader: Person,
    /// A member of a team with no relation to eng: reads nothing of T.
    stranger: Person,
}

async fn world(pool: &PgPool) -> World {
    let org = team(pool, "org", &[]).await;
    let eng = team(pool, "eng", &[org]).await;
    let eng_web = team(pool, "eng-web", &[eng]).await;
    let eng_sec = team(pool, "eng-sec", &[eng]).await;
    let elsewhere = team(pool, "elsewhere", &[]).await;

    let sec_member = person(pool, "sec-member").await;
    join(pool, eng_sec, sec_member.id, "owner").await;
    let reader = person(pool, "reader").await;
    join(pool, eng_web, reader.id, "member").await;
    let stranger = person(pool, "stranger").await;
    join(pool, elsewhere, stranger.id, "member").await;

    let ctx_t = context(pool, eng_sec, "ctx-t").await;
    let ctx_web = context(pool, eng_web, "ctx-web").await;

    open(pool, eng_sec, eng, sec_member.id).await;
    assert!(
        ctx_reads(pool, reader.id, ctx_t).await,
        "precondition: the reader reads ctx-t through public_within"
    );
    assert!(
        !ctx_reads(pool, stranger.id, ctx_t).await,
        "precondition: the stranger reads nothing of T"
    );

    World {
        org,
        eng,
        eng_web,
        eng_sec,
        ctx_t,
        ctx_web,
        sec_member,
        reader,
        stranger,
    }
}

async fn resolve(pool: &PgPool, who: Uuid, ref_str: &str) -> Result<Uuid, ApiError> {
    let r = parse_context_ref(ref_str).expect("valid ref");
    context_service::resolve_context_ref(pool, ProfileId::from(who), &r)
        .await
        .map(|id| *id)
}

async fn rendered(err: ApiError) -> (axum::http::StatusCode, bytes::Bytes) {
    use axum::response::IntoResponse as _;
    let response = err.into_response();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read refusal body");
    (status, body)
}

async fn refusal(
    pool: &PgPool,
    who: Uuid,
    ref_str: &str,
) -> (axum::http::StatusCode, bytes::Bytes) {
    match resolve(pool, who, ref_str).await {
        Ok(id) => panic!("{ref_str} must refuse; it resolved to {id}"),
        Err(e) => rendered(e).await,
    }
}

// ─── The resolver ─────────────────────────────────────────────────────────────

/// A person who reads T's context through public_within addresses it by `+T/slug`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_public_reader_resolves_the_teams_context_by_slug(pool: PgPool) {
    let w = world(&pool).await;
    assert_eq!(
        resolve(&pool, w.reader.id, "+eng-sec/ctx-t")
            .await
            .expect("resolves"),
        w.ctx_t
    );
    // A member is unchanged.
    assert_eq!(
        resolve(&pool, w.sec_member.id, "+eng-sec/ctx-t")
            .await
            .expect("member resolves"),
        w.ctx_t
    );
}

/// Every non-member refusal is the one `403` a stranger gets for a real slug, byte for byte: for
/// the public reader a missing slug, a retired slug and an unopened context (a connection's
/// intake, which public_within never opens) all read alike, and alike to the stranger's answer.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_non_member_refusal_is_the_strangers_forbidden(pool: PgPool) {
    let w = world(&pool).await;
    let retired = context(&pool, w.eng_sec, "ctx-old").await;
    retire(&pool, retired, "ctx-old-retired").await;
    let intake = context(&pool, w.eng_sec, "ctx-intake").await;
    let _agent = connection_agent(&pool, "intake-agent", w.sec_member.id, intake).await;
    assert!(
        !ctx_reads(&pool, w.reader.id, intake).await,
        "precondition: intake is unopened"
    );

    let baseline = refusal(&pool, w.stranger.id, "+eng-sec/ctx-t").await;
    assert_eq!(baseline.0, axum::http::StatusCode::FORBIDDEN);

    for (who, label, r) in [
        (w.stranger.id, "stranger, missing", "+eng-sec/no-such"),
        (
            w.stranger.id,
            "stranger, retired",
            "+eng-sec/ctx-old-retired",
        ),
        (w.reader.id, "reader, missing", "+eng-sec/no-such"),
        (w.reader.id, "reader, retired", "+eng-sec/ctx-old-retired"),
        (
            w.reader.id,
            "reader, unopened intake",
            "+eng-sec/ctx-intake",
        ),
    ] {
        assert_eq!(
            refusal(&pool, who, r).await,
            baseline,
            "{label}: must be the stranger's 403, byte for byte"
        );
    }
}

/// A member of a descendant team reads its parent's owned contexts by membership; it now
/// addresses them by `+parent/slug` too. This is the recorded behaviour change that needs no
/// public_within row at all.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_descendant_reader_resolves_the_parents_context(pool: PgPool) {
    let w = world(&pool).await;
    let ctx_eng = context(&pool, w.eng, "ctx-eng").await;
    // The reader is in eng-web, a child of eng: membership reach, no row involved.
    sqlx::query("DELETE FROM kb_team_public_within")
        .execute(&pool)
        .await
        .expect("no rows at all");
    assert_eq!(
        resolve(&pool, w.reader.id, "+eng/ctx-eng")
            .await
            .expect("a descendant member resolves its parent's context"),
        ctx_eng
    );
    assert_eq!(
        refusal(&pool, w.reader.id, "+eng/no-such").await,
        refusal(&pool, w.stranger.id, "+eng/ctx-eng").await,
        "the descendant's miss is the non-member 403"
    );
}

/// A machine principal of the very person who resolves `+T/slug` — a machine client and a
/// connection agent, each a member of the reader's own team — gets the non-member `403`: the
/// public_within arm is never in a machine's reading set.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_principal_of_the_reader_is_refused(pool: PgPool) {
    let w = world(&pool).await;
    let robot = machine(&pool, "reader-robot", w.reader.id).await;
    join(&pool, w.eng_web, robot, "member").await;
    let agent = connection_agent(&pool, "reader-agent", w.reader.id, w.ctx_web).await;
    join(&pool, w.eng_web, agent, "member").await;

    let baseline = refusal(&pool, w.stranger.id, "+eng-sec/ctx-t").await;
    assert_eq!(refusal(&pool, robot, "+eng-sec/ctx-t").await, baseline);
    assert_eq!(refusal(&pool, agent, "+eng-sec/ctx-t").await, baseline);
    // Their membership reach is untouched: the parent's context still resolves for them.
    let ctx_eng = context(&pool, w.eng, "ctx-eng").await;
    assert_eq!(
        resolve(&pool, robot, "+eng/ctx-eng").await.expect("robot"),
        ctx_eng
    );
}

/// A machine client authenticating over HTTP with its own token gets the same `403` from the
/// resolve endpoint.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_token_gets_the_forbidden_over_http(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let w = world(&app.pool).await;
    let robot = machine(&app.pool, "reader-robot-http", w.reader.id).await;
    join(&app.pool, w.eng_web, robot, "member").await;

    let resp = app
        .client
        .get(app.url("/api/contexts/resolve"))
        .query(&[("context_ref", "+eng-sec/ctx-t")])
        .header(
            "Authorization",
            format!(
                "Bearer {}",
                common::generate_machine_jwt("reader-robot-http")
            ),
        )
        .send()
        .await
        .expect("resolve");
    assert_eq!(
        resp.status().as_u16(),
        403,
        "{}",
        resp.text().await.unwrap_or_default()
    );

    // Control: the same token resolves through its membership reach, so the 403 above is the
    // resolver's answer and not an authentication or front-door refusal.
    let ctx_eng = context(&app.pool, w.eng, "ctx-eng").await;
    let ok = app
        .client
        .get(app.url("/api/contexts/resolve"))
        .query(&[("context_ref", "+eng/ctx-eng")])
        .header(
            "Authorization",
            format!(
                "Bearer {}",
                common::generate_machine_jwt("reader-robot-http")
            ),
        )
        .send()
        .await
        .expect("resolve control");
    assert_eq!(
        ok.status().as_u16(),
        200,
        "{}",
        ok.text().await.unwrap_or_default()
    );
    let ok: serde_json::Value = ok.json().await.expect("json");
    assert_eq!(ok["context_id"], json!(ctx_eng.to_string()));
}

/// A row that stops conferring flips the answer back to the non-member `403`: a forbid above T
/// (dormant), and the link that put T under its anchor going away (lapse).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_dormant_or_lapsed_row_flips_the_answer_back(pool: PgPool) {
    let w = world(&pool).await;
    let baseline = refusal(&pool, w.stranger.id, "+eng-sec/ctx-t").await;
    assert!(resolve(&pool, w.reader.id, "+eng-sec/ctx-t").await.is_ok());

    // Dormant: a forbid above T.
    sqlx::query("UPDATE kb_teams SET public_within_forbidden = true WHERE id = $1")
        .bind(w.org)
        .execute(&pool)
        .await
        .expect("forbid");
    assert!(
        !ctx_reads(&pool, w.reader.id, w.ctx_t).await,
        "precondition: dormant"
    );
    assert_eq!(
        refusal(&pool, w.reader.id, "+eng-sec/ctx-t").await,
        baseline
    );

    // Clear it: the row revives, and so does the address.
    sqlx::query("UPDATE kb_teams SET public_within_forbidden = false WHERE id = $1")
        .bind(w.org)
        .execute(&pool)
        .await
        .expect("clear forbid");
    assert!(resolve(&pool, w.reader.id, "+eng-sec/ctx-t").await.is_ok());

    // Lapse: T is no longer under its anchor.
    sqlx::query("DELETE FROM kb_teams_parents WHERE parent_id = $1 AND child_id = $2")
        .bind(w.eng)
        .bind(w.eng_sec)
        .execute(&pool)
        .await
        .expect("unlink");
    assert!(
        !ctx_reads(&pool, w.reader.id, w.ctx_t).await,
        "precondition: lapsed"
    );
    assert_eq!(
        refusal(&pool, w.reader.id, "+eng-sec/ctx-t").await,
        baseline
    );
}

// ─── Resolving grants no act ─────────────────────────────────────────────────

async fn resources_homed_in(pool: &PgPool, ctx: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_homes \
          WHERE anchor_table = 'kb_contexts' AND anchor_id = $1",
    )
    .bind(ctx)
    .fetch_one(pool)
    .await
    .expect("count")
}

/// A public reader who aims a create, an ingest or a move at `+T/slug` resolves the ref, then
/// meets the backend's authorship check: `403`, and nothing is written into T.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_public_reader_cannot_write_through_a_resolved_ref(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let w = world(&app.pool).await;
    let auth = format!("Bearer {}", w.reader.token);
    let before = resources_homed_in(&app.pool, w.ctx_t).await;

    // Resolve, then create by id: the path an MCP or CLI client takes with a ref.
    let resolved: serde_json::Value = app
        .client
        .get(app.url("/api/contexts/resolve"))
        .query(&[("context_ref", "+eng-sec/ctx-t")])
        .header("Authorization", &auth)
        .send()
        .await
        .expect("resolve")
        .json()
        .await
        .expect("resolve json");
    assert_eq!(resolved["context_id"], json!(w.ctx_t.to_string()));
    let create = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", &auth)
        .json(&json!({
            "kb_context_id": w.ctx_t.to_string(),
            "doc_type": "research",
            "origin_uri": "test://pw-create",
            "title": "should not land",
        }))
        .send()
        .await
        .expect("create");
    assert_eq!(
        create.status().as_u16(),
        403,
        "{}",
        create.text().await.unwrap_or_default()
    );

    // Ingest by ref.
    let ingest = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", &auth)
        .json(&json!({
            "title": "should not land either",
            "origin_uri": "test://pw-ingest",
            "context_ref": "+eng-sec/ctx-t",
            "doc_type_name": "research",
            "content": "body",
        }))
        .send()
        .await
        .expect("ingest");
    assert_eq!(
        ingest.status().as_u16(),
        403,
        "{}",
        ingest.text().await.unwrap_or_default()
    );

    // Move one of the reader's own resources into T by ref.
    let own = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", &auth)
        .json(&json!({
            "kb_context_id": w.ctx_web.to_string(),
            "doc_type": "research",
            "origin_uri": "test://pw-own",
            "title": "mine",
        }))
        .send()
        .await
        .expect("create own");
    assert!(
        own.status().is_success(),
        "{}",
        own.text().await.unwrap_or_default()
    );
    let own: serde_json::Value = own.json().await.expect("own json");
    let own_id = own["id"].as_str().expect("id").to_string();
    let mv = app
        .client
        .patch(app.url(&format!("/api/resources/{own_id}")))
        .header("Authorization", &auth)
        .json(&json!({ "context_to": "+eng-sec/ctx-t" }))
        .send()
        .await
        .expect("move");
    assert_eq!(
        mv.status().as_u16(),
        403,
        "{}",
        mv.text().await.unwrap_or_default()
    );

    assert_eq!(
        resources_homed_in(&app.pool, w.ctx_t).await,
        before,
        "nothing was written into T"
    );
}

// ─── The fence is the principal, not the token ───────────────────────────────

/// A mention-shaped request — the person's own bearer, relayed through the MCP server the way the
/// Slack act-as-the-human path relays it — reads T and resolves `+T/slug`. There is no delegated
/// marker: the person reads with any token that identifies them. This pins that ruling, so a
/// later fence on the token is a deliberate change that fails here.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_mention_shaped_request_from_a_reader_reads_and_resolves(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let w = world(&app.pool).await;
    let auth = format!("Bearer {}", w.reader.token);

    let resolved = app
        .client
        .get(app.url("/api/contexts/resolve"))
        .query(&[("context_ref", "+eng-sec/ctx-t")])
        .header("Authorization", &auth)
        .header("X-Temper-Surface", "mcp")
        .header("X-Temper-Relayed-Surface", "mcp")
        .send()
        .await
        .expect("resolve");
    assert_eq!(resolved.status().as_u16(), 200);
    let resolved: serde_json::Value = resolved.json().await.expect("json");
    assert_eq!(resolved["context_id"], json!(w.ctx_t.to_string()));

    let got = app
        .client
        .get(app.url(&format!("/api/contexts/{}", w.ctx_t)))
        .header("Authorization", &auth)
        .header("X-Temper-Surface", "mcp")
        .header("X-Temper-Relayed-Surface", "mcp")
        .send()
        .await
        .expect("get");
    assert_eq!(got.status().as_u16(), 200, "the mention path reads T");
}

// ─── reach.via on the single-object reads ────────────────────────────────────

async fn get_json(app: &common::TestApp, token: &str, path: &str) -> (u16, serde_json::Value) {
    let resp = app
        .client
        .get(app.url(path))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("get");
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(serde_json::Value::Null))
}

async fn create_in(app: &common::TestApp, token: &str, ctx: Uuid, title: &str) -> String {
    let resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": ctx.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://{title}"),
            "title": title,
        }))
        .send()
        .await
        .expect("create");
    assert!(
        resp.status().is_success(),
        "{}",
        resp.text().await.unwrap_or_default()
    );
    let body: serde_json::Value = resp.json().await.expect("json");
    body["id"].as_str().expect("id").to_string()
}

/// Context get and resource get say `public_within` to a reader who reads T only through
/// public_within, and `membership` to a member of T. The answer names no team and no person, and
/// it is the whole of `reach`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_get_says_how_the_reader_reaches_it(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let w = world(&app.pool).await;
    let r_t = create_in(&app, &w.sec_member.token, w.ctx_t, "r-t").await;

    let ctx_path = format!("/api/contexts/{}", w.ctx_t);
    let res_path = format!("/api/resources/{r_t}");
    for (who, token, via) in [
        ("public reader", &w.reader.token, "public_within"),
        ("member", &w.sec_member.token, "membership"),
    ] {
        for path in [&ctx_path, &res_path] {
            let (status, body) = get_json(&app, token, path).await;
            assert_eq!(status, 200, "{who} reads {path}");
            assert_eq!(
                body["reach"],
                json!({ "via": via }),
                "{who} on {path}: reach is exactly {{via: {via}}}"
            );
        }
    }
}

/// A reader who reads T's context both by membership and through public_within reaches it by
/// membership: membership wins whenever it holds.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn membership_wins_when_both_routes_hold(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let w = world(&app.pool).await;
    let r_t = create_in(&app, &w.sec_member.token, w.ctx_t, "r-t").await;
    let res_path = format!("/api/resources/{r_t}");

    // A direct read grant on one resource of T is a membership-shaped route to that resource.
    let (_, body) = get_json(&app, &w.reader.token, &res_path).await;
    assert_eq!(
        body["reach"],
        json!({ "via": "public_within" }),
        "precondition"
    );
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_resources', $1::uuid, 'kb_profiles', $2, true, $3)",
    )
    .bind(&r_t)
    .bind(w.reader.id)
    .bind(w.sec_member.id)
    .execute(&app.pool)
    .await
    .expect("grant the resource to the reader");
    let (_, body) = get_json(&app, &w.reader.token, &res_path).await;
    assert_eq!(
        body["reach"],
        json!({ "via": "membership" }),
        "a grant is membership"
    );

    // Joining T, at any role, makes the context membership too.
    join(&app.pool, w.eng_sec, w.reader.id, "watcher").await;
    let (_, body) = get_json(&app, &w.reader.token, &format!("/api/contexts/{}", w.ctx_t)).await;
    assert_eq!(body["reach"], json!({ "via": "membership" }));
}

/// With no public_within row anywhere, every get says `membership`: the owner of a personal
/// context and a descendant member reading its parent's context.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn without_a_row_every_reach_is_membership(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let w = world(&app.pool).await;
    sqlx::query("DELETE FROM kb_team_public_within")
        .execute(&app.pool)
        .await
        .expect("no rows");
    let ctx_eng = context(&app.pool, w.eng, "ctx-eng").await;
    let (status, body) = get_json(&app, &w.reader.token, &format!("/api/contexts/{ctx_eng}")).await;
    assert_eq!(status, 200);
    assert_eq!(body["reach"], json!({ "via": "membership" }));
    let own = create_in(&app, &w.reader.token, w.ctx_web, "own").await;
    let (_, body) = get_json(&app, &w.reader.token, &format!("/api/resources/{own}")).await;
    assert_eq!(body["reach"], json!({ "via": "membership" }));
}

/// Reach never answers for an object the caller cannot read: a stranger's get is the same 404 it
/// always was, with no `reach` in it; and the lists, which are out of scope, never carry it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reach_is_absent_where_it_is_not_answered(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let w = world(&app.pool).await;
    let r_t = create_in(&app, &w.sec_member.token, w.ctx_t, "r-t").await;

    // A stranger's get is the 404 it always was.
    for path in [
        format!("/api/contexts/{}", w.ctx_t),
        format!("/api/resources/{r_t}"),
    ] {
        let (status, _) = get_json(&app, &w.stranger.token, &path).await;
        assert_eq!(status, 404, "{path}");
    }

    // HTTP create of a resource reads back through the write readback, which never asks for reach.
    let created = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", w.sec_member.token))
        .json(&json!({
            "kb_context_id": w.ctx_t.to_string(),
            "doc_type": "research",
            "origin_uri": "test://pw-created",
            "title": "created",
        }))
        .send()
        .await
        .expect("create");
    assert!(created.status().is_success());
    let created: serde_json::Value = created.json().await.expect("json");
    assert!(created.get("id").is_some(), "{created}");
    assert!(
        created.get("reach").is_none(),
        "create carries no reach: {created}"
    );

    // Context create, and the admin-axis read of a retired context.
    let ctx = app
        .client
        .post(app.url("/api/contexts"))
        .header("Authorization", format!("Bearer {}", w.sec_member.token))
        .json(&json!({ "name": "ctx-new", "owner": { "Team": "eng-sec" } }))
        .send()
        .await
        .expect("create context");
    assert!(
        ctx.status().is_success(),
        "{}",
        ctx.text().await.unwrap_or_default()
    );
    let ctx: serde_json::Value = ctx.json().await.expect("json");
    assert!(
        ctx.get("reach").is_none(),
        "context create carries no reach: {ctx}"
    );
    let ctx_id = ctx["id"].as_str().expect("id").to_string();
    let retired = app
        .client
        .delete(app.url(&format!("/api/contexts/{ctx_id}")))
        .header("Authorization", format!("Bearer {}", w.sec_member.token))
        .send()
        .await
        .expect("retire");
    assert!(
        retired.status().is_success(),
        "{}",
        retired.text().await.unwrap_or_default()
    );
    let (status, admin_row) = get_json(
        &app,
        &w.sec_member.token,
        &format!("/api/contexts/{ctx_id}"),
    )
    .await;
    assert_eq!(
        status, 200,
        "the administrator reads the retired context on the admin axis"
    );
    assert_eq!(admin_row["retired"], json!(true), "{admin_row}");
    assert!(
        admin_row.get("reach").is_none(),
        "the admin axis carries no reach: {admin_row}"
    );
    let (_, list) = get_json(&app, &w.reader.token, "/api/contexts").await;
    let list = list.as_array().expect("context list");
    assert!(
        list.iter().any(|c| c["id"] == json!(w.ctx_t.to_string())),
        "reader lists ctx-t"
    );
    assert!(
        list.iter().all(|c| c.get("reach").is_none()),
        "the list carries no reach"
    );
    let (_, resources) = get_json(
        &app,
        &w.reader.token,
        "/api/resources?context_ref=%2Beng-sec%2Fctx-t",
    )
    .await;
    let rows = resources["rows"].as_array().expect("resource list rows");
    assert!(
        !rows.is_empty(),
        "the reader lists T's resource by +T/slug scope"
    );
    assert!(
        rows.iter().all(|r| r.get("reach").is_none()),
        "the list carries no reach"
    );
}

/// A machine principal reaches only by membership, so it never sees `public_within`, and it does
/// not read T at all.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_never_reaches_through_public_within(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let w = world(&app.pool).await;
    let robot = machine(&app.pool, "reach-robot", w.reader.id).await;
    join(&app.pool, w.eng_web, robot, "member").await;
    let token = common::generate_machine_jwt("reach-robot");
    let (status, _) = get_json(&app, &token, &format!("/api/contexts/{}", w.ctx_t)).await;
    assert_eq!(status, 404, "a machine does not read T");
    let (status, body) = get_json(&app, &token, &format!("/api/contexts/{}", w.ctx_web)).await;
    assert_eq!(status, 200);
    assert_eq!(body["reach"], json!({ "via": "membership" }));
}

/// A member of a team T shares a context to, and a holder of a read grant on one of T's contexts,
/// both resolve `+T/slug` with no membership in T. Both reach by membership, even though T has a
/// public_within row (so the reach function takes its membership-probe path, not its early return).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_share_or_grant_holder_resolves_and_reaches_by_membership(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let w = world(&app.pool).await;

    // A share of ctx-t to an unrelated team.
    let partner = team(&app.pool, "partner", &[]).await;
    let partner_member = person(&app.pool, "partner-member").await;
    join(&app.pool, partner, partner_member.id, "member").await;
    sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(w.ctx_t)
        .bind(partner)
        .execute(&app.pool)
        .await
        .expect("share ctx-t to partner");

    // A read grant on another of T's contexts, to a person with no team relation to T.
    let ctx_granted = context(&app.pool, w.eng_sec, "ctx-granted").await;
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
    )
    .bind(ctx_granted)
    .bind(w.stranger.id)
    .bind(w.sec_member.id)
    .execute(&app.pool)
    .await
    .expect("grant ctx-granted to the stranger");

    for (who, token, slug, ctx) in [
        (partner_member.id, &partner_member.token, "ctx-t", w.ctx_t),
        (w.stranger.id, &w.stranger.token, "ctx-granted", ctx_granted),
    ] {
        assert_eq!(
            resolve(&app.pool, who, &format!("+eng-sec/{slug}"))
                .await
                .expect("resolves"),
            ctx
        );
        let (status, body) = get_json(&app, token, &format!("/api/contexts/{ctx}")).await;
        assert_eq!(status, 200);
        assert_eq!(
            body["reach"],
            json!({ "via": "membership" }),
            "{slug}: {body}"
        );
    }
}

/// A machine that is a member of T reads T's context, which has a public_within row, and reaches
/// it by membership: the reach function's membership probe, for a machine.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_member_of_an_opened_team_reaches_by_membership(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let w = world(&app.pool).await;
    let robot = machine(&app.pool, "sec-robot", w.sec_member.id).await;
    join(&app.pool, w.eng_sec, robot, "member").await;
    let token = common::generate_machine_jwt("sec-robot");
    let (status, body) = get_json(&app, &token, &format!("/api/contexts/{}", w.ctx_t)).await;
    assert_eq!(status, 200);
    assert_eq!(body["reach"], json!({ "via": "membership" }));
}
