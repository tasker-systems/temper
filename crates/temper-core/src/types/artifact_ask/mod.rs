//! Asking data artifacts by what they are — the pure half (spec
//! `temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md` §2.1, §2.4,
//! §3.1–§3.6).
//!
//! An **ask** names one governing shape by id and predicates over paths that shape declares. This
//! module decides, with no I/O and no JSON Schema validator, whether an ask is expressible against
//! its shape, and teaches the shape's own vocabulary when it is not:
//!
//! - [`walk`] — the field-tree schema walk: what a schema declares at one path ([`walk::resolve`]),
//!   and every askable path up to the bounds ([`walk::enumerate`]);
//! - [`validate`] — refusal step 4 of §3.5, with its shape-free half [`validate_ask_form`]; the
//!   steps 1–3 refusals a [`ShapeLoad`] carries; the [`RefusalTeaching`] a refusal carries;
//! - [`session`] — [`AskSession`], the one way an ask is validated: one request's asks against one
//!   [`ShapeContext`], each shape's walk built once and one membership budget for them all, and
//!   each shape's [`validate::PathTeaching`] made once;
//! - [`mod@reference`] — the Rust reference evaluator the SQL is differential-tested against;
//! - [`batch`] — the batch read's wire ([`AskRequest`], [`AskResponse`]) and the refusals a page
//!   earns without its shape.
//!
//! **Two halves, one seam** (§2.4). The loader in `temper-services` holds the pool, the principal
//! and the `jsonschema` compiler, and turns each named [`ShapeId`] into a [`ShapeLoad`]. Everything
//! after that is here, so the composable arm's gate and seal and the batch read share one
//! definition of what can be asked.

pub mod batch;
pub mod reference;
pub mod session;
pub mod validate;
pub mod walk;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::ids::ShapeId;
use crate::types::query::filter::property_value_bytes;
use crate::types::query::PropertyOp;

pub use batch::{AskOrder, AskRequest, AskResponse, AskedShape, OversizedArtifact};
pub use reference::matches;
pub use session::AskSession;
pub use validate::{
    validate_ask_form, RefusalTeaching, TeachingElision, ValidatedAsk, MAX_ARTIFACT_PATH_DEPTH,
    MAX_ARTIFACT_PATH_SEGMENT_BYTES, MAX_ASK_PREDICATES, MAX_ASK_PROBES, MAX_MEMBERSHIP_STEPS,
    MAX_TEACHING_BYTES,
};
pub use walk::{
    enumerate, is_scalar, reads_whole, resolve, within_schema_bound, AskOperator, AskablePath,
    BoundKind, JsonType, PathFacts, PathResolution, Vocabulary, WalkBounds,
};

/// The selection intents an ask may narrow by — the closed vocabulary of
/// `kb_data_artifacts.intent` (`20260820000020`).
pub const ARTIFACT_INTENTS: [&str; 3] = ["current", "member", "pinned"];

/// The intents an ask admits when it names none (§2.1). `pinned` is *"never auto-selected"*
/// (`20260820000020`), so an ask reaches a pinned artifact only by naming the intent.
pub const DEFAULT_ARTIFACT_INTENTS: [&str; 2] = ["current", "member"];

/// The shape states an ask may narrow by, and its default when it names none (§2.1, §5) — every
/// state an artifact under a live shape can be in. `never_declared`, the fourth member of
/// `temper_substrate::payloads::ShapeState`, is unreachable under a live shape and refused as such.
pub const ASKABLE_SHAPE_STATES: [&str; 3] = [
    "declared_satisfied",
    "declared_not_satisfied",
    "declared_not_yet_checked",
];

