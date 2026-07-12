//! Graded, lens-relative, component-scoped drift detection (decision §1–§4). Replaces binary
//! event-recency staleness with **fingerprint comparison**: which components' membership inputs
//! actually changed, classified into the two tiers so refresh is scoped — re-cluster only the changed
//! components (expensive), or re-run the SQL readouts over fixed membership (cheap). The boolean
//! "is it stale" stays derivable (any non-`Fresh` tier ⇒ stale), so this is backwards-observable.

use crate::affinity::{affinity, candidate_pairs};
use crate::cluster::connected_components;
use crate::fingerprint::component_fingerprint;
use crate::substrate::Substrate;
use anyhow::Result;
use sqlx::{PgPool, Row};
use std::collections::{HashMap, HashSet};
use temper_core::types::home::HomeAnchor;
use uuid::Uuid;

/// The tier of refresh a (cogmap, lens) needs (decision §1). Ordered cheap → expensive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DriftTier {
    /// No event since this lens last materialized — the regions are current.
    Fresh,
    /// Something changed but no component's membership inputs did: only the SQL readouts (centroid →
    /// cohesion/alignment; provenance → reference_standing) need re-running over fixed membership.
    Readout,
    /// At least one component's membership inputs changed, or scope changed — those components must be
    /// re-clustered. A lens-config edit lands here too (it shifts every component's fingerprint).
    Structural,
}

/// The component-level diff between the live substrate and the persisted components: which persisted
/// components are reused untouched, which current member-sets must be (re-)clustered, which persisted
/// components are now stale. By fingerprint comparison — same (member-set, fingerprint) ⇒ provably
/// unchanged membership.
#[derive(Clone, Debug, Default)]
pub struct ComponentDiff {
    /// persisted component ids reused as-is.
    pub unchanged: Vec<Uuid>,
    /// current component member-sets needing (re-)cluster — new, split, bridged, or reweighted.
    pub changed: Vec<Vec<Uuid>>,
    /// persisted component ids whose inputs no longer match any current component (fold them).
    pub stale: Vec<Uuid>,
}

impl ComponentDiff {
    /// Any membership-level change — the gate between the readout and structural tiers.
    pub fn has_structural_change(&self) -> bool {
        !self.changed.is_empty() || !self.stale.is_empty()
    }
}

/// Classify current components (member-set + fingerprint) against the persisted live components.
/// Pure. Components are a disjoint partition on each side, so a member-set is unique within each side
/// and the (member-set, fingerprint) match is unambiguous.
pub fn classify(
    current: &[(Vec<Uuid>, String)],
    priors: &[(Uuid, Vec<Uuid>, String)],
) -> ComponentDiff {
    let prior_by_key: HashMap<(Vec<Uuid>, String), Uuid> = priors
        .iter()
        .map(|(id, members, fp)| ((members.clone(), fp.clone()), *id))
        .collect();
    let mut matched: HashSet<Uuid> = HashSet::new();
    let mut diff = ComponentDiff::default();
    for (members, fp) in current {
        if let Some(pid) = prior_by_key.get(&(members.clone(), fp.clone())) {
            matched.insert(*pid);
            diff.unchanged.push(*pid);
        } else {
            diff.changed.push(members.clone());
        }
    }
    diff.stale = priors
        .iter()
        .map(|(id, _, _)| *id)
        .filter(|id| !matched.contains(id))
        .collect();
    diff
}

/// Two-tier classification (decision §1), given whether any component changed structurally and whether
/// any event touched the cogmap since this lens last materialized. Pure — the decision table itself.
pub fn tier(has_structural_change: bool, touched_since_materialize: bool) -> DriftTier {
    if has_structural_change {
        DriftTier::Structural
    } else if touched_since_materialize {
        DriftTier::Readout
    } else {
        DriftTier::Fresh
    }
}

/// The live (non-folded) persisted components for a lens: (id, sorted member ids, fingerprint) — the
/// diff basis shared by drift detection and incremental materialization.
pub(crate) async fn live_components(
    pool: &PgPool,
    anchor: HomeAnchor,
    lens_id: Uuid,
) -> Result<Vec<(Uuid, Vec<Uuid>, String)>> {
    let rows = sqlx::query(
        "SELECT id, member_ids, fingerprint FROM kb_cogmap_components \
         WHERE home_anchor_table=$1 AND home_anchor_id=$2 AND lens_id=$3 AND NOT is_folded",
    )
    .bind(anchor.table())
    .bind(anchor.uuid())
    .bind(lens_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get::<Uuid, _>("id"),
                r.get::<Vec<Uuid>, _>("member_ids"),
                r.get::<String, _>("fingerprint"),
            )
        })
        .collect())
}

/// The current nonzero-affinity components of a substrate as (sorted member ids, fingerprint) — the
/// lighter sibling of `write::cluster_components` (no agglomeration; drift only needs the fingerprints).
pub(crate) fn current_component_fingerprints(s: &Substrate) -> Vec<(Vec<Uuid>, String)> {
    // `affinity`/`cluster` bridge: feed the opaque node uuids, lift each pair to `ResourceId` (mirrors
    // `write::cluster_components`).
    let aff = |x: Uuid, y: Uuid| affinity(x.into(), y.into(), &s.edges, &s.facets, &s.knn, &s.lens);
    let node_uuids: Vec<Uuid> = s.nodes.iter().map(|n| n.uuid()).collect();
    let candidates = candidate_pairs(&s.nodes, &s.edges, &s.facets, &s.knn);
    connected_components(&node_uuids, &candidates, &aff)
        .into_iter()
        .map(|members| {
            let fp = component_fingerprint(&members, &s.edges, &s.facets, &s.knn, &s.lens);
            (members, fp)
        })
        .collect()
}

