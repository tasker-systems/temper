//! The sparse exact-kNN affinity graph — the context regime's primary signal (spec §3.1).
//!
//! Cosine is DENSE: every pair of resources has a nonzero similarity. Dropping a raw cosine into
//! [`crate::affinity::affinity`] would make the affinity graph COMPLETE — `connected_components`
//! returns one blob, the pre-pass that makes agglomeration tractable stops doing anything, and cost
//! goes Θ(n²) in the agglomerator too. So the embedding term contributes a *sparsified* graph: each
//! node's top-k neighbours above a similarity floor, and nothing else.
//!
//! Computed EXACTLY, never via HNSW. Two reasons, and the second is the binding one:
//!   1. A scoped corpus is small enough to scan (the same reasoning as the #358 scoped-search fix).
//!   2. An approximate index is not reproducible across index rebuilds, and `membership_fingerprint`
//!      depends on formation being deterministic.
//!
//! SYMMETRIZATION IS MUTUAL-**OR**: a pair kept by *either* endpoint's top-k survives. This is what
//! keeps a hub-and-spoke topology (goals — spec §3.3) from being severed by a popular node's own k
//! limit: a hub with 40 spokes can only name 12 of them, but each spoke names the hub, so all 40
//! edges survive. An AND-symmetrization would shred exactly the topology contexts are richest in.
//!
//! SCALE CEILING (spec §7): this is O(n²) in pairwise cosines. Comfortable at ~1k nodes (@me/temper
//! is 1,012), fine at a few thousand, NOT fine at 50k. When a context crosses that, the options are
//! blocked/tiled exact computation or accepting an approximate index and giving up fingerprint
//! determinism. Revisit here.

use std::collections::HashMap;

use crate::ids::ResourceId;

/// A symmetric, sparse similarity graph over resources. Absent pair ⇒ similarity 0.0.
#[derive(Debug, Default, Clone)]
pub struct KnnGraph {
    /// Symmetric: an entry exists under both `(a,b)` and `(b,a)`.
    sims: HashMap<(ResourceId, ResourceId), f64>,
    adj: HashMap<ResourceId, Vec<ResourceId>>,
}

impl KnnGraph {
    /// Similarity of the pair, or 0.0 if `b` is not a retained neighbour of `a`. The 0.0 default is
    /// the sparsity: a pair outside the graph contributes nothing to affinity however similar its
    /// raw cosine may be.
    pub fn sim(&self, a: ResourceId, b: ResourceId) -> f64 {
        self.sims.get(&(a, b)).copied().unwrap_or(0.0)
    }

    /// `a`'s retained neighbours, sorted by id (deterministic iteration order).
    pub fn neighbours(&self, a: ResourceId) -> &[ResourceId] {
        self.adj.get(&a).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// True when no pair was retained — the declared-only regime, where the graph is never built.
    pub fn is_empty(&self) -> bool {
        self.sims.is_empty()
    }

    /// Every retained pair as `(a, b, sim)` with `a < b` — each undirected pair yielded ONCE, in a
    /// deterministic order. This is the form [`crate::fingerprint::component_fingerprint`] hashes.
    pub fn pairs(&self) -> Vec<(ResourceId, ResourceId, f64)> {
        let mut out: Vec<(ResourceId, ResourceId, f64)> = self
            .sims
            .iter()
            .filter(|((a, b), _)| a < b)
            .map(|((a, b), s)| (*a, *b, *s))
            .collect();
        out.sort_by(|x, y| (x.0, x.1).cmp(&(y.0, y.1)));
        out
    }

    /// Test constructor: build directly from explicit `(a, b, sim)` triples, bypassing the embedding
    /// scan. Every triple is retained — this is a hand-built graph, not a top-k selection.
    pub fn from_pairs(pairs: &[(ResourceId, ResourceId, f64)]) -> Self {
        let mut g = KnnGraph::default();
        for &(a, b, s) in pairs {
            g.link(a, b, s);
        }
        g.canonicalize();
        g
    }

    fn link(&mut self, a: ResourceId, b: ResourceId, s: f64) {
        self.sims.insert((a, b), s);
        self.sims.insert((b, a), s);
        self.adj.entry(a).or_default().push(b);
        self.adj.entry(b).or_default().push(a);
    }

    /// Sort + dedup every adjacency list, so `neighbours()` is deterministic and the OR-symmetrized
    /// double-insert of a mutually-selected pair doesn't show up twice.
    fn canonicalize(&mut self) {
        for v in self.adj.values_mut() {
            v.sort();
            v.dedup();
        }
    }
}

/// Cosine similarity of two vectors that are ALREADY unit-normalized — i.e. a plain dot product.
/// Normalizing up front turns the O(n²) inner loop from three accumulators into one.
fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum()
}

