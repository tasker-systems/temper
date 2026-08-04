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
//! # The tiers here, and why each exists
//!
//! 1. [`the_reachable_teams_expression_has_exactly_one_home`] — the **clause-1 witness**. It scans
//!    `pg_proc` and fails if any function other than the authoritative one carries a hand-rolled
//!    copy. This is the test that **fails against pre-extraction state** (nine hits), which is what
//!    the equivalence tier structurally cannot do.
//!    [`the_gate_catches_a_copy_written_with_different_aliases`] is that gate's own bite probe: it
//!    plants copies the gate has historically been blind to and asserts they are caught.
//!
//! 2. The **clause-2 equivalence tier** — one characterization test per routed function, asserting
//!    the transitive-membership behavior the expression is responsible for. These pass both before
//!    and after the extraction *by construction*; that is their job as a regression net, and it is
//!    also why on their own they evidence nothing.
//!
//! 3. [`every_fold_site_is_load_bearing`] — the **per-function mutation gate**. For each of the
//!    nine it empties *that function's own* fold site and requires the assertion naming it to die.
//!
//! 4. [`the_equivalence_tier_actually_bites`] — the **bite probe**. It flattens the authoritative
//!    relation to direct-membership-only and requires all nine doors to go dark while a
//!    direct-membership door stays lit.
//!
//! **Why 3 and 4 are both needed, and why 3 exists at all.** Tier 4 alone was not enough: a function
//! can join the shared relation faithfully (so flattening it propagates) while the test *named after
//! that function* rides some **other** function's already-covered CTE. Adversarial review on
//! 2026-08-04 found exactly that for four of the nine — `resources_visible_to`, `edges_visible_to`,
//! `cogmap_list_rows` and `graph_home_cogmaps` each passed with their own fold site deleted, or
//! widened to every team in the database. Tier 3 is the standard that catches it, and the fixtures
//! in tier 2 were rebuilt to enter through arms each function's own CTE actually feeds.
//!
//! The general lesson, which outlives this file: **a green test is evidence that something passed,
//! never that the thing you named is what made it pass.**
//!
//! # The fixture
//!
//! ```text
//!   EPD ─▸ engineering ─▸ payroll-group ─▸ squad-two
//!                      └▸ security-it-ops        (the sibling — must stay invisible)
//! ```
//!
//! `dana` is a DIRECT member of `squad-two` only, and therefore a transitive member of
//! `payroll-group`, `engineering` and `EPD`. Every assertion below is "dana reaches it through an
//! ancestor; outsider does not", because that difference is precisely what `reachable_teams`
//! computes and nothing else in these functions does.
//!
//! **`outsider` does NOT belong to nothing, and this file used to say it did.** Corrected 2026-08-04
//! after adversarial review. Inserting a bare `kb_profiles` row auto-creates a `personal-<handle>`
//! team membership, so measured against the live schema `profile_reachable_teams(outsider)` returns
//! **2** teams and `cogmap_visible_maps(outsider)` returns **1** — the L0 kernel map, which is
//! root-team-joined and public by design.
//!
//! The negative assertions still hold, and hold for the right reason: `outsider` reaches none of
//! *these fixtures*, because none is attached to a personal team or to the root team. But it is a
//! negative control over the fixture set, **not** a principal that reaches nothing — and a comment
//! claiming the latter would send the next reader looking for a bug when a future assertion about
//! the L0 kernel comes back true.
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