/// One ask: a governing shape, and what the asker holds about artifacts it governs (§2.1).
///
/// `intent` and `shape_states` are **strings**, validated by [`AskSession::validate`] into
/// `unknown_artifact_intent` / `unknown_shape_state` with `teaches` naming the vocabulary — never
/// serde enums, whose unknown-variant failure would land before validation, in the deserializer's
/// words, and outside the every-refusal-at-once body. Empty means the default, applied by the
/// validator: `current` and `member`, and every askable state.
///
/// **Strict**, like `ActInvocation`: an unknown field is a deserialization error, never a field
/// that silently narrows nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
#[serde(deny_unknown_fields)]
pub struct ArtifactAsk {
    /// The governing shape, by id. An id names one version (§2.2).
    pub shape: ShapeId,
    /// What the asker holds, AND-composed. Empty is the whole governed population — a vacuous
    /// conjunction, said on purpose (§2.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub predicates: Vec<ArtifactPredicate>,
    /// Narrow by selection intent: `current`, `member`, `pinned`. Empty: `current` and `member`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intent: Vec<String>,
    /// Admit folded (superseded) artifacts. Not served: `true` is declined by name
    /// (`filter_not_applicable`), and an ask reads live artifacts only.
    #[serde(default)]
    pub include_folded: bool,
    /// Narrow by verdict state: `declared_satisfied`, `declared_not_satisfied`,
    /// `declared_not_yet_checked`. Empty: all three.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shape_states: Vec<String>,
}

/// One predicate: a path of object-key segments into the content, and an operator from the closed
/// `PropertyOp` set — reused, not copied (§3.2). There is no `eq`, regex, prefix or text-search
/// operator, so a resemblance-shaped question has no way to be written down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
#[serde(deny_unknown_fields)]
pub struct ArtifactPredicate {
    /// Object keys, outermost first: `["measurements", "unit"]`. Not a string syntax; bound as a
    /// `text[]`, so no caller text becomes an identifier or a JSON-path fragment (§3.1).
    pub path: Vec<String>,
    pub op: PropertyOp,
}

impl ArtifactAsk {
    /// The bytes of every value this ask's predicates bind — each `contains` value and `compare`
    /// bound in compact JSON — in the measure `property_value_bytes` gives a property predicate's,
    /// so an ask's values and a filter's count toward one budget in one unit. The composable arm
    /// counts them into a composition's `MAX_COMPOSITION_PROPERTY_VALUE_BYTES`.
    pub fn value_bytes(&self) -> usize {
        self.predicates
            .iter()
            .flat_map(|p| p.op.values())
            .map(property_value_bytes)
            .sum()
    }
}

/// A shape the loader read through the gated by-id read and compiled with the no-fetch compiler —
/// so the walk sees a schema that compiled (decision P4: *the loader decided compilability*).
/// Nothing here compiles it.
///
/// **Live, unless it is a continuation's pinned row.** A first load (`shape_loader::load_one`)
/// refuses a folded row `shape_superseded`, so what it loads is live. A batch read's later pages
/// load the row their first page named (`shape_loader::load_one_pinned`) without that step, so the
/// row they load may since have been folded by an amendment (spec §7.3).
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedShape {
    pub shape_id: ShapeId,
    /// The chain depth of the assert/fold lineage, as `ShapeView::shape_version`.
    pub shape_version: i32,
    /// `"kb_contexts"` or `"kb_cogmaps"` — with `home_anchor_id`, the home the ask is scoped to.
    pub home_anchor_table: String,
    pub home_anchor_id: Uuid,
    /// `"kb_profiles"` or `"kb_teams"` — with `kind_owner_id` and `artifact_kind`, the family.
    pub kind_owner_table: String,
    pub kind_owner_id: Uuid,
    pub artifact_kind: String,
    pub schema: serde_json::Value,
}

impl LoadedShape {
    /// Everything that names this version, without its schema: what a validated ask carries to the
    /// seal and to `compile`.
    pub fn identity(&self) -> ShapeIdentity {
        // Destructured whole, so a field added here does not compile until it is placed.
        let LoadedShape {
            shape_id,
            shape_version,
            home_anchor_table,
            home_anchor_id,
            kind_owner_table,
            kind_owner_id,
            artifact_kind,
            schema: _,
        } = self;
        ShapeIdentity {
            shape_id: *shape_id,
            shape_version: *shape_version,
            home_anchor_table: home_anchor_table.clone(),
            home_anchor_id: *home_anchor_id,
            kind_owner_table: kind_owner_table.clone(),
            kind_owner_id: *kind_owner_id,
            artifact_kind: artifact_kind.clone(),
        }
    }
}

