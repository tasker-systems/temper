use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::connection::{ConnectionCredential, ProvisionConnectionRequest};
use temper_core::types::ids::ProfileId;
use temper_core::types::team::TeamRole;

use crate::error::ApiError;
use crate::services::connection_service as svc;

fn req(name: &str, owner_team_id: Option<Uuid>) -> ProvisionConnectionRequest {
    ProvisionConnectionRequest {
        provider: "github".into(),
        name: name.into(),
        owner_team_id,
        reach_granularity: Some("repo-set".into()),
        reach_covers: Some("acme/temper".into()),
    }
}

/// A provision request that declares NO reach fidelity — neither granularity nor covers.
fn req_no_reach(name: &str, owner_team_id: Option<Uuid>) -> ProvisionConnectionRequest {
    ProvisionConnectionRequest {
        provider: "github".into(),
        name: name.into(),
        owner_team_id,
        reach_granularity: None,
        reach_covers: None,
    }
}

/// Seed a caller who is genuinely a system admin. Under D11 that is a `kb_principal_governance`
/// grant (`is_system_admin`) plus an `approved` `kb_principal_standing` (`has_system_access`, the
/// front door) — no longer gating-team ownership. The gating-team upsert below is retained only
/// because `temper-system` already exists in a migrated database and other rows reference it; it
/// no longer carries authorization meaning.
async fn seed_admin(pool: &PgPool) -> ProfileId {
    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, 'conn-admin', 'conn-admin')",
        id,
    )
    .execute(pool)
    .await
    .expect("seed admin profile");

    // The caller must carry its `<handle>@web` emitter — grant_reach now resolves one to author
    // the grant_created event, exactly as production does (a fixture that skips this passes while
    // production 500s). Provision via the same production path.
    let mut conn = pool.acquire().await.expect("acquire");
    crate::services::profile_service::provision_profile_entities(&mut conn, id, "conn-admin")
        .await
        .expect("provision caller emitters");
    drop(conn);

    let team: Uuid = sqlx::query_scalar!(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system', 'Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("gating team");

    sqlx::query!("UPDATE kb_system_settings SET gating_team_slug = 'temper-system'")
        .execute(pool)
        .await
        .expect("configure gating team");

    sqlx::query!(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
         VALUES ($1, $2, 'owner'::team_role) \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
        team,
        id,
    )
    .execute(pool)
    .await
    .expect("join gating team as owner");

    // What actually confers admin-ness now: an approved standing (front door) + governance.
    crate::test_support::approved_admin(pool, id).await;

    ProfileId::from(id)
}

/// Seed a profile holding `role` on a fresh team, who is NOT a system admin.
///
/// `invite_only` FIRST: `temper-system` carries `auto_join_role = 'watcher'` and
/// `trg_sync_system_membership` fires on profile INSERT, so under the default `open` mode
/// every new profile is auto-joined to the gating team. In open mode this fixture would be
/// asserting about the trigger, not about our gate.
async fn seed_team_member(
    pool: &PgPool,
    handle: &str,
    team_slug: &str,
    role: TeamRole,
) -> (ProfileId, Uuid) {
    sqlx::query!("UPDATE kb_system_settings SET gating_team_slug = 'temper-system'")
        .execute(pool)
        .await
        .expect("configure the gating team");

    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)",
        id,
        handle,
    )
    .execute(pool)
    .await
    .expect("seed profile");

    // Emitters, so this profile can author a grant event if it ends up calling grant_reach.
    let mut conn = pool.acquire().await.expect("acquire");
    crate::services::profile_service::provision_profile_entities(&mut conn, id, handle)
        .await
        .expect("provision caller emitters");
    drop(conn);

    let team: Uuid = sqlx::query_scalar!(
        "INSERT INTO kb_teams (slug, name) VALUES ($1, $1) \
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name RETURNING id",
        team_slug,
    )
    .fetch_one(pool)
    .await
    .expect("seed team");

    sqlx::query!(
        r#"INSERT INTO kb_team_members (team_id, profile_id, role)
               VALUES ($1, $2, $3)"#,
        team,
        id,
        role as TeamRole,
    )
    .execute(pool)
    .await
    .expect("seed membership");

    sqlx::query!(
        "DELETE FROM kb_team_members m USING kb_teams t \
          WHERE m.team_id = t.id AND t.slug = 'temper-system' AND m.profile_id = $1",
        id,
    )
    .execute(pool)
    .await
    .expect("no gating-team membership");

    (ProfileId::from(id), team)
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn provision_creates_profile_entity_context_and_row(pool: PgPool) {
    let admin = seed_admin(&pool).await;

    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");

    assert_eq!(c.provider, "github");
    assert_eq!(c.slug, "acme-github");
    assert_eq!(c.registered_by_profile_id, *admin);

    // Born needs_credential, both capability tiers empty. It never silently pretends to be
    // more than it is.
    assert!(c.needs_credential(), "born needs_credential");
    assert!(!c.is_ledger_capable(), "no webhook events registered yet");
    assert!(!c.is_reach_capable(), "no tool manifest yet");

    // The emitter exists and is named `<handle>@webhook`.
    let entity_name = sqlx::query_scalar!(
        "SELECT name FROM kb_entities WHERE id = $1",
        c.emitter_entity_id
    )
    .fetch_one(&pool)
    .await
    .expect("emitter entity");
    assert!(
        entity_name.ends_with("@webhook"),
        "emitter is the webhook entity, got {entity_name}"
    );

    // The home context exists and is owned by the connection's own profile (teamless here).
    let (owner_table, owner_id) = sqlx::query!(
        r#"SELECT owner_table, owner_id FROM kb_contexts WHERE id = $1"#,
        c.home_context_id
    )
    .fetch_one(&pool)
    .await
    .map(|r| (r.owner_table, r.owner_id))
    .expect("home context");
    assert_eq!(owner_table, "kb_profiles");
    assert_eq!(owner_id, c.profile_id);
}

/// The three excess-reach affirmation columns are born NULL (unaffirmed) and round-trip
/// through `get`. NO grant requiring affirmation has happened, so nothing is stamped — the
/// honest "never affirmed" state. Read-side only; enforcement is Beat 2.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn reach_affirmation_is_born_null_and_round_trips(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");

    assert!(c.reach_affirmed_by.is_none(), "unaffirmed by default");
    assert!(c.reach_affirmed_at.is_none(), "unaffirmed by default");
    assert!(c.reach_affirmation.is_none(), "unaffirmed by default");

    // Round-trip through a fresh get — the columns exist and read back NULL.
    let reloaded = svc::get(&pool, c.id).await.expect("get");
    assert!(reloaded.reach_affirmed_by.is_none());
    assert!(reloaded.reach_affirmed_at.is_none());
    assert!(reloaded.reach_affirmation.is_none());
}

