#![cfg(feature = "test-db")]

//! The edge-conjunction guard harness.
//!
//! # The invariant
//!
//! > Every function that returns, traverses, counts or scores edges for a caller admits an edge
//! > only if it is live, its home anchor is readable by the caller, and both endpoints are readable
//! > by the caller. A HISTORY surface may also show a folded edge, but only under the same home and
//! > endpoint conditions.
//!
//! Linking from what you author to anything you can read is safe exactly as far as this holds. A
//! surface that shows an edge on fewer conditions turns linking into a leak.
//!
//! # How it is guarded
//!
//! [`every_public_function_that_reads_kb_edges_is_classified`] enumerates every `public` function
//! OVERLOAD whose body (comments stripped) names `kb_edges`, and fails on any overload missing
//! from [`CLASSIFIED`], or classified but gone. Each overload, keyed `name/arity`, is in exactly
//! one class:
//!
//! - [`Class::Definition`]: `edges_visible_to` itself, the one set-based definition.
//! - [`Class::Delegates`]: every `edges_visible_to(..)` call in the body passes one of the
//!   function's own `p_*` parameters (never NULL, never a literal), and the body's direct
//!   `kb_edges` references are pinned to a count. A second direct read changes the count and
//!   fails the scan until it is reviewed.
//! - [`Class::Probed`]: caller-facing and spelling the conjunction itself. It names a probe below.
//! - [`Class::History`]: caller-facing and showing folded edges by design (a trail, a lineage, a
//!   feed, a staleness clock). It names a probe that also folds every edge and asserts the home
//!   and endpoint conditions still hold.
//! - [`Class::NotCallerFacing`]: write gates, projectors, producers, erasure acts and backfills,
//!   each with its reason.
//!
//! Every probe builds one world through the real write path: a readable control edge, and one
//! probe edge for each way an edge can be unreadable (its home in a context, its home in a cogmap,
//! its target, its source, a blob endpoint). It asserts the control is seen and every probe edge
//! is not.
//!
//! The Rust side is held the same way by [`every_rust_side_kb_edges_read_is_classified`]: every
//! table reference to `kb_edges` under `crates/*/src` and `api/` is counted per file and enclosing
//! `fn`, and the counts must match [`RUST_CLASSIFIED`].
//!
//! # Scope
//!
//! This scan covers edge ROWS read through `kb_edges`. Edge data read through `kb_edge_properties`,
//! the owner-polymorphic property views, or edge events in `kb_events` is out of scope for it.
//! Those reads answer to their own gates (`check_edge_mutable` for facets, the change feed and the
//! element trail for events, each probed here as a history surface).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sqlx::{PgPool, Row};
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, EdgeHome, SeedAction};
use temper_substrate::ids::{CogmapId, ContextId, EdgeId, EntityId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::writes;
use uuid::Uuid;

/// What a function overload that reads `kb_edges` is, with respect to the conjunction.
#[derive(Debug, Clone, Copy)]
enum Class {
    /// `edges_visible_to`: the definition everything else must agree with.
    Definition,
    /// Calls `edges_visible_to` with its own principal parameter; its direct `kb_edges`
    /// references, comments stripped, number exactly this many.
    Delegates { kb_edges_refs: usize },
    /// Caller-facing, spells the conjunction itself; names the probe test that proves it.
    Probed(&'static str),
    /// Caller-facing, shows folded edges by design; names the probe that proves the home and
    /// endpoint conditions on live and folded edges alike.
    History(&'static str),
    /// Not a caller's read: why.
    NotCallerFacing(&'static str),
}

/// This file's source, so a classification can be checked against the probe it names.
const THIS_FILE: &str = include_str!("edge_conjunction_harness.rs");

/// Every `public` function overload whose body names `kb_edges`, keyed `name/arity`, classified.
const CLASSIFIED: &[(&str, Class)] = &[
    ("edges_visible_to/1", Class::Definition),
    // ── Delegates ─────────────────────────────────────────────────────────────────────────────
    (
        "_resource_contradiction_balance/3",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    ("cogmap_regulation/3", Class::Delegates { kb_edges_refs: 1 }),
    ("graph_atlas_nodes/3", Class::Delegates { kb_edges_refs: 1 }),
    (
        "graph_atlas_nodes_cogmap/3",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    (
        "graph_atlas_nodes_visible/2",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    (
        "graph_cogmap_orphan_nodes/2",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    (
        "graph_context_containers/4",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    (
        "graph_context_residual_counts/5",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    (
        "graph_context_residual_members/6",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    (
        "graph_induced_edges/3",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    (
        "graph_orphan_salient_nodes/2",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    (
        "graph_territory_bridges/2",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    ("graph_traverse/3", Class::Delegates { kb_edges_refs: 2 }),
    (
        "graph_traverse_cogmap_scoped/5",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    (
        "graph_visible_degree_bounds/3",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    (
        "graph_visible_degree_ranking/4",
        Class::Delegates { kb_edges_refs: 1 },
    ),
    // ── Probed ────────────────────────────────────────────────────────────────────────────────
    (
        "__temper_ungated_follow_from_homed/11",
        Class::Probed("follow_from_admits_only_visible_edges"),
    ),
    (
        "graph_traverse_scoped/5",
        Class::Probed("the_scoped_traversal_admits_only_visible_edges"),
    ),
    // ── History ───────────────────────────────────────────────────────────────────────────────
    (
        "element_trail_edge/2",
        Class::History("the_edge_trail_admits_only_a_visible_edge"),
    ),
    (
        "resource_lineage/4",
        Class::History("lineage_walks_only_visible_edges"),
    ),
    (
        "context_changes_after/6",
        Class::History("the_change_feed_names_only_visible_edges"),
    ),
    (
        "context_cursor/2",
        Class::History("the_change_feed_names_only_visible_edges"),
    ),
    (
        "anchor_staleness/4",
        Class::History("staleness_is_touched_only_by_visible_edges"),
    ),
    // ── Not caller-facing ─────────────────────────────────────────────────────────────────────
    (
        "_edge_phantom_reassertions/1",
        Class::NotCallerFacing(
            "helper of element_trail_edge, which applies the home and endpoint checks to the edge \
             before any row it contributes is returned; no Rust caller",
        ),
    ),
    (
        "_project_relationship_asserted/2",
        Class::NotCallerFacing("projector: writes the edge row from its event"),
    ),
    (
        "_project_relationship_folded/2",
        Class::NotCallerFacing("projector: writes the fold from its event"),
    ),
    (
        "_project_relationship_retyped/2",
        Class::NotCallerFacing("projector: writes the retype from its event"),
    ),
    (
        "_project_relationship_reweighted/2",
        Class::NotCallerFacing("projector: writes the reweight from its event"),
    ),
    (
        "relationship_assert/5",
        Class::NotCallerFacing(
            "write path: authorized in Rust (check_home_authorable, endpoint readability) before \
             it is called",
        ),
    ),
    (
        "relationship_fold/5",
        Class::NotCallerFacing("write path: authorized by check_edge_mutable before it is called"),
    ),
    (
        "relationship_retype/5",
        Class::NotCallerFacing("write path: authorized by check_edge_mutable before it is called"),
    ),
    (
        "relationship_reweight/5",
        Class::NotCallerFacing("write path: authorized by check_edge_mutable before it is called"),
    ),
    (
        "_property_owner_anchor/2",
        Class::NotCallerFacing(
            "write guard: resolves the anchor a property's owner is homed in, for \
             facet_set/property_set; returns no edge",
        ),
    ),
    (
        "_resource_write_guard_owner/2",
        Class::NotCallerFacing(
            "write guard inside the projectors: resolves the owning resource of a write",
        ),
    ),
    (
        "_resource_erasure_apply_redaction/3",
        Class::NotCallerFacing("erasure act: redacts what the act's scope names"),
    ),
    (
        "_resource_erasure_trail_scope/1",
        Class::NotCallerFacing("erasure act: computes the trail the act redacts"),
    ),
    (
        "resource_erasure_execute/5",
        Class::NotCallerFacing("erasure act: authorized in resource_erasure_service"),
    ),
    (
        "resource_erasure_survey_plan/1",
        Class::NotCallerFacing(
            "erasure act's own scope computation, shared with the survey door, which authorizes \
             the actor over the resource first",
        ),
    ),
    (
        "backfill_goal_parent_of_to_advances/0",
        Class::NotCallerFacing("one-shot data backfill, run by a migration"),
    ),
    (
        "cogmap_region_centrality/1",
        Class::NotCallerFacing(
            "producer: computes a stored region aggregate at materialize; not returned per caller",
        ),
    ),
    (
        "cogmap_region_internal_tension/2",
        Class::NotCallerFacing(
            "producer: computes a stored region aggregate at materialize; not returned per caller",
        ),
    ),
    (
        "context_goal_liveness/2",
        Class::NotCallerFacing(
            "producer: feeds the telos embedding at materialize; not returned per caller",
        ),
    ),
    (
        "resource_contradiction_balance/1",
        Class::NotCallerFacing(
            "producer: feeds refresh_resource_standing's memo, which is not a read surface; the \
             evidence read (GET /api/resources/{id}/evidence, resource_standing_shape) calls \
             _resource_contradiction_balance instead",
        ),
    ),
];

// =================================================================================================
// The scan.
// =================================================================================================

/// One overload, as the scan reads it: comment-stripped body, parameter names.
struct Overload {
    body: String,
    params: Vec<String>,
}

/// The arguments of every `edges_visible_to(..)` call in `body`.
fn edges_visible_to_args(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let needle = "edges_visible_to";
    let mut from = 0;
    while let Some(i) = body[from..].find(needle) {
        let start = from + i;
        let after = &body[start + needle.len()..];
        let prev_ok = start == 0 || {
            let c = body.as_bytes()[start - 1];
            !(c.is_ascii_alphanumeric() || c == b'_')
        };
        let trimmed = after.trim_start();
        if prev_ok && trimmed.starts_with('(') {
            if let Some(close) = trimmed.find(')') {
                out.push(trimmed[1..close].trim().to_string());
            }
        }
        from = start + needle.len();
    }
    out
}

/// Direct `kb_edges` references in `body` (a whole-word match).
fn kb_edges_refs(body: &str) -> usize {
    let bytes = body.as_bytes();
    body.match_indices("kb_edges")
        .filter(|(i, m)| {
            let before = *i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let end = i + m.len();
            let after =
                end >= bytes.len() || !(bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_');
            before && after
        })
        .count()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_public_function_that_reads_kb_edges_is_classified(pool: PgPool) -> sqlx::Result<()> {
    // Per overload, comments stripped, so a comment naming a call or a table is not a read.
    let rows = sqlx::query(
        "SELECT p.proname || '/' || p.pronargs AS key, \
                regexp_replace(regexp_replace(p.prosrc, '/\\*.*?\\*/', '', 'g'), '--[^\\n]*', '', 'g') AS body, \
                coalesce(p.proargnames[1:p.pronargs], '{}') AS params \
           FROM pg_proc p \
           JOIN pg_namespace n ON n.oid = p.pronamespace \
          WHERE n.nspname = 'public' AND p.prokind = 'f' \
          ORDER BY 1",
    )
    .fetch_all(&pool)
    .await?;
    let scanned: BTreeMap<String, Overload> = rows
        .iter()
        .map(|r| {
            (
                r.get::<String, _>("key"),
                Overload {
                    body: r.get("body"),
                    params: r.get("params"),
                },
            )
        })
        .filter(|(_, o)| kb_edges_refs(&o.body) > 0)
        .collect();

    assert!(
        scanned.contains_key("edges_visible_to/1") && scanned.len() > 10,
        "the scan returned {} overload(s) — the vocabulary no longer matches, so this gate is \
         vacuous",
        scanned.len()
    );

    let classified: BTreeMap<&str, Class> = CLASSIFIED.iter().copied().collect();
    assert_eq!(
        classified.len(),
        CLASSIFIED.len(),
        "an overload is classified twice"
    );

    let unclassified: Vec<&String> = scanned
        .keys()
        .filter(|n| !classified.contains_key(n.as_str()))
        .collect();
    assert!(
        unclassified.is_empty(),
        "{} overload(s) read kb_edges and are not classified: {unclassified:?}\n\n\
         Every function that returns, traverses, counts or scores edges for a caller must admit an \
         edge only if it is live, its home is readable and both endpoints are readable. Call \
         edges_visible_to with your own principal (DELEGATES), or add a probe here (PROBED or \
         HISTORY), or say why it is not a caller's read (NOT_CALLER_FACING).",
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

    for (key, class) in CLASSIFIED {
        let o = &scanned[*key];
        let args = edges_visible_to_args(&o.body);
        match class {
            Class::Delegates { kb_edges_refs: n } => {
                assert!(
                    !args.is_empty(),
                    "{key} is classified DELEGATES but its body does not call edges_visible_to"
                );
                for arg in &args {
                    assert!(
                        o.params.iter().any(|p| p == arg),
                        "{key} calls edges_visible_to({arg}): DELEGATES must pass its own \
                         principal parameter, one of {:?}",
                        o.params
                    );
                }
                let found = kb_edges_refs(&o.body);
                assert_eq!(
                    found, *n,
                    "{key} now references kb_edges {found} time(s), not {n}: review whether every \
                     read still reaches edges through edges_visible_to, then update the count"
                );
            }
            Class::Probed(probe) | Class::History(probe) => {
                assert!(
                    args.is_empty(),
                    "{key} now calls edges_visible_to: reclassify it as DELEGATES"
                );
                assert!(
                    THIS_FILE.contains(&format!("async fn {probe}(")),
                    "{key} names the probe {probe}, which is not a test in this file"
                );
            }
            Class::NotCallerFacing(reason) => {
                assert!(
                    args.is_empty(),
                    "{key} now calls edges_visible_to: reclassify it as DELEGATES"
                );
                assert!(
                    !reason.trim().is_empty(),
                    "{key}: say why it is not a caller's read"
                );
            }
            Class::Definition => {}
        }
    }
    Ok(())
}

// =================================================================================================
// The probe world.
// =================================================================================================

/// ```text
///   team A  (reader is a member)  ─ owns ─ context C:  resources a, b, c
///   team B  (owner only)          ─ owns ─ context U:  resource  h, blob z
///                                 ─ joins ─ cogmap M
///
///   e_ok    a ─▸ b   homed in C     the control: everything readable
///   e_home  a ─▸ c   homed in U     both endpoints readable, context home NOT
///   e_cog   a ─▸ b   homed in M     both endpoints readable, cogmap home NOT
///   e_end   a ─▸ h   homed in C     home readable, TARGET not
///   e_src   h ─▸ a   homed in C     home readable, SOURCE not
///   e_blob  a ─▸ z   homed in C     home readable, blob endpoint not
/// ```
///
/// Created in that order, so every probe edge is a newer touch on C than `e_ok`. The resource
/// edges are `leads_to`/`derived_from`, so the lineage walk sees all of them.
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
    /// Every probe edge, each unreadable for a different reason.
    probes: Vec<(&'static str, Uuid)>,
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

async fn assert_edge(pool: &PgPool, src: Uuid, tgt: Uuid, home: EdgeHome, emitter: Uuid) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    let id: EdgeId = fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(ResourceId::from(src)),
            tgt: AnchorRef::resource(ResourceId::from(tgt)),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("derived_from"),
            weight: 1.0,
            home,
            emitter: EntityId::from(emitter),
        },
    )
    .await
    .expect("assert edge")
    .relationship()
    .expect("an edge id");
    tx.commit().await.unwrap();
    id.uuid()
}

async fn edge(pool: &PgPool, src: Uuid, tgt: Uuid, home: Uuid, emitter: Uuid) -> Uuid {
    assert_edge(
        pool,
        src,
        tgt,
        EdgeHome::Context(ContextId::from(home)),
        emitter,
    )
    .await
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
    let cogmap_m: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (id, name, telos_resource_id) \
         VALUES (uuid_generate_v7(), 'ec-m', $1) RETURNING id",
    )
    .bind(h)
    .fetch_one(pool)
    .await
    .expect("cogmap M");
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap_m)
        .bind(team_b)
        .execute(pool)
        .await
        .expect("M joined to B");
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY id LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("an event");
    let blob: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_blobs (id, content_hash, content_type, content_bytes, \
                               asserted_by_event_id, last_event_id, home_table, home_id, \
                               owner_profile_id, originator_profile_id) \
         VALUES (uuid_generate_v7(), 'ec-z', 'text/plain', 1, $1, $1, 'kb_contexts', $2, $3, $3) \
         RETURNING id",
    )
    .bind(event)
    .bind(ctx_u)
    .bind(owner)
    .fetch_one(pool)
    .await
    .expect("blob z");

    let e_ok = edge(pool, a, b, ctx_c, emitter).await;
    let e_home = edge(pool, a, c, ctx_u, emitter).await;
    let e_cog = assert_edge(
        pool,
        a,
        b,
        EdgeHome::Cogmap(CogmapId::from(cogmap_m)),
        emitter,
    )
    .await;
    let e_end = edge(pool, a, h, ctx_c, emitter).await;
    let e_src = edge(pool, h, a, ctx_c, emitter).await;
    let e_blob: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_edges (id, source_table, source_id, target_table, target_id, edge_kind, \
                               label, home_anchor_table, home_anchor_id, asserted_by_event_id, \
                               last_event_id) \
         VALUES (uuid_generate_v7(), 'kb_resources', $1, 'kb_blobs', $2, 'leads_to', \
                 'derived_from', 'kb_contexts', $3, $4, $4) RETURNING id",
    )
    .bind(a)
    .bind(blob)
    .bind(ctx_c)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("e_blob");

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
        probes: vec![
            ("an edge whose context HOME the reader cannot read", e_home),
            ("an edge whose cogmap HOME the reader cannot read", e_cog),
            ("an edge whose TARGET the reader cannot read", e_end),
            ("an edge whose SOURCE the reader cannot read", e_src),
            ("an edge whose BLOB endpoint the reader cannot read", e_blob),
        ],
    };
    // The world is what it claims: the definition admits the control and no probe edge.
    let visible: BTreeSet<Uuid> = sqlx::query_scalar("SELECT edge_id FROM edges_visible_to($1)")
        .bind(w.reader)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert!(visible.contains(&w.e_ok), "fixture: the control is visible");
    for (why, e) in &w.probes {
        assert!(!visible.contains(e), "fixture: {why} is not visible");
    }
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

/// Fold every edge in the world through the write path: the control first, then the probe edges,
/// so every probe fold is a newer touch than the control's. The blob edge, which carries no
/// assertion event of its own, is folded in place.
async fn fold_all(pool: &PgPool, w: &World) {
    writes::fold_relationship(pool, EdgeId::from(w.e_ok), None, EntityId::from(w.emitter))
        .await
        .expect("fold the control");
    for (_, e) in w.probes.iter().take(4) {
        writes::fold_relationship(pool, EdgeId::from(*e), None, EntityId::from(w.emitter))
            .await
            .expect("fold a probe edge");
    }
    sqlx::query("UPDATE kb_edges SET is_folded = true WHERE id = $1")
        .bind(w.probes[4].1)
        .execute(pool)
        .await
        .expect("fold the blob edge");
}

/// The verdict every probe asserts: the control is seen, no probe edge is.
fn assert_conjunction(what: &str, w: &World, seen: &BTreeSet<Uuid>) {
    assert!(
        seen.contains(&w.e_ok),
        "{what}: the readable control edge is missing, so the absences below would be vacuous"
    );
    for (why, e) in &w.probes {
        assert!(!seen.contains(e), "{what}: admitted {why}");
    }
}

// =================================================================================================
// The follow-from walk and graph_traverse.
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

/// The walk admits an edge only under the conjunction: from `a` it reaches `b` through the control
/// alone. `c` sits behind an unreadable context home, the cogmap-homed edge to `b` adds nothing,
/// `h` is unreadable, and no `via` entry names a probe edge's far end. The ranking is identical
/// with the probe edges live and folded away.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn follow_from_admits_only_visible_edges(pool: PgPool) {
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
        "only the control's target is reached"
    );
    let via: String = rows.iter().map(|r| r.get::<String, _>("via")).collect();
    assert!(!via.contains(&w.c.to_string()) && !via.contains(&w.h.to_string()));
    assert_eq!(
        via.matches("\"source_id\"").count(),
        1,
        "exactly one parent edge, the control: {via}"
    );

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

    // Ranking: the answer with every probe edge live equals the answer with them folded away.
    let with_edges = expand(&pool, &w).await;
    sqlx::query("UPDATE kb_edges SET is_folded = true WHERE id = ANY($1)")
        .bind(w.probes.iter().map(|(_, e)| *e).collect::<Vec<_>>())
        .execute(&pool)
        .await
        .unwrap();
    let without_edges = expand(&pool, &w).await;
    assert_eq!(
        with_edges, without_edges,
        "an edge the caller cannot see must not move search_graph_expand's ranking"
    );
    assert_eq!(
        with_edges.len(),
        1,
        "and the ranking is not empty: {with_edges:?}"
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

/// `graph_traverse` keeps its depth clamp: on a cycle, a huge `p_depth` still stops at 10. Without
/// the clamp the walk never terminates, because `UNION` keeps every row that differs by depth.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn graph_traverse_clamps_its_depth_on_a_cycle(pool: PgPool) {
    let w = world(&pool).await;
    edge(&pool, w.b, w.a, w.ctx_c, w.emitter).await; // a ⇄ b, both readable
    let deepest: Option<i32> =
        sqlx::query_scalar("SELECT max(depth) FROM graph_traverse($1, $2, 1000000)")
            .bind(w.reader)
            .bind(vec![w.a])
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        deepest,
        Some(10),
        "the walk ran to the clamp and stopped there"
    );
}

// =================================================================================================
// PROBED.
// =================================================================================================

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

// =================================================================================================
// HISTORY: live, then with every edge folded.
// =================================================================================================

async fn trail_seen(pool: &PgPool, w: &World) -> BTreeSet<Uuid> {
    let mut seen = BTreeSet::new();
    for e in std::iter::once(w.e_ok).chain(w.probes.iter().map(|(_, e)| *e)) {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM element_trail_edge($1, $2)")
            .bind(w.reader)
            .bind(e)
            .fetch_one(pool)
            .await
            .unwrap();
        if n > 0 {
            seen.insert(e);
        }
    }
    seen
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_edge_trail_admits_only_a_visible_edge(pool: PgPool) {
    let w = world(&pool).await;
    assert_conjunction("element_trail_edge", &w, &trail_seen(&pool, &w).await);
    fold_all(&pool, &w).await;
    assert_conjunction(
        "element_trail_edge, folded",
        &w,
        &trail_seen(&pool, &w).await,
    );
}

async fn lineage_seen(pool: &PgPool, w: &World) -> BTreeSet<Uuid> {
    let mut seen = BTreeSet::new();
    for direction in ["ancestors", "descendants"] {
        let ids: Vec<Uuid> =
            sqlx::query_scalar("SELECT edge_id FROM resource_lineage($1, $2, $3, 16)")
                .bind(w.reader)
                .bind(w.a)
                .bind(direction)
                .fetch_all(pool)
                .await
                .unwrap();
        seen.extend(ids);
    }
    seen
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn lineage_walks_only_visible_edges(pool: PgPool) {
    let w = world(&pool).await;
    assert_conjunction("resource_lineage", &w, &lineage_seen(&pool, &w).await);
    fold_all(&pool, &w).await;
    assert_conjunction(
        "resource_lineage, folded",
        &w,
        &lineage_seen(&pool, &w).await,
    );
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

async fn feed_check(pool: &PgPool, w: &World, what: &str) {
    settle(pool).await;
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
    let text = format!("{}{}", feed(w.ctx_c).await, feed(w.ctx_u).await);
    let seen: BTreeSet<Uuid> = std::iter::once(w.e_ok)
        .chain(w.probes.iter().map(|(_, e)| *e))
        .filter(|e| text.contains(&e.to_string()))
        .collect();
    assert_conjunction(what, w, &seen);

    // The cursor never points at an event that writes a probe edge.
    let probe_events: BTreeSet<Uuid> = sqlx::query_scalar(
        "SELECT ev.id FROM kb_events ev \
          WHERE (ev.payload ->> 'edge_id')::uuid = ANY($1)",
    )
    .bind(w.probes.iter().map(|(_, e)| *e).collect::<Vec<_>>())
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .collect();
    for ctx in [w.ctx_c, w.ctx_u] {
        let cursor: Option<Uuid> = sqlx::query_scalar("SELECT context_cursor($1, $2)")
            .bind(w.reader)
            .bind(ctx)
            .fetch_one(pool)
            .await
            .unwrap();
        assert!(
            cursor.is_none_or(|c| !probe_events.contains(&c)),
            "{what}: context_cursor points at an event that writes a probe edge"
        );
    }
}

/// The change feed of C names the control and never a probe edge; the feed of U (whose home the
/// reader cannot read) names nothing. The cursor never points at a probe edge's event. Then the
/// same with every edge folded, when the feed carries the fold events too.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_change_feed_names_only_visible_edges(pool: PgPool) {
    let w = world(&pool).await;
    feed_check(&pool, &w, "context_changes_after").await;
    fold_all(&pool, &w).await;
    feed_check(&pool, &w, "context_changes_after, folded").await;
}

async fn latest_touch(
    pool: &PgPool,
    w: &World,
    ctx: Uuid,
) -> Option<chrono::DateTime<chrono::Utc>> {
    sqlx::query_scalar(
        "SELECT latest_touch FROM anchor_staleness('kb_contexts', $1, 'profile', $2)",
    )
    .bind(ctx)
    .bind(w.reader)
    .fetch_optional(pool)
    .await
    .unwrap()
    .flatten()
}

async fn touched_at(pool: &PgPool, e: Uuid) -> chrono::DateTime<chrono::Utc> {
    sqlx::query_scalar(
        "SELECT ev.occurred_at FROM kb_edges e JOIN kb_events ev ON ev.id = e.last_event_id \
          WHERE e.id = $1",
    )
    .bind(e)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// C's latest touch is the control's, not any newer probe edge's; U answers nothing to the reader.
/// Staleness is folded-inclusive by design, so the same holds once every edge is folded.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn staleness_is_touched_only_by_visible_edges(pool: PgPool) {
    let w = world(&pool).await;
    for folded in [false, true] {
        if folded {
            fold_all(&pool, &w).await;
        }
        let ok_at = touched_at(&pool, w.e_ok).await;
        // e_src is homed in C and is a newer touch than the control, live and folded alike, or the
        // equality below would prove nothing.
        assert!(
            touched_at(&pool, w.probes[3].1).await > ok_at,
            "fixture (folded: {folded}): a probe edge in C is the newer touch"
        );
        assert_eq!(
            latest_touch(&pool, &w, w.ctx_c).await,
            Some(ok_at),
            "anchor_staleness (folded: {folded}): C's latest touch must be the readable control"
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
}

// =================================================================================================
// DELEGATES with a principal kind of their own: the map's principal arm, and the standing shape.
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

/// The standing shape's contradiction balance counts only edges the reader can see: a readable
/// `supports` edge scores, one homed where the reader cannot read does not. The context and map
/// principals count only edges homed in that context or map.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn standing_scores_only_edges_the_reader_sees(pool: PgPool) {
    let w = world(&pool).await;
    let support = |src: Uuid, home: Uuid| {
        let pool = pool.clone();
        let (b, emitter) = (w.b, w.emitter);
        async move {
            writes::assert_relationship(
                &pool,
                writes::AssertParams {
                    src: ResourceId::from(src),
                    tgt: ResourceId::from(b),
                    kind: EdgeKind::Express,
                    polarity: EdgePolarity::Forward,
                    label: Some("supports"),
                    weight: 1.0,
                    home: ContextId::from(home),
                    emitter: EntityId::from(emitter),
                },
            )
            .await
            .unwrap();
        }
    };
    let balance = || {
        let pool = pool.clone();
        let (b, reader) = (w.b, w.reader);
        async move {
            sqlx::query_scalar::<_, Option<f64>>(
                "SELECT contradiction_balance FROM resource_standing_shape($1, 'profile', $2)",
            )
            .bind(b)
            .bind(reader)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    support(w.a, w.ctx_c).await;
    assert_eq!(
        balance().await,
        Some(1.0),
        "control: a readable supports edge scores"
    );
    support(w.c, w.ctx_u).await;
    assert_eq!(
        balance().await,
        Some(1.0),
        "a supports edge homed where the reader cannot read must not score"
    );

    // A context's own principal counts only edges homed in it between its own resources.
    let as_kind = |kind: &'static str, principal: Uuid| {
        let pool = pool.clone();
        let b = w.b;
        async move {
            sqlx::query_scalar::<_, Option<f64>>(
                "SELECT contradiction_balance FROM resource_standing_shape($1, $2, $3)",
            )
            .bind(b)
            .bind(kind)
            .bind(principal)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(
        as_kind("context", w.ctx_c).await,
        Some(1.0),
        "the context principal counts the edge homed in it, not the one homed in U"
    );

    // A map's own principal counts only edges homed in that map between resources it can access.
    let map: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (id, name, telos_resource_id) VALUES (uuid_generate_v7(), \
         'ec-standing-map', $1) RETURNING id",
    )
    .bind(w.a)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE kb_resource_homes SET anchor_table = 'kb_cogmaps', anchor_id = $1 \
          WHERE resource_id = ANY($2)",
    )
    .bind(map)
    .bind(vec![w.a, w.b])
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        as_kind("cogmap", map).await,
        Some(0.0),
        "the map principal counts no edge homed in a context, though it can access both endpoints"
    );
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(ResourceId::from(w.a)),
            tgt: AnchorRef::resource(ResourceId::from(w.b)),
            kind: EdgeKind::Express,
            polarity: EdgePolarity::Forward,
            label: Some("contradicts"),
            weight: 1.0,
            home: EdgeHome::Cogmap(CogmapId::from(map)),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        as_kind("cogmap", map).await,
        Some(-1.0),
        "an edge homed in the map itself counts for the map principal"
    );
}

// =================================================================================================
// The Rust side.
// =================================================================================================

/// What a Rust-side `kb_edges` read is. Same meaning as [`Class`]; a site that spells the
/// conjunction inline says where.
#[derive(Debug, Clone, Copy)]
enum RustClass {
    /// The enclosing fn reaches edges through `edges_visible_to` (verified in its text).
    Delegates,
    /// The query spells the conjunction (home readable, both endpoints readable) inline.
    InlineConjunction(&'static str),
    NotCallerFacing(&'static str),
}

/// `(file under the repo root, enclosing fn, table references to kb_edges in it)` → class.
const RUST_CLASSIFIED: &[(&str, &str, usize, RustClass)] = &[
    (
        "crates/temper-services/src/services/edge_service.rs",
        "list_resource_edges",
        1,
        RustClass::Delegates,
    ),
    (
        "crates/temper-services/src/services/edge_service.rs",
        "list_resource_connections",
        2,
        RustClass::Delegates,
    ),
    (
        "crates/temper-services/src/services/edge_service.rs",
        "list_edge_facets",
        1,
        RustClass::Delegates,
    ),
    (
        "crates/temper-substrate/src/readback/mod.rs",
        "blob_relations",
        1,
        RustClass::Delegates,
    ),
    (
        "crates/temper-services/src/backend/substrate_read.rs",
        "filtered_visible_page",
        1,
        RustClass::Delegates,
    ),
    (
        "crates/temper-services/src/backend/db_backend.rs",
        "check_edge_mutable",
        1,
        RustClass::NotCallerFacing(
            "the edge-mutation write gate; it joins edges_visible_to and answers a hidden edge \
             as missing",
        ),
    ),
    (
        "crates/temper-substrate/src/readback/mod.rs",
        "invocation_show",
        1,
        RustClass::InlineConjunction(
            "an event naming an edge is withheld unless anchor_readable_by_profile(home) and both \
             endpoints are readable",
        ),
    ),
    (
        "crates/temper-services/src/backend/db_backend.rs",
        "fold_goal_edges",
        1,
        RustClass::NotCallerFacing("write path: the goal-patch act folds the actor's own edges"),
    ),
    (
        "crates/temper-services/src/backend/db_backend.rs",
        "lock_goal_rows",
        1,
        RustClass::NotCallerFacing("write path: locks the goal-patch act's rows"),
    ),
    (
        "crates/temper-services/src/backend/db_backend.rs",
        "validate_keyed_edge_write",
        1,
        RustClass::NotCallerFacing("write gate, after check_edge_mutable"),
    ),
    (
        "crates/temper-services/src/services/blob_service.rs",
        "delete_blob",
        1,
        RustClass::NotCallerFacing(
            "write gate: the strike enumerates every live relation to require delete standing on \
             each peer",
        ),
    ),
    (
        "crates/temper-services/src/services/resource_erasure_service.rs",
        "other_author_edges",
        1,
        RustClass::NotCallerFacing("erasure act's scope"),
    ),
    (
        "crates/temper-services/src/services/resource_erasure_service.rs",
        "other_author_edge_properties",
        1,
        RustClass::NotCallerFacing("erasure act's scope"),
    ),
    (
        "crates/temper-services/src/services/resource_erasure_service.rs",
        "blob_co_links",
        1,
        RustClass::NotCallerFacing("erasure act's scope"),
    ),
    (
        "crates/temper-substrate/src/readback/mod.rs",
        "find_edge",
        1,
        RustClass::NotCallerFacing("write path: idempotency lookup before an assert"),
    ),
    (
        "crates/temper-substrate/src/readback/mod.rs",
        "neighbors",
        2,
        RustClass::NotCallerFacing("test oracle for the neighbors parity test; no product caller"),
    ),
    (
        "crates/temper-substrate/src/replay.rs",
        "?",
        1,
        RustClass::NotCallerFacing("replay's table snapshot, compared byte for byte"),
    ),
    (
        "crates/temper-substrate/src/scenario/runner.rs",
        "apply_mutation",
        1,
        RustClass::NotCallerFacing("scenario DSL runner (fixtures)"),
    ),
    (
        "crates/temper-substrate/src/substrate.rs",
        "load",
        1,
        RustClass::NotCallerFacing(
            "producer: the region kernel's input at materialize; not returned per caller",
        ),
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

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The name of the `fn` a line declares, if it declares one.
fn fn_name(line: &str) -> Option<String> {
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
}

/// Is the `kb_edges` at byte `at` of `text` a table reference? Walks back over an optional
/// `public.` prefix, then whitespace and `\` line continuations (so `FROM` may sit on an earlier
/// line), and accepts a preceding `FROM`/`JOIN`/`UPDATE`/`INTO`/`USING` keyword or a comma join.
fn is_table_ref(text: &str, at: usize) -> bool {
    let b = text.as_bytes();
    let mut k = at;
    if k >= 7 && &text[k - 7..k] == "public." {
        k -= 7;
    }
    while k > 0 && (b[k - 1].is_ascii_whitespace() || b[k - 1] == b'\\') {
        k -= 1;
    }
    if k > 0 && b[k - 1] == b',' {
        return true;
    }
    let end = k;
    while k > 0 && is_word(b[k - 1]) {
        k -= 1;
    }
    let word = text[k..end].to_ascii_uppercase();
    ["FROM", "JOIN", "UPDATE", "INTO", "USING"].contains(&word.as_str())
}

/// `(file, enclosing fn)` → (table references to `kb_edges`, the fn's text), over every non-comment
/// line under `crates/*/src` and `api/`. `?` when no `fn` precedes the reference.
fn rust_side_reads() -> BTreeMap<(String, String), (usize, String)> {
    let root = repo_root();
    let mut files = Vec::new();
    for krate in std::fs::read_dir(root.join("crates")).expect("crates") {
        let src = krate.expect("crate").path().join("src");
        if src.is_dir() {
            rust_sources(&src, &mut files);
        }
    }
    rust_sources(&root.join("api"), &mut files);

    let mut out: BTreeMap<(String, String), (usize, String)> = BTreeMap::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read source");
        let rel = file
            .strip_prefix(&root)
            .expect("under the root")
            .to_string_lossy()
            .replace('\\', "/");
        let lines: Vec<&str> = text.lines().collect();
        let mut line_starts = Vec::with_capacity(lines.len());
        let mut pos = 0;
        for l in &lines {
            line_starts.push(pos);
            pos += l.len() + 1;
        }
        for (at, m) in text.match_indices("kb_edges") {
            let b = text.as_bytes();
            let end = at + m.len();
            if (at > 0 && is_word(b[at - 1])) || (end < b.len() && is_word(b[end])) {
                continue;
            }
            let line = line_starts.partition_point(|&s| s <= at) - 1;
            if lines[line].trim_start().starts_with("//") || !is_table_ref(&text, at) {
                continue;
            }
            let decl = (0..=line).rev().find(|&i| fn_name(lines[i]).is_some());
            let (name, body) = match decl {
                Some(d) => {
                    let next = (d + 1..lines.len())
                        .find(|&i| fn_name(lines[i]).is_some())
                        .unwrap_or(lines.len());
                    (
                        fn_name(lines[d]).expect("a declaration"),
                        lines[d..next].join("\n"),
                    )
                }
                None => ("?".to_string(), String::new()),
            };
            let entry = out.entry((rel.clone(), name)).or_insert((0, body));
            entry.0 += 1;
        }
    }
    out
}

/// Every Rust-side table reference to `kb_edges` is classified, counted per fn, and nothing
/// classified is stale. A DELEGATES fn must really reach `edges_visible_to`.
#[test]
fn every_rust_side_kb_edges_read_is_classified() {
    let found = rust_side_reads();
    assert!(
        found.len() > 10,
        "the Rust-side scan found {} site(s): the pattern no longer matches, so this is vacuous",
        found.len()
    );
    let classified: BTreeMap<(String, String), (usize, RustClass)> = RUST_CLASSIFIED
        .iter()
        .map(|(f, n, c, k)| ((f.to_string(), n.to_string()), (*c, *k)))
        .collect();
    let unclassified: Vec<_> = found
        .keys()
        .filter(|k| !classified.contains_key(*k))
        .collect();
    assert!(
        unclassified.is_empty(),
        "Rust-side kb_edges read(s) not classified in RUST_CLASSIFIED: {unclassified:?}\n\
         Join edges_visible_to, spell the conjunction, or say why the site is not a caller's read."
    );
    let stale: Vec<_> = classified
        .keys()
        .filter(|k| !found.contains_key(*k))
        .collect();
    assert!(
        stale.is_empty(),
        "classified Rust-side site(s) no longer read kb_edges: {stale:?}"
    );
    for (key, (count, class)) in &classified {
        let (seen, body) = &found[key];
        assert_eq!(
            seen, count,
            "{key:?} now reads kb_edges {seen} time(s), not {count}: classify the new read"
        );
        match class {
            RustClass::Delegates => assert!(
                body.contains("edges_visible_to"),
                "{key:?} is classified Delegates but never reaches edges_visible_to"
            ),
            RustClass::InlineConjunction(r) | RustClass::NotCallerFacing(r) => {
                assert!(!r.trim().is_empty(), "{key:?}: give the reason")
            }
        }
    }
}