/// One shape version's identity — id and version, home pair, family triple — as a validated ask
/// carries it (spec §4.3 item 3: the seal holds *"the loaded shape rows (id, home, family,
/// version) for every ask it admitted"*). The home and family define the governed population
/// (§2.3); the id and version are what the trace discloses as validated against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeIdentity {
    pub shape_id: ShapeId,
    pub shape_version: i32,
    pub home_anchor_table: String,
    pub home_anchor_id: Uuid,
    pub kind_owner_table: String,
    pub kind_owner_id: Uuid,
    pub artifact_kind: String,
}

impl ShapeIdentity {
    /// The home this version is declared in — the scope of every ask against it (§2.1).
    pub fn home(&self) -> ShapeHome {
        ShapeHome {
            anchor_table: self.home_anchor_table.clone(),
            anchor_id: self.home_anchor_id,
        }
    }
}

/// A shape's home: the context or cogmap whose resources an ask against it ranges over. A resource
/// has exactly one home, so asks AND-ed on one resource must share one (§4.1, *Scope*).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ShapeHome {
    /// `"kb_contexts"` or `"kb_cogmaps"`.
    pub anchor_table: String,
    pub anchor_id: Uuid,
}

/// The version in force of a shape's lineage: the live row at its home and family when a read
/// found it. One type for one fact, named by each holder's field for how the row relates to it —
/// `successor` on a folded shape's refusal, `in_force` on a query trace's artifact ask. The type
/// carries the refusal's name; a trace entry whose shape was not amended names the asked row itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ShapeSuccessor {
    pub shape_id: ShapeId,
    pub shape_version: i32,
}

/// What the loader found for one named shape — refusal steps 1–3 of §3.5, in that order. Steps
/// 1–3 become refusals through [`ShapeLoad::loaded`]; [`AskSession::validate`] runs all four.
#[derive(Debug, Clone, PartialEq)]
pub enum ShapeLoad {
    /// Readable, and its schema compiles: live on a first load, and on a continuation's pinned load
    /// possibly folded since (see [`LoadedShape`]).
    Loaded(LoadedShape),
    /// Absent, or not readable by the asker — one arm, so the two are identical by construction.
    NotAskable,
    /// Folded. `successor` is `None` when no live row holds the home and family (decision P16:
    /// unreachable today, since amendment is the only fold); the refusal then omits it.
    Superseded { successor: Option<ShapeSuccessor> },
    /// The schema did not compile under the no-fetch compiler.
    Uncompilable,
    /// The schema is larger than the walk reads ([`walk::MAX_SCHEMA_BYTES`]), so it was never
    /// compiled. Step 3 still: the loader decides compilability (§2.4), and a schema this server
    /// will not read is one it does not compile. Refused `shape_schema_uncompilable` like
    /// [`ShapeLoad::Uncompilable`], with its own sentence and `teaches.beyond_bound: schema_bytes`,
    /// so the bound is said as data, the way a path refusal says it (§3.3).
    SchemaTooLarge,
}

/// Every shape an ask in one request names, loaded before the gate (§4.3): plain data, fixed
/// before validation runs and untouched by anything after it.
///
/// What a request derives from these shapes — each one's walk, the paths it has resolved, what a
/// path refusal teaches of it, each ask's answer — and the membership budget its asks share live in
/// the [`AskSession`] begun from it with [`ShapeContext::session`], which borrows the shapes held
/// here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShapeContext {
    loads: BTreeMap<ShapeId, ShapeLoad>,
}

impl ShapeContext {
    /// No shapes: the context of a request that names none. An ask validated against it is
    /// refused `shape_not_askable`, as an absent shape's is — never checked against nothing and
    /// admitted.
    pub const fn empty() -> ShapeContext {
        ShapeContext {
            loads: BTreeMap::new(),
        }
    }

    /// What the loader found for a shape, or `None` for a shape it was not asked to load.
    pub fn get(&self, shape: &ShapeId) -> Option<&ShapeLoad> {
        self.loads.get(shape)
    }
}

