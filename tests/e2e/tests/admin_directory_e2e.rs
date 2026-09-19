//! The operator directory's CLI surface, at the production caller's level: a real server, a
//! real `temper` binary, real rendered stdout (spec §9 PR-2 row).
//!
//! PR-1 pinned the API contract; this file witnesses what the arc is FOR — the four audit
//! journeys (grounding doc `01a0b642-9e8c-7c10-8632-38573615c0bf`) through the door operators
//! actually type:
//!
//! - **J1/J3** — a SAML-provisioned principal who signed in but never requested is enumerable
//!   by the default `admin profiles list` (the findable-denied clause, stated as a query).
//! - **J2** — an email alone resolves to a state card (`admin profiles show --email`); no
//!   out-of-band UUID enters the chain.
//! - **J4** — the card → hint → act chain completes: the UUID that reaches `admin access
//!   approve` comes from the card's own rendered output, never from the fixture.
//! - the negative face — a non-admin at the same door is refused, and a pending invitation's
//!   redemption token never reaches rendered output even while the invitation itself does.
//!
//! Every assertion on "rendered output" reads the real binary's captured stdout — the bytes an
//! operator or agent transcript would hold — not an in-process value.

#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use uuid::Uuid;

/// The irreducible operator root step (CONFORM admin_surface_e2e): gating team + first admin,
/// so the app token's profile clears both the front door and the admin gate.
async fn bootstrap_admin(app: &common::E2eTestApp, admin_id: Uuid) {
    sqlx::query(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name",
    )
    .execute(&app.pool)
    .await
    .expect("team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(&app.pool)
        .await
        .expect("gating");
    common::approved_admin(&app.pool, admin_id).await;
}

/// GET /api/profile → this token's profile UUID (mints the profile on first hit), then approve.
/// CONFORM admin_ledger_e2e's provision.
async fn provision(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: Value = resp.json().await.expect("json");
    let pid: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    common::approve(&app.pool, pid).await;
    pid
}

/// Seed a human principal who has NEVER interacted: a profile with one verified auth link and
/// NO standing row at all (CONFORM admin_directory_service's own fixtures). The LEFT JOIN's
/// absent side — the emblematic never-requested SAML principal the audit found invisible.
async fn seed_never_requested(pool: &sqlx::PgPool, handle: &str, email: &str) -> Uuid {
    let profile_id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)")
        .bind(profile_id)
        .bind(handle)
        .execute(pool)
        .await
        .expect("seed human profile");
    sqlx::query(
        "INSERT INTO kb_profile_auth_links \
         (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, \
          is_default) \
         VALUES ($1, $2, 'saml:okta', $3, $4, true, true)",
    )
    .bind(Uuid::now_v7())
    .bind(profile_id)
    .bind(format!("saml:okta-{}", profile_id.simple()))
    .bind(email)
    .execute(pool)
    .await
    .expect("seed auth link");
    profile_id
}

/// Seed a pending team invitation addressed to `email`, with a KNOWN token — the bearer
/// capability the card must never render.
async fn seed_invitation(pool: &sqlx::PgPool, inviter: Uuid, email: &str, token: &str) {
    let team_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('acme-eng', 'Acme Engineering') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("create the inviting team");
    sqlx::query(
        "INSERT INTO kb_team_invitations
             (id, team_id, invited_email, invited_by_profile_id, role, token)
         VALUES (uuid_generate_v7(), $1, $2, $3, 'member', $4)",
    )
    .bind(team_id)
    .bind(email)
    .bind(inviter)
    .bind(token)
    .execute(pool)
    .await
    .expect("seed the invitation");
}