/// L2-normalize, or `None` for a zero vector (no direction ⇒ no meaningful cosine ⇒ excluded from
/// the graph entirely rather than silently scoring 0.0 against everything).
fn normalize(v: &[f32]) -> Option<Vec<f32>> {
    let norm: f64 = v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    if norm == 0.0 || !norm.is_finite() {
        return None;
    }
    Some(v.iter().map(|x| (f64::from(*x) / norm) as f32).collect())
}

/// Build the symmetric sparse kNN graph over `embeddings`.
///
/// Deterministic by construction, which `membership_fingerprint` depends on:
///   - the node list is sorted by id, so the outer scan doesn't inherit hash-map iteration order;
///   - neighbour selection sorts by `(similarity DESC, id ASC)` — a TOTAL order, so an exact float
///     tie falls to a stable id comparison rather than to whichever candidate was visited first;
///   - adjacency lists are sorted and deduped at the end.
///
/// Pairs below `floor` are dropped before top-k, so `k` bounds the neighbours a node may *keep*, not
/// the neighbours it must find.
pub fn build(embeddings: &HashMap<ResourceId, Vec<f32>>, k: usize, floor: f64) -> KnnGraph {
    let mut g = KnnGraph::default();
    if k == 0 {
        return g;
    }

    // Sorted, normalized, zero-vectors dropped. Parallel arrays so the hot loop indexes rather than
    // hashes.
    let mut nodes: Vec<ResourceId> = embeddings.keys().copied().collect();
    nodes.sort();
    let unit: Vec<(ResourceId, Vec<f32>)> = nodes
        .into_iter()
        .filter_map(|id| normalize(&embeddings[&id]).map(|v| (id, v)))
        .collect();

    // Upper triangle only — cosine is symmetric, so each pair is scored ONCE and offered to both
    // endpoints' candidate lists.
    let mut cands: Vec<Vec<(ResourceId, f64)>> = vec![Vec::new(); unit.len()];
    for i in 0..unit.len() {
        for j in (i + 1)..unit.len() {
            let s = dot(&unit[i].1, &unit[j].1);
            if s >= floor {
                cands[i].push((unit[j].0, s));
                cands[j].push((unit[i].0, s));
            }
        }
    }

    for (i, (id, _)) in unit.iter().enumerate() {
        let c = &mut cands[i];
        // similarity DESC, then id ASC — a total order, so no float tie falls to visit order.
        c.sort_by(|x, y| {
            y.1.partial_cmp(&x.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| x.0.cmp(&y.0))
        });
        for &(b, s) in c.iter().take(k) {
            // Mutual-OR: this pair binds because *this* endpoint selected it. If `b` also selected
            // `id`, the second insert is a no-op — `canonicalize` dedups the adjacency.
            g.link(*id, b, s);
        }
    }
    g.canonicalize();
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn rid(n: u128) -> ResourceId {
        ResourceId::from(Uuid::from_u128(n))
    }

    #[test]
    fn build_keeps_only_top_k_above_the_floor_and_is_symmetric() {
        let (a, b, c) = (rid(1), rid(2), rid(3));
        // a is close to b, orthogonal to c. A floor of 0.55 excludes c.
        let embs = HashMap::from([
            (a, vec![1.0, 0.0]),
            (b, vec![0.95, 0.31]),
            (c, vec![0.0, 1.0]),
        ]);
        let g = build(&embs, 2, 0.55);
        assert!(g.sim(a, b) > 0.9, "a near-duplicate pair is retained");
        assert_eq!(g.sim(a, c), 0.0, "below the floor contributes nothing");
        assert_eq!(g.sim(a, b), g.sim(b, a), "the graph is symmetric");
    }

    #[test]
    fn a_pair_below_the_floor_is_absent_even_with_room_under_k() {
        // The floor is not a tie-breaker of last resort — it is a hard gate. k=10 leaves plenty of
        // room, and the orthogonal pair STILL must not enter the graph.
        let (a, b) = (rid(1), rid(2));
        let embs = HashMap::from([(a, vec![1.0, 0.0]), (b, vec![0.0, 1.0])]);
        let g = build(&embs, 10, 0.55);
        assert_eq!(g.sim(a, b), 0.0);
        assert!(g.is_empty(), "no pair cleared the floor, so no graph");
    }

    #[test]
    fn k_bounds_the_neighbour_count() {
        // 5 mutually-similar nodes, k=2 => each node SELECTS at most 2, but OR-symmetrization means
        // it may also be selected BY others, so degree is bounded by ~2k, not k.
        let ids: Vec<ResourceId> = (1..=5).map(rid).collect();
        let embs: HashMap<_, _> = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, vec![1.0, i as f32 * 0.01]))
            .collect();
        let g = build(&embs, 2, 0.0);
        for &id in &ids {
            assert!(
                g.neighbours(id).len() <= 4,
                "symmetrized degree stays bounded by ~2k, not the full n-1"
            );
        }
        // Sparsity is the POINT: a complete graph on 5 nodes would give every node degree 4.
        let total: usize = ids.iter().map(|&i| g.neighbours(i).len()).sum();
        assert!(total < 5 * 4, "the graph must be sparser than complete");
    }

    #[test]
    fn a_hub_keeps_all_its_spokes_under_or_symmetrization() {
        // Spec §3.3: goals are hubs. A hub with more spokes than k can only NAME k of them — but each
        // spoke names the hub, and OR-symmetrization keeps a pair EITHER endpoint selected. If this
        // ever regresses to AND (or to "the selector's list wins"), the hub topology that contexts are
        // richest in gets shredded: a goal would keep k of its tasks and silently drop the rest.
        //
        // A true hub-and-spoke needs spokes each NEAR THE HUB but FAR FROM EACH OTHER — which cannot
        // be built in 2D. Give each spoke the hub's direction plus its own orthogonal dimension:
        //   cos(hub, spoke) = 0.7   (every spoke's single best neighbour is the hub)
        //   cos(spoke, spoke') = 0.49  (spokes are mutually distant — their private dims are orthogonal)
        const N: usize = 8;
        let hub = rid(1);
        let spokes: Vec<ResourceId> = (2..=(N as u128 + 1)).map(rid).collect();

        let mut hub_vec = vec![0.0f32; N + 1];
        hub_vec[0] = 1.0;
        let mut embs = HashMap::from([(hub, hub_vec)]);
        for (i, &s) in spokes.iter().enumerate() {
            let mut v = vec![0.0f32; N + 1];
            v[0] = 0.7; // shared direction with the hub
            v[i + 1] = 0.714; // this spoke's private, mutually-orthogonal direction
            embs.insert(s, v);
        }

        // k=1: the hub may select exactly ONE spoke. Every spoke selects the hub (its best neighbour).
        // Floor 0.0, so the floor does no work here — this test is purely about symmetrization.
        let g = build(&embs, 1, 0.0);

        for &s in &spokes {
            assert!(
                g.sim(hub, s) > 0.0,
                "every spoke must survive: it selected the hub even though the hub (k=1) could not \
                 select it back"
            );
        }
        // The result is a clean star: the hub holds all N spokes, each spoke holds only the hub.
        assert_eq!(g.neighbours(hub).len(), N, "the hub keeps every spoke");
        for &s in &spokes {
            assert_eq!(
                g.neighbours(s),
                &[hub],
                "a spoke's only neighbour is the hub — the spokes are mutually distant"
            );
        }
    }

    #[test]
    fn build_is_deterministic() {
        // Determinism is a hard requirement — membership_fingerprint depends on it. This is also why
        // we compute exact kNN and never touch HNSW.
        let ids: Vec<ResourceId> = (1..=8).map(rid).collect();
        let embs: HashMap<_, _> = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, vec![(i as f32).sin(), (i as f32).cos()]))
            .collect();
        let a = build(&embs, 3, 0.0);
        let b = build(&embs, 3, 0.0);
        for &x in &ids {
            for &y in &ids {
                assert_eq!(
                    a.sim(x, y),
                    b.sim(x, y),
                    "identical inputs must give identical graphs"
                );
            }
            assert_eq!(a.neighbours(x), b.neighbours(x));
        }
        assert_eq!(a.pairs(), b.pairs(), "the hashed pair form is stable too");
    }

    #[test]
    fn exact_ties_break_on_id_not_visit_order() {
        // Three nodes IDENTICAL to the query vector: every candidate scores exactly 1.0. With k=1 the
        // selection is a pure tie, and it must fall to the lowest id — not to hash-map order.
        let q = rid(100);
        let (a, b, c) = (rid(1), rid(2), rid(3));
        let embs = HashMap::from([
            (q, vec![1.0, 0.0]),
            (a, vec![1.0, 0.0]),
            (b, vec![1.0, 0.0]),
            (c, vec![1.0, 0.0]),
        ]);
        let g = build(&embs, 1, 0.9);
        // q selects `a` (lowest id among the exact ties).
        assert!(g.sim(q, a) > 0.99, "the tie must resolve to the lowest id");
    }

    #[test]
    fn a_zero_vector_is_excluded_rather_than_scoring_zero_against_everything() {
        let (a, z) = (rid(1), rid(2));
        let embs = HashMap::from([(a, vec![1.0, 0.0]), (z, vec![0.0, 0.0])]);
        let g = build(&embs, 5, 0.0);
        assert_eq!(g.sim(a, z), 0.0);
        assert!(
            g.neighbours(z).is_empty(),
            "a directionless vector has no neighbours"
        );
    }

    #[test]
    fn pairs_yields_each_undirected_pair_once() {
        let (a, b, c) = (rid(1), rid(2), rid(3));
        let g = KnnGraph::from_pairs(&[(a, b, 0.8), (b, c, 0.7)]);
        let p = g.pairs();
        assert_eq!(
            p.len(),
            2,
            "two undirected pairs, not four directed entries"
        );
        assert_eq!(p[0], (a, b, 0.8));
        assert_eq!(p[1], (b, c, 0.7));
    }
}
