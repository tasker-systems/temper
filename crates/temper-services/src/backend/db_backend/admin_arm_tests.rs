use super::*;
use temper_core::types::reblock::ReblockScope;

const L0_COGMAP: CogmapId = CogmapId(uuid::Uuid::from_u128(
    0x00000000_0000_0000_0005_000000000001,
));

async fn set_gating_team(pool: &PgPool) {
    sqlx::query!("UPDATE kb_system_settings SET gating_team_slug = 'temper-system' WHERE id = 1")
        .execute(pool)
        .await
        .expect("set gating team slug");
}

async fn admin_person(pool: &PgPool) -> crate::auth::Caller {
    let id: uuid::Uuid = sqlx::query_scalar!(
        "INSERT INTO kb_profiles (handle, display_name) \
         VALUES ('seam-admin-' || gen_random_uuid(), 'seam admin') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("seed admin");
    crate::test_support::approved_admin(pool, id).await;
    crate::test_support::caller_for(pool, id).await
}

fn reblock_all() -> ReblockResources {
    ReblockResources {
        scope: ReblockScope::All,
        dry_run: true,
        limit: 1,
        after_id: None,
        origin: temper_workflow::operations::Surface::ApiHttp,
    }
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_machine_cannot_reconcile_an_admin_only_map(pool: PgPool) {
    set_gating_team(&pool).await;
    let machine = crate::test_support::governed_machine_caller(&pool).await;

    let refused = DbBackend::with_proof(pool.clone(), &machine)
        .authorize_reconcile(L0_COGMAP)
        .await;

    assert!(
        matches!(refused, Err(TemperError::Forbidden)),
        "a machine is refused L0's admin regime whatever the database says; got {refused:?}"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_admin_person_still_reconciles_an_admin_only_map(pool: PgPool) {
    set_gating_team(&pool).await;
    let admin = admin_person(&pool).await;

    DbBackend::with_proof(pool.clone(), &admin)
        .authorize_reconcile(L0_COGMAP)
        .await
        .expect("an admin person passes L0's regime gate");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_machine_cannot_reblock_the_whole_deployment(pool: PgPool) {
    let machine = crate::test_support::governed_machine_caller(&pool).await;

    let refused = DbBackend::with_proof(pool.clone(), &machine)
        .reblock_resources(reblock_all())
        .await;

    assert!(
        matches!(refused, Err(TemperError::Forbidden)),
        "a machine is refused reblock `All` whatever the database says; got {:?}",
        refused.map(|_| ())
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_admin_person_still_reblocks_the_whole_deployment(pool: PgPool) {
    let admin = admin_person(&pool).await;

    DbBackend::with_proof(pool.clone(), &admin)
        .reblock_resources(reblock_all())
        .await
        .expect("an admin person passes reblock `All`'s gate");
}
