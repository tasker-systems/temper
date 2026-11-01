//! Machine-registration authority — may this caller act on a machine owned by this team? — and its
//! per-row twin, [`MachineClientControlAuthority`], for acts on an existing machine client. A
//! system admin's registration arm stops at someone else's personal team; their per-row arm does
//! not (task `01a191b4`), because a system admin manages every principal.
//!
//! **Fails closed on `None`** (spec D2): a teamless machine (`team_id IS NULL`) is admin-only to
//! create, read, or operate. *"No team to check" must never mean "nothing to deny"* — so the
//! absent-team branch is an explicit denial arm here, not a fallthrough.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::team::TeamRole;

use super::{Principal, ScopedAuthority};
use crate::error::{ApiError, ApiResult};
use crate::services::machine_client_service::{self, MACHINE_CLIENT_REFUSAL};
use crate::services::{machine_authz::MachineRegistrationAuthority, team_service};

#[async_trait]
impl ScopedAuthority for MachineRegistrationAuthority {
    /// The team that owns (or will own) the machine. `None` is a real, denied case — see the
    /// module doc — not an "unknown" to be skipped over.
    type Subject = Option<Uuid>;

    async fn resolve(
        pool: &PgPool,
        principal: Principal<'_>,
        team: Option<Uuid>,
    ) -> ApiResult<Self> {
        let caller = principal.profile_id();
        // The admin arm stops at a personal team (task `01a191b4`): a machine or connection owned by
        // one is its owner's alone. A teamless machine stays admin-only (spec D2).
        let personal = match team {
            Some(team_id) => team_service::personal_space_owner(pool, team_id)
                .await?
                .is_some_and(|owner| owner != caller),
            None => false,
        };
        if !personal && principal.system_admin(pool).await?.is_some() {
            return Ok(MachineRegistrationAuthority::SystemAdmin);
        }

        let Some(team_id) = team else {
            return Ok(MachineRegistrationAuthority::None);
        };

        Ok(
            match team_service::role_on_team(pool, team_id, caller).await? {
                Some(TeamRole::Owner) => MachineRegistrationAuthority::TeamOwner,
                _ => MachineRegistrationAuthority::None,
            },
        )
    }

    fn is_denial(&self) -> bool {
        matches!(self, MachineRegistrationAuthority::None)
    }

    fn denial() -> ApiError {
        ApiError::Forbidden
    }
}

/// May this caller act on **this machine client** — the per-row gate behind get, revoke and
/// secret rotation?
///
/// [`MachineRegistrationAuthority`] is keyed on a team, which is right for registration (the caller
/// names the team that will own the machine, and there is no row yet). A per-row act is keyed on
/// the row: the owning team is read from it, never supplied, and the policy is then
/// `MachineRegistrationAuthority`'s, **called, not restated**.
///
/// **Its refusal is `NotFound`, not `Forbidden`, and that is the reason it exists.** Keyed on the
/// team, a refusal arrived only after the row was loaded, so a missing id answered 404 and a
/// present-but-forbidden one 403 — an existence oracle any approved bearer could probe. Here a
/// denial renders `machine_client_service::MACHINE_CLIENT_REFUSAL`, the same sentence the row
/// lookup renders for a missing id, so the two are indistinguishable. Nothing is withheld from a
/// caller who controls the row: they can `GET` it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MachineClientControlAuthority {
    /// A system admin.
    SystemAdmin,
    /// Owner of the team that owns this machine client.
    OwnerOfOwningTeam,
    /// Neither — including a teamless machine, which fails closed (spec D2).
    None,
}

#[async_trait]
impl ScopedAuthority for MachineClientControlAuthority {
    /// The machine client's own id. Its owning team is derived from the row.
    type Subject = Uuid;