/// `declares_reach()` is the honest, non-computing signal: TRUE when the connection was
/// provisioned WITH a declared reach fidelity, FALSE when it declares none. No enforcement —
/// this only reads whether an affirmation would be required (Beat 2 acts on it).
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn declares_reach_reflects_the_declared_fidelity(pool: PgPool) {
    let admin = seed_admin(&pool).await;

    let with_reach = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision with reach");
    assert!(
        with_reach.declares_reach(),
        "a connection provisioned with reach_granularity/reach_covers declares reach"
    );

    let no_reach = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req_no_reach("Bare GitHub", None),
    )
    .await
    .expect("provision without reach");
    assert!(
        !no_reach.declares_reach(),
        "a connection provisioned with no reach fidelity declares no reach"
    );
}

/// The direction-of-credential assertion, and the single easiest thing to get wrong here.
/// `kb_machine_clients` answers "who may authenticate TO temper". A connection is the
/// opposite: temper authenticating to a REMOTE system. GitHub holds no temper token, so a
/// connection must have NO auth link and NO machine-client row.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_connection_has_no_auth_link_and_no_machine_client_row(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");

    let auth_links = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_profile_auth_links WHERE profile_id = $1",
        c.profile_id,
    )
    .fetch_one(&pool)
    .await
    .expect("count auth links")
    .unwrap_or(0);
    assert_eq!(auth_links, 0, "a connection never authenticates TO temper");

    let machine_rows = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_machine_clients WHERE profile_id = $1",
        c.profile_id,
    )
    .fetch_one(&pool)
    .await
    .expect("count machine clients")
    .unwrap_or(0);
    assert_eq!(machine_rows, 0, "a connection is not a machine client");
}

/// The connection's profile gets exactly ONE entity — the webhook emitter — and none of the
/// four `Surface::ALL` emitters. `webhook` is deliberately not a Surface variant: making it
/// one would provision a webhook emitter onto every human profile.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn the_connection_profile_gets_only_the_webhook_emitter(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");

    let names: Vec<String> = sqlx::query_scalar!(
        "SELECT name FROM kb_entities WHERE profile_id = $1 ORDER BY name",
        c.profile_id,
    )
    .fetch_all(&pool)
    .await
    .expect("entities");

    assert_eq!(names.len(), 1, "exactly one emitter, got {names:?}");
    assert!(names[0].ends_with("@webhook"));
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_team_owner_may_provision_for_their_own_team(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "conn-owner", "acme", TeamRole::Owner).await;

    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme Linear", Some(team)),
    )
    .await
    .expect("a team owner runs their own connections");
    assert_eq!(c.owner_team_id, Some(team));

    // The home context is owned by the TEAM, so read authz inherits for free.
    let owner_table = sqlx::query_scalar!(
        "SELECT owner_table FROM kb_contexts WHERE id = $1",
        c.home_context_id
    )
    .fetch_one(&pool)
    .await
    .expect("home context");
    assert_eq!(owner_table, "kb_teams");
}

/// Owner, not maintainer. `machine_authz::authorize` admits `Some(TeamRole::Owner)` only.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_mere_maintainer_cannot_provision_for_the_team(pool: PgPool) {
    let (maintainer, team) =
        seed_team_member(&pool, "conn-maintainer", "acme", TeamRole::Maintainer).await;

    let err = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, maintainer.uuid()).await,
        &req("Acme Linear", Some(team)),
    )
    .await
    .expect_err("a maintainer is not an owner");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
}

/// Teamless fails closed. "No team to check" must never mean "nothing to deny".
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_teamless_connection_is_admin_only(pool: PgPool) {
    let (outsider, _team) = seed_team_member(&pool, "conn-outsider", "acme", TeamRole::Owner).await;

    let err = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, outsider.uuid()).await,
        &req("Rogue GitHub", None),
    )
    .await
    .expect_err("teamless is admin-only");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
}

/// Auth before writes: a rejected provisioning must leave the DB completely unchanged — no
/// orphaned profile, no orphaned entity, no orphaned context. Resolving authority *before*
/// the transaction opens is what makes this assertable.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_rejected_provisioning_writes_nothing(pool: PgPool) {
    let (outsider, _team) = seed_team_member(&pool, "conn-outsider", "acme", TeamRole::Owner).await;

    let before = sqlx::query_scalar!("SELECT count(*) FROM kb_profiles")
        .fetch_one(&pool)
        .await
        .expect("count")
        .unwrap_or(0);

    svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, outsider.uuid()).await,
        &req("Rogue GitHub", None),
    )
    .await
    .expect_err("denied");

    let after = sqlx::query_scalar!("SELECT count(*) FROM kb_profiles")
        .fetch_one(&pool)
        .await
        .expect("count")
        .unwrap_or(0);
    assert_eq!(before, after, "a denied provisioning left a profile behind");

    let connections = sqlx::query_scalar!("SELECT count(*) FROM kb_connections")
        .fetch_one(&pool)
        .await
        .expect("count")
        .unwrap_or(0);
    assert_eq!(connections, 0);
}

/// Two connections can share a name; their slugs must still be distinct.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_slug_collision_suffixes(pool: PgPool) {
    let admin = seed_admin(&pool).await;

    let a = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("first");
    let b = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("second");

    assert_eq!(a.slug, "acme-github");
    assert_eq!(b.slug, "acme-github-2");
}

/// Revocation denies the connection, and nothing else: the profile, emitter, and home context
/// survive, because events already attributed to that emitter must keep resolving.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn revoke_marks_dead_but_keeps_the_emitter_resolvable(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");

    let revoked = svc::revoke(
        &pool,
        c.id,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
    )
    .await
    .expect("revoke");
    assert!(revoked.revoked_at.is_some());
    assert_eq!(revoked.revoked_by_profile_id, Some(*admin));

    let entity_still_there = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_entities WHERE id = $1) AS "e!: bool""#,
        c.emitter_entity_id,
    )
    .fetch_one(&pool)
    .await
    .expect("entity");
    assert!(entity_still_there, "the emitter must keep resolving");

    // Hidden from the default list, visible when asked for.
    let visible = svc::list(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        false,
    )
    .await
    .expect("list");
    assert!(visible.is_empty());
    let all = svc::list(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        true,
    )
    .await
    .expect("list revoked");
    assert_eq!(all.len(), 1);
}

/// An empty scope must DENY. A team owner sees only connections owned by a team they own —
/// never a teamless one, which is admin-only.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn list_scopes_to_teams_the_caller_owns(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let (owner, team) = seed_team_member(&pool, "conn-owner", "acme", TeamRole::Owner).await;

    svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Teamless GitHub", None),
    )
    .await
    .expect("teamless");
    svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme Linear", Some(team)),
    )
    .await
    .expect("team-owned");

    let seen = svc::list(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        false,
    )
    .await
    .expect("list");
    assert_eq!(seen.len(), 1, "the teamless connection must not be visible");
    assert_eq!(seen[0].owner_team_id, Some(team));

    let admin_sees = svc::list(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        false,
    )
    .await
    .expect("list");
    assert_eq!(admin_sees.len(), 2, "an admin sees every row");
}