/// True iff any formation-or-content event touched the anchor since this lens last materialized — the
/// readout-tier gate. (Structural changes also touch, so callers check structural drift FIRST.) Always
/// false before the first materialize, where structural drift dominates anyway.
async fn touched_since_last_materialize(
    pool: &PgPool,
    anchor: HomeAnchor,
    lens_id: Uuid,
) -> Result<bool> {
    // The payload probe (and its pre-T3 `cogmap_id` fallback) is shared with write.rs's copy — see
    // `replay::last_materialize_event`. `None` bound: drift wants the latest act, full stop.
    let last = crate::replay::last_materialize_event(pool, anchor, lens_id, None).await?;
    match last {
        None => Ok(false),
        Some(watermark) => crate::replay::formation_touched_since(pool, anchor, watermark).await,
    }
}

/// The graded, lens-relative drift signal: the component-level diff (by fingerprint comparison) plus
/// its two-tier classification. Component-scoped — `diff` names exactly which components must
/// re-cluster and which are provably current, so a refresh re-clusters only the touched components
/// instead of the whole map (the over-trigger binary staleness suffered).
pub async fn lens_drift(
    pool: &PgPool,
    anchor: HomeAnchor,
    lens_name: &str,
) -> Result<(DriftTier, ComponentDiff)> {
    let s = crate::substrate::load(pool, anchor, lens_name).await?;
    let current = current_component_fingerprints(&s);
    let priors = live_components(pool, anchor, s.lens_id.uuid()).await?;
    let diff = classify(&current, &priors);
    let touched = touched_since_last_materialize(pool, anchor, s.lens_id.uuid()).await?;
    Ok((tier(diff.has_structural_change(), touched), diff))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn tier_is_fresh_only_when_nothing_touched() {
        assert_eq!(tier(false, false), DriftTier::Fresh);
        assert_eq!(tier(false, true), DriftTier::Readout);
        // structural dominates regardless of the touch flag.
        assert_eq!(tier(true, false), DriftTier::Structural);
        assert_eq!(tier(true, true), DriftTier::Structural);
    }

    #[test]
    fn classify_all_unchanged_has_no_structural_change() {
        let priors = vec![(id(100), vec![id(1), id(2)], "fpA".into())];
        let current = vec![(vec![id(1), id(2)], "fpA".to_string())];
        let diff = classify(&current, &priors);
        assert_eq!(diff.unchanged, vec![id(100)]);
        assert!(diff.changed.is_empty());
        assert!(diff.stale.is_empty());
        assert!(!diff.has_structural_change());
    }

    #[test]
    fn classify_reweighted_component_same_members_different_fingerprint() {
        // an intra-component affinity change keeps the member set but busts the fingerprint: the
        // current component is `changed`, the prior is `stale`, and it IS a structural change.
        let priors = vec![(id(100), vec![id(1), id(2)], "fpOld".into())];
        let current = vec![(vec![id(1), id(2)], "fpNew".to_string())];
        let diff = classify(&current, &priors);
        assert!(diff.unchanged.is_empty());
        assert_eq!(diff.changed, vec![vec![id(1), id(2)]]);
        assert_eq!(diff.stale, vec![id(100)]);
        assert!(diff.has_structural_change());
    }

    #[test]
    fn classify_isolates_the_touched_component_and_reuses_the_rest() {
        // two prior components; only the second's inputs changed. The first is provably unchanged.
        let priors = vec![
            (id(100), vec![id(1), id(2)], "personaFp".into()),
            (id(200), vec![id(3), id(4)], "commitFpOld".into()),
        ];
        let current = vec![
            (vec![id(1), id(2)], "personaFp".to_string()),
            (vec![id(3), id(4)], "commitFpNew".to_string()),
        ];
        let diff = classify(&current, &priors);
        assert_eq!(diff.unchanged, vec![id(100)]);
        assert_eq!(diff.changed, vec![vec![id(3), id(4)]]);
        assert_eq!(diff.stale, vec![id(200)]);
    }

    #[test]
    fn classify_new_component_is_changed_priors_untouched() {
        let priors = vec![(id(100), vec![id(1), id(2)], "fpA".into())];
        let current = vec![
            (vec![id(1), id(2)], "fpA".to_string()),
            (vec![id(9)], "fpNew".to_string()),
        ];
        let diff = classify(&current, &priors);
        assert_eq!(diff.unchanged, vec![id(100)]);
        assert_eq!(diff.changed, vec![vec![id(9)]]);
        assert!(diff.stale.is_empty());
        assert!(diff.has_structural_change());
    }

    #[test]
    fn classify_bridged_components_union_into_one_changed_both_priors_stale() {
        // a new bridging edge unions two prior components into one current component: the union is
        // `changed`, and BOTH priors are `stale`.
        let priors = vec![
            (id(100), vec![id(1), id(2)], "fp1".into()),
            (id(200), vec![id(3), id(4)], "fp2".into()),
        ];
        let current = vec![(vec![id(1), id(2), id(3), id(4)], "fpUnion".to_string())];
        let diff = classify(&current, &priors);
        assert!(diff.unchanged.is_empty());
        assert_eq!(diff.changed, vec![vec![id(1), id(2), id(3), id(4)]]);
        assert_eq!(diff.stale.len(), 2);
        assert!(diff.stale.contains(&id(100)) && diff.stale.contains(&id(200)));
    }
}
