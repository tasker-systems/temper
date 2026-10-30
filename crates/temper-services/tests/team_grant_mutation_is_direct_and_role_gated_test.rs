#![cfg(feature = "test-db")]

//! A team-anchored grant confers MUTATION only on direct members of the granted team with an
//! authoring role; READ still inherits down the team DAG at any role
//! (`migrations/20261109000000_team_grant_mutation_direct_role_gated.sql`).
//!
//! The rule it pins is the one `context_authorable_by_profile`'s narrowed arm already states:
//! "Read inherits up the enclosure chain; mutation does not. `watcher` is read-only."
//!
//! ```text
//!   umbrella  ─▸ squad          (squad is a child of umbrella)
//!   wanda: watcher on squad      (direct, child only)
//!   walt:  watcher on umbrella   (direct, the granted team itself)
//!   mo:    member on squad       (control for the role-aware arm)
//! ```

use sqlx::PgPool;
use uuid::Uuid;

async fn team(pool: &PgPool, slug: &str) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
}

async fn profile(pool: &PgPool, handle: &str) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) \
         VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
}

async fn join(pool: &PgPool, team_id: Uuid, profile_id: Uuid, role: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
    )
    .bind(team_id)
    .bind(profile_id)
    .bind(role)
    .execute(pool)
    .await?;
    Ok(())
}

async fn team_context(pool: &PgPool, owner_team: Uuid, slug: &str) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(owner_team)
    .bind(slug)
    .fetch_one(pool)
    .await
}

/// A resource homed in `context_id`, owned by `owner` (who is none of the probed principals, so
/// the owner arm of `can_modify_resource` never admits them).
async fn resource_in(pool: &PgPool, context_id: Uuid, owner: Uuid) -> sqlx::Result<Uuid> {
    let resource: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (id, title, origin_uri) \
         VALUES (uuid_generate_v7(), 'granted', '') RETURNING id",
    )
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (id, resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES (uuid_generate_v7(), $1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(resource)
    .bind(context_id)
    .bind(owner)
    .execute(pool)
    .await?;
    Ok(resource)
}

/// A full-power grant (read, write, delete, grant) on a subject, anchored to a TEAM.
async fn full_grant_to_team(
    pool: &PgPool,
    subject_table: &str,
    subject_id: Uuid,
    team_id: Uuid,
    granted_by: Uuid,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (id, subject_table, subject_id, principal_table, principal_id, \
            can_read, can_write, can_delete, can_grant, granted_by_profile_id) \
         VALUES (uuid_generate_v7(), $1, $2, 'kb_teams', $3, true, true, true, true, $4)",
    )
    .bind(subject_table)
    .bind(subject_id)
    .bind(team_id)
    .bind(granted_by)
    .execute(pool)
    .await?;
    Ok(())
}

struct Fixture {
    umbrella: Uuid,
    squad: Uuid,
    wanda: Uuid,
    walt: Uuid,
    mo: Uuid,
    owner: Uuid,
}

async fn fixture(pool: &PgPool) -> sqlx::Result<Fixture> {
    let umbrella = team(pool, "umbrella").await?;
    let squad = team(pool, "squad").await?;
    sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
        .bind(squad)
        .bind(umbrella)
        .execute(pool)
        .await?;
    let wanda = profile(pool, "wanda").await?;
    let walt = profile(pool, "walt").await?;
    let mo = profile(pool, "mo").await?;
    let owner = profile(pool, "owner").await?;
    join(pool, squad, wanda, "watcher").await?;
    join(pool, umbrella, walt, "watcher").await?;
    join(pool, squad, mo, "member").await?;
    Ok(Fixture {
        umbrella,
        squad,
        wanda,
        walt,
        mo,
        owner,
    })
}

async fn can_modify(pool: &PgPool, who: Uuid, resource: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(who)
        .bind(resource)
        .fetch_one(pool)
        .await
}

async fn explicit(
    pool: &PgPool,
    who: Uuid,
    action: &str,
    table: &str,
    subject: Uuid,
) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT profile_explicit_grant($1, $2, $3, $4)")
        .bind(who)
        .bind(action)
        .bind(table)
        .bind(subject)
        .fetch_one(pool)
        .await
}

async fn context_authorable(pool: &PgPool, who: Uuid, context: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT context_authorable_by_profile($1, $2)")
        .bind(who)
        .bind(context)
        .fetch_one(pool)
        .await
}

/// Control: the narrowed, role-aware arm holds. A watcher cannot author a context owned by their own
/// team, and an ancestor-owned context is not authorable by a descendant member.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn control_team_owned_context_authoring_is_role_aware_and_direct(
    pool: PgPool,
) -> sqlx::Result<()> {
    let f = fixture(&pool).await?;
    let squad_ctx = team_context(&pool, f.squad, "squad-ctx").await?;
    let umbrella_ctx = team_context(&pool, f.umbrella, "umbrella-ctx").await?;

    assert!(
        !context_authorable(&pool, f.wanda, squad_ctx).await?,
        "a watcher does not author their own team's context"
    );
    assert!(
        context_authorable(&pool, f.mo, squad_ctx).await?,
        "a member authors their own team's context"
    );
    assert!(
        !context_authorable(&pool, f.mo, umbrella_ctx).await?,
        "a member of a child team does not author the parent's context"
    );
    Ok(())
}

