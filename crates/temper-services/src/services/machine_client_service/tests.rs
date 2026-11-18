use sqlx::PgPool;
use uuid::Uuid;

use crate::authz::Principal;
use crate::services::machine_client_service as svc;
use temper_core::types::ids::ProfileId;

const BACKFILL: &str =
    include_str!("../../../../../migrations/20260711000011_backfill_machine_clients.sql");

/// Seed a caller who is genuinely a system admin.
///
/// B2 D5 authorizes the lifecycle inside the service against the row's owning team, and the
/// rows these tests seed are teamless — which is admin-only (D2). So the caller can no longer
/// be a bare profile. Under D11 admin-ness is a `kb_principal_governance` grant plus an
/// `approved` `kb_principal_standing`, not gating-team ownership; the gating-team upsert below
/// is retained only because `temper-system` already exists in a migrated database.
async fn seed_admin(pool: &PgPool, handle: &str) -> ProfileId {
    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, $2, $2, NULL, '{}')",
        id,
        handle,
    )
    .execute(pool)
    .await
    .expect("seed admin profile");

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

    // What confers admin-ness now: approved standing (front door) + a governance grant.
    crate::test_support::approved_admin(pool, id).await;

    ProfileId::from(id)
}

/// Seed a profile plus an `auth0-m2m` auth link, as prod carries for the steward.
async fn seed_agent_link(pool: &PgPool, client_id: &str) -> Uuid {
    let profile_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, $2, $3, NULL, '{}')",
        profile_id,
        format!("agent-{client_id}"),
        format!("agent-{client_id}"),
    )
    .execute(pool)
    .await
    .expect("seed profile");

    sqlx::query!(
        "INSERT INTO kb_profile_auth_links \
           (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at) \
         VALUES ($1, $2, 'auth0-m2m', $3, NULL, false, true, now())",
        Uuid::now_v7(),
        profile_id,
        client_id,
    )
    .execute(pool)
    .await
    .expect("seed auth link");

    profile_id
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn backfill_registers_existing_m2m_links_and_is_idempotent(pool: PgPool) {
    let profile_id = seed_agent_link(&pool, "steward-client-1").await;

    sqlx::raw_sql(BACKFILL)
        .execute(&pool)
        .await
        .expect("backfill runs");

    let row = sqlx::query!(
        "SELECT profile_id, registered_by_profile_id, label, issuer, revoked_at \
           FROM kb_machine_clients WHERE client_id = $1",
        "steward-client-1",
    )
    .fetch_one(&pool)
    .await
    .expect("backfilled row exists");

    assert_eq!(row.profile_id, profile_id);
    assert_eq!(
        row.registered_by_profile_id, profile_id,
        "backfilled rows are self-registered: no human authorized them (D13)"
    );
    assert!(row.label.starts_with("backfilled: "));
    assert_eq!(row.issuer, "auth0-m2m");
    assert!(row.revoked_at.is_none());

    // Re-running is a no-op, not a duplicate-key error.
    sqlx::raw_sql(BACKFILL)
        .execute(&pool)
        .await
        .expect("backfill is idempotent");
    let count = sqlx::query_scalar!("SELECT count(*) FROM kb_machine_clients")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, Some(1));
}

