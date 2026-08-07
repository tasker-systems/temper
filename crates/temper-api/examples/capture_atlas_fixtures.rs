//! capture_atlas_fixtures — build a real-shaped `/dev/atlas` fixture bundle from a live database.
//!
//! # Why this exists in Rust rather than as a browser console script
//!
//! The harness renders the real `AtlasPage` against captured `AtlasViewData` scenarios. A fixture
//! is only useful if it is **shaped by the API contract**: a bundle transformed out of a database
//! dump will render and lie, because the mapping from SQL rows to wire DTOs (`TerritoryKind`
//! selection, `salience: Some(..)` vs `None`, the visible-member gate, the orphan-node sparsity
//! rule) lives in the service layer, not in the schema.
//!
//! So this tool calls **the same `graph_service` / `context_graph_service` functions the HTTP
//! handlers call**, with the same arguments and defaults, and serializes their real return types.
//! The payload halves of the bundle are therefore the wire types by construction — there is no
//! hand-shaping step that could drift. `crates/temper-api/src/handlers/graph.rs` is the file to
//! read beside this one; each `capture_*` below names the handler it mirrors.
//!
//! The predecessor recipe drove a logged-in browser and captured SvelteKit's `__data.json`,
//! because the `/api/graph/*` reads are server-side and never touch the browser network tab.
//! That path needed a human to paste a script into devtools, so it was not reproducible by
//! anyone who was not in the session. This is.
//!
//! # What it does NOT do
//!
//! It does not sanitize. Output holds real titles, handles, excerpts and ids, so it is written to
//! the **gitignored** `atlas-fixtures.local.json` and passed through
//! `scripts/sanitize-atlas-fixtures.mjs` to produce the committed bundle. See
//! `packages/temper-ui/src/routes/dev/atlas/README.md`.
//!
//! # Usage
//!
//! ```bash
//! SQLX_OFFLINE=true \
//! DATABASE_URL="$(neonctl connection-string main \
//!     --project-id <project> --org-id <org> \
//!     --role-name neondb_owner --database-name neondb 2>/dev/null | tail -1)" \
//!   cargo run -p temper-api --example capture_atlas_fixtures -- \
//!     --handle j-cole-taylor \
//!     --out packages/temper-ui/static/dev/atlas-fixtures.local.json
//! ```
//!
//! **`SQLX_OFFLINE=true` is not optional here.** `sqlx`'s compile-time `query!` macros read
//! `DATABASE_URL`, so without it the *build* would verify every macro in the dependency tree
//! against the production database instead of the committed `.sqlx` cache — slow, and it fails
//! outright whenever prod's schema and `migrations/` have not converged. Offline mode pins
//! compilation to the cache while the *runtime* connection still uses the URL. (`cargo make`
//! tasks set this globally, which is why the hazard only appears on a bare `cargo run`.)
//!
//! The connection is opened with `default_transaction_read_only=on`, so the session cannot write
//! even if a called service function were changed to attempt one. Anchor selection is
//! **discovered, never hardcoded** — region ids in particular are ephemeral (the steward re-sweeps
//! clusters and folds the old rows), and a stale one previously produced a hard 500.

use std::collections::BTreeMap;

use serde::Serialize;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::context_ref::ContextRef;
use temper_core::types::element_trail::{ElementKind, EventTrail};
use temper_core::types::graph_atlas::{AtlasSubgraph, SliceRequest};
use temper_core::types::graph_context::ContextPanorama;
use temper_core::types::graph_home::{AtlasHome, HomeCogmap, HomeContext};
use temper_core::types::graph_territory::TerritoryOverview;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_core::types::resource_view::ResourceView;
use temper_services::backend::DbBackend;
use temper_services::services::context_graph_service::{self, ResidualMemberQuery};
use temper_services::services::context_service::resolve_context_ref;
use temper_services::services::{event_service, graph_service};
use temper_workflow::operations::{Backend, ShowResource, Surface};

// ── Handler-mirrored constants ─────────────────────────────────────────────────────────────────
// These are the surface defaults, not new choices. `graph.rs` holds the originals; a divergence
// here would silently capture a payload no real caller ever receives.

/// Mirrors `graph.rs`'s private `CONTAINER_WALK_DEPTH`. The panorama's residual buckets are
/// computed by walking containers to this depth, and a bucket drill MUST resolve its members at
/// the same depth or it seeds a different set than the tray displayed.
const CONTAINER_WALK_DEPTH: i32 = 2;

/// Mirrors `parse_container_types(None)` — the `["goal"]` default (spec D4).
fn default_container_types() -> Vec<String> {
    vec!["goal".to_string()]
}

/// Mirrors `ContextPanoramaQuery::group_by`'s `unwrap_or("doc_type")` default (spec D2).
const DEFAULT_GROUP_BY: &str = "doc_type";

