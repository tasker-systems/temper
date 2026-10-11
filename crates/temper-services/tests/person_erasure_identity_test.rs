#![cfg(feature = "test-db")]
//! Witnesses for the person act's reach into operational identity data (20261023100000; rulings R2,
//! R3 and R4 of the 2026-10-10 erasure inventory, and the rulings recorded on task
//! 01a125a1-7b5b-75ee-aafb-3046b16d532b).
//!
//! Each world is built through the real committers where one exists (`principal_standing_apply`,
//! `principal_governance_set`, the Slack link, vault and disconnect services) and by row otherwise,
//! then erased through `erasure_service::execute_erasure`. Before this migration the act removed the
//! profile's and the auth links' identifiers and left the like kinds: invitation emails, login
//! claims and refresh tokens, connection credentials, entity metadata, staged upload bytes, Slack
//! stores held with no current link, the Slack id on the ledger, and free text about the subject.
//! Every witness below fails against that act.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_services::auth_config::AuthMode;
use temper_services::services::erasure_service::{execute_erasure, ErasureCompletion};
use temper_services::services::grant_crypto::VaultKey;
use temper_services::services::slack_disconnect_service::{
    disconnect_slack_principal, DisconnectRequest,
};
use temper_services::services::{slack_grant_vault_service, slack_link_service};
use temper_workflow::operations::Surface;

/// Text about the subject that must end nowhere unless a policy keeps it.
const ABOUT: &str = "Jane Roe harassment complaint";

struct Person {
    profile: Uuid,
    email: String,
    emitter: Uuid,
    /// An `@me` context: in the estate.
    me: Uuid,
}

async fn person(pool: &PgPool) -> Person {
    let profile = Uuid::now_v7();
    let handle = format!("user-{profile}");
    let email = format!("{handle}@x.test");
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
                 VALUES ($1, $2, $2, $3, '{}'::jsonb)",
    )
    .bind(profile)
    .bind(&handle)
    .bind(&email)
    .execute(pool)
    .await
    .expect("seed profile");
    let emitter: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name, metadata) \
              VALUES ($1, $2, '{\"persona\": \"Jane Roe\", \"model\": \"m\"}'::jsonb) RETURNING id",
    )
    .bind(profile)
    .bind(format!("{handle}@web"))
    .fetch_one(pool)
    .await
    .expect("seed emitter");
    let me: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
              VALUES ('kb_profiles', $1, 'notes', 'notes') RETURNING id",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .expect("seed @me context");
    Person {
        profile,
        email,
        emitter,
        me,
    }
}

/// A team that is not the person's personal team, with a context: outside the estate.
async fn team(pool: &PgPool) -> (Uuid, Uuid) {
    sqlx::query_as(
        "WITH t AS (INSERT INTO kb_teams (slug, name) VALUES ('team-' || gen_random_uuid(), 'T') \
                    RETURNING id) \
         INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
              SELECT 'kb_teams', t.id, 'shared', 'shared' FROM t RETURNING owner_id, id",
    )
    .fetch_one(pool)
    .await
    .expect("seed a team and its context")
}

async fn erase(pool: &PgPool, subject: Uuid) -> ErasureCompletion {
    let op = person(pool).await;
    temper_services::test_support::approved_admin(pool, op.profile).await;
    let admin = temper_services::test_support::system_admin_proof_for(pool, op.profile).await;
    execute_erasure(
        pool,
        &admin,
        ProfileId::from(subject),
        Uuid::now_v7(),
        Surface::ApiHttp,
        None,
    )
    .await
    .expect("the person act completes")
}

async fn count(pool: &PgPool, sql: &str, id: Uuid) -> i64 {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("count")
}

/// Ledger rows whose payload or metadata contains `needle`, case-insensitively.
async fn ledger_quoting(pool: &PgPool, needle: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events \
          WHERE payload::text ILIKE '%' || $1 || '%' OR metadata::text ILIKE '%' || $1 || '%'",
    )
    .bind(needle)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn outcome_of<'a>(c: &'a ErasureCompletion, target: &str) -> Vec<&'a str> {
    c.targets
        .iter()
        .filter(|t| t.target == target)
        .map(|t| t.outcome.as_str())
        .collect()
}

