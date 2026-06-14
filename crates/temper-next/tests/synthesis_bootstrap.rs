#![cfg(feature = "artifact-tests")]
//! synthesis::bootstrap seeds the administrative infrastructure (§1/§2): the per-profile
//! `kb_profiles`, the `migration` entity + the three per-surface entities (`pete@{cli,mcp,web}`),
//! and the owner-scoped, slugged `kb_contexts` (§2 amended) — profile-owned contexts remap their
//! owner through the profile map; team-owned contexts synthesize the owning `kb_teams` row and an
//! explicit `kb_team_contexts` auto-share. It returns the old→new remaps the resource pass consumes.
//! Entity/profile/context/team creation is administrative — direct inserts, NO event (§1 residue).
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

    // Contexts: the three fixture contexts referenced by active resources, by name (§2 amended) —
    // two profile-owned (C1, C2) plus the team-owned C3.
    let ctx_names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM temper_next.kb_contexts ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        ctx_names,
        vec![
            "fixture-context-one".to_string(),
            "fixture-context-two".to_string(),
            "fixture-team-context".to_string(),
        ]
    );

    // The team-owned context (§2 amended): owner carried verbatim + remapped to the synthesized team
    // id, a derived slug, the production name — and an explicit `kb_team_contexts` auto-share row so
    // the owning team still reaches the context's contents through the unchanged visibility function.
    let team_new_id: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM temper_next.kb_teams WHERE slug = 'fixture-team'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let team_ctx_new = maps
        .context_id_by_old
        .get(&fixture_ids::CONTEXT_TEAM)
        .expect("team-owned context remapped");
    let (owner_table, owner_id, slug, name): (String, uuid::Uuid, String, String) = sqlx::query_as(
        "SELECT owner_table, owner_id, slug, name FROM temper_next.kb_contexts WHERE id = $1",
    )
    .bind(team_ctx_new.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(owner_table, "kb_teams", "team-owned context owner_table");
    assert_eq!(
        owner_id, team_new_id,
        "team-owned context owner remapped to the synthesized team id"
    );
    assert_eq!(slug, "fixture-team-context", "derived slug");
    assert_eq!(name, "fixture-team-context", "production name carried");
    let share_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM temper_next.kb_team_contexts WHERE context_id=$1 AND team_id=$2)",
    )
    .bind(team_ctx_new.uuid())
    .bind(team_new_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        share_exists,
        "team-owned context gets a kb_team_contexts(context_id, owning_team_id) auto-share row"
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
