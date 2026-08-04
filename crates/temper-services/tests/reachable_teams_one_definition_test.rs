#![cfg(feature = "test-db")]

//! `reachable_teams` has exactly one definition — goal `019fb881` clause 1, exercised.
//!
//! # What this is about
//!
//! One expression decides, everywhere in the schema, which teams a profile's reads and writes
//! reach:
//!
//! ```sql
//! SELECT DISTINCT a.team_id
//! FROM profile_effective_teams(p_profile) e
//! CROSS JOIN LATERAL team_ancestors(e.team_id) a
//! ```
//!
//! It expands the profile's DIRECT team memberships UP the enclosure chain, so a thing attached to
//! an ancestor reaches every member beneath it. `team_ancestors` is itself a `WITH RECURSIVE` walk,
//! run once per direct team.
//!
//! Measured 2026-08-04 against the live schema, that expression was **hand-copied verbatim into
//! nine functions** — byte-identical once normalized for whitespace and parameter name, two of them
//! merely naming the CTE `member_teams` instead of `reachable_teams`. Not drifted. Nine copies in
//! lockstep by coincidence, which is exactly the condition goal `019fb881`'s why-anchor names:
//!
//! > "keeps hand-copied CTE blocks in lockstep by reading both — where drift is caught only by an
//! > after-the-fact CI equivalence test, a coincidence enforced, not a structure guaranteed."
//!
//! # The three tiers here, and why each exists
//!
//! 1. [`the_reachable_teams_expression_has_exactly_one_home`] — the **clause-1 witness**. It scans
//!    `pg_proc` and fails if any function other than the authoritative one carries a hand-rolled
//!    copy. This is the test that **fails against pre-extraction state** (nine hits), which is what
//!    the equivalence tier structurally cannot do.
//!
//! 2. The **clause-2 equivalence tier** — one characterization test per routed function, asserting
//!    the transitive-membership behavior the expression is responsible for. These pass both before
//!    and after the extraction *by construction*; that is their job as a regression net, and it is
//!    also why on their own they evidence nothing.
//!
//! 3. [`the_equivalence_tier_actually_bites`] — the **bite probe**. It replaces the authoritative
//!    relation with a flat, direct-membership-only body and asserts the tier above goes red. Without
//!    it, a tier that passes before and after is indistinguishable from a tier that cannot fail.
//!
//! # The fixture
//!
//! ```text
//!   EPD ─▸ engineering ─▸ payroll-group ─▸ squad-two
//!                      └▸ security-it-ops        (the sibling — must stay invisible)
//! ```
//!
//! `dana` is a DIRECT member of `squad-two` only, and therefore a transitive member of
//! `payroll-group`, `engineering` and `EPD`. `outsider` belongs to nothing. Every assertion below
//! is "dana reaches it through an ancestor; outsider does not", because that difference is
//! precisely what `reachable_teams` computes and nothing else in these functions does.
//!
//! A third profile, `owner`, exists only to hold the fixtures. It is not decoration: both
//! `resources_visible_to` and `can_modify_resource` admit on `kb_resource_homes.owner_profile_id`
//! *before* any team reach is consulted, so an `outsider` who owns the fixture resource passes
//! every check for a reason that has nothing to do with the expression under test. That is not a
//! hypothetical — the first draft of this file did exactly that, and
//! `can_modify_resource_reaches_up_the_chain` failed its own negative control.

use sqlx::PgPool;
use uuid::Uuid;

/// The one expression, as a Postgres regex over whitespace-normalized `pg_get_functiondef` output.
///
/// Pinned as the WHOLE expression, not just the lateral join. A looser first pass matching only
/// `CROSS JOIN LATERAL team_ancestors` returned ten functions, two of them false positives:
/// `steward_team_contexts` walks from a *cogmap's* joined teams, and `graph_home_contexts` uses
/// `profile_effective_teams` *flat* to pick a display label. Same lateral walk, different subject —
/// neither is this expression.
const REACHABLE_TEAMS_EXPR: &str =
    r"profile_effective_teams\([a-z_]+\) e CROSS JOIN LATERAL team_ancestors\(e\.team_id\) a";

/// The single authoritative home. Every other function joins this instead of restating it.
const AUTHORITATIVE: &str = "profile_reachable_teams";

// =================================================================================================
// Tier 1 — the clause-1 witness. Fails against pre-extraction state.
// =================================================================================================