fn credential() -> ConnectionCredential {
    ConnectionCredential {
        broker: "vercel-connect".to_string(),
        connector: "conn_abc123".to_string(),
        installation: Some("inst_42".to_string()),
    }
}

/// A broker that mints successfully — the default for tests where the mint
/// outcome is not what is under test.
fn granting_broker() -> crate::broker::FakeBroker {
    crate::broker::FakeBroker::granting(serde_json::json!({"repository_selection": "all"}))
}

/// The round-trip is the whole point of typing the column: what we write must come back as the
/// same struct, or the broker seam cannot dispatch on `broker`. Asserted through
/// `credential_typed`, not by eyeballing the raw JSON.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn attaching_a_credential_flips_needs_credential_and_round_trips(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");
    assert!(c.needs_credential(), "born needs_credential");

    let attached = svc::attach_credential(
        &pool,
        &granting_broker(),
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        c.id,
        &credential(),
    )
    .await
    .expect("attach")
    .connection;

    // It flips off because the COLUMN is non-NULL — there is no status to set.
    assert!(!attached.needs_credential());
    assert_eq!(
        attached
            .credential_typed()
            .expect("credential present")
            .expect("credential parses"),
        credential(),
        "the stored credential must round-trip through ConnectionCredential"
    );

    // Attaching a credential confers NEITHER capability tier. They are separately provisioned.
    assert!(!attached.is_ledger_capable());
    assert!(!attached.is_reach_capable());
}

/// A broker that mints successfully records what it observed, and the observed
/// reach rides back next to the connection's declared reach.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_granting_broker_records_the_observed_reach(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");

    let out = svc::attach_credential(
        &pool,
        &granting_broker(),
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        c.id,
        &credential(),
    )
    .await
    .expect("attach");

    assert!(out.verification.verified, "a successful mint is verified");
    assert_eq!(
        out.verification
            .observed_reach
            .as_ref()
            .and_then(|r| r.get("repository_selection"))
            .and_then(|v| v.as_str()),
        Some("all"),
        "the mint metadata is surfaced as observed_reach"
    );
    assert!(!out.connection.needs_credential());
}

/// A broker that *proves* the credential bad blocks the attach and writes
/// nothing — the seam failing loudly at attach, not silently at S3.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_rejecting_broker_blocks_the_attach_and_writes_nothing(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");

    let broker = crate::broker::FakeBroker::rejecting();
    assert!(
        svc::attach_credential(
            &pool,
            &broker,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            c.id,
            &credential()
        )
        .await
        .is_err(),
        "a proven-bad credential must be rejected"
    );

    let reloaded = svc::get(&pool, c.id).await.expect("still there");
    assert!(
        reloaded.needs_credential(),
        "a rejected attach must write nothing — the credential stays NULL"
    );
}

/// The broker's own error text, as a Vercel Connect rejection carries it.
const BROKER_DETAIL: &str =
    r#"broker returned 403: {"error":{"code":"forbidden","message":"team_detail_7f3a"}}"#;

/// A rejection answers a fixed sentence. The broker's error text goes to the server log only.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_rejected_attach_answers_a_fixed_sentence_and_logs_the_brokers_error(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let caller = crate::test_support::human_principal_for(&pool, admin.uuid()).await;
    let c = svc::provision(&pool, &caller, &req("Rejected Detail", None))
        .await
        .expect("provision");

    let broker = crate::broker::FakeBroker::rejecting_with(BROKER_DETAIL);
    let (out, logs) = crate::test_support::captured_logs(svc::attach_credential(
        &pool,
        &broker,
        &caller,
        c.id,
        &credential(),
    ))
    .await;
    match out {
        Err(ApiError::BadRequest(msg)) => {
            assert_eq!(msg, "connector rejected the credential");
        }
        other => panic!("expected a 400, got {other:?}"),
    }
    assert!(
        logs.contains("team_detail_7f3a"),
        "the broker's error is logged: {logs}"
    );
}

/// A broker failure that says nothing about the credential attaches with a fixed note. The
/// broker's error text goes to the server log only.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_unverified_attach_notes_a_fixed_sentence_and_logs_the_brokers_error(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let caller = crate::test_support::human_principal_for(&pool, admin.uuid()).await;
    let c = svc::provision(&pool, &caller, &req("Unverified Detail", None))
        .await
        .expect("provision");

    let broker = crate::broker::FakeBroker::failing_with(BROKER_DETAIL);
    let (out, logs) = crate::test_support::captured_logs(svc::attach_credential(
        &pool,
        &broker,
        &caller,
        c.id,
        &credential(),
    ))
    .await;
    let out = out.expect("an unverifiable credential still attaches");
    assert!(!out.verification.verified);
    let note = out.verification.note.expect("an unverified attach says so");
    assert_eq!(
        note,
        "not verified — the broker's error is in the server log"
    );
    assert!(
        logs.contains("team_detail_7f3a"),
        "the broker's error is logged: {logs}"
    );
}

/// A connector that needs consent, or a deployment with no broker, still
/// attaches — but says, out loud, that it is not verified (invariant 6).
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_unverifiable_but_not_rejected_credential_attaches_with_a_note(pool: PgPool) {
    let admin = seed_admin(&pool).await;

    // needs-consent: persisted, flagged.
    let c1 = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Consent Pending", None),
    )
    .await
    .expect("provision");
    let consent = crate::broker::FakeBroker::needs_consent();
    let out = svc::attach_credential(
        &pool,
        &consent,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        c1.id,
        &credential(),
    )
    .await
    .expect("attach still succeeds");
    assert!(
        !out.connection.needs_credential(),
        "the credential is recorded"
    );
    assert!(!out.verification.verified);
    assert!(
        out.verification.note.is_some(),
        "an unverified attach must carry a note saying why"
    );

    // no broker configured: same shape.
    let c2 = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("No Broker", None),
    )
    .await
    .expect("provision");
    let out2 = svc::attach_credential(
        &pool,
        &crate::broker::NullBroker,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        c2.id,
        &credential(),
    )
    .await
    .expect("attach still succeeds without a broker");
    assert!(!out2.verification.verified);
    assert!(out2.verification.note.is_some());
}

/// Ledger-capable and reach-capable are independent. A connection may legitimately be
/// ledger-only: events land, judgment is simply not yet possible — and it says so.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn the_two_capability_tiers_are_independently_settable(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");

    let events = vec!["pull_request".to_string(), "push".to_string()];
    let ledger_only = svc::set_webhook_events(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        c.id,
        &events,
    )
    .await
    .expect("set webhooks");
    assert!(ledger_only.is_ledger_capable(), "events land");
    assert!(
        !ledger_only.is_reach_capable(),
        "ledger-capable must NOT imply reach-capable — a ledger-only connection is legal, \
         and inert for judgment"
    );
    assert_eq!(ledger_only.webhook_events, events);

    let tools = vec!["get_pull_request".to_string()];
    let both = svc::set_tool_manifest(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        c.id,
        &tools,
    )
    .await
    .expect("set tools");
    assert!(both.is_reach_capable(), "judgment becomes possible");
    assert!(both.is_ledger_capable(), "and the webhook set survives");
    assert_eq!(
        both.tool_manifest,
        serde_json::json!(["get_pull_request"]),
        "the manifest is a JSON array of tool names — what is_reach_capable reads"
    );
}

