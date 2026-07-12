//! Declarative YAML model for seeds, scenarios, and the system boot-seed.
//!
//! A `Seed` is the substrate template a foundational cogmap is born from — the *cogmap
//! specification*: cogmap (telos charter), `world`, `resources`, `edges`, and the names of the
//! (system-seeded) lenses it uses. Template-only: no runbook. A `Scenario` is the *assertion
//! specification*: it references a seed by path (or embeds one inline) and adds the ordered `steps`
//! runbook (materialize / emit-event / assert). Lenses are referenced by name; their weights live in
//! the system `BootSeed`. The same structs derive `schemars::JsonSchema` (gated) so the wire schemas
//! and the loader read one source of truth.

use crate::affinity::EdgeKind;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::borrow::Cow;
use std::path::Path;

fn one() -> f64 {
    1.0
}

fn home_cogmap() -> String {
    "cogmap".into()
}

/// The seed document (`tests/fixtures/seeds/*.yaml`): the shape-of-the-seed a foundational cogmap
/// is born from. `name` is the cogmap name `cogmap_genesis` registers.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct Seed {
    pub name: String,
    pub cogmap: CogmapDef,
    pub world: WorldDef,
    pub resources: Vec<ResourceDef>,
    #[serde(default)]
    pub edges: Vec<EdgeDef>,
    /// Names of (system-seeded) lenses this seed's cogmap uses; validated up front.
    pub uses_lenses: Vec<String>,
}

/// The scenario document (`tests/fixtures/scenarios/*.yaml`): a seed reference (or embed) plus the
/// ordered `steps` runbook.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct Scenario {
    pub name: String,
    pub seed: SeedRef,
    pub steps: Vec<Step>,
}

/// `seed:` is either a path to a seed document (resolved relative to the scenario file's directory)
/// or the seed embedded inline. Untagged: a YAML string is a reference, a map is an embed.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum SeedRef {
    Path(String),
    Inline(Box<Seed>),
}

impl Scenario {
    /// Resolve the seed document: parse the referenced file (against `base_dir`, the scenario
    /// file's directory) or borrow the embedded one.
    pub fn resolve_seed(&self, base_dir: &Path) -> Result<Cow<'_, Seed>> {
        match &self.seed {
            SeedRef::Inline(seed) => Ok(Cow::Borrowed(seed.as_ref())),
            SeedRef::Path(rel) => {
                let path = base_dir.join(rel);
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading seed {}", path.display()))?;
                let seed = serde_yaml::from_str(&text)
                    .with_context(|| format!("parsing seed {}", path.display()))?;
                Ok(Cow::Owned(seed))
            }
        }
    }
}

/// The system boot-seed (`tests/fixtures/seeds/system.yaml`): what any temper system needs, distinct
/// from any scenario. Loaded by `bootseed::seed_system`.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct BootSeed {
    pub event_types: Vec<String>,
    pub lenses: Vec<LensDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct CogmapDef {
    pub telos: TelosDef,
    /// profile handle (declared in `world.profiles`)
    pub owner: String,
    /// entity name (declared in `world.entities`)
    pub emitter: String,
}

/// The telos charter as real content-blocks (domain-B §1, content-block-primitive β): block-0 is the
/// `statement`, blocks 1..n are the `questions` (each `question + "\n\n" + context`), then `framing`
/// blocks situate the telos. Each block's kind is stamped as a `block_role` property
/// (`statement`/`question`/`framing`) by the persist path — see `block_specs`. The loader
/// turns this into an ordered prose list for `content::prepare_blocks`.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct TelosDef {
    pub title: String,
    pub statement: String,
    #[serde(default)]
    pub questions: Vec<QuestionDef>,
    #[serde(default)]
    pub framing: Vec<String>,
}

impl TelosDef {
    /// The charter flattened to its ordered `(role, prose)` block specs for
    /// `content::prepare_blocks`: block-0 is the statement (role `"statement"`), then each question
    /// (role `"question"`, `question + "\n\n" + context`, or just the question when context is empty),
    /// then the framing blocks (role `"framing"`). Positional by index — `seq` is assigned downstream;
    /// `role` is the `block_role` property the persist path stamps so reads distinguish the kinds.
    pub fn block_specs(&self) -> Vec<(&'static str, String)> {
        let questions: Vec<temper_core::charter::CharterQuestion> = self
            .questions
            .iter()
            .map(|q| temper_core::charter::CharterQuestion {
                question: q.question.clone(),
                context: q.context.clone(),
            })
            .collect();
        temper_core::charter::charter_block_specs(&self.statement, &questions, &self.framing)
    }
}