/// **Goal `019fb881` clause 1**: *"A principal-agnostic fragment shared by more than one function
/// has exactly one authoritative definition."*
///
/// This is the test that bites in the right direction on its own: before the extraction it names
/// all nine hand-copies and fails; after it, the only function carrying the expression is the one
/// that owns it.
///
/// It is deliberately a **catalog scan, not a fixed list**. A list would have to be edited by
/// whoever adds the tenth copy — which is the person the test exists to stop.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_reachable_teams_expression_has_exactly_one_home(pool: PgPool) -> sqlx::Result<()> {
    let copies: Vec<String> = sqlx::query_scalar(
        "SELECT p.proname \
           FROM pg_proc p \
           JOIN pg_namespace n ON n.oid = p.pronamespace \
           CROSS JOIN LATERAL (SELECT regexp_replace(pg_get_functiondef(p.oid), '\\s+', ' ', 'g') AS def) d \
          WHERE n.nspname = 'public' AND p.prokind = 'f' \
            AND p.proname <> $1 \
            AND d.def ~ $2 \
          ORDER BY 1",
    )
    .bind(AUTHORITATIVE)
    .bind(REACHABLE_TEAMS_EXPR)
    .fetch_all(&pool)
    .await?;

    assert!(
        copies.is_empty(),
        "{} function(s) still carry a hand-rolled copy of the reachable_teams expression \
         instead of joining {AUTHORITATIVE}(): {copies:?}\n\
         \n\
         Route it through {AUTHORITATIVE}() rather than restating it. Two copies in lockstep are \
         a coincidence, not a structure.",
        copies.len(),
    );

    Ok(())
}

/// The authoritative relation exists, and is the shape the routed functions join.
///
/// Separate from the scan above because the scan is vacuously satisfiable: a schema with **no**
/// copies and **no** authoritative home passes it. This pins the other half.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_authoritative_relation_exists_and_is_principal_parameterized(
    pool: PgPool,
) -> sqlx::Result<()> {
    let sig: Option<(String, String)> = sqlx::query_as(
        "SELECT pg_get_function_arguments(p.oid), pg_get_function_result(p.oid) \
           FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
          WHERE n.nspname = 'public' AND p.prokind = 'f' AND p.proname = $1",
    )
    .bind(AUTHORITATIVE)
    .fetch_optional(&pool)
    .await?;

    let (args, result) = sig.unwrap_or_else(|| panic!("{AUTHORITATIVE} does not exist"));

    // Negative face 2: "a shared view/SRF must never embed a specific principal". Parameterized by
    // the principal is exactly right; a zero-argument relation would have baked one in.
    assert!(
        args.contains("uuid"),
        "{AUTHORITATIVE} must take the principal as a parameter, got args: {args}"
    );
    assert!(
        result.contains("team_id") && result.contains("uuid"),
        "{AUTHORITATIVE} must return a joinable team_id relation, got: {result}"
    );

    Ok(())
}

// =================================================================================================
// The fixture — the org enclosure hierarchy the expression is actually about.
// =================================================================================================

struct Org {
    engineering: Uuid,
    squad_two: Uuid,
    security_it_ops: Uuid,
    /// Direct member of `squad_two` only; transitive member of everything enclosing it.
    dana: Uuid,
    /// Belongs to nothing and is granted nothing — the negative control.
    ///
    /// Kept strictly free of ownership: `kb_resource_homes.owner_profile_id` is itself an admitting
    /// arm in both `resources_visible_to` and `can_modify_resource`, so an outsider who *owns* the
    /// fixture resource passes every check for a reason that has nothing to do with team reach.
    /// That is what [`Org::owner`] is for.
    outsider: Uuid,
    /// Owns the fixture resources and issues the grants. In no team, so it contributes no reach —
    /// it exists only so `outsider` can stay a clean negative control.
    owner: Uuid,
}

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

async fn encloses(pool: &PgPool, parent: Uuid, child: Uuid) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await?;
    Ok(())
}