/// The registered set MIRRORS what the remote is configured to send, so it replaces rather
/// than merges. A merge would let a stale entry outlive the webhook it names — a claimed
/// capability we do not have.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn setting_webhook_events_replaces_rather_than_merges(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");

    svc::set_webhook_events(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        c.id,
        &["push".to_string()],
    )
    .await
    .expect("first");
    let second = svc::set_webhook_events(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        c.id,
        &["pull_request".to_string()],
    )
    .await
    .expect("second");

    assert_eq!(
        second.webhook_events,
        vec!["pull_request".to_string()],
        "`push` must be gone, not merged"
    );
}

/// A revoked connection is dead: reactivation is a new provisioning, never an UPDATE. Every
/// mutator must REFUSE one rather than issue an UPDATE that matches no rows and reports success.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_revoked_connection_refuses_every_mutation(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");
    svc::revoke(
        &pool,
        c.id,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
    )
    .await
    .expect("revoke");

    assert!(matches!(
        svc::attach_credential(
            &pool,
            &granting_broker(),
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            c.id,
            &credential()
        )
        .await,
        Err(ApiError::Conflict(_))
    ));
    assert!(matches!(
        svc::set_webhook_events(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            c.id,
            &["push".to_string()]
        )
        .await,
        Err(ApiError::Conflict(_))
    ));
    assert!(matches!(
        svc::set_tool_manifest(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            c.id,
            &["t".to_string()]
        )
        .await,
        Err(ApiError::Conflict(_))
    ));

    // And nothing was written despite three attempts.
    let after = svc::get(&pool, c.id).await.expect("get");
    assert!(after.needs_credential());
    assert!(!after.is_ledger_capable());
    assert!(!after.is_reach_capable());
}

/// The mutators pass `ConnectionControlAuthority` — `MachineRegistrationAuthority`'s policy,
/// keyed on the EXISTING row's owning team. A maintainer is not an owner, and is refused as if
/// the connection did not exist.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_mere_maintainer_cannot_attach_a_credential(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let (maintainer, team) =
        seed_team_member(&pool, "conn-maint", "acme", TeamRole::Maintainer).await;

    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");

    assert!(
        matches!(
            svc::attach_credential(
                &pool,
                &granting_broker(),
                &crate::test_support::human_principal_for(&pool, maintainer.uuid()).await,
                c.id,
                &credential()
            )
            .await,
            Err(ApiError::NotFound(m)) if m == svc::CONNECTION_REFUSAL
        ),
        "a maintainer does not control the connection, so it reads as missing"
    );
    assert!(
        svc::get(&pool, c.id).await.expect("get").needs_credential(),
        "a rejected attach writes nothing"
    );
}

/// A credential naming no broker cannot be dispatched on, so it is not a credential. Reject it
/// rather than storing a row that flips `needs_credential` off while being unusable.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_empty_broker_or_connector_is_rejected(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");

    let no_broker = ConnectionCredential {
        broker: "  ".to_string(),
        ..credential()
    };
    assert!(matches!(
        svc::attach_credential(
            &pool,
            &granting_broker(),
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            c.id,
            &no_broker
        )
        .await,
        Err(ApiError::BadRequest(_))
    ));

    let no_connector = ConnectionCredential {
        connector: String::new(),
        ..credential()
    };
    assert!(matches!(
        svc::attach_credential(
            &pool,
            &granting_broker(),
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            c.id,
            &no_connector
        )
        .await,
        Err(ApiError::BadRequest(_))
    ));

    assert!(
        svc::get(&pool, c.id).await.expect("get").needs_credential(),
        "neither rejection may leave the connection looking credentialed"
    );
}

// -----------------------------------------------------------------------
// Reach grants — a TEAM's read-reach on a connection. Same connection-side
// gate as every other mutator (`ConnectionControlAuthority`, keyed on the
// connection's owning team; teamless fails closed; a refusal reads as a
// missing connection), plus the receiving-team bar, writing a
// `kb_access_grants` row.
// -----------------------------------------------------------------------

/// A bare team, so a reach grant has a real principal to point at.
async fn seed_team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar!(
        "INSERT INTO kb_teams (slug, name) VALUES ($1, $1) \
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name RETURNING id",
        slug,
    )
    .fetch_one(pool)
    .await
    .expect("seed team")
}

/// A team the caller MANAGES, so a reach grant clears the target-team bar.
///
/// Most reach tests want this rather than [`seed_team`]: since `grant_reach` requires a
/// manage-capable role on the RECEIVING team, a bare team is the *denial* fixture and this is
/// the *permission* fixture. `maintainer` deliberately — the bar is `can_manage`, not `owner`,
/// and seeding the weakest role that should pass is what proves that.
async fn seed_managed_team(pool: &PgPool, slug: &str, caller: ProfileId) -> Uuid {
    let team = seed_team(pool, slug).await;
    sqlx::query!(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
         VALUES ($1, $2, 'maintainer'::team_role) \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
        team,
        *caller,
    )
    .execute(pool)
    .await
    .expect("seed manage role on the target team");
    team
}

/// Does a read-reach grant row exist for `team_id` on `connection_id`?
async fn reach_grant_exists(pool: &PgPool, connection_id: Uuid, team_id: Uuid) -> bool {
    sqlx::query_scalar!(
        r#"SELECT EXISTS(
                 SELECT 1 FROM kb_access_grants
                  WHERE subject_table = 'kb_connections' AND subject_id = $1
                    AND principal_table = 'kb_teams' AND principal_id = $2
                    AND can_read
               ) AS "e!: bool""#,
        connection_id,
        team_id,
    )
    .fetch_one(pool)
    .await
    .expect("grant existence check")
}

/// The owner of the connection's owning team may grant a team read-reach — provided they also
/// manage the RECEIVING team. The grant lands.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_team_owner_may_grant_reach(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    let beta = seed_managed_team(&pool, "beta", owner).await;

    svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        Some("beta reviews acme CI".into()),
    )
    .await
    .expect("a team owner may grant reach on their own connection");

    assert!(
        reach_grant_exists(&pool, c.id, beta).await,
        "the read-reach grant must be visible after granting"
    );
}