/// A watcher on the GRANTED team itself reads, and holds no write, delete or grant.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_watcher_on_the_granted_team_reads_but_does_not_mutate(pool: PgPool) -> sqlx::Result<()> {
    let f = fixture(&pool).await?;
    let elsewhere = team(&pool, "elsewhere").await?;
    let ctx = team_context(&pool, elsewhere, "elsewhere-ctx").await?;
    let resource = resource_in(&pool, ctx, f.owner).await?;
    full_grant_to_team(&pool, "kb_resources", resource, f.umbrella, f.owner).await?;

    assert!(explicit(&pool, f.walt, "read", "kb_resources", resource).await?);
    assert!(!can_modify(&pool, f.walt, resource).await?);
    for action in ["write", "delete", "grant"] {
        assert!(
            !explicit(&pool, f.walt, action, "kb_resources", resource).await?,
            "a watcher on the granted team holds no `{action}`"
        );
    }
    Ok(())
}

/// A grant on an ancestor reaches a descendant's members for READ only — at any role, including
/// `member`, because mutation does not inherit.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_ancestor_grant_reaches_descendants_for_read_only(pool: PgPool) -> sqlx::Result<()> {
    let f = fixture(&pool).await?;
    let elsewhere = team(&pool, "elsewhere").await?;
    let ctx = team_context(&pool, elsewhere, "elsewhere-ctx").await?;
    let resource = resource_in(&pool, ctx, f.owner).await?;
    full_grant_to_team(&pool, "kb_resources", resource, f.umbrella, f.owner).await?;

    for who in [f.wanda, f.mo] {
        assert!(
            explicit(&pool, who, "read", "kb_resources", resource).await?,
            "read inherits down the DAG"
        );
        assert!(!can_modify(&pool, who, resource).await?);
        for action in ["write", "delete", "grant"] {
            assert!(
                !explicit(&pool, who, action, "kb_resources", resource).await?,
                "`{action}` does not inherit from the umbrella's grant"
            );
        }
    }

    // Negative control: a profile on neither team holds nothing.
    let outsider = profile(&pool, "outsider").await?;
    assert!(!explicit(&pool, outsider, "read", "kb_resources", resource).await?);
    assert!(!can_modify(&pool, outsider, resource).await?);
    Ok(())
}

/// A write grant on a CONTEXT, anchored on an ancestor, does not let a descendant author it — so
/// the container-write cascade does not open its resources either.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_ancestor_context_grant_does_not_make_a_descendant_an_author(
    pool: PgPool,
) -> sqlx::Result<()> {
    let f = fixture(&pool).await?;
    let elsewhere = team(&pool, "elsewhere").await?;
    let ctx = team_context(&pool, elsewhere, "elsewhere-ctx").await?;
    let resource = resource_in(&pool, ctx, f.owner).await?;
    full_grant_to_team(&pool, "kb_contexts", ctx, f.umbrella, f.owner).await?;

    for who in [f.wanda, f.mo] {
        assert!(!context_authorable(&pool, who, ctx).await?);
        assert!(!can_modify(&pool, who, resource).await?);
    }
    Ok(())
}

/// The positive arm: direct members with an authoring role mutate through a team grant, and only
/// managers re-share.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn direct_authoring_members_mutate_and_only_managers_regrant(
    pool: PgPool,
) -> sqlx::Result<()> {
    let f = fixture(&pool).await?;
    let elsewhere = team(&pool, "elsewhere").await?;
    let ctx = team_context(&pool, elsewhere, "elsewhere-ctx").await?;
    let resource = resource_in(&pool, ctx, f.owner).await?;
    full_grant_to_team(&pool, "kb_resources", resource, f.squad, f.owner).await?;
    let max = profile(&pool, "max").await?;
    join(&pool, f.squad, max, "maintainer").await?;

    // mo is a direct member: write and delete, no grant.
    assert!(can_modify(&pool, f.mo, resource).await?);
    assert!(explicit(&pool, f.mo, "write", "kb_resources", resource).await?);
    assert!(explicit(&pool, f.mo, "delete", "kb_resources", resource).await?);
    assert!(!explicit(&pool, f.mo, "grant", "kb_resources", resource).await?);
    // max is a direct maintainer: all four.
    for action in ["read", "write", "delete", "grant"] {
        assert!(explicit(&pool, max, action, "kb_resources", resource).await?);
    }
    // wanda is a direct watcher: read only.
    assert!(explicit(&pool, f.wanda, "read", "kb_resources", resource).await?);
    assert!(!can_modify(&pool, f.wanda, resource).await?);

    // A soft-deleted granted team confers nothing to mutate.
    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(f.squad)
        .execute(&pool)
        .await?;
    assert!(!can_modify(&pool, f.mo, resource).await?);
    assert!(!explicit(&pool, max, "grant", "kb_resources", resource).await?);
    Ok(())
}
