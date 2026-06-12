use uuid::Uuid;

const EPS: f64 = 1e-12;

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

/// Average-link agglomerative clustering core (spec §2b), run over a single connected component.
/// - nodes are processed in ascending-UUID order (stable);
/// - merges the two clusters of highest average-link affinity until the best falls below `resolution`;
/// - ties (within EPS) broken by the lexicographically-smaller merged UUID set (stable);
/// - a node with no above-resolution link remains its own cluster (separation = absence, spec §2a).
///
/// Exposed for the incremental-materialize path, which clusters one component at a time.
pub fn agglomerate<F: Fn(Uuid, Uuid) -> f64>(
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
                        // take the new pair if strictly better, OR a tie (within EPS) broken by the
                        // lexicographically-smaller merged UUID set (stable); else keep the best.
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
                clusters.remove(j); // j > i, remove the later index first
                clusters[i] = merged;
            }
            _ => break,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
