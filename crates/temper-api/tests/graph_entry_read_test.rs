//! Chunk A — the entry read (spec `2026-08-20-grounding-and-navigation-split-design.md` §5.1/§5.4).
//!
//! The invariant every test here exists for:
//!
//! > **Rank by corpus degree; return the induced subgraph over the top-K.**
//!
//! The defect being replaced ranked one set and drew another — it walked from every visible
//! resource while drawing 200 rows ordered by `updated DESC` — so 244 of 250 marks arrived with
//! their edges dropped for having an endpoint off-canvas. What makes that unrepresentable here is
//! that the drawn set and the edge set are decided by the same criterion, at depth 0.
#![cfg(all(test, feature = "test-db"))]

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_services::services::graph_service;

mod common;
use common::{
    seed_contains_edge, seed_context_with_goal_and_tasks, seed_genesis_event, seed_profile,
    seed_resource,
};

/// The L0 kernel cogmap's telos resource — deliberately public and root-team-joined, so it is
/// visible to EVERY profile including a brand-new stranger. Named here so the visibility tests
/// assert against the owner's material rather than against an empty canvas that the kernel's mere
/// existence would falsify.
const KERNEL_TELOS: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000002);

/// The load-bearing property, asserted structurally rather than by counting: no returned edge may
/// name an endpoint that is not in the returned node set. A failure here is the 244-of-250 bug.
fn assert_no_dangling_edges(sub: &temper_core::types::graph_atlas::AtlasEntry) {
    let present: std::collections::HashSet<Uuid> = sub.nodes.iter().map(|n| n.id).collect();
    for e in &sub.edges {
        assert!(
            present.contains(&e.source),
            "edge {} points at source {} which is not drawn — this is the defect chunk A replaces",
            e.id,
            e.source
        );
        assert!(
            present.contains(&e.target),
            "edge {} points at target {} which is not drawn — this is the defect chunk A replaces",
            e.id,
            e.target
        );
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn entry_draws_the_induced_subgraph_and_never_dangles(pool: PgPool) {
    // A star: one goal at degree 5, five tasks at degree 1 each.
    let (profile, _ctx, goal) = seed_context_with_goal_and_tasks(&pool, 5).await;

    let sub = graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(6))
        .await
        .expect("entry read");

    assert_eq!(sub.nodes.len(), 6, "the whole star is drawn at k=6");
    assert_eq!(sub.edges.len(), 5, "and every edge among it");
    assert_no_dangling_edges(&sub);
    assert_eq!(
        sub.nodes.first().map(|n| n.id),
        Some(goal),
        "the hub ranks first — degree 5 against the tasks' 1"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_high_corpus_degree_does_not_imply_an_edge_inside_the_drawing(pool: PgPool) {
    // THE SPEC'S OWN §10.1 IS WRONG, AND THIS IS THE MINIMAL WITNESS. It claims degree-ordered
    // selection means "every node above the cut has at least one edge to another drawn node BY
    // CONSTRUCTION, so the unconnected band does not exist until the connected material runs out."
    //
    // It does not. A hub's neighbours are LEAVES — which is exactly why they are low-degree and
    // miss the cut. Draw the hub alone and it is unconnected despite being the single
    // most-connected resource in the corpus.
    //
    // Measured at scale on production the same day: at K=50, 18 of 50 nodes with corpus degree
    // >= 16 had no induced edge at all. This test is that finding, shrunk to five tasks.
    let (profile, _ctx, goal) = seed_context_with_goal_and_tasks(&pool, 5).await;

    let sub = graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(1))
        .await
        .expect("entry read");

    assert_eq!(sub.nodes.len(), 1);
    assert_eq!(sub.nodes[0].id, goal);
    assert_eq!(
        sub.nodes[0].degree, 5,
        "it carries its CORPUS degree — five edges exist"
    );
    assert!(
        sub.edges.is_empty(),
        "and draws none of them, because their other endpoint is not on the canvas"
    );
    assert_no_dangling_edges(&sub);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn degree_zero_resources_are_excluded_but_never_silently(pool: PgPool) {
    // `[ruled — 2026-08-21, Pete]` The floor, and the declaration that pays for it.
    //
    // An earlier draft of this test asserted the OPPOSITE — that degree-zero resources are drawn —
    // on spec §6's "returns its rows with their degrees, zeros included". The measurement changed
    // the ruling: selecting purely by rank pads a sparse corpus with unconnected marks up to K,
    // rebuilding the 244-of-250 band on the corpus least able to absorb it.
    //
    // So they are excluded. What makes that legitimate rather than a silent omission — the clause
    // this whole goal sits under — is that the bounds still COUNT them.
    let (profile, ctx, _goal) = seed_context_with_goal_and_tasks(&pool, 2).await;
    let event = seed_genesis_event(&pool, profile, ctx).await;
    let lonely = seed_resource(&pool, ctx, profile, event, "Unconnected", "note").await;

    let sub =
        graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(50))
            .await
            .expect("entry read");

    assert!(
        !sub.nodes.iter().any(|n| n.id == lonely),
        "an unconnected resource is not drawn"
    );
    assert_eq!(
        sub.bounds.in_scope - sub.bounds.eligible,
        2,
        "but it IS counted — it and the L0 kernel telos are the two undrawn"
    );
    assert_eq!(sub.bounds.drawn, sub.nodes.len() as i32);
    assert!(
        sub.nodes.iter().all(|n| n.degree >= 1),
        "nothing below the floor reaches the canvas"
    );
    assert_no_dangling_edges(&sub);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn folded_edges_do_not_rank_anything(pool: PgPool) {
    // Spec §5.1 constraint 1. A folded edge is a RETRACTED assertion; counting it would never
    // surface as a crash, only as subtly wrong ordering — which is why it gets a test rather than
    // a comment. The predicate is not restated in the ranking function: it is delegated to
    // `edges_visible_to`, and this asserts that delegation actually holds.
    let (profile, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 1).await;
    let event = seed_genesis_event(&pool, profile, ctx).await;

    // A second resource wired out to FOUR more resources, then every one of those retracted.
    // Distinct targets are required, not incidental: `uq_kb_edges_assertion` makes an edge unique
    // per (source, target, kind, label, home) — an edge IS an assertion, and the same assertion
    // cannot be made twice.
    let ghost = seed_resource(&pool, ctx, profile, event, "Ghost", "note").await;
    for i in 0..4 {
        let leaf = seed_resource(&pool, ctx, profile, event, &format!("Leaf {i}"), "note").await;
        seed_contains_edge(&pool, ghost, leaf, ctx, event).await;
    }
    sqlx::query("UPDATE kb_edges SET is_folded = true WHERE source_id = $1")
        .bind(ghost)
        .execute(&pool)
        .await
        .expect("fold the ghost's edges");

    let sub =
        graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(50))
            .await
            .expect("entry read");

    // With the connection floor in place this is a sharper witness than a degree comparison: four
    // retracted edges leave the ghost at degree ZERO, so it falls below the floor entirely. Were
    // folding ignored it would rank at degree 4 and be drawn ahead of the goal.
    assert!(
        !sub.nodes.iter().any(|n| n.id == ghost),
        "four retracted edges must count for nothing — the ghost is below the floor"
    );
    let goal_node = sub.nodes.iter().find(|n| n.id == goal).expect("goal drawn");
    assert_eq!(goal_node.degree, 1, "the goal keeps its one LIVE edge");
    assert_no_dangling_edges(&sub);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn deny_direction_a_stranger_sees_nothing(pool: PgPool) {
    // Deny-as-absence: not an empty-but-existing answer, and no leak of existence through counts.
    let (owner, _ctx, goal) = seed_context_with_goal_and_tasks(&pool, 5).await;
    let stranger = seed_profile(&pool, "stranger").await;

    let owned: std::collections::HashSet<Uuid> =
        graph_service::entry_orientation_slice(&pool, ProfileId::from(owner), &[], Some(50))
            .await
            .expect("owner read")
            .nodes
            .iter()
            .map(|n| n.id)
            .collect();
    assert!(owned.contains(&goal), "the owner sees their own star");

    let sub =
        graph_service::entry_orientation_slice(&pool, ProfileId::from(stranger), &[], Some(50))
            .await
            .expect("entry read");

    // NOT `nodes.is_empty()`. Every profile sees the L0 kernel cogmap's telos — it is deliberately
    // public and root-team-joined (`internal/agents/key-patterns.md`), so an empty-canvas assertion
    // here would be testing the kernel's existence rather than this read's visibility gate. What
    // deny-as-absence actually claims is that none of the OWNER's material appears.
    for n in &sub.nodes {
        assert!(
            !owned.contains(&n.id) || n.id == KERNEL_TELOS,
            "stranger sees the owner's resource {} ({})",
            n.id,
            n.title
        );
    }
    assert!(
        !sub.nodes.iter().any(|n| n.id == goal),
        "and specifically not the goal"
    );
    assert!(
        sub.edges.is_empty(),
        "no edges either — the kernel telos is a lone node to a stranger"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn k_is_bounded_and_non_positive_is_refused(pool: PgPool) {
    let (profile, _ctx, _goal) = seed_context_with_goal_and_tasks(&pool, 3).await;

    assert!(
        graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(0))
            .await
            .is_err(),
        "k=0 is a caller error, not an empty canvas"
    );
    assert!(
        graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(-1))
            .await
            .is_err()
    );

    // Above the ceiling the read still answers — it clamps rather than refusing, because the
    // ceiling is our policy and not the caller's mistake.
    let sub =
        graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(10_000))
            .await
            .expect("clamped, not refused");
    // Four seeded and connected (a goal and three tasks). The public L0 kernel telos every profile
    // sees carries no resource-to-resource edges, so it sits below the floor: counted, not drawn.
    assert_eq!(sub.nodes.len(), 4, "the connected corpus, bounded above");
    assert!(
        !sub.nodes.iter().any(|n| n.id == KERNEL_TELOS),
        "the kernel telos is unconnected here, so it is declared rather than drawn"
    );
    assert_eq!(sub.bounds.in_scope, 5, "it is still counted");
    assert_eq!(sub.bounds.eligible, 4);
    assert!(!sub.bounds.truncated, "nothing eligible was left undrawn");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_deprecated_wrapper_and_the_canonical_body_cannot_drift(pool: PgPool) {
    // Spec §5.2's constraint, applied to §5.4's rename: "one body, two names". The old name is kept
    // only so the migration is additive for a binary that predates it, and it survives until chunk
    // E deletes the context door. Two SQL functions that must agree and are linked by nothing will
    // drift silently, both still returning plausible rows — so they are linked by this.
    let (profile, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 4).await;
    let event = seed_genesis_event(&pool, profile, ctx).await;
    let extra = seed_resource(&pool, ctx, profile, event, "Extra", "note").await;
    seed_contains_edge(&pool, goal, extra, ctx, event).await;

    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT resource_id FROM resources_visible_to($1)")
        .bind(profile)
        .fetch_all(&pool)
        .await
        .expect("visible ids");

    for depth in 0..=3_i32 {
        let canonical: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM graph_induced_edges($1, $2, $3) ORDER BY id")
                .bind(profile)
                .bind(&ids)
                .bind(depth)
                .fetch_all(&pool)
                .await
                .expect("canonical");

        let wrapper: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM graph_context_composition_edges($1, $2, $3) ORDER BY id",
        )
        .bind(profile)
        .bind(&ids)
        .bind(depth)
        .fetch_all(&pool)
        .await
        .expect("wrapper");

        assert_eq!(
            canonical, wrapper,
            "graph_context_composition_edges must remain the same body as graph_induced_edges (depth {depth})"
        );
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn depth_zero_is_induced_and_depth_one_expands(pool: PgPool) {
    // Why the service passes 0 and not a "sensible default". Depth 0 returns edges only among the
    // ids given; any depth above it reaches OUTWARD first and then returns the edges among
    // everything reached — which reintroduces endpoints that are not drawn. Production, K=130:
    // depth 0 → 275 edges, depth 1 → 2672.
    let (profile, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;
    let event = seed_genesis_event(&pool, profile, ctx).await;
    let outer = seed_resource(&pool, ctx, profile, event, "Outer", "note").await;
    seed_contains_edge(&pool, goal, outer, ctx, event).await;

    let just_the_goal = vec![goal];

    let induced: i64 = sqlx::query_scalar("SELECT count(*) FROM graph_induced_edges($1, $2, 0)")
        .bind(profile)
        .bind(&just_the_goal)
        .fetch_one(&pool)
        .await
        .expect("depth 0");
    assert_eq!(
        induced, 0,
        "the goal alone induces no edges — all four point off the given set"
    );

    let expanded: i64 = sqlx::query_scalar("SELECT count(*) FROM graph_induced_edges($1, $2, 1)")
        .bind(profile)
        .bind(&just_the_goal)
        .fetch_one(&pool)
        .await
        .expect("depth 1");
    assert_eq!(expanded, 4, "one hop out reaches all four and returns them");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_response_is_in_ranking_order(pool: PgPool) {
    // The contract, stated directly rather than inferred from `first()`/`last()`.
    //
    // This exists because a bite probe caught it missing: `graph_atlas_nodes_visible` carries no
    // `ORDER BY`, so before the service re-imposed the ranking, the node order was a query-plan
    // artifact — and reversing the ranking direction in SQL left order assertions still passing,
    // which is the precise shape of a test that cannot fail.
    let (profile, ctx, _goal) = seed_context_with_goal_and_tasks(&pool, 6).await;
    let event = seed_genesis_event(&pool, profile, ctx).await;
    seed_resource(&pool, ctx, profile, event, "Isolated", "note").await;

    let sub =
        graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(100))
            .await
            .expect("entry read");

    assert!(
        sub.nodes.len() > 3,
        "enough nodes for the order to mean something"
    );
    let degrees: Vec<i32> = sub.nodes.iter().map(|n| n.degree).collect();
    let mut sorted = degrees.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(
        degrees, sorted,
        "degrees must be non-increasing across the response — most-connected first"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_named_place_ranks_within_that_place(pool: PgPool) {
    // `[ruled — 2026-08-21, Pete]` What lets `/graph/@me?in=ctx` with no question be answered by
    // this read instead of by the recency page — which is the condition on spec §10.4's deletion of
    // `readSeedRows` actually being possible. §5.1's own headline is "A place, and no question at
    // all", and without scoping the read would silently answer across the reader's whole corpus,
    // ignoring the place they named.
    let (profile, ctx_a, goal_a) = seed_context_with_goal_and_tasks(&pool, 3).await;

    // A second context owned by the SAME profile — so both are visible, and only scoping separates
    // them. Wired more densely, so that without scoping it would outrank everything in ctx_a.
    let ctx_b = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_profiles', $2, $3, $3)",
    )
    .bind(ctx_b)
    .bind(profile)
    .bind(format!("ctx-b-{ctx_b}"))
    .execute(&pool)
    .await
    .expect("second context");
    let event_b = seed_genesis_event(&pool, profile, ctx_b).await;
    let hub_b = seed_resource(&pool, ctx_b, profile, event_b, "Hub B", "goal").await;
    for i in 0..8 {
        let leaf = seed_resource(&pool, ctx_b, profile, event_b, &format!("B{i}"), "task").await;
        seed_contains_edge(&pool, hub_b, leaf, ctx_b, event_b).await;
    }

    let unscoped =
        graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(50))
            .await
            .expect("unscoped");
    assert!(
        unscoped.nodes.iter().any(|n| n.id == hub_b)
            && unscoped.nodes.iter().any(|n| n.id == goal_a),
        "unscoped, both places are on the canvas"
    );

    let scoped =
        graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[ctx_a], Some(50))
            .await
            .expect("scoped");
    assert!(
        scoped.nodes.iter().any(|n| n.id == goal_a),
        "the named place's own material is drawn"
    );
    assert!(
        !scoped.nodes.iter().any(|n| n.id == hub_b),
        "and the other place's is absent — even though it is visible and ranks higher"
    );
    assert!(
        scoped.bounds.in_scope < unscoped.bounds.in_scope,
        "the denominator narrows with the scope, so the bound line describes the place asked about"
    );
    assert_no_dangling_edges(&scoped);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_bounds_declare_what_was_left_undrawn(pool: PgPool) {
    // §7.1: the bound line is chrome, not a warning — present whether or not the view is partial,
    // "so complete is something the reader is TOLD rather than something they infer from silence."
    // The composition trace used to hand it these numbers; the entry read runs no composition.
    let (profile, _ctx, _goal) = seed_context_with_goal_and_tasks(&pool, 20).await;

    let partial =
        graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(5))
            .await
            .expect("k=5");
    assert_eq!(partial.bounds.drawn, 5);
    assert_eq!(
        partial.bounds.eligible, 21,
        "goal plus twenty tasks are connected"
    );
    assert_eq!(
        partial.bounds.in_scope, 22,
        "the kernel telos is in scope but not eligible"
    );
    assert!(
        partial.bounds.truncated,
        "more eligible exist than were drawn"
    );

    let whole =
        graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(500))
            .await
            .expect("k=500");
    assert_eq!(whole.bounds.drawn, 21);
    assert!(
        !whole.bounds.truncated,
        "nothing eligible left over — and the reader is TOLD that, not left to infer it"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_corpus_with_no_structure_still_reports_its_size(pool: PgPool) {
    // Spec §6, rung 2 — the case the bounds exist for. A corpus with resources but no edges must be
    // able to say "a graph is the wrong instrument for this, here is the list view" rather than draw
    // nothing and look broken. It cannot say that from an empty payload alone; it needs the size.
    //
    // This is also why the bounds are read SEPARATELY from the ranking: counts carried on result
    // rows would vanish in exactly this case, which is the one that most needs explaining.
    let (profile, ctx, _g) = seed_context_with_goal_and_tasks(&pool, 0).await;
    let event = seed_genesis_event(&pool, profile, ctx).await;
    for i in 0..6 {
        seed_resource(&pool, ctx, profile, event, &format!("Loose {i}"), "note").await;
    }

    let sub =
        graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(50))
            .await
            .expect("entry read");

    assert!(sub.nodes.is_empty(), "nothing clears the floor");
    assert!(sub.edges.is_empty());
    assert_eq!(sub.bounds.eligible, 0, "which is the rung-2 signal");
    assert!(
        sub.bounds.in_scope >= 7,
        "but the reader still has material, and the surface is told how much"
    );
    assert!(!sub.bounds.truncated, "nothing eligible was withheld");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn nodes_carry_where_they_live_and_when_they_moved(pool: PgPool) {
    // `[ruled — 2026-08-21, Pete]` The orientation screen is the one a reader meets first, and its
    // marks have no `ResourceView` behind them — so without these two fields its hover cards would
    // be the thinnest on the surface, unable to say where a resource lives or when it last moved.
    //
    // `home_id` is an ID, not a decorated ref, and that is the point: rendering `@owner/slug`
    // server-side would duplicate `graph_home_contexts`' owner_ref CASE. The client already holds
    // every anchor it can read and resolves the id against them.
    let (profile, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;

    let sub =
        graph_service::entry_orientation_slice(&pool, ProfileId::from(profile), &[], Some(50))
            .await
            .expect("entry read");

    let node = sub.nodes.iter().find(|n| n.id == goal).expect("goal drawn");
    assert_eq!(
        node.home_id,
        Some(ctx),
        "the node names the anchor that homes it, so the client can resolve its ref locally"
    );
    assert!(
        node.updated.is_some(),
        "and carries its own recency rather than requiring a second read"
    );
    assert!(
        sub.nodes.iter().all(|n| n.home_id.is_some()),
        "every drawn node knows its home — a mark that cannot say where it lives is the thin card"
    );
}