/// Mirrors `+page.server.ts`'s `NEIGHBORHOOD_DEPTH`.
const NEIGHBORHOOD_DEPTH: u32 = 2;

/// Mirrors `readRegionComposition`'s `depth = 1` default.
const COMPOSITION_DEPTH: i32 = 1;

/// The `~`-join `nav.ts::territoryIds` splits a shift-selected region union on.
const UNION_SEP: &str = "~";

// ── The view wrapper ───────────────────────────────────────────────────────────────────────────
// `AtlasViewData` is a TypeScript interface (`src/lib/graph/atlas/viewData.ts`) composed by the
// `/graph/[owner]` page load: the wire payloads above plus URL-derived navigation state. Only the
// navigation half is mirrored here; the payload half is the real DTOs.
//
// Drift on the navigation half is caught on the TS side: `fixtures.test.ts` pins the key set to
// the type via `satisfies Record<keyof AtlasViewData, true>`, so adding or removing an
// `AtlasViewData` field fails `bun run check` until the fixtures are regenerated.

/// Mirrors `nav.ts::Focus`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Focus {
    None,
    Territory {
        id: String,
    },
    Node {
        id: Uuid,
    },
    Container {
        id: Uuid,
    },
    Bucket {
        #[serde(rename = "groupKey")]
        group_key: String,
        value: String,
    },
}

/// Mirrors `nav.ts::SelectedElement`. The `edge` variant is part of the union the TS type
/// declares; no captured scenario selects an edge today.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum SelectedElement {
    None,
    Node { id: Uuid },
}

/// Mirrors `nav.ts::GraphFilters`. The page load hands both branches the same default bag.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GraphFilters {
    lens_id: Option<String>,
    edge_kinds: Vec<String>,
    doc_types: Vec<String>,
}

impl GraphFilters {
    /// Mirrors `+page.server.ts`'s `defaultFilters`.
    fn default_bag() -> Self {
        Self {
            lens_id: None,
            edge_kinds: vec![],
            doc_types: vec![],
        }
    }
}

/// Mirrors `viewData.ts::CrumbTerritory`.
#[derive(Debug, Clone, Serialize)]
struct CrumbTerritory {
    id: String,
    label: Option<String>,
}

/// Mirrors `viewData.ts::AtlasViewData` — one harness scenario.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AtlasViewData {
    owner: String,
    cogmap_id: Option<Uuid>,
    cogmap_name: Option<String>,
    tier: u8,
    focus: Focus,
    home: Option<AtlasHome>,
    context_slug: Option<String>,
    panorama: Option<ContextPanorama>,
    territories: Option<TerritoryOverview>,
    neighborhood: Option<AtlasSubgraph>,
    selection: SelectedElement,
    trail: Option<EventTrail>,
    resource_row: Option<ResourceView>,
    filters: GraphFilters,
    focus_path: Vec<Focus>,
    crumb_territory: Option<CrumbTerritory>,
    scope_filter: Option<String>,
}

impl AtlasViewData {
    /// The all-null baseline every scenario starts from, so a new `AtlasViewData` field is added
    /// in exactly one place rather than fourteen. `tier` mirrors `nav.ts::deriveTier`, which is
    /// a pure function of focus — so it is derived here rather than passed, and cannot disagree
    /// with the focus it accompanies.
    fn base(owner: &str, focus: Focus) -> Self {
        let tier = match focus {
            Focus::None => 0,
            Focus::Territory { .. } | Focus::Container { .. } | Focus::Bucket { .. } => 1,
            Focus::Node { .. } => 2,
        };
        Self {
            owner: owner.to_string(),
            cogmap_id: None,
            cogmap_name: None,
            tier,
            focus,
            home: None,
            context_slug: None,
            panorama: None,
            territories: None,
            neighborhood: None,
            selection: SelectedElement::None,
            trail: None,
            resource_row: None,
            filters: GraphFilters::default_bag(),
            focus_path: vec![],
            crumb_territory: None,
            scope_filter: None,
        }
    }
}

/// The emitted bundle: `_meta` plus one entry per scenario. A `BTreeMap` keeps key order stable
/// across runs so a re-capture diffs cleanly instead of churning the whole file.
#[derive(Debug, Serialize)]
struct Bundle {
    _meta: BundleMeta,
    #[serde(flatten)]
    scenarios: BTreeMap<String, AtlasViewData>,
}

#[derive(Debug, Serialize)]
struct BundleMeta {
    /// False — this is a raw capture. The sanitizer replaces this stamp with `synthetic: true`,
    /// and `fixtures.test.ts` asserts the committed bundle carries the sanitized one. That
    /// assertion is what stops a raw capture reaching a public repository.
    synthetic: bool,
    captured_from: String,
    note: String,
}