/// Register `client_id` against a freshly seeded agent profile.
async fn seed_registered(pool: &PgPool, client_id: &str) -> Uuid {
    let profile_id = seed_agent_link(pool, client_id).await;
    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_machine_clients (id, client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, $2, 'test', $3, $3)",
        id,
        client_id,
        profile_id,
    )
    .execute(pool)
    .await
    .expect("seed machine client");
    id
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn lookup_finds_registered_and_misses_unregistered(pool: PgPool) {
    seed_registered(&pool, "known").await;

    let hit = svc::lookup_by_client_id(&pool, "known")
        .await
        .expect("lookup");
    assert!(hit.is_some(), "registered client resolves");
    assert_eq!(hit.expect("some").client_id, "known");

    let miss = svc::lookup_by_client_id(&pool, "never-registered")
        .await
        .expect("lookup");
    assert!(miss.is_none(), "unregistered client must not resolve");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn touch_last_seen_is_coarse(pool: PgPool) {
    let id = seed_registered(&pool, "coarse").await;

    // First touch writes (last_seen_at was NULL).
    assert!(svc::touch_last_seen(&pool, id).await.expect("touch 1"));

    // Second touch, immediately after, does NOT write: the row is inside the
    // five-minute window. This is what keeps authentication read-only (D9).
    assert!(
        !svc::touch_last_seen(&pool, id).await.expect("touch 2"),
        "two authentications inside five minutes must produce one write"
    );

    // Age the row past the window; the next touch writes again.
    sqlx::query!(
        "UPDATE kb_machine_clients SET last_seen_at = now() - interval '6 minutes' WHERE id = $1",
        id,
    )
    .execute(&pool)
    .await
    .expect("age row");
    assert!(svc::touch_last_seen(&pool, id).await.expect("touch 3"));
}

/// Seed a temper-issued client with a known secret hash. Returns the machine_client id.
async fn seed_temper_issued(pool: &PgPool, client_id: &str, secret: &str) -> Uuid {
    let profile_id = seed_agent_link(pool, client_id).await;
    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_machine_clients \
           (id, client_id, issuer, label, profile_id, registered_by_profile_id, secret_hash) \
         VALUES ($1, $2, 'temper', 'test', $3, $3, $4)",
        id,
        client_id,
        profile_id,
        crate::auth::secret::sha256_hex(secret),
    )
    .execute(pool)
    .await
    .expect("seed temper-issued");
    id
}

