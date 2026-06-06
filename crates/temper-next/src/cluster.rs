use uuid::Uuid;

const EPS: f64 = 1e-12;

/// Deterministic average-link agglomerative clustering (spec §2b).
/// - nodes are processed in ascending-UUID order (stable);
/// - merges the two clusters of highest average-link affinity until the best falls below `resolution`;
/// - ties (within EPS) broken by the lexicographically-smaller merged UUID set (stable);
/// - a node with no above-resolution link remains its own cluster (separation = absence, spec §2a).
///
/// No random initialization. Same inputs -> identical output.
pub fn cluster<F: Fn(Uuid, Uuid) -> f64>(
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