async fn org(pool: &PgPool) -> sqlx::Result<Org> {
    let epd = team(pool, "epd").await?;
    let engineering = team(pool, "engineering").await?;
    let payroll_group = team(pool, "payroll-group").await?;
    let squad_two = team(pool, "squad-two").await?;
    let security_it_ops = team(pool, "security-it-ops").await?;

    encloses(pool, epd, engineering).await?;
    encloses(pool, engineering, payroll_group).await?;
    encloses(pool, payroll_group, squad_two).await?;
    encloses(pool, engineering, security_it_ops).await?;

    let dana = profile(pool, "dana").await?;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(squad_two)
    .bind(dana)
    .execute(pool)
    .await?;

    let outsider = profile(pool, "outsider").await?;
    let owner = profile(pool, "owner").await?;

    Ok(Org {
        engineering,
        squad_two,
        security_it_ops,
        dana,
        outsider,
        owner,
    })
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

async fn resource_in(
    pool: &PgPool,
    context_id: Uuid,
    owner: Uuid,
    title: &str,
) -> sqlx::Result<Uuid> {
    let resource: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (id, title, origin_uri) \
         VALUES (uuid_generate_v7(), $1, '') RETURNING id",
    )
    .bind(title)
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

/// A cogmap joined to `team`. The telos resource is required (NOT NULL) and is otherwise irrelevant.
async fn cogmap_joined_to(pool: &PgPool, team_id: Uuid, name: &str) -> sqlx::Result<Uuid> {
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (id, title, origin_uri) \
         VALUES (uuid_generate_v7(), $1, '') RETURNING id",
    )
    .bind(format!("{name}-telos"))
    .fetch_one(pool)
    .await?;
    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (id, name, telos_resource_id) \
         VALUES (uuid_generate_v7(), $1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team_id)
        .execute(pool)
        .await?;
    Ok(cogmap)
}