/// BITE: `grant_reach`'s owner arm is decided again under the owning team's lock. An admin
/// removes the connection's owner from the owning team, uncommitted; the owner's grant of reach
/// to a team they manage blocks on the owning team's row; the removal commits. The owner is
/// refused as a stranger to the connection is, and nothing is written: no reach grant (which
/// would have kept them reading the connection through `beta`), no affirmation.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_owner_removed_while_grant_reach_waits_on_the_team_lock_grants_nothing(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    seed_team_member(&pool, "reach-co-owner", "acme", TeamRole::Owner).await;
    let (stranger, _) =
        seed_team_member(&pool, "reach-stranger", "elsewhere", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    let beta = seed_managed_team(&pool, "beta", owner).await;

    let stranger_err = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, stranger.uuid()).await,
        c.id,
        beta,
        Some("not theirs".into()),
    )
    .await
    .expect_err("a stranger to the connection grants nothing");

    let holder =
        crate::test_support::hold_member_removal(&pool, admin.uuid(), team, owner.uuid()).await;
    let (p, by, id) = (
        pool.clone(),
        crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
    );
    let contender = tokio::spawn(async move {
        svc::grant_reach(&p, &by, id, beta, Some("beta reviews acme CI".into())).await
    });
    crate::test_support::await_team_lock_waiter(&pool).await;
    holder.commit().await.unwrap();

    let err = contender
        .await
        .unwrap()
        .expect_err("an owner removed under the lock grants no reach");
    assert!(
        matches!(&err, ApiError::NotFound(m) if m == svc::CONNECTION_REFUSAL),
        "got {err:?}"
    );
    assert_eq!(
        format!("{err:?}"),
        format!("{stranger_err:?}"),
        "the removed owner is told what a stranger is told"
    );
    assert!(
        !reach_grant_exists(&pool, c.id, beta).await,
        "no reach grant was written"
    );
    assert!(
        svc::get(&pool, c.id)
            .await
            .unwrap()
            .reach_affirmed_by
            .is_none(),
        "no affirmation was stamped"
    );
}

/// A system admin may grant reach — even on a teamless connection (the admin bypass), and to
/// a team they hold NO role on. `beta` is deliberately a bare `seed_team`: the target-team
/// manage bar is exempted for admins, so this is the D5 bypass proving it survives.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_system_admin_may_grant_reach(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Teamless GitHub", None),
    )
    .await
    .expect("provision");
    let beta = seed_team(&pool, "beta").await;
    assert!(
        crate::services::team_service::role_on_team(&pool, beta, admin)
            .await
            .expect("role lookup")
            .is_none(),
        "precondition: the admin manages `beta` not at all"
    );

    svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        c.id,
        beta,
        Some("admin binds teamless reach".into()),
    )
    .await
    .expect("a system admin may grant reach");

    assert!(reach_grant_exists(&pool, c.id, beta).await);
}

/// Neither admin nor owner-of-the-owning-team is denied — read-reach is not self-service.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_non_owner_non_admin_cannot_grant_reach(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    let (outsider, _other) =
        seed_team_member(&pool, "reach-outsider", "other", TeamRole::Owner).await;

    let err = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, outsider.uuid()).await,
        c.id,
        team,
        None,
    )
    .await
    .expect_err("an owner of a different team is not authorized here");
    assert!(
        matches!(&err, ApiError::NotFound(m) if m == svc::CONNECTION_REFUSAL),
        "no control over the connection reads as a missing connection, got {err:?}"
    );
    assert!(
        !reach_grant_exists(&pool, c.id, team).await,
        "a denied grant must write nothing"
    );
}

/// The target-team bar. Owning the connection is NOT enough — the caller must also hold a
/// manage-capable role on the team RECEIVING the reach. Without this, the owner of one team
/// could bind their connection's reach to any team in the instance.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn granting_reach_to_an_unmanaged_team_is_denied(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    // A real team the owner has no relationship with whatsoever.
    let stranger_team = seed_team(&pool, "stranger").await;

    let err = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        stranger_team,
        Some("reaching somewhere I do not manage".into()),
    )
    .await
    .expect_err("owning the connection does not license binding it to an arbitrary team");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");

    assert!(
        !reach_grant_exists(&pool, c.id, stranger_team).await,
        "a denied grant must write nothing"
    );
    let reloaded = svc::get(&pool, c.id).await.expect("get");
    assert!(
        reloaded.reach_affirmed_at.is_none(),
        "a denied grant must stamp no affirmation either"
    );
}

/// Mere membership is not management — the bar is `can_manage`, so `member` is refused where
/// `maintainer` (see `seed_managed_team`) passes.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn granting_reach_to_a_team_you_merely_belong_to_is_denied(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    let beta = seed_team(&pool, "beta").await;
    sqlx::query!(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
         VALUES ($1, $2, 'member'::team_role)",
        beta,
        *owner,
    )
    .execute(&pool)
    .await
    .expect("seed mere membership");

    let err = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        Some("I am merely a member".into()),
    )
    .await
    .expect_err("member != can_manage");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
    assert!(!reach_grant_exists(&pool, c.id, beta).await);
}

/// **The gating team is an ordinary target here, deliberately** — and this test exists to make
/// that a decision rather than an oversight.
///
/// The two sibling two-sided gates both refuse it: `cogmap_service::can_bind` (a binding to the
/// gating team IS the admin-only-regime switch, `cogmap_write_requires_admin`, and the same gate serves *unbind*)
/// and `context_service::can_share` (which also gates `reassign`, a transfer of ownership that
/// `context_reassign`'s plpgsql independently forbids). `contain_target_team` has no equivalent
/// reason: a reach grant writes one read-only `kb_access_grants` row over the caller's OWN
/// connection. It flips no regime and transfers no ownership. Post-D11 the gating team confers
/// neither standing (`has_system_access` reads `kb_principal_standing`) nor admin-ness
/// (`is_system_admin` reads `kb_principal_governance`), so for this act it is just a team.
///
/// Spec §6.1, `temper-artifacts:specs/2026-07-22-scoped-authority-policy-layer-design.md`.
/// If a future change makes this red, that is a behaviour change to the asymmetry — read §6.1
/// before editing this test.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn reach_to_the_gating_team_is_allowed_for_a_non_admin(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");

    // The target IS the gating team, and the caller manages it — the exact principal the two
    // sibling gates refuse. A shared team, not `temper-system`: the root is refused to a
    // non-admin for its own reason (G14, `reach_to_the_root_is_refused_to_a_non_admin`).
    let gating = seed_managed_team(&pool, "gate", owner).await;
    sqlx::query!("UPDATE kb_system_settings SET gating_team_slug = 'gate'")
        .execute(&pool)
        .await
        .expect("point the gating slug at the shared team");

    // Both halves of "the exact principal the siblings refuse" must be true, or this test
    // pins nothing. A fixture where the gating slug went unconfigured, or where the caller
    // happened to be an admin, would pass for the wrong reason.
    let is_gating = sqlx::query_scalar!(
        r#"SELECT EXISTS( SELECT 1 FROM kb_teams t
                                JOIN kb_system_settings s ON t.slug = s.gating_team_slug
                               WHERE t.id = $1 ) AS "e!: bool""#,
        gating,
    )
    .fetch_one(&pool)
    .await
    .expect("gating check");
    assert!(is_gating, "fixture must actually target the gating team");

    let is_admin = sqlx::query_scalar!(r#"SELECT is_system_admin($1) AS "a!: bool""#, *owner)
        .fetch_one(&pool)
        .await
        .expect("admin check");
    assert!(
        !is_admin,
        "the caller must be a non-admin, or the D5 bypass carries this"
    );

    svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        gating,
        Some("the root team reviews acme CI".into()),
    )
    .await
    .expect("a reach grant to the gating team is not an escalation — spec §6.1");

    assert!(
        reach_grant_exists(&pool, c.id, gating).await,
        "the grant must land; the gating team is an ordinary principal for reach"
    );
}

