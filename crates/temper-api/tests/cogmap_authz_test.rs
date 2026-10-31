#![cfg(feature = "test-db")]
//! Reconcile's authority, decided by regime (`DbBackend::authorize_reconcile`): a map in the
//! admin-only regime — the reserved L0 kernel, or a map joined to the gating (root) team — requires
//! `is_system_admin`; every other map requires authorship of the map, a system admin included
//! (ruled 2026-10-01). The L0 system-default map (`20260625000001`) is joined to `temper-system`, so
//! it is the canonical admin-regime case.
//!
//! The canonical seed leaves `kb_system_settings.gating_team_slug` NULL (open mode). Both the
//! regime's root-join detection AND `is_system_admin` resolve through that slug, so these tests first
//! configure it to `temper-system` — the production-shaped config the gate is designed for.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

mod common;

const L0_COGMAP: CogmapId = CogmapId(Uuid::from_u128(0x00000000_0000_0000_0005_000000000001));

/// Configure the gating team slug to the root team born by the L0 migration. Without this the
/// canonical seed runs in `open` mode with a NULL gating slug (no root team configured).
async fn set_gating_team(pool: &PgPool) {
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'temper-system' WHERE id = 1")
        .execute(pool)
        .await
        .expect("set gating team slug");
}

/// Mint an admin profile under D11: admin-ness is `approved` standing + a `kb_principal_governance`
/// grant — neither the Phase-2-retired `system_access` column nor gating ownership confers it.
async fn admin_profile(pool: &PgPool, email: &str) -> Uuid {
    let id = common::fixtures::create_test_profile(pool, email).await;
    common::fixtures::make_test_admin(pool, id).await;
    id
}

fn as_profile(pool: &PgPool, id: Uuid) -> DbBackend {
    DbBackend::new(pool.clone(), ProfileId::from(id))
}

/// FAILS IF a non-admin may reconcile the root-joined L0 map, or an admin may not.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_reconcile_requires_system_admin(pool: PgPool) {
    set_gating_team(&pool).await;

    let non_admin = common::fixtures::create_test_profile(&pool, "nonadmin@example.com").await;
    let denied = as_profile(&pool, non_admin)
        .authorize_reconcile(L0_COGMAP)
        .await;
    assert!(
        matches!(denied, Err(TemperError::Forbidden)),
        "non-admin must be Forbidden on the root-team-joined L0 map, got {denied:?}"
    );

    let admin = admin_profile(&pool, "admin@example.com").await;
    as_profile(&pool, admin)
        .authorize_reconcile(L0_COGMAP)
        .await
        .expect("an admin passes L0's regime gate");
}

/// FAILS IF L0 is reconcilable by anyone while gating is unconfigured. The regime is fail-CLOSED:
/// the reserved map requires `is_system_admin` unconditionally, and with gating unconfigured
/// `is_system_admin` is false for everyone — so L0 is immutable until an operator configures gating.
/// Without the unconditional L0 branch a NULL gating slug would drop L0 into the authorship arm.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_is_immutable_when_gating_unconfigured(pool: PgPool) {
    let any_profile = common::fixtures::create_test_profile(&pool, "anyone@example.com").await;
    let denied = as_profile(&pool, any_profile)
        .authorize_reconcile(L0_COGMAP)
        .await;
    assert!(
        matches!(denied, Err(TemperError::Forbidden)),
        "L0 must be immutable (Forbidden to all) when gating is unconfigured, got {denied:?}"
    );
}

/// FAILS IF a map outside the admin-only regime is reconcilable without authorship. A map NOT joined
/// to the gating team falls to `check_cogmap_authorable`: a principal who holds no grant on it (here
/// an unknown map, which nobody authors) is refused in the argument-free dialect.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_non_root_cogmap_requires_authorship(pool: PgPool) {
    set_gating_team(&pool).await;

    let non_admin = common::fixtures::create_test_profile(&pool, "user@example.com").await;
    let denied = as_profile(&pool, non_admin)
        .authorize_reconcile(CogmapId::new())
        .await;
    assert!(
        matches!(denied, Err(TemperError::Forbidden)),
        "a non-author must be refused on a map outside the admin-only regime, got {denied:?}"
    );
}

/// Genesis a map as `creator` (not L0, joined to no team). Returns its id.
async fn genesis_map(pool: &PgPool, creator: Uuid, name: &str) -> Uuid {
    use temper_workflow::operations::{Backend, CreateCognitiveMap, Surface};
    as_profile(pool, creator)
        .create_cognitive_map(CreateCognitiveMap {
            request: temper_core::types::reconcile::CreateCogmapRequest {
                cogmap_id: None,
                telos_resource_id: None,
                name: name.to_string(),
                telos_title: format!("{name} telos"),
                telos: None,
            },
            origin: Surface::ApiHttp,
        })
        .await
        .expect("genesis a map")
        .value
        .cogmap_id
}