// =================================================================================================
// Tier 2 — the clause-2 equivalence net. One characterization per routed function.
//
// Each asserts the SAME property from a different door: reach inherits UP the enclosure chain for
// dana and reaches the outsider not at all. That property is what `reachable_teams` computes, so
// each of these is a distinct witness that routing a given function through the authoritative
// relation preserved its result set.
// =================================================================================================

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn contexts_readable_by_reaches_up_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let eng = team_context(&pool, o.engineering, "engineering-ctx").await?;
    let sec = team_context(&pool, o.security_it_ops, "security-ctx").await?;

    let dana: Vec<Uuid> = sqlx::query_scalar("SELECT context_id FROM contexts_readable_by($1)")
        .bind(o.dana)
        .fetch_all(&pool)
        .await?;
    let outsider: Vec<Uuid> = sqlx::query_scalar("SELECT context_id FROM contexts_readable_by($1)")
        .bind(o.outsider)
        .fetch_all(&pool)
        .await?;

    assert!(dana.contains(&eng), "an ancestor team's own context reads");
    assert!(!dana.contains(&sec), "a sibling team's context never reads");
    assert!(!outsider.contains(&eng), "the outsider reaches nothing");
    Ok(())
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn resources_visible_to_reaches_up_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let eng = team_context(&pool, o.engineering, "engineering-ctx").await?;
    let sec = team_context(&pool, o.security_it_ops, "security-ctx").await?;
    let reachable = resource_in(&pool, eng, o.owner, "reachable").await?;
    let sideways = resource_in(&pool, sec, o.owner, "sideways").await?;

    let seen: Vec<Uuid> = sqlx::query_scalar("SELECT resource_id FROM resources_visible_to($1)")
        .bind(o.dana)
        .fetch_all(&pool)
        .await?;

    assert!(
        seen.contains(&reachable),
        "resource in an ancestor's context"
    );
    assert!(!seen.contains(&sideways), "resource in a sibling's context");
    Ok(())
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn cogmap_visible_maps_reaches_up_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let eng_map = cogmap_joined_to(&pool, o.engineering, "eng-map").await?;
    let sec_map = cogmap_joined_to(&pool, o.security_it_ops, "sec-map").await?;

    let seen: Vec<Uuid> = sqlx::query_scalar("SELECT * FROM cogmap_visible_maps($1)")
        .bind(o.dana)
        .fetch_all(&pool)
        .await?;

    assert!(seen.contains(&eng_map), "map joined to an ancestor team");
    assert!(!seen.contains(&sec_map), "map joined to a sibling team");
    Ok(())
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn cogmap_readable_by_profile_reaches_up_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let eng_map = cogmap_joined_to(&pool, o.engineering, "eng-map").await?;
    let sec_map = cogmap_joined_to(&pool, o.security_it_ops, "sec-map").await?;

    let reads = |p: Uuid, m: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>("SELECT cogmap_readable_by_profile($1, $2)")
                .bind(p)
                .bind(m)
                .fetch_one(&pool)
                .await
        }
    };

    assert!(reads(o.dana, eng_map).await?, "ancestor-joined map reads");
    assert!(
        !reads(o.dana, sec_map).await?,
        "sibling-joined map does not"
    );
    assert!(
        !reads(o.outsider, eng_map).await?,
        "the outsider reads nothing"
    );
    Ok(())
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn cogmap_list_rows_reaches_up_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let eng_map = cogmap_joined_to(&pool, o.engineering, "eng-map").await?;
    let sec_map = cogmap_joined_to(&pool, o.security_it_ops, "sec-map").await?;

    let listed: Vec<Uuid> = sqlx::query_scalar("SELECT cogmap_id FROM cogmap_list_rows($1)")
        .bind(o.dana)
        .fetch_all(&pool)
        .await?;

    assert!(listed.contains(&eng_map), "ancestor-joined map is listed");
    assert!(!listed.contains(&sec_map), "sibling-joined map is not");
    Ok(())
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn graph_home_cogmaps_reaches_up_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let eng_map = cogmap_joined_to(&pool, o.engineering, "eng-map").await?;
    let sec_map = cogmap_joined_to(&pool, o.security_it_ops, "sec-map").await?;

    let listed: Vec<Uuid> = sqlx::query_scalar("SELECT cogmap_id FROM graph_home_cogmaps($1)")
        .bind(o.dana)
        .fetch_all(&pool)
        .await?;

    assert!(
        listed.contains(&eng_map),
        "ancestor-joined map is in the graph view"
    );
    assert!(!listed.contains(&sec_map), "sibling-joined map is not");
    Ok(())
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn edges_visible_to_reaches_up_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let eng = team_context(&pool, o.engineering, "engineering-ctx").await?;
    let a = resource_in(&pool, eng, o.owner, "a").await?;
    let b = resource_in(&pool, eng, o.owner, "b").await?;

    // `edge_kind` is the enum; `label` is free text. Both event references are NOT NULL — reuse an
    // event the migrations already emitted.
    let edge: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_edges \
           (id, source_table, source_id, target_table, target_id, edge_kind, label, \
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         SELECT uuid_generate_v7(), 'kb_resources', $1, 'kb_resources', $2, 'near', 'relates_to', \
                'kb_contexts', $3, e.id, e.id \
           FROM kb_events e ORDER BY e.id LIMIT 1 \
         RETURNING id",
    )
    .bind(a)
    .bind(b)
    .bind(eng)
    .fetch_one(&pool)
    .await?;

    let dana_sees: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM edges_visible_to($1) e WHERE e.edge_id = $2)",
    )
    .bind(o.dana)
    .bind(edge)
    .fetch_one(&pool)
    .await?;
    let outsider_sees: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM edges_visible_to($1) e WHERE e.edge_id = $2)",
    )
    .bind(o.outsider)
    .bind(edge)
    .fetch_one(&pool)
    .await?;

    assert!(
        dana_sees,
        "edge homed in an ancestor team's context is visible"
    );
    assert!(!outsider_sees, "the outsider sees no edge");
    Ok(())
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn profile_explicit_grant_reaches_up_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let subject = team_context(&pool, o.security_it_ops, "granted-ctx").await?;

    // Granted to ENGINEERING — an ancestor of dana's direct team, not her direct team.
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (id, subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES (uuid_generate_v7(), 'kb_contexts', $1, 'kb_teams', $2, true, $3)",
    )
    .bind(subject)
    .bind(o.engineering)
    .bind(o.owner)
    .execute(&pool)
    .await?;

    let dana: bool =
        sqlx::query_scalar("SELECT profile_explicit_grant($1, 'read', 'kb_contexts', $2)")
            .bind(o.dana)
            .bind(subject)
            .fetch_one(&pool)
            .await?;
    let outsider: bool =
        sqlx::query_scalar("SELECT profile_explicit_grant($1, 'read', 'kb_contexts', $2)")
            .bind(o.outsider)
            .bind(subject)
            .fetch_one(&pool)
            .await?;

    assert!(
        dana,
        "a grant to an ancestor team reaches a member beneath it"
    );
    assert!(!outsider, "the outsider holds no grant");
    Ok(())
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn can_modify_resource_reaches_up_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let sec = team_context(&pool, o.security_it_ops, "security-ctx").await?;
    let resource = resource_in(&pool, sec, o.owner, "granted").await?;

    // A WRITE grant to ENGINEERING, an ancestor of dana's direct team.
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (id, subject_table, subject_id, principal_table, principal_id, can_read, can_write, granted_by_profile_id) \
         VALUES (uuid_generate_v7(), 'kb_resources', $1, 'kb_teams', $2, true, true, $3)",
    )
    .bind(resource)
    .bind(o.engineering)
    .bind(o.owner)
    .execute(&pool)
    .await?;

    let dana: bool = sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(o.dana)
        .bind(resource)
        .fetch_one(&pool)
        .await?;
    let outsider: bool = sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(o.outsider)
        .bind(resource)
        .fetch_one(&pool)
        .await?;

    assert!(
        dana,
        "a write grant to an ancestor team reaches a member beneath it"
    );
    assert!(!outsider, "the outsider may modify nothing");
    Ok(())
}