async fn completion_payload(pool: &PgPool, subject: Uuid) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'principal_erased' AND e.payload->>'subject_id' = $1::text",
    )
    .bind(subject)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn set_policy(pool: &PgPool, name: &str, category: &str, by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_erasure_retention_policies (name, category, basis, period, set_by_profile_id) \
              VALUES ($1, $2, 'GDPR 17(3)(e)', interval '1 year', $3)",
    )
    .bind(name)
    .bind(category)
    .bind(by)
    .execute(pool)
    .await
    .expect("seed a retention policy");
}

/// A Slack auth link through the real link helper.
async fn link_slack(pool: &PgPool, subject: Uuid, principal: &str) {
    let mut conn = pool.acquire().await.unwrap();
    slack_link_service::link_slack_principal(&mut conn, subject, principal)
        .await
        .expect("link");
}

fn vault_key() -> VaultKey {
    VaultKey::from_base64(&STANDARD.encode([3u8; 32])).expect("key")
}

/// A vault grant through the real vault helper. It need not have a link: a grant can outlive one.
async fn vault_grant(pool: &PgPool, subject: Uuid, principal: &str) {
    let mut conn = pool.acquire().await.unwrap();
    slack_grant_vault_service::store_grant(
        &mut conn,
        &vault_key(),
        slack_grant_vault_service::NewGrant {
            profile_id: subject,
            slack_principal_id: principal,
            refresh_token: "rt",
            access_token: "at",
            access_ttl_secs: Some(3600),
        },
    )
    .await
    .expect("store grant");
}

async fn intent(pool: &PgPool, principal: &str) {
    sqlx::query(
        "INSERT INTO kb_slack_link_intents (id, state_nonce, code_verifier, slack_principal_id, expires_at) \
              VALUES (gen_random_uuid(), gen_random_uuid()::text, 'v', $1, now() + interval '1 hour')",
    )
    .bind(principal)
    .execute(pool)
    .await
    .expect("seed intent");
}

/// FAILS IF an invitation sent by the subject, or addressed to any spelling of their email, keeps
/// an email, a usable token or a pending state. An invitation between two other people is left.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn invitations_sent_by_or_to_the_subject_keep_no_identifier(pool: PgPool) {
    let p = person(&pool).await;
    let other = person(&pool).await;
    let (team_id, _) = team(&pool).await;
    let invite = |by: Uuid, email: String| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO kb_team_invitations (id, team_id, invited_email, invited_by_profile_id, role, token) \
                      VALUES (gen_random_uuid(), $1, $2, $3, 'member', gen_random_uuid()::text) RETURNING id",
            )
            .bind(team_id)
            .bind(email)
            .bind(by)
            .fetch_one(&pool)
            .await
            .expect("seed invitation")
        }
    };
    let sent = invite(p.profile, "friend@elsewhere.test".into()).await;
    // Addressed to the subject, in another case: the match is case-blind.
    let received = invite(other.profile, p.email.to_uppercase()).await;
    let unrelated = invite(other.profile, "someone@elsewhere.test".into()).await;

    let c = erase(&pool, p.profile).await;

    for id in [sent, received] {
        let (email, token, revoked): (String, String, bool) = sqlx::query_as(
            "SELECT invited_email, token, revoked_at IS NOT NULL FROM kb_team_invitations WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            email,
            format!("erased-{id}"),
            "the email is the row's sentinel"
        );
        assert_eq!(
            token,
            format!("erased-{id}"),
            "the token can no longer be presented"
        );
        assert!(revoked, "a pending invitation is revoked");
    }
    let kept: String =
        sqlx::query_scalar("SELECT invited_email FROM kb_team_invitations WHERE id = $1")
            .bind(unrelated)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        kept, "someone@elsewhere.test",
        "other people's invitations stay"
    );
    assert_eq!(outcome_of(&c, "kb_team_invitations.invited_email").len(), 1);
}

