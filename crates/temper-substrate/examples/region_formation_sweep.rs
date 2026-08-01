//! Does the cogmap regime's lens actually cause its singleton regions? Sweep it and see.
//!
//! **The question.** On prod, the `Temper — self-cognition` map holds 385 live regions of which 239
//! (62%) are singletons — and 224 of those 239 sit *inside* structured connected components, so they
//! are leaves that had neighbours and never merged, not isolated nodes. The hypothesis is that
//! `telos-default`'s edge weights sit under the merge threshold: `resolution` is 0.5 for every lens,
//! but the map's two commonest edge kinds are weighted `w_leads_to` 0.6 and `w_near` 0.3, while the
//! context lens gives `leads_to` 0.9 *and* adds a kNN cosine term (`w_cos` 1.0).
//!
//! **This runs the SHIPPED algorithm, not a model of it.** `substrate::load` → `candidate_pairs` →
//! `connected_components` → `agglomerate` is exactly the sequence `write.rs::cluster_components`
//! performs; the body below is that function's body with the lens varied between runs.
//!
//! **Read-only.** `substrate::load` and `load_lens` are pure SELECTs, and nothing here calls the
//! materialize/write path. Point `DATABASE_URL` at whatever you want measured.
//!
//! The BASELINE arm is the validation: run under the map's own live lens it must reproduce the
//! region shape actually stored on prod. If it does not, the sweep says nothing and the run reports
//! that rather than its numbers.
//!
//! ```bash
//! DATABASE_URL="$(neonctl connection-string main --project-id … --role-name …)" \
//!   cargo run --release -p temper-substrate --features artifact-tests \
//!   --example region_formation_sweep -- "Temper — self-cognition"
//! ```

use anyhow::Result;
use temper_core::types::home::HomeAnchor;
use temper_substrate::affinity::{affinity, candidate_pairs, Lens};
use temper_substrate::cluster::{agglomerate, connected_components};
use temper_substrate::substrate::{self, Substrate};

/// The shape a clustering run produced — what the sweep compares between arms.
struct Shape {
    components: usize,
    regions: usize,
    singletons: usize,
    largest: usize,
    mean_members: f64,
    /// Singletons that sit inside a component that produced more than one region — the leaves that
    /// had somewhere to merge and did not. The isolated-node singletons are not interesting; these
    /// are the ones the hypothesis is about.
    singletons_in_structured_components: usize,
}

fn shape_of(clusters_per_component: &[Vec<Vec<uuid::Uuid>>]) -> Shape {
    let all: Vec<&Vec<uuid::Uuid>> = clusters_per_component.iter().flatten().collect();
    let regions = all.len();
    let singletons = all.iter().filter(|c| c.len() <= 1).count();
    let total_members: usize = all.iter().map(|c| c.len()).sum();
    let singletons_in_structured_components = clusters_per_component
        .iter()
        .filter(|comp| comp.len() > 1 || comp.iter().map(Vec::len).sum::<usize>() > 1)
        .flatten()
        .filter(|c| c.len() <= 1)
        .count();
    Shape {
        components: clusters_per_component.len(),
        regions,
        singletons,
        largest: all.iter().map(|c| c.len()).max().unwrap_or(0),
        mean_members: if regions == 0 {
            0.0
        } else {
            total_members as f64 / regions as f64
        },
        singletons_in_structured_components,
    }
}

/// `write.rs::cluster_components`, with the lens supplied rather than read off the substrate.
fn cluster_with(s: &Substrate, lens: &Lens) -> Vec<Vec<Vec<uuid::Uuid>>> {
    let aff = |x: uuid::Uuid, y: uuid::Uuid| {
        affinity(x.into(), y.into(), &s.edges, &s.facets, &s.knn, lens)
    };
    let node_uuids: Vec<uuid::Uuid> = s.nodes.iter().map(|n| n.uuid()).collect();
    let candidates = candidate_pairs(&s.nodes, &s.edges, &s.facets, &s.knn);
    connected_components(&node_uuids, &candidates, &aff)
        .into_iter()
        .map(|members| agglomerate(&members, &candidates, &aff, lens.resolution))
        .collect()
}