/// Team DAG review G14: the root is not an ordinary target. `temper-system` is every personal
/// team's parent, so a reach grant onto it hands the connection's data to every profile — the
/// grant the grant door's recipient bar refuses a delegate. Refused to a non-admin who manages
/// the root, with the gating team pointed elsewhere so the gating exemption is not in play.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn reach_to_the_root_is_refused_to_a_non_admin(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    let _gate = seed_team(&pool, "gate").await;
    sqlx::query!("UPDATE kb_system_settings SET gating_team_slug = 'gate'")
        .execute(&pool)
        .await
        .expect("point the gating slug away from the root");
    let root = seed_managed_team(&pool, "temper-system", owner).await;

    let err = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        root,
        None,
    )
    .await
    .expect_err("a non-admin may not grant reach to the root");
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    assert!(!reach_grant_exists(&pool, c.id, root).await);
}

/// `kb_access_grants.principal_id` carries NO foreign key, so an unvalidated `team_id` would
/// persist a dangling grant row pointing at nothing. A system admin skips the manage bar, so
/// the existence check is the only thing standing between an admin's typo and that row.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn granting_reach_to_a_nonexistent_team_writes_no_dangling_row(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");
    let ghost = Uuid::now_v7(); // no kb_teams row will ever carry this

    let err = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        c.id,
        ghost,
        Some("typo".into()),
    )
    .await
    .expect_err("a team that does not exist cannot receive reach");
    assert!(matches!(err, ApiError::NotFound(_)), "got {err:?}");

    assert!(
        !reach_grant_exists(&pool, c.id, ghost).await,
        "no dangling grant row may be written"
    );
}

/// A teamless connection fails closed for a non-admin: "no team to check" ≠ "nothing to deny".
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_teamless_connection_grant_reach_fails_closed(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Teamless GitHub", None),
    )
    .await
    .expect("provision");
    let (outsider, _other) =
        seed_team_member(&pool, "reach-outsider", "other", TeamRole::Owner).await;
    let beta = seed_team(&pool, "beta").await;

    let err = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, outsider.uuid()).await,
        c.id,
        beta,
        None,
    )
    .await
    .expect_err("teamless is admin-only");
    assert!(
        matches!(&err, ApiError::NotFound(m) if m == svc::CONNECTION_REFUSAL),
        "no control over the connection reads as a missing connection, got {err:?}"
    );
}

/// `revoke_reach` removes a previously granted row, gated the same way.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn revoke_reach_removes_a_previously_granted_row(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    let beta = seed_managed_team(&pool, "beta", owner).await;

    svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        Some("beta reviews acme CI".into()),
    )
    .await
    .expect("grant");
    assert!(reach_grant_exists(&pool, c.id, beta).await, "granted");

    svc::revoke_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
    )
    .await
    .expect("revoke");
    assert!(
        !reach_grant_exists(&pool, c.id, beta).await,
        "the grant must be gone after revoke"
    );
}

/// **The grant/revoke asymmetry, asserted so nobody "fixes" it into symmetry.**
///
/// The caller grants while managing `beta`, then LOSES that role. Revoke must still work: they
/// still control the connection. Were revoke gated on the target team the way grant is, this
/// grant would now be permanently unrevokable — access stranded on a connection whose own
/// owner can see the grant and cannot withdraw it. De-escalation is never harder than
/// escalation.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn revoke_reach_survives_losing_the_target_team_role(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    let beta = seed_managed_team(&pool, "beta", owner).await;

    svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        Some("beta reviews acme CI".into()),
    )
    .await
    .expect("granted while managing beta");
    assert!(reach_grant_exists(&pool, c.id, beta).await, "granted");

    // The role goes away — a reorg, a handover, a mistake. The grant remains.
    sqlx::query!(
        "DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
        beta,
        *owner,
    )
    .execute(&pool)
    .await
    .expect("drop the target-team role");
    assert!(
        svc::grant_reach(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
            c.id,
            beta,
            Some("again".into())
        )
        .await
        .is_err(),
        "precondition: re-GRANTING is now denied — the role really is gone"
    );

    svc::revoke_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
    )
    .await
    .expect("revoke must not require the target-team role grant demands");
    assert!(
        !reach_grant_exists(&pool, c.id, beta).await,
        "the stranded grant must be withdrawable by the connection's owner"
    );
}

// -----------------------------------------------------------------------
// Excess-reach affirmation (B3 beat 2). Granting a team reach on a
// connection that DECLARES reach must be affirmed as intentional or it
// FAILS — the trigger is purely `declares_reach()`, never a computed
// comparison against the team. Affirming records who/when/why; it does not
// narrow the remote reach.
// -----------------------------------------------------------------------

/// Declared reach, no affirmation: the grant is refused, and nothing is written — no grant row,
/// and the affirmation stamp stays NULL.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn grant_on_a_reach_declaring_connection_without_affirmation_is_refused(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    assert!(c.declares_reach(), "req declares reach");
    let beta = seed_managed_team(&pool, "beta", owner).await;

    let err = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        None,
    )
    .await
    .expect_err("a reach-declaring grant must be affirmed");
    assert!(matches!(err, ApiError::Conflict(_)), "got {err:?}");

    assert!(
        !reach_grant_exists(&pool, c.id, beta).await,
        "a refused grant must write no grant row"
    );
    let reloaded = svc::get(&pool, c.id).await.expect("get");
    assert!(
        reloaded.reach_affirmed_at.is_none(),
        "a refused grant must stamp no affirmation"
    );
}

/// Declared reach, affirmed: the affirmation records who/when/why AND the grant lands. The
/// returned connection (and a fresh get) carry the populated affirmation fields.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn affirming_a_reach_declaring_grant_records_who_when_why_and_writes_the_grant(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    let beta = seed_managed_team(&pool, "beta", owner).await;

    let out = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        Some("beta reviews acme CI".into()),
    )
    .await
    .expect("an affirmed grant lands");

    assert!(
        reach_grant_exists(&pool, c.id, beta).await,
        "the grant must land alongside the affirmation"
    );
    // The returned struct is re-read post-UPDATE, so it carries the stamp.
    assert_eq!(out.reach_affirmed_by, Some(*owner), "who");
    assert_eq!(
        out.reach_affirmation.as_deref(),
        Some("beta reviews acme CI"),
        "why"
    );
    assert!(out.reach_affirmed_at.is_some(), "when");

    let reloaded = svc::get(&pool, c.id).await.expect("get");
    assert_eq!(reloaded.reach_affirmed_by, Some(*owner));
    assert_eq!(
        reloaded.reach_affirmation.as_deref(),
        Some("beta reviews acme CI")
    );
    assert!(reloaded.reach_affirmed_at.is_some());
}

