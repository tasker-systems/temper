//! T6 grounding benchmark: what does forming a CONTEXT's regions actually cost, and where does the
//! time go?
//!
//! The task's open question was *"is the re-cluster alone expensive enough to need a finer invalidation
//! grain?"*, and the pre-existing `cluster::agglomerate_benchmark_large_component` cannot answer it, by
//! construction: it times the clustering core over **a precomputed-matrix affinity closure**, "so we
//! measure clustering, not the affinity scan." In the context regime the affinity scan IS the cost.
//! This bench restores it — it runs the real `affinity` closure, which linear-scans the edge slice on
//! every call.
//!
//! Dimensions are the LIVE `@me/temper` context, read from prod (2026-07-12):
//!   n = 1071 resources, E = 570 declared edges, F = 0 facets (every context has zero facets today).
//!
//! What it found, and what the answer turned out to be: the affinity relation is **1.5% dense**, and
//! formation was spending ~1.15M `affinity` calls to discover ~8.5k nonzero pairs — because
//! `connected_components` and `agglomerate` each scanned all n²/2 pairs to find out which were nonzero.
//! Those pairs are knowable a priori ([`temper_substrate::affinity::candidate_pairs`]), so the answer
//! is NOT a finer invalidation grain — it is to stop enumerating pairs that cannot be nonzero. This
//! bench measures both paths side by side and prints the speedup.
//!
//! Run with:
//!   cargo nextest run -p temper-substrate context_formation_cost --run-ignored all --no-capture

use std::cell::Cell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use temper_substrate::affinity::{affinity, candidate_pairs, Edge, EdgeKind, Facet, Lens};
use temper_substrate::cluster::{agglomerate, connected_components, CandidatePairs};
use temper_substrate::ids::ResourceId;
use temper_substrate::knn;
use uuid::Uuid;

/// Live prod dimensions of the `@me/temper` context.
const N_NODES: usize = 1071;
const N_EDGES: usize = 570;
const EMBED_DIM: usize = 768;
/// Latent topics the synthetic corpus is drawn from — enough that within-topic cosine clears the
/// lens's 0.55 floor and cross-topic cosine does not, which is what makes the kNN graph SPARSE (the
/// property the whole kernel leans on).
const N_TOPICS: usize = 40;

/// xorshift64* — deterministic, no dev-dependency.
struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in [-1, 1).
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 11) as f32 / (1u64 << 53) as f32 * 2.0 - 1.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

fn rid(n: usize) -> ResourceId {
    ResourceId::from(Uuid::from_u128(n as u128))
}

/// A corpus with real topical structure: each node is a latent topic vector plus noise, so
/// within-topic pairs land above the lens floor and cross-topic pairs below it. A corpus of pure
/// noise would clear NO pairs and hand the bench an empty kNN graph — i.e. it would silently measure
/// the cogmap regime, not the context one, and the whole benchmark would be a lie.
fn corpus(rng: &mut Rng) -> HashMap<ResourceId, Vec<f32>> {
    let topics: Vec<Vec<f32>> = (0..N_TOPICS)
        .map(|_| (0..EMBED_DIM).map(|_| rng.unit()).collect())
        .collect();

    (0..N_NODES)
        .map(|i| {
            let t = &topics[i % N_TOPICS];
            // 0.9 topic + 0.1 noise ⇒ within-topic cosine well above the 0.55 floor, cross-topic ≈ 0.
            let v = t
                .iter()
                .map(|c| c * 0.9 + rng.unit() * 0.1)
                .collect::<Vec<f32>>();
            (rid(i + 1), v)
        })
        .collect()
}

fn edges(rng: &mut Rng) -> Vec<Edge> {
    let kinds = [
        EdgeKind::LeadsTo,
        EdgeKind::Near,
        EdgeKind::Express,
        EdgeKind::Contains,
    ];
    (0..N_EDGES)
        .map(|i| Edge {
            src: rid(rng.below(N_NODES) + 1),
            tgt: rid(rng.below(N_NODES) + 1),
            kind: kinds[i % kinds.len()],
            weight: 1.0,
            label: None,
        })
        .collect()
}