// ── Anchor discovery ───────────────────────────────────────────────────────────────────────────

/// The anchors a capture run selected, reported to stderr so the operator can see which real
/// places the bundle is standing on — and notice when the corpus has moved enough that a
/// scenario no longer finds its shape.
#[derive(Debug)]
struct Picks {
    /// Most live regions — the singleton-dominated panorama case.
    dense_cogmap: HomeCogmap,
    /// Resources but zero live regions — the cold-start case. `None` if every reachable cogmap
    /// has derived structure.
    cold_cogmap: Option<HomeCogmap>,
    /// Most container territories — the organized-context case.
    dense_context: (HomeContext, ContextPanorama),
    /// **Volume with the least organization**: the context whose largest share of material
    /// reaches no container at all. Selected by residual FRACTION above a volume floor, not by
    /// `containers.is_empty()` — measured 2026-08-01, no context with real volume has zero
    /// containers, so a zero-container rule picks a one-resource context and the scenario
    /// silently stops being about volume. `None` if nothing clears the floor.
    sparse_context: Option<(HomeContext, ContextPanorama)>,
    /// The smallest readable context — the near-empty anchor a reader can still arrive at.
    ///
    /// Note this is the *smallest*, not an EMPTY one. `graph_home_contexts` applies no
    /// resource-count filter, so an empty context would appear here if one were readable; on the
    /// measured corpus none is. A wholly empty anchor is therefore **declared uncovered** rather
    /// than faked — see the README's known-gaps section.
    smallest_context: Option<(HomeContext, ContextPanorama)>,
}

/// Total resources in a panorama's residual tray — the material reaching no container.
fn residual_total(p: &ContextPanorama) -> i32 {
    p.residual.buckets.iter().map(|b| b.count).sum()
}

/// Minimum resource count for a context to be a candidate for the sparse-anchor scenario. Below
/// this a context is "small", not "unorganized", and the two are different shapes.
const SPARSE_VOLUME_FLOOR: i32 = 20;

/// How many contexts to probe for container structure. Probing calls the real panorama read per
/// context, so this is a cost bound, not a display bound — it is reported when it binds.
const CONTEXT_PROBE_LIMIT: usize = 8;

fn pick_cogmaps(home: &AtlasHome) -> Result<(HomeCogmap, Option<HomeCogmap>), String> {
    let dense = home
        .research
        .iter()
        .max_by_key(|c| c.region_count)
        .cloned()
        .ok_or_else(|| {
            "profile reaches no cogmaps — cannot capture a cogmap scenario".to_string()
        })?;
    if dense.region_count == 0 {
        return Err(
            "no reachable cogmap has a live region — every cogmap panorama would be \
                    the cold-start case, so there is no dense panorama to capture"
                .to_string(),
        );
    }
    // Cold start: no derived structure at all, but real material behind it. Prefer the one with
    // the most facets, so the orphan-node sparsity rule has something to surface.
    let cold = home
        .research
        .iter()
        .filter(|c| c.region_count == 0 && c.facet_count > 0)
        .max_by_key(|c| c.facet_count)
        .cloned();
    Ok((dense, cold))
}

/// Probe the profile's contexts for container structure. Returns the densest, a cold-start one,
/// and an empty one — the three context-side shapes the corpus actually reaches.
async fn probe_contexts(
    pool: &PgPool,
    profile: ProfileId,
    home: &AtlasHome,
) -> Result<
    (
        (HomeContext, ContextPanorama),
        Option<(HomeContext, ContextPanorama)>,
        Option<(HomeContext, ContextPanorama)>,
    ),
    String,