/// FAILS IF any credential of the subject's survives the act: a login flow or refresh token bound
/// to them or naming them in its claims, a replay record, a live or credentialed connection of
/// theirs, or a Slack grant, link intent or vault row, whether reached through a link or only
/// through the vault's own profile. A team-homed connection the subject merely registered stays,
/// and so does every credential of another person.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn no_live_credential_of_the_subject_survives(pool: PgPool) {
    let p = person(&pool).await;
    let other = person(&pool).await;
    let (team_id, team_ctx) = team(&pool).await;
    // An IdP-subject auth link: the claims of an unbound row name this id.
    sqlx::query(
        "INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id, email) \
              VALUES (gen_random_uuid(), $1, 'auth0', $2, $3)",
    )
    .bind(p.profile)
    .bind(format!("auth0|{}", p.profile))
    .bind(&p.email)
    .execute(&pool)
    .await
    .expect("seed auth link");

    let flow = |profile: Option<Uuid>, claims: serde_json::Value| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO kb_oauth_flow (relay_state, status, client_id, redirect_uri, code_challenge, \
                                            code_challenge_method, oauth_state, audience, claims, expires_at, profile_id) \
                      VALUES (gen_random_uuid()::text, 'consumed', 'c', 'r', 'cc', 'S256', 's', 'a', $1, \
                              now() + interval '1 hour', $2)",
            )
            .bind(claims)
            .bind(profile)
            .execute(&pool)
            .await
            .expect("seed flow");
        }
    };
    flow(Some(p.profile), serde_json::json!({"sub": "x"})).await;
    flow(None, serde_json::json!({"email": p.email.to_uppercase()})).await;
    flow(Some(other.profile), serde_json::json!({"sub": "y"})).await;

    let token = |profile: Option<Uuid>, claims: serde_json::Value| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO kb_oauth_refresh_tokens (token_hash, client_id, claims, expires_at, profile_id) \
                      VALUES (gen_random_uuid()::text, 'c', $1, now() + interval '1 day', $2) RETURNING id",
            )
            .bind(claims)
            .bind(profile)
            .fetch_one(&pool)
            .await
            .expect("seed refresh token")
        }
    };
    let bound = token(Some(p.profile), serde_json::json!({"sub": "x"})).await;
    token(
        None,
        serde_json::json!({"sub": format!("auth0|{}", p.profile)}),
    )
    .await;
    let others = token(Some(other.profile), serde_json::json!({"sub": "y"})).await;
    sqlx::query(
        "INSERT INTO kb_oauth_refresh_replays (token_id, chain_id, profile_id, client_id, rotated_at, first_age_seconds) \
              SELECT id, chain_id, profile_id, client_id, now(), 1 FROM kb_oauth_refresh_tokens WHERE id = $1",
    )
    .bind(bound)
    .execute(&pool)
    .await
    .expect("seed replay evidence (RESTRICTs the token's delete)");

    let connection = |profile: Uuid, registered_by: Uuid, home: Uuid, emitter: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO kb_connections (provider, slug, name, owner_team_id, registered_by_profile_id, \
                                             profile_id, emitter_entity_id, home_context_id, credential, observed_reach) \
                      VALUES ('github', 'c-' || gen_random_uuid(), 'c', $1, $2, $3, $4, $5, \
                              '{\"connector\": \"jane-roe-gh\"}'::jsonb, '{\"login\": \"janeroe\"}'::jsonb) \
                   RETURNING id",
            )
            .bind(team_id)
            .bind(registered_by)
            .bind(profile)
            .bind(emitter)
            .bind(home)
            .fetch_one(&pool)
            .await
            .expect("seed connection")
        }
    };
    // Homed in the subject's estate (its machine principal is someone else's).
    let estate_homed = connection(other.profile, p.profile, p.me, other.emitter).await;
    // The subject's own machine principal, homed in a team.
    let own = connection(p.profile, other.profile, team_ctx, p.emitter).await;
    // Registered by the subject, homed in a team, another principal's: the team's integration.
    let teams = connection(other.profile, p.profile, team_ctx, other.emitter).await;

    link_slack(&pool, p.profile, "slack:T1:ULINKED").await;
    vault_grant(&pool, p.profile, "slack:T1:ULINKED").await;
    // A grant that outlived its link: reached only through vault.profile_id.
    vault_grant(&pool, p.profile, "slack:T1:UORPHAN").await;
    intent(&pool, "slack:T1:UORPHAN").await;
    vault_grant(&pool, other.profile, "slack:T1:UOTHER").await;

    erase(&pool, p.profile).await;

    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM kb_oauth_flow WHERE profile_id = $1 OR claims::text ILIKE '%' || $2 || '%') \
              + (SELECT count(*) FROM kb_oauth_refresh_tokens \
                  WHERE profile_id = $1 OR claims::text ILIKE '%' || $1::text || '%') \
              + (SELECT count(*) FROM kb_oauth_refresh_replays WHERE profile_id = $1) \
              + (SELECT count(*) FROM kb_slack_grant_vault WHERE profile_id = $1) \
              + (SELECT count(*) FROM kb_slack_link_intents WHERE slack_principal_id LIKE 'slack:T1:U%' \
                                                               AND slack_principal_id <> 'slack:T1:UOTHER')",
    )
    .bind(p.profile)
    .bind(&p.email)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        left, 0,
        "no login, refresh or Slack credential of the subject survives"
    );

    for id in [estate_homed, own] {
        let (revoked, credential, reach): (bool, Option<serde_json::Value>, Option<serde_json::Value>) =
            sqlx::query_as(
                "SELECT revoked_at IS NOT NULL, credential, observed_reach FROM kb_connections WHERE id = $1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(revoked, "the subject's connection is revoked in temper");
        assert_eq!((credential, reach), (None, None), "its binding is emptied");
    }
    let (revoked, credential): (bool, Option<serde_json::Value>) = sqlx::query_as(
        "SELECT revoked_at IS NOT NULL, credential FROM kb_connections WHERE id = $1",
    )
    .bind(teams)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !revoked && credential.is_some(),
        "the team's connection stays live"
    );

    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM kb_oauth_refresh_tokens WHERE id = $1",
            others
        )
        .await,
        1,
        "another person's token stays"
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM kb_slack_grant_vault WHERE profile_id = $1",
            other.profile
        )
        .await,
        1,
        "another person's grant stays"
    );
}