/// FAILS IF joining a map to the gating team does not put it in the admin-only regime. A non-admin
/// who holds a WRITE grant — authorship, which admits it on an unjoined map (the control below) — is
/// refused on a non-L0 map joined to the gating team, so the refusal is the regime's, not a missing
/// grant's. The bite: make `access_service::cogmap_write_requires_admin` answer false for every map
/// but L0 — the joined map then falls to authorship and the grant admits it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_gating_team_map_refuses_a_non_admin_author(pool: PgPool) {
    set_gating_team(&pool).await;
    let creator = admin_profile(&pool, "creator@example.com").await;
    let author = common::fixtures::create_test_profile(&pool, "author@example.com").await;

    let unjoined = genesis_map(&pool, creator, "Unjoined map").await;
    common::fixtures::grant_cogmap_write(&pool, unjoined, author).await;
    as_profile(&pool, author)
        .authorize_reconcile(CogmapId::from(unjoined))
        .await
        .expect("control: a write grant admits its holder on a map outside the admin-only regime");

    let joined = genesis_map(&pool, creator, "Gating-team map").await;
    sqlx::query(
        "INSERT INTO kb_team_cogmaps (cogmap_id, team_id) \
         SELECT $1, id FROM kb_teams WHERE slug = 'temper-system'",
    )
    .bind(joined)
    .execute(&pool)
    .await
    .expect("join the map to the gating team");
    common::fixtures::grant_cogmap_write(&pool, joined, author).await;
    let denied = as_profile(&pool, author)
        .authorize_reconcile(CogmapId::from(joined))
        .await;
    assert!(
        matches!(denied, Err(TemperError::Forbidden)),
        "a non-admin author is refused on a gating-team map, got {denied:?}"
    );
}

// ── Content creation reads the regime too (`cogmap_authorable_by_profile`, 20261114000000) ─────
//
// Reconcile and the grant door have always asked the regime. These ask the content path, the one
// most callers take: `create_resource` into a map, which reaches `cogmap_authorable_by_profile`
// through `check_cogmap_authorable`. A map in the regime is authored by a system admin only, and a
// write grant on it is inert, however and whenever it got there.

/// Bind `cogmap` to the team whose slug is `slug`.
async fn bind(pool: &PgPool, cogmap: Uuid, slug: &str) {
    sqlx::query(
        "INSERT INTO kb_team_cogmaps (cogmap_id, team_id) SELECT $1, id FROM kb_teams WHERE slug = $2",
    )
    .bind(cogmap)
    .bind(slug)
    .execute(pool)
    .await
    .expect("bind the map");
}

async fn unbind(pool: &PgPool, cogmap: Uuid, slug: &str) {
    sqlx::query(
        "DELETE FROM kb_team_cogmaps WHERE cogmap_id = $1 \
           AND team_id = (SELECT id FROM kb_teams WHERE slug = $2)",
    )
    .bind(cogmap)
    .bind(slug)
    .execute(pool)
    .await
    .expect("unbind the map");
}

/// A shared (non-personal) team, which `gating_team_slug` may name.
async fn shared_team(pool: &PgPool, slug: &str) {
    sqlx::query("INSERT INTO kb_teams (slug, name) VALUES ($1, $1)")
        .bind(slug)
        .execute(pool)
        .await
        .expect("create a shared team");
}

async fn point_gating_at(pool: &PgPool, slug: &str) {
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = $1 WHERE id = 1")
        .bind(slug)
        .execute(pool)
        .await
        .expect("point the gating slug");
}

async fn homes_in(pool: &PgPool, cogmap: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_homes WHERE anchor_table = 'kb_cogmaps' AND anchor_id = $1",
    )
    .bind(cogmap)
    .fetch_one(pool)
    .await
    .expect("count homes")
}

/// Author a resource into `cogmap` as `profile`, through the backend's create.
async fn author_into(
    pool: &PgPool,
    profile: Uuid,
    cogmap: Uuid,
    slug: &str,
) -> Result<(), TemperError> {
    use temper_workflow::operations::Backend;
    as_profile(pool, profile)
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: slug.to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Cogmap(CogmapId::from(cogmap)),
            title: format!("regime {slug}"),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: Some(format!("test://regime-{slug}")),
            chunks_packed: None,
            content_hash: None,
            goal: None,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .map(|_| ())
}

fn is_refusal(r: &Result<(), TemperError>) -> bool {
    matches!(
        r,
        Err(TemperError::Forbidden | TemperError::ForbiddenDetail(_))
    )
}