> {
    let mut populated: Vec<HomeContext> = home
        .build
        .iter()
        .filter(|c| c.resource_count > 0)
        .cloned()
        .collect();
    populated.sort_by_key(|c| std::cmp::Reverse(c.resource_count));
    if populated.len() > CONTEXT_PROBE_LIMIT {
        eprintln!(
            "  note: {} populated contexts, probing the {CONTEXT_PROBE_LIMIT} largest — a \
             smaller context with denser containers would not be seen",
            populated.len()
        );
        populated.truncate(CONTEXT_PROBE_LIMIT);
    }

    let mut probed: Vec<(HomeContext, ContextPanorama)> = Vec::new();
    for ctx in populated {
        let panorama = read_context_panorama(pool, profile, ctx.id).await?;
        let residual = residual_total(&panorama);
        eprintln!(
            "  probed context {:<28} resources={:<5} containers={:<4} buckets={:<3} \
             residual={:<5} ({:.0}% uncontained)",
            truncate(&ctx.name, 28),
            ctx.resource_count,
            panorama.containers.len(),
            panorama.residual.buckets.len(),
            residual,
            100.0 * f64::from(residual) / f64::from(ctx.resource_count.max(1))
        );
        probed.push((ctx, panorama));
    }

    let dense = probed
        .iter()
        .max_by_key(|(_, p)| p.containers.len())
        .cloned()
        .ok_or_else(|| "profile has no context with resources".to_string())?;
    if dense.1.containers.is_empty() {
        eprintln!(
            "  warning: no probed context has ANY container territory — the 'organized context' \
             scenario will be a second cold-start case, not a contrast"
        );
    }

    // Volume with the least organization. Ranked by the FRACTION of material reaching no
    // container, so the scenario stays about unorganized volume even as the corpus grows.
    // f64 has no total order, hence the scaled-integer key rather than partial_cmp.
    let sparse = probed
        .iter()
        .filter(|(c, _)| c.resource_count >= SPARSE_VOLUME_FLOOR)
        .max_by_key(|(c, p)| {
            (1000_i64 * i64::from(residual_total(p))) / i64::from(c.resource_count)
        })
        .cloned();
    if sparse.is_none() {
        eprintln!(
            "  warning: no context clears the {SPARSE_VOLUME_FLOOR}-resource volume floor — \
             skipping the unorganized-volume scenario rather than shrinking it to a small context"
        );
    }

    // The near-empty anchor. Smallest, not empty: see `Picks::smallest_context`.
    let smallest = probed.iter().min_by_key(|(c, _)| c.resource_count).cloned();
    if let Some((c, _)) = &smallest {
        if c.resource_count > 0 {
            eprintln!(
                "  note: no readable context is EMPTY; smallest is {:?} at {} resource(s). The \
                 wholly-empty anchor is declared uncovered, not faked.",
                c.name, c.resource_count
            );
        }
    }

    Ok((dense, sparse, smallest))
}

// ── Read wrappers, each mirroring one handler ──────────────────────────────────────────────────

/// Mirrors `graph.rs::context_panorama`: resolve the ref (the visibility gate), then read with
/// the surface's own defaults.
async fn read_context_panorama(
    pool: &PgPool,
    profile: ProfileId,
    context_uuid: Uuid,
) -> Result<ContextPanorama, String> {
    let context_id = resolve_context_ref(pool, profile, &ContextRef::Id(context_uuid))
        .await
        .map_err(|e| format!("resolve context {context_uuid}: {e}"))?;
    context_graph_service::context_panorama(
        pool,
        profile,
        context_id,
        DEFAULT_GROUP_BY,
        &default_container_types(),
        CONTAINER_WALK_DEPTH,
    )
    .await
    .map_err(|e| format!("context panorama {context_uuid}: {e}"))
}

/// Mirrors `graph.rs::context_composition`'s container arm — seeds are the single container id.
async fn read_container_composition(
    pool: &PgPool,
    profile: ProfileId,
    container: Uuid,
) -> Result<AtlasSubgraph, String> {
    context_graph_service::context_composition(pool, profile, &[container], COMPOSITION_DEPTH)
        .await
        .map_err(|e| format!("container composition {container}: {e}"))
}

/// Mirrors `graph.rs::context_composition`'s bucket arm — resolve the bucket's members at the
/// panorama's walk depth first, then compose. Using the drill depth here instead would seed a
/// different set than the tray displayed.
async fn read_bucket_composition(
    pool: &PgPool,
    profile: ProfileId,
    context_id: ContextId,
    group_key: &str,
    group_value: &str,
) -> Result<AtlasSubgraph, String> {
    let seeds = context_graph_service::residual_member_ids(
        pool,
        ResidualMemberQuery {
            profile_id: profile,
            context_id,
            group_key,
            group_value,
            container_types: &default_container_types(),
            depth: CONTAINER_WALK_DEPTH,
        },
    )
    .await
    .map_err(|e| format!("residual members {group_key}:{group_value}: {e}"))?;
    if seeds.is_empty() {
        return Err(format!(
            "residual bucket {group_key}:{group_value} resolved to zero members"
        ));
    }
    context_graph_service::context_composition(pool, profile, &seeds, COMPOSITION_DEPTH)
        .await
        .map_err(|e| format!("bucket composition {group_key}:{group_value}: {e}"))
}

/// Mirrors `graph.rs::cogmap_panorama` with no lens override (the D2 default-lens path).
async fn read_cogmap_panorama(
    pool: &PgPool,
    profile: ProfileId,
    cogmap: Uuid,
) -> Result<TerritoryOverview, String> {
    graph_service::cogmap_panorama(pool, profile, cogmap, None)
        .await
        .map_err(|e| format!("cogmap panorama {cogmap}: {e}"))
}

