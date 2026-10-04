#![cfg(feature = "test-db")]
//! `GET /api/contexts/resolve` — a context ref resolved to its id, for the caller.
//!
//! The route exists so the MCP `context_anchor` resolver can relay instead of reading the
//! database (the route-first half of the network door's teardown). It is
//! `context_service::resolve_context_ref` behind a route, so these pins hold it to that
//! resolver's contract, per ref form:
//!
//! - every form resolves for a caller who can read the context (`@me/`, `@<handle>/`,
//!   `+<team>/` for a member who is not the owner, and a bare UUID);
//! - **no existence oracle**: a context the caller cannot read answers byte-identically to one
//!   that does not exist, on the UUID arm and the `@<handle>` arm (an unknown handle included);
//! - a malformed ref answers 400 with the shared parser's sentence;
//! - the `+<team>` arm keeps the resolver's existing non-member `Forbidden` — pinned here as
//!   the resolver's behaviour, not introduced by this route (the graph and ingest routes give
//!   the same answer for the same ref).

mod common;

use reqwest::StatusCode;
use temper_core::context_ref::{parse_context_ref, ContextOwnerRef};
use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};
use uuid::Uuid;

/// Resolve `context_ref` as the holder of `token`, over the raw wire. Returns the status and
/// the body text, so refusals can be compared byte for byte.
async fn resolve_as(
    app: &common::E2eTestApp,
    token: &str,
    context_ref: &str,
) -> (StatusCode, String) {
    let resp = app
        .reqwest_client
        .get(app.url("/api/contexts/resolve"))
        .query(&[("context_ref", context_ref)])
        .bearer_auth(token)
        .send()
        .await
        .expect("resolve request");
    let status = resp.status();
    (status, resp.text().await.expect("resolve body"))
}

/// Every ref form resolves, through the typed client, for a caller who can read the context.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_ref_form_resolves_for_a_reader(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("owner profile");

    let ctx = app
        .client
        .contexts()
        .create("resolve route home", None)
        .await
        .expect("create context");
    // `owner_ref` is the already-sigil'd `@<handle>`.
    assert!(
        ctx.owner_ref.starts_with('@'),
        "a profile-owned context: {}",
        ctx.owner_ref
    );

    for context_ref in [
        format!("@me/{}", ctx.slug),
        format!("{}/{}", ctx.owner_ref, ctx.slug),
        ctx.id.to_string(),
    ] {
        let resolved = app
            .client
            .contexts()
            .resolve(&context_ref)
            .await
            .unwrap_or_else(|e| panic!("{context_ref} should resolve for its owner: {e}"));
        assert_eq!(
            resolved.context_id, ctx.id,
            "{context_ref} resolves to the context"
        );
    }
}

/// The `+<team>` arm resolves for a member who is not the context's creator, and refuses a
/// non-member with the resolver's existing `Forbidden`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_team_ref_resolves_for_a_member_and_refuses_a_non_member(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("owner profile");
    let member_id = common::provision_and_approve_second(&app).await;
    let member_token = common::generate_second_user_jwt();

    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "resolve-route-team".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("create team");
    let ctx = app
        .client
        .contexts()
        .create(
            "resolve route team home",
            Some(ContextOwnerRef::Team("resolve-route-team".to_owned())),
        )
        .await
        .expect("create team context");
    let team_ref = format!("+resolve-route-team/{}", ctx.slug);

    // Not yet a member: the resolver's membership gate answers 403.
    let (status, _) = resolve_as(&app, &member_token, &team_ref).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a non-member is refused at the membership gate"
    );
    // The 403 is the existing-team arm only: a team that does not exist answers 404.
    let (status, _) = resolve_as(&app, &member_token, "+no-such-team/no-such-context").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an absent team is not found, not forbidden"
    );

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
        .expect("add member");

    let (status, body) = resolve_as(&app, &member_token, &team_ref).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a member resolves the team's context: {body}"
    );
    let resolved: temper_core::types::context::ContextResolution =
        serde_json::from_str(&body).expect("resolution body");
    assert_eq!(resolved.context_id, ctx.id);

    // A member's miss names only the slug they supplied, in their own team's namespace.
    let (status, _) = resolve_as(&app, &member_token, "+resolve-route-team/no-such-context").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// No existence oracle: a context the caller cannot read answers exactly as one that does not
/// exist — on the UUID arm, and on the `@<handle>` arm (an unknown handle included).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unreadable_answers_exactly_as_absent(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("owner profile");
    common::provision_and_approve_second(&app).await;
    let stranger = common::generate_second_user_jwt();

    let ctx = app
        .client
        .contexts()
        .create("resolve route private", None)
        .await
        .expect("create context");
    let owner_ref = &ctx.owner_ref; // `@<handle>`

    // UUID arm: the owner's private context vs an id that names nothing.
    let unreadable = resolve_as(&app, &stranger, &ctx.id.to_string()).await;
    let absent = resolve_as(&app, &stranger, &Uuid::now_v7().to_string()).await;
    assert_eq!(unreadable.0, StatusCode::NOT_FOUND);
    assert_eq!(
        unreadable, absent,
        "UUID arm: unreadable and absent are indistinguishable"
    );

    // `@<handle>` arm: the owner's real slug, a slug the owner does not have, and a handle no
    // one holds — all three one answer.
    let unreadable = resolve_as(&app, &stranger, &format!("{owner_ref}/{}", ctx.slug)).await;
    let absent_slug = resolve_as(&app, &stranger, &format!("{owner_ref}/no-such-context")).await;
    let absent_handle = resolve_as(&app, &stranger, "@no-such-handle/no-such-context").await;
    assert_eq!(unreadable.0, StatusCode::NOT_FOUND);
    assert_eq!(
        unreadable, absent_slug,
        "@handle arm: unreadable equals an absent slug"
    );
    assert_eq!(
        unreadable, absent_handle,
        "@handle arm: unreadable equals an unknown handle"
    );

    // The owner, by contrast, resolves the same ref.
    let (status, _) = resolve_as(&app, &app.token, &format!("{owner_ref}/{}", ctx.slug)).await;
    assert_eq!(status, StatusCode::OK);
}

/// A malformed ref answers 400 with the shared parser's own sentence — one grammar, no
/// server-side dialect.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_malformed_ref_answers_400_with_the_parsers_sentence(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("owner profile");

    for malformed in ["temper", "@me", "+team", "@UPPER/temper"] {
        let sentence = parse_context_ref(malformed)
            .expect_err("the shared parser refuses it")
            .to_string();
        let (status, body) = resolve_as(&app, &app.token, malformed).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{malformed}: {body}");
        let body: serde_json::Value = serde_json::from_str(&body).expect("error body is JSON");
        assert_eq!(
            body["error"]["message"],
            format!("Bad request: {sentence}"),
            "{malformed}: the message is the parser's sentence, under the 400 label"
        );
    }
}
