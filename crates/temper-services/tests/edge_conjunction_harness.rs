#![cfg(feature = "test-db")]

//! The edge-conjunction guard harness (temper-artifacts `specs/2026-11-06-public-within-design.md`
//! P12, N12; plan Phase 0.4).
//!
//! # The invariant
//!
//! > Every function that returns, traverses, counts or scores edges for a caller admits an edge
//! > only if it is live, its home anchor is readable by the caller, and both endpoints are readable
//! > by the caller.
//!
//! Linking from what you author to anything you can read is safe exactly as far as this holds. A
//! surface that shows an edge on fewer conditions turns linking into a leak (PW-6 was one:
//! `follow_from`'s adjacency checked the endpoints and never the home).
//!
//! # How it is guarded
//!
//! [`every_public_function_that_reads_kb_edges_is_classified`] enumerates every `public` function
//! whose `pg_get_functiondef` names `kb_edges` (the vocabulary-scan pattern of
//! `visibility_invariant_harness.rs`), and fails on any function missing from [`CLASSIFIED`]. Each
//! one is in exactly one class:
//!
//! - [`Class::Definition`]: `edges_visible_to` itself, the one set-based definition.
//! - [`Class::Delegates`]: its body calls `edges_visible_to`. Checked mechanically against `prosrc`.
//! - [`Class::Probed`]: caller-facing and spelling the conjunction itself. It names the probe below
//!   that builds an edge with an unreadable home and one with an unreadable endpoint, beside a
//!   readable control, and asserts both are absent and the control present.
//! - [`Class::NotCallerFacing`]: write gates, projectors, producers (materialize, kernels),
//!   erasure acts and backfills, each with its reason.
//! - [`Class::KnownViolation`]: caller-facing, breaks the conjunction today, and is not a small
//!   enough gate to fix in Phase 0. Each has a probe that fails on purpose, marked `#[ignore]` with
//!   the reason, so the violation is written down where the invariant is, rather than forgotten.
//!
//! The Rust side is held the same way by [`every_rust_side_kb_edges_read_is_classified`]: every
//! line under `crates/*/src` that names `kb_edges` after `FROM`/`JOIN`/`UPDATE`/`INTO` is keyed by
//! its file and enclosing `fn`, and must be in [`RUST_CLASSIFIED`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sqlx::{PgPool, Row};
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, EdgeHome, SeedAction};
use temper_substrate::ids::{CogmapId, ContextId, EdgeId, EntityId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::payloads::EdgePolarity;
use temper_substrate::writes;
use uuid::Uuid;

/// What a function that reads `kb_edges` is, with respect to the conjunction.
#[derive(Debug, Clone, Copy)]
enum Class {
    /// `edges_visible_to`: the definition everything else must agree with.
    Definition,
    /// Calls `edges_visible_to` (verified against the body).
    Delegates,
    /// Caller-facing, spells the conjunction itself; names the probe test that proves it.
    Probed(&'static str),
    /// Not a caller's read: why.
    NotCallerFacing(&'static str),
    /// Caller-facing and breaks the conjunction today: the failing-on-purpose probe, and why it is
    /// not fixed here.
    KnownViolation(&'static str),
}

/// This file's source, so a classification can be checked against the probe it names.
const THIS_FILE: &str = include_str!("edge_conjunction_harness.rs");

/// Every `public` function whose definition names `kb_edges`, classified. One row per name: every
/// overload of a name shares its class.
const CLASSIFIED: &[(&str, Class)] = &[
    ("edges_visible_to", Class::Definition),
    // ── Delegates ─────────────────────────────────────────────────────────────────────────────
    ("__temper_ungated_follow_from_homed", Class::Delegates),
    // Its 'profile' arm calls edges_visible_to; its 'cogmap' arm (the map reading itself) is
    // pinned by `cogmap_regulation_admits_only_visible_edges`.
    ("cogmap_regulation", Class::Delegates),
    ("graph_atlas_nodes", Class::Delegates),
    ("graph_atlas_nodes_cogmap", Class::Delegates),
    ("graph_atlas_nodes_visible", Class::Delegates),
    ("graph_cogmap_orphan_nodes", Class::Delegates),
    ("graph_context_containers", Class::Delegates),
    ("graph_context_residual_counts", Class::Delegates),
    ("graph_context_residual_members", Class::Delegates),
    ("graph_induced_edges", Class::Delegates),
    ("graph_orphan_salient_nodes", Class::Delegates),
    ("graph_territory_bridges", Class::Delegates),
    ("graph_traverse", Class::Delegates),
    ("graph_traverse_cogmap_scoped", Class::Delegates),
    ("graph_visible_degree_bounds", Class::Delegates),
    ("graph_visible_degree_ranking", Class::Delegates),
    // ── Probed ────────────────────────────────────────────────────────────────────────────────
    (
        "element_trail_edge",
        Class::Probed("the_edge_trail_admits_only_a_visible_edge"),
    ),
    (
        "graph_traverse_scoped",
        Class::Probed("the_scoped_traversal_admits_only_visible_edges"),
    ),
    (
        "resource_lineage",
        Class::Probed("lineage_walks_only_visible_edges"),
    ),
    (
        "context_changes_after",
        Class::Probed("the_change_feed_names_only_visible_edges"),
    ),
    (
        "context_cursor",
        Class::Probed("the_change_feed_names_only_visible_edges"),
    ),
    (
        "anchor_staleness",
        Class::Probed("staleness_is_touched_only_by_visible_edges"),
    ),
    // ── Not caller-facing ─────────────────────────────────────────────────────────────────────
    (
        "_edge_phantom_reassertions",
        Class::NotCallerFacing(
            "helper of element_trail_edge, which applies the conjunction to the edge before any \
             row it contributes is returned; no Rust caller",
        ),
    ),
    (
        "_project_relationship_asserted",
        Class::NotCallerFacing("projector: writes the edge row from its event"),
    ),
    (
        "_project_relationship_folded",
        Class::NotCallerFacing("projector: writes the fold from its event"),
    ),
    (
        "_project_relationship_retyped",
        Class::NotCallerFacing("projector: writes the retype from its event"),
    ),
    (
        "_project_relationship_reweighted",
        Class::NotCallerFacing("projector: writes the reweight from its event"),
    ),
    (
        "relationship_assert",
        Class::NotCallerFacing(
            "write path: authorized in Rust (check_home_authorable, endpoint readability) before \
             it is called",
        ),
    ),
    (
        "relationship_fold",
        Class::NotCallerFacing("write path: authorized by check_edge_mutable before it is called"),
    ),
    (
        "relationship_retype",
        Class::NotCallerFacing("write path: authorized by check_edge_mutable before it is called"),
    ),
    (
        "relationship_reweight",
        Class::NotCallerFacing("write path: authorized by check_edge_mutable before it is called"),
    ),
    (
        "_property_owner_anchor",
        Class::NotCallerFacing(
            "write guard: resolves the anchor a property's owner (an edge among them) is homed in, \
             for facet_set/property_set; returns no edge to anyone",
        ),
    ),
    (
        "_resource_write_guard_owner",
        Class::NotCallerFacing(
            "write guard inside the projectors: resolves the owning resource of a write",
        ),
    ),
    (
        "_resource_erasure_apply_redaction",
        Class::NotCallerFacing("erasure act: redacts what the act's scope names"),
    ),
    (
        "_resource_erasure_trail_scope",
        Class::NotCallerFacing("erasure act: computes the trail the act redacts"),
    ),
    (
        "resource_erasure_execute",
        Class::NotCallerFacing("erasure act: authorized in resource_erasure_service"),
    ),
    (
        "resource_erasure_survey_plan",
        Class::NotCallerFacing(
            "erasure act's own scope computation, shared with the survey door, which authorizes \
             the actor over the resource first",
        ),
    ),
    (
        "backfill_goal_parent_of_to_advances",
        Class::NotCallerFacing("one-shot data backfill, run by a migration"),
    ),
    (
        "cogmap_region_centrality",
        Class::NotCallerFacing(
            "producer: region kernel scoring at materialize (temper-substrate write.rs). Derived \
             structure from edges homed in the region's own anchor (spec P12, Residuals)",
        ),
    ),
    (
        "cogmap_region_internal_tension",
        Class::NotCallerFacing(
            "producer: region kernel scoring at materialize (temper-substrate write.rs). Derived \
             structure (spec P12, Residuals)",
        ),
    ),
    (
        "context_goal_liveness",
        Class::NotCallerFacing(
            "producer: feeds anchor_telos_embedding, the telos materialize; derived structure \
             (spec P12, Residuals)",
        ),
    ),
    // ── Known violations ──────────────────────────────────────────────────────────────────────
    (
        "resource_contradiction_balance",
        Class::KnownViolation(
            "known_violation_standing_scores_unreadable_edges: sums supports/contradicts edges \
             with no gate and no principal. Reached by resource_standing_shape, which gates only \
             the finding, behind GET /api/resources/{id}/evidence",
        ),
    ),
];

// =================================================================================================
// The scan.
// =================================================================================================

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_public_function_that_reads_kb_edges_is_classified(pool: PgPool) -> sqlx::Result<()> {
    // name -> does any overload's BODY (prosrc, so the signature line cannot match) call
    // edges_visible_to?
    let rows = sqlx::query(
        "SELECT p.proname AS name, \
                bool_or(p.prosrc ~ '\\medges_visible_to\\s*\\(') AS delegates \
           FROM pg_proc p \
           JOIN pg_namespace n ON n.oid = p.pronamespace \
          CROSS JOIN LATERAL (SELECT pg_get_functiondef(p.oid) AS def) d \
          WHERE n.nspname = 'public' AND p.prokind = 'f' \
            AND d.def ~ '\\mkb_edges\\M' \
          GROUP BY p.proname \
          ORDER BY 1",
    )
    .fetch_all(&pool)
    .await?;
    let scanned: BTreeMap<String, bool> = rows
        .iter()
        .map(|r| (r.get::<String, _>("name"), r.get::<bool, _>("delegates")))
        .collect();

    assert!(
        scanned.contains_key("edges_visible_to") && scanned.len() > 10,
        "the scan returned {} function(s) — the vocabulary no longer matches, so this gate is \
         vacuous",
        scanned.len()
    );

    let classified: BTreeMap<&str, Class> = CLASSIFIED.iter().copied().collect();
    assert_eq!(
        classified.len(),
        CLASSIFIED.len(),
        "a name is classified twice"
    );

    let unclassified: Vec<&String> = scanned
        .keys()
        .filter(|n| !classified.contains_key(n.as_str()))
        .collect();
    assert!(
        unclassified.is_empty(),
        "{} function(s) read kb_edges and are not classified: {unclassified:?}\n\n\
         Every function that returns, traverses, counts or scores edges for a caller must admit an \
         edge only if it is live, its home is readable and both endpoints are readable (spec P12). \
         Call edges_visible_to (DELEGATES), or add a probe here (PROBED), or say why it is not a \
         caller's read (NOT_CALLER_FACING).",
        unclassified.len()
    );

    let stale: Vec<&&str> = classified
        .keys()
        .filter(|n| !scanned.contains_key(**n))
        .collect();
    assert!(
        stale.is_empty(),
        "classified but no longer reading kb_edges (or gone): {stale:?} — remove them"
    );

    for (name, class) in CLASSIFIED {
        let delegates = scanned[*name];
        match class {
            Class::Delegates => assert!(
                delegates,
                "{name} is classified DELEGATES but its body does not call edges_visible_to"
            ),
            Class::Probed(_) | Class::NotCallerFacing(_) | Class::KnownViolation(_) => assert!(
                !delegates,
                "{name} now calls edges_visible_to: reclassify it as DELEGATES"
            ),
            Class::Definition => {}
        }
        match class {
            // A probe is a test in this file, by name: a renamed or deleted probe fails here.
            Class::Probed(probe) => assert!(
                THIS_FILE.contains(&format!("async fn {probe}(")),
                "{name} names the probe {probe}, which is not a test in this file"
            ),
            // A known violation's reason starts with its failing-on-purpose probe's name.
            Class::KnownViolation(reason) => {
                let probe = reason.split(':').next().unwrap_or_default();
                assert!(
                    THIS_FILE.contains(&format!("async fn {probe}(")),
                    "{name}'s known violation names the probe `{probe}`, which is not a test here"
                );
            }
            Class::NotCallerFacing(reason) => {
                assert!(
                    !reason.trim().is_empty(),
                    "{name}: say why it is not a caller's read"
                );
            }
            Class::Delegates | Class::Definition => {}
        }
    }
    Ok(())
}

// =================================================================================================
// The probe world.
// =================================================================================================

/// ```text
///   team A  (reader is a member)  ─ owns ─ context C:  resources a, b, c
///   team B  (owner only)          ─ owns ─ context U:  resource  h
///
///   e_ok   a ─▸ b   homed in C     the control: everything readable
///   e_home a ─▸ c   homed in U     both endpoints readable, home NOT
///   e_end  a ─▸ h   homed in C     home readable, target NOT
/// ```
///
/// Created in that order, each in its own transaction, so `e_end` is the newest touch on C and
/// `e_ok` the newest readable one. Every edge is `leads_to`/`derived_from`, so the lineage walk
/// sees all three.
struct World {
    reader: Uuid,
    team_a: Uuid,
    ctx_c: Uuid,
    ctx_u: Uuid,
    a: Uuid,
    b: Uuid,
    c: Uuid,
    h: Uuid,
    emitter: Uuid,
    e_ok: Uuid,
    e_home: Uuid,
    e_end: Uuid,
}

async fn profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) \
         VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("profile")
}

async fn team(pool: &PgPool, slug: &str, member: Uuid) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("team");
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(id)
    .bind(member)
    .execute(pool)
    .await
    .expect("membership");
    id
}

async fn context(pool: &PgPool, team: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(team)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("context")
}

async fn resource(pool: &PgPool, ctx: Uuid, owner: Uuid, title: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (id, title, origin_uri) VALUES (uuid_generate_v7(), $1, '') \
         RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .expect("resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (id, resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES (uuid_generate_v7(), $1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home");
    id
}

async fn edge(pool: &PgPool, src: Uuid, tgt: Uuid, home: Uuid, emitter: Uuid) -> Uuid {
    let id: EdgeId = writes::assert_relationship(
        pool,
        writes::AssertParams {
            src: ResourceId::from(src),
            tgt: ResourceId::from(tgt),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("derived_from"),
            weight: 1.0,
            home: ContextId::from(home),
            emitter: EntityId::from(emitter),
        },
    )
    .await
    .expect("assert edge");
    id.uuid()
}

async fn world(pool: &PgPool) -> World {
    let reader = profile(pool, "ec-reader").await;
    let owner = profile(pool, "ec-owner").await;
    let emitter: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name, metadata) \
         VALUES ($1, 'ec-owner@web', '{}'::jsonb) RETURNING id",
    )
    .bind(owner)
    .fetch_one(pool)
    .await
    .expect("emitter");
    let team_a = team(pool, "ec-a", reader).await;
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')")
        .bind(team_a)
        .bind(owner)
        .execute(pool)
        .await
        .expect("owner on A");
    let team_b = team(pool, "ec-b", owner).await;
    let ctx_c = context(pool, team_a, "ec-c").await;
    let ctx_u = context(pool, team_b, "ec-u").await;
    let a = resource(pool, ctx_c, owner, "ec-a").await;
    let b = resource(pool, ctx_c, owner, "ec-b").await;
    let c = resource(pool, ctx_c, owner, "ec-c").await;
    let h = resource(pool, ctx_u, owner, "ec-h").await;
    let e_ok = edge(pool, a, b, ctx_c, emitter).await;
    let e_home = edge(pool, a, c, ctx_u, emitter).await;
    let e_end = edge(pool, a, h, ctx_c, emitter).await;

    let w = World {
        reader,
        team_a,
        ctx_c,
        ctx_u,
        a,
        b,
        c,
        h,
        emitter,
        e_ok,
        e_home,
        e_end,
    };
    // The world is what it claims: the definition admits the control and neither probe edge.
    let visible: BTreeSet<Uuid> = sqlx::query_scalar("SELECT edge_id FROM edges_visible_to($1)")
        .bind(w.reader)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert!(visible.contains(&w.e_ok), "fixture: the control is visible");
    assert!(
        !visible.contains(&w.e_home),
        "fixture: e_home's home is unreadable"
    );
    assert!(
        !visible.contains(&w.e_end),
        "fixture: e_end's target is unreadable"
    );
    let resources: BTreeSet<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM resources_visible_to($1)")
            .bind(w.reader)
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .collect();
    assert!(
        [w.a, w.b, w.c].iter().all(|r| resources.contains(r)) && !resources.contains(&w.h),
        "fixture: a, b, c are visible and h is not"
    );
    w
}