/// Mirrors `graph.rs::region_composition`.
async fn read_region_composition(
    pool: &PgPool,
    profile: ProfileId,
    regions: &[Uuid],
) -> Result<AtlasSubgraph, String> {
    graph_service::region_composition_slice(pool, profile, regions, COMPOSITION_DEPTH)
        .await
        .map_err(|e| format!("region composition {regions:?}: {e}"))
}

/// Mirrors `graph.rs::cogmap_neighborhood_slice`.
async fn read_neighborhood(
    pool: &PgPool,
    profile: ProfileId,
    cogmap: Uuid,
    seed: Uuid,
) -> Result<AtlasSubgraph, String> {
    graph_service::cogmap_neighborhood_slice(
        pool,
        profile,
        cogmap,
        SliceRequest {
            seeds: vec![seed],
            depth: NEIGHBORHOOD_DEPTH,
            edge_kinds: vec![],
        },
    )
    .await
    .map_err(|e| format!("neighborhood {seed}: {e}"))
}

/// Mirrors `events.rs::element_trail` for a node.
async fn read_node_trail(
    pool: &PgPool,
    profile: ProfileId,
    node: Uuid,
) -> Result<EventTrail, String> {
    event_service::element_trail(pool, profile, ElementKind::Node, node)
        .await
        .map_err(|e| format!("trail {node}: {e}"))
}

/// Mirrors `resources.rs::get` — the same `ShowResource` command through the same backend.
async fn read_resource_row(
    pool: &PgPool,
    profile: ProfileId,
    node: Uuid,
) -> Result<ResourceView, String> {
    let backend = DbBackend::new(pool.clone(), profile);
    backend
        .show_resource(ShowResource {
            resource: ResourceId::from(node),
            origin: Surface::ApiHttp,
        })
        .await
        .map(|out| out.value)
        .map_err(|e| format!("resource row {node}: {e}"))
}

// ── Scenario capture ───────────────────────────────────────────────────────────────────────────

/// The cogmap-door scenarios: panorama, region drill, region union, and the three node views.
///
/// Every id is derived from the payload just read — never carried across runs. Region ids in
/// particular are ephemeral, and a stale one previously produced a hard 500.
async fn capture_cogmap_scenarios(
    pool: &PgPool,
    profile: ProfileId,
    owner: &str,
    cogmap: &HomeCogmap,
    out: &mut BTreeMap<String, AtlasViewData>,
) -> Result<(), String> {
    let panorama = read_cogmap_panorama(pool, profile, cogmap.id).await?;
    eprintln!(
        "  cogmap {:<28} regions={:<5} singletons={:<5} orphans={}",
        truncate(&cogmap.name, 28),
        panorama.territories.len(),
        panorama
            .territories
            .iter()
            .filter(|t| t.member_count == 1)
            .count(),
        panorama.orphan_nodes.len()
    );

    out.insert("cogmapPanorama".into(), {
        let mut v = AtlasViewData::base(owner, Focus::None);
        v.cogmap_id = Some(cogmap.id);
        v.cogmap_name = Some(cogmap.name.clone());
        v.territories = Some(panorama.clone());
        v
    });

    // Prefer multi-member regions for the drill: a singleton region's "interior" is the same
    // object as its overview mark, so drilling one shows nothing new. Fall back to whatever
    // exists rather than failing — a wholly singleton map is itself a real shape.
    let mut regions: Vec<&_> = panorama.territories.iter().collect();
    regions.sort_by_key(|t| std::cmp::Reverse(t.member_count));
    let primary = regions
        .first()
        .ok_or_else(|| format!("cogmap {} has no visible region to drill", cogmap.name))?;
    let primary_id = primary.id;
    let drill = read_region_composition(pool, profile, &[primary_id]).await?;

    out.insert("regionDrill".into(), {
        let focus = Focus::Territory {
            id: primary_id.to_string(),
        };
        let mut v = AtlasViewData::base(owner, focus.clone());
        v.cogmap_id = Some(cogmap.id);
        v.cogmap_name = Some(cogmap.name.clone());
        v.neighborhood = Some(drill.clone());
        v.focus_path = vec![focus];
        v.crumb_territory = Some(CrumbTerritory {
            id: primary_id.to_string(),
            label: None,
        });
        v
    });

    if let Some(second) = regions.get(1) {
        let union_id = format!("{primary_id}{UNION_SEP}{}", second.id);
        let union = read_region_composition(pool, profile, &[primary_id, second.id]).await?;
        out.insert("regionDrillUnion".into(), {
            let focus = Focus::Territory {
                id: union_id.clone(),
            };
            let mut v = AtlasViewData::base(owner, focus.clone());
            v.cogmap_id = Some(cogmap.id);
            v.cogmap_name = Some(cogmap.name.clone());
            v.neighborhood = Some(union);
            v.focus_path = vec![focus];
            v.crumb_territory = Some(CrumbTerritory {
                id: union_id,
                label: Some("2 regions".to_string()),
            });
            v
        });
    } else {
        eprintln!("  warning: only one region available — skipping regionDrillUnion");
    }

    capture_node_scenarios(pool, profile, owner, cogmap, &drill, primary_id, out).await
}

