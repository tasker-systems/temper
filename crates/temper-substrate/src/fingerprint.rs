//! Per-component input fingerprints (decision §4) — the unifying mechanism for incremental
//! materialization: a content hash of exactly the inputs that determine a component's region
//! membership. Same fingerprint ⇒ provably identical membership ⇒ the prior regions can be reused;
//! different ⇒ re-cluster that component. It is simultaneously the drift signal and the cache key.

use crate::affinity::{Edge, Facet, Lens};
use crate::knn::KnnGraph;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use uuid::Uuid;

/// SHA-256 over a component's MEMBERSHIP-determining inputs: the sorted member ids, the intra-component
/// declared edges (direction-normalized — affinity is symmetric — with kind + bit-exact weight, label
/// EXCLUDED), the members' facets (path/value/bit-exact weight), the lens's affinity weights +
/// resolution, and — **in the context regime only** — the intra-component kNN pairs. Those are the
/// *only* inputs to [`crate::cluster::agglomerate`] for this component, so identical fingerprint ⇒
/// identical membership.
///
/// ## Why the kNN term is regime-conditional, and why that is not a hack
///
/// Under a declared-only lens (`w_cos == 0.0`) the kNN similarity is multiplied by zero, so it is
/// **provably not an input to formation** — and this function's contract is to hash *exactly* the
/// inputs that determine membership. Excluding it there is the honest reading of the contract, and it
/// has a second, load-bearing consequence: a cogmap's fingerprint stays **byte-identical** to its
/// pre-kernel value. Fingerprints are PERSISTED (`kb_cogmap_components.fingerprint`), so a format
/// change would make every component in production read as drifted, and the first materialize after
/// deploy would fold and recreate every region of every map — new ids, new events, for no change in
/// membership. The conditional is what buys a silent deploy.
///
/// Under a context lens (`w_cos != 0.0`) the kNN pairs derive from EMBEDDINGS, which move when
/// content moves. Omitting them would be a silent correctness bug, not a missed optimization: edit a
/// resource's body, its embedding shifts, its neighbours change, its region membership *should*
/// change — but `(members, edges, facets, lens)` are all untouched, so the fingerprint would match,
/// drift would classify the component as structurally clean, and the stale region would be **reused
/// forever**. The content-drift tier only refreshes readouts; it never re-forms.
///
/// Deliberately excluded, because they don't affect membership (and so must not bust the structural
/// signal): edge labels (they bind like any edge), and the salience weights `s_telos`/`s_ref`/
/// `s_central` (they tune readouts, the cheap readout-drift tier, not clustering). Floats are hashed
/// by their exact bit pattern so a no-op re-serialization can't perturb the hash. This is a sound
/// over-approximation: same fingerprint ⇒ provably identical membership; different ⇒ recompute to be
/// sure (a weight nudge that never crosses the merge frontier over-busts — safe, just recomputes).
pub fn component_fingerprint(
    members: &[Uuid],
    edges: &[Edge],
    facets: &[Facet],
    knn: &KnnGraph,
    lens: &Lens,
) -> String {
    let member_set: BTreeSet<Uuid> = members.iter().copied().collect();

    let mut s = String::from("m:");
    for m in &member_set {
        s.push_str(&m.to_string());
        s.push(',');
    }

    let mut es: Vec<String> = edges
        .iter()
        .filter(|e| member_set.contains(&e.src.uuid()) && member_set.contains(&e.tgt.uuid()))
        .map(|e| {
            let (a, b) = if e.src <= e.tgt {
                (e.src, e.tgt)
            } else {
                (e.tgt, e.src)
            };
            format!("{a}-{b}:{}:{:016x}", e.kind.as_sql(), e.weight.to_bits())
        })
        .collect();
    es.sort();
    s.push_str("|e:");
    for e in &es {
        s.push_str(e);
        s.push(',');
    }

    let mut fs: Vec<String> = facets
        .iter()
        .filter(|f| member_set.contains(&f.owner.uuid()))
        .map(|f| {
            format!(
                "{}:{}={}:{:016x}",
                f.owner,
                f.path,
                f.value,
                f.weight.to_bits()
            )
        })
        .collect();
    fs.sort();
    s.push_str("|f:");
    for f in &fs {
        s.push_str(f);
        s.push(',');
    }

    s.push_str(&format!(
        "|l:{:016x},{:016x},{:016x},{:016x},{:016x},r{:016x}",
        lens.w_express.to_bits(),
        lens.w_contains.to_bits(),
        lens.w_leads_to.to_bits(),
        lens.w_near.to_bits(),
        lens.w_prop.to_bits(),
        lens.resolution.to_bits(),
    ));

    // The inferred half — appended ONLY in the context regime, so a declared-only fingerprint is
    // byte-identical to its pre-kernel value (see the doc comment: prod persists these). `pairs()`
    // yields each undirected pair once in id order, so the section is deterministic.
    if lens.w_cos != 0.0 {
        let mut ks: Vec<String> = knn
            .pairs()
            .into_iter()
            .filter(|(a, b, _)| member_set.contains(&a.uuid()) && member_set.contains(&b.uuid()))
            .map(|(a, b, sim)| format!("{a}-{b}:{:016x}", sim.to_bits()))
            .collect();
        ks.sort();
        s.push_str("|k:");
        for k in &ks {
            s.push_str(k);
            s.push(',');
        }
        // The kernel params themselves: a change to k or the floor reshapes the graph, and so the
        // membership, even over an identical corpus.
        s.push_str(&format!(
            "|kp:{:016x},{},{:016x}",
            lens.w_cos.to_bits(),
            lens.knn_k,
            lens.cos_floor.to_bits(),
        ));
    }

    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::affinity::{Edge, EdgeKind, Facet, Lens};
    use crate::ids::ResourceId;
    use uuid::Uuid;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn rid(n: u128) -> ResourceId {
        ResourceId::from(id(n))
    }

    fn edge(src: u128, tgt: u128, kind: EdgeKind, weight: f64, label: Option<&str>) -> Edge {
        Edge {
            src: rid(src),
            tgt: rid(tgt),
            kind,
            weight,
            label: label.map(str::to_owned),
        }
    }

    fn facet(owner: u128, path: &str, value: &str, weight: f64) -> Facet {
        Facet {
            owner: rid(owner),
            path: path.to_owned(),
            value: value.to_owned(),
            weight,
        }
    }

    /// The declared-only regime's graph: empty, and never consulted (`w_cos == 0.0`).
    fn no_knn() -> KnnGraph {
        KnnGraph::default()
    }

    /// A context lens — the only regime in which the kNN pairs enter the hash.
    fn ctx_lens() -> Lens {
        Lens::workflow_default()
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let members = [id(1), id(2)];
        let edges = [edge(1, 2, EdgeKind::Near, 0.7, None)];
        let facets = [facet(1, "layer", "persona", 1.0)];
        let lens = Lens::telos_default();
        let a = component_fingerprint(&members, &edges, &facets, &no_knn(), &lens);
        let b = component_fingerprint(&members, &edges, &facets, &no_knn(), &lens);
        assert_eq!(a, b);
    }

    #[test]
    fn intra_component_edge_weight_change_busts_the_fingerprint() {
        let members = [id(1), id(2)];
        let lens = Lens::telos_default();
        let base = component_fingerprint(
            &members,
            &[edge(1, 2, EdgeKind::Near, 0.7, None)],
            &[],
            &no_knn(),
            &lens,
        );
        let moved = component_fingerprint(
            &members,
            &[edge(1, 2, EdgeKind::Near, 0.3, None)],
            &[],
            &no_knn(),
            &lens,
        );
        assert_ne!(
            base, moved,
            "a within-component affinity change must bust the fingerprint"
        );
    }

    #[test]
    fn edge_outside_the_component_does_not_affect_the_fingerprint() {
        // component {1,2}; an edge 1—3 leaves the component (3 ∉ members) ⇒ does not enter affinity
        // among members ⇒ must not change the fingerprint.
        let members = [id(1), id(2)];
        let lens = Lens::telos_default();
        let without = component_fingerprint(
            &members,
            &[edge(1, 2, EdgeKind::Near, 0.7, None)],
            &[],
            &no_knn(),
            &lens,
        );
        let with_outside = component_fingerprint(
            &members,
            &[
                edge(1, 2, EdgeKind::Near, 0.7, None),
                edge(1, 3, EdgeKind::LeadsTo, 0.9, None),
            ],
            &[],
            &no_knn(),
            &lens,
        );
        assert_eq!(without, with_outside);
    }

    #[test]
    fn edge_label_does_not_affect_the_fingerprint() {
        // labels are not affinity-relevant (they bind like any edge), so a label-only change must not
        // bust the structural fingerprint.
        let members = [id(1), id(2)];
        let lens = Lens::telos_default();
        let unlabelled = component_fingerprint(
            &members,
            &[edge(1, 2, EdgeKind::Near, 0.7, None)],
            &[],
            &no_knn(),
            &lens,
        );
        let labelled = component_fingerprint(
            &members,
            &[edge(1, 2, EdgeKind::Near, 0.7, Some("contends-with"))],
            &[],
            &no_knn(),
            &lens,
        );
        assert_eq!(unlabelled, labelled);
    }

    #[test]
    fn membership_relevant_lens_weight_busts_but_salience_weight_does_not() {
        let members = [id(1), id(2)];
        let edges = [edge(1, 2, EdgeKind::LeadsTo, 0.8, None)];
        let base = component_fingerprint(&members, &edges, &[], &no_knn(), &Lens::telos_default());

        // w_leads_to changes affinity ⇒ membership-relevant ⇒ busts.
        let reweighted = component_fingerprint(
            &members,
            &edges,
            &[],
            &no_knn(),
            &Lens {
                w_leads_to: 0.1,
                ..Lens::telos_default()
            },
        );
        assert_ne!(
            base, reweighted,
            "an affinity weight change must bust the fingerprint"
        );

        // s_telos tunes the salience readout, NOT membership ⇒ must NOT bust.
        let resalienced = component_fingerprint(
            &members,
            &edges,
            &[],
            &no_knn(),
            &Lens {
                s_telos: 0.9,
                ..Lens::telos_default()
            },
        );
        assert_eq!(
            base, resalienced,
            "a salience weight change must not bust the structural fingerprint"
        );
    }

    #[test]
    fn member_facet_change_busts_the_fingerprint() {
        let members = [id(1), id(2)];
        let lens = Lens::telos_default();
        let base = component_fingerprint(
            &members,
            &[],
            &[facet(1, "layer", "persona", 1.0)],
            &no_knn(),
            &lens,
        );
        let reweighted = component_fingerprint(
            &members,
            &[],
            &[facet(1, "layer", "persona", 0.5)],
            &no_knn(),
            &lens,
        );
        assert_ne!(base, reweighted);
    }

    // ── the kernel: the kNN half of the hash ─────────────────────────────────────────────────────

    #[test]
    fn a_declared_only_lens_ignores_the_knn_graph_entirely() {
        // THE DEPLOY-SAFETY PROPERTY. Fingerprints are PERSISTED in kb_cogmap_components. If a cogmap's
        // fingerprint moved when the kernel shipped, every component in production would read as
        // drifted and the first materialize after deploy would fold and recreate every region of every
        // map — new ids, new events — for zero change in membership. At w_cos = 0 the kNN is provably
        // not an input (it is multiplied by zero), so it must not enter the hash AT ALL: not its pairs,
        // not its params, not even an empty section marker.
        let members = [id(1), id(2)];
        let lens = Lens::telos_default(); // w_cos == 0.0
        let loaded = KnnGraph::from_pairs(&[(rid(1), rid(2), 0.99)]);
        assert_eq!(
            component_fingerprint(&members, &[], &[], &no_knn(), &lens),
            component_fingerprint(&members, &[], &[], &loaded, &lens),
            "a declared-only lens must hash identically however the kNN graph is populated"
        );
    }

    #[test]
    fn a_context_lens_busts_when_an_intra_component_knn_similarity_moves() {
        // THE INVALIDATION BUG THIS FIXES. Under w_cos > 0 the kNN pairs derive from EMBEDDINGS, which
        // move when content moves. If they were absent from the hash, editing a resource's body would
        // shift its embedding, change its neighbours, change the membership it SHOULD have — while
        // (members, edges, facets, lens) all stayed identical. The fingerprint would match, drift would
        // call the component structurally clean, and the stale region would be reused forever.
        let members = [id(1), id(2)];
        let lens = ctx_lens();
        let before = component_fingerprint(
            &members,
            &[],
            &[],
            &KnnGraph::from_pairs(&[(rid(1), rid(2), 0.90)]),
            &lens,
        );
        let after = component_fingerprint(
            &members,
            &[],
            &[],
            &KnnGraph::from_pairs(&[(rid(1), rid(2), 0.60)]),
            &lens,
        );
        assert_ne!(
            before, after,
            "an embedding-driven affinity change must bust the fingerprint in the context regime"
        );
    }

    #[test]
    fn a_context_lens_busts_when_a_knn_pair_appears_or_vanishes() {
        // The sharper case: the pair drops below cos_floor and leaves the graph entirely. Membership
        // can change (the component may split), so the fingerprint MUST move.
        let members = [id(1), id(2)];
        let lens = ctx_lens();
        let linked = component_fingerprint(
            &members,
            &[],
            &[],
            &KnnGraph::from_pairs(&[(rid(1), rid(2), 0.90)]),
            &lens,
        );
        let severed = component_fingerprint(&members, &[], &[], &KnnGraph::from_pairs(&[]), &lens);
        assert_ne!(
            linked, severed,
            "losing the only kNN link must bust the hash"
        );
    }

    #[test]
    fn a_knn_pair_outside_the_component_does_not_affect_the_fingerprint() {
        // Same scoping rule the declared edges obey: a pair with an endpoint outside the component
        // cannot affect affinity AMONG members, so it must not bust their fingerprint.
        let members = [id(1), id(2)];
        let lens = ctx_lens();
        let within = component_fingerprint(
            &members,
            &[],
            &[],
            &KnnGraph::from_pairs(&[(rid(1), rid(2), 0.9)]),
            &lens,
        );
        let with_outside = component_fingerprint(
            &members,
            &[],
            &[],
            &KnnGraph::from_pairs(&[(rid(1), rid(2), 0.9), (rid(1), rid(3), 0.8)]),
            &lens,
        );
        assert_eq!(within, with_outside);
    }

    #[test]
    fn context_lens_kernel_params_are_membership_relevant() {
        // k and the floor reshape the graph, and so the membership, over an identical corpus. They are
        // formation inputs, not readout tuning — they must bust.
        let members = [id(1), id(2)];
        let knn = KnnGraph::from_pairs(&[(rid(1), rid(2), 0.9)]);
        let base = component_fingerprint(&members, &[], &[], &knn, &ctx_lens());

        for tweaked in [
            Lens {
                knn_k: 3,
                ..ctx_lens()
            },
            Lens {
                cos_floor: 0.8,
                ..ctx_lens()
            },
            Lens {
                w_cos: 0.5,
                ..ctx_lens()
            },
        ] {
            assert_ne!(
                base,
                component_fingerprint(&members, &[], &[], &knn, &tweaked),
                "a kernel param change must bust the fingerprint"
            );
        }
    }
}
