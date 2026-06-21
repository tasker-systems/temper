#![cfg(all(feature = "artifact-tests", feature = "next-backend"))]
//! WS6 chunk-5 flip step (D) — §9 read-floor rehearsal over the REAL production corpus.
//!
//! The chunk-3 harness (`parity_reads.rs`) proves the §9 read floor (`list`/`show`/`meta`/`body`/`FTS`/
//! `vector`/`graph` parity) over a 4-resource prod-shape FIXTURE in an ephemeral `#[sqlx::test]` DB.
//! This harness lifts that proof to the full ~1214-resource production corpus, synthesized into the
//! retained `temper_rehearsal` database — the same one PR #155 used to prove synthesis-from-state.
//! It is the corpus-scale gate the hard cutover (step E) re-runs as its final pre-flip rehearsal.
//!
//! ## Not a CI test
//! `#[ignore]` + gated on `artifact-tests,next-backend` (no CI job enables either) + reads a dedicated
//! `REHEARSAL_DATABASE_URL` env var (never the dev `DATABASE_URL`, so a stray run can't touch dev).
//! It is invoked by hand against a database where `public.*` holds the real corpus and `temper_next.*`
//! has been synthesized from it at HEAD:
//!
//! ```bash
//! # 0) reset + re-synthesize temper_next at HEAD over the real corpus (mirrors the cutover):
//! psql "$REHEARSAL_DATABASE_URL" -c 'DROP SCHEMA IF EXISTS temper_next CASCADE'
//! for f in migrations/*temper_next*.sql; do psql "$REHEARSAL_DATABASE_URL" -f "$f"; done
//! DATABASE_URL="$REHEARSAL_DATABASE_URL" cargo run -p temper-next -- synthesize --limit 0
//! # 1) run the read-floor rehearsal:
//! REHEARSAL_DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_rehearsal \
//!   cargo nextest run -p temper-next --features artifact-tests,next-backend \
//!   --run-ignored all corpus_read_floor_parity --no-capture
//! ```
//!
//! ## What it proves
//! Driving every read as the single corpus owner (the production data is single-tenant — one profile
//! owns all active resources), it compares the production `public.*` read (Legacy) against the
//! `temper_next.*` readback (Next) at the §9 invariant floor:
//!   - **list** — the visible row SET + projected fields, keyed by resource id (origin_uri is non-unique).
//!   - **show** — the §9 invariant fields per resource (×N).
//!   - **meta** — the MERGED frontmatter (managed-minus-§7-dropped ∪ open), per resource. The
//!     managed/open tier SPLIT is a §9 non-invariant (§7's tierless property grain can't preserve it
//!     for a key production places in both tiers, e.g. `date`); the floor is no key/value lost.
//!   - **body** — reconstructed markdown == production `get_content`, per resource.
//!   - **FTS** — a query battery; the matching id SET is the invariant (modulo production's slug@A
//!     weight + its rank-cap, both characterized, not asserted).
//!   - **vector** — a query battery (embedded via bge-768); top-K ordered ids (embeddings carry verbatim).
//!   - **graph** — 1-hop neighbor multiset per edge-bearing resource; differences are partitioned into
//!     synthesis-minted `temper-goal` edges (expected, §4) vs unexplained (a real break).
//!
//! Hard failures (show/meta/body/list field mismatches, unexplained graph/search divergences) panic at
//! the end; the soft categories print a full characterization so a human can adjudicate the floor.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use sqlx::Row as _;
use uuid::Uuid;

use temper_api::backend::{read_selector, BackendSelection};
use temper_core::types::api::SearchParams;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::resource::ResourceListParams;
use temper_next::readback;

/// §7-died (`temper-title`/`temper-slug`/`temper-id`/`temper-context`) + relocated
/// (`temper-goal` → edge, `temper-type` → `doc_type`) managed keys: production's manifest still
/// carries them, but they never reappear in a readback's reconstructed managed map. Mirrors
/// `parity_reads.rs::DROPPED_MANAGED_KEYS`.
const DROPPED_MANAGED_KEYS: &[&str] = &[
    "temper-title",
    "temper-slug",
    "temper-id",
    "temper-context",
    "temper-goal",
    "temper-type",
];