/// FAILS IF a write grant minted before a map is bound into the gating team still authors content
/// into it afterwards, or if the admin is not the one who authors there, or if unbinding does not
/// hand authorship back to the grant. The bite: return `cogmap_authorable_by_profile` to the bare
/// write grant, and the post-bind create succeeds.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_write_grant_minted_before_a_gating_bind_stops_authoring(pool: PgPool) {
    set_gating_team(&pool).await;
    let admin = admin_profile(&pool, "regime-admin@example.com").await;
    let author = common::fixtures::create_test_profile(&pool, "regime-author@example.com").await;
    common::fixtures::approve_standing(&pool, author).await;

    let map = genesis_map(&pool, admin, "Bound later").await;
    common::fixtures::grant_cogmap_write(&pool, map, author).await;
    author_into(&pool, author, map, "before-bind")
        .await
        .expect("control: before the bind, the write grant authors");

    bind(&pool, map, "temper-system").await;
    let before = homes_in(&pool, map).await;
    let denied = author_into(&pool, author, map, "after-bind").await;
    assert!(
        is_refusal(&denied),
        "a grant minted before the bind must not author a gating-team map: {denied:?}"
    );
    assert_eq!(
        homes_in(&pool, map).await,
        before,
        "the refused create wrote nothing"
    );

    author_into(&pool, admin, map, "admin-after-bind")
        .await
        .expect("a system admin authors a map in the regime, grant or none");

    unbind(&pool, map, "temper-system").await;
    author_into(&pool, author, map, "after-unbind")
        .await
        .expect("out of the regime again, the grant (left in place, inert) authors once more");
}

/// FAILS IF a map bound to temper-system is outside the admin-only regime when the gating slug
/// names another team. temper-system reaches every profile whatever the gating team is, so it is in
/// the regime on its own account. Asked on both spellings of the regime: content creation
/// (`cogmap_authorable_by_profile`) and reconcile (`access_service::cogmap_write_requires_admin`).
/// The bite: drop the `temper-system` arm of `cogmap_write_requires_admin(uuid)`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_map_bound_to_the_root_is_admin_only_when_the_gating_team_is_elsewhere(pool: PgPool) {
    shared_team(&pool, "elsewhere-gate").await;
    point_gating_at(&pool, "elsewhere-gate").await;
    let admin = admin_profile(&pool, "root-admin@example.com").await;
    let author = common::fixtures::create_test_profile(&pool, "root-author@example.com").await;
    common::fixtures::approve_standing(&pool, author).await;

    let map = genesis_map(&pool, admin, "Root-bound").await;
    common::fixtures::grant_cogmap_write(&pool, map, author).await;
    author_into(&pool, author, map, "unbound")
        .await
        .expect("control: unbound, the write grant authors");

    bind(&pool, map, "temper-system").await;
    let denied = author_into(&pool, author, map, "root-bound").await;
    assert!(
        is_refusal(&denied),
        "a non-admin must not author a map bound to temper-system: {denied:?}"
    );
    let reconcile = as_profile(&pool, author)
        .authorize_reconcile(CogmapId::from(map))
        .await;
    assert!(
        matches!(reconcile, Err(TemperError::Forbidden)),
        "reconcile must treat a root-bound map as admin-only too: {reconcile:?}"
    );
    as_profile(&pool, admin)
        .authorize_reconcile(CogmapId::from(map))
        .await
        .expect("an admin passes the root-bound map's regime gate");
}

/// FAILS IF repointing the gating slug at a team leaves that team's maps authorable by the grants
/// they already carry. A bind is not the only way into the regime; this is the other one, and a
/// check made only at bind time would miss it. The bite: as for the bind case above.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn repointing_the_gating_slug_brings_bound_maps_into_the_regime(pool: PgPool) {
    set_gating_team(&pool).await;
    shared_team(&pool, "future-gate").await;
    let admin = admin_profile(&pool, "repoint-admin@example.com").await;
    let author = common::fixtures::create_test_profile(&pool, "repoint-author@example.com").await;
    common::fixtures::approve_standing(&pool, author).await;

    let map = genesis_map(&pool, admin, "Repointed").await;
    bind(&pool, map, "future-gate").await;
    common::fixtures::grant_cogmap_write(&pool, map, author).await;
    author_into(&pool, author, map, "before-repoint")
        .await
        .expect("control: bound to an ordinary team, the write grant authors");

    point_gating_at(&pool, "future-gate").await;
    let denied = author_into(&pool, author, map, "after-repoint").await;
    assert!(
        is_refusal(&denied),
        "once its team is the gating team, the map is admin-only: {denied:?}"
    );
}