/// The three-way verdict every probe asserts: the control is seen, neither probe edge is.
fn assert_conjunction(what: &str, w: &World, seen: &BTreeSet<Uuid>) {
    assert!(
        seen.contains(&w.e_ok),
        "{what}: the readable control edge is missing, so the absences below would be vacuous"
    );
    assert!(
        !seen.contains(&w.e_home),
        "{what}: admitted an edge whose HOME the caller cannot read"
    );
    assert!(
        !seen.contains(&w.e_end),
        "{what}: admitted an edge whose ENDPOINT the caller cannot read"
    );
}

// =================================================================================================
// PW-6: follow_from and search_graph_expand (plan 0.1), and graph_traverse.
// =================================================================================================

async fn expand(pool: &PgPool, w: &World) -> Vec<(Uuid, f32)> {
    sqlx::query(
        "SELECT resource_id, graph_score FROM search_graph_expand($1, $2, 2, NULL, 0.5) \
         ORDER BY graph_score DESC, resource_id",
    )
    .bind(w.reader)
    .bind(vec![w.a])
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get("resource_id"), r.get("graph_score")))
    .collect()
}

/// An edge between two visible resources, homed in a context the caller cannot read, is absent
/// from `follow_from` (its node and its `via`) and does not move `search_graph_expand`'s ranking.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn follow_from_ignores_an_edge_whose_home_is_unreadable(pool: PgPool) {
    let w = world(&pool).await;

    let rows = sqlx::query(
        "SELECT resource_id, via::text AS via FROM query_follow_from(\
         $1, $2, 2, 0.5, NULL, NULL, NULL, 50, NULL, NULL)",
    )
    .bind(w.reader)
    .bind(vec![w.a])
    .fetch_all(&pool)
    .await
    .unwrap();
    let nodes: BTreeSet<Uuid> = rows.iter().map(|r| r.get("resource_id")).collect();
    assert_eq!(
        nodes,
        BTreeSet::from([w.b]),
        "only the control's target is reached: c sits behind an unreadable home, h is unreadable"
    );
    let via: String = rows.iter().map(|r| r.get::<String, _>("via")).collect();
    assert!(!via.contains(&w.c.to_string()) && !via.contains(&w.h.to_string()));

    // The compiler's emitter: the `_homed` core with the principal as its anchor reader.
    let core: BTreeSet<Uuid> = sqlx::query_scalar(
        "SELECT resource_id FROM __temper_ungated_follow_from_homed(\
         ARRAY(SELECT resource_id FROM resources_visible_to($1)), $2, 2, 0.5, \
         NULL, NULL, NULL, 50, NULL, NULL, $1)",
    )
    .bind(w.reader)
    .bind(vec![w.a])
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .collect();
    assert_eq!(core, nodes, "the core and the gated wrapper agree");

    // Ranking: the answer with e_home live equals the answer with e_home folded away.
    let with_edge = expand(&pool, &w).await;
    sqlx::query("UPDATE kb_edges SET is_folded = true WHERE id = $1")
        .bind(w.e_home)
        .execute(&pool)
        .await
        .unwrap();
    let without_edge = expand(&pool, &w).await;
    assert_eq!(
        with_edge, without_edge,
        "an edge the caller cannot see must not move search_graph_expand's ranking"
    );
    assert_eq!(
        with_edge.len(),
        1,
        "and the ranking is not empty: {with_edge:?}"
    );
}