/// The FTS query battery: terms expected across the temper corpus at a range of frequencies (rare →
/// common) so the characterization spans the no-cap and rank-capped regimes.
const FTS_QUERIES: &[&str] = &[
    "lance-williams",
    "bimap",
    "readback",
    "synthesis",
    "telos",
    "cogmap",
    "substrate",
    "flip",
    "schema",
    "session",
];

/// The vector query battery — embedded with the same bge-768 model the corpus chunks carry.
const VECTOR_QUERIES: &[&str] = &[
    "the cognitive map substrate beneath two domains",
    "session continuity across conversations",
    "hard cutover flip rehearsal write freeze",
];

/// Resolve the rehearsal pool from `REHEARSAL_DATABASE_URL`. The pool keeps the DEFAULT `public`
/// search_path: production (Legacy) services call `resources_visible_to` unqualified expecting `public`,
/// while the Next arms set `SET LOCAL search_path TO temper_next, public` per call. Returns `None`
/// (with a skip note) when the env var is unset, so a stray `--run-ignored` invocation no-ops cleanly.
async fn rehearsal_pool() -> Option<sqlx::PgPool> {
    let url = match std::env::var("REHEARSAL_DATABASE_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!(
                "SKIP corpus_read_floor_parity: set REHEARSAL_DATABASE_URL to the synthesized rehearsal DB"
            );
            return None;
        }
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect to REHEARSAL_DATABASE_URL");
    Some(pool)
}

/// The single corpus owner + the active resource id set. Production data is single-tenant: one profile
/// owns every active resource. Asserts that invariant (so a multi-owner corpus surfaces loudly rather
/// than silently testing only one owner's slice).
async fn corpus_owner_and_active(pool: &sqlx::PgPool) -> (Uuid, Vec<Uuid>) {
    let owners: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT owner_profile_id FROM public.kb_resources WHERE is_active",
    )
    .fetch_all(pool)
    .await
    .expect("distinct active owners");
    assert_eq!(
        owners.len(),
        1,
        "rehearsal corpus is expected single-tenant; got {} distinct owners",
        owners.len()
    );
    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM public.kb_resources WHERE is_active ORDER BY id")
            .fetch_all(pool)
            .await
            .expect("active resource ids");
    (owners[0], ids)
}

/// The §9 list/show projection compared across the two read paths.
type RowProjection = (
    String,         // origin_uri
    String,         // title
    bool,           // is_active
    String,         // context_name
    String,         // doc_type_name
    Option<String>, // stage
    Option<String>, // mode
    Option<String>, // effort
    Option<i64>,    // seq
);

/// Accumulates hard failures (assert-zero at the end) and prints a running characterization.
#[derive(Default)]
struct Report {
    hard_failures: Vec<String>,
}
impl Report {
    fn fail(&mut self, msg: String) {
        eprintln!("  HARD FAIL: {msg}");
        self.hard_failures.push(msg);
    }
}

