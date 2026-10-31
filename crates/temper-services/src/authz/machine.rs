//! Machine-registration authority — may this caller act on a machine owned by this team? — and its
//! per-row twin, [`MachineClientControlAuthority`], for acts on an existing machine client.
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
        let sub = create_subscription(
            &pool,
            manager,
            "kb_contexts",
            alices,
            team,
            conn,
            SubscriptionSelector::GitHubRepository {
                repo: GITHUB_REPO.into(),
                event_types: vec![],
            },
        )
        .await;

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

    /// A machine on a person's personal team is that person's to mint credentials for, whatever
    /// shared team owns its row: not a system admin's, not another owner of that team's.
    #[sqlx::test(migrations = "../../migrations")]
    async fn only_the_person_mints_for_a_machine_on_their_personal_team(pool: PgPool) {
        use crate::services::machine_client_service::{
            admitting_person, require_credential_minter,
        };
        let (admin, _) = person(&pool, "mint-admin").await;
        crate::test_support::grant_governance(&pool, *admin).await;
        let (alice, alices_team) = person(&pool, "mint-alice").await;
        let (machine, _) = person(&pool, "mint-machine").await;
        let acme: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (slug, name) VALUES ('mint-acme', 'acme') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_machine_clients (client_id, issuer, label, profile_id, team_id, \
                                             registered_by_profile_id) \
             VALUES ('mint-client', 'auth0-m2m', 'm', $1, $2, $3)",
        )
        .bind(*machine)
        .bind(acme)
        .bind(*alice)
        .execute(&pool)
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

        assert_eq!(
            admitting_person(&pool, *machine).await.unwrap(),
            Some(*alice)
        );
        assert!(matches!(
            require_credential_minter(&pool, *machine, *admin).await,
            Err(ApiError::NotFound(_))
        ));
        require_credential_minter(&pool, *machine, *alice)
            .await
            .expect("alice mints for her own machine");
        let (lonely, _) = person(&pool, "mint-lonely").await;
        require_credential_minter(&pool, *lonely, *admin)
            .await
            .expect("no personal team admits it, so the ordinary gate decides");
    }
}