    async fn resolve(
        pool: &PgPool,
        caller: Principal<'_>,
        machine_client: Uuid,
    ) -> ApiResult<Self> {
        // A missing row is `NotFound(MACHINE_CLIENT_REFUSAL)` from the lookup itself — the same
        // refusal `denial` renders below, which is what closes the oracle.
        let client = machine_client_service::get(pool, machine_client).await?;

        // A system admin manages every principal, machines on a personal team included (task
        // `01a191b4`): get, revoke and rotation are theirs. What protects the personal team is
        // that a credential minted by anyone but its person severs the machine from it
        // (`machine_client_service::sever_personal_reach`), not a refusal here. Registration and
        // connections keep `MachineRegistrationAuthority`'s personal-team stop.
        if caller.system_admin(pool).await?.is_some() {
            return Ok(MachineClientControlAuthority::SystemAdmin);
        }

        Ok(
            match <MachineRegistrationAuthority as ScopedAuthority>::resolve(
                pool,
                caller,
                client.team_id,
            )
            .await?
            {
                MachineRegistrationAuthority::SystemAdmin => {
                    MachineClientControlAuthority::SystemAdmin
                }
                MachineRegistrationAuthority::TeamOwner => {
                    MachineClientControlAuthority::OwnerOfOwningTeam
                }
                MachineRegistrationAuthority::None => {
                    super::log_concealed_refusal(caller, "machine_client");
                    MachineClientControlAuthority::None
                }
            },
        )
    }

    fn is_denial(&self) -> bool {
        matches!(self, MachineClientControlAuthority::None)
    }

    /// `NotFound`, byte-identical to the missing-row refusal — see the type's doc.
    fn denial() -> ApiError {
        ApiError::NotFound(MACHINE_CLIENT_REFUSAL.to_string())
    }
}

#[cfg(all(test, feature = "test-db"))]
mod personal_space_tests {
    //! Task 01a191b4: a system admin's arm stops at someone else's personal team, for machines and
    //! connections (this authority) and for subscriptions (`SubscriptionAuthority`), and keeps the
    //! admin's own personal team and every shared team.

    use super::*;
    use crate::authz::SubscriptionAuthority;
    use temper_core::types::ids::ProfileId;

    async fn person(pool: &PgPool, handle: &str) -> (ProfileId, Uuid) {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap();
        let personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        (ProfileId::from(id), personal)
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_admin_arm_stops_at_someone_elses_personal_team(pool: PgPool) {
        let (admin, admins_own) = person(&pool, "admin").await;
        let (_owner, theirs) = person(&pool, "owner").await;
        crate::test_support::grant_governance(&pool, *admin).await;
        let shared: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (slug, name) VALUES ('acme', 'acme') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let p = || Principal::Bare(admin);

        assert_eq!(
            MachineRegistrationAuthority::resolve(&pool, p(), Some(theirs))
                .await
                .unwrap(),
            MachineRegistrationAuthority::None
        );
        assert_eq!(
            SubscriptionAuthority::resolve(&pool, p(), theirs)
                .await
                .unwrap(),
            SubscriptionAuthority::None
        );
        for team in [Some(shared), Some(admins_own), None] {
            assert_eq!(
                MachineRegistrationAuthority::resolve(&pool, p(), team)
                    .await
                    .unwrap(),
                MachineRegistrationAuthority::SystemAdmin,
                "{team:?}"
            );
        }
        assert_eq!(
            SubscriptionAuthority::resolve(&pool, p(), shared)
                .await
                .unwrap(),
            SubscriptionAuthority::SystemAdmin
        );
    }

    /// A teamless connection homes its context on its own agent profile. That is no person's
    /// personal space, so the admin arm still reaches it: counting it as personal would leave it
    /// with no one able to act on it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_agents_own_context_is_no_ones_personal_space(pool: PgPool) {
        use crate::services::subscription_test_support::{seed_admin, seed_connection};
        let admin = seed_admin(&pool).await;
        let conn = seed_connection(&pool, None, admin).await;
        let home: Uuid =
            sqlx::query_scalar("SELECT home_context_id FROM kb_connections WHERE id = $1")
                .bind(conn)
                .fetch_one(&pool)
                .await
                .unwrap();
        let owner: Option<Uuid> = sqlx::query_scalar("SELECT context_personal_owner($1)")
            .bind(home)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(owner, None, "an agent's home context is not personal space");
        assert!(
            !crate::services::context_service::is_others_personal_context(&pool, home, admin)
                .await
                .unwrap()
        );
    }