/// The three node views, seeded from a drill's composition: a cogmap-homed node's neighborhood,
/// that node selected (trail + resource row), a context-homed node selected under the territory
/// focus, and the lowest-degree node as the neighbor-less leaf case.
async fn capture_node_scenarios(
    pool: &PgPool,
    profile: ProfileId,
    owner: &str,
    cogmap: &HomeCogmap,
    drill: &AtlasSubgraph,
    territory_id: Uuid,
    out: &mut BTreeMap<String, AtlasViewData>,
) -> Result<(), String> {
    use temper_core::types::graph_atlas::NodeHome;

    let node = drill
        .nodes
        .iter()
        .find(|n| n.home == NodeHome::Cogmap)
        .or_else(|| drill.nodes.first())
        .ok_or_else(|| "region drill returned no node to focus".to_string())?;
    let node_uuid = node.id;
    let neighborhood = read_neighborhood(pool, profile, cogmap.id, node_uuid).await?;

    let node_view = |neigh: AtlasSubgraph, selected: bool| {
        let focus = Focus::Node { id: node_uuid };
        let mut v = AtlasViewData::base(owner, focus.clone());
        v.cogmap_id = Some(cogmap.id);
        v.cogmap_name = Some(cogmap.name.clone());
        v.neighborhood = Some(neigh);
        v.focus_path = vec![focus];
        if selected {
            v.selection = SelectedElement::Node { id: node_uuid };
        }
        v
    };

    out.insert(
        "nodeNeighborhood".into(),
        node_view(neighborhood.clone(), false),
    );

    let mut selected = node_view(neighborhood, true);
    selected.trail = Some(read_node_trail(pool, profile, node_uuid).await?);
    selected.resource_row = Some(read_resource_row(pool, profile, node_uuid).await?);
    out.insert("nodeSelected".into(), selected);

    // A context-homed node inside the composition, selected on top of the TERRITORY focus — the
    // cross-home rail case. Selection is orthogonal to focus here, so the focus path stays at the
    // territory level; that is what `?sel=node` on top of a territory focus produces.
    match drill.nodes.iter().find(|n| n.home == NodeHome::Context) {
        Some(ctx_node) => {
            let ctx_uuid = ctx_node.id;
            let focus = Focus::Territory {
                id: territory_id.to_string(),
            };
            let mut v = AtlasViewData::base(owner, focus.clone());
            v.cogmap_id = Some(cogmap.id);
            v.cogmap_name = Some(cogmap.name.clone());
            v.neighborhood = Some(read_region_composition(pool, profile, &[territory_id]).await?);
            v.focus_path = vec![focus];
            v.crumb_territory = Some(CrumbTerritory {
                id: territory_id.to_string(),
                label: None,
            });
            v.selection = SelectedElement::Node { id: ctx_uuid };
            v.trail = Some(read_node_trail(pool, profile, ctx_uuid).await?);
            v.resource_row = Some(read_resource_row(pool, profile, ctx_uuid).await?);
            out.insert("nodeSelectedContext".into(), v);
        }
        None => eprintln!(
            "  warning: region drill surfaced no context-homed node — skipping \
             nodeSelectedContext (the cross-home rail case)"
        ),
    }

    // The neighbour-less leaf: lowest degree in the drill. Its neighborhood is read the same way
    // as any other node's — the point is that the read comes back (near-)empty and the rail still
    // opens off the resource row.
    let leaf = drill
        .nodes
        .iter()
        .min_by_key(|n| n.degree)
        .ok_or_else(|| "region drill returned no node for the leaf case".to_string())?;
    let leaf_uuid = leaf.id;
    let focus = Focus::Node { id: leaf_uuid };
    let mut leaf_view = AtlasViewData::base(owner, focus.clone());
    leaf_view.cogmap_id = Some(cogmap.id);
    leaf_view.cogmap_name = Some(cogmap.name.clone());
    leaf_view.neighborhood = Some(read_neighborhood(pool, profile, cogmap.id, leaf_uuid).await?);
    leaf_view.focus_path = vec![focus];
    leaf_view.selection = SelectedElement::Node { id: leaf_uuid };
    leaf_view.trail = Some(read_node_trail(pool, profile, leaf_uuid).await?);
    leaf_view.resource_row = Some(read_resource_row(pool, profile, leaf_uuid).await?);
    out.insert("leafBare".into(), leaf_view);

    Ok(())
}