/// Run the real binary, assert success, hand back stdout.
async fn run_ok(app: &common::E2eTestApp, args: &[&str]) -> String {
    let output = common::run_temper_cli(app, args)
        .await
        .expect("spawn temper");
    assert!(
        output.status.success(),
        "`temper {}` must succeed\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Run the real binary as a principal OTHER than the app token (their own JWT), asserting
/// success. Same env contract as the shared spawn site, different token.
async fn run_ok_as(app: &common::E2eTestApp, token: &str, args: &[&str]) -> String {
    let config_toml = toml::to_string(&app.config).expect("serialize test TemperConfig to TOML");
    let config_path = app.vault_dir.path().join("actor-temper-config.toml");
    std::fs::write(&config_path, config_toml).expect("write actor config TOML");
    let output = common::run_temper_cli_with_token(&app.base_url(), token, &config_path, args)
        .await
        .expect("spawn temper");
    assert!(
        output.status.success(),
        "`temper {}` must succeed\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Parse the CLI's `--format json` stdout.
fn parse_json(stdout: &str) -> Value {
    serde_json::from_str(stdout).expect("stdout is valid JSON")
}

/// J1 + J3 — the never-requested principal is enumerable by the DEFAULT list: no flags at all,
/// the work queue. Both absence-as-denied (no standing row) and row-denied (first-login
/// birth state) render `denied`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn never_requested_principal_is_enumerable_by_default_list(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    bootstrap_admin(&app, admin_id).await;

    // The audit's invisible principal: signed in via SAML, never requested anything.
    // Seeded directly because the fixture wants NO standing row — the absence side.
    let _never = seed_never_requested(&app.pool, "never-pat", "never.pat@example.com").await;
    // The first-login twin: JIT-provisioned (born Denied, standing row present), still denied.
    // Found by the verified email its auth link carries — not by handle, whose scheme is the
    // display name generator's business, not this assertion's.
    let first_login_token = common::generate_second_user_jwt();
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {first_login_token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let stdout = run_ok(&app, &["admin", "profiles", "--format", "json", "list"]).await;
    let page = parse_json(&stdout);

    let handles: Vec<&str> = page["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .map(|e| e["handle"].as_str().expect("handle"))
        .collect();
    assert!(
        handles.contains(&"never-pat"),
        "the never-requested principal must appear under the default view\n{stdout}"
    );

    let entry = page["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["handle"] == "never-pat")
        .expect("never-pat entry");
    assert_eq!(entry["standing"], "denied", "absence renders as denied");
    assert!(
        entry["standing_updated"].is_null(),
        "a profile with no standing row renders standing_updated null, not absent/hidden"
    );

    let first_login_entry = page["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["email"] == "second@test.example.com")
        .expect("first-login principal entry");
    assert_eq!(first_login_entry["standing"], "denied");

    assert!(
        page["total"].as_i64().expect("total") >= 2,
        "the response carries total, not just the page\n{stdout}"
    );
}

/// J2 — an email alone resolves to the state card, server-side. The test holds no UUID before
/// the card renders; the resolution is the CLI's one act, not client-side narrowing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_email_alone_resolves_the_state_card(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    bootstrap_admin(&app, admin_id).await;

    let seeded = seed_never_requested(&app.pool, "email-pat", "email.pat@example.com").await;

    let stdout = run_ok(
        &app,
        &[
            "admin",
            "profiles",
            "--format",
            "json",
            "show",
            "--email",
            "email.pat@example.com",
        ],
    )
    .await;
    let card = parse_json(&stdout);

    assert_eq!(
        card["profile_id"].as_str().expect("profile_id"),
        seeded.to_string(),
        "the card is the seeded principal's — resolved from the email alone\n{stdout}"
    );
    assert_eq!(card["standing"], "denied");
    assert!(
        !card["hints"].as_array().expect("hints").is_empty(),
        "a denied principal's card carries enablement hints\n{stdout}"
    );
}

/// J4, arm (a) — the emblematic never-requested journey: a principal who signed in (birthing
/// `Denied`) but never requested anything is found by email and approved straight from the
/// card's hint. The UUID that reaches `admin access approve` is read out of the card's
/// RENDERED OUTPUT — the test never touches the fixture UUID, so the witness breaks if any
/// out-of-band UUID were required.
///
/// (Direct approve is machine-legal from Denied — `temper-principal/src/transition.rs` — but
/// NOT from standing-row ABSENCE; a never-signed-in principal must authenticate once before
/// any admin act can reach them. The directory's no-row fixture is witnessed by the
/// enumeration test above, which is the surface that absent class needs.)
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn email_to_card_to_approve_completes_without_an_out_of_band_uuid(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    bootstrap_admin(&app, admin_id).await;

    // The never-requested principal's whole existence, from their side: one sign-in.
    let subject_token = common::generate_second_user_jwt();
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {subject_token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // The operator's side: email in, card out — the ONLY input the act gets.
    let card_stdout = run_ok(
        &app,
        &[
            "admin",
            "profiles",
            "--format",
            "json",
            "show",
            "--email",
            "second@test.example.com",
        ],
    )
    .await;
    let card = parse_json(&card_stdout);
    assert_eq!(card["standing"], "denied");

    // The hint is copy-runnable: shlex it exactly as an operator's shell would and hand it to
    // the real binary (CONFORM access_gate_test's advertised-command check).
    let approve_hint = card["hints"]
        .as_array()
        .expect("hints")
        .iter()
        .map(|h| h.as_str().expect("hint string"))
        .find(|h| h.starts_with("temper admin access approve "))
        .expect("a denied principal's card advertises the approve act")
        .to_string();
    let argv = shlex::split(&approve_hint).expect("hint is shell-splittable");
    let cli_argv: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
    let approve_stdout = run_ok(&app, &cli_argv).await;
    assert!(
        approve_stdout.contains("approved"),
        "the hint's own invocation must complete the act\n{approve_stdout}"
    );

    // The work queue drains: the now-approved principal is GONE from the default view but
    // present under --standing all as approved.
    let default_stdout = run_ok(&app, &["admin", "profiles", "--format", "json", "list"]).await;
    let default_page = parse_json(&default_stdout);
    let default_emails: Vec<&str> = default_page["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter_map(|e| e["email"].as_str())
        .collect();
    assert!(
        !default_emails.contains(&"second@test.example.com"),
        "an approved principal must leave the needs-access default\n{default_stdout}"
    );

    let all_stdout = run_ok(
        &app,
        &[
            "admin",
            "profiles",
            "--format",
            "json",
            "list",
            "--standing",
            "all",
        ],
    )
    .await;
    let all_entry = parse_json(&all_stdout)["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|e| e["email"] == "second@test.example.com")
        .expect("approved principal still exists under --standing all")
        .clone();
    assert_eq!(all_entry["standing"], "approved");
}

/// J4, arm (b) — the queue-state bridge: a principal who FILED a join request is approved
/// through `admin requests review <id> --approve`, the command the card's queue-state hints.
/// Again every act input comes from rendered output.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn email_to_card_to_review_completes_the_queue_bridge(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    bootstrap_admin(&app, admin_id).await;

    // The principal's side, all real: sign in (born Denied), then request access.
    let subject_token = common::generate_third_user_jwt();
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {subject_token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    run_ok_as(
        &app,
        &subject_token,
        &["auth", "request-access", "--message", "let me in"],
    )
    .await;

    // The operator sees the request state on the card before acting — non-vacuous.
    let card_stdout = run_ok(
        &app,
        &[
            "admin",
            "profiles",
            "--format",
            "json",
            "show",
            "--email",
            "third@test.example.com",
        ],
    )
    .await;
    let card = parse_json(&card_stdout);
    assert_eq!(card["standing"], "requested");
    assert!(
        card["open_join_request"]["id"].is_string(),
        "the open join request rides the card\n{card_stdout}"
    );

    let review_hint = card["hints"]
        .as_array()
        .expect("hints")
        .iter()
        .map(|h| h.as_str().expect("hint string"))
        .find(|h| h.contains("admin requests review"))
        .expect("an open join request advertises its review command")
        .to_string();
    let argv = shlex::split(&review_hint).expect("hint is shell-splittable");
    let cli_argv: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
    run_ok(&app, &cli_argv).await;

    let all_stdout = run_ok(
        &app,
        &[
            "admin",
            "profiles",
            "--format",
            "json",
            "list",
            "--standing",
            "all",
        ],
    )
    .await;
    let reviewed = parse_json(&all_stdout)["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|e| e["email"] == "third@test.example.com")
        .expect("reviewed principal under --standing all")
        .clone();
    assert_eq!(reviewed["standing"], "approved");
}

/// The token-absence clause: a pending invitation's redemption token is a bearer capability
/// that ACCEPTS the invitation. The card renders the invitation; it must never render the
/// token — asserted on the real binary's rendered stdout, in BOTH output formats.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn rendered_output_never_carries_the_invitation_token(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    bootstrap_admin(&app, admin_id).await;

    let _seeded = seed_never_requested(&app.pool, "token-pat", "token.pat@example.com").await;
    let secret_token = format!("tok-{}", Uuid::now_v7());
    seed_invitation(&app.pool, admin_id, "token.pat@example.com", &secret_token).await;

    // Non-vacuous first: the invitation itself IS on the card.
    let json_stdout = run_ok(
        &app,
        &[
            "admin",
            "profiles",
            "--format",
            "json",
            "show",
            "--email",
            "token.pat@example.com",
        ],
    )
    .await;
    let card = parse_json(&json_stdout);
    let invitations = card["pending_invitations"]
        .as_array()
        .expect("pending invitations");
    assert_eq!(
        invitations.len(),
        1,
        "the invitation must be present for the absence assertion to mean anything\n{json_stdout}"
    );
    assert_eq!(invitations[0]["team_slug"], "acme-eng");

    // The clause, in json: neither the token value nor any `token` key reaches stdout.
    assert!(
        !json_stdout.contains(&secret_token),
        "the invitation token must not appear in rendered json output\n{json_stdout}"
    );
    assert!(
        !json_stdout.contains("\"token\""),
        "no `token` key may appear in rendered json output, in any object\n{json_stdout}"
    );

    // The clause, in the default format — the format a human terminal shows.
    let plain_stdout = run_ok(
        &app,
        &[
            "admin",
            "profiles",
            "show",
            "--email",
            "token.pat@example.com",
        ],
    )
    .await;
    assert!(
        !plain_stdout.contains(&secret_token),
        "the invitation token must not appear in default-format output\n{plain_stdout}"
    );

    // And the list rows: the enumeration surface stays token-free too.
    let list_stdout = run_ok(&app, &["admin", "profiles", "--format", "json", "list"]).await;
    assert!(
        !list_stdout.contains(&secret_token),
        "the invitation token must not appear in list output\n{list_stdout}"
    );
}

/// The hints clause: every hint on a card must parse at the real CLI's own parser tree — a
/// hint naming a command that does not exist is a wrong-principal bridge waiting to be typed
/// (spec §6; PR-2 review focus).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_card_hint_parses_at_the_real_cli(pool: sqlx::PgPool) {
    use clap::Parser;

    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    bootstrap_admin(&app, admin_id).await;

    // A principal with the richest legal hint surface: signed in (Denied), then filed a
    // request (Requested + open join request) — both hints fire together.
    let subject_token = common::generate_second_user_jwt();
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {subject_token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    run_ok_as(
        &app,
        &subject_token,
        &["auth", "request-access", "--message", "let me in"],
    )
    .await;

    let card = parse_json(
        &run_ok(
            &app,
            &[
                "admin",
                "profiles",
                "--format",
                "json",
                "show",
                "--email",
                "second@test.example.com",
            ],
        )
        .await,
    );

    let hints: Vec<String> = card["hints"]
        .as_array()
        .expect("hints")
        .iter()
        .map(|h| h.as_str().expect("hint string").to_string())
        .collect();
    assert!(
        !hints.is_empty(),
        "a denied principal with an open request carries at least two hints"
    );
    assert!(
        hints.iter().any(|h| h.contains("admin requests review")),
        "an open join request advertises its review command"
    );

    for hint in &hints {
        let argv =
            shlex::split(hint).unwrap_or_else(|| panic!("hint `{hint}` is not shell-splittable"));
        if let Err(err) = temper_cli::cli::Cli::try_parse_from(&argv) {
            panic!("hint `{hint}` does not parse at the real CLI: {err}");
        }
    }
}

/// The goal's negative face, through the NEW command's own door: a non-admin running
/// `admin profiles list` is refused — enumeration is not reachable below the gate. PR-1
/// pinned the API; this witnesses the CLI transport of the same refusal.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_non_admin_is_refused_at_the_cli_door(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let _admin_id = provision(&app, &app.token).await;

    let stranger_token = common::generate_second_user_jwt();
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {stranger_token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let output = common::run_temper_cli(&app, &["admin", "profiles", "list"])
        .await
        .expect("spawn temper");
    assert!(
        !output.status.success(),
        "a non-admin must be refused at the CLI door\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