/// The **alias-blind** form of the same question, and the one the gate actually runs on.
///
/// [`REACHABLE_TEAMS_EXPR`] pins the literal aliases `e` and `a`, which is what the nine original
/// copies happened to use. That makes it useless for the case this witness exists to catch — the
/// *tenth* copy, written later by someone who has never read this file and picks `pet`/`anc`:
///
/// ```sql
/// SELECT DISTINCT anc.team_id
/// FROM profile_effective_teams(p_profile) pet
/// CROSS JOIN LATERAL team_ancestors(pet.team_id) anc
/// ```
///
/// That is the same fragment and the regex does not see it. So the gate asks the structural
/// question instead: **does this function reference both primitives at all?** Measured against the
/// live schema, that predicate exactly characterizes "is a `reachable_teams` copy" — the functions
/// touching only one are all legitimately doing something else (`graph_home_contexts` uses
/// `profile_effective_teams` flat for a display label; `graph_region_territories`,
/// `resources_in_team_scope`, `steward_team_contexts` and `vis_team` walk `team_ancestors` from a
/// *team*, never from a profile).
///
/// **It can false-positive**, on a future function that legitimately needs both primitives for
/// unrelated reasons. That direction is deliberate: on a gate protecting who-can-see-what, a
/// spurious failure costs a conversation and a missed copy costs a silent divergence in an
/// authorization predicate.
///
/// **Corrected 2026-08-04 after adversarial review: the paragraph above was once wrong about the
/// direction, which is the worst way for a gate's doc to be wrong.** The first version required
/// both `profile_effective_teams` *and* `team_ancestors` to appear, and claimed it erred only
/// toward false positives. It did not. A copy that **inlines** `profile_effective_teams`'s two-line
/// body instead of calling it —
///
/// ```sql
/// SELECT DISTINCT anc.team_id
/// FROM (SELECT tm.team_id FROM kb_team_members tm JOIN kb_teams t ON t.id = tm.team_id
///        WHERE tm.profile_id = p_profile AND t.is_active) pet
/// CROSS JOIN LATERAL team_ancestors(pet.team_id) anc
/// ```
///
/// — names only `team_ancestors`, so the `AND` failed and the gate stayed green. Planted as a
/// probe: caught nothing. And inlining a two-line helper is not exotic; it is an ordinary thing to
/// do while writing SQL.
///
/// So the reach half is now a disjunction: **called or inlined.** Measured against the live schema,
/// the broadened predicate flags **zero** functions, so it costs no false positive today —
/// `steward_team_contexts`, `vis_team`, `graph_region_territories` and `resources_in_team_scope`
/// all walk `team_ancestors` from a *team* and touch neither reach form.
///
/// The lesson worth keeping: **a stated one-directional error is what stops anyone re-examining a
/// gate.** The claim was more harmful than the hole.
const REACH_FORMS: (&str, &str) = ("%profile_effective_teams%", "%kb_team_members%");
const ANCESTOR_WALK: &str = "%team_ancestors%";

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
///
/// The gate runs on [`REACH_FORMS`] × [`ANCESTOR_WALK`], not on [`REACHABLE_TEAMS_EXPR`]: the regex
/// pins the literal aliases the nine original copies happened to use, so it would miss a later copy
/// that spells them differently. Read [`REACH_FORMS`]'s doc for why the structural question is the
/// right one, and for the correction that made its reach half a disjunction. The regex is still
/// applied, as a **strictly weaker cross-check** — if it ever matches something the structural scan
/// does not, one of the two is wrong and the test says so rather than quietly preferring either.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_reachable_teams_expression_has_exactly_one_home(pool: PgPool) -> sqlx::Result<()> {
    let (calls_pet, inlines_pet) = REACH_FORMS;

    // The gate: alias-blind, and blind to whether `profile_effective_teams` is called or inlined.
    let copies: Vec<String> = sqlx::query_scalar(
        "SELECT p.proname \
           FROM pg_proc p \
           JOIN pg_namespace n ON n.oid = p.pronamespace \
           CROSS JOIN LATERAL (SELECT pg_get_functiondef(p.oid) AS def) d \
          WHERE n.nspname = 'public' AND p.prokind = 'f' \
            AND p.proname NOT IN ($1, 'profile_effective_teams', 'team_ancestors') \
            AND d.def LIKE $2 \
            AND (d.def LIKE $3 OR d.def LIKE $4) \
          ORDER BY 1",
    )
    .bind(AUTHORITATIVE)
    .bind(ANCESTOR_WALK)
    .bind(calls_pet)
    .bind(inlines_pet)
    .fetch_all(&pool)
    .await?;

    assert!(
        copies.is_empty(),
        "{} function(s) walk `team_ancestors` over a profile's own memberships instead of joining \
         {AUTHORITATIVE}(): {copies:?}\n\
         \n\
         Route it through {AUTHORITATIVE}() rather than restating the walk. Copies in lockstep are \
         a coincidence, not a structure — this fragment had NINE of them, byte-identical and \
         undrifted, right up until it mattered.\n\
         \n\
         If a function here genuinely needs both for an unrelated reason, this gate has \
         false-positived: say so explicitly and narrow it, rather than deleting it.",
        copies.len(),
    );

    // Cross-check: the exact-expression regex is a subset of the structural scan. It matching
    // something the scan missed would mean the scan's premise is wrong, not that a copy slipped.
    let by_regex: Vec<String> = sqlx::query_scalar(
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
        by_regex.is_empty(),
        "the exact-expression regex matched {by_regex:?} while the structural scan came back \
         clean — the two disagree, so the structural predicate is no longer a superset. Fix the \
         predicate, do not silence either half."
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

/// The gate above claims it catches the **tenth** copy — the one written later, by someone who has
/// not read this file, with whatever aliases they happen to pick. That claim needs evidence, or it
/// is just a comment.
///
/// So: plant decoys and assert the gate names them. **Two** decoys, because the gate has been wrong
/// in two different ways and each correction needs its own standing evidence:
///
/// 1. **Alias-renamed** — shares no alias with the nine originals, so a gate keyed to `e`/`a`
///    cannot see it.
/// 2. **Inlined reach** — inlines `profile_effective_teams`'s body rather than calling it, so a
///    gate requiring that *name* cannot see it. Found by adversarial review after the first
///    correction shipped, and planted here so the second correction cannot silently regress.
///
/// Runs in the test's own database, so the decoys die with it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_gate_catches_a_copy_written_with_different_aliases(pool: PgPool) -> sqlx::Result<()> {
    let (calls_pet, inlines_pet) = REACH_FORMS;

    sqlx::query(
        "CREATE OR REPLACE FUNCTION decoy_tenth_copy(p_profile uuid) \
           RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS \
         $$ SELECT DISTINCT anc.team_id \
              FROM profile_effective_teams(p_profile) pet \
              CROSS JOIN LATERAL team_ancestors(pet.team_id) anc $$",
    )
    .execute(&pool)
    .await?;

    // Decoy 2: the same reach, with `profile_effective_teams` INLINED rather than called. Its body
    // is that function's body verbatim, so it is the same relation by construction.
    sqlx::query(
        "CREATE OR REPLACE FUNCTION decoy_inlined_reach(p_profile uuid) \
           RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS \
         $$ SELECT DISTINCT anc.team_id \
              FROM (SELECT tm.team_id FROM kb_team_members tm \
                      JOIN kb_teams t ON t.id = tm.team_id \
                     WHERE tm.profile_id = p_profile AND t.is_active) pet \
              CROSS JOIN LATERAL team_ancestors(pet.team_id) anc $$",
    )
    .execute(&pool)
    .await?;

    let structural: Vec<String> = sqlx::query_scalar(
        "SELECT p.proname FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
           CROSS JOIN LATERAL (SELECT pg_get_functiondef(p.oid) AS def) d \
          WHERE n.nspname = 'public' AND p.prokind = 'f' \
            AND p.proname NOT IN ($1, 'profile_effective_teams', 'team_ancestors') \
            AND d.def LIKE $2 AND (d.def LIKE $3 OR d.def LIKE $4)",
    )
    .bind(AUTHORITATIVE)
    .bind(ANCESTOR_WALK)
    .bind(calls_pet)
    .bind(inlines_pet)
    .fetch_all(&pool)
    .await?;

    assert!(
        structural.contains(&"decoy_tenth_copy".to_string()),
        "the structural gate did NOT catch an alias-renamed copy — it caught {structural:?}. \
         The gate cannot do the job its doc claims."
    );
    assert!(
        structural.contains(&"decoy_inlined_reach".to_string()),
        "the structural gate did NOT catch a copy that INLINES profile_effective_teams — it caught \
         {structural:?}. This is the exact hole adversarial review found on 2026-08-04: a gate that \
         requires the NAME is blind to the body."
    );

    // And the half that proves the strengthening was necessary rather than decorative: the
    // alias-pinned regex is blind to this decoy.
    let by_regex: Vec<String> = sqlx::query_scalar(
        "SELECT p.proname FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
           CROSS JOIN LATERAL (SELECT regexp_replace(pg_get_functiondef(p.oid), '\\s+', ' ', 'g') AS def) d \
          WHERE n.nspname = 'public' AND p.prokind = 'f' AND p.proname <> $1 AND d.def ~ $2",
    )
    .bind(AUTHORITATIVE)
    .bind(REACHABLE_TEAMS_EXPR)
    .fetch_all(&pool)
    .await?;

    assert!(
        !by_regex.contains(&"decoy_tenth_copy".to_string()),
        "the alias-pinned regex unexpectedly matched the decoy, so it is not actually blind to \
         alias renaming — the stated reason for the structural gate is wrong and this test is \
         asserting the wrong thing."
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
    /// The negative control **over this file's fixtures** — not a principal that reaches nothing.
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

/// A read-grant on a RESOURCE subject, anchored to a TEAM principal.
///
/// This is the arm that routes through `resources_visible_to`'s own `reachable_teams` CTE, which is
/// why it exists — a context-homed resource enters through `contexts_readable_by` instead and
/// witnesses nothing about this function's fold.
async fn grant_resource_to_team(
    pool: &PgPool,
    resource_id: Uuid,
    team_id: Uuid,
    granted_by: Uuid,
    can_write: bool,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (id, subject_table, subject_id, principal_table, principal_id, can_read, can_write, granted_by_profile_id) \
         VALUES (uuid_generate_v7(), 'kb_resources', $1, 'kb_teams', $2, true, $3, $4)",
    )
    .bind(resource_id)
    .bind(team_id)
    .bind(can_write)
    .bind(granted_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// An edge homed in a COGMAP, between two cogmap endpoints.
///
/// Every gate this edge passes — home anchor, source, target — resolves through
/// `edges_visible_to`'s `readable_cogmaps` CTE, which is the only consumer of that function's
/// folded `reachable_teams`. A context-homed edge with resource endpoints (the original fixture)
/// resolves entirely through `contexts_readable_by` and `resources_visible_to` instead, and so
/// never touches the fold at all.
async fn cogmap_homed_edge(
    pool: &PgPool,
    home: Uuid,
    source: Uuid,
    target: Uuid,
) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO kb_edges \
           (id, source_table, source_id, target_table, target_id, edge_kind, label, \
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         SELECT uuid_generate_v7(), 'kb_cogmaps', $1, 'kb_cogmaps', $2, 'near', 'relates_to', \
                'kb_cogmaps', $3, e.id, e.id \
           FROM kb_events e ORDER BY e.id LIMIT 1 \
         RETURNING id",
    )
    .bind(source)
    .bind(target)
    .bind(home)
    .fetch_one(pool)
    .await
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
    assert!(
        !outsider.contains(&eng),
        "the outsider does not reach an ancestor-team context"
    );
    Ok(())
}

/// **This test must exercise `resources_visible_to`'s OWN folded CTE, and originally did not.**
///
/// The first version asserted only on a context-homed resource. That resource is admitted by the
/// arm `FROM contexts_readable_by(p_profile) rc JOIN kb_resource_homes h …` — a *different*
/// function's CTE, already covered by its own test. The local `reachable_teams` CTE feeds only the
/// team-anchored grant arm and the two cogmap arms, none of which the fixture built. Measured: the
/// fold's line could be deleted entirely, or widened to every team in the database, and this test
/// stayed green.
///
/// The `team_granted` fixture below closes that: a team-anchored READ grant to **engineering**,
/// which is reachable by dana only through the ancestor walk, and which enters via the arm that
/// actually joins this function's own CTE. `mutation_probes` enforces that it stays that way.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn resources_visible_to_reaches_up_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let eng = team_context(&pool, o.engineering, "engineering-ctx").await?;
    let sec = team_context(&pool, o.security_it_ops, "security-ctx").await?;
    let reachable = resource_in(&pool, eng, o.owner, "reachable").await?;
    let sideways = resource_in(&pool, sec, o.owner, "sideways").await?;

    // Homed where dana cannot reach it by context, and admitted ONLY by a team-anchored grant to an
    // ancestor team — i.e. through this function's own `reachable_teams` CTE.
    let team_granted = resource_in(&pool, sec, o.owner, "team-granted").await?;
    grant_resource_to_team(&pool, team_granted, o.engineering, o.owner, false).await?;

    let seen: Vec<Uuid> = sqlx::query_scalar("SELECT resource_id FROM resources_visible_to($1)")
        .bind(o.dana)
        .fetch_all(&pool)
        .await?;

    assert!(
        seen.contains(&reachable),
        "resource in an ancestor's context"
    );
    assert!(
        seen.contains(&team_granted),
        "resource admitted by a team-anchored grant on an ANCESTOR team — this is the assertion \
         that rides this function's own folded CTE"
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
        "the outsider does not read an ancestor-joined map"
    );
    Ok(())
}

/// **`cogmap_id` is the wrong column to assert on here, and asserting only it was the defect.**
///
/// The visible row set comes from `cogmap_visible_maps(p_profile)` — a different function, with its
/// own test. This function's folded `member_teams` CTE feeds *only* the `owner_ref`
/// (`min(mt.slug)`) and `team_ids` (`array_agg(…) FILTER (… IN (SELECT team_id FROM member_teams))`)
/// projections. So a test reading only `cogmap_id` passes with the fold's line deleted.
///
/// Measured consequence of emptying it: `owner_ref` flips from `+engineering` to the universal
/// `temper` marker and `team_ids` empties — a real behavioural change no assertion observed.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn cogmap_list_rows_reaches_up_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let eng_map = cogmap_joined_to(&pool, o.engineering, "eng-map").await?;
    let sec_map = cogmap_joined_to(&pool, o.security_it_ops, "sec-map").await?;

    let rows: Vec<(Uuid, String, Vec<Uuid>)> =
        sqlx::query_as("SELECT cogmap_id, owner_ref, team_ids FROM cogmap_list_rows($1)")
            .bind(o.dana)
            .fetch_all(&pool)
            .await?;

    let listed: Vec<Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(listed.contains(&eng_map), "ancestor-joined map is listed");
    assert!(!listed.contains(&sec_map), "sibling-joined map is not");

    // The assertions that ride this function's OWN folded CTE.
    let eng_row = rows
        .iter()
        .find(|r| r.0 == eng_map)
        .expect("the ancestor-joined map must be listed");
    assert_eq!(
        eng_row.1, "+engineering",
        "owner_ref must name the ANCESTOR team that made the map visible, not the universal \
         `temper` marker — this projection is the only consumer of this function's member_teams CTE"
    );
    assert!(
        eng_row.2.contains(&o.engineering),
        "team_ids must carry the ancestor team, got {:?}",
        eng_row.2
    );
    Ok(())
}

