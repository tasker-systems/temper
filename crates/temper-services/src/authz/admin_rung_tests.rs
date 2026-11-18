use super::*;
use uuid::Uuid;

struct Cast {
    admin: HumanPrincipal,
    plain: HumanPrincipal,
    governed_machine: Caller,
}

async fn seed_profile(pool: &PgPool, handle: &str) -> Uuid {
    let id =
        crate::test_support::profile::insert(pool, &format!("{handle}-{}", Uuid::now_v7())).await;
    crate::test_support::approve(pool, id).await;
    id
}

async fn cast(pool: &PgPool) -> Cast {
    let admin = seed_profile(pool, "rung-admin").await;
    crate::test_support::grant_governance(pool, admin).await;
    let plain = seed_profile(pool, "rung-plain").await;
    Cast {
        admin: crate::test_support::human_principal_for(pool, admin).await,
        plain: crate::test_support::human_principal_for(pool, plain).await,
        governed_machine: crate::test_support::governed_machine_caller(pool).await,
    }
}

macro_rules! rung {
    ($name:ident, $authority:ty, $admin_arm:pat, $subject:expr) => {
        #[sqlx::test(migrator = "crate::MIGRATOR")]
        async fn $name(pool: PgPool) {
            let c = cast(&pool).await;
            let subject = $subject;

            let arm = <$authority>::resolve(&pool, Principal::Human(&c.admin), subject)
                .await
                .expect("resolve for the admin person");
            assert!(
                matches!(arm, $admin_arm),
                "an admin person takes the admin arm, got {arm:?}"
            );

            let arm = <$authority>::resolve(&pool, Principal::Human(&c.plain), subject)
                .await
                .expect("resolve for the plain person");
            assert!(
                !matches!(arm, $admin_arm),
                "a plain person does not, got {arm:?}"
            );

            let arm = <$authority>::resolve(&pool, Principal::from(&c.governed_machine), subject)
                .await
                .expect("resolve for the governed machine");
            assert!(
                !matches!(arm, $admin_arm),
                "a machine never takes the admin arm, whatever the database says; got {arm:?}"
            );
        }
    };
}

rung!(
    grant_authority_admin_arm_is_minted_from_a_person,
    crate::services::access_service::GrantAuthority,
    crate::services::access_service::GrantAuthority::SystemAdmin,
    temper_substrate::payloads::RefTarget {
        kind: temper_substrate::payloads::AnchorTable::Contexts,
        id: Uuid::now_v7(),
    }
);
rung!(
    two_sided_authority_admin_arm_is_minted_from_a_person,
    TwoSidedAuthority,
    TwoSidedAuthority::SystemAdmin,
    TwoSidedScope::context(Uuid::now_v7(), Uuid::now_v7())
);
rung!(
    context_admin_authority_admin_arm_is_minted_from_a_person,
    ContextAdminAuthority,
    ContextAdminAuthority::SystemAdmin,
    Uuid::now_v7()
);
rung!(
    machine_authority_admin_arm_is_minted_from_a_person,
    crate::services::machine_authz::MachineRegistrationAuthority,
    crate::services::machine_authz::MachineRegistrationAuthority::SystemAdmin,
    None::<Uuid>
);
rung!(
    subscription_authority_admin_arm_is_minted_from_a_person,
    SubscriptionAuthority,
    SubscriptionAuthority::SystemAdmin,
    Uuid::now_v7()
);
rung!(
    team_read_authority_admin_arm_is_minted_from_a_person,
    TeamReadAuthority,
    TeamReadAuthority::SystemAdmin,
    Uuid::now_v7()
);
rung!(
    actor_history_authority_admin_arm_is_minted_from_a_person,
    ActorHistoryAuthority,
    ActorHistoryAuthority::SystemAdmin,
    ProfileId::from(Uuid::now_v7())
);