/// A guiding question with its situating context. `context` defaults empty so a bare
/// `{ question: ... }` is valid; a rich question-with-context naturally chunks into >1 window.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct QuestionDef {
    pub question: String,
    #[serde(default)]
    pub context: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct WorldDef {
    pub profiles: Vec<ProfileDef>,
    pub entities: Vec<EntityDef>,
}

/// The `system_access` PG enum — a profile's platform-wide access tier. Typed
/// in the YAML model so an invalid value fails at deserialization rather than at
/// the `$n::system_access` cast after the load transaction opens.
#[derive(Debug, Clone, Copy, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SystemAccess {
    None,
    Approved,
    Admin,
}

impl SystemAccess {
    /// Canonical `system_access` label, for binding behind a `::system_access` cast.
    pub fn as_sql(self) -> &'static str {
        match self {
            SystemAccess::None => "none",
            SystemAccess::Approved => "approved",
            SystemAccess::Admin => "admin",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ProfileDef {
    pub handle: String,
    pub display_name: String,
    pub system_access: SystemAccess,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct EntityDef {
    pub name: String,
    /// profile handle
    pub profile: String,
}

#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
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
    /// The kNN cosine weight — the regime switch (spec §3.1). Defaulted to 0.0 (declared-only), so
    /// every pre-kernel fixture YAML stays valid and keeps meaning exactly what it meant.
    #[serde(default)]
    pub w_cos: f64,
    #[serde(default = "default_knn_k")]
    pub knn_k: u32,
    #[serde(default = "default_cos_floor")]
    pub cos_floor: f64,
    pub s_telos: f64,
    pub s_ref: f64,
    pub s_central: f64,
    pub resolution: f64,
}

fn default_knn_k() -> u32 {
    12
}
fn default_cos_floor() -> f64 {
    0.55
}

/// Internally tagged by `do:` — serde_yaml 0.9 rejects the externally-tagged single-key-map form
/// (it wants `!Variant` tags), so the runbook discriminates on a `do` field. Each mutation variant
/// mirrors a `SeedAction` (events.rs) 1:1; `materialize`/`assert` drive + check the projection.
#[derive(Debug, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum Step {
    /// Add a concept resource to the map mid-runbook (one content-block body, optional facets). Its
    /// `key` joins the runner's key map for later steps. Home is always the scenario cogmap.
    CreateResource {
        key: String,
        #[serde(default)]
        title: Option<String>,
        origin_uri: String,
        #[serde(default)]
        doc_type: Option<String>,
        body: String,
        #[serde(default)]
        facets: Option<FacetDef>,
    },
    /// Set a resource's `facet` property (re-facet / re-weight). `resource` is a key.
    SetFacet {
        resource: String,
        values: serde_json::Map<String, serde_json::Value>,
        #[serde(default = "one")]
        weight: f64,
    },
    /// Assert a typed edge between two keyed resources (replaces the old special-cased `emit_event`).
    AssertEdge {
        from: String,
        to: String,
        kind: EdgeKind,
        #[serde(default)]
        label: Option<String>,
        #[serde(default = "one")]
        weight: f64,
    },
    /// Fold the live edge at `{from,to,kind}` coordinates (retire a relationship). The runner resolves
    /// the non-folded edge to its id, then fires `relationship_fold`.
    FoldEdge {
        from: String,
        to: String,
        kind: EdgeKind,
        #[serde(default)]
        reason: Option<String>,
    },
    /// Revise a concept resource's body prose (a content-only mutation: new chunk embeddings, no edge
    /// or facet change). Fires `block_mutated` on the resource's single body block. `resource` is a key.
    Revise {
        resource: String,
        body: String,
    },
    Materialize {
        lens: String,
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
    DriftTier {
        lens: String,
        tier: DriftTierName,
    },
}

/// The drift tier names as they appear in scenario YAML (`check: drift_tier, tier: readout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum DriftTierName {
    Fresh,
    Readout,
    Structural,
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

    const SEED_YAML: &str = r#"
name: t
cogmap: { telos: { title: T, statement: S, questions: [{ question: q1 }] }, owner: alice, emitter: "agent#1" }
world: { profiles: [{ handle: alice, display_name: Alice, system_access: approved }], entities: [{ name: "agent#1", profile: alice }] }
resources:
  - { key: a, origin_uri: "temper://c/a", home: cogmap, body: "hello", facets: { values: { phase: x } } }
  - { key: b, origin_uri: "temper://c/b", home: cogmap, body: "world" }
edges: [{ from: a, to: b, kind: leads_to, weight: 1.0 }]
uses_lenses: [L]
"#;

    const STEPS_YAML: &str = r#"
steps:
  - { do: create_resource, key: c, origin_uri: "temper://c/c", body: "a third concept" }
  - { do: set_facet, resource: c, values: { phase: x }, weight: 1.5 }
  - { do: materialize, lens: L }
  - do: assert
    checks:
      - { check: co_region, lens: L, members: [a, b], expect: true }
  - { do: assert_edge, from: b, to: a, kind: express, label: related }
  - { do: fold_edge, from: a, to: b, kind: leads_to, reason: "superseded" }
  - do: assert
    checks:
      - { check: stale, expect: true }
"#;

    #[test]
    fn deserializes_seed_and_bootseed() {
        let s: Seed = serde_yaml::from_str(SEED_YAML).unwrap();
        assert_eq!(s.uses_lenses, vec!["L".to_string()]);
        assert_eq!(s.resources.len(), 2);
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
    fn scenario_embeds_a_seed_inline() {
        // `seed:` as a map ⇒ the seed embedded inline; resolve_seed borrows it.
        let yaml = format!(
            "name: inline-test\nseed:\n{}\n{STEPS_YAML}",
            SEED_YAML
                .trim()
                .lines()
                .map(|l| format!("  {l}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let s: Scenario = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(s.steps.len(), 7);
        let seed = s.resolve_seed(Path::new("/nonexistent")).unwrap();
        assert_eq!(seed.name, "t");
        assert_eq!(seed.resources.len(), 2);
    }

    #[test]
    fn scenario_references_a_seed_file() {
        // `seed:` as a string ⇒ a path resolved against the scenario file's directory.
        let dir = std::env::temp_dir().join(format!("seed-ref-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t.seed.yaml"), SEED_YAML).unwrap();

        let yaml = format!("name: ref-test\nseed: t.seed.yaml\n{STEPS_YAML}");
        let s: Scenario = serde_yaml::from_str(&yaml).unwrap();
        assert!(matches!(s.seed, SeedRef::Path(_)));
        let seed = s.resolve_seed(&dir).unwrap();
        assert_eq!(seed.name, "t");
        assert_eq!(seed.edges.len(), 1);

        let missing = Scenario {
            name: "x".into(),
            seed: SeedRef::Path("nope.yaml".into()),
            steps: vec![],
        };
        let err = missing.resolve_seed(&dir).unwrap_err().to_string();
        assert!(err.contains("nope.yaml"), "error names the path: {err}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn telos_deserializes_questions_with_context_and_framing() {
        // The charter is real content-blocks: block-0 statement, blocks 1..n questions-with-context,
        // framing blocks. `context` defaults empty so a bare `{ question: ... }` is valid.
        let telos: TelosDef = serde_yaml::from_str(
            r#"
title: T
statement: "The telos statement, possibly multi-paragraph."
questions:
  - { question: "What transfers?", context: "Surface prior knowledge that maps onto this codebase." }
  - { question: "Smallest real change?" }
framing:
  - "This map situates first-week onboarding."
"#,
        )
        .unwrap();
        assert_eq!(telos.questions.len(), 2);
        assert_eq!(telos.questions[0].question, "What transfers?");
        assert!(telos.questions[0].context.contains("prior knowledge"));
        assert_eq!(telos.questions[1].context, ""); // defaulted
        assert_eq!(
            telos.framing,
            vec!["This map situates first-week onboarding."]
        );
    }

    #[test]
    fn telos_block_specs_tags_statement_questions_framing() {
        let telos = TelosDef {
            title: "T".into(),
            statement: "The statement.".into(),
            questions: vec![
                QuestionDef {
                    question: "Q1?".into(),
                    context: "C1.".into(),
                },
                QuestionDef {
                    question: "Q2?".into(),
                    context: String::new(),
                },
            ],
            framing: vec!["Framing one.".into()],
        };
        let specs = telos.block_specs();
        assert_eq!(
            specs,
            vec![
                ("statement", "The statement.".to_string()),
                ("question", "Q1?\n\nC1.".to_string()), // question + context joined
                ("question", "Q2?".to_string()),        // empty context ⇒ bare question
                ("framing", "Framing one.".to_string()),
            ]
        );
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