/// `graph_traverse` (no Rust caller) is gated the same way in place.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn graph_traverse_admits_only_visible_edges(pool: PgPool) {
    let w = world(&pool).await;
    let reached: BTreeSet<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM graph_traverse($1, $2, 2)")
            .bind(w.reader)
            .bind(vec![w.a])
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .collect();
    assert_eq!(reached, BTreeSet::from([w.b]));
}

// =================================================================================================
// PROBED.
// =================================================================================================

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_edge_trail_admits_only_a_visible_edge(pool: PgPool) {
    let w = world(&pool).await;
    let mut seen = BTreeSet::new();
    for e in [w.e_ok, w.e_home, w.e_end] {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM element_trail_edge($1, $2)")
            .bind(w.reader)
            .bind(e)
            .fetch_one(&pool)
            .await
            .unwrap();
        if n > 0 {
            seen.insert(e);
        }
    }
    assert_conjunction("element_trail_edge", &w, &seen);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_scoped_traversal_admits_only_visible_edges(pool: PgPool) {
    let w = world(&pool).await;
    let seen: BTreeSet<Uuid> =
        sqlx::query_scalar("SELECT id FROM graph_traverse_scoped($1, $2, $3, 2, NULL)")
            .bind(w.reader)
            .bind(w.team_a)
            .bind(vec![w.a])
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .collect();
    assert_conjunction("graph_traverse_scoped", &w, &seen);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn lineage_walks_only_visible_edges(pool: PgPool) {
    let w = world(&pool).await;
    let seen: BTreeSet<Uuid> =
        sqlx::query_scalar("SELECT edge_id FROM resource_lineage($1, $2, 'ancestors', 16)")
            .bind(w.reader)
            .bind(w.a)
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .collect();
    assert_conjunction("resource_lineage", &w, &seen);
}

/// Wait until every transaction id assigned so far is below the cluster-wide horizon, which the
/// change feed serves up to. Other test databases in the same cluster hold it while their
/// transactions run (the `context_change_doors_test.rs` helper).
async fn settle(pool: &PgPool) {
    let target: String = sqlx::query_scalar("SELECT pg_snapshot_xmax(pg_current_snapshot())::text")
        .fetch_one(pool)
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let settled: bool =
            sqlx::query_scalar("SELECT pg_snapshot_xmin(pg_current_snapshot()) >= $1::text::xid8")
                .bind(&target)
                .fetch_one(pool)
                .await
                .unwrap();
        if settled {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the horizon did not pass xid {target} within 120 s"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// The change feed of C names e_ok and never e_end; the feed of U (whose home the reader cannot
/// read) answers nothing that names e_home. The cursor of C never points at e_end's event.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_change_feed_names_only_visible_edges(pool: PgPool) {
    let w = world(&pool).await;
    settle(&pool).await;
    let feed = |ctx: Uuid| {
        let pool = pool.clone();
        let reader = w.reader;
        async move {
            sqlx::query_scalar::<_, Option<serde_json::Value>>(
                "SELECT context_changes_after($1, $2, '0'::xid8, 0, 1000)",
            )
            .bind(reader)
            .bind(ctx)
            .fetch_one(&pool)
            .await
            .unwrap()
            .map(|v| v.to_string())
            .unwrap_or_default()
        }
    };
    let c_feed = feed(w.ctx_c).await;
    let u_feed = feed(w.ctx_u).await;
    let mut seen = BTreeSet::new();
    for e in [w.e_ok, w.e_home, w.e_end] {
        let id = e.to_string();
        if c_feed.contains(&id) || u_feed.contains(&id) {
            seen.insert(e);
        }
    }
    assert_conjunction("context_changes_after", &w, &seen);

    let event_of = |e: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>("SELECT asserted_by_event_id FROM kb_edges WHERE id = $1")
                .bind(e)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let cursor: Option<Uuid> = sqlx::query_scalar("SELECT context_cursor($1, $2)")
        .bind(w.reader)
        .bind(w.ctx_c)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(cursor.is_some(), "context_cursor: C has readable events");
    assert_ne!(
        cursor,
        Some(event_of(w.e_end).await),
        "context_cursor: points at the event of an edge whose endpoint the reader cannot read"
    );
    let u_cursor: Option<Uuid> = sqlx::query_scalar("SELECT context_cursor($1, $2)")
        .bind(w.reader)
        .bind(w.ctx_u)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(
        u_cursor,
        Some(event_of(w.e_home).await),
        "context_cursor: points at the event of an edge whose home the reader cannot read"
    );
}

/// C's latest touch is e_ok's event, not the newer e_end's; U answers nothing to the reader.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn staleness_is_touched_only_by_visible_edges(pool: PgPool) {
    let w = world(&pool).await;
    let at = |e: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
                "SELECT ev.occurred_at FROM kb_edges e JOIN kb_events ev ON ev.id = e.last_event_id \
                  WHERE e.id = $1",
            )
            .bind(e)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let touch: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT latest_touch FROM anchor_staleness('kb_contexts', $1, 'profile', $2)",
    )
    .bind(w.ctx_c)
    .bind(w.reader)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    let (ok_at, end_at) = (at(w.e_ok).await, at(w.e_end).await);
    assert!(end_at > ok_at, "fixture: e_end is the newer touch");
    assert_eq!(
        touch,
        Some(ok_at),
        "anchor_staleness: C's latest touch must be the readable control, not the edge whose \
         endpoint the reader cannot read"
    );
    let u_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM anchor_staleness('kb_contexts', $1, 'profile', $2)",
    )
    .bind(w.ctx_u)
    .bind(w.reader)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        u_rows, 0,
        "anchor_staleness: an unreadable anchor answers nothing"
    );
}

