use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeSet, BinaryHeap, HashMap};
use uuid::Uuid;

/// Quantum for the merge-selection total order. The Lance-Williams recurrence maintains avg-link
/// affinities incrementally, so a maintained value differs from a fresh recomputation in the low-order
/// bits (float addition isn't associative) — two MATHEMATICALLY-equal candidate merges can therefore
/// carry avg-links that differ by a few ULPs. Quantizing the sort key onto this grid collapses that
/// float noise so such pairs tie EXACTLY and fall to the deterministic UUID tie-break, while staying
/// far finer than the gap between any two genuinely-distinct declared-affinity values (sums of products
/// of declared weights over small integer pair-counts, separated by ≫ 1e-9 in practice). This — plus
/// the [`Candidate`] tie-break — is the principled total order that REPLACES the old O(n³) core's
/// EPS-tolerance + iteration-order ("first-found") fallback. The byte-identical guarantee is therefore
/// "same final partition" (verified against the reference impl and the scenario corpus), NOT bit-equal
/// intermediates — which no incremental scheme can offer, since it reorders the float additions.
const SELECT_QUANTUM: f64 = 1e-9;

/// Quantize an avg-link onto the selection grid (see [`SELECT_QUANTUM`]). Bounded in practice: avg-links
/// are O(declared weight magnitudes), so `avg / 1e-9` stays many orders of magnitude inside i64.
fn qkey(avg: f64) -> i64 {
    (avg / SELECT_QUANTUM).round() as i64
}

/// Canonical (smaller-id, larger-id) key for the symmetric sparse affinity map.
fn canon(a: usize, b: usize) -> (usize, usize) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// A candidate merge sitting in the priority queue. Ordering is the selection total order: prefer the
/// higher quantized avg-link; break ties by the LEXICOGRAPHICALLY-SMALLER (cluster-min, cluster-min)
/// pair — `Reverse` so the smaller pair is "greatest" under the max-heap. `(va, vb)` are the endpoint
/// versions at push time; a pop whose endpoints have since merged (version bumped, or deactivated) is
/// stale and discarded (lazy deletion). The tie pair is unique per active cluster-pair (cluster mins are
/// unique), so the order is total over all live candidates — no iteration-order fallback survives.
#[derive(Debug)]
struct Candidate {
    qkey: i64,
    tie: Reverse<(Uuid, Uuid)>,
    a: usize,
    b: usize,
    va: u64,
    vb: u64,
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.qkey
            .cmp(&other.qkey)
            .then_with(|| self.tie.cmp(&other.tie))
    }
}

// PartialEq/Eq are hand-written to agree with `Ord` (the std contract: `a == b` iff `cmp` is `Equal`)
// — they compare only the ordering key `(qkey, tie)`, NOT the bookkeeping fields. A derive would
// compare all fields and silently break that contract; `BinaryHeap` only needs `Ord`, but the
// inconsistency is a latent footgun no lint catches.
impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Candidate {}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Build a candidate for the active cluster-pair (a, b) at the current avg-link `avg`. The tie key uses
/// each cluster's CURRENT minimum UUID (`members[i][0]`, members kept sorted) — for a merged cluster
/// that is the merged set's min, exactly the quantity the old core's `tie_key` compared.
fn make_candidate(
    a: usize,
    b: usize,
    avg: f64,
    va: u64,
    vb: u64,
    members: &[Vec<Uuid>],
) -> Candidate {
    let (ma, mb) = (members[a][0], members[b][0]);
    let tie = if ma < mb { (ma, mb) } else { (mb, ma) };
    Candidate {
        qkey: qkey(avg),
        tie: Reverse(tie),
        a,
        b,
        va,
        vb,
    }
}