/// FAILS IF the subject's entities keep their launch metadata, or a staged upload of theirs, or one
/// homed in their estate, keeps its bytes.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn entity_metadata_and_staged_uploads_are_gone(pool: PgPool) {
    let p = person(&pool).await;
    let other = person(&pool).await;
    let (_, team_ctx) = team(&pool).await;
    let upload = |owner: Uuid, home: Uuid| {
        let pool = pool.clone();
        async move {
            let id = Uuid::now_v7();
            sqlx::query(
                "WITH u AS (INSERT INTO kb_blob_uploads (id, owner_profile_id, home_table, home_id, content_type) \
                                 VALUES ($1, $2, 'kb_contexts', $3, 'text/plain') RETURNING id) \
                 INSERT INTO kb_blob_upload_segments (upload_id, seq, bytes, segment_hash) \
                      SELECT id, 0, convert_to('Jane Roe', 'UTF8'), 'h' FROM u",
            )
            .bind(id)
            .bind(owner)
            .bind(home)
            .execute(&pool)
            .await
            .expect("seed staged upload");
            id
        }
    };
    let owned = upload(p.profile, team_ctx).await;
    let homed = upload(other.profile, p.me).await;
    let others = upload(other.profile, team_ctx).await;

    let c = erase(&pool, p.profile).await;

    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM kb_entities WHERE profile_id = $1 AND metadata <> '{}'::jsonb",
            p.profile
        )
        .await,
        0,
        "the subject's entities keep no launch metadata"
    );
    for id in [owned, homed] {
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM kb_blob_upload_segments WHERE upload_id = $1",
                id
            )
            .await,
            0,
            "staged bytes of the subject's, or in their estate, are gone"
        );
    }
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM kb_blob_upload_segments WHERE upload_id = $1",
            others
        )
        .await,
        1,
        "another person's upload elsewhere stays"
    );
    assert_eq!(outcome_of(&c, "kb_entities.metadata"), vec!["erased"]);
}

