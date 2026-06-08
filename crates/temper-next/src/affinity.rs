use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum EdgeKind {
    Express,
    Contains,
    LeadsTo,
    Near,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub src: Uuid,
    pub tgt: Uuid,
    pub kind: EdgeKind,
    pub weight: f64,
    pub label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Facet {
    pub owner: Uuid,
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
            s_telos: 0.5,
            s_ref: 0.3,
            s_central: 0.2,
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
fn facet_overlap(a: Uuid, b: Uuid, facets: &[Facet]) -> f64 {
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

/// Declared-only symmetric affinity (spec §2a). Cosine is ABSENT — it enters only as a downstream
/// readout (Plan 1 SQL), never here.
pub fn affinity(a: Uuid, b: Uuid, edges: &[Edge], facets: &[Facet], lens: &Lens) -> f64 {
    // Labels are not reserved (spec §2a): every label is ordinary positive relatedness, so
    // contradiction BINDS (shared frame), never separates. No label factor until a lens overrides.
    let edge_sum: f64 = edges
        .iter()
        .filter(|e| (e.src == a && e.tgt == b) || (e.src == b && e.tgt == a))
        .filter(|e| !e.weight.is_nan())
        .map(|e| lens.w_kind(e.kind) * e.weight)
        .sum();
    edge_sum + lens.w_prop * facet_overlap(a, b, facets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ids() -> (Uuid, Uuid) {
        (Uuid::from_u128(1), Uuid::from_u128(2))
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
        assert!((affinity(a, b, &edges, &[], &lens) - 0.48).abs() < 1e-9);
    }

    #[test]
    fn no_declared_edge_no_facet_means_zero_affinity() {
        let (a, b) = ids();
        assert_eq!(affinity(a, b, &[], &[], &Lens::telos_default()), 0.0);
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
        assert!((affinity(a, b, &[], &facets, &lens) - 0.5).abs() < 1e-9);
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
        assert!((affinity(a, b, &edges, &[], &lens) - 1.0).abs() < 1e-9);
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
