//! Per-component input fingerprints (decision §4) — the unifying mechanism for incremental
//! materialization: a content hash of exactly the inputs that determine a component's region
//! membership. Same fingerprint ⇒ provably identical membership ⇒ the prior regions can be reused;
//! different ⇒ re-cluster that component. It is simultaneously the drift signal and the cache key.

use crate::affinity::{Edge, Facet, Lens};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use uuid::Uuid;

/// SHA-256 over a component's MEMBERSHIP-determining inputs: the sorted member ids, the intra-component
/// declared edges (direction-normalized — affinity is symmetric — with kind + bit-exact weight, label
/// EXCLUDED), the members' facets (path/value/bit-exact weight), and the lens's affinity weights +
/// resolution. Because formation is declared-only (no cosine), these are the *only* inputs to
/// [`crate::cluster::agglomerate`] for this component, so identical fingerprint ⇒ identical membership.
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

    #[test]
    fn fingerprint_is_deterministic() {
        let members = [id(1), id(2)];
        let edges = [edge(1, 2, EdgeKind::Near, 0.7, None)];
        let facets = [facet(1, "layer", "persona", 1.0)];
        let lens = Lens::telos_default();
        let a = component_fingerprint(&members, &edges, &facets, &lens);
        let b = component_fingerprint(&members, &edges, &facets, &lens);
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
            &lens,
        );
        let moved = component_fingerprint(
            &members,
            &[edge(1, 2, EdgeKind::Near, 0.3, None)],
            &[],
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
            &lens,
        );
        let with_outside = component_fingerprint(
            &members,
            &[
                edge(1, 2, EdgeKind::Near, 0.7, None),
                edge(1, 3, EdgeKind::LeadsTo, 0.9, None),
            ],
            &[],
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
            &lens,
        );
        let labelled = component_fingerprint(
            &members,
            &[edge(1, 2, EdgeKind::Near, 0.7, Some("contends-with"))],
            &[],
            &lens,
        );
        assert_eq!(unlabelled, labelled);
    }

    #[test]
    fn membership_relevant_lens_weight_busts_but_salience_weight_does_not() {
        let members = [id(1), id(2)];
        let edges = [edge(1, 2, EdgeKind::LeadsTo, 0.8, None)];
        let base = component_fingerprint(&members, &edges, &[], &Lens::telos_default());

        // w_leads_to changes affinity ⇒ membership-relevant ⇒ busts.
        let reweighted = component_fingerprint(
            &members,
            &edges,
            &[],
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
        let base =
            component_fingerprint(&members, &[], &[facet(1, "layer", "persona", 1.0)], &lens);
        let reweighted =
            component_fingerprint(&members, &[], &[facet(1, "layer", "persona", 0.5)], &lens);
        assert_ne!(base, reweighted);
    }
}