/// FAILS IF free text about the subject survives, in its row or on the ledger: a standing reason, a
/// governance reason, join and review notes, or the disconnected Slack id in its ledger record.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn free_text_about_the_subject_is_erased_in_rows_and_ledger(pool: PgPool) {
    let p = person(&pool).await;
    let admin = person(&pool).await;
    let (team_id, _) = team(&pool).await;
    sqlx::query("SELECT principal_standing_apply($1, 'revoke', 'revoked', $2, $3)")
        .bind(p.profile)
        .bind(admin.profile)
        .bind(ABOUT)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("SELECT principal_governance_set($1, true, $2, NULL)")
        .bind(p.profile)
        .bind(admin.profile)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("SELECT principal_governance_set($1, false, $2, $3)")
        .bind(p.profile)
        .bind(admin.profile)
        .bind(ABOUT)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO kb_join_requests (id, team_id, requesting_profile_id, message, source, decision_note) \
              VALUES (gen_random_uuid(), $1, $2, $3, 'web', $3)",
    )
    .bind(team_id)
    .bind(p.profile)
    .bind(ABOUT)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_principal_review_requests (id, profile_id, message, decision_note) \
              VALUES (gen_random_uuid(), $1, $2, $2)",
    )
    .bind(p.profile)
    .bind(ABOUT)
    .execute(&pool)
    .await
    .unwrap();
    // A disconnect through the real service writes the Slack id onto the ledger.
    link_slack(&pool, p.profile, "slack:T1:UJANEROE").await;
    disconnect_slack_principal(
        &pool,
        ProfileId::from(p.profile),
        DisconnectRequest {
            slack_principal_id: "slack:T1:UJANEROE",
            key: &vault_key(),
            mode: AuthMode::ExternalIdp,
            revoke_url: "http://127.0.0.1:1/unused".to_string(),
            client_id: "c",
        },
    )
    .await
    .unwrap();
    assert!(
        ledger_quoting(&pool, ABOUT).await > 0,
        "the world holds the text first"
    );
    assert!(ledger_quoting(&pool, "UJANEROE").await > 0);

    let c = erase(&pool, p.profile).await;

    let rows: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM kb_principal_standing_events WHERE profile_id = $1 AND reason IS NOT NULL) \
              + (SELECT count(*) FROM kb_join_requests WHERE requesting_profile_id = $1 \
                                                        AND (message IS NOT NULL OR decision_note IS NOT NULL)) \
              + (SELECT count(*) FROM kb_principal_review_requests WHERE profile_id = $1 \
                                                        AND (message IS NOT NULL OR decision_note IS NOT NULL))",
    )
    .bind(p.profile)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 0, "no row keeps free text about the subject");
    assert_eq!(
        ledger_quoting(&pool, ABOUT).await,
        0,
        "the ledger keeps no reason about them"
    );
    assert_eq!(
        ledger_quoting(&pool, "UJANEROE").await,
        0,
        "nor their Slack id"
    );
    assert!(
        outcome_of(&c, "kb_events.payload")
            .iter()
            .any(|o| o.contains("admin event(s) about the subject")),
        "the record names the ledger rewrite: {:?}",
        c.targets
    );
    // The rewrite is the act's own, under its authority, at the paths its record lists.
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_event_field_redactions WHERE authority = 'principal' \
            AND path IN ('reason', 'slack_principal_id')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 3, "standing reason, governance reason and Slack id");
}