// =================================================================================================
// KNOWN VIOLATIONS: probes that fail on purpose until the function is fixed.
// =================================================================================================

/// `cogmap_regulation` returns the telos's `express` edges only under the conjunction. For a
/// profile: an edge homed where the reader cannot read never surfaces. For the map's own principal:
/// only an edge homed in the map itself surfaces, never one homed in a context.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn cogmap_regulation_admits_only_visible_edges(pool: PgPool) {
    let w = world(&pool).await;
    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (id, name, telos_resource_id) VALUES (uuid_generate_v7(), \
         'ec-map', $1) RETURNING id",
    )
    .bind(w.a)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(w.team_a)
        .execute(&pool)
        .await
        .unwrap();
    for (tgt, home) in [(w.b, w.ctx_c), (w.c, w.ctx_u)] {
        writes::assert_relationship(
            &pool,
            writes::AssertParams {
                src: ResourceId::from(w.a),
                tgt: ResourceId::from(tgt),
                kind: EdgeKind::Express,
                polarity: EdgePolarity::Forward,
                label: Some("regulates"),
                weight: 1.0,
                home: ContextId::from(home),
                emitter: EntityId::from(w.emitter),
            },
        )
        .await
        .unwrap();
    }
    let targets: BTreeSet<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM cogmap_regulation($1, 'profile', $2)")
            .bind(cogmap)
            .bind(w.reader)
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .collect();
    assert!(
        targets.contains(&w.b),
        "control: the readable edge surfaces"
    );
    assert!(
        !targets.contains(&w.c),
        "an express edge homed in a context the reader cannot read surfaced through \
         cogmap_regulation"
    );

    // The map reading itself. Move the telos and `b` into the map's own interior, so both
    // endpoints are ones the map can access (a team-owned context is not in its reach). Only an
    // edge homed in the map counts: the context-homed a→b edge above still never surfaces to it.
    sqlx::query(
        "UPDATE kb_resource_homes SET anchor_table = 'kb_cogmaps', anchor_id = $1 \
          WHERE resource_id = ANY($2)",
    )
    .bind(cogmap)
    .bind(vec![w.a, w.b])
    .execute(&pool)
    .await
    .unwrap();
    let as_map = |pool: PgPool| async move {
        sqlx::query_scalar::<_, Uuid>("SELECT resource_id FROM cogmap_regulation($1, 'cogmap', $1)")
            .bind(cogmap)
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .collect::<BTreeSet<Uuid>>()
    };
    assert!(
        as_map(pool.clone()).await.is_empty(),
        "the map principal sees no edge homed in a context, though it can access both endpoints"
    );
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(ResourceId::from(w.a)),
            tgt: AnchorRef::resource(ResourceId::from(w.b)),
            kind: EdgeKind::Express,
            polarity: EdgePolarity::Forward,
            label: Some("regulates"),
            weight: 1.0,
            home: EdgeHome::Cogmap(CogmapId::from(cogmap)),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        as_map(pool.clone()).await,
        BTreeSet::from([w.b]),
        "an edge homed in the map itself surfaces to the map principal"
    );
    let other: i64 = sqlx::query_scalar("SELECT count(*) FROM cogmap_regulation($1, 'cogmap', $2)")
        .bind(cogmap)
        .bind(w.team_a)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        other, 0,
        "a cogmap principal other than the map itself admits nothing"
    );
}