// =================================================================================================
// Tier 3 — the bite probe.
// =================================================================================================

/// **The tier-2 net passes both before and after the extraction — by construction.** That is what a
/// behavior-preservation witness *is*, and it is also why, on its own, it evidences nothing: a net
/// that cannot fail is indistinguishable from one that holds.
///
/// So: break the authoritative relation in the one way that matters — make it FLAT, direct
/// memberships only, dropping the ancestor walk — and assert every door goes dark for dana while
/// staying correct for a direct member. The break is the exact inverse of the invariant the tier
/// asserts (reach inherits UP), so a green tier-2 against a flattened relation would mean the tier
/// is measuring something else.
///
/// Runs inside the test's own database, so the redefinition dies with it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_equivalence_tier_actually_bites(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let eng = team_context(&pool, o.engineering, "engineering-ctx").await?;
    let own = team_context(&pool, o.squad_two, "squad-two-ctx").await?;
    let eng_map = cogmap_joined_to(&pool, o.engineering, "eng-map").await?;

    let reads_ctx = |c: Uuid| {
        let pool = pool.clone();
        let dana = o.dana;
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM contexts_readable_by($1) c WHERE c.context_id = $2)",
            )
            .bind(dana)
            .bind(c)
            .fetch_one(&pool)
            .await
        }
    };
    let reads_map = |m: Uuid| {
        let pool = pool.clone();
        let dana = o.dana;
        async move {
            sqlx::query_scalar::<_, bool>("SELECT cogmap_readable_by_profile($1, $2)")
                .bind(dana)
                .bind(m)
                .fetch_one(&pool)
                .await
        }
    };

    // Baseline: the ancestor arm and the direct arm both reach.
    assert!(
        reads_ctx(eng).await?,
        "precondition: ancestor context reads"
    );
    assert!(
        reads_ctx(own).await?,
        "precondition: own team's context reads"
    );
    assert!(
        reads_map(eng_map).await?,
        "precondition: ancestor map reads"
    );

    // Flatten the one definition: direct memberships only, no ancestor walk.
    sqlx::query(
        "CREATE OR REPLACE FUNCTION profile_reachable_teams(p_profile uuid) \
           RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS \
         $$ SELECT e.team_id FROM profile_effective_teams(p_profile) e $$",
    )
    .execute(&pool)
    .await?;

    // The ancestor arm must go dark everywhere the extraction routed.
    assert!(
        !reads_ctx(eng).await?,
        "BITE PROBE FAILED: contexts_readable_by still reached an ancestor team's context with a \
         flattened reachable-teams relation. Either that function is not routed through \
         {AUTHORITATIVE}, or it carries its own copy — in which case the clause-1 witness is the \
         test to look at."
    );
    assert!(
        !reads_map(eng_map).await?,
        "BITE PROBE FAILED: cogmap_readable_by_profile still reached an ancestor-joined map with a \
         flattened reachable-teams relation."
    );

    // ...while the DIRECT arm still works. This is what makes the probe a probe and not a
    // demonstration that breaking things breaks things: the flattening removed exactly one axis.
    assert!(
        reads_ctx(own).await?,
        "the flattened relation should still admit dana's DIRECT team — if this fails the probe \
         broke more than the ancestor walk and proves less than it claims"
    );

    Ok(())
}