/// FAILS IF a category the deployment's policy names is not kept, or is kept without the record
/// naming it. The kept text stays in its row and on the ledger; a category no policy names is
/// erased beside it; and the record carries the policy as it stood.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_retained_category_is_kept_and_named_in_the_record(pool: PgPool) {
    let p = person(&pool).await;
    let admin = person(&pool).await;
    set_policy(&pool, "security-audit", "standing-reasons", admin.profile).await;
    sqlx::query("SELECT principal_standing_apply($1, 'revoke', 'revoked', $2, $3)")
        .bind(p.profile)
        .bind(admin.profile)
        .bind(ABOUT)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("SELECT principal_governance_set($1, true, $2, NULL)")
        .bind(p.profile)
        .bind(admin.profile)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("SELECT principal_governance_set($1, false, $2, 'governance: Jane Roe')")
        .bind(p.profile)
        .bind(admin.profile)
        .execute(&pool)
        .await
        .unwrap();

    let c = erase(&pool, p.profile).await;

    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM kb_principal_standing_events WHERE profile_id = $1 AND reason IS NOT NULL",
            p.profile
        )
        .await,
        1,
        "the kept reason stays in its row"
    );
    assert_eq!(ledger_quoting(&pool, ABOUT).await, 1, "and on the ledger");
    assert_eq!(
        ledger_quoting(&pool, "governance: Jane Roe").await,
        0,
        "a category no policy names is erased"
    );
    assert_eq!(
        outcome_of(&c, "kb_principal_standing_events.reason"),
        vec!["retained under policy security-audit"]
    );
    assert_eq!(
        outcome_of(&c, "kb_events.payload (standing reasons)"),
        vec!["retained under policy security-audit"]
    );
    let retained = &completion_payload(&pool, p.profile).await["retained"];
    assert_eq!(
        retained,
        &serde_json::json!([{
            "category": "standing-reasons",
            "policy": "security-audit",
            "basis": "GDPR 17(3)(e)",
            "period": "1 year",
        }]),
        "the record carries the policy as it stood"
    );
}

/// FAILS IF keeping replay evidence keeps a usable token. Under a credential-replay-evidence policy
/// the replay rows stay, and the tokens they reference stay only as dead referents: revoked, claims
/// emptied, the hash replaced.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn kept_replay_evidence_keeps_no_usable_token(pool: PgPool) {
    let p = person(&pool).await;
    set_policy(
        &pool,
        "copy-forensics",
        "credential-replay-evidence",
        p.profile,
    )
    .await;
    let token: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_oauth_refresh_tokens (token_hash, client_id, claims, expires_at, profile_id) \
              VALUES ('live-hash', 'c', $1, now() + interval '1 day', $2) RETURNING id",
    )
    .bind(serde_json::json!({"email": p.email}))
    .bind(p.profile)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_oauth_refresh_replays (token_id, chain_id, profile_id, client_id, rotated_at, first_age_seconds) \
              SELECT id, chain_id, profile_id, client_id, now(), 1 FROM kb_oauth_refresh_tokens WHERE id = $1",
    )
    .bind(token)
    .execute(&pool)
    .await
    .unwrap();

    let c = erase(&pool, p.profile).await;

    let (revoked, claims, hash): (bool, serde_json::Value, String) = sqlx::query_as(
        "SELECT revoked_at IS NOT NULL, claims, token_hash FROM kb_oauth_refresh_tokens WHERE id = $1",
    )
    .bind(token)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(revoked, "the kept token is revoked");
    assert_eq!(claims, serde_json::json!({}), "and names no one");
    assert_eq!(
        hash,
        format!("erased-{token}"),
        "and no presented token can match it"
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM kb_oauth_refresh_replays WHERE token_id = $1",
            token
        )
        .await,
        1,
        "the evidence stays"
    );
    assert_eq!(
        outcome_of(&c, "kb_oauth_refresh_replays"),
        vec!["retained under policy copy-forensics"]
    );
}

