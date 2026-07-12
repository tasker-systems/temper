use temper_substrate::affinity::{affinity, Edge, EdgeKind, Facet, Lens};
use temper_substrate::cluster::{cluster, CandidatePairs};
use temper_substrate::knn::KnnGraph;
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// Three nodes: a—b strongly edged (above resolution), c isolated. Expect {a,b} and {c}.
fn fixture() -> (Vec<Uuid>, Vec<Edge>, Vec<Facet>, Lens) {
    let (a, b, c) = (id(1), id(2), id(3));
    let lens = Lens {
        w_leads_to: 1.0,
        resolution: 0.5,
        ..Lens::telos_default()
    };
    let edges = vec![Edge {
        src: a.into(),
        tgt: b.into(),
        kind: EdgeKind::LeadsTo,
        weight: 0.9,
        label: None,
    }];
    (vec![a, b, c], edges, vec![], lens)
}

#[test]
fn isolated_node_forms_its_own_cluster() {
    let (nodes, edges, facets, lens) = fixture();
    // the declared-only regime: `lens` is telos-default (w_cos = 0), so the graph is never consulted
    let knn = KnnGraph::default();
    let aff = |x: Uuid, y: Uuid| affinity(x.into(), y.into(), &edges, &facets, &knn, &lens);
    let clusters = cluster(
        &nodes,
        &CandidatePairs::dense(&nodes),
        &aff,
        lens.resolution,
    );
    assert_eq!(clusters.len(), 2);
    assert!(clusters.iter().any(|c| c == &vec![id(1), id(2)]));
    assert!(clusters.iter().any(|c| c == &vec![id(3)]));
}

#[test]
fn reproducible_byte_identical_on_rerun() {
    let (nodes, edges, facets, lens) = fixture();
    // the declared-only regime: `lens` is telos-default (w_cos = 0), so the graph is never consulted
    let knn = KnnGraph::default();
    let aff = |x: Uuid, y: Uuid| affinity(x.into(), y.into(), &edges, &facets, &knn, &lens);
    let one = cluster(
        &nodes,
        &CandidatePairs::dense(&nodes),
        &aff,
        lens.resolution,
    );
    let two = cluster(
        &nodes,
        &CandidatePairs::dense(&nodes),
        &aff,
        lens.resolution,
    );
    assert_eq!(one, two);
}
