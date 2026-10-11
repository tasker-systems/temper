#![cfg(feature = "test-db")]
mod common;

use serde_json::Value;
use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};
use uuid::Uuid;

/// GET /api/profile → this token's profile UUID (mints the profile on first hit).
async fn provision(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight");
    let body: Value = resp.json().await.expect("json");
    // D11: a fresh principal is born Denied. Approve so this actor clears the front door
    // and the ENDPOINT authz (ownership, admin-only, grants) is what the test exercises.
    let __pid: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    common::approve(&app.pool, __pid).await;
    __pid
}

/// POST /api/ingest → the new resource's UUID. Homes the resource in `context_id`.
async fn ingest_into_context(
    app: &common::E2eTestApp,
    token: &str,
    context_id: Uuid,
    title: &str,
    slug: &str,
) -> Uuid {
    let resp = app
        .reqwest_client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "title": title,
            "origin_uri": format!("test://resource-grant-e2e/{}", Uuid::new_v4()),
            "context_ref": context_id.to_string(),
            "doc_type_name": "research",
            "slug": slug,
            "content": "A resource owned by the granter, shared to a team by capability grant.",
        }))
        .send()
        .await
        .expect("ingest request failed");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::OK,
        "owner ingests into own context"
    );
    let body: Value = resp.json().await.expect("ingest json");
    body["id"]
        .as_str()
        .expect("resource id")
        .parse()
        .expect("uuid")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn owner_can_administer_grant_seam(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let owner_id = provision(&app, &app.token).await;
    let stranger_token = common::generate_second_user_jwt();
    let stranger_id = provision(&app, &stranger_token).await;

    let context = app
        .client
        .contexts()
        .create("seam-ctx", None)
        .await
        .expect("ctx");
    let resource_id =
        ingest_into_context(&app, &app.token, *context.id, "seam doc", "seam-doc").await;

    // The owner-grant seam: the resource's owner CAN administer grants; a stranger cannot.
    let owner_can: Option<bool> =
        sqlx::query_scalar("SELECT can('kb_profiles', $1, 'grant', 'kb_resources', $2)")
            .bind(owner_id)
            .bind(resource_id)
            .fetch_one(&pool)
            .await
            .expect("can() query");
    assert_eq!(
        owner_can,
        Some(true),
        "resource owner may administer grants (the new seam)"
    );

    let stranger_can: Option<bool> =
        sqlx::query_scalar("SELECT can('kb_profiles', $1, 'grant', 'kb_resources', $2)")
            .bind(stranger_id)
            .bind(resource_id)
            .fetch_one(&pool)
            .await
            .expect("can() query");
    assert_eq!(
        stranger_can,
        Some(false),
        "a non-owner, non-admin cannot administer grants"
    );
}

