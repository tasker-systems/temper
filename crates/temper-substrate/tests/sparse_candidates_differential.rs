//! Differential: the sparse candidate enumeration must produce a partition **identical** to the dense
//! n²/2 scan it replaces — on every substrate, in both regimes.
//!
//! This is the whole safety argument for T6's formation speedup, and it is asserted rather than
//! reasoned about. The claim is that [`candidate_pairs`] is a SUPERSET of the nonzero-affinity pairs,
//! so feeding it to the clustering core instead of every pair changes nothing: a pair it omits has all
//! three affinity terms zero, and both consumers already discard exactly-zero pairs.
//!
//! A hand-written expectation would only reproduce the author's understanding of that argument. So
//! these tests never state what the partition *should* be — they run BOTH paths over randomized
//! substrates and assert the outputs agree. The dense path is the incumbent, shipped, scenario-corpus-
//! verified behavior; it is the oracle.
//!
//! Deliberately exercised: negative edge weights (which are nonzero, so they must be offered as
//! candidates and can cross the merge threshold via a Lance-Williams blend), terms that CANCEL to
//! exactly zero, facets (the term contexts don't have but cogmaps do), multi-component graphs, and
//! both `w_cos` regimes.

use std::collections::HashMap;

use temper_substrate::affinity::{affinity, candidate_pairs, Edge, EdgeKind, Facet, Lens};
use temper_substrate::cluster::{cluster, CandidatePairs};
use temper_substrate::ids::ResourceId;
use temper_substrate::knn::{self, KnnGraph};
use uuid::Uuid;

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
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
    /// Uniform in [-1, 1).
    fn signed(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

fn rid(n: usize) -> ResourceId {
    ResourceId::from(Uuid::from_u128(n as u128))
}

/// One randomized substrate. `dim`/`topics` shape the embeddings so a realistic (sparse, nonempty)
/// kNN graph falls out; `neg` mixes in negative edge weights.
struct Fixture {
    nodes: Vec<ResourceId>,
    edges: Vec<Edge>,
    facets: Vec<Facet>,
    knn: KnnGraph,
    lens: Lens,
}

impl Fixture {
    fn build(seed: u64, n: usize, n_edges: usize, n_facets: usize, lens: Lens) -> Self {
        let mut rng = Rng(seed);
        let nodes: Vec<ResourceId> = (1..=n).map(rid).collect();

        let kinds = [
            EdgeKind::Express,
            EdgeKind::Contains,
            EdgeKind::LeadsTo,
            EdgeKind::Near,
        ];
        let edges: Vec<Edge> = (0..n_edges)
            .map(|i| Edge {
                src: rid(rng.below(n) + 1),
                tgt: rid(rng.below(n) + 1),
                kind: kinds[i % kinds.len()],
                // signed: negative weights are nonzero, so they MUST be offered as candidates. A
                // subset-not-superset bug would silently drop them and change the partition.
                weight: rng.signed() * 2.0,
                label: None,
            })
            .collect();

        // A small facet vocabulary, so keys are genuinely shared and `facet_overlap` is nonzero for
        // real pairs — the term a context never exercises but a cogmap leans on.
        let facets: Vec<Facet> = (0..n_facets)
            .map(|_| Facet {
                owner: rid(rng.below(n) + 1),
                path: format!("p{}", rng.below(3)),
                value: format!("v{}", rng.below(4)),
                weight: rng.signed().abs() + 0.1,
            })
            .collect();

        // The context regime builds a kNN graph; the cogmap regime (w_cos == 0) never does — mirror
        // `substrate::load`, so this fixture is faithful to what the producer is actually handed.
        let knn = if lens.w_cos == 0.0 {
            KnnGraph::default()
        } else {
            let dim = 32;
            let topics: Vec<Vec<f32>> = (0..6)
                .map(|_| (0..dim).map(|_| rng.signed() as f32).collect())
                .collect();
            let embeddings: HashMap<ResourceId, Vec<f32>> = nodes
                .iter()
                .enumerate()
                .map(|(i, &id)| {
                    let t = &topics[i % topics.len()];
                    let v = t
                        .iter()
                        .map(|c| c * 0.9 + rng.signed() as f32 * 0.1)
                        .collect();
                    (id, v)
                })
                .collect();
            knn::build(&embeddings, lens.knn_k, lens.cos_floor)
        };

        Fixture {
            nodes,
            edges,
            facets,
            knn,
            lens,
        }
    }

    /// Both paths over the identical substrate. Returns (dense partition, sparse partition, how many
    /// candidate pairs the sparse path offered, how many pairs the dense path enumerated).
    fn both_paths(&self) -> (Vec<Vec<Uuid>>, Vec<Vec<Uuid>>, usize, usize) {
        let aff = |x: Uuid, y: Uuid| {
            affinity(
                x.into(),
                y.into(),
                &self.edges,
                &self.facets,
                &self.knn,
                &self.lens,
            )
        };
        let uuids: Vec<Uuid> = self.nodes.iter().map(|r| r.uuid()).collect();

        let dense = CandidatePairs::dense(&uuids);
        let sparse = candidate_pairs(&self.nodes, &self.edges, &self.facets, &self.knn);

        let by_dense = cluster(&uuids, &dense, &aff, self.lens.resolution);
        let by_sparse = cluster(&uuids, &sparse, &aff, self.lens.resolution);
        (by_dense, by_sparse, sparse.len(), dense.len())
    }

    /// Every pair whose affinity is genuinely nonzero — computed the slow, obvious way. The superset
    /// invariant is checked against THIS, not against the enumerator's own idea of itself.
    fn nonzero_pairs(&self) -> Vec<(Uuid, Uuid)> {
        let mut out = Vec::new();
        for i in 0..self.nodes.len() {
            for j in (i + 1)..self.nodes.len() {
                let (a, b) = (self.nodes[i], self.nodes[j]);
                let v = affinity(a, b, &self.edges, &self.facets, &self.knn, &self.lens);
                if v != 0.0 {
                    let (x, y) = (a.uuid(), b.uuid());
                    out.push(if x < y { (x, y) } else { (y, x) });
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The core claim, over many random substrates in the CONTEXT regime (`w_cos > 0`: edges + kNN).
#[test]
fn sparse_and_dense_agree_in_the_context_regime() {
    for seed in 0..40u64 {
        let f = Fixture::build(
            0xC0FFEE ^ (seed << 8),
            60,
            40,
            0, // contexts carry zero facets
            Lens::workflow_default(),
        );
        let (dense, sparse, offered, all) = f.both_paths();
        assert_eq!(
            dense, sparse,
            "seed {seed}: sparse candidates changed the partition (offered {offered} of {all} pairs)"
        );
    }
}

/// …and in the COGMAP regime (`w_cos == 0`: edges + facets, no kNN graph at all). This is the
/// regression floor of the whole arc — a cogmap's regions must stay byte-identical.
#[test]
fn sparse_and_dense_agree_in_the_cogmap_regime() {
    for seed in 0..40u64 {
        let f = Fixture::build(
            0xBEEF ^ (seed << 8),
            60,
            40,
            50, // cogmaps DO carry facets — exercise facet_overlap's candidate contribution
            Lens::telos_default(),
        );
        let (dense, sparse, offered, all) = f.both_paths();
        assert_eq!(
            dense, sparse,
            "seed {seed}: sparse candidates changed the partition (offered {offered} of {all} pairs)"
        );
    }
}

/// The invariant the equivalence rests on, asserted directly rather than inferred from the partitions
/// agreeing: every genuinely-nonzero pair is offered. If this ever fails, the partitions might still
/// coincidentally agree — so check it on its own.
#[test]
fn candidates_are_a_superset_of_the_nonzero_pairs() {
    for (label, lens, n_facets) in [
        ("context", Lens::workflow_default(), 0usize),
        ("cogmap", Lens::telos_default(), 50),
    ] {
        for seed in 0..25u64 {
            let f = Fixture::build(0x5A17 ^ (seed << 8), 50, 45, n_facets, lens.clone());
            let offered = candidate_pairs(&f.nodes, &f.edges, &f.facets, &f.knn);
            let offered: Vec<(Uuid, Uuid)> = offered.iter().copied().collect();
            let nonzero = f.nonzero_pairs();

            for pair in &nonzero {
                assert!(
                    offered.contains(pair),
                    "{label} seed {seed}: nonzero pair {pair:?} was NOT offered as a candidate — \
                     the enumeration is a subset, not a superset, and the partition can silently change"
                );
            }
            // And it must be worth doing: the offered set stays far below the dense one.
            assert!(
                offered.len() <= (f.nodes.len() * (f.nodes.len() - 1)) / 2,
                "{label} seed {seed}: candidate set exceeded the dense set"
            );
        }
    }
}

/// A pair whose terms CANCEL to exactly zero is still offered, still evaluated, and still dropped —
/// exactly as under the dense scan (`cluster::agglomerate_drops_a_blend_that_cancels_to_zero` pins the
/// clustering side of this). The enumeration must not try to be clever and pre-filter it: it does not
/// know the lens weights, and it must not need to.
#[test]
fn a_pair_that_cancels_to_zero_is_offered_and_then_dropped() {
    let (a, b) = (rid(1), rid(2));
    let lens = Lens {
        w_near: 1.0,
        w_leads_to: 1.0,
        ..Lens::telos_default()
    };
    // +1.0 via `near`, −1.0 via `leads_to` ⇒ affinity exactly 0.0.
    let edges = vec![
        Edge {
            src: a,
            tgt: b,
            kind: EdgeKind::Near,
            weight: 1.0,
            label: None,
        },
        Edge {
            src: a,
            tgt: b,
            kind: EdgeKind::LeadsTo,
            weight: -1.0,
            label: None,
        },
    ];
    let facets: Vec<Facet> = vec![];
    let g = KnnGraph::default();

    assert_eq!(
        affinity(a, b, &edges, &facets, &g, &lens),
        0.0,
        "fixture must actually cancel"
    );

    let offered = candidate_pairs(&[a, b], &edges, &facets, &g);
    assert_eq!(
        offered.len(),
        1,
        "the pair carries edges, so it IS offered — the enumerator does not evaluate affinity"
    );

    let uuids = vec![a.uuid(), b.uuid()];
    let aff = |x: Uuid, y: Uuid| affinity(x.into(), y.into(), &edges, &facets, &g, &lens);
    assert_eq!(
        cluster(&uuids, &offered, &aff, lens.resolution),
        cluster(
            &uuids,
            &CandidatePairs::dense(&uuids),
            &aff,
            lens.resolution
        ),
        "…and is then dropped by the != 0.0 guard, leaving two singletons — same as the dense scan"
    );
}

/// Nodes with no candidate pair at all must still come back as singleton components. The union-find
/// seeds every node as its own root, so this holds — but it is exactly the kind of thing an
/// "iterate the pairs instead of the nodes" rewrite silently loses.
#[test]
fn isolated_nodes_survive_as_singletons() {
    let nodes: Vec<ResourceId> = (1..=5).map(rid).collect();
    let lens = Lens::telos_default();
    // only 1—2 are bound; 3, 4, 5 touch nothing.
    let edges = vec![Edge {
        src: rid(1),
        tgt: rid(2),
        kind: EdgeKind::Express,
        weight: 1.0,
        label: None,
    }];
    let facets: Vec<Facet> = vec![];
    let g = KnnGraph::default();

    let uuids: Vec<Uuid> = nodes.iter().map(|r| r.uuid()).collect();
    let aff = |x: Uuid, y: Uuid| affinity(x.into(), y.into(), &edges, &facets, &g, &lens);
    let sparse = candidate_pairs(&nodes, &edges, &facets, &g);

    let by_sparse = cluster(&uuids, &sparse, &aff, lens.resolution);
    let by_dense = cluster(
        &uuids,
        &CandidatePairs::dense(&uuids),
        &aff,
        lens.resolution,
    );

    assert_eq!(by_sparse, by_dense);
    assert_eq!(
        by_sparse.len(),
        4,
        "{{1,2}} merged, and 3/4/5 each survive as their own region: {by_sparse:?}"
    );
}
