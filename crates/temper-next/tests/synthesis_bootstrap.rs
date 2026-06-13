#![cfg(feature = "artifact-tests")]
//! synthesis::bootstrap seeds the administrative infrastructure (§1/§2): the per-profile
//! `kb_profiles`, the `migration` entity + the three per-surface entities (`pete@{cli,mcp,web}`),
//! and the thin unowned `kb_contexts` (by name), returning the old→new remaps the resource pass
//! consumes. Entity/profile/context creation is administrative — direct inserts, NO event (§1 residue).
//!
//! Runs on its own ephemeral DB via `#[sqlx::test(migrator = ...)]`: the full migration chain
//! (including the additive `temper_next` install) is applied, so `public` is migrated-but-empty and
//! `temper_next` is empty. The prod-shape fixture seeds `public.*` only. NOT in the write-path group.

mod common;

use common::fixture_ids;
use temper_next::ids::EntityId;

#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn bootstrap_seeds_entities_profiles_contexts(pool: sqlx::PgPool) {
    common::seed_prod_shape_fixture(&pool).await;
    let resources = temper_next::synthesis::source::active_resources(&pool)
        .await
        .unwrap();
    let maps = temper_next::synthesis::bootstrap::run(&pool, &resources)
        .await
        .unwrap();

    // Contexts: exactly the two fixture contexts (both referenced by active resources) by name (§2).
    let ctx_names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM temper_next.kb_contexts ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        ctx_names,
        vec![
            "fixture-context-one".to_string(),
            "fixture-context-two".to_string()
        ]
    );

    // Remap covers every context referenced by an active resource, and each maps to a present row.
    for r in &resources {
        let new_ctx = maps
            .context_id_by_old
            .get(&r.kb_context_id)
            .expect("referenced context remapped");
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM temper_next.kb_contexts WHERE id = $1)",
        )
        .bind(new_ctx.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            exists,
            "remapped context id present in temper_next.kb_contexts"
        );
    }

    // The migration entity carries the §1a intent+source metadata.
    let intent: String =
        sqlx::query_scalar("SELECT metadata->>'intent' FROM temper_next.kb_entities WHERE id = $1")
            .bind(maps.migration_entity.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(intent, "migration");
    let source: String =
        sqlx::query_scalar("SELECT metadata->>'source' FROM temper_next.kb_entities WHERE id = $1")
            .bind(maps.migration_entity.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(source, "temper-production");

    // The three per-surface entities (§1b) exist, exactly once each, all returned in the maps.
    for (name, id) in [
        ("pete@cli", maps.surfaces.cli),
        ("pete@mcp", maps.surfaces.mcp),
        ("pete@web", maps.surfaces.web),
    ] {
        let by_id: String =
            sqlx::query_scalar("SELECT name FROM temper_next.kb_entities WHERE id = $1")
                .bind(EntityId::uuid(id))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            by_id, name,
            "surface entity {name} present at its returned id"
        );
    }

    // All four entities (migration + three surfaces) bind to one (Pete's) profile.
    let entity_profiles: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT DISTINCT profile_id FROM temper_next.kb_entities")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        entity_profiles.len(),
        1,
        "migration + surface entities all bound to one profile"
    );

    // Profiles: both fixture owner + originator carried (originator≠owner in the fixture), keyed by
    // old id, with handles sourced from the production slug.
    assert!(maps
        .profile_id_by_old
        .contains_key(&fixture_ids::OWNER_PROFILE));
    assert!(maps
        .profile_id_by_old
        .contains_key(&fixture_ids::ORIGINATOR_PROFILE));
    let handles: Vec<String> =
        sqlx::query_scalar("SELECT handle FROM temper_next.kb_profiles ORDER BY handle")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        handles.contains(&"fixture-owner".to_string()),
        "owner handle present: {handles:?}"
    );
    assert!(
        handles.contains(&"fixture-originator".to_string()),
        "originator handle present: {handles:?}"
    );
}