/// GET /api/resources/{id} status as `token` — the full-stack `resources_visible_to` oracle.
async fn show_status(app: &common::E2eTestApp, token: &str, resource: Uuid) -> reqwest::StatusCode {
    app.reqwest_client
        .get(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("show request failed")
        .status()
}

/// Direct SQL oracle for the write axis.
async fn can_modify(pool: &sqlx::PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT can_modify_resource($1, $2)")
        .bind(profile)
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("can_modify_resource")
}

/// Full-stack acceptance: a NON-admin resource owner grants a team read/write via the real
/// CLI, a team member gains visibility + modify, and revoke reverses it. Team creation is
/// open (no admin bootstrap), so the owner is never a system admin — this proves the seam.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn owner_grants_resource_to_team_via_cli_and_revokes(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Owner drives app.client/app.token (NON-admin). Member is a second profile.
    let _owner_id = provision(&app, &app.token).await;
    let member_token = common::generate_second_user_jwt();
    let member_id = provision(&app, &member_token).await;

    // Owner creates a team (becomes its owner — team creation is open) and adds the member.
    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "grant-team".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("owner creates team");
    app.client
        .teams()
        .add_member(
            team.id,
            &AddMemberRequest {
                profile_id: member_id,
                role: TeamRole::Member,
            },
        )
        .await
        .expect("owner adds member");

    // Owner homes a resource in their own context.
    let context = app
        .client
        .contexts()
        .create("grant-ctx", None)
        .await
        .expect("ctx");
    let resource_id =
        ingest_into_context(&app, &app.token, *context.id, "grant doc", "grant-doc").await;
    let resource_str = resource_id.to_string();
    let team_str = team.id.to_string();

    // Pre-grant: member cannot see the resource.
    assert_eq!(
        show_status(&app, &member_token, resource_id).await,
        reqwest::StatusCode::NOT_FOUND,
        "member cannot see the resource before any grant"
    );

    // Owner grants READ to the team via the real CLI.
    let out = common::run_temper_cli(
        &app,
        &[
            "resource",
            "grant",
            &resource_str,
            "--to-team",
            &team_str,
            "--read",
        ],
    )
    .await
    .expect("spawn temper cli");
    assert!(
        out.status.success(),
        "grant --read failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // Member now sees it; but cannot modify (read-only grant).
    assert_eq!(
        show_status(&app, &member_token, resource_id).await,
        reqwest::StatusCode::OK,
        "member sees the resource after a read grant to their team"
    );
    assert!(
        !can_modify(&pool, member_id, resource_id).await,
        "read grant does not confer modify"
    );

    // Owner upgrades to WRITE via the CLI → member can modify.
    let out = common::run_temper_cli(
        &app,
        &[
            "resource",
            "grant",
            &resource_str,
            "--to-team",
            &team_str,
            "--write",
        ],
    )
    .await
    .expect("spawn temper cli");
    assert!(
        out.status.success(),
        "grant --write failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        can_modify(&pool, member_id, resource_id).await,
        "write grant confers modify"
    );

    // Owner revokes via the CLI → visibility + modify gone.
    let out = common::run_temper_cli(
        &app,
        &[
            "resource",
            "revoke",
            &resource_str,
            "--from-team",
            &team_str,
        ],
    )
    .await
    .expect("spawn temper cli");
    assert!(
        out.status.success(),
        "revoke failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        show_status(&app, &member_token, resource_id).await,
        reqwest::StatusCode::NOT_FOUND,
        "member loses visibility after revoke"
    );
    assert!(
        !can_modify(&pool, member_id, resource_id).await,
        "revoke removes modify"
    );

    // Decorated-ref strip: grant using `some-slug-<uuid>` for --to-team still resolves.
    let decorated_team = format!("any-slug-here-{team_str}");
    let out = common::run_temper_cli(
        &app,
        &[
            "resource",
            "grant",
            &resource_str,
            "--to-team",
            &decorated_team,
            "--read",
        ],
    )
    .await
    .expect("spawn temper cli");
    assert!(
        out.status.success(),
        "decorated --to-team ref failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        show_status(&app, &member_token, resource_id).await,
        reqwest::StatusCode::OK,
        "decorated team ref (slug-<uuid>) resolves via parse_ref"
    );
}

/// POST or DELETE `/api/resources/{resource}/grants` as `token` for `principal`: status and body.
async fn grant_door(
    app: &common::E2eTestApp,
    token: &str,
    method: reqwest::Method,
    resource: Uuid,
    principal: Uuid,
) -> (reqwest::StatusCode, Value) {
    let body = if method == reqwest::Method::DELETE {
        serde_json::json!({ "principal_table": "kb_profiles", "principal_id": principal })
    } else {
        serde_json::json!({
            "principal_table": "kb_profiles",
            "principal_id": principal,
            "can_read": true,
            "can_write": false,
            "can_delete": false,
            "can_grant": false,
        })
    };
    let resp = app
        .reqwest_client
        .request(
            method,
            app.url(&format!("/api/resources/{resource}/grants")),
        )
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("grant door request");
    let status = resp.status();
    (status, resp.json().await.unwrap_or(Value::Null))
}

/// The grant door's gate and outcomes, for people: an approved person who neither owns the
/// resource nor holds `can_grant` on it is refused both verbs (`403`, the plain `FORBIDDEN`), and
/// the owner grants (`granted: true` on a fresh grant) and revokes (`revoked: true`; a second
/// revoke of the now-absent grant is the no-op `revoked: false`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_grant_door_admits_the_owner_and_refuses_a_stranger(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    provision(&app, &app.token).await;
    let stranger_token = common::generate_second_user_jwt();
    let stranger_id = provision(&app, &stranger_token).await;

    let context = app
        .client
        .contexts()
        .create("grant-door-ctx", None)
        .await
        .expect("ctx");
    let resource_id =
        ingest_into_context(&app, &app.token, *context.id, "door doc", "door-doc").await;

    for method in [reqwest::Method::POST, reqwest::Method::DELETE] {
        let (status, body) = grant_door(
            &app,
            &stranger_token,
            method.clone(),
            resource_id,
            stranger_id,
        )
        .await;
        assert_eq!(
            status,
            reqwest::StatusCode::FORBIDDEN,
            "{method}: a non-owner without can_grant is refused; body: {body}"
        );
        assert_eq!(body["error"]["code"], "FORBIDDEN", "{method}: {body}");
    }

    let (status, body) = grant_door(
        &app,
        &app.token,
        reqwest::Method::POST,
        resource_id,
        stranger_id,
    )
    .await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "the owner grants; body: {body}"
    );
    assert_eq!(body["granted"], true, "{body}");

    let (status, body) = grant_door(
        &app,
        &app.token,
        reqwest::Method::DELETE,
        resource_id,
        stranger_id,
    )
    .await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "the owner revokes; body: {body}"
    );
    assert_eq!(body["revoked"], true, "{body}");

    let (status, body) = grant_door(
        &app,
        &app.token,
        reqwest::Method::DELETE,
        resource_id,
        stranger_id,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(
        body["revoked"], false,
        "revoke of an absent grant is a no-op; {body}"
    );
}