/// Same defect and same fix as [`cogmap_list_rows_reaches_up_the_chain`] — see its doc. The row set
/// comes from `cogmap_visible_maps`; only `owner_ref` and `team_ids` ride the local fold.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn graph_home_cogmaps_reaches_up_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let eng_map = cogmap_joined_to(&pool, o.engineering, "eng-map").await?;
    let sec_map = cogmap_joined_to(&pool, o.security_it_ops, "sec-map").await?;

    let rows: Vec<(Uuid, String, Vec<Uuid>)> =
        sqlx::query_as("SELECT cogmap_id, owner_ref, team_ids FROM graph_home_cogmaps($1)")
            .bind(o.dana)
            .fetch_all(&pool)
            .await?;

    let listed: Vec<Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(
        listed.contains(&eng_map),
        "ancestor-joined map is in the graph view"
    );
    assert!(!listed.contains(&sec_map), "sibling-joined map is not");

    let eng_row = rows
        .iter()
        .find(|r| r.0 == eng_map)
        .expect("the ancestor-joined map must be listed");
    assert_eq!(
        eng_row.1, "+engineering",
        "owner_ref must name the ancestor team, not the universal `temper` marker"
    );
    assert!(
        eng_row.2.contains(&o.engineering),
        "team_ids must carry the ancestor team, got {:?}",
        eng_row.2
    );
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

    // The context-homed edge above resolves entirely through `readable_contexts`
    // (= `contexts_readable_by`) and `vis` (= `resources_visible_to`) — both other functions, both
    // separately tested. This function's OWN folded CTE feeds only `readable_cogmaps`, so without
    // the cogmap-homed edge below the fold's line here could be deleted with this test still green.
    let eng_map = cogmap_joined_to(&pool, o.engineering, "eng-map").await?;
    let other_map = cogmap_joined_to(&pool, o.engineering, "eng-map-2").await?;
    let cogmap_edge = cogmap_homed_edge(&pool, eng_map, eng_map, other_map).await?;

    let dana_sees_cogmap_edge: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM edges_visible_to($1) e WHERE e.edge_id = $2)",
    )
    .bind(o.dana)
    .bind(cogmap_edge)
    .fetch_one(&pool)
    .await?;

    assert!(
        dana_sees_cogmap_edge,
        "an edge homed in an ancestor-team-joined COGMAP, with cogmap endpoints, must be visible — \
         every gate it passes routes through this function's own readable_cogmaps/member CTE, \
         which is what makes this the assertion that rides the fold"
    );
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