/// A connection that declares NO reach grants without any affirmation — the affirmation stamp
/// stays NULL because no affirmation was required or made.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_connection_that_declares_no_reach_grants_without_affirmation(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req_no_reach("Bare GitHub", Some(team)),
    )
    .await
    .expect("provision");
    assert!(!c.declares_reach(), "req_no_reach declares no reach");
    let beta = seed_managed_team(&pool, "beta", owner).await;

    svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        None,
    )
    .await
    .expect("a connection with no declared reach grants freely");

    assert!(
        reach_grant_exists(&pool, c.id, beta).await,
        "the grant lands"
    );
    let reloaded = svc::get(&pool, c.id).await.expect("get");
    assert!(
        reloaded.reach_affirmed_at.is_none(),
        "no affirmation is required or recorded"
    );
}

/// Affirming a connection that declares NO reach is refused — the flag is inapplicable. Honest
/// over lenient: it is not silently ignored, and nothing is written.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn affirming_a_connection_that_declares_no_reach_is_refused(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req_no_reach("Bare GitHub", Some(team)),
    )
    .await
    .expect("provision");
    let beta = seed_managed_team(&pool, "beta", owner).await;

    let err = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        Some("but I insist".into()),
    )
    .await
    .expect_err("--affirm-reach is inapplicable when no reach is declared");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");

    assert!(
        !reach_grant_exists(&pool, c.id, beta).await,
        "a refused grant must write nothing"
    );
}

/// Authorization precedes affirmation: a non-owner/non-admin caller passing `affirm_reach` on a
/// reach-declaring connection is still refused — as a missing connection, since it does not
/// control this one (the gate runs FIRST) — and nothing is written.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn authorization_precedes_affirmation(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    let (outsider, _other) =
        seed_team_member(&pool, "reach-outsider", "other", TeamRole::Owner).await;
    let beta = seed_team(&pool, "beta").await;

    let err = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, outsider.uuid()).await,
        c.id,
        beta,
        Some("I insist".into()),
    )
    .await
    .expect_err("auth runs before affirmation");
    assert!(
        matches!(&err, ApiError::NotFound(m) if m == svc::CONNECTION_REFUSAL),
        "no control over the connection reads as a missing connection, got {err:?}"
    );

    assert!(
        !reach_grant_exists(&pool, c.id, beta).await,
        "an auth-denied grant writes no grant row"
    );
    let reloaded = svc::get(&pool, c.id).await.expect("get");
    assert!(
        reloaded.reach_affirmed_at.is_none(),
        "an auth-denied grant stamps no affirmation"
    );
}

/// Re-affirming overwrites the prior affirmation (last-writer). The second rationale and
/// affirmer win.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn re_affirming_overwrites_the_prior_affirmation(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    let beta = seed_managed_team(&pool, "beta", owner).await;
    let gamma = seed_managed_team(&pool, "gamma", owner).await;

    svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        Some("first reason".into()),
    )
    .await
    .expect("first affirmation");
    let second = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        gamma,
        Some("second reason".into()),
    )
    .await
    .expect("second affirmation");

    assert_eq!(
        second.reach_affirmation.as_deref(),
        Some("second reason"),
        "the last writer's rationale wins"
    );
    assert_eq!(second.reach_affirmed_by, Some(*owner));
}

// -----------------------------------------------------------------------
// B3 remainder — the drift check's observed-vs-declared gap wired into the
// grant path. The gate previously fired on `declares_reach()` alone (a static
// declaration). With `observed_reach` persisted at attach, the gate fires when
// the connection declares reach AND the mint's witness does not confirm the
// declaration: observed is missing (unwitnessed), or observed scope is broader
// than declared (the commensurable, remote-domain gap). Remote-vs-temper scope
// stays incommensurable and uncomputed.
// -----------------------------------------------------------------------

/// A broker that mints with `repository_selection: "selected"` — the observed
/// reach agrees with a specific `reach_covers` declaration (no remote-domain
/// gap). Used by the "gate does NOT fire" tests below.
fn narrow_granting_broker() -> crate::broker::FakeBroker {
    crate::broker::FakeBroker::granting(serde_json::json!({
        "repository_selection": "selected",
        "permissions": { "contents": "read", "metadata": "read", "pull_requests": "read" }
    }))
}

/// The attach persists `observed_reach` onto the connection row — the witness
/// must outlive the attach response so the grant path can consult it.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn attach_persists_observed_reach_onto_the_connection_row(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");
    assert!(
        c.observed_reach.is_none(),
        "born with no observed reach — the credential has not been minted"
    );

    let out = svc::attach_credential(
        &pool,
        &granting_broker(),
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        c.id,
        &credential(),
    )
    .await
    .expect("attach");

    // The response carries the observed reach (B4, unchanged)…
    assert_eq!(
        out.verification
            .observed_reach
            .as_ref()
            .and_then(|r| r.get("repository_selection"))
            .and_then(|v| v.as_str()),
        Some("all"),
    );
    // …and it is now PERSISTED on the row, readable by a fresh get.
    let reloaded = svc::get(&pool, c.id).await.expect("get");
    assert_eq!(
        reloaded
            .observed_reach
            .as_ref()
            .and_then(|r| r.get("repository_selection"))
            .and_then(|v| v.as_str()),
        Some("all"),
        "observed_reach must persist on the connection row, not just the response"
    );
}

/// An unverified attach (no broker configured) persists `observed_reach = NULL`
/// — the witness is honestly absent, not silently defaulted. The grant gate
/// treats NULL as "unwitnessed" and fires (the declaration is unconfirmed).
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_unverified_attach_persists_null_observed_reach(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("Acme GitHub", None),
    )
    .await
    .expect("provision");

    let null_broker = crate::broker::NullBroker;
    let out = svc::attach_credential(
        &pool,
        &null_broker,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        c.id,
        &credential(),
    )
    .await
    .expect("an unverified attach still persists the credential");
    assert!(!out.verification.verified, "no broker ⇒ not verified");
    assert!(
        out.verification.observed_reach.is_none(),
        "no observed reach"
    );

    let reloaded = svc::get(&pool, c.id).await.expect("get");
    assert!(
        reloaded.observed_reach.is_none(),
        "an unverified attach persists NULL observed_reach, honestly absent"
    );
}

/// A reach-declaring connection whose mint witnessed `repository_selection:
/// "all"` (broader than `reach_covers: "acme/temper"`) STILL requires
/// affirmation — the remote-domain gap the mint detected sharpens the gate.
/// This is the C8 remainder: excess reach detected by mint, not merely declared.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn observed_reach_broader_than_declared_still_requires_affirmation(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    // Attach with the default granting broker (repository_selection: "all").
    svc::attach_credential(
        &pool,
        &granting_broker(),
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        &credential(),
    )
    .await
    .expect("attach");
    let beta = seed_managed_team(&pool, "beta", owner).await;

    let err = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        None,
    )
    .await
    .expect_err("observed all-vs-declared-specific gap must be affirmed");
    assert!(matches!(err, ApiError::Conflict(_)), "got {err:?}");
    assert!(
        !reach_grant_exists(&pool, c.id, beta).await,
        "writes nothing"
    );
}