impl FromIterator<(ShapeId, ShapeLoad)> for ShapeContext {
    fn from_iter<I: IntoIterator<Item = (ShapeId, ShapeLoad)>>(iter: I) -> ShapeContext {
        ShapeContext {
            loads: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shape_id() -> ShapeId {
        ShapeId::from(Uuid::nil())
    }

    #[test]
    fn a_resemblance_shaped_operator_cannot_be_written_down() {
        // `asking-is-never-resemblance`, refusal face: the vocabulary is closed, so these are not
        // refused — they fail to exist.
        for op in [
            json!({ "op": "regex", "value": "^ms" }),
            json!({ "op": "eq", "value": "ms" }),
            json!({ "op": "text", "value": "milliseconds" }),
        ] {
            let predicate = json!({ "path": ["unit"], "op": op });
            assert!(
                serde_json::from_value::<ArtifactPredicate>(predicate.clone()).is_err(),
                "{predicate}"
            );
        }
    }

    #[test]
    fn the_three_operators_deserialize_from_the_property_op_vocabulary() {
        for op in [
            json!({ "op": "has_key" }),
            json!({ "op": "contains", "values": ["ms"] }),
            json!({ "op": "compare", "direction": "gte", "value": 3 }),
        ] {
            let predicate = json!({ "path": ["unit"], "op": op });
            assert!(serde_json::from_value::<ArtifactPredicate>(predicate).is_ok());
        }
    }

    #[test]
    fn an_unknown_intent_deserializes_so_that_validation_can_refuse_it() {
        let ask: ArtifactAsk = serde_json::from_value(json!({
            "shape": Uuid::nil(),
            "intent": ["favourite"],
            "shape_states": ["never_declared"]
        }))
        .expect("intent and shape_states are strings, not enums");
        assert_eq!(ask.intent, vec!["favourite".to_string()]);
        assert_eq!(ask.shape_states, vec!["never_declared".to_string()]);
    }

    #[test]
    fn an_ask_naming_only_its_shape_is_the_whole_population_with_the_defaults_left_to_validation() {
        let ask: ArtifactAsk = serde_json::from_value(json!({ "shape": Uuid::nil() })).unwrap();
        assert_eq!(
            ask,
            ArtifactAsk {
                shape: shape_id(),
                predicates: vec![],
                intent: vec![],
                include_folded: false,
                shape_states: vec![],
            }
        );
        assert_eq!(
            serde_json::to_value(&ask).unwrap(),
            json!({ "shape": Uuid::nil(), "include_folded": false })
        );
    }

    #[test]
    fn an_unknown_field_on_an_ask_is_a_deserialization_error() {
        assert!(serde_json::from_value::<ArtifactAsk>(json!({
            "shape": Uuid::nil(),
            "predicate": []
        }))
        .is_err());
        assert!(serde_json::from_value::<ArtifactPredicate>(json!({
            "path": ["a"], "op": { "op": "has_key" }, "negate": true
        }))
        .is_err());
    }

    #[test]
    fn a_shape_context_answers_only_for_the_shapes_it_was_given() {
        let other = ShapeId::from(Uuid::from_u128(1));
        let context: ShapeContext = [(shape_id(), ShapeLoad::NotAskable)].into_iter().collect();
        assert_eq!(context.get(&shape_id()), Some(&ShapeLoad::NotAskable));
        assert_eq!(context.get(&other), None);
    }

    #[test]
    fn an_asks_value_bytes_are_its_bound_values_in_compact_json() {
        let ask: ArtifactAsk = serde_json::from_value(json!({
            "shape": Uuid::nil(),
            "predicates": [
                { "path": ["a"], "op": { "op": "has_key" } },
                { "path": ["b"], "op": { "op": "contains", "values": ["ms", { "k": [1, 2] }] } },
                { "path": ["c"], "op": { "op": "compare", "direction": "gte", "value": 10 } }
            ]
        }))
        .unwrap();
        // `"ms"` 4, `{"k":[1,2]}` 11, `10` 2; `has_key` binds nothing.
        assert_eq!(ask.value_bytes(), 17);
    }

    #[test]
    fn the_empty_context_holds_no_shape() {
        assert_eq!(ShapeContext::empty(), ShapeContext::default());
        assert_eq!(ShapeContext::empty().get(&shape_id()), None);
    }
}