/// `resource_standing_shape` gates the finding, then scores it with
/// `resource_contradiction_balance`, which sums every live support/contradict edge on it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
#[ignore = "KNOWN VIOLATION (spec P12): resource_contradiction_balance scores edges with no gate; \
            fails on purpose until it takes a principal. See CLASSIFIED."]
async fn known_violation_standing_scores_unreadable_edges(pool: PgPool) {
    let w = world(&pool).await;
    writes::assert_relationship(
        &pool,
        writes::AssertParams {
            src: ResourceId::from(w.c),
            tgt: ResourceId::from(w.b),
            kind: EdgeKind::Express,
            polarity: EdgePolarity::Forward,
            label: Some("supports"),
            weight: 1.0,
            home: ContextId::from(w.ctx_u),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();
    let balance: Option<f64> = sqlx::query_scalar(
        "SELECT contradiction_balance FROM resource_standing_shape($1, 'profile', $2)",
    )
    .bind(w.b)
    .bind(w.reader)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    assert_eq!(
        balance,
        Some(0.0),
        "the only supporting edge is homed where the reader cannot read; it must not score"
    );
}

// =================================================================================================
// The Rust side.
// =================================================================================================

/// What a Rust-side `kb_edges` read is. Same meaning as [`Class`]; the Rust side has no probe
/// tests of its own, so a site that spells the conjunction inline names where it does.
#[derive(Debug, Clone, Copy)]
enum RustClass {
    /// The query joins `edges_visible_to`, or the function gates the edge on it first.
    Delegates,
    /// The query spells the conjunction (home readable, both endpoints readable) inline.
    InlineConjunction(&'static str),
    NotCallerFacing(&'static str),
}

/// `(file under the repo root, enclosing fn)` → class.
const RUST_CLASSIFIED: &[(&str, &str, RustClass)] = &[
    (
        "crates/temper-services/src/services/edge_service.rs",
        "list_resource_edges",
        RustClass::Delegates,
    ),
    (
        "crates/temper-services/src/services/edge_service.rs",
        "list_resource_connections",
        RustClass::Delegates,
    ),
    (
        "crates/temper-services/src/services/edge_service.rs",
        "list_edge_facets",
        RustClass::Delegates,
    ),
    (
        "crates/temper-substrate/src/readback/mod.rs",
        "blob_relations",
        RustClass::Delegates,
    ),
    (
        "crates/temper-services/src/backend/db_backend.rs",
        "check_edge_mutable",
        RustClass::NotCallerFacing(
            "the edge-mutation write gate; it joins edges_visible_to and answers a hidden edge \
             as missing",
        ),
    ),
    (
        "crates/temper-substrate/src/readback/mod.rs",
        "invocation_show",
        RustClass::InlineConjunction(
            "an event naming an edge is withheld unless anchor_readable_by_profile(home) and both \
             endpoints are readable",
        ),
    ),
    (
        "crates/temper-services/src/backend/db_backend.rs",
        "fold_goal_edges",
        RustClass::NotCallerFacing("write path: the goal-patch act folds the actor's own edges"),
    ),
    (
        "crates/temper-services/src/backend/db_backend.rs",
        "lock_goal_rows",
        RustClass::NotCallerFacing("write path: locks the goal-patch act's rows"),
    ),
    (
        "crates/temper-services/src/backend/db_backend.rs",
        "validate_keyed_edge_write",
        RustClass::NotCallerFacing("write gate, after check_edge_mutable"),
    ),
    (
        "crates/temper-services/src/services/blob_service.rs",
        "delete_blob",
        RustClass::NotCallerFacing(
            "write gate: the strike enumerates every live relation to require delete standing on \
             each peer",
        ),
    ),
    (
        "crates/temper-services/src/services/resource_erasure_service.rs",
        "other_author_edges",
        RustClass::NotCallerFacing("erasure act's scope"),
    ),
    (
        "crates/temper-services/src/services/resource_erasure_service.rs",
        "other_author_edge_properties",
        RustClass::NotCallerFacing("erasure act's scope"),
    ),
    (
        "crates/temper-services/src/services/resource_erasure_service.rs",
        "blob_co_links",
        RustClass::NotCallerFacing("erasure act's scope"),
    ),
    (
        "crates/temper-substrate/src/readback/mod.rs",
        "find_edge",
        RustClass::NotCallerFacing("write path: idempotency lookup before an assert"),
    ),
    (
        "crates/temper-substrate/src/readback/mod.rs",
        "neighbors",
        RustClass::NotCallerFacing("test oracle for the neighbors parity test; no product caller"),
    ),
    (
        "crates/temper-substrate/src/replay.rs",
        "?",
        RustClass::NotCallerFacing("replay's table snapshot, compared byte for byte"),
    ),
    (
        "crates/temper-substrate/src/scenario/runner.rs",
        "apply_mutation",
        RustClass::NotCallerFacing("scenario DSL runner (fixtures)"),
    ),
    (
        "crates/temper-substrate/src/substrate.rs",
        "load",
        RustClass::NotCallerFacing(
            "producer: the region kernel's input, edges homed in the anchor being materialized \
             (derived structure, spec P12 Residuals)",
        ),
    ),
    (
        "crates/temper-services/src/backend/substrate_read.rs",
        "filtered_visible_page",
        RustClass::Delegates,
    ),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `(file, enclosing fn)` for every non-comment line under `crates/*/src` that reads `kb_edges`
/// as a table (`FROM`/`JOIN`/`UPDATE`/`INTO kb_edges`). `?` when no `fn` precedes the line.
fn rust_side_reads() -> BTreeSet<(String, String)> {
    let root = repo_root();
    let table_ref = |line: &str| {
        let upper = line.to_ascii_uppercase();
        ["FROM", "JOIN", "UPDATE", "INTO"].iter().any(|kw| {
            upper.match_indices(kw).any(|(i, _)| {
                let rest = upper[i + kw.len()..].trim_start();
                rest.starts_with("KB_EDGES")
                    && !rest["KB_EDGES".len()..]
                        .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
                    && (i == 0 || !upper.as_bytes()[i - 1].is_ascii_alphanumeric())
            })
        })
    };
    let fn_name = |line: &str| -> Option<String> {
        let t = line.trim_start();
        if t.starts_with("//") {
            return None;
        }
        let i = t.find("fn ")?;
        if i > 0 && !t.as_bytes()[i - 1].is_ascii_whitespace() {
            return None;
        }
        let name: String = t[i + 3..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        (!name.is_empty()).then_some(name)
    };
    let mut files = Vec::new();
    for krate in std::fs::read_dir(root.join("crates")).expect("crates") {
        let src = krate.expect("crate").path().join("src");
        if src.is_dir() {
            rust_sources(&src, &mut files);
        }
    }
    let mut out = BTreeSet::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read source");
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("//") || !table_ref(line) {
                continue;
            }
            let enclosing = lines[..=i]
                .iter()
                .rev()
                .find_map(|l| fn_name(l))
                .unwrap_or_else(|| "?".to_string());
            let rel = file
                .strip_prefix(&root)
                .expect("under the root")
                .to_string_lossy()
                .replace('\\', "/");
            out.insert((rel, enclosing));
        }
    }
    out
}

/// Every Rust-side read of `kb_edges` is classified, and nothing classified is stale.
#[test]
fn every_rust_side_kb_edges_read_is_classified() {
    let found = rust_side_reads();
    assert!(
        found.len() > 10,
        "the Rust-side scan found {} site(s): the pattern no longer matches, so this is vacuous",
        found.len()
    );
    let classified: BTreeSet<(String, String)> = RUST_CLASSIFIED
        .iter()
        .map(|(f, n, _)| (f.to_string(), n.to_string()))
        .collect();
    let unclassified: Vec<_> = found.difference(&classified).collect();
    assert!(
        unclassified.is_empty(),
        "Rust-side kb_edges read(s) not classified in RUST_CLASSIFIED: {unclassified:?}\n\
         Join edges_visible_to, spell the conjunction, or say why the site is not a caller's read."
    );
    let stale: Vec<_> = classified.difference(&found).collect();
    assert!(
        stale.is_empty(),
        "classified Rust-side site(s) no longer read kb_edges: {stale:?}"
    );
    for (file, name, class) in RUST_CLASSIFIED {
        let reason = match class {
            RustClass::Delegates => continue,
            RustClass::InlineConjunction(r) | RustClass::NotCallerFacing(r) => r,
        };
        assert!(!reason.trim().is_empty(), "{file}::{name}: give the reason");
    }
}