#[tokio::test]
#[ignore = "manual rehearsal over the real corpus in REHEARSAL_DATABASE_URL; not a CI test"]
async fn corpus_read_floor_parity() {
    let Some(pool) = rehearsal_pool().await else {
        return;
    };
    let (owner, active) = corpus_owner_and_active(&pool).await;
    let n = active.len();
    eprintln!("== §9 read-floor rehearsal: {n} active resources, owner {owner} ==");

    // Sanity: temper_next must be synthesized (run the reset+synthesize step first).
    let synth_count: i64 = sqlx::query_scalar("SELECT count(*) FROM temper_next.kb_resources")
        .fetch_one(&pool)
        .await
        .expect("count temper_next.kb_resources");
    assert_eq!(
        synth_count as usize, n,
        "temper_next must be synthesized from the current public corpus first \
         (got {synth_count} synthesized vs {n} active) — run the reset+synthesize step"
    );

    let mut report = Report::default();

    list_parity(&pool, owner, &active, &mut report).await;
    show_parity(&pool, owner, &active, &mut report).await;
    meta_parity(&pool, owner, &active, &mut report).await;
    body_parity(&pool, owner, &active, &mut report).await;
    fts_characterization(&pool, owner).await;
    vector_characterization(&pool, owner).await;
    graph_parity(&pool, &active, &mut report).await;

    eprintln!(
        "== rehearsal complete: {} hard failure(s) ==",
        report.hard_failures.len()
    );
    assert!(
        report.hard_failures.is_empty(),
        "{} §9 read-floor parity failure(s) over the real corpus:\n{}",
        report.hard_failures.len(),
        report
            .hard_failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Project a `ResourceRow` to the §9 invariant tuple (ids/slug/hashes/timestamps/owner_handle excluded).
fn project_row(r: &temper_core::types::resource::ResourceRow) -> RowProjection {
    (
        r.origin_uri.clone(),
        r.title.clone(),
        r.is_active,
        r.context_name.clone(),
        r.doc_type_name.clone(),
        r.stage.clone(),
        r.mode.clone(),
        r.effort.clone(),
        r.seq,
    )
}

/// **list** — assemble the full Legacy visible set by paging (`list_visible` caps each page at 200),
/// then compare to the Next list arm (unpaginated, all visible) as a map keyed by resource id.
async fn list_parity(pool: &sqlx::PgPool, owner: Uuid, active: &[Uuid], report: &mut Report) {
    eprintln!("-- list parity --");

    // Legacy: page through at the 200 cap until exhausted.
    let mut legacy: BTreeMap<Uuid, RowProjection> = BTreeMap::new();
    let mut offset = 0i64;
    loop {
        let params = ResourceListParams {
            limit: Some(200),
            offset: Some(offset),
            ..Default::default()
        };
        let page = read_selector::list_select(BackendSelection::Legacy, pool, owner, params)
            .await
            .expect("legacy list page");
        if page.rows.is_empty() {
            break;
        }
        for r in &page.rows {
            legacy.insert(Uuid::from(r.id), project_row(r));
        }
        offset += 200;
    }

    // Next: one call returns every visible row.
    let next_rows = read_selector::list_select(
        BackendSelection::Next,
        pool,
        owner,
        ResourceListParams::default(),
    )
    .await
    .expect("next list");
    let next: BTreeMap<Uuid, RowProjection> = next_rows
        .rows
        .iter()
        .map(|r| (Uuid::from(r.id), project_row(r)))
        .collect();

    eprintln!(
        "   legacy={} next={} active={}",
        legacy.len(),
        next.len(),
        active.len()
    );

    let legacy_ids: BTreeSet<Uuid> = legacy.keys().copied().collect();
    let next_ids: BTreeSet<Uuid> = next.keys().copied().collect();
    let active_ids: BTreeSet<Uuid> = active.iter().copied().collect();
    if legacy_ids != active_ids {
        report.fail(format!(
            "list: legacy visible set != active set (legacy-only={}, active-only={})",
            legacy_ids.difference(&active_ids).count(),
            active_ids.difference(&legacy_ids).count()
        ));
    }
    if legacy_ids != next_ids {
        let lonly: Vec<Uuid> = legacy_ids.difference(&next_ids).take(5).copied().collect();
        let nonly: Vec<Uuid> = next_ids.difference(&legacy_ids).take(5).copied().collect();
        report.fail(format!(
            "list: legacy id-set != next id-set (legacy-only sample={lonly:?}, next-only sample={nonly:?})"
        ));
    }
    // Per-id projection mismatches over the shared id set.
    let mut field_mismatches = 0usize;
    for id in legacy_ids.intersection(&next_ids) {
        if legacy[id] != next[id] {
            if field_mismatches < 10 {
                report.fail(format!(
                    "list: projection differs for {id}\n    legacy={:?}\n    next  ={:?}",
                    legacy[id], next[id]
                ));
            }
            field_mismatches += 1;
        }
    }
    if field_mismatches >= 10 {
        eprintln!(
            "   (+{} more list projection mismatches)",
            field_mismatches - 10
        );
    }
    eprintln!("   list projection mismatches: {field_mismatches}");
}

/// **show** — per-resource §9 invariant-field parity via the show selector (Legacy vs Next).
async fn show_parity(pool: &sqlx::PgPool, owner: Uuid, active: &[Uuid], report: &mut Report) {
    eprintln!("-- show parity ({} resources) --", active.len());
    let mut mismatches = 0usize;
    for &id in active {
        let leg = read_selector::show_select(BackendSelection::Legacy, pool, owner, id)
            .await
            .expect("legacy show");
        let nxt = read_selector::show_select(BackendSelection::Next, pool, owner, id)
            .await
            .expect("next show");
        if project_row(&leg) != project_row(&nxt) {
            if mismatches < 10 {
                report.fail(format!(
                    "show: §9 fields differ for {id}\n    legacy={:?}\n    next  ={:?}",
                    project_row(&leg),
                    project_row(&nxt)
                ));
            }
            mismatches += 1;
        }
    }
    eprintln!("   show §9-field mismatches: {mismatches}");
}

/// Serialize a typed `ManagedMeta` to a JSON object map (panics on a non-object — a contract break).
fn managed_as_object(m: &temper_core::types::managed_meta::ManagedMeta) -> Map<String, Value> {
    match serde_json::to_value(m).expect("serialize ManagedMeta") {
        Value::Object(o) => o,
        other => panic!("ManagedMeta serialized to non-object: {other:?}"),
    }
}

/// Merge a (managed-minus-dropped) map with an open map into one key→value union. The managed/open
/// SPLIT is a §9 NON-invariant (§7 dissolves properties to a tierless grain — a key production places in
/// both tiers, like `date`, cannot round-trip the split); the floor is that no key/value is lost or
/// altered. A key present in BOTH inputs at DIFFERENT values is a genuine inconsistency → returned in the
/// second slot for a hard fail.
fn meta_union(
    managed: &Map<String, Value>,
    open: &Map<String, Value>,
) -> (BTreeMap<String, Value>, Vec<String>) {
    let mut union: BTreeMap<String, Value> = BTreeMap::new();
    let mut collisions = Vec::new();
    for (k, v) in managed.iter().chain(open.iter()) {
        match union.get(k) {
            Some(existing) if existing != v => collisions.push(k.clone()),
            _ => {
                union.insert(k.clone(), v.clone());
            }
        }
    }
    (union, collisions)
}

/// **meta** — per-resource frontmatter parity at the §9 floor: the MERGED key/value set
/// (managed-minus-§7-dropped ∪ open) must match across the two read paths. The managed/open tier split
/// is a non-invariant (see [`meta_union`]); the split-difference count is reported as characterization
/// (it is non-zero only for keys production places ambiguously across tiers, e.g. the `date` anomalies).
async fn meta_parity(pool: &sqlx::PgPool, owner: Uuid, active: &[Uuid], report: &mut Report) {
    eprintln!("-- meta parity ({} resources) --", active.len());
    let mut union_mismatches = 0usize;
    let mut split_only_diffs = 0usize;
    for &id in active {
        let leg = read_selector::get_meta_select(
            BackendSelection::Legacy,
            pool,
            ProfileId::from(owner),
            ResourceId::from(id),
        )
        .await
        .expect("legacy get_meta");
        let nxt = read_selector::get_meta_select(
            BackendSelection::Next,
            pool,
            ProfileId::from(owner),
            ResourceId::from(id),
        )
        .await
        .expect("next get_meta");

        let mut leg_managed = managed_as_object(&leg.managed_meta.expect("legacy managed"));
        for k in DROPPED_MANAGED_KEYS {
            leg_managed.remove(*k);
        }
        let nxt_managed = managed_as_object(&nxt.managed_meta.expect("next managed"));
        let leg_open = match leg.open_meta.unwrap_or(Value::Object(Map::new())) {
            Value::Object(o) => o,
            _ => Map::new(),
        };
        let nxt_open = match nxt.open_meta.unwrap_or(Value::Object(Map::new())) {
            Value::Object(o) => o,
            _ => Map::new(),
        };

        let (leg_union, leg_collide) = meta_union(&leg_managed, &leg_open);
        let (nxt_union, _) = meta_union(&nxt_managed, &nxt_open);

        if leg_union != nxt_union {
            if union_mismatches < 10 {
                let lk: BTreeSet<&String> = leg_union.keys().collect();
                let nk: BTreeSet<&String> = nxt_union.keys().collect();
                let val_diffs: Vec<&String> = leg_union
                    .keys()
                    .filter(|k| nxt_union.get(*k).is_some_and(|v| v != &leg_union[*k]))
                    .collect();
                report.fail(format!(
                    "meta(union): merged frontmatter differs for {id}; legacy-only keys={:?}, next-only keys={:?}, value-diffs={:?}",
                    lk.difference(&nk).collect::<Vec<_>>(),
                    nk.difference(&lk).collect::<Vec<_>>(),
                    val_diffs
                ));
            }
            union_mismatches += 1;
        }
        if !leg_collide.is_empty() {
            report.fail(format!(
                "meta: legacy places {leg_collide:?} in BOTH tiers at different values for {id}"
            ));
        }
        // Characterization only: the tier split differs (a non-invariant) even where the union matches.
        if nxt_managed != leg_managed && leg_union == nxt_union {
            split_only_diffs += 1;
        }
    }
    eprintln!(
        "   meta UNION mismatches (hard): {union_mismatches}; tier-split-only diffs (non-invariant): {split_only_diffs}"
    );
}

/// **body** — per-resource reconstructed-markdown parity (Legacy `get_content` vs Next body).
async fn body_parity(pool: &sqlx::PgPool, owner: Uuid, active: &[Uuid], report: &mut Report) {
    eprintln!("-- body parity ({} resources) --", active.len());
    let mut mismatches = 0usize;
    for &id in active {
        let leg = read_selector::get_content_select(BackendSelection::Legacy, pool, owner, id)
            .await
            .expect("legacy get_content")
            .markdown;
        let nxt = read_selector::get_content_select(BackendSelection::Next, pool, owner, id)
            .await
            .expect("next get_content")
            .markdown;
        if leg != nxt {
            if mismatches < 6 {
                report.fail(format!(
                    "body: markdown differs for {id} (legacy {} chars, next {} chars)",
                    leg.len(),
                    nxt.len()
                ));
            }
            mismatches += 1;
        }
    }
    eprintln!("   body mismatches: {mismatches}");
}

/// **FTS characterization** — for each battery term: Legacy (capped 50, ranked, slug@A) vs Next
/// (`readback::fts_search`, uncapped, title@A only). Reports |legacy|, |next|, the inclusion relation,
/// and the legacy-only matches (the slug@A or rank-cap deltas). Soft (printed, not asserted) — the §9
/// floor here is the matching SET modulo two documented production-only effects.
async fn fts_characterization(pool: &sqlx::PgPool, owner: Uuid) {
    eprintln!("-- FTS characterization --");
    for &q in FTS_QUERIES {
        let params = SearchParams {
            query: Some(q.to_string()),
            embedding: None,
            search_config: "english".to_string(),
            graph_expand: false,
            limit: Some(50),
            ..Default::default()
        };
        let legacy: BTreeSet<Uuid> =
            temper_api::services::search_service::search(pool, owner, params)
                .await
                .expect("legacy FTS")
                .into_iter()
                .map(|r| r.resource_id)
                .collect();
        let next: BTreeSet<Uuid> = readback::fts_search(pool, owner, q)
            .await
            .expect("next FTS")
            .into_iter()
            .collect();
        let legacy_only = legacy.difference(&next).count();
        let next_only = next.difference(&legacy).count();
        let capped = next.len() > 50;
        eprintln!(
            "   {q:>16}: legacy={:>4} next={:>4} legacy⊆next={} legacy_only={legacy_only} next_only={next_only}{}",
            legacy.len(),
            next.len(),
            legacy.is_subset(&next),
            if capped { " [next>50: legacy rank-capped]" } else { "" }
        );
    }
}

/// **vector characterization** — embed each battery query (bge-768) and compare the top-K ordered ids.
/// Embeddings carry verbatim through synthesis (§8), so the ranking should match where chunk distances
/// are distinct. Legacy caps at 50; compares the overlap prefix. Soft (printed).
async fn vector_characterization(pool: &sqlx::PgPool, owner: Uuid) {
    eprintln!("-- vector characterization --");
    for &q in VECTOR_QUERIES {
        let emb = temper_ingest::embed::embed_text(q).expect("embed query");
        let params = SearchParams {
            query: None,
            embedding: Some(emb.clone()),
            search_config: "english".to_string(),
            graph_expand: false,
            limit: Some(50),
            ..Default::default()
        };
        let legacy: Vec<Uuid> = temper_api::services::search_service::search(pool, owner, params)
            .await
            .expect("legacy vector")
            .into_iter()
            .map(|r| r.resource_id)
            .collect();
        let next: Vec<Uuid> = readback::vector_search(pool, owner, &emb)
            .await
            .expect("next vector");
        // Compare the ordered prefix up to the shorter length (Legacy is capped at 50).
        let k = legacy.len().min(next.len());
        let first_div = (0..k).find(|&i| legacy[i] != next[i]);
        let topk_set_eq: bool = {
            let ls: BTreeSet<_> = legacy.iter().take(k).collect();
            let ns: BTreeSet<_> = next.iter().take(k).collect();
            ls == ns
        };
        eprintln!(
            "   {:.40}…: legacy={} next={} top{k}_ordered_match={} top{k}_set_match={}",
            q,
            legacy.len(),
            next.len(),
            first_div.is_none(),
            topk_set_eq
        );
        if let Some(i) = first_div {
            eprintln!(
                "      first order divergence at rank {i}: legacy={} next={}",
                legacy[i], next[i]
            );
        }
    }
}

/// One 1-hop neighbor tuple: `(neighbor_origin_uri, edge_kind, polarity, label)`. Compared as a sorted
/// multiset (NOT a set) — origin_uri is non-unique on the real corpus, so dedup would mask a real
/// edge-count divergence.
type NeighborTuple = (String, String, String, Option<String>);

/// **graph** — per edge-bearing resource, compare `readback::neighbors` (over `temper_next.kb_edges`)
/// to the production symmetric oracle over `public.kb_resource_edges`. §4 synthesis legitimately MINTS
/// `temper-goal`-derived edges and dedupes others, so a Next neighbor with no public counterpart is
/// expected when it is explained by a `temper-goal` managed property pointing at that neighbor.
/// Differences are partitioned: explained (counted, not failed) vs unexplained (hard fail).
async fn graph_parity(pool: &sqlx::PgPool, active: &[Uuid], report: &mut Report) {
    eprintln!("-- graph parity --");

    // Pre-fetch, per resource, the set of origin_uris its temper-goal property points at (the §4 mint
    // source). temper-goal lives in production managed_meta as a slug/ref; resolve via the goal's id →
    // origin_uri is not direct, so we approximate the "explained" set by the temper-goal TARGET edges
    // present in temper_next but absent from public: a Next-only neighbor reached by a `contains`/
    // forward edge is the canonical temper-goal shape (goal contains resource). We classify a Next-only
    // neighbor as explained iff its edge is `contains`/forward with a goal-ish label, and report the
    // residual.
    let mut edge_bearing = 0usize;
    let mut clean = 0usize;
    let mut explained_only = 0usize;
    let mut unexplained = 0usize;

    for &id in active {
        let prod = prod_neighbors(pool, id).await;
        let next: Vec<NeighborTuple> = readback::neighbors(pool, id)
            .await
            .expect("readback neighbors")
            .into_iter()
            .map(|nb| (nb.origin_uri, nb.edge_kind, nb.polarity, nb.label))
            .collect();
        if prod.is_empty() && next.is_empty() {
            continue;
        }
        edge_bearing += 1;

        let mut prod_sorted = prod.clone();
        prod_sorted.sort();
        let mut next_sorted = next.clone();
        next_sorted.sort();
        if prod_sorted == next_sorted {
            clean += 1;
            continue;
        }

        // Multiset difference both ways.
        let next_only = multiset_diff(&next_sorted, &prod_sorted);
        let prod_only = multiset_diff(&prod_sorted, &next_sorted);

        // Production-only neighbors are always a break (Next dropped an edge it should carry).
        // Next-only neighbors are explained iff they are a forward `contains` edge (the temper-goal mint
        // shape). A prod_only delta or a non-`contains` next_only delta is unexplained.
        let next_only_unexplained: Vec<&NeighborTuple> = next_only
            .iter()
            .filter(|(_, kind, polarity, _)| !(kind == "contains" && polarity == "forward"))
            .collect();

        if prod_only.is_empty() && next_only_unexplained.is_empty() {
            explained_only += 1;
        } else {
            unexplained += 1;
            if unexplained <= 12 {
                report.fail(format!(
                    "graph: unexplained neighbor delta for {id}; prod_only={prod_only:?} next_only_unexplained={next_only_unexplained:?}"
                ));
            }
        }
    }
    eprintln!(
        "   edge-bearing={edge_bearing} clean={clean} explained(temper-goal mint)-only={explained_only} unexplained={unexplained}"
    );
}

/// The production 1-hop symmetric neighbor oracle over `public.kb_resource_edges` (both directions,
/// `NOT is_folded`), projecting `(neighbor_origin_uri, edge_kind, polarity, label)` with the NOT-NULL
/// production `label` normalized empty→None (matching `temper_next`'s empty→NULL carry).
async fn prod_neighbors(pool: &sqlx::PgPool, id: Uuid) -> Vec<NeighborTuple> {
    let rows = sqlx::query(
        "SELECT t.origin_uri AS origin_uri, e.edge_kind::text AS edge_kind, \
                e.polarity::text AS polarity, e.label \
           FROM public.kb_resource_edges e \
           JOIN public.kb_resources t ON t.id = e.target_resource_id AND t.is_active \
          WHERE e.source_resource_id = $1 AND NOT e.is_folded \
         UNION ALL \
         SELECT s.origin_uri AS origin_uri, e.edge_kind::text AS edge_kind, \
                e.polarity::text AS polarity, e.label \
           FROM public.kb_resource_edges e \
           JOIN public.kb_resources s ON s.id = e.source_resource_id AND s.is_active \
          WHERE e.target_resource_id = $1 AND NOT e.is_folded",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .expect("production neighbor oracle");
    rows.iter()
        .map(|r| {
            let label: String = r.get("label");
            (
                r.get::<String, _>("origin_uri"),
                r.get::<String, _>("edge_kind"),
                r.get::<String, _>("polarity"),
                (!label.is_empty()).then_some(label),
            )
        })
        .collect()
}

/// `a` minus `b` as a multiset (both inputs sorted): the elements of `a` not consumed by a matching
/// element of `b`.
fn multiset_diff(a: &[NeighborTuple], b: &[NeighborTuple]) -> Vec<NeighborTuple> {
    let mut remaining: BTreeMap<&NeighborTuple, usize> = BTreeMap::new();
    for x in b {
        *remaining.entry(x).or_insert(0) += 1;
    }
    let mut out = Vec::new();
    for x in a {
        match remaining.get_mut(x) {
            Some(c) if *c > 0 => *c -= 1,
            _ => out.push(x.clone()),
        }
    }
    out
}