/// A context-door panorama scenario under a chosen key.
fn context_panorama_view(
    owner: &str,
    ctx: &HomeContext,
    panorama: ContextPanorama,
) -> AtlasViewData {
    let mut v = AtlasViewData::base(owner, Focus::None);
    v.context_slug = Some(ctx.slug.clone());
    v.panorama = Some(panorama);
    v
}

/// Drill a panorama's largest residual bucket — the only path to material that reaches no
/// container, and therefore the reachability question the tray exists to answer.
async fn capture_residual_drill(
    pool: &PgPool,
    profile: ProfileId,
    owner: &str,
    ctx: &HomeContext,
    panorama: &ContextPanorama,
    out: &mut BTreeMap<String, AtlasViewData>,
) -> Result<(), String> {
    let Some(bucket) = panorama.residual.buckets.iter().max_by_key(|b| b.count) else {
        eprintln!(
            "  warning: {:?} has an empty residual tray — skipping residualDrill",
            ctx.name
        );
        return Ok(());
    };
    let context_id = resolve_context_ref(pool, profile, &ContextRef::Id(ctx.id))
        .await
        .map_err(|e| format!("resolve context for residual drill: {e}"))?;
    let group_key = panorama.residual.group_key.clone();
    let subgraph =
        read_bucket_composition(pool, profile, context_id, &group_key, &bucket.value).await?;
    eprintln!(
        "  residual drill {}:{} — {} in bucket, {} nodes / {} edges returned",
        group_key,
        bucket.value,
        bucket.count,
        subgraph.nodes.len(),
        subgraph.edges.len()
    );
    let focus = Focus::Bucket {
        group_key,
        value: bucket.value.clone(),
    };
    let mut v = AtlasViewData::base(owner, focus.clone());
    v.context_slug = Some(ctx.slug.clone());
    v.neighborhood = Some(subgraph);
    v.focus_path = vec![focus];
    out.insert("residualDrill".into(), v);
    Ok(())
}

/// The context-door scenarios: the organized panorama and its container drill, the residual
/// drill at real scale, the unorganized-volume anchor, and the near-empty anchor.
async fn capture_context_scenarios(
    pool: &PgPool,
    profile: ProfileId,
    owner: &str,
    picks: &Picks,
    out: &mut BTreeMap<String, AtlasViewData>,
) -> Result<(), String> {
    let (dense_ctx, dense_panorama) = &picks.dense_context;
    out.insert(
        "contextPanorama".into(),
        context_panorama_view(owner, dense_ctx, dense_panorama.clone()),
    );

    if let Some(container) = dense_panorama.containers.first() {
        let focus = Focus::Container { id: container.id };
        let mut v = AtlasViewData::base(owner, focus.clone());
        v.context_slug = Some(dense_ctx.slug.clone());
        v.neighborhood = Some(read_container_composition(pool, profile, container.id).await?);
        v.focus_path = vec![focus];
        out.insert("contextDrill".into(), v);
    } else {
        eprintln!("  warning: densest context has no container — skipping contextDrill");
    }

    // The residual drill runs on the DENSEST context, not the sparsest: 61% of the largest
    // context reaches no container, so this is where "region-less material is reachable" is a
    // question about scale rather than about a handful of stragglers. Note the asymmetry it
    // exercises — `context_composition` caps SEEDS (250) but has no node cap, where the cogmap
    // door's `region_composition_slice` truncates at 120 nodes.
    capture_residual_drill(pool, profile, owner, dense_ctx, dense_panorama, out).await?;

    // Volume with the least organization — the anchor whose material overwhelmingly reaches no
    // container. This is `entry-does-not-presume-organization` with real weight behind it.
    if let Some((sparse_ctx, sparse_panorama)) = &picks.sparse_context {
        out.insert(
            "sparseAnchor".into(),
            context_panorama_view(owner, sparse_ctx, sparse_panorama.clone()),
        );
    } else {
        eprintln!("  warning: no unorganized-volume context found — skipping sparseAnchor");
    }

    // The near-empty anchor: a real place a reader can arrive at with essentially nothing to
    // organize by. Not a WHOLLY empty one — none is readable on the measured corpus, and that
    // shape is declared uncovered rather than fabricated.
    if let Some((small_ctx, small_panorama)) = &picks.smallest_context {
        out.insert(
            "nearEmptyAnchor".into(),
            context_panorama_view(owner, small_ctx, small_panorama.clone()),
        );
    } else {
        eprintln!("  warning: profile reaches no context at all — skipping nearEmptyAnchor");
    }

    Ok(())
}

// ── Entry point ────────────────────────────────────────────────────────────────────────────────

struct Args {
    handle: String,
    out: String,
}

fn parse_args() -> Result<Args, String> {
    let mut handle = None;
    let mut out = None;
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--handle" => handle = argv.next(),
            "--out" => out = argv.next(),
            "--help" | "-h" => {
                return Err("usage: --handle <profile-handle> --out <path.json>".to_string())
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Args {
        handle: handle.ok_or("--handle is required")?,
        out: out.ok_or("--out is required")?,
    })
}