/// BITE: `rotate_secret`'s owner arm is decided again under the owning team's lock. An admin
/// removes the machine's team owner from the team, uncommitted; the owner's rotation blocks on
/// the team row; the removal commits. The owner is refused as a stranger to the team is (the
/// missing-id sentence), and no secret is minted: the stored hash, and who rotated it, are as
/// they were. Before the fix the pool check's verdict stood and the removed owner walked away
/// with a live secret.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_owner_removed_while_rotation_waits_on_the_team_lock_mints_no_secret(pool: PgPool) {
    let admin = seed_admin(&pool, "rot-remover").await;
    // The removal is a recorded act; its actor needs the `<handle>@web` emitter it authors from.
    let mut conn = pool.acquire().await.unwrap();
    crate::services::profile_service::provision_profile_entities(
        &mut conn,
        admin.uuid(),
        "rot-remover",
    )
    .await
    .expect("remover emitters");
    drop(conn);
    let id = seed_temper_issued(&pool, "tmpr_rot_owner", "old-secret").await;
    let person = |handle: &'static str| {
        let pool = pool.clone();
        async move {
            let id = Uuid::now_v7();
            sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)")
                .bind(id)
                .bind(handle)
                .execute(&pool)
                .await
                .expect("seed person");
            id
        }
    };
    let (owner, co_owner, stranger) = (
        person("rot-owner").await,
        person("rot-co-owner").await,
        person("rot-stranger").await,
    );
    let team: Uuid =
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ('rot', 'rot') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    for who in [owner, co_owner] {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')",
        )
        .bind(team)
        .bind(who)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("UPDATE kb_machine_clients SET team_id = $2 WHERE id = $1")
        .bind(id)
        .bind(team)
        .execute(&pool)
        .await
        .unwrap();
    let mode = crate::auth_config::AuthMode::TemperAs;
    let stranger_err = svc::rotate_secret(
        &pool,
        &crate::test_support::human_principal_for(&pool, stranger).await,
        id,
        0,
        mode,
    )
    .await
    .expect_err("a stranger to the team rotates nothing");

    let holder = crate::test_support::hold_member_removal(&pool, admin.uuid(), team, owner).await;
    let (p, by) = (
        pool.clone(),
        crate::test_support::human_principal_for(&pool, owner).await,
    );
    let contender = tokio::spawn(async move { svc::rotate_secret(&p, &by, id, 0, mode).await });
    crate::test_support::await_team_lock_waiter(&pool).await;
    holder.commit().await.unwrap();

    let err = contender
        .await
        .unwrap()
        .expect_err("an owner removed under the lock mints no secret");
    assert!(
        matches!(&err, crate::error::ApiError::NotFound(m) if m == svc::MACHINE_CLIENT_REFUSAL),
        "got {err:?}"
    );
    assert_eq!(
        format!("{err:?}"),
        format!("{stranger_err:?}"),
        "the removed owner is told what a stranger is told"
    );
    let row: (Option<String>, Option<Uuid>) = sqlx::query_as(
        "SELECT secret_hash, secret_rotated_by_profile_id FROM kb_machine_clients WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row,
        (Some(crate::auth::secret::sha256_hex("old-secret")), None),
        "the secret is the one it was, and nobody rotated it"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn rotate_secret_moves_current_to_previous_with_expiry(pool: PgPool) {
    let admin = seed_admin(&pool, "rot-admin").await;
    let id = seed_temper_issued(&pool, "tmpr_rot", "old-secret").await;

    let cred = svc::rotate_secret(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        id,
        3600,
        crate::auth_config::AuthMode::ExternalIdp,
    )
    .await
    .expect("rotate");
    assert!(
        cred.dormant,
        "under an external IdP a rotated temper credential is still dormant"
    );

    // A fresh plaintext is returned and its hash is the new current.
    let row = sqlx::query!(
        "SELECT secret_hash, secret_hash_previous, secret_previous_expires_at, secret_rotated_at \
           FROM kb_machine_clients WHERE id = $1",
        id,
    )
    .fetch_one(&pool)
    .await
    .expect("row");
    assert_eq!(
        row.secret_hash.as_deref(),
        Some(crate::auth::secret::sha256_hex(&cred.client_secret).as_str()),
        "current is the new secret"
    );
    assert_eq!(
        row.secret_hash_previous.as_deref(),
        Some(crate::auth::secret::sha256_hex("old-secret").as_str()),
        "previous is the old secret"
    );
    // The grace expiry is now()+grace, computed by make_interval — assert the math, not just
    // presence (a wrong unit or an `as f64` surprise would pass an is_some() check).
    let expiry = row
        .secret_previous_expires_at
        .expect("previous has a grace expiry");
    let delta = (expiry - chrono::Utc::now()).num_seconds();
    assert!(
        (3540..=3660).contains(&delta),
        "previous expiry is ~now()+3600s, got {delta}s"
    );
    assert!(row.secret_rotated_at.is_some(), "rotation is stamped");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn rotate_secret_rejects_out_of_range_grace(pool: PgPool) {
    let admin = seed_admin(&pool, "grace-admin").await;
    let id = seed_temper_issued(&pool, "tmpr_grace", "s").await;
    assert!(
        matches!(
            svc::rotate_secret(
                &pool,
                &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
                id,
                -1,
                crate::auth_config::AuthMode::ExternalIdp
            )
            .await
            .expect_err("negative grace"),
            crate::error::ApiError::BadRequest(_)
        ),
        "a negative grace is rejected"
    );
    assert!(
        matches!(
            svc::rotate_secret(
                &pool,
                &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
                id,
                999_999_999,
                crate::auth_config::AuthMode::ExternalIdp
            )
            .await
            .expect_err("excessive grace"),
            crate::error::ApiError::BadRequest(_)
        ),
        "a grace past the 7-day cap is rejected (keeps an old secret alive too long)"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn rotate_secret_rejects_a_non_temper_issued_client(pool: PgPool) {
    let admin = seed_admin(&pool, "issuer-admin").await;
    // A plain auth0-m2m registration (issuer default), no secret.
    let id = seed_registered(&pool, "auth0-client").await;

    let err = svc::rotate_secret(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        id,
        3600,
        crate::auth_config::AuthMode::ExternalIdp,
    )
    .await
    .expect_err("must reject");
    assert!(
        matches!(err, crate::error::ApiError::BadRequest(_)),
        "auth0-m2m secrets are managed by the IdP, not temper; got {err:?}"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn revoke_marks_dead_and_list_hides_by_default(pool: PgPool) {
    let id = seed_registered(&pool, "doomed").await;
    let admin = seed_admin(&pool, "admin-actor").await;

    let revoked = svc::revoke(
        &pool,
        id,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
    )
    .await
    .expect("revoke");
    assert!(revoked.revoked_at.is_some());
    assert_eq!(revoked.revoked_by_profile_id, Some(*admin));

    let active = svc::list(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        false,
    )
    .await
    .expect("list active");
    assert!(active.iter().all(|c| c.client_id != "doomed"));

    let all = svc::list(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        true,
    )
    .await
    .expect("list all");
    assert!(all.iter().any(|c| c.client_id == "doomed"));
}

/// A machine client with genuine reach — a read grant and a team membership, the two things D11
/// says a revoke must NOT touch. Returns the machine-client row id (what `revoke` takes), the
/// machine's profile id, and the acting admin.
struct MachineReach {
    machine_id: Uuid,
    machine_profile: Uuid,
    admin: ProfileId,
}

async fn seed_machine_with_reach(pool: &PgPool) -> MachineReach {
    let admin = seed_admin(pool, "revoke-standing-admin").await;
    let team: Uuid = sqlx::query_scalar!("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(pool)
        .await
        .expect("gating team seeded by seed_admin");
    let machine_id = seed_registered(pool, "reachful-machine").await;
    let machine_profile: Uuid = sqlx::query_scalar!(
        "SELECT profile_id FROM kb_machine_clients WHERE id = $1",
        machine_id
    )
    .fetch_one(pool)
    .await
    .expect("machine profile");

    // subject_table is CHECK-constrained to the grantable object kinds; subject_id has no FK, so
    // a synthetic context id stands in for "some object this machine can read".
    sqlx::query!(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
        Uuid::now_v7(),
        machine_profile,
        *admin,
    )
    .execute(pool)
    .await
    .expect("grant reach");

    sqlx::query!(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
         VALUES ($1, $2, 'watcher'::team_role) \
         ON CONFLICT (team_id, profile_id) DO NOTHING",
        team,
        machine_profile,
    )
    .execute(pool)
    .await
    .expect("member reach");

    MachineReach {
        machine_id,
        machine_profile,
        admin,
    }
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_admins_revoke_also_revokes_standing_but_leaves_grants(pool: PgPool) {
    // D17 — one revocation fact, one place. `revoked_at` becomes purely an authentication
    // detail; standing tells the admission story; the two cannot drift because an admin's
    // revocation of a machine's last live credential drives both. A team owner's revoke drives
    // only the credential (`a_team_owners_revoke_kills_the_credential_and_leaves_standing`).
    let f = seed_machine_with_reach(&pool).await;
    crate::test_support::approve(&pool, f.machine_profile).await;

    let grants_before: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_access_grants WHERE principal_id = $1",
        f.machine_profile
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(0);
    let members_before: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_team_members WHERE profile_id = $1",
        f.machine_profile
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(0);
    assert!(
        grants_before > 0 && members_before > 0,
        "fixture must have reach to preserve"
    );

    svc::revoke(
        &pool,
        f.machine_id,
        &crate::test_support::human_principal_for(&pool, f.admin.uuid()).await,
    )
    .await
    .unwrap();

    let state: String = sqlx::query_scalar!(
        "SELECT state FROM kb_principal_standing WHERE profile_id = $1",
        f.machine_profile
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        state, "revoked",
        "one revocation fact, not two that can drift"
    );

    // THE PRIOR INTENT, PROVEN NOT ASSUMED. Revocation deliberately leaves grants and
    // memberships so a rebind cannot silently resurrect them (D11). Revoke on standing denies
    // admission, which sits ABOVE grants and does not touch them.
    let grants_after: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_access_grants WHERE principal_id = $1",
        f.machine_profile
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(0);
    let members_after: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_team_members WHERE profile_id = $1",
        f.machine_profile
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(0);
    assert_eq!(grants_after, grants_before, "D11's intent survives D17");
    assert_eq!(members_after, members_before, "D11's intent survives D17");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn revoking_an_unapproved_machine_is_not_an_error(pool: PgPool) {
    // Revoke is illegal from `Denied` (§6 — you cannot revoke what was never granted), but a
    // credential revocation must still succeed. The standing fire is load-guarded to `Approved`
    // in exactly this cell, so the credential revocation the operator asked for never fails.
    let f = seed_machine_with_reach(&pool).await; // born Denied under D11, never approved

    svc::revoke(
        &pool,
        f.machine_id,
        &crate::test_support::human_principal_for(&pool, f.admin.uuid()).await,
    )
    .await
    .expect("credential revocation must succeed even with nothing to revoke on standing");

    // The load-first guard fired nothing (this machine was never `Approved`), so no `revoked`
    // row was written — the credential revocation the operator asked for still succeeded.
    let state: Option<String> = sqlx::query_scalar!(
        "SELECT state FROM kb_principal_standing WHERE profile_id = $1",
        f.machine_profile
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_ne!(
        state.as_deref(),
        Some("revoked"),
        "an unapproved machine has nothing to revoke on the admission axis"
    );
}

/// A team the given profile owns. The machine-client control gate keys on the owner of the
/// machine's owning team, so this is how a test makes a non-admin a machine's controller.
async fn team_owned_by(pool: &PgPool, slug: &str, owner: Uuid) -> Uuid {
    let team = crate::test_support::team::insert(pool, slug).await;
    crate::test_support::team::set_member(pool, team, owner, "owner").await;
    team
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_team_owners_revoke_kills_the_credential_and_leaves_standing(pool: PgPool) {
    // `Act::Revoke` on standing is an admin act. A team owner controls the credential, not the
    // admission an admin granted, so their revoke must stop at the credential (ruled
    // 2026-10-09: revoking a credential revokes the login credential only).
    let f = seed_machine_with_reach(&pool).await;
    crate::test_support::approve(&pool, f.machine_profile).await;

    let owner_profile = seed_agent_link(&pool, "revoking-team-owner").await;
    let team = team_owned_by(&pool, "revoking-owners-team", owner_profile).await;
    sqlx::query("UPDATE kb_machine_clients SET team_id = $2 WHERE id = $1")
        .bind(f.machine_id)
        .bind(team)
        .execute(&pool)
        .await
        .expect("give the machine an owning team");
    let owner = crate::test_support::human_principal_for(&pool, owner_profile).await;
    assert!(
        Principal::Human(&owner)
            .system_admin(&pool)
            .await
            .unwrap()
            .is_none(),
        "the revoker must not be an admin, or this test proves nothing"
    );

    let revoked = svc::revoke(&pool, f.machine_id, &owner)
        .await
        .expect("the owner of the machine's team may revoke its credential");
    assert!(revoked.revoked_at.is_some(), "the credential is dead");
    assert_eq!(revoked.revoked_by_profile_id, Some(owner_profile));

    let state: String = sqlx::query_scalar!(
        "SELECT state FROM kb_principal_standing WHERE profile_id = $1",
        f.machine_profile
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        state, "approved",
        "a team owner's revoke must not perform the admin-only standing revoke"
    );
}

/// A former owner of a soft-deleted team controls none of its machines: rotating a secret would
/// hand them a working credential that keeps every live membership and grant the machine holds
/// (soft-delete design D1). Admitted while the team is live; refused, as for a missing id, once
/// it is dead.
///
/// FAILS IF `team_service::role_on_team` answers for a dead team again.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_former_owner_of_a_dead_team_cannot_rotate_its_machines_secret(pool: PgPool) {
    let id = seed_temper_issued(&pool, "tmpr_dead_team", "old-secret").await;
    let owner_profile = seed_agent_link(&pool, "dead-team-owner").await;
    let team = team_owned_by(&pool, "dead-machines-team", owner_profile).await;
    sqlx::query("UPDATE kb_machine_clients SET team_id = $2 WHERE id = $1")
        .bind(id)
        .bind(team)
        .execute(&pool)
        .await
        .expect("give the machine an owning team");
    let owner = crate::test_support::human_principal_for(&pool, owner_profile).await;

    svc::rotate_secret(
        &pool,
        &owner,
        id,
        0,
        crate::auth_config::AuthMode::ExternalIdp,
    )
    .await
    .expect("the owner of a live team rotates its machine's secret");

    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(team)
        .execute(&pool)
        .await
        .expect("soft-delete the team");
    let before: Option<String> =
        sqlx::query_scalar("SELECT secret_hash FROM kb_machine_clients WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("hash");
    let err = svc::rotate_secret(
        &pool,
        &owner,
        id,
        0,
        crate::auth_config::AuthMode::ExternalIdp,
    )
    .await
    .expect_err("a dead team's former owner rotates nothing");
    assert!(
        matches!(err, crate::error::ApiError::NotFound(_)),
        "refused as for a missing id: {err:?}"
    );
    let after: Option<String> =
        sqlx::query_scalar("SELECT secret_hash FROM kb_machine_clients WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("hash");
    assert_eq!(before, after, "the secret is unchanged");
}

/// The existence oracle, closed: a caller outside a machine's authority gets the **same** error
/// for an existing machine client as for a missing id — variant and message — on every per-row
/// act. A `Forbidden` on any of these would mean the row exists.
///
/// Four targets, because the row's state is what a mis-ordered check would leak:
/// - teamless (admin-only, spec D2);
/// - owned by a team the prober does not own, while the prober owns a team of their own;
/// - revoked;
/// - IdP-issued.
///
/// The last two are what pin `rotate_secret`'s order. Move its revoked or issuer `400` above
/// the gate and those rows answer differently from a missing id, which this test then catches.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_denied_machine_client_is_indistinguishable_from_a_missing_one(pool: PgPool) {
    let teamless = seed_temper_issued(&pool, "probe-teamless", "s3cret").await;

    let owned = seed_temper_issued(&pool, "probe-owned", "s3cret").await;
    let rightful_owner = seed_agent_link(&pool, "probe-rightful-owner").await;
    let owning_team = team_owned_by(&pool, "probe-owning-team", rightful_owner).await;
    sqlx::query("UPDATE kb_machine_clients SET team_id = $2 WHERE id = $1")
        .bind(owned)
        .bind(owning_team)
        .execute(&pool)
        .await
        .expect("give the machine an owning team");

    let revoked = seed_temper_issued(&pool, "probe-revoked", "s3cret").await;
    sqlx::query("UPDATE kb_machine_clients SET revoked_at = now() WHERE id = $1")
        .bind(revoked)
        .execute(&pool)
        .await
        .expect("revoke");

    let idp_issued = seed_registered(&pool, "probe-idp-issued").await;

    // The prober is a team owner too — just not of any team that owns a target.
    let prober_profile = seed_agent_link(&pool, "probe-agent").await;
    team_owned_by(&pool, "probe-own-team", prober_profile).await;
    let prober = crate::test_support::human_principal_for(&pool, prober_profile).await;
    let missing = Uuid::now_v7();

    let targets = [
        ("teamless", teamless),
        ("owned by another team", owned),
        ("revoked", revoked),
        ("IdP-issued", idp_issued),
    ];
    for (state, target) in targets {
        for id in [target, missing] {
            let refusals = [
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
                    "rotate",
                    svc::rotate_secret(
                        &pool,
                        &prober,
                        id,
                        0,
                        crate::auth_config::AuthMode::ExternalIdp,
                    )
                    .await
                    .expect_err("rotate"),
                ),
            ];
            for (act, err) in refusals {
                assert!(
                    matches!(&err, crate::error::ApiError::NotFound(m) if m == svc::MACHINE_CLIENT_REFUSAL),
                    "{act} on a {state} target (or a missing id): must refuse as a missing \
                     machine client, got {err:?}"
                );
            }
        }
    }

    for (state, target) in targets {
        let rotated_at: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar("SELECT secret_rotated_at FROM kb_machine_clients WHERE id = $1")
                .bind(target)
                .fetch_one(&pool)
                .await
                .expect("read rotation");
        assert!(
            rotated_at.is_none(),
            "{state}: the denied rotate wrote nothing"
        );
    }
    for target in [teamless, owned, idp_issued] {
        assert!(
            svc::get(&pool, target)
                .await
                .expect("get")
                .revoked_at
                .is_none(),
            "the denied revoke wrote nothing"
        );
    }
}