/// Deterministic region clustering (spec §2b), component-decomposed (decision §3.2).
///
/// Cuts the node set into connected components of the nonzero-affinity graph, runs the average-link
/// agglomerative core ([`agglomerate`]) on each component independently, then unions the results.
/// This is byte-identical to agglomerating the whole node set, because cross-component average-link is
/// 0 and never reaches a positive `resolution` (precondition: `resolution > 0`, true for all seeded
/// lenses) — so no merge ever spans a component boundary. The decomposition is the foundation for
/// incremental re-materialization: a change re-clusters only its touched component(s).
///
/// No random initialization. Same inputs -> identical output.
pub fn cluster<F: Fn(Uuid, Uuid) -> f64>(
    nodes: &[Uuid],
    aff: &F,
    resolution: f64,
) -> Vec<Vec<Uuid>> {
    let mut clusters: Vec<Vec<Uuid>> = Vec::new();
    for component in connected_components(nodes, aff) {
        clusters.extend(agglomerate(&component, aff, resolution));
    }
    // components are disjoint ⇒ every cluster's min UUID is unique ⇒ this is a stable total order,
    // identical to whole-graph agglomeration's final `sort_by(x[0])`.
    clusters.sort_by(|x, y| x[0].cmp(&y[0]));
    clusters
}