/// FAILS IF the person act's ledger authority reaches another principal's admin events: a forged
/// principal_erased record for a tombstoned subject, listing a stranger's standing event, cannot
/// rewrite that stranger's reason.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_principal_authority_reaches_only_the_subjects_own_admin_events(pool: PgPool) {
    let p = person(&pool).await;
    let stranger = person(&pool).await;
    sqlx::query(
        "SELECT principal_standing_apply($1, 'revoke', 'revoked', NULL, 'about the stranger')",
    )
    .bind(stranger.profile)
    .execute(&pool)
    .await
    .unwrap();
    let theirs: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'principal_standing_changed' AND e.payload->>'subject_id' = $1::text",
    )
    .bind(stranger.profile)
    .fetch_one(&pool)
    .await
    .unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE kb_profiles SET tombstoned_at = now() WHERE id = $1")
        .bind(p.profile)
        .execute(&mut *tx)
        .await
        .unwrap();
    let record = serde_json::json!({
        "subject_table": "kb_profiles",
        "subject_id": p.profile,
        "estate_contexts": [],
        "resource_erasures": [],
        "charters_held": [],
        "retained": [],
        "redacted_fields": [{"event": theirs, "paths": ["reason"]}],
    });
    let ev: Uuid =
        sqlx::query_scalar("SELECT _event_append('principal_erased', $1, NULL, NULL, $2)")
            .bind(p.emitter)
            .bind(&record)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO kb_event_field_redactions (event_id, path, redacted_by, authority) \
              VALUES ($1, 'reason', $2, 'principal')",
    )
    .bind(theirs)
    .bind(ev)
    .execute(&mut *tx)
    .await
    .unwrap();
    let rewrite = sqlx::query(
        "UPDATE kb_events SET payload = jsonb_set(payload, '{reason}', 'null'::jsonb) WHERE id = $1",
    )
    .bind(theirs)
    .execute(&mut *tx)
    .await;
    tx.rollback().await.unwrap();
    let err = rewrite.expect_err("a stranger's reason is outside the subject's scope");
    assert!(
        err.to_string().contains("append-only"),
        "refused by the ledger verifier: {err}"
    );
}

// ── Slack revocation at Slack (ruled 2026-10-10: best-effort before the act, a bounded retry). ──

use temper_services::services::erasure_slack_revocation_service::{self, SlackRevokeConfig};
use temper_services::services::slack_disconnect_service::IdpRevokeTarget;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn erase_with_slack(pool: &PgPool, subject: Uuid, revoke_url: &str) -> ErasureCompletion {
    let op = person(pool).await;
    temper_services::test_support::approved_admin(pool, op.profile).await;
    let admin = temper_services::test_support::system_admin_proof_for(pool, op.profile).await;
    let key = vault_key();
    let cfg = SlackRevokeConfig {
        key: &key,
        target: IdpRevokeTarget {
            mode: AuthMode::ExternalIdp,
            revoke_url,
            client_id: "slack-client",
        },
    };
    execute_erasure(
        pool,
        &admin,
        ProfileId::from(subject),
        Uuid::now_v7(),
        Surface::ApiHttp,
        Some(&cfg),
    )
    .await
    .expect("the person act completes")
}

/// A revoke endpoint that answers `status` to every revoke of the token "rt".
async fn slack_answering(status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/revoke"))
        .and(body_string_contains("token=rt"))
        .respond_with(ResponseTemplate::new(status))
        .mount(&server)
        .await;
    server
}

async fn queued(pool: &PgPool) -> Vec<(Uuid, Option<Vec<u8>>, Option<Vec<u8>>, Option<String>)> {
    sqlx::query_as(
        "SELECT id, rt_nonce, rt_ciphertext, outcome FROM kb_erasure_slack_revocations ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// FAILS IF the act destroys the subject's Slack grants without first asking Slack to revoke them,
/// through a link or only through the vault's own profile.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_subjects_slack_grants_are_revoked_at_slack_before_the_act(pool: PgPool) {
    let p = person(&pool).await;
    link_slack(&pool, p.profile, "slack:T1:ULINKED").await;
    vault_grant(&pool, p.profile, "slack:T1:ULINKED").await;
    vault_grant(&pool, p.profile, "slack:T1:UORPHAN").await;
    let slack = slack_answering(200).await;

    let c = erase_with_slack(&pool, p.profile, &format!("{}/oauth/revoke", slack.uri())).await;

    assert_eq!(
        c.slack_revocation.revoked, 2,
        "both grants revoked at Slack"
    );
    assert_eq!(c.slack_revocation.queued, 0);
    assert_eq!(
        slack.received_requests().await.unwrap().len(),
        2,
        "Slack was asked once per grant"
    );
    assert!(queued(&pool).await.is_empty(), "nothing to retry");
}

/// FAILS IF a grant whose revoke fails is either destroyed unrevoked with no retry, or kept beside
/// anything that names its owner. The failed token is queued, sealed under the queue row's id, and
/// the vault row is gone.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_failed_revoke_is_queued_sealed_and_names_no_one(pool: PgPool) {
    let p = person(&pool).await;
    vault_grant(&pool, p.profile, "slack:T1:UJANEROE").await;
    let slack = slack_answering(503).await;

    let c = erase_with_slack(&pool, p.profile, &format!("{}/oauth/revoke", slack.uri())).await;

    assert_eq!(c.slack_revocation.queued, 1, "the failed revoke is queued");
    let rows = queued(&pool).await;
    assert_eq!(rows.len(), 1);
    let (id, nonce, ciphertext, outcome) = &rows[0];
    assert_eq!(outcome, &None, "pending");
    let row: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(r) FROM kb_erasure_slack_revocations r WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let text = row.to_string();
    assert!(
        !text.contains("UJANEROE") && !text.contains(&p.profile.to_string()),
        "the queued row names neither the Slack principal nor the profile: {text}"
    );
    let rt = temper_services::services::slack_grant_vault_service::open_for_revocation(
        &vault_key(),
        *id,
        nonce.as_deref().unwrap(),
        ciphertext.as_deref().unwrap(),
    )
    .expect("sealed under its own id");
    assert_eq!(rt, "rt");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM kb_slack_grant_vault WHERE profile_id = $1",
            p.profile
        )
        .await,
        0,
        "the vault row is destroyed"
    );
}