/// Resolve the profile whose reach the capture runs under.
///
/// This is the one query the tool issues itself, and it is deliberately the runtime-checked
/// `query_scalar` rather than the `query!` macro: an example is not covered by the workspace
/// `cargo sqlx prepare` ritual, so a macro here would add an orphan `.sqlx` entry that the
/// offline build then has to carry.
async fn resolve_profile(pool: &PgPool, handle: &str) -> Result<ProfileId, String> {
    let id: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle = $1")
        .bind(handle)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("profile lookup: {e}"))?
        .ok_or_else(|| format!("no profile with handle {handle:?}"))?;
    Ok(ProfileId::from(id))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
}

async fn run() -> Result<(), String> {
    let args = parse_args()?;
    let url = std::env::var("DATABASE_URL").map_err(|_| {
        "DATABASE_URL is not set — see this file's header for the recipe".to_string()
    })?;

    // Read-only at the session level: the tool calls only read services, and this makes that a
    // property of the connection rather than of the call sites.
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("SET default_transaction_read_only = on")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .map_err(|e| format!("connect: {e}"))?;

    let profile = resolve_profile(&pool, &args.handle).await?;
    eprintln!("capturing as @{} ({})", args.handle, profile.as_uuid());

    let home = graph_service::atlas_home(&pool, profile)
        .await
        .map_err(|e| format!("atlas home: {e}"))?;
    eprintln!(
        "  home: {} contexts, {} cogmaps",
        home.build.len(),
        home.research.len()
    );

    let (dense_cogmap, cold_cogmap) = pick_cogmaps(&home)?;
    let (dense_context, sparse_context, smallest_context) =
        probe_contexts(&pool, profile, &home).await?;
    let picks = Picks {
        dense_cogmap,
        cold_cogmap,
        dense_context,
        sparse_context,
        smallest_context,
    };

    // `AtlasViewData.owner` is the ROUTE PARAM, not data read from the database — it is
    // `params.owner` from `/graph/[owner]`. The self-addressed route the UI actually navigates
    // to is `/graph/@me`, which is what every real page load (and every prior capture, raw
    // included) carries. Emitting the literal handle instead would be both less faithful and a
    // personal-data leak into a public repository, since nothing downstream treats this field
    // as sensitive.
    let owner = "@me".to_string();
    let mut scenarios: BTreeMap<String, AtlasViewData> = BTreeMap::new();

    // The unaddressed front door — the whole footprint, nothing named. Carries the corpus's
    // concentration (one enormous anchor and a tail of near-empty ones) for free.
    scenarios.insert("home".into(), {
        let mut v = AtlasViewData::base(&owner, Focus::None);
        v.home = Some(home.clone());
        v
    });

    capture_cogmap_scenarios(&pool, profile, &owner, &picks.dense_cogmap, &mut scenarios).await?;

    // A cogmap with material and no derived structure at all: territories empty, orphan nodes
    // carrying the whole view via the sparsity rule.
    match &picks.cold_cogmap {
        Some(cold) => {
            let panorama = read_cogmap_panorama(&pool, profile, cold.id).await?;
            eprintln!(
                "  cold cogmap {:<22} regions={} orphans={}",
                truncate(&cold.name, 22),
                panorama.territories.len(),
                panorama.orphan_nodes.len()
            );
            scenarios.insert("coldStartCogmap".into(), {
                let mut v = AtlasViewData::base(&owner, Focus::None);
                v.cogmap_id = Some(cold.id);
                v.cogmap_name = Some(cold.name.clone());
                v.territories = Some(panorama);
                v
            });
        }
        None => eprintln!(
            "  warning: every reachable cogmap has live regions — skipping coldStartCogmap"
        ),
    }

    capture_context_scenarios(&pool, profile, &owner, &picks, &mut scenarios).await?;

    let bundle = Bundle {
        _meta: BundleMeta {
            synthetic: false,
            captured_from: "capture_atlas_fixtures against a live database".to_string(),
            note: "RAW capture — real titles/handles/ids. Gitignored. Run \
                   scripts/sanitize-atlas-fixtures.mjs to produce the committed bundle."
                .to_string(),
        },
        scenarios,
    };

    let json = serde_json::to_string_pretty(&bundle).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&args.out, json + "\n").map_err(|e| format!("write {}: {e}", args.out))?;
    eprintln!("wrote {} scenarios → {}", bundle.scenarios.len(), args.out);
    for name in bundle.scenarios.keys() {
        eprintln!("  · {name}");
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("capture_atlas_fixtures: {e}");
        std::process::exit(1);
    }
}