/// Average-link (UPGMA) agglomerative clustering core (spec §2b), run over a single connected component.
/// - nodes are processed in ascending-UUID order (stable);
/// - merges the two clusters of highest average-link affinity until the best falls below `resolution`;
/// - ties broken by the lexicographically-smaller (cluster-min, cluster-min) pair (a deterministic total
///   order, decision §3.3 — replaces the old core's EPS + first-found fallback);
/// - a node with no above-resolution link remains its own cluster (separation = absence, spec §2a).
///
/// Implementation (decision §3.3): the **Lance-Williams recurrence for UPGMA** + a binary-heap priority
/// queue + a sparse precomputed affinity map. On merging clusters i and j (sizes nᵢ, nⱼ) the affinity to
/// any other cluster k is updated, not recomputed, as `(nᵢ·a(i,k) + nⱼ·a(j,k)) / (nᵢ+nⱼ)`. The affinity
/// map stays sparse: a pair is present only when nonzero, and `a(ij,k)` is nonzero only if `a(i,k)` or
/// `a(j,k)` was — so the neighbor set of a merged cluster is the union of its parents' neighbors. This
/// is ~O(n² log n) vs the old ~O(n³) all-pairs-rescan, and produces the same final partition (the
/// `agglomerate_reference` equivalence test + the scenario corpus are the regression guard).
///
/// Exposed for the incremental-materialize path, which clusters one component at a time.
pub fn agglomerate<F: Fn(Uuid, Uuid) -> f64>(
    nodes: &[Uuid],
    aff: &F,
    resolution: f64,
) -> Vec<Vec<Uuid>> {
    let mut sorted = nodes.to_vec();
    sorted.sort();
    let n = sorted.len();

    // active-cluster slabs, indexed by a stable id (0..n initially). A merge keeps the smaller-min id
    // as the survivor and deactivates the other; `version` invalidates that cluster's stale heap entries.
    let mut members: Vec<Vec<Uuid>> = sorted.iter().map(|&u| vec![u]).collect();
    let mut active: Vec<bool> = vec![true; n];
    let mut version: Vec<u64> = vec![0; n];
    // sparse symmetric avg-link over active clusters: canonical (id,id) -> weight, nonzero pairs only.
    let mut weight: HashMap<(usize, usize), f64> = HashMap::new();
    let mut adj: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    let mut heap: BinaryHeap<Candidate> = BinaryHeap::new();

    // Precompute the sparse affinity map ONCE (O(n²) affinity calls). Singletons' avg-link IS aff(x,y)
    // exactly (no accumulation), so the initial selection ties match the old core bit-for-bit. Negative
    // affinities are kept (nonzero) — they can't merge alone (< resolution > 0) but a Lance-Williams
    // blend can later cross the threshold; pairs that are exactly 0 never enter and never merge.
    for i in 0..n {
        for j in (i + 1)..n {
            let a = aff(sorted[i], sorted[j]);
            // nonzero AND finite. Production `affinity` already excludes NaN edges and bounds facet
            // overlap, so non-finite never arrives — but a non-finite weight here would slip past the
            // `avg < resolution` guard (NaN/`+inf` both fail `<`) and wrongly MERGE, where the reference
            // (`avg >= resolution`) would stop. Defense-in-depth: treat non-finite as absent (no edge).
            if a != 0.0 && a.is_finite() {
                weight.insert((i, j), a);
                adj[i].insert(j);
                adj[j].insert(i);
                heap.push(make_candidate(i, j, a, version[i], version[j], &members));
            }
        }
    }

    loop {
        // pop the best still-valid candidate; discard any whose endpoint merged since it was pushed.
        let chosen = loop {
            match heap.pop() {
                None => break None,
                Some(c) => {
                    if active[c.a] && active[c.b] && version[c.a] == c.va && version[c.b] == c.vb {
                        break Some(c);
                    }
                    // stale (endpoint merged/deactivated) -> drop and keep popping.
                }
            }
        };
        let Some(c) = chosen else { break };

        // a valid candidate's weight is unchanged since push (any change would have bumped a version),
        // so the map lookup is authoritative. A below-resolution candidate is SKIPPED, not a hard stop:
        // selection ranks by the QUANTIZED avg-link, so a below-res pair can momentarily out-rank an
        // at-res one within the same quantum — `continue` lets the at-res pair still merge, where a hard
        // `break` would wrongly strand a node that has an above-resolution link. With unambiguous
        // (well-separated) affinities this is identical to breaking: once the max is below res, all are,
        // and the heap simply drains. Raw `>=`, matching the old core — the threshold is not EPS-fuzzed.
        let avg = weight[&canon(c.a, c.b)];
        if avg < resolution {
            continue;
        }

        // survivor = the smaller-min cluster; absorb the other. (Membership output is min-sorted either
        // way; choosing by min keeps the survivor id deterministic and the tie keys monotone.)
        let (keep, gone) = if members[c.a][0] < members[c.b][0] {
            (c.a, c.b)
        } else {
            (c.b, c.a)
        };
        let n_keep = members[keep].len() as f64;
        let n_gone = members[gone].len() as f64;

        // Lance-Williams update of keep↔k for every neighbor of keep OR gone, using PRE-merge weights.
        let neighbors: Vec<usize> = adj[keep]
            .iter()
            .chain(adj[gone].iter())
            .copied()
            .filter(|&k| k != keep && k != gone)
            .collect::<BTreeSet<usize>>()
            .into_iter()
            .collect();
        let updates: Vec<(usize, f64)> = neighbors
            .iter()
            .map(|&k| {
                let w_keep = weight.get(&canon(keep, k)).copied().unwrap_or(0.0);
                let w_gone = weight.get(&canon(gone, k)).copied().unwrap_or(0.0);
                (k, (n_keep * w_keep + n_gone * w_gone) / (n_keep + n_gone))
            })
            .collect();

        // detach `gone` from the graph entirely.
        for k in adj[gone].clone() {
            adj[k].remove(&gone);
            weight.remove(&canon(gone, k));
        }
        adj[gone].clear();

        // merge member sets (keep stays sorted), deactivate gone, bump both versions.
        let gone_members = std::mem::take(&mut members[gone]);
        members[keep].extend(gone_members);
        members[keep].sort();
        active[gone] = false;
        version[gone] = version[gone].wrapping_add(1);
        version[keep] = version[keep].wrapping_add(1);

        // apply the Lance-Williams updates to keep; a blend that cancels to exactly 0 drops from the map.
        for (k, new_w) in updates {
            let ck = canon(keep, k);
            if new_w != 0.0 {
                weight.insert(ck, new_w);
                adj[keep].insert(k);
                adj[k].insert(keep);
                heap.push(make_candidate(
                    keep,
                    k,
                    new_w,
                    version[keep],
                    version[k],
                    &members,
                ));
            } else {
                weight.remove(&ck);
                adj[keep].remove(&k);
                adj[k].remove(&keep);
            }
        }
    }

    let mut clusters: Vec<Vec<Uuid>> = (0..n)
        .filter(|&i| active[i])
        .map(|i| std::mem::take(&mut members[i]))
        .collect();
    clusters.sort_by(|x, y| x[0].cmp(&y[0]));
    clusters
}

