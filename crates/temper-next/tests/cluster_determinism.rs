use temper_next::affinity::{affinity, Edge, EdgeKind, Facet, Lens};
use temper_next::cluster::cluster;
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
        src: a,
        tgt: b,
        kind: EdgeKind::LeadsTo,
        weight: 0.9,
        label: None,
    }];
    (vec![a, b, c], edges, vec![], lens)
}

#[test]
fn isolated_node_forms_its_own_cluster() {
    let (nodes, edges, facets, lens) = fixture();
    let aff = |x: Uuid, y: Uuid| affinity(x, y, &edges, &facets, &lens);
    let clusters = cluster(&nodes, &aff, lens.resolution);
    assert_eq!(clusters.len(), 2);
    assert!(clusters.iter().any(|c| c == &vec![id(1), id(2)]));
    assert!(clusters.iter().any(|c| c == &vec![id(3)]));
}

#[test]
fn reproducible_byte_identical_on_rerun() {
    let (nodes, edges, facets, lens) = fixture();
    let aff = |x: Uuid, y: Uuid| affinity(x, y, &edges, &facets, &lens);
    let one = cluster(&nodes, &aff, lens.resolution);
    let two = cluster(&nodes, &aff, lens.resolution);
    assert_eq!(one, two);
}
