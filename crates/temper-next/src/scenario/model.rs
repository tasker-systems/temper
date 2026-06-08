//! Declarative YAML model for scenarios + the system boot-seed.
//!
//! A `Scenario` is a substrate template (`cogmap`/`world`/`resources`/`edges`) plus an ordered
//! `steps` runbook (materialize / emit-event / assert) — the *cogmap specification* fused with the
//! *assertion specification*. Lenses are referenced by name (`uses_lenses`); their weights live in the
//! system `BootSeed`. The same structs derive `schemars::JsonSchema` (gated) so the wire schema and the
//! loader read one source of truth.

use crate::affinity::EdgeKind;
use serde::Deserialize;

fn one() -> f64 {
    1.0
}

fn home_cogmap() -> String {
    "cogmap".into()
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct Scenario {
    pub name: String,
    pub cogmap: CogmapDef,
    pub world: WorldDef,
    pub resources: Vec<ResourceDef>,
    #[serde(default)]
    pub edges: Vec<EdgeDef>,
    /// Names of (system-seeded) lenses this scenario uses; validated up front.
    pub uses_lenses: Vec<String>,
    pub steps: Vec<Step>,
}

/// The system boot-seed (`schema-artifact/seeds/system.yaml`): what any temper system needs, distinct
/// from any scenario. Loaded by `bootseed::seed_system`.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct BootSeed {
    pub event_types: Vec<String>,
    pub lenses: Vec<LensDef>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct CogmapDef {
    pub telos: TelosDef,
    /// profile handle (declared in `world.profiles`)
    pub owner: String,
    /// entity name (declared in `world.entities`)
    pub emitter: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct TelosDef {
    pub title: String,
    pub statement: String,
    #[serde(default)]
    pub questions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct WorldDef {
    pub profiles: Vec<ProfileDef>,
    pub entities: Vec<EntityDef>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ProfileDef {
    pub handle: String,
    pub display_name: String,
    /// 'none' | 'approved' | 'admin'
    pub system_access: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct EntityDef {
    pub name: String,
    /// profile handle
    pub profile: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ResourceDef {
    /// local stable id — edges/asserts reference this
    pub key: String,
    #[serde(default)]
    pub title: Option<String>,
    pub origin_uri: String,
    #[serde(default = "home_cogmap")]
    pub home: String,
    #[serde(default)]
    pub doc_type: Option<String>,
    pub body: String,
    #[serde(default)]
    pub facets: Option<FacetDef>,
}

/// One `property_key='facet'` row per resource. `values` is the coherent multi-key JSONB object
/// (scalar or array values); `weight` applies to every `(path,value)` pair it expands to. A bare map
/// is sugar: `facets: { phase: x }` == `{ values: { phase: x }, weight: 1.0 }`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum FacetDef {
    Explicit {
        values: serde_json::Map<String, serde_json::Value>,
        #[serde(default = "one")]
        weight: f64,
    },
    Bare(serde_json::Map<String, serde_json::Value>),
}

impl FacetDef {
    pub fn values(&self) -> &serde_json::Map<String, serde_json::Value> {
        match self {
            FacetDef::Explicit { values, .. } => values,
            FacetDef::Bare(v) => v,
        }
    }
    pub fn weight(&self) -> f64 {
        match self {
            FacetDef::Explicit { weight, .. } => *weight,
            FacetDef::Bare(_) => 1.0,
        }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct EdgeDef {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "one")]
    pub weight: f64,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct LensDef {
    pub name: String,
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

/// Internally tagged by `do:` — serde_yaml 0.9 rejects the externally-tagged single-key-map form
/// (it wants `!Variant` tags), so the runbook discriminates on a `do` field.
#[derive(Debug, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum Step {
    Materialize {
        lens: String,
    },
    EmitEvent {
        #[serde(rename = "type")]
        event_type: String,
        edges: Vec<EdgeDef>,
    },
    Assert {
        checks: Vec<Expectation>,
    },
}

/// Internally tagged by `check:` (same serde_yaml constraint as `Step`).
#[derive(Debug, Deserialize)]
#[serde(tag = "check", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum Expectation {
    RegionCount {
        lens: String,
        op: CmpOp,
        value: i64,
    },
    CoRegion {
        lens: String,
        members: Vec<String>,
        expect: bool,
    },
    CohesionOrder {
        lens: String,
        greater: String,
        lesser: String,
    },
    RegionSize {
        lens: String,
        member: String,
        value: i64,
    },
    InternalTension {
        lens: String,
        member: String,
        op: CmpOp,
        value: f64,
    },
    Reproducible {
        lens: String,
    },
    FingerprintDiffers {
        lens_a: String,
        lens_b: String,
    },
    Stale {
        expect: bool,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum CmpOp {
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = "==")]
    Eq,
}

impl CmpOp {
    pub fn cmp_f64(self, a: f64, b: f64) -> bool {
        match self {
            CmpOp::Ge => a >= b,
            CmpOp::Gt => a > b,
            CmpOp::Eq => (a - b).abs() < f64::EPSILON,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_scenario_and_bootseed() {
        let scenario_yaml = r#"
name: t
cogmap: { telos: { title: T, statement: S, questions: [q1] }, owner: alice, emitter: "agent#1" }
world: { profiles: [{ handle: alice, display_name: Alice, system_access: approved }], entities: [{ name: "agent#1", profile: alice }] }
resources:
  - { key: a, origin_uri: "temper://c/a", home: cogmap, body: "hello", facets: { values: { phase: x } } }
  - { key: b, origin_uri: "temper://c/b", home: cogmap, body: "world" }
edges: [{ from: a, to: b, kind: leads_to, weight: 1.0 }]
uses_lenses: [L]
steps:
  - { do: materialize, lens: L }
  - do: assert
    checks:
      - { check: co_region, lens: L, members: [a, b], expect: true }
  - { do: emit_event, type: relationship_asserted, edges: [{ from: b, to: a, kind: express, label: related }] }
  - do: assert
    checks:
      - { check: stale, expect: true }
"#;
        let s: Scenario = serde_yaml::from_str(scenario_yaml).unwrap();
        assert_eq!(s.uses_lenses, vec!["L".to_string()]);
        assert_eq!(s.resources.len(), 2);
        assert_eq!(s.steps.len(), 4);
        assert!(s.resources[1].facets.is_none());
        assert_eq!(s.resources[0].facets.as_ref().unwrap().weight(), 1.0);

        let boot: BootSeed = serde_yaml::from_str(
            "event_types: [resource_created, lens_created]\nlenses:\n  - { name: L, w_express: 1.0, w_contains: 1.0, w_leads_to: 0.6, w_near: 0.3, w_prop: 0.4, s_telos: 0.5, s_ref: 0.3, s_central: 0.2, resolution: 0.5 }\n",
        )
        .unwrap();
        assert_eq!(boot.lenses.len(), 1);
        assert_eq!(boot.event_types.len(), 2);
    }

    #[test]
    fn facet_bare_map_is_sugar_for_explicit() {
        let bare: FacetDef = serde_yaml::from_str("phase: first-week\n").unwrap();
        assert_eq!(bare.weight(), 1.0);
        assert_eq!(bare.values().get("phase").unwrap(), "first-week");
        let explicit: FacetDef =
            serde_yaml::from_str("values: { topic: deployment }\nweight: 1.5\n").unwrap();
        assert_eq!(explicit.weight(), 1.5);
    }

    #[test]
    fn rejects_unknown_edge_kind() {
        assert!(
            serde_yaml::from_str::<EdgeDef>("from: a\nto: b\nkind: sideways\nweight: 1.0\n")
                .is_err()
        );
    }
}