/// Connected components of the NONZERO-affinity graph (decision §3.2). Two nodes share a component
/// iff a path of strictly-nonzero affinity edges links them. This MUST be the nonzero graph, NOT the
/// resolution-thresholded one: average-link transitively pulls a zero-direct-affinity node into a
/// cluster (C joins {A,B} on avg(aff(A,C),aff(B,C))≥resolution with aff(A,C)=0), so a region can span
/// any nonzero-connected set but never crosses a nonzero-component boundary (cross-component avg-link
/// is 0 < resolution). Deterministic: each component sorted ascending, components ordered by min UUID.
pub fn connected_components<F: Fn(Uuid, Uuid) -> f64>(nodes: &[Uuid], aff: &F) -> Vec<Vec<Uuid>> {
    let mut sorted = nodes.to_vec();
    sorted.sort();
    // union-find over indices into `sorted` (stable order ⇒ deterministic).
    let mut parent: Vec<usize> = (0..sorted.len()).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]]; // path halving
            x = parent[x];
        }
        x
    }
    for i in 0..sorted.len() {
        for j in (i + 1)..sorted.len() {
            // index-pair iteration is intentional: union-find merges indices into `sorted`, and the
            // symmetric upper-triangle (j>i) visits each unordered pair once.
            if aff(sorted[i], sorted[j]) != 0.0 {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    // union toward the smaller root index to keep the merge order deterministic.
                    parent[ri.max(rj)] = ri.min(rj);
                }
            }
        }
    }
    // group indices by representative root, preserving ascending-UUID order within each component.
    let mut by_root: std::collections::BTreeMap<usize, Vec<Uuid>> =
        std::collections::BTreeMap::new();
    for (i, &node) in sorted.iter().enumerate() {
        let r = find(&mut parent, i);
        by_root.entry(r).or_default().push(node);
    }
    // BTreeMap keys are root indices into `sorted` (ascending) ⇒ components already ordered by their
    // smallest member's UUID. Each Vec is built in ascending index order ⇒ ascending UUID.
    by_root.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EPS tolerance of the ORIGINAL O(n³) core, preserved here only for [`agglomerate_reference`].
    const EPS: f64 = 1e-12;

    /// The original O(n³) average-link core (rescans + recomputes every pair each merge), kept verbatim
    /// as the equivalence reference: [`prop_agglomerate_matches_reference`] asserts the production
    /// Lance-Williams [`agglomerate`] returns the same partition across randomized fixtures. Its tie
    /// rule is the old one (EPS-tie → `tie_key` → first-found-in-iteration-order).
    fn agglomerate_reference<F: Fn(Uuid, Uuid) -> f64>(
        nodes: &[Uuid],
        aff: &F,
        resolution: f64,
    ) -> Vec<Vec<Uuid>> {
        let mut sorted = nodes.to_vec();
        sorted.sort();
        let mut clusters: Vec<Vec<Uuid>> = sorted.into_iter().map(|n| vec![n]).collect();

        loop {
            let mut best: Option<(usize, usize, f64)> = None;
            for i in 0..clusters.len() {
                for j in (i + 1)..clusters.len() {
                    let a = avg_link(&clusters[i], &clusters[j], aff);
                    best = match best {
                        None => Some((i, j, a)),
                        Some((bi, bj, b)) => {
                            let take_new = a > b + EPS
                                || ((a - b).abs() <= EPS
                                    && tie_key(&clusters[i], &clusters[j])
                                        < tie_key(&clusters[bi], &clusters[bj]));
                            if take_new {
                                Some((i, j, a))
                            } else {
                                Some((bi, bj, b))
                            }
                        }
                    };
                }
            }
            match best {
                Some((i, j, a)) if a >= resolution => {
                    let mut merged = clusters[i].clone();
                    merged.extend(clusters[j].clone());
                    merged.sort();
                    clusters.remove(j);
                    clusters[i] = merged;
                }
                _ => break,
            }
        }
        clusters.sort_by(|x, y| x[0].cmp(&y[0]));
        clusters
    }

    fn avg_link<F: Fn(Uuid, Uuid) -> f64>(a: &[Uuid], b: &[Uuid], aff: &F) -> f64 {
        let mut sum = 0.0;
        for &x in a {
            for &y in b {
                sum += aff(x, y);
            }
        }
        sum / (a.len() * b.len()) as f64
    }

    fn tie_key(a: &[Uuid], b: &[Uuid]) -> Uuid {
        let mut all: Vec<Uuid> = a.iter().chain(b.iter()).copied().collect();
        all.sort();
        all[0]
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// affinity built from an explicit symmetric edge list: aff(x,y)=w for any listed {x,y}, else 0.
    fn aff_from<'a>(edges: &'a [(u128, u128, f64)]) -> impl Fn(Uuid, Uuid) -> f64 + 'a {
        move |x: Uuid, y: Uuid| {
            edges
                .iter()
                .find(|(a, b, _)| (id(*a) == x && id(*b) == y) || (id(*a) == y && id(*b) == x))
                .map_or(0.0, |(_, _, w)| *w)
        }
    }

    #[test]
    fn connected_components_splits_disjoint_nonzero_islands() {
        // {1,2} bound, {3,4} bound, no cross affinity → two components.
        let nodes = [id(1), id(2), id(3), id(4)];
        let aff = aff_from(&[(1, 2, 1.0), (3, 4, 1.0)]);
        let comps = connected_components(&nodes, &aff);
        assert_eq!(comps, vec![vec![id(1), id(2)], vec![id(3), id(4)]]);
    }

    #[test]
    fn connected_components_uses_nonzero_graph_not_thresholded() {
        // THE crux invariant (decision §3.2): components cut on the NONZERO graph, not the
        // resolution-thresholded one. 1-2 strong, 2-3 nonzero-but-tiny (0.01, far below any
        // resolution), 1-3 exactly zero. Average-link can still transitively pull 3 into {1,2},
        // so 1,2,3 MUST be one component even though aff(1,3)=0 and aff(2,3) never crosses 0.5.
        let nodes = [id(1), id(2), id(3)];
        let aff = aff_from(&[(1, 2, 1.0), (2, 3, 0.01)]);
        let comps = connected_components(&nodes, &aff);
        assert_eq!(comps, vec![vec![id(1), id(2), id(3)]]);
    }

    #[test]
    fn connected_components_isolated_node_is_own_component() {
        // 3 has zero affinity to everyone → its own singleton component (separation = absence).
        let nodes = [id(1), id(2), id(3)];
        let aff = aff_from(&[(1, 2, 1.0)]);
        let comps = connected_components(&nodes, &aff);
        assert_eq!(comps, vec![vec![id(1), id(2)], vec![id(3)]]);
    }

    /// The byte-identical invariant the decomposition rests on: component-decomposed `cluster` produces
    /// exactly what whole-graph `agglomerate` produces, across multiple islands at the same resolution.
    /// Guards future incremental slices from drifting the production output away from a full re-cluster.
    #[test]
    fn cluster_decomposed_equals_whole_graph_agglomerate() {
        // island A: 1-2-3 chain (each link 0.9, above resolution); island B: 4-5 (0.7); 6 isolated.
        let nodes = [id(1), id(2), id(3), id(4), id(5), id(6)];
        let aff = aff_from(&[(1, 2, 0.9), (2, 3, 0.9), (4, 5, 0.7)]);
        let resolution = 0.5;

        let decomposed = cluster(&nodes, &aff, resolution);

        // independent reference: agglomerate the whole node set in one pass (the pre-decomposition path).
        let mut all = nodes.to_vec();
        all.sort();
        let whole_graph = {
            let mut cs = agglomerate(&all, &aff, resolution);
            cs.sort_by(|x: &Vec<Uuid>, y: &Vec<Uuid>| x[0].cmp(&y[0]));
            cs
        };

        assert_eq!(decomposed, whole_graph);
        // sanity: this fixture is genuinely multi-region, not a degenerate single blob.
        assert!(decomposed.len() > 1, "fixture should split into islands");
    }

    /// The Lance-Williams `agglomerate` matches the old reference partition on a hand-picked transitive
    /// case where a zero-direct-affinity pair is pulled together by average-link.
    #[test]
    fn agglomerate_matches_reference_on_transitive_pull() {
        // 1-2 strong (0.9), 2-3 strong (0.9), 1-3 zero. avg-link pulls 3 into {1,2} (avg(1,3),(2,3)) once
        // {1,2} forms — exercising a Lance-Williams blend over a zero direct affinity.
        let nodes = [id(1), id(2), id(3)];
        let aff = aff_from(&[(1, 2, 0.9), (2, 3, 0.9)]);
        let res = 0.4;
        assert_eq!(
            agglomerate(&nodes, &aff, res),
            agglomerate_reference(&nodes, &aff, res)
        );
    }

    /// Cancellation-to-exactly-0 drop (the novel sparse-map path): after {1,2} merges, its average-link
    /// to 3 is (aff(1,3)+aff(2,3))/2 = (0.5 + −0.5)/2 = 0, so the (merged, 3) pair blends to exactly 0
    /// and is dropped from the sparse map — 3 stays its own cluster. Matches the reference, which
    /// computes the same 0 < resolution fresh.
    #[test]
    fn agglomerate_drops_a_blend_that_cancels_to_zero() {
        let nodes = [id(1), id(2), id(3)];
        let aff = aff_from(&[(1, 2, 0.9), (1, 3, 0.5), (2, 3, -0.5)]);
        let res = 0.4;
        let got = agglomerate(&nodes, &aff, res);
        assert_eq!(got, agglomerate_reference(&nodes, &aff, res));
        assert_eq!(got, vec![vec![id(1), id(2)], vec![id(3)]]);
    }

    /// A negative direct affinity can't merge alone (< resolution > 0) but a Lance-Williams blend can
    /// pull it across: 2 has aff −0.2 to 3 but +0.9 to 1; once {1,3}-ish forms the blend to 2 can clear
    /// resolution. Compare against the reference to lock in the negative-blend path.
    #[test]
    fn agglomerate_negative_affinity_can_cross_threshold_via_blend() {
        // 1-3 strong (0.95), 1-2 strong (0.9), 2-3 negative (−0.2). After {1,3} forms, avg-link to 2 is
        // (aff(1,2)+aff(2,3))/2 = (0.9 + −0.2)/2 = 0.35; with res 0.3 that clears and pulls 2 in.
        let nodes = [id(1), id(2), id(3)];
        let aff = aff_from(&[(1, 3, 0.95), (1, 2, 0.9), (2, 3, -0.2)]);
        let res = 0.3;
        let got = agglomerate(&nodes, &aff, res);
        assert_eq!(got, agglomerate_reference(&nodes, &aff, res));
        assert_eq!(got, vec![vec![id(1), id(2), id(3)]]);
    }

    // ---- a tiny deterministic PRNG, so the property test needs no external dep ----
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            // xorshift64*
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        /// a "nice" declared weight from a small set, so EXACT ties between pairs are common — the
        /// regime that stresses tie-break DETERMINISM (see the order-independence property test).
        fn nice_weight(&mut self) -> f64 {
            const W: [f64; 6] = [0.3, 0.5, 0.6, 0.7, 0.9, 1.0];
            W[self.below(W.len() as u64) as usize]
        }
        /// a continuous weight in [0.30, 1.00). High-entropy, so EXACT inter-pair ties and
        /// resolution-boundary straddles are measure-zero — selection is unambiguous and the production
        /// core MUST reproduce the reference partition (no accepted tie-break divergence to confound it).
        fn cont_weight(&mut self) -> f64 {
            0.30 + (self.next() as f64 / u64::MAX as f64) * 0.70
        }
        /// a continuous weight in (−1.00, −0.20] ∪ [0.20, 1.00) — magnitude ≥ 0.20 so connected
        /// components stay well-defined (no near-zero edges), sign random. Stresses the negative-blend
        /// path at scale while keeping ties/straddles measure-zero (so the reference is reproduced).
        fn cont_signed_weight(&mut self) -> f64 {
            let mag = 0.20 + (self.next() as f64 / u64::MAX as f64) * 0.80;
            if self.below(2) == 0 {
                -mag
            } else {
                mag
            }
        }
    }

    /// Randomized equivalence: the production Lance-Williams `agglomerate` returns exactly the same
    /// partition as the O(n³) reference across many random components built from CONTINUOUS weights.
    /// Continuous weights make ties (and resolution-boundary straddles) measure-zero, so selection is
    /// unambiguous — this isolates and verifies the Lance-Williams recurrence + heap reproduce the
    /// reference's merge math. It is the ungated (no-DB, no-ONNX) correctness guard; the tie regime's
    /// determinism is covered by [`agglomerate_tiebreak_is_deterministic_and_order_independent`], and
    /// the authoritative byte-identical guard for real (tie-prone) data is the scenario corpus
    /// (`artifact-tests`). Where the old first-found fallback and the new principled total order CAN
    /// legitimately differ — genuine ties at the merge frontier — is exactly what continuous weights
    /// exclude here by construction.
    #[test]
    fn prop_agglomerate_matches_reference() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for seed in 0..500u64 {
            rng.0 ^= seed.wrapping_mul(0xD1B5_4A32_D192_ED03).wrapping_add(1);
            let m = 2 + rng.below(11) as u128; // 2..=12 nodes
            let mut edges: Vec<(u128, u128, f64)> = Vec::new();
            for a in 1..=m {
                for b in (a + 1)..=m {
                    // ~55% edge density, so multi-region splits and singletons both occur.
                    if rng.below(100) < 55 {
                        edges.push((a, b, rng.cont_weight()));
                    }
                }
            }
            let nodes: Vec<Uuid> = (1..=m).map(id).collect();
            let aff = aff_from(&edges);
            let res = [0.45f64, 0.6, 0.75][rng.below(3) as usize];

            let got = cluster(&nodes, &aff, res);
            let want = {
                let mut cs = agglomerate_reference(&nodes, &aff, res);
                cs.sort_by(|x: &Vec<Uuid>, y: &Vec<Uuid>| x[0].cmp(&y[0]));
                cs
            };
            assert_eq!(
                got, want,
                "seed {seed}: edges={edges:?} res={res} diverged from reference"
            );
        }
    }

    /// Same equivalence guarantee with SIGNED continuous weights, so the negative-affinity paths (a
    /// negative direct link, and a Lance-Williams blend pulling a negative pair across resolution) run
    /// at scale against the reference. Continuous magnitudes keep selection unambiguous.
    #[test]
    fn prop_agglomerate_matches_reference_signed() {
        let mut rng = Rng(0x51ED_2A17_C0FF_EE42);
        for seed in 0..500u64 {
            rng.0 ^= seed.wrapping_mul(0xD1B5_4A32_D192_ED03).wrapping_add(1);
            let m = 2 + rng.below(11) as u128; // 2..=12 nodes
            let mut edges: Vec<(u128, u128, f64)> = Vec::new();
            for a in 1..=m {
                for b in (a + 1)..=m {
                    if rng.below(100) < 55 {
                        edges.push((a, b, rng.cont_signed_weight()));
                    }
                }
            }
            let nodes: Vec<Uuid> = (1..=m).map(id).collect();
            let aff = aff_from(&edges);
            let res = [0.25f64, 0.4, 0.6][rng.below(3) as usize];

            let got = cluster(&nodes, &aff, res);
            let want = {
                let mut cs = agglomerate_reference(&nodes, &aff, res);
                cs.sort_by(|x: &Vec<Uuid>, y: &Vec<Uuid>| x[0].cmp(&y[0]));
                cs
            };
            assert_eq!(
                got, want,
                "seed {seed}: edges={edges:?} res={res} diverged from reference (signed)"
            );
        }
    }

    /// Tie-break determinism + input-order independence over TIE-PRONE "nice" weights (the regime the
    /// equivalence test deliberately avoids). The principled total order may legitimately resolve a
    /// genuine tie differently from the old first-found fallback, but it must be a FUNCTION of the
    /// inputs: shuffling input order must not change the partition (we sort internally), and a re-run
    /// must be identical. This is what the determinism acceptance ("no random init; min-UUID tie-break")
    /// asserts in the ambiguous regime.
    #[test]
    fn agglomerate_tiebreak_is_deterministic_and_order_independent() {
        let mut rng = Rng(0x0BAD_C0DE_F00D_1357);
        for seed in 0..300u64 {
            rng.0 ^= seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
            let m = 2 + rng.below(9) as u128; // 2..=10 nodes
            let mut edges: Vec<(u128, u128, f64)> = Vec::new();
            for a in 1..=m {
                for b in (a + 1)..=m {
                    if rng.below(100) < 60 {
                        edges.push((a, b, rng.nice_weight()));
                    }
                }
            }
            let aff = aff_from(&edges);
            let res = [0.3f64, 0.5, 0.7][rng.below(3) as usize];

            // ascending order vs a deterministically shuffled order vs a re-run.
            let ascending: Vec<Uuid> = (1..=m).map(id).collect();
            let mut shuffled = ascending.clone();
            for i in (1..shuffled.len()).rev() {
                shuffled.swap(i, rng.below((i + 1) as u64) as usize);
            }
            let base = agglomerate(&ascending, &aff, res);
            assert_eq!(
                base,
                agglomerate(&shuffled, &aff, res),
                "seed {seed}: partition must not depend on input node order"
            );
            assert_eq!(
                base,
                agglomerate(&ascending, &aff, res),
                "seed {seed}: re-run must be identical (no random init)"
            );
        }
    }

    /// Benchmark (decision §3.3 acceptance: "benchmarked improvement on a synthetic large component").
    /// `#[ignore]`d — timing assertions don't gate CI. Run with:
    ///   cargo nextest run -p temper-substrate agglomerate_benchmark_large_component --run-ignored all
    /// or `cargo test -p temper-substrate -- --ignored agglomerate_benchmark_large_component --nocapture`.
    /// Builds one dense ~250-node component, times the old O(n³) reference vs the Lance-Williams core
    /// (over a precomputed-matrix affinity closure so we measure clustering, not the affinity scan),
    /// asserts identical partitions, and prints the speedup.
    #[test]
    #[ignore = "benchmark: run explicitly with --ignored"]
    fn agglomerate_benchmark_large_component() {
        use std::time::Instant;

        let n: u128 = 250;
        let mut rng = Rng(0x1234_5678_9ABC_DEF0);
        // dense nonzero matrix so it stays ONE big component with lots of merges.
        let mut matrix = std::collections::HashMap::new();
        for a in 1..=n {
            for b in (a + 1)..=n {
                matrix.insert((a, b), rng.nice_weight());
            }
        }
        let aff = |x: Uuid, y: Uuid| {
            let (a, b) = (x.as_u128(), y.as_u128());
            let key = if a < b { (a, b) } else { (b, a) };
            matrix.get(&key).copied().unwrap_or(0.0)
        };
        let nodes: Vec<Uuid> = (1..=n).map(id).collect();
        let res = 0.7;

        let t0 = Instant::now();
        let reference = agglomerate_reference(&nodes, &aff, res);
        let t_ref = t0.elapsed();

        let t1 = Instant::now();
        let lw = agglomerate(&nodes, &aff, res);
        let t_lw = t1.elapsed();

        assert_eq!(
            lw, reference,
            "benchmark fixture must stay partition-identical"
        );
        println!(
            "agglomerate {n} nodes: reference(O(n^3))={t_ref:?}  lance-williams={t_lw:?}  speedup={:.1}x",
            t_ref.as_secs_f64() / t_lw.as_secs_f64().max(1e-9)
        );
        assert!(
            t_lw <= t_ref,
            "Lance-Williams ({t_lw:?}) should not be slower than the O(n^3) reference ({t_ref:?})"
        );
    }
}