    /// A subscription a shared team authors into someone else's personal context (shared to that
    /// team) is hidden from an admin's list, and its per-row get/revoke refuse the admin too; the
    /// team's manager keeps both.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_subscription_into_someone_elses_personal_context_is_not_the_admins(pool: PgPool) {
        use crate::authz::SubscriptionControlAuthority;
        use crate::services::subscription_test_support::*;
        use temper_core::types::subscription::SubscriptionSelector;

        let manager = seed_admin(&pool).await;
        let auditor = seed_admin(&pool).await;
        let (alice, _) = person(&pool, "sub-alice").await;
        let team = seed_team(&pool, manager).await;
        let alices: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_profiles', $1, 'sub-alices', 'sub-alices') RETURNING id",
        )
        .bind(*alice)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
            .bind(alices)
            .bind(team)
            .execute(&pool)
            .await
            .unwrap();
        let conn = seed_connection(&pool, Some(team), manager).await;
        grant_reach(&pool, manager, conn, team).await;
        // Seeded directly: since the subscriber must be authored by whoever declares it, no one but
        // Alice can now declare one into her personal context, and she does not manage `team`. The
        // question here is who controls such a row once it exists (a pre-existing declaration),
        // which the create gate does not decide.
        let declared = crate::services::subscription_service::create(
            &pool,
            &crate::test_support::human_principal_for(&pool, *manager).await,
            &temper_core::types::subscription::CreateSubscriptionRequest {
                subscriber_table: "kb_contexts".into(),
                subscriber_id: alices,
                authoring_team_id: team,
                connection_id: conn,
                selector: SubscriptionSelector::GitHubRepository {
                    repo: GITHUB_REPO.into(),
                    event_types: vec![],
                },
            },
        )
        .await;
        assert!(
            matches!(declared, Err(crate::error::ApiError::Forbidden)),
            "the share alone does not let the team's manager declare into Alice's context: {declared:?}"
        );
        let selector = serde_json::to_value(SubscriptionSelector::GitHubRepository {
            repo: GITHUB_REPO.into(),
            event_types: vec![],
        })
        .unwrap();
        let sub: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_subscriptions (subscriber_table, subscriber_id, authoring_team_id, \
                                           connection_id, selector, created_by_profile_id) \
             VALUES ('kb_contexts', $1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(alices)
        .bind(team)
        .bind(conn)
        .bind(selector)
        .bind(*manager)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(
            SubscriptionControlAuthority::resolve(&pool, Principal::Bare(auditor), sub)
                .await
                .unwrap(),
            SubscriptionControlAuthority::None
        );
        assert_eq!(
            SubscriptionControlAuthority::resolve(&pool, Principal::Bare(manager), sub)
                .await
                .unwrap(),
            SubscriptionControlAuthority::TeamManager
        );
    }

    /// A system admin manages every machine principal, one on a person's personal team included;
    /// but a credential they (or an owner of the machine row's team) mint severs the machine from
    /// that personal team, recorded, so it cannot open it. The person minting keeps the membership.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_credential_minted_by_anyone_but_the_person_severs_their_personal_team(pool: PgPool) {
        use crate::services::machine_client_service::{
            others_personal_teams, sever_personal_reach,
        };
        let (admin, _) = person(&pool, "mint-admin").await;
        crate::test_support::grant_governance(&pool, *admin).await;
        // The admin acts through their `@web` emitter, which records the severance.
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, 'mint-admin@web', '{}')",
        )
        .bind(*admin)
        .execute(&pool)
        .await
        .unwrap();
        let (alice, alices_team) = person(&pool, "mint-alice").await;
        let (machine, _) = person(&pool, "mint-machine").await;
        let acme: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (slug, name) VALUES ('mint-acme', 'acme') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let client: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_machine_clients (client_id, issuer, label, profile_id, team_id, \
                                             registered_by_profile_id) \
             VALUES ('mint-client', 'auth0-m2m', 'm', $1, $2, $3) RETURNING id",
        )
        .bind(*machine)
        .bind(acme)
        .bind(*alice)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
        )
        .bind(alices_team)
        .bind(*machine)
        .execute(&pool)
        .await
        .expect("alice's machine joins her personal team");
        // Direct grants on alice's personal context, on a shared team's context, and on quinn's
        // personal context (task `01a1933d`). Alice, the machine's own person,
        // minting withdraws none of them (ruled 2026-10-31); an admin's credential takes both
        // personal ones and leaves the shared one.
        let (quinn, _) = person(&pool, "mint-quinn").await;
        let mut contexts = Vec::new();
        for (owner_table, owner_id, slug) in [
            ("kb_profiles", *alice, "mint-alices"),
            ("kb_teams", acme, "mint-acmes"),
            ("kb_profiles", *quinn, "mint-quinns"),
        ] {
            let ctx: Uuid = sqlx::query_scalar(
                "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
                 VALUES ($1, $2, $3, $3) RETURNING id",
            )
            .bind(owner_table)
            .bind(owner_id)
            .bind(slug)
            .fetch_one(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, \
                   principal_id, can_read, granted_by_profile_id) \
                 VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
            )
            .bind(ctx)
            .bind(*machine)
            .bind(*alice)
            .execute(&pool)
            .await
            .unwrap();
            contexts.push(ctx);
        }
        let holds = |ctx: Uuid| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS (SELECT 1 FROM kb_access_grants WHERE subject_id = $1 \
                       AND principal_id = $2)",
                )
                .bind(ctx)
                .bind(*machine)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        let on_alices = || async {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM kb_team_members WHERE team_id = $1 AND profile_id = $2)",
            )
            .bind(alices_team)
            .bind(*machine)
            .fetch_one(&pool)
            .await
            .unwrap()
        };

        // The admin manages the principal: the per-row gate admits them.
        assert_eq!(
            MachineClientControlAuthority::resolve(&pool, Principal::Bare(admin), client)
                .await
                .unwrap(),
            MachineClientControlAuthority::SystemAdmin
        );

        // Alice minting for her own machine keeps it on her personal team.
        let mut tx = pool.begin().await.unwrap();
        assert!(sever_personal_reach(&mut tx, *machine, alice)
            .await
            .unwrap()
            .is_empty());
        tx.commit().await.unwrap();
        assert!(on_alices().await);
        assert!(
            holds(contexts[0]).await && holds(contexts[1]).await && holds(contexts[2]).await,
            "the machine's own person rotating withdraws nothing, quinn's grant included"
        );

        // The admin minting severs it, and the removal is recorded with the admin as actor.
        let mut conn = pool.acquire().await.unwrap();
        assert_eq!(
            others_personal_teams(&mut conn, *machine, *admin)
                .await
                .unwrap(),
            vec![alices_team]
        );
        drop(conn);
        let mut tx = pool.begin().await.unwrap();
        assert_eq!(
            sever_personal_reach(&mut tx, *machine, admin)
                .await
                .unwrap(),
            vec![alices_team]
        );
        tx.commit().await.unwrap();
        assert!(
            !on_alices().await,
            "the admin's credential does not open alice's personal team"
        );
        assert!(
            !holds(contexts[0]).await,
            "nor her personal space through the machine's own grant"
        );
        assert!(
            !holds(contexts[2]).await,
            "nor quinn's personal space, whoever granted it"
        );
        assert!(holds(contexts[1]).await, "a shared grant stays");
        let revoked: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
              WHERE t.name = 'grant_revoked' AND e.payload->>'subject_id' = $1::text",
        )
        .bind(contexts[0])
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(revoked, 1, "the withdrawal is recorded");
        let recorded: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
              WHERE t.name = 'team_membership_changed' \
                AND e.payload->>'via' = 'machine_reach' \
                AND e.payload->>'actor' = $1::text \
                AND e.payload->>'role' IS NULL",
        )
        .bind(*admin)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            recorded, 1,
            "the severance is a recorded membership removal"
        );
    }
}