/// One fixture reaching every arm the nine folds actually feed, plus the SQL form of each
/// characterization's assertion.
///
/// **Shared by both tier-3 gates, deliberately.** They ask different questions — one empties a
/// single function's fold site, the other flattens the shared relation for all nine — but they ask
/// them of the *same* nine claims. Two copies of this list would drift, which is the disease this
/// entire file exists to treat; keeping one means a probe fixed for one gate is fixed for both.
///
/// If a probe stops matching the assertion its characterization test makes, both gates silently
/// start measuring the wrong thing. Change the probe and the test together, or neither.
async fn fold_probes(pool: &PgPool, o: &Org) -> sqlx::Result<Vec<(&'static str, String)>> {
    let eng_ctx = team_context(pool, o.engineering, "engineering-ctx").await?;
    let sec_ctx = team_context(pool, o.security_it_ops, "security-ctx").await?;
    let eng_map = cogmap_joined_to(pool, o.engineering, "eng-map").await?;
    let other_map = cogmap_joined_to(pool, o.engineering, "eng-map-2").await?;
    let cogmap_edge = cogmap_homed_edge(pool, eng_map, eng_map, other_map).await?;

    // These two MUST be homed in a context dana cannot read by any route, or they are admitted by
    // the `contexts_readable_by` arm and the probe measures a different function's CTE. `sec_ctx`
    // will not do — the `profile_explicit_grant` probe below grants it to engineering, which makes
    // it readable and would silently re-route the admission. That is not hypothetical: the first
    // version of this fixture used `sec_ctx` and the mutation gate reported `resources_visible_to`
    // as inert for exactly this reason, which is the gate working.
    let sealed_ctx = team_context(pool, o.security_it_ops, "sealed-ctx").await?;
    let team_read = resource_in(pool, sealed_ctx, o.owner, "team-read").await?;
    grant_resource_to_team(pool, team_read, o.engineering, o.owner, false).await?;
    let team_write = resource_in(pool, sealed_ctx, o.owner, "team-write").await?;
    grant_resource_to_team(pool, team_write, o.engineering, o.owner, true).await?;

    sqlx::query(
        "INSERT INTO kb_access_grants \
           (id, subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES (uuid_generate_v7(), 'kb_contexts', $1, 'kb_teams', $2, true, $3)",
    )
    .bind(sec_ctx)
    .bind(o.engineering)
    .bind(o.owner)
    .execute(pool)
    .await?;

    let d = o.dana;
    Ok(vec![
        (
            "contexts_readable_by",
            format!("SELECT EXISTS (SELECT 1 FROM contexts_readable_by('{d}') c WHERE c.context_id='{eng_ctx}')"),
        ),
        (
            "resources_visible_to",
            format!("SELECT EXISTS (SELECT 1 FROM resources_visible_to('{d}') v WHERE v.resource_id='{team_read}')"),
        ),
        (
            "cogmap_visible_maps",
            format!("SELECT EXISTS (SELECT 1 FROM cogmap_visible_maps('{d}') t(id) WHERE t.id='{eng_map}')"),
        ),
        (
            "cogmap_readable_by_profile",
            format!("SELECT cogmap_readable_by_profile('{d}','{eng_map}')"),
        ),
        (
            "edges_visible_to",
            format!("SELECT EXISTS (SELECT 1 FROM edges_visible_to('{d}') e WHERE e.edge_id='{cogmap_edge}')"),
        ),
        (
            "cogmap_list_rows",
            // COALESCE, because under the tier-3 global flatten this function returns NO ROW at
            // all — `cogmap_visible_maps` stops admitting the map — rather than a different
            // `owner_ref`. A bare scalar select would raise RowNotFound instead of reporting a
            // dark door, and an errored probe is not a measured one.
            format!(
                "SELECT COALESCE((SELECT owner_ref = '+engineering' FROM cogmap_list_rows('{d}') \
                  WHERE cogmap_id='{eng_map}'), false)"
            ),
        ),
        (
            "graph_home_cogmaps",
            // COALESCE, because under the tier-3 global flatten this function returns NO ROW at
            // all — `cogmap_visible_maps` stops admitting the map — rather than a different
            // `owner_ref`. A bare scalar select would raise RowNotFound instead of reporting a
            // dark door, and an errored probe is not a measured one.
            format!(
                "SELECT COALESCE((SELECT owner_ref = '+engineering' FROM graph_home_cogmaps('{d}') \
                  WHERE cogmap_id='{eng_map}'), false)"
            ),
        ),
        (
            "profile_explicit_grant",
            format!("SELECT profile_explicit_grant('{d}','read','kb_contexts','{sec_ctx}')"),
        ),
        (
            "can_modify_resource",
            format!("SELECT can_modify_resource('{d}','{team_write}')"),
        ),
    ])
}