fn report(label: &str, sh: &Shape) {
    println!(
        "{label:<38} {:>4} {:>8} {:>11} {:>10} {:>8} {:>9.2}",
        sh.components,
        sh.regions,
        sh.singletons,
        sh.singletons_in_structured_components,
        sh.largest,
        sh.mean_members
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let map_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Temper — self-cognition".to_owned());

    let pool = substrate::connect().await?;
    let cogmap_id = substrate::cogmap_by_name(&pool, &map_name).await?;
    let anchor = HomeAnchor::Cogmap(cogmap_id);

    // The map's own live lens, and the substrate under it (declared-only ⇒ no kNN graph built).
    let s = substrate::load(&pool, anchor, "telos-default").await?;
    println!(
        "map {map_name:?}: {} live nodes, {} edges, {} facets, knn pairs {}",
        s.nodes.len(),
        s.edges.len(),
        s.facets.len(),
        s.knn.pairs().len()
    );
    println!(
        "telos-default: resolution {} | leads_to {} near {} express {} contains {} prop {} cos {}",
        s.lens.resolution,
        s.lens.w_leads_to,
        s.lens.w_near,
        s.lens.w_express,
        s.lens.w_contains,
        s.lens.w_prop,
        s.lens.w_cos
    );
    println!();
    println!(
        "{:<38} {:>4} {:>8} {:>11} {:>10} {:>8} {:>9}",
        "arm", "comp", "regions", "singletons", "in-struct", "largest", "mean"
    );

    // ---- BASELINE. Must reproduce what prod actually stores, or nothing below is trustworthy.
    let base = shape_of(&cluster_with(&s, &s.lens));
    report("BASELINE telos-default (live)", &base);

    // ---- Arm 1: lower the merge threshold, weights untouched.
    for r in [0.4, 0.3, 0.25, 0.2, 0.1] {
        let mut lens = s.lens.clone();
        lens.resolution = r;
        report(
            &format!("resolution {r}"),
            &shape_of(&cluster_with(&s, &lens)),
        );
    }

    // ---- Arm 2: raise the weight on `near`, the map's second-commonest edge and the one that at
    // 0.3 cannot reach a 0.5 threshold on its own however many times it appears.
    for w in [0.5, 0.6, 0.9] {
        let mut lens = s.lens.clone();
        lens.w_near = w;
        report(&format!("w_near {w}"), &shape_of(&cluster_with(&s, &lens)));
    }

    // ---- Arm 3: the context lens's `leads_to` weight, on the map's own graph.
    for w in [0.9, 1.0] {
        let mut lens = s.lens.clone();
        lens.w_leads_to = w;
        report(
            &format!("w_leads_to {w}"),
            &shape_of(&cluster_with(&s, &lens)),
        );
    }

    // ---- Arm 4: both edge weights at the context lens's values, threshold untouched.
    {
        let mut lens = s.lens.clone();
        lens.w_leads_to = 0.9;
        lens.w_near = 0.35;
        report(
            "w_leads_to 0.9 + w_near 0.35",
            &shape_of(&cluster_with(&s, &lens)),
        );
    }

    // ---- Arm 5: the whole context regime on the map — INCLUDING the kNN term, which needs a
    // substrate loaded under a lens with `w_cos > 0` (load skips the embedding query otherwise).
    let s_cos = substrate::load(&pool, anchor, "workflow-default").await?;
    println!(
        "\nworkflow-default on the same map: knn pairs {} | resolution {} leads_to {} near {} cos {}",
        s_cos.knn.pairs().len(),
        s_cos.lens.resolution,
        s_cos.lens.w_leads_to,
        s_cos.lens.w_near,
        s_cos.lens.w_cos
    );
    println!(
        "{:<38} {:>4} {:>8} {:>11} {:>10} {:>8} {:>9}",
        "arm", "comp", "regions", "singletons", "in-struct", "largest", "mean"
    );
    report(
        "FULL context regime (workflow-default)",
        &shape_of(&cluster_with(&s_cos, &s_cos.lens)),
    );

    // Isolate the kNN term: the map's own weights and threshold, plus cosine only.
    for w in [0.5, 1.0] {
        let mut lens = s.lens.clone();
        lens.w_cos = w;
        report(
            &format!("telos-default + w_cos {w}"),
            &shape_of(&cluster_with(&s_cos, &lens)),
        );
    }

    Ok(())
}