/// FAILS IF the drain neither retries a queued revoke nor empties the token once Slack confirms:
/// a revoke that fails stays queued with a later attempt, and one that succeeds keeps only its
/// outcome.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_drain_retries_a_queued_revoke_until_slack_confirms(pool: PgPool) {
    let p = person(&pool).await;
    vault_grant(&pool, p.profile, "slack:T1:UJANEROE").await;
    let down = slack_answering(503).await;
    erase_with_slack(&pool, p.profile, &format!("{}/oauth/revoke", down.uri())).await;

    let key = vault_key();
    fn cfg<'a>(key: &'a VaultKey, url: &'a str) -> SlackRevokeConfig<'a> {
        SlackRevokeConfig {
            key,
            target: IdpRevokeTarget {
                mode: AuthMode::ExternalIdp,
                revoke_url: url,
                client_id: "slack-client",
            },
        }
    }
    let down_url = format!("{}/oauth/revoke", down.uri());
    let still_down = erasure_slack_revocation_service::drain(&pool, Some(&cfg(&key, &down_url)))
        .await
        .unwrap();
    assert_eq!((still_down.attempted, still_down.retrying), (1, 1));
    let later: bool = sqlx::query_scalar(
        "SELECT next_attempt_at > now() AND attempts = 1 FROM kb_erasure_slack_revocations",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(later, "a failed retry waits before the next");

    // Due again; Slack is back.
    sqlx::query("UPDATE kb_erasure_slack_revocations SET next_attempt_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    let up = slack_answering(200).await;
    let up_url = format!("{}/oauth/revoke", up.uri());
    let back = erasure_slack_revocation_service::drain(&pool, Some(&cfg(&key, &up_url)))
        .await
        .unwrap();
    assert_eq!(back.revoked, 1);
    let rows = queued(&pool).await;
    assert_eq!(
        (
            rows[0].1.is_none(),
            rows[0].2.is_none(),
            rows[0].3.as_deref()
        ),
        (true, true, Some("revoked")),
        "the token is emptied and only the outcome stays"
    );
}

/// FAILS IF a queued token can outlive its retry window, even on a deployment with Slack no
/// longer configured.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_queued_token_never_outlives_its_window(pool: PgPool) {
    let p = person(&pool).await;
    vault_grant(&pool, p.profile, "slack:T1:UJANEROE").await;
    let down = slack_answering(503).await;
    erase_with_slack(&pool, p.profile, &format!("{}/oauth/revoke", down.uri())).await;
    sqlx::query("UPDATE kb_erasure_slack_revocations SET give_up_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();

    let tick = erasure_slack_revocation_service::drain(&pool, None)
        .await
        .unwrap();

    assert_eq!(tick.abandoned, 1);
    let rows = queued(&pool).await;
    assert_eq!(
        (
            rows[0].1.is_none(),
            rows[0].2.is_none(),
            rows[0].3.as_deref()
        ),
        (true, true, Some("abandoned")),
    );
}
