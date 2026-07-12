use crate::cluster::CandidatePairs;
use crate::ids::ResourceId;
use crate::knn::KnnGraph;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum EdgeKind {
    Express,
    Contains,
    LeadsTo,
    Near,
}

impl EdgeKind {
    /// The `edge_kind` SQL enum label (bound with a `::edge_kind` cast at the query edge).
    pub fn as_sql(self) -> &'static str {
        match self {
            EdgeKind::Express => "express",
            EdgeKind::Contains => "contains",
            EdgeKind::LeadsTo => "leads_to",
            EdgeKind::Near => "near",
        }
    }

    /// Parse an `edge_kind` SQL enum label back into the typed kind — the inverse of [`as_sql`], used
    /// by synthesis to map a production `kb_resource_edges.edge_kind` text value onto the typed enum
    /// (§4: kind carries verbatim). `None` for an unrecognized label (escalates at the call site).
    ///
    /// [`as_sql`]: EdgeKind::as_sql
    pub fn from_sql(s: &str) -> Option<Self> {
        match s {
            "express" => Some(EdgeKind::Express),
            "contains" => Some(EdgeKind::Contains),
            "leads_to" => Some(EdgeKind::LeadsTo),
            "near" => Some(EdgeKind::Near),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub src: ResourceId,
    pub tgt: ResourceId,
    pub kind: EdgeKind,
    pub weight: f64,
    pub label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Facet {
    pub owner: ResourceId,
    pub path: String,
    pub value: String,
    pub weight: f64,
}

#[derive(Clone, Debug)]
pub struct Lens {
    pub w_express: f64,
    pub w_contains: f64,
    pub w_leads_to: f64,
    pub w_near: f64,
    pub w_prop: f64,
    /// Weight on the sparse exact-kNN cosine term. `0.0` = the **cogmap regime**: the declared graph
    /// is the whole signal and formation is byte-identical to its pre-kernel behavior. `> 0` = the
    /// **context regime**, where the embedding is the PRIMARY evidence of regionality rather than a
    /// second-order readout. Spec §3.1.
    ///
    /// This field is the regime switch, and a lot hangs off it being exactly zero on a cogmap: the
    /// [`crate::knn::KnnGraph`] is never even built (`substrate::load`), and the kNN pairs are
    /// excluded from the component fingerprint ([`crate::fingerprint::component_fingerprint`]).
    pub w_cos: f64,
    /// Neighbours each node may retain in the kNN graph. Only read when `w_cos != 0.0`.
    pub knn_k: usize,
    /// Similarity floor below which a pair never enters the kNN graph. Only read when `w_cos != 0.0`.
    pub cos_floor: f64,
    pub s_telos: f64,
    pub s_ref: f64,
    pub s_central: f64,
    pub resolution: f64,
}

impl Lens {
    /// Concrete starting defaults (spec §5c; tunable, OQ-2). MUST mirror the seeded telos-default row.
    pub fn telos_default() -> Self {
        Lens {
            w_express: 1.0,
            w_contains: 1.0,
            w_leads_to: 0.6,
            w_near: 0.3,
            w_prop: 0.4,
            w_cos: 0.0, // the cogmap regime: declared graph only
            knn_k: 12,
            cos_floor: 0.55,
            s_telos: 0.5,
            s_ref: 0.3,
            s_central: 0.2,
            resolution: 0.5,
        }
    }

    /// The context regime (spec §3.2). MUST mirror the seeded `workflow-default` row.
    ///
    /// Note what is NOT zeroed: `w_express`, `w_contains`, and `w_prop` are held at **cogmap parity**
    /// even though contexts carry zero facets and almost no express/contains edges today. A lens
    /// weight is a rate of exchange — what a signal is WORTH when present — not a prior on how often
    /// it appears. Sparsity already handles itself: a pair with no express edge contributes zero from
    /// that term whatever the weight. And an express edge asserted mid-session is *more* evidential
    /// than one a steward asserts as its job, because the rarity is what makes it informative.
    ///
    /// The binding reason is a feedback loop: a weight of 0.0 makes the discipline provably
    /// unrewarded, and an information system that returns no signal for signal provided gets routed
    /// around — which forecloses the only mechanism by which contexts ever BECOME better-structured.
    pub fn workflow_default() -> Self {
        Lens {
            w_express: 1.0,  // parity — deliberate, rare, high-information
            w_contains: 1.0, // parity
            w_prop: 0.4,     // parity
            w_leads_to: 0.9, // `advances` — cheap to create, but it IS the hub topology (§3.3)
            w_near: 0.35,    // `relates_to` — cheapest, most abundant. Real but weak.
            w_cos: 1.0,      // the regime switch: inferred similarity is PRIMARY here
            knn_k: 12,
            cos_floor: 0.55,
            s_telos: 0.6,
            s_ref: 0.15, // contexts have shallower provenance depth than distilled nodes
            s_central: 0.25,
            resolution: 0.5,
        }
    }

    fn w_kind(&self, k: EdgeKind) -> f64 {
        match k {
            EdgeKind::Express => self.w_express,
            EdgeKind::Contains => self.w_contains,
            EdgeKind::LeadsTo => self.w_leads_to,
            EdgeKind::Near => self.w_near,
        }
    }
}

/// min-weighted overlap over shared (path,value) facet pairs (spec §4b). Declared only — never cosine.
fn facet_overlap(a: ResourceId, b: ResourceId, facets: &[Facet]) -> f64 {
    let fa: Vec<&Facet> = facets.iter().filter(|f| f.owner == a).collect();
    let fb: Vec<&Facet> = facets.iter().filter(|f| f.owner == b).collect();
    let mut sum = 0.0;
    for x in &fa {
        for y in &fb {
            if x.path == y.path && x.value == y.value {
                sum += x.weight.min(y.weight);
            }
        }
    }
    sum
}

/// Symmetric affinity (spec §3.1). Three ADDITIVE terms — the two regimes differ only in `w_cos`:
///
/// ```text
/// affinity(a,b) =  Σ_edges  w_kind · weight        (declared: weak supervision in a context)
///               +  w_prop · facet_overlap(a,b)     (declared: weak supervision in a context)
///               +  w_cos  · knn_sim(a,b)           (inferred: sparse by construction; 0 in a cogmap)
/// ```
///
/// `knn` is a SPARSE graph, not a dense cosine — `knn.sim(a,b)` is 0.0 for any pair outside the
/// top-k-above-floor construction, however similar the raw vectors are. That sparsity is load-bearing:
/// see [`crate::knn`].
///
/// Labels are not reserved (spec §2a): every label is ordinary positive relatedness, so contradiction
/// BINDS (shared frame), never separates.
pub fn affinity(
    a: ResourceId,
    b: ResourceId,
    edges: &[Edge],
    facets: &[Facet],
    knn: &KnnGraph,
    lens: &Lens,
) -> f64 {
    let edge_sum: f64 = edges
        .iter()
        .filter(|e| (e.src == a && e.tgt == b) || (e.src == b && e.tgt == a))
        .filter(|e| !e.weight.is_nan())
        .map(|e| lens.w_kind(e.kind) * e.weight)
        .sum();
    edge_sum + lens.w_prop * facet_overlap(a, b, facets) + lens.w_cos * knn.sim(a, b)
}

/// Enumerate the pairs whose [`affinity`] **can** be nonzero — the sparse input to the clustering core
/// ([`CandidatePairs`]).
///
/// This is the superset invariant, and it is a direct reading of `affinity` above: the affinity of a
/// pair is the sum of exactly three terms, and each term is zero unless the pair appears in the
/// corresponding structure.
///
/// - `Σ_edges w_kind · weight` — zero unless a declared edge joins the pair.
/// - `w_prop · facet_overlap`  — zero unless the pair shares a `(path, value)` facet.
/// - `w_cos · knn.sim`         — zero unless the pair is a RETAINED kNN neighbour. `knn.sim` returns
///   0.0 for anything outside the top-k-above-floor construction, however similar the raw vectors are
///   ([`crate::knn`]); that sparsity is what makes this enumeration worth doing at all.
///
/// A pair in none of the three has all three terms zero, hence affinity **exactly** 0.0 — and both
/// consumers already discard exactly-zero pairs. So omitting it changes nothing, and the clustering is
/// partition-identical to the dense scan (`tests/sparse_candidates_differential.rs` asserts it).
///
/// Deliberately NOT lens-aware. A zero weight (`w_cos = 0` in the cogmap regime, say) makes a term
/// vanish, so a lens-aware version could prune further — but the result would still be a superset, and
/// making correctness depend on the lens's weights would put a silent partition change one config edit
/// away. A superset is always safe; the pairs it over-offers are dropped by the `!= 0.0` guard. It also
/// costs nothing in practice: in the cogmap regime the kNN graph is never even built, so its
/// contribution here is empty regardless.
///
/// Terms can also CANCEL to zero (a negative edge weight against a positive cosine — see
/// `cluster::agglomerate_drops_a_blend_that_cancels_to_zero`). Those pairs are still offered, still
/// evaluated, and still dropped by the `!= 0.0` guard — exactly as under the dense scan.
pub fn candidate_pairs(
    nodes: &[ResourceId],
    edges: &[Edge],
    facets: &[Facet],
    knn: &KnnGraph,
) -> CandidatePairs {
    let members: HashSet<ResourceId> = nodes.iter().copied().collect();
    let mut pairs: Vec<(Uuid, Uuid)> = Vec::new();

    // term 1 — declared edges.
    for e in edges {
        if members.contains(&e.src) && members.contains(&e.tgt) {
            pairs.push((e.src.uuid(), e.tgt.uuid()));
        }
    }

    // term 3 — retained kNN neighbours (empty in the cogmap regime; never built there).
    for &a in nodes {
        for &b in knn.neighbours(a) {
            if members.contains(&b) {
                pairs.push((a.uuid(), b.uuid()));
            }
        }
    }

    // term 2 — a shared (path, value) facet. Group by the facet key and pair up its owners: those are
    // precisely the pairs `facet_overlap` can score above zero. A key held by many owners yields many
    // pairs — but those pairs genuinely ARE nonzero, so this is the true relation, not waste, and it
    // can never exceed the dense scan it replaces.
    let mut by_key: HashMap<(&str, &str), Vec<ResourceId>> = HashMap::new();
    for f in facets {
        if members.contains(&f.owner) {
            by_key
                .entry((f.path.as_str(), f.value.as_str()))
                .or_default()
                .push(f.owner);
        }
    }
    for owners in by_key.values_mut() {
        owners.sort_unstable();
        owners.dedup();
        for i in 0..owners.len() {
            for j in (i + 1)..owners.len() {
                pairs.push((owners[i].uuid(), owners[j].uuid()));
            }
        }
    }

    CandidatePairs::from_pairs(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ids() -> (ResourceId, ResourceId) {
        (
            ResourceId::from(Uuid::from_u128(1)),
            ResourceId::from(Uuid::from_u128(2)),
        )
    }

    /// The declared-only regime's graph: empty. This is what `substrate::load` hands the producer
    /// whenever `w_cos == 0.0` — it never even runs the embedding query.
    fn no_knn() -> KnnGraph {
        KnnGraph::default()
    }

    #[test]
    fn edge_affinity_is_lens_weighted_kind_times_weight() {
        let (a, b) = ids();
        let lens = Lens {
            w_leads_to: 0.6,
            w_prop: 0.4,
            ..Lens::telos_default()
        };
        let edges = vec![Edge {
            src: a,
            tgt: b,
            kind: EdgeKind::LeadsTo,
            weight: 0.8,
            label: None,
        }];
        // 0.6 * 0.8 * label_factor(None)=1.0 = 0.48
        assert!((affinity(a, b, &edges, &[], &no_knn(), &lens) - 0.48).abs() < 1e-9);
    }

    #[test]
    fn no_declared_edge_no_facet_means_zero_affinity() {
        let (a, b) = ids();
        assert_eq!(
            affinity(a, b, &[], &[], &no_knn(), &Lens::telos_default()),
            0.0
        );
    }

    #[test]
    fn w_cos_zero_reproduces_the_declared_only_kernel() {
        // THE REGRESSION FLOOR, at unit grain. A cogmap lens must be BLIND to the embedding term —
        // even for a pair the kNN graph rates as near-identical. If this goes red, the kernel has
        // leaked into the declared-only path and every cogmap in production re-clusters.
        let (a, b) = ids();
        let lens = Lens::telos_default(); // w_cos == 0.0
        let knn = KnnGraph::from_pairs(&[(a, b, 0.99)]); // maximal similarity
        assert_eq!(
            affinity(a, b, &[], &[], &knn, &lens),
            0.0,
            "with w_cos=0 a near-identical pair must still have zero affinity"
        );
    }

    #[test]
    fn w_cos_contributes_the_knn_similarity_when_the_pair_is_a_neighbour() {
        let (a, b) = ids();
        let lens = Lens {
            w_cos: 1.0,
            ..Lens::telos_default()
        };
        let knn = KnnGraph::from_pairs(&[(a, b, 0.8)]);
        assert!((affinity(a, b, &[], &[], &knn, &lens) - 0.8).abs() < 1e-9);
    }

    #[test]
    fn a_pair_outside_the_knn_graph_contributes_nothing_however_similar() {
        // Sparsity is the whole point: cosine is DENSE, so only the top-k above the floor may
        // contribute. Otherwise the affinity graph is complete and connected_components is useless.
        let (a, b) = ids();
        let lens = Lens {
            w_cos: 1.0,
            ..Lens::telos_default()
        };
        let knn = KnnGraph::from_pairs(&[]); // b is not among a's neighbours
        assert_eq!(affinity(a, b, &[], &[], &knn, &lens), 0.0);
    }

    #[test]
    fn declared_edges_and_cosine_are_additive_not_exclusive() {
        // The context regime: cosine is primary, the declared graph is weak supervision ON TOP —
        // they compose, they do not compete.
        let (a, b) = ids();
        let lens = Lens {
            w_cos: 1.0,
            w_near: 0.35,
            ..Lens::telos_default()
        };
        let knn = KnnGraph::from_pairs(&[(a, b, 0.6)]);
        let edges = vec![Edge {
            src: a,
            tgt: b,
            kind: EdgeKind::Near,
            weight: 1.0,
            label: None,
        }];
        // 0.35*1.0 (declared) + 1.0*0.6 (inferred) = 0.95
        assert!((affinity(a, b, &edges, &[], &knn, &lens) - 0.95).abs() < 1e-9);
    }

    #[test]
    fn workflow_default_holds_the_deliberate_signals_at_cogmap_parity() {
        // Spec §3.2. A weight is meaning-when-present, not a frequency prior. Contexts carry zero
        // facets TODAY — that is a fact about the corpus, not about what a facet is WORTH. Zeroing
        // these would make the discipline provably unrewarded, and an information system that returns
        // no signal for signal provided gets routed around. If someone ever asserts an express edge or
        // a facet in a context, it MUST count.
        let ctx = Lens::workflow_default();
        let map = Lens::telos_default();
        assert_eq!(ctx.w_express, map.w_express);
        assert_eq!(ctx.w_contains, map.w_contains);
        assert_eq!(ctx.w_prop, map.w_prop);
        // ...and the regime switch is the ONLY thing that flips the kernel on.
        assert_eq!(ctx.w_cos, 1.0);
        assert_eq!(map.w_cos, 0.0);
    }

    #[test]
    fn facet_overlap_is_min_weighted_shared_pairs() {
        let (a, b) = ids();
        let facets = vec![
            Facet {
                owner: a,
                path: "topic".into(),
                value: "deployment".into(),
                weight: 0.9,
            },
            Facet {
                owner: b,
                path: "topic".into(),
                value: "deployment".into(),
                weight: 0.5,
            },
            Facet {
                owner: b,
                path: "phase".into(),
                value: "first-week".into(),
                weight: 1.0,
            },
        ];
        let lens = Lens {
            w_prop: 1.0,
            ..Lens::telos_default()
        };
        // shared ("topic","deployment"): min(0.9,0.5)=0.5; "phase" not shared. w_prop*0.5 = 0.5
        assert!((affinity(a, b, &[], &facets, &no_knn(), &lens) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn contradiction_label_binds_not_separates() {
        let (a, b) = ids();
        let lens = Lens {
            w_near: 1.0,
            ..Lens::telos_default()
        };
        let edges = vec![Edge {
            src: a,
            tgt: b,
            kind: EdgeKind::Near,
            weight: 1.0,
            label: Some("contradicts".into()),
        }];
        // contradiction BINDS: a labelled edge contributes its full kind-weighted affinity (no label factor)
        assert!((affinity(a, b, &edges, &[], &no_knn(), &lens) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn edge_kind_deserializes_snake_case_and_rejects_unknown() {
        assert_eq!(
            serde_yaml::from_str::<EdgeKind>("leads_to").unwrap(),
            EdgeKind::LeadsTo
        );
        assert_eq!(
            serde_yaml::from_str::<EdgeKind>("express").unwrap(),
            EdgeKind::Express
        );
        assert!(serde_yaml::from_str::<EdgeKind>("sideways").is_err());
    }
}