/// The gap, affirmed, records the affirmation AND lands the grant — same
/// atomic stamp-then-grant as a merely-declared reach.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn affirming_the_observed_gap_records_and_grants(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    svc::attach_credential(
        &pool,
        &granting_broker(),
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        &credential(),
    )
    .await
    .expect("attach (observed all)");
    let beta = seed_managed_team(&pool, "beta", owner).await;

    let out = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        Some("beta reads the whole org deliberately".into()),
    )
    .await
    .expect("affirmed grant lands");

    assert!(reach_grant_exists(&pool, c.id, beta).await, "grant lands");
    assert_eq!(
        out.reach_affirmation.as_deref(),
        Some("beta reads the whole org deliberately"),
    );
    assert!(out.reach_affirmed_at.is_some(), "stamp recorded");
}

/// A reach-declaring connection whose mint witnessed `repository_selection:
/// "selected"` (agrees with `reach_covers: "acme/temper"`) grants WITHOUT
/// affirmation — the mint witnessed no remote-domain gap, so the declaration
/// was honest and there is nothing to acknowledge. This is the gate sharpened:
/// `declares_reach()` alone no longer fires it; the observed-vs-declared
/// comparison does.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn observed_reach_agreeing_with_declared_grants_without_affirmation(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    // Attach with the NARROW broker (repository_selection: "selected").
    svc::attach_credential(
        &pool,
        &narrow_granting_broker(),
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        &credential(),
    )
    .await
    .expect("attach (observed selected)");

    let reloaded = svc::get(&pool, c.id).await.expect("get");
    assert_eq!(
        reloaded
            .observed_reach
            .as_ref()
            .and_then(|r| r.get("repository_selection"))
            .and_then(|v| v.as_str()),
        Some("selected"),
        "precondition: the narrow broker's observed reach is persisted"
    );

    let beta = seed_managed_team(&pool, "beta", owner).await;
    // No affirmation: the grant proceeds because the mint witnessed no gap.
    svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        None,
    )
    .await
    .expect("a witnessed, agreeing reach grants without affirmation");
    assert!(
        reach_grant_exists(&pool, c.id, beta).await,
        "the grant lands"
    );

    let reloaded = svc::get(&pool, c.id).await.expect("get");
    assert!(
        reloaded.reach_affirmed_at.is_none(),
        "no affirmation is recorded when the gate does not fire"
    );
}

/// Affirming a reach-declaring connection whose mint witnessed NO gap is
/// refused — the flag is inapplicable (there is no gap to acknowledge). Honest
/// over lenient, same as the no-declared-reach case: do not silently accept a
/// spurious affirmation.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn affirming_when_observed_reach_agrees_with_declared_is_refused(pool: PgPool) {
    let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
    let c = svc::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        &req("Acme GitHub", Some(team)),
    )
    .await
    .expect("provision");
    svc::attach_credential(
        &pool,
        &narrow_granting_broker(),
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        &credential(),
    )
    .await
    .expect("attach (observed selected)");
    let beta = seed_managed_team(&pool, "beta", owner).await;

    let err = svc::grant_reach(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner.uuid()).await,
        c.id,
        beta,
        Some("but I insist".into()),
    )
    .await
    .expect_err("affirming a no-gap reach is inapplicable");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    assert!(
        !reach_grant_exists(&pool, c.id, beta).await,
        "writes nothing"
    );
}

/// The existence oracle, closed: every per-row act refuses a caller who does not control the
/// connection with the **same** error a missing id gets — variant and message — so probing ids
/// learns nothing. A `Forbidden` on any of these would mean the row exists.
///
/// Run against a live connection AND a revoked one: the revoked target is what pins the order
/// of `authorize_live`. Move its `409 revoked` check above the gate and a revoked connection
/// answers differently from a missing one — which this test then catches.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_denied_connection_is_indistinguishable_from_a_missing_one(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let admin_auth = crate::test_support::human_principal_for(&pool, admin.uuid()).await;
    let (_owner, team) = seed_team_member(&pool, "conn-owner", "acme", TeamRole::Owner).await;
    let live = svc::provision(&pool, &admin_auth, &req("Acme GitHub", Some(team)))
        .await
        .expect("provision live");
    let revoked = svc::provision(&pool, &admin_auth, &req("Acme Linear", Some(team)))
        .await
        .expect("provision revoked");
    svc::revoke(&pool, revoked.id, &admin_auth)
        .await
        .expect("revoke as admin");

    let (outsider, _other) = seed_team_member(&pool, "conn-prober", "other", TeamRole::Owner).await;
    let prober = crate::test_support::human_principal_for(&pool, outsider.uuid()).await;
    let missing = Uuid::now_v7();

    for (state, target) in [("live", live.id), ("revoked", revoked.id)] {
        for id in [target, missing] {
            let refusals: [(&str, ApiError); 7] = [
                (
                    "get",
                    svc::get_for_caller(&pool, &prober, id)
                        .await
                        .expect_err("get"),
                ),
                (
                    "revoke",
                    svc::revoke(&pool, id, &prober).await.expect_err("revoke"),
                ),
                (
                    "credential",
                    svc::attach_credential(&pool, &granting_broker(), &prober, id, &credential())
                        .await
                        .expect_err("credential"),
                ),
                (
                    "webhook events",
                    svc::set_webhook_events(&pool, &prober, id, &["push".into()])
                        .await
                        .expect_err("webhook events"),
                ),
                (
                    "tool manifest",
                    svc::set_tool_manifest(&pool, &prober, id, &["read_pr".into()])
                        .await
                        .expect_err("tool manifest"),
                ),
                (
                    "grant reach",
                    svc::grant_reach(&pool, &prober, id, team, None)
                        .await
                        .expect_err("grant reach"),
                ),
                (
                    "revoke reach",
                    svc::revoke_reach(&pool, &prober, id, team)
                        .await
                        .expect_err("revoke reach"),
                ),
            ];
            for (act, err) in refusals {
                assert!(
                    matches!(&err, ApiError::NotFound(m) if m == svc::CONNECTION_REFUSAL),
                    "{act} on a {state} target (or a missing id): must refuse as a missing \
                     connection, got {err:?}"
                );
            }
        }
    }

    let reloaded = svc::get(&pool, live.id).await.expect("get");
    assert!(
        reloaded.revoked_at.is_none(),
        "the denied revoke wrote nothing"
    );
    assert!(
        reloaded.needs_credential(),
        "the denied attach wrote nothing"
    );
    assert!(
        reloaded.webhook_events.is_empty(),
        "the denied webhook write wrote nothing"
    );
    assert!(
        !reloaded.is_reach_capable(),
        "the denied manifest write wrote nothing"
    );
}