// =================================================================================================
// Tier 3a — the per-function mutation gate.
//
// This exists because tier 2 was WRONG for four of its nine members, and nothing detected that.
// A green characterization was accepted as evidence it constrained the function it named; for
// `resources_visible_to`, `edges_visible_to`, `cogmap_list_rows` and `graph_home_cogmaps` it did
// not — each admitted through a DIFFERENT function's already-covered CTE, so the line the fold
// introduced could be deleted (or widened to every team) with the suite still green.
//
// So the standard is no longer "the test passes". It is: **empty this function's own fold site and
// the assertion that names it must die.** That is checked here, mechanically, for all nine.
// =================================================================================================

/// Every routed function's fold site is load-bearing for the property its own test asserts.
///
/// **How the mutation is derived, and why not by hand.** Writing nine sabotaged bodies into this
/// file would mean nine more copies to drift — the exact disease this whole suite treats. Instead
/// the sabotage is derived from the live definition: take `pg_get_functiondef`, redirect that
/// function's `profile_reachable_teams(` call to a zero-row stand-in, and execute the result. Only
/// the named function changes; the authoritative relation and all eight siblings stay intact, which
/// is what isolates the claim to one fold site.
///
/// Each probe below is the SQL form of the assertion its characterization test makes. If a probe
/// stops matching its test, this gate silently starts measuring the wrong thing — so keep them
/// together, and prefer changing both or neither.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn every_fold_site_is_load_bearing(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let probes = fold_probes(&pool, &o).await?;

    // The zero-row stand-in the sabotage redirects to.
    sqlx::query(
        "CREATE OR REPLACE FUNCTION empty_reachable_teams(p_profile uuid) \
           RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS \
         $$ SELECT NULL::uuid AS team_id WHERE false $$",
    )
    .execute(&pool)
    .await?;

    let mut inert = Vec::new();
    for (func, probe) in &probes {
        let baseline: Option<bool> = sqlx::query_scalar(probe).fetch_one(&pool).await?;
        assert_eq!(
            baseline,
            Some(true),
            "precondition failed: {func}'s probe must hold before sabotage, or the mutation below \
             proves nothing. Probe: {probe}"
        );

        let original: String = sqlx::query_scalar(
            "SELECT pg_get_functiondef(p.oid) FROM pg_proc p \
               JOIN pg_namespace n ON n.oid = p.pronamespace \
              WHERE n.nspname='public' AND p.prokind='f' AND p.proname=$1",
        )
        .bind(func)
        .fetch_one(&pool)
        .await?;

        let sabotaged = original.replace("profile_reachable_teams(", "empty_reachable_teams(");
        assert_ne!(
            sabotaged, original,
            "{func} does not call profile_reachable_teams — it is either not routed, or this gate's \
             function list is stale"
        );

        sqlx::query(&sabotaged).execute(&pool).await?;
        let after: Option<bool> = sqlx::query_scalar(probe).fetch_one(&pool).await?;
        sqlx::query(&original).execute(&pool).await?; // restore before the next iteration

        if after == Some(true) {
            inert.push(*func);
        }
    }

    assert!(
        inert.is_empty(),
        "for {} of 9 routed function(s), emptying that function's OWN fold site changed nothing \
         its characterization asserts: {inert:?}\n\
         \n\
         That test is not a witness for that function — it is passing through some other \
         function's already-covered CTE. Build a fixture that enters via an arm this function's \
         own `reachable_teams`/`member_teams` CTE actually feeds, and assert on it.\n\
         \n\
         This is not hypothetical: four of these nine were inert when first written.",
        inert.len(),
    );

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
/// memberships only, dropping the ancestor walk — and assert **every one of the nine doors** goes
/// dark for dana, while a door that rides her DIRECT membership stays lit. The break is the exact
/// inverse of the invariant the tier asserts (reach inherits UP), so a green tier-2 against a
/// flattened relation would mean the tier is measuring something else.
///
/// **Corrected 2026-08-04 after adversarial review.** This probe used to assert exactly two doors —
/// `contexts_readable_by` and `cogmap_readable_by_profile` — while its own doc claimed "every door
/// goes dark". Seven doors had no bite evidence at all, including *both mutation gates*, which are
/// the functions whose risk class is the stated reason migration `20260804000020` is a separate
/// file. A docstring overclaiming coverage is worse than the gap, because it stops anyone checking.
/// It now runs the shared [`fold_probes`] list, so "every door" is enforced rather than asserted.
///
/// Distinct from [`every_fold_site_is_load_bearing`], which empties ONE function's call site at a
/// time: this one changes what the shared relation *returns* and asks whether that propagates
/// everywhere. Both are needed — a function could join the relation faithfully (passing this) while
/// its own probe rides some other function's CTE (failing that).
///
/// Runs inside the test's own database, so the redefinition dies with it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_equivalence_tier_actually_bites(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let probes = fold_probes(&pool, &o).await?;

    // A door that rides dana's DIRECT membership, not the ancestor walk. It must stay lit, which is
    // what makes this a probe rather than a demonstration that breaking things breaks things.
    let own = team_context(&pool, o.squad_two, "squad-two-ctx").await?;
    let direct_arm = format!(
        "SELECT EXISTS (SELECT 1 FROM contexts_readable_by('{}') c WHERE c.context_id='{own}')",
        o.dana
    );

    for (func, probe) in &probes {
        let baseline: Option<bool> = sqlx::query_scalar(probe).fetch_one(&pool).await?;
        assert_eq!(
            baseline,
            Some(true),
            "precondition failed: {func}'s probe must hold before the relation is flattened"
        );
    }
    let direct_before: Option<bool> = sqlx::query_scalar(&direct_arm).fetch_one(&pool).await?;
    assert_eq!(
        direct_before,
        Some(true),
        "precondition: direct arm reaches"
    );

    // Flatten the one definition: direct memberships only, no ancestor walk.
    sqlx::query(
        "CREATE OR REPLACE FUNCTION profile_reachable_teams(p_profile uuid) \
           RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS \
         $$ SELECT e.team_id FROM profile_effective_teams(p_profile) e $$",
    )
    .execute(&pool)
    .await?;

    let mut still_lit = Vec::new();
    for (func, probe) in &probes {
        let after: Option<bool> = sqlx::query_scalar(probe).fetch_one(&pool).await?;
        if after == Some(true) {
            still_lit.push(*func);
        }
    }

    assert!(
        still_lit.is_empty(),
        "BITE PROBE FAILED for {} of 9 door(s): {still_lit:?} still reached ancestor-attached \
         material with a FLATTENED reachable-teams relation.\n\
         \n\
         Either those functions are not routed through {AUTHORITATIVE}(), or they carry their own \
         copy of the walk — in which case the clause-1 witness is the test to look at.",
        still_lit.len(),
    );

    // ...while the DIRECT arm still works: the flattening removed exactly one axis.
    let direct_after: Option<bool> = sqlx::query_scalar(&direct_arm).fetch_one(&pool).await?;
    assert_eq!(
        direct_after,
        Some(true),
        "the flattened relation should still admit dana's DIRECT team — if this fails the probe \
         broke more than the ancestor walk and proves less than it claims"
    );

    Ok(())
}