#[test]
#[ignore = "benchmark: run explicitly with --run-ignored all --no-capture"]
fn context_formation_cost_at_live_prod_dimensions() {
    let mut rng = Rng(0x5EED_1234_5678_9ABC);
    let embeddings = corpus(&mut rng);
    let edges = edges(&mut rng);
    let facets: Vec<Facet> = Vec::new(); // every live context has zero facets
    let lens = Lens::workflow_default(); // the context regime: w_cos = 1.0, k = 12, floor = 0.55

    // ---- the kNN build, inside substrate::load, before the producer runs at all. O(n²·768) exact
    // cosine — this is the one genuinely irreducible O(n²) term, and after the fix below it dominates.
    let t = Instant::now();
    let g = knn::build(&embeddings, lens.knn_k, lens.cos_floor);
    let t_knn = t.elapsed();

    let node_ids: Vec<ResourceId> = (0..N_NODES).map(|i| rid(i + 1)).collect();
    let nodes: Vec<Uuid> = node_ids.iter().map(|r| r.uuid()).collect();

    // The REAL production closure — a full linear scan of `edges` per call, exactly as `write.rs`
    // builds it. Counting calls is the point: the pre-existing bench's precomputed matrix hides them.
    let calls = Cell::new(0usize);
    let aff = |x: Uuid, y: Uuid| {
        calls.set(calls.get() + 1);
        affinity(x.into(), y.into(), &edges, &facets, &g, &lens)
    };

    // Run one full `cluster_components`-equivalent pass over a given candidate set, returning
    // (elapsed, affinity calls, components, regions).
    let pass = |cands: &CandidatePairs| {
        calls.set(0);
        let t = Instant::now();
        let comps = connected_components(&nodes, cands, &aff);
        let regions: Vec<_> = comps
            .iter()
            .flat_map(|c| agglomerate(c, cands, &aff, lens.resolution))
            .collect();
        (t.elapsed(), calls.get(), comps, regions)
    };

    // ---- the OLD path: every pair evaluated, twice (once to find components, once to precompute the
    // agglomerator's affinity map).
    let dense = CandidatePairs::dense(&nodes);
    let (t_dense, calls_dense, comps_dense, regions_dense) = pass(&dense);

    // ---- the NEW path: only the pairs that CAN be nonzero — declared edges ∪ retained kNN
    // neighbours ∪ facet-sharing pairs.
    let t = Instant::now();
    let sparse = candidate_pairs(&node_ids, &edges, &facets, &g);
    let t_enumerate = t.elapsed();
    let (t_sparse, calls_sparse, comps_sparse, regions_sparse) = pass(&sparse);

    // How sparse is the relation the dense path spent n² calls to discover?
    calls.set(0);
    let mut nonzero = 0usize;
    for i in 0..nodes.len() {
        for j in (i + 1)..nodes.len() {
            if aff(nodes[i], nodes[j]) != 0.0 {
                nonzero += 1;
            }
        }
    }
    let pairs = calls.get();

    let pct = |x: usize| 100.0 * x as f64 / pairs as f64;
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!("\n=== context formation cost @ live prod dimensions ===");
    println!(
        "  n={N_NODES} nodes, E={N_EDGES} edges, F=0 facets, k={}, floor={}",
        lens.knn_k, lens.cos_floor
    );
    println!("  pairs (n²/2)             {pairs}");
    println!(
        "  NONZERO affinity pairs   {nonzero}  ({:.2}% of pairs)",
        pct(nonzero)
    );
    println!(
        "  candidate pairs offered  {}  ({:.2}% of pairs)",
        sparse.len(),
        pct(sparse.len())
    );
    println!("  components               {}", comps_dense.len());
    println!("  regions formed           {}", regions_dense.len());
    println!("  ---");
    println!(
        "  knn::build               {:>8.1}ms   (O(n²·768) — irreducible here)",
        ms(t_knn)
    );
    println!(
        "  cluster [dense]          {:>8.1}ms   ({calls_dense} affinity calls)",
        ms(t_dense)
    );
    println!(
        "  cluster [sparse]         {:>8.1}ms   ({calls_sparse} affinity calls, +{:.1}ms to enumerate)",
        ms(t_sparse),
        ms(t_enumerate)
    );
    println!("  ---");
    println!("  formation TOTAL  before  {:>8.1}ms", ms(t_knn + t_dense));
    println!(
        "  formation TOTAL  after   {:>8.1}ms   ({:.1}x on the clustering, {:.1}x end-to-end)",
        ms(t_knn + t_enumerate + t_sparse),
        ms(t_dense) / ms(t_sparse + t_enumerate).max(1e-9),
        ms(t_knn + t_dense) / ms(t_knn + t_enumerate + t_sparse).max(1e-9)
    );
    println!();

    // The differential guarantee, at prod scale: the sparse path is not an approximation. (The
    // property-based version over many random substrates and both regimes lives in
    // `sparse_candidates_differential.rs`; this pins it on the shape we actually ship against.)
    assert_eq!(
        comps_sparse, comps_dense,
        "sparse candidates must yield identical components"
    );
    assert_eq!(
        regions_sparse, regions_dense,
        "sparse candidates must yield an identical partition"
    );

    // Structural claims T6's design rests on — these, not the timings, are what gate CI if someone
    // makes the affinity relation dense or the graph multi-component, either of which would
    // invalidate the design this bench justifies.
    assert_eq!(
        comps_dense.len(),
        1,
        "a context is ONE component (T4's finding)"
    );
    assert!(
        nonzero * 20 < pairs,
        "the affinity relation must be SPARSE — {nonzero} nonzero of {pairs} pairs"
    );
    assert!(
        sparse.len() >= nonzero,
        "candidate pairs MUST be a superset of the nonzero pairs — {} offered, {nonzero} nonzero",
        sparse.len()
    );
    assert!(
        calls_sparse * 10 < calls_dense,
        "the sparse path must evaluate an order of magnitude fewer pairs — {calls_sparse} vs {calls_dense}"
    );
}
