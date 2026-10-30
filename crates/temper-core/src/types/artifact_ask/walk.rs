//! The schema walk — what a shape declares at a path, decided over a `serde_json::Value` with no
//! JSON Schema validator (spec §2.4, §3.3).
//!
//! **It reads a field tree, and refuses anything else.** A shape compiles only as a field tree
//! (`temper_substrate::shape_schema`, spec `2026-10-27-a-shape-is-a-field-tree-design`): schemas at
//! the root, in each `properties` value, in `additionalProperties` and in `items`, each an object
//! or a boolean, with no `$ref`, `$defs` or combinator. The walk reads `properties` chains, `type`,
//! `enum`, `const`, boolean subschemas and a single `items` schema. It passes over the keywords
//! that constrain content without changing what a path declares — `required`, `pattern`, the
//! length, size and range bounds, the annotations, and `$schema` and `$id` at the root.
//! `additionalProperties` is **not walked**, so a path reachable only through it is not declared —
//! the declared hole of spec §11, refused as `path_outside_shape` rather than guessed at.
//!
//! **Anything else is refused, never read as unconstrained.** Any other keyword, a `$schema` other
//! than the 2020-12 spelling, a value that is not a schema where a schema sits, or an `enum` or
//! `type` in a form the walk does not read fails [`reads_whole`], and the walk reads that schema as
//! `false` — it declares nothing: every path is `NotDeclared` and the vocabulary is empty — and
//! logs the mismatch. No shape that compiles reaches it while the compile and the walk agree, which
//! `temper-substrate`'s field-tree tests hold them to; if the field tree is widened, the walk
//! refuses the new construct until someone decides how it reads.
//!
//! The walk's list is its own, not the compile's. The two answer different questions — what a shape
//! may say, and what the walk has decided how to read — and one shared list would widen the walk
//! silently whenever the compile widened.
//!
//! # Bounded by the schema (§3.3: the walk "never stalls or exhausts the server")
//!
//! With no `$ref` and no combinator, each schema node is reached by one path and read once:
//! resolving a path reads the nodes along it and its `items` chain, and enumerating reads each node
//! once. So the walk's work is linear in the schema, which it reads only up to [`MAX_SCHEMA_BYTES`]
//! — the size declare holds a schema to. [`MAX_WALK_PATHS`] bounds the vocabulary a read returns
//! and [`MAX_WALK_DEPTH`] how deep an ask and a vocabulary reach. Nothing recurses over the schema
//! but an `items` chain, held to the depth bound.
//!
//! **What a vocabulary holds.** A vocabulary copies each path's `enum` and `const` values onto the
//! wire. No two paths share a keyword, so each value is copied at most once and a vocabulary holds
//! no more values than its schema. An ask, and a refusal's teaching until it sends a path, read the
//! values where they sit and copy none.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::reference::canonical;
use super::validate::{PathTeaching, MAX_ARTIFACT_PATH_SEGMENT_BYTES};
use crate::types::ids::ShapeId;
use crate::types::query::filter::json_string_bytes;

/// The deepest path, in object-key segments, the walk descends to. A deeper ask is
/// [`PathResolution::BeyondBound`]`(`[`BoundKind::Depth`]`)`, refused `path_outside_shape` with
/// `teaches` naming the bound (spec §2.5). It also bounds how many array levels deep element facts
/// ([`PathFacts::items`]) are reported; past it an element is unconstrained.
///
/// # 16
///
/// Structured content in a shape is a record of a few levels; 16 levels of object nesting is far
/// past any shape a person writes. A field tree nests at most this deep (`MAX_SHAPE_DEPTH`, held
/// equal to it), so the walk sees a field tree whole.
pub const MAX_WALK_DEPTH: usize = 16;

/// The most paths [`enumerate`] advertises. Past it the vocabulary is truncated at
/// [`BoundKind::Paths`], visibly (spec §2.5).
///
/// # 1024
///
/// The vocabulary is read whole by `schema show`, `GET /api/shapes/{id}` and
/// `get_data_artifact_shape`, so this bounds a response body. It is far above a shape a person
/// writes — a record with tens of fields — and below what a schema at [`MAX_SCHEMA_BYTES`] can
/// declare: about twenty thousand empty fields fit in one.
///
/// It bounds how many paths, not how large each is. A path's text is at most [`MAX_WALK_DEPTH`]
/// segments of at most `MAX_ARTIFACT_PATH_SEGMENT_BYTES` — a longer key is never advertised — so
/// 4 KiB.
pub const MAX_WALK_PATHS: usize = 1024;

/// The largest schema the walk reads, in compact serialized JSON bytes. Past it nothing is walked:
/// every path is [`PathResolution::BeyondBound`]`(`[`BoundKind::SchemaBytes`]`)` and the
/// vocabulary is empty and says why. The measurement itself stops at the bound, so a schema far
/// over it costs no more to refuse than one just over it.
///
/// # 256 KiB
///
/// A JSON Schema for one family of structured records is kilobytes; 256 KiB admits two orders of
/// magnitude more than that. Declare refuses a larger schema, so a stored shape is never past it.
pub const MAX_SCHEMA_BYTES: usize = 256 * 1024;

/// The walk's bounds. The values the build runs are [`WalkBounds::BUILD`]; tests construct smaller
/// ones to reach a bound without a large schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkBounds {
    pub max_depth: usize,
    pub max_paths: usize,
    pub max_schema_bytes: usize,
}

impl WalkBounds {
    /// The bounds every production caller walks under.
    pub const BUILD: WalkBounds = WalkBounds {
        max_depth: MAX_WALK_DEPTH,
        max_paths: MAX_WALK_PATHS,
        max_schema_bytes: MAX_SCHEMA_BYTES,
    };
}

/// Which bound reading a shape met. A truncated vocabulary and a refusal both name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BoundKind {
    /// The path is deeper than the walk descends.
    Depth,
    /// The vocabulary has more paths than are advertised.
    Paths,
    /// The schema is larger than the walk reads.
    SchemaBytes,
    /// Checking the request's values against the shape's `enum`s spent its membership steps
    /// before it could say.
    Work,
}

/// A JSON type as JSON Schema's `type` keyword names it. `integer` is the integral subset of
/// `number`, which is why it is its own member here and not in jsonb's `jsonb_typeof`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum JsonType {
    Null,
    Boolean,
    Integer,
    Number,
    String,
    Array,
    Object,
}

impl JsonType {
    /// The type of a value as `jsonb_typeof` sees it: every number is [`JsonType::Number`].
    pub fn of(value: &Value) -> JsonType {
        match value {
            Value::Null => JsonType::Null,
            Value::Bool(_) => JsonType::Boolean,
            Value::Number(_) => JsonType::Number,
            Value::String(_) => JsonType::String,
            Value::Array(_) => JsonType::Array,
            Value::Object(_) => JsonType::Object,
        }
    }

    fn from_keyword(name: &str) -> Option<JsonType> {
        Some(match name {
            "null" => JsonType::Null,
            "boolean" => JsonType::Boolean,
            "integer" => JsonType::Integer,
            "number" => JsonType::Number,
            "string" => JsonType::String,
            "array" => JsonType::Array,
            "object" => JsonType::Object,
            _ => return None,
        })
    }
}

/// What the schema declares at one path. Each field is `None` when the schema leaves it
/// unconstrained, so an absent field never reads as "nothing is admitted".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct PathFacts {
    /// The declared JSON types. `None`: no `type` constrains the path. An empty set: no value is
    /// admissible — a `type` naming no JSON type, or the element of an `items: false` array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<BTreeSet<JsonType>>,
    /// The `enum` / `const` values. `None`: no value set constrains the path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<Value>>,
    /// What an element of an array at this path declares. `None`: elements are unconstrained.
    /// Reported at most as many arrays deep as the walk's depth bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "web-api", schema(no_recursion))]
    pub items: Option<Box<PathFacts>>,
}

impl PathFacts {
    /// Whether `compare` is feasible here: the path declares `number`, `integer` or `string`, or
    /// declares no type (spec §3.4, the `compare` row).
    pub fn admits_compare(&self) -> bool {
        admits_compare(self.types.as_ref())
    }

    /// The operators feasible at this path, in [`AskOperator`] order. `has_key` and `contains`
    /// always are — whether a `contains` value is admissible is the value's question
    /// (`value_outside_shape`), never the operator's.
    pub fn operators(&self) -> Vec<AskOperator> {
        operators(self.types.as_ref())
    }
}

/// [`PathFacts::admits_compare`], over the declared types alone.
fn admits_compare(types: Option<&BTreeSet<JsonType>>) -> bool {
    types.is_none_or(|t| {
        t.iter()
            .any(|t| matches!(t, JsonType::Number | JsonType::Integer | JsonType::String))
    })
}

/// [`PathFacts::operators`], over the declared types alone, so the wire copy and the borrowed
/// view of one path list the same operators.
fn operators(types: Option<&BTreeSet<JsonType>>) -> Vec<AskOperator> {
    let mut ops = vec![AskOperator::HasKey, AskOperator::Contains];
    if admits_compare(types) {
        ops.push(AskOperator::Compare);
    }
    ops
}

/// The name of a `PropertyOp` operator, for a vocabulary to
/// list. Not a second operator type: it carries no value, and binds nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AskOperator {
    HasKey,
    Contains,
    Compare,
}

/// One askable path: its segments, what the schema declares there, and the operators feasible at
/// it (spec §2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AskablePath {
    pub path: Vec<String>,
    pub facts: PathFacts,
    pub operators: Vec<AskOperator>,
}

impl AskablePath {
    pub(super) fn new(path: Vec<String>, facts: PathFacts) -> AskablePath {
        let operators = facts.operators();
        AskablePath {
            path,
            facts,
            operators,
        }
    }
}

/// What a shape lets a caller ask: its askable paths, and every bound the list was cut at.
///
/// `truncated` names each walk bound met, never a silent short list (spec §2.5): a tree nested
/// deeper than the walk descends is `depth`, a shape declaring more paths than are advertised is
/// `paths`, and an oversized schema is `schema_bytes` with no paths. Ordered, and at most one of
/// each.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Vocabulary {
    pub paths: Vec<AskablePath>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub truncated: BTreeSet<BoundKind>,
}

/// Where one asked path stands against the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathResolution {
    /// Reached through `properties` at every segment, and not forbidden by a `false`.
    Declared(PathFacts),
    /// Not reached through `properties`, or forbidden (`false`).
    NotDeclared,
    /// The walk stopped at a bound before it could say.
    BeyondBound(BoundKind),
}

/// Resolve one path against a schema: one walk down it, never an enumeration.
pub fn resolve(schema: &Value, path: &[String], bounds: &WalkBounds) -> PathResolution {
    match Walk::new(schema, bounds) {
        Ok(walk) => walk.resolve(path),
        Err(bound) => PathResolution::BeyondBound(bound),
    }
}

/// Every askable path of a schema, up to the bounds, for `askable` (spec §2.5).
pub fn enumerate(schema: &Value, bounds: &WalkBounds) -> Vocabulary {
    match Walk::new(schema, bounds) {
        Ok(walk) => walk.enumerate(),
        Err(bound) => unwalked(bound),
    }
}

/// Whether a schema is within the size the walk reads ([`MAX_SCHEMA_BYTES`], compact JSON bytes),
/// measured only as far as the bound — so a schema far over it costs no more to judge than one
/// just over it. Asking checks this before it compiles a schema — the shape loader, the by-id
/// read's `askable` and declare — so none of them compiles a schema the walk will not read (spec
/// §3.3, the walk "never stalls or exhausts the server"). Commit and reconcile do not ask it.
pub fn within_schema_bound(schema: &Value) -> bool {
    json_bytes_up_to(schema, MAX_SCHEMA_BYTES) <= MAX_SCHEMA_BYTES
}

/// The vocabulary of a schema the walk would not read: no paths, and the bound that stopped it.
fn unwalked(bound: BoundKind) -> Vocabulary {
    Vocabulary {
        paths: Vec::new(),
        truncated: BTreeSet::from([bound]),
    }
}

/// An enumeration's paths, each with its facts in whichever form the enumeration keeps them, and
/// every bound it was cut at — a [`Vocabulary`] before its facts are put on the wire.
#[derive(Debug)]
pub(super) struct Enumerated<F> {
    pub(super) paths: Vec<(Vec<String>, F)>,
    pub(super) truncated: BTreeSet<BoundKind>,
}

impl<F> Enumerated<F> {
    /// The enumeration of a schema the walk would not read, as [`unwalked`] says it.
    fn unwalked(bound: BoundKind) -> Enumerated<F> {
        Enumerated {
            paths: Vec::new(),
            truncated: BTreeSet::from([bound]),
        }
    }
}

/// The `$schema` spellings the walk reads as 2020-12, at the root only: the bundled one, with or
/// without a trailing `#`. The walk's own list beside the compile's, for the reason the module
/// gives.
const READ_SCHEMA_URIS: [&str; 2] = [
    "https://json-schema.org/draft/2020-12/schema",
    "https://json-schema.org/draft/2020-12/schema#",
];

/// Whether the walk reads every construct in the schema: in each schema position — the root, each
/// `properties` value and an `items` schema — an object or a boolean, holding only keywords the
/// walk reads or passes over, with `$schema` (the 2020-12 spelling, with or without a trailing `#`)
/// and `$id` at the root alone. An `enum` must be an array, and a `type` a name or an array of
/// names, since the walk would read any other form as declaring nothing. What
/// `additionalProperties` holds is not walked, so it is not read here.
///
/// One pass from an explicit stack, each schema position pushed once, so it is linear in the
/// schema and never recurses on the Rust stack.
pub fn reads_whole(schema: &Value) -> bool {
    let mut pending = vec![schema];
    while let Some(node) = pending.pop() {
        let map = match node {
            Value::Bool(_) => continue,
            Value::Object(map) => map,
            _ => return false,
        };
        let root = std::ptr::eq(node, schema);
        for (keyword, value) in map {
            match keyword.as_str() {
                "properties" => match value {
                    Value::Object(fields) => pending.extend(fields.values()),
                    _ => return false,
                },
                "items" => match value {
                    Value::Bool(_) | Value::Object(_) => pending.push(value),
                    _ => return false,
                },
                // What a path declares, in the forms the walk reads.
                "type" if is_type_form(value) => {}
                "enum" if value.is_array() => {}
                "const"
                // Not walked: a path reachable only through it is not declared.
                | "additionalProperties"
                // What content must satisfy, which changes nothing a path declares.
                | "required" | "pattern" | "minLength" | "maxLength" | "minItems" | "maxItems"
                | "minimum" | "maximum"
                // Annotations.
                | "title" | "description" | "$comment" | "default" | "examples" => {}
                "$schema"
                    if root
                        && value
                            .as_str()
                            .is_some_and(|uri| READ_SCHEMA_URIS.contains(&uri)) => {}
                "$id" if root => {}
                _ => return false,
            }
        }
    }
    true
}

/// Whether a `type` is in a form the walk reads: one name, or an array of names.
fn is_type_form(value: &Value) -> bool {
    match value {
        Value::String(_) => true,
        Value::Array(names) => names.iter().all(Value::is_string),
        _ => false,
    }
}

/// What the walk reads in place of a schema it does not read whole: `false`, declaring no path.
static DECLARES_NOTHING: Value = Value::Bool(false);

/// One schema, measured and read whole, ready to resolve paths against.
#[derive(Debug)]
pub(super) struct Walk<'s> {
    root: &'s Value,
    bounds: WalkBounds,
}

impl<'s> Walk<'s> {
    /// Measure the schema once, no further than the bound, then check that the walk reads every
    /// construct in it ([`reads_whole`]). Over [`WalkBounds::max_schema_bytes`] it is
    /// [`BoundKind::SchemaBytes`]. With a construct the walk does not read it is walked as `false`,
    /// declaring nothing, and logged as the server mismatch it is — under the caller's span, which
    /// names the shape.
    pub(super) fn new(schema: &'s Value, bounds: &WalkBounds) -> Result<Walk<'s>, BoundKind> {
        if json_bytes_up_to(schema, bounds.max_schema_bytes) > bounds.max_schema_bytes {
            return Err(BoundKind::SchemaBytes);
        }
        let root = if reads_whole(schema) {
            schema
        } else {
            tracing::error!(
                "the schema walk met a construct it does not read; the shape declares nothing"
            );
            &DECLARES_NOTHING
        };
        #[cfg(test)]
        WALKS_BUILT.with(|built| built.set(built.get() + 1));
        Ok(Walk {
            root,
            bounds: *bounds,
        })
    }

    /// One path, its facts copied onto the wire.
    fn resolve(&self, path: &[String]) -> PathResolution {
        match self.reach(path) {
            Resolved::Declared(facts) => PathResolution::Declared(facts.to_wire()),
            Resolved::NotDeclared => PathResolution::NotDeclared,
            Resolved::BeyondBound(bound) => PathResolution::BeyondBound(bound),
        }
    }

    /// One path, its facts left where they sit in the schema.
    pub(super) fn reach(&self, path: &[String]) -> Resolved<'s> {
        #[cfg(test)]
        PATHS_REACHED.with(|reached| reached.set(reached.get() + 1));
        if path.len() > self.bounds.max_depth {
            return Resolved::BeyondBound(BoundKind::Depth);
        }
        let reached = path
            .iter()
            .try_fold(self.root, |node, segment| property(node, segment));
        match reached.and_then(|node| self.facts_of(node, 0)) {
            Some(facts) => Resolved::Declared(facts),
            None => Resolved::NotDeclared,
        }
    }

    /// Breadth-first over property keys, shallowest paths first and keys in byte order within a
    /// level, so a truncated vocabulary is the shallow part of the full one. Every value is copied
    /// onto the wire: the whole vocabulary, for `askable`.
    fn enumerate(&self) -> Vocabulary {
        let found = self.enumerate_with(|facts| facts.to_wire());
        Vocabulary {
            paths: found
                .paths
                .into_iter()
                .map(|(path, facts)| AskablePath::new(path, facts))
                .collect(),
            truncated: found.truncated,
        }
    }

    /// The same paths in the same order with the same cuts, each with the walk's own facts,
    /// borrowed from the schema: no value is copied. What a refusal teaches from — it copies only
    /// the paths it sends (`teaching` in `validate`).
    pub(super) fn enumerate_borrowed(&self) -> Enumerated<Facts<'s>> {
        self.enumerate_with(|facts| facts)
    }

    /// One enumeration, with `keep` deciding the form each declared path's facts are kept in.
    fn enumerate_with<F>(&self, keep: impl Fn(Facts<'s>) -> F) -> Enumerated<F> {
        #[cfg(test)]
        VOCABULARIES_ENUMERATED.with(|enumerated| enumerated.set(enumerated.get() + 1));
        let mut found = Enumerated {
            paths: Vec::new(),
            truncated: BTreeSet::new(),
        };
        if let Err(bound) = self.enumerate_into(&mut found, &keep) {
            found.truncated.insert(bound);
        }
        found
    }

    /// The breadth-first walk behind [`Walk::enumerate_with`]. A depth cut is recorded and the walk
    /// goes on; the path bound stops it, and is returned.
    ///
    /// **No prefix is held twice.** A queued node names the path it sits under by its index in
    /// `out.paths`, so a prefix lives only in the path the vocabulary carries.
    fn enumerate_into<F>(
        &self,
        out: &mut Enumerated<F>,
        keep: &impl Fn(Facts<'s>) -> F,
    ) -> Result<(), BoundKind> {
        // (the index in `out.paths` of the path a node sits under — none for the root; the node)
        let mut queue: VecDeque<(Option<usize>, &'s Value)> = VecDeque::from([(None, self.root)]);
        while let Some((parent, node)) = queue.pop_front() {
            let depth = parent.map_or(0, |at| out.paths[at].0.len());
            if depth == self.bounds.max_depth {
                // Whether keys exist below the bound, read without collecting them: a cut costs the
                // node, not the keys under it. Whether each is declared is past the bound too, so
                // this says "there may be more" — the honest reading of a depth cut.
                if declares_properties(node) {
                    out.truncated.insert(BoundKind::Depth);
                }
                continue;
            }
            for (key, child) in child_properties(node) {
                let Some(facts) = self.facts_of(child, 0) else {
                    continue;
                };
                if out.paths.len() == self.bounds.max_paths {
                    return Err(BoundKind::Paths);
                }
                let prefix: &[String] = match parent {
                    Some(at) => &out.paths[at].0,
                    None => &[],
                };
                let mut path = Vec::with_capacity(prefix.len() + 1);
                path.extend_from_slice(prefix);
                path.push(key.to_string());
                out.paths.push((path, keep(facts)));
                queue.push_back((Some(out.paths.len() - 1), child));
            }
        }
        Ok(())
    }

    /// What a subschema declares at a path's end, `arrays` arrays below it: `None` for `false`,
    /// nothing at all for `true`, and an object's own `type`, `enum` / `const` and element facts.
    /// Element facts stop at the depth bound, past which an element is unconstrained.
    fn facts_of(&self, node: &'s Value, arrays: usize) -> Option<Facts<'s>> {
        let map = match node {
            Value::Bool(false) => return None,
            Value::Object(map) => map,
            // `true`. Nothing else sits in a schema position of a schema the walk reads whole.
            _ => return Some(Facts::default()),
        };
        let items = map
            .get("items")
            .filter(|_| arrays < self.bounds.max_depth)
            .and_then(|element| match self.facts_of(element, arrays + 1) {
                // `items: false` admits no element at all.
                None => Some(Arc::new(Facts {
                    types: Some(BTreeSet::new()),
                    ..Facts::default()
                })),
                Some(facts) if facts.is_unconstrained() => None,
                Some(facts) => Some(Arc::new(facts)),
            });
        Some(Facts {
            types: declared_types(map),
            values: declared_values(map),
            items,
        })
    }
}

/// The subschema `properties[key]` declares on `node`, if any. A `true`, a `false` and a schema
/// without `properties` declare nothing below them.
fn property<'s>(node: &'s Value, key: &str) -> Option<&'s Value> {
    node.get("properties")?.get(key)
}

/// The types a `type` keyword declares: one name, a list of them, or — for any other form — none
/// admissible.
fn declared_types(map: &Map<String, Value>) -> Option<BTreeSet<JsonType>> {
    Some(match map.get("type")? {
        Value::String(name) => JsonType::from_keyword(name).into_iter().collect(),
        Value::Array(names) => names
            .iter()
            .filter_map(|n| n.as_str().and_then(JsonType::from_keyword))
            .collect(),
        _ => BTreeSet::new(),
    })
}

/// The value set `enum` and `const` declare between them: an `enum`'s members, a `const`'s one
/// value, or with both the members jsonb-equal to the `const`.
fn declared_values<'s>(map: &'s Map<String, Value>) -> Option<Values<'s>> {
    let enumerated = map.get("enum").and_then(Value::as_array);
    let values: Vec<&'s Value> = match (enumerated, map.get("const")) {
        (None, None) => return None,
        (Some(members), None) => members.iter().collect(),
        (None, Some(constant)) => vec![constant],
        (Some(members), Some(constant)) => {
            let constant = canonical(constant);
            members
                .iter()
                .filter(|member| canonical(member) == constant)
                .collect()
        }
    };
    Some(Arc::new(values))
}

/// One shape's walk as a request reads it: built once, on the first ask that needs it, and kept
/// with what it has said — each path it has resolved, and what a path refusal teaches of the shape
/// once made — so every later ask in the request reuses them (spec §2.4: a shape row is immutable
/// per id, so what is derived from it is cached by id safely). Resolved paths are kept as the walk
/// found them, borrowed from the schema: no value is copied until a refusal's teaching sends it.
///
/// **Owned by an [`AskSession`](super::AskSession), never by the schema's owner.** A [`Walk`]
/// borrows the schema it reads, so the cache cannot live beside the schema in one struct; it lives
/// in a session that borrows the context holding the schemas, for as long as the request runs.
#[derive(Debug)]
pub(super) struct ShapeWalk<'s> {
    shape: ShapeId,
    schema: &'s Value,
    /// Built on first use, so an ask refused before the shape is consulted builds nothing.
    walk: Option<Result<Walk<'s>, BoundKind>>,
    resolved: HashMap<Vec<String>, Resolved<'s>>,
    /// What a path refusal teaches of the shape: the one its context remembered, or the one made
    /// here on the first path refusal.
    taught: Option<Taught>,
    /// Whether this walk enumerated the shape's vocabulary — at most once, to make its teaching.
    enumerated: bool,
}

/// The walk of one shape's schema under the build's bounds, built under a span naming the shape, so
/// a mismatch [`Walk::new`] logs says which shape it met.
fn walk_of(shape: ShapeId, schema: &Value) -> Result<Walk<'_>, BoundKind> {
    let _shape = tracing::error_span!("shape_walk", %shape).entered();
    Walk::new(schema, &WalkBounds::BUILD)
}

/// Where a shape's path teaching came from: its context, which remembered it from an earlier
/// request, or this walk, which made it.
#[derive(Debug)]
enum Taught {
    Remembered(PathTeaching),
    Made(PathTeaching),
}

impl<'s> ShapeWalk<'s> {
    /// A walk of `schema`, the shape `shape`'s, holding `remembered` — what a path refusal teaches
    /// of that shape, if its context carried one — so that no refusal enumerates it again.
    pub(super) fn new(
        shape: ShapeId,
        schema: &'s Value,
        remembered: Option<PathTeaching>,
    ) -> ShapeWalk<'s> {
        ShapeWalk {
            shape,
            schema,
            walk: None,
            resolved: HashMap::new(),
            taught: remembered.map(Taught::Remembered),
            enumerated: false,
        }
    }

    /// The shape this walk reads.
    pub(super) fn shape(&self) -> ShapeId {
        self.shape
    }

    /// One path, as this walk last said it, or reached now — so a path is walked once per request
    /// however many asks name it.
    pub(super) fn resolve(&mut self, path: &[String]) -> Resolved<'s> {
        if let Some(known) = self.resolved.get(path) {
            return known.clone();
        }
        let (shape, schema) = (self.shape, self.schema);
        let reached = match self.walk.get_or_insert_with(|| walk_of(shape, schema)) {
            Ok(walk) => walk.reach(path),
            Err(at) => Resolved::BeyondBound(*at),
        };
        self.resolved.insert(path.to_vec(), reached.clone());
        reached
    }

    /// What a path refusal teaches of this shape: the teaching its context remembered, or — the
    /// first time one is needed, when there is none — the one `teach` makes from the shape's
    /// askable vocabulary over borrowed facts ([`Walk::enumerate_borrowed`]), enumerated here once.
    /// Either is kept for every later refusal; the vocabulary is not.
    pub(super) fn path_teaching(
        &mut self,
        teach: impl FnOnce(&Enumerated<Facts<'s>>, ShapeId) -> PathTeaching,
    ) -> &PathTeaching {
        let ShapeWalk {
            shape,
            schema,
            walk,
            taught,
            enumerated,
            ..
        } = self;
        let schema: &'s Value = schema;
        let shape = *shape;
        let taught = taught.get_or_insert_with(|| {
            let vocabulary = match walk.get_or_insert_with(|| walk_of(shape, schema)) {
                Ok(walk) => {
                    *enumerated = true;
                    walk.enumerate_borrowed()
                }
                Err(at) => Enumerated::unwalked(*at),
            };
            Taught::Made(teach(&vocabulary, shape))
        });
        match taught {
            Taught::Remembered(teaching) | Taught::Made(teaching) => teaching,
        }
    }

    /// The path teaching this walk made, if it made one — never one its context remembered.
    pub(super) fn made_path_teaching(&self) -> Option<&PathTeaching> {
        match &self.taught {
            Some(Taught::Made(teaching)) => Some(teaching),
            Some(Taught::Remembered(_)) | None => None,
        }
    }

    /// Whether this walk enumerated the shape's vocabulary.
    pub(super) fn enumerated(&self) -> bool {
        self.enumerated
    }
}

/// A value's compact JSON size, counted only until it passes `limit`: a result over `limit` says
/// the value is larger, not by how much. The counting sink refuses the first write past the limit,
/// which stops the serializer there, so measuring a schema far over [`MAX_SCHEMA_BYTES`] costs
/// about that much work, not the schema's size.
pub(super) fn json_bytes_up_to<T: Serialize + ?Sized>(value: &T, limit: usize) -> usize {
    struct Count {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes = self.bytes.saturating_add(buf.len());
            if self.bytes > self.limit {
                return Err(std::io::Error::other("past the measured limit"));
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count { bytes: 0, limit };
    // What this crate measures — a `Value`, or a vocabulary built of them — always serializes, so
    // an error is the sink stopping past the limit, which the count already says.
    let _stopped = serde_json::to_writer(&mut count, value);
    count.bytes
}

/// The askable keys a node's `properties` declares, each with its subschema, in byte order.
///
/// A key longer than an asked segment may be (`MAX_ARTIFACT_PATH_SEGMENT_BYTES`, the form cap
/// `validate_ask_form` refuses a longer segment by) is skipped: no ask can name it, so it is never
/// advertised and nothing below it is walked.
fn child_properties(node: &Value) -> Vec<(&str, &Value)> {
    let mut children: Vec<(&str, &Value)> = node
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(key, child)| (key.as_str(), child))
        .filter(|(key, _)| askable_segment(key))
        .collect();
    children.sort_unstable_by_key(|&(key, _)| key);
    children
}

/// Whether a key could be one segment of an asked path: within `MAX_ARTIFACT_PATH_SEGMENT_BYTES`,
/// JSON-escaped, as `validate_ask_form` measures it. Its length is read first — escaping only
/// lengthens a key, so one longer than the cap is past it.
fn askable_segment(key: &str) -> bool {
    key.len() <= MAX_ARTIFACT_PATH_SEGMENT_BYTES
        && json_string_bytes(key) <= MAX_ARTIFACT_PATH_SEGMENT_BYTES
}

/// Whether a node declares any property at all.
fn declares_properties(node: &Value) -> bool {
    node.get("properties")
        .and_then(Value::as_object)
        .is_some_and(|props| !props.is_empty())
}

/// A value set, borrowed from the schema and shared rather than copied when a result is kept.
pub(super) type Values<'s> = Arc<Vec<&'s Value>>;

/// What the walk has found at a path: [`PathFacts`], with its values borrowed from the schema and
/// every part shared, so carrying it costs nothing per value. An ask's validation reads its paths
/// in this form and never copies them; only the wire copies.
#[derive(Debug, Clone, Default)]
pub(super) struct Facts<'s> {
    pub(super) types: Option<BTreeSet<JsonType>>,
    pub(super) values: Option<Values<'s>>,
    pub(super) items: Option<Arc<Facts<'s>>>,
}

impl Facts<'_> {
    /// Whether nothing at all is declared — the facts of `{}` or `true`.
    fn is_unconstrained(&self) -> bool {
        self.types.is_none() && self.values.is_none() && self.items.is_none()
    }

    /// The facts as the wire carries them, every value copied. A refusal's teaching copies only a
    /// path it has first measured, through [`AskableView`], to fit.
    pub(super) fn to_wire(&self) -> PathFacts {
        PathFacts {
            types: self.types.clone(),
            values: self.values.as_ref().map(|values| {
                #[cfg(test)]
                VALUES_COPIED.with(|copied| copied.set(copied.get() + values.len()));
                values.iter().map(|&v| v.clone()).collect()
            }),
            items: self
                .items
                .as_ref()
                .map(|element| Box::new(element.to_wire())),
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Every value [`Facts::to_wire`] has copied on this thread — what a test reads to see that a
    /// copy was, or was not, made.
    pub(crate) static VALUES_COPIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Every [`Walk`] built on this thread — measured and read whole.
    pub(crate) static WALKS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Every vocabulary a [`Walk`] has enumerated on this thread, copied or borrowed.
    pub(crate) static VOCABULARIES_ENUMERATED: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
    /// Every path a [`Walk`] has reached on this thread — what a test reads to see that a path was,
    /// or was not, walked again.
    pub(crate) static PATHS_REACHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The paths reached on this thread so far ([`PATHS_REACHED`]).
#[cfg(test)]
pub(crate) fn paths_reached() -> usize {
    PATHS_REACHED.with(std::cell::Cell::get)
}

/// The wire form of [`PathFacts`], written from the borrowed facts without copying a value, so a
/// path can be measured before it is copied. Field for field what `PathFacts`'s derived
/// `Serialize` writes — each field present only when set — which
/// `the_borrowed_view_writes_the_wire_bytes` holds it to.
impl Serialize for Facts<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let fields = usize::from(self.types.is_some())
            + usize::from(self.values.is_some())
            + usize::from(self.items.is_some());
        let mut out = serializer.serialize_struct("PathFacts", fields)?;
        match &self.types {
            Some(types) => out.serialize_field("types", types)?,
            None => out.skip_field("types")?,
        }
        match &self.values {
            Some(values) => out.serialize_field("values", values.as_slice())?,
            None => out.skip_field("values")?,
        }
        match &self.items {
            Some(element) => out.serialize_field("items", &**element)?,
            None => out.skip_field("items")?,
        }
        out.end()
    }
}

/// One askable path over borrowed facts: what [`AskablePath`] writes, byte for byte, with no value
/// copied. A refusal's teaching measures a path through it, and copies only a path that fits.
#[derive(Debug, Serialize)]
pub(super) struct AskableView<'a, 's> {
    path: &'a [String],
    facts: &'a Facts<'s>,
    operators: Vec<AskOperator>,
}

impl<'a, 's> AskableView<'a, 's> {
    pub(super) fn new(path: &'a [String], facts: &'a Facts<'s>) -> AskableView<'a, 's> {
        AskableView {
            path,
            facts,
            operators: operators(facts.types.as_ref()),
        }
    }

    /// The path as the wire carries it, its facts copied.
    pub(super) fn into_askable(self) -> AskablePath {
        AskablePath {
            path: self.path.to_vec(),
            facts: self.facts.to_wire(),
            operators: self.operators,
        }
    }
}

/// What one path of an ask declares, as the walk found it. The ask's form of
/// [`PathResolution`], whose facts are copied onto the wire.
#[derive(Debug, Clone)]
pub(super) enum Resolved<'s> {
    Declared(Facts<'s>),
    NotDeclared,
    BeyondBound(BoundKind),
}

#[cfg(test)]
mod tests {
    //! The walk's reading of a field tree, row by row, each with a hand-written expected
    //! [`PathResolution`]; then its refusal of everything else, and its bounds.

    use super::*;
    use crate::types::query::filter::json_bytes;
    use serde_json::json;

    fn path(segments: &[&str]) -> Vec<String> {
        segments.iter().map(|s| s.to_string()).collect()
    }

    fn at(schema: &Value, segments: &[&str]) -> PathResolution {
        resolve(schema, &path(segments), &WalkBounds::BUILD)
    }

    fn types(ts: &[JsonType]) -> Option<BTreeSet<JsonType>> {
        Some(ts.iter().copied().collect())
    }

    fn typed(ts: &[JsonType]) -> PathResolution {
        PathResolution::Declared(PathFacts {
            types: types(ts),
            ..PathFacts::default()
        })
    }

    fn untyped() -> PathResolution {
        PathResolution::Declared(PathFacts::default())
    }

    // ── `properties` chains ──────────────────────────────────────────────────────────────────

    #[test]
    fn a_properties_chain_declares_each_level_and_nothing_beside_it() {
        let s = json!({
            "type": "object",
            "properties": {
                "measurements": {
                    "type": "object",
                    "properties": { "unit": { "type": "string" } }
                }
            }
        });
        assert_eq!(
            at(&s, &["measurements", "unit"]),
            typed(&[JsonType::String])
        );
        assert_eq!(at(&s, &["measurements"]), typed(&[JsonType::Object]));
        assert_eq!(at(&s, &["unit"]), PathResolution::NotDeclared);
        assert_eq!(
            at(&s, &["measurements", "count"]),
            PathResolution::NotDeclared
        );
    }

    #[test]
    fn a_path_under_a_scalar_property_is_not_declared() {
        let s = json!({ "properties": { "unit": { "type": "string" } } });
        assert_eq!(at(&s, &["unit", "x"]), PathResolution::NotDeclared);
    }

    // ── Boolean subschemas ───────────────────────────────────────────────────────────────────

    #[test]
    fn a_true_subschema_is_declared_and_untyped() {
        let s = json!({ "properties": { "anything": true } });
        assert_eq!(at(&s, &["anything"]), untyped());
    }

    #[test]
    fn a_true_subschema_declares_nothing_below_it() {
        let s = json!({ "properties": { "anything": true } });
        assert_eq!(at(&s, &["anything", "x"]), PathResolution::NotDeclared);
    }

    #[test]
    fn a_false_subschema_is_not_declared() {
        let s = json!({ "properties": { "never": false } });
        assert_eq!(at(&s, &["never"]), PathResolution::NotDeclared);
    }

    // ── Value sets ───────────────────────────────────────────────────────────────────────────

    #[test]
    fn enum_and_const_are_value_sets_and_together_keep_the_jsonb_equal_members() {
        let s = json!({ "properties": {
            "e": { "enum": ["ms", "s"] },
            "c": { "const": "x" },
            "both": { "enum": [1, 2, 3], "const": 2.0 },
            "neither": { "enum": ["a"], "const": "b" }
        } });
        let valued = |values: Vec<Value>| {
            PathResolution::Declared(PathFacts {
                values: Some(values),
                ..PathFacts::default()
            })
        };
        assert_eq!(at(&s, &["e"]), valued(vec![json!("ms"), json!("s")]));
        assert_eq!(at(&s, &["c"]), valued(vec![json!("x")]));
        assert_eq!(at(&s, &["both"]), valued(vec![json!(2)]));
        assert_eq!(at(&s, &["neither"]), valued(vec![]));
    }

    // ── `additionalProperties`, not walked ───────────────────────────────────────────────────

    #[test]
    fn a_path_reachable_only_through_additional_properties_is_not_declared() {
        let s = json!({
            "properties": { "a": { "type": "string" } },
            "additionalProperties": { "type": "string" }
        });
        assert_eq!(at(&s, &["a"]), typed(&[JsonType::String]));
        assert_eq!(at(&s, &["b"]), PathResolution::NotDeclared);
    }

    // ── No type and no enum ──────────────────────────────────────────────────────────────────

    #[test]
    fn a_path_with_no_type_and_no_enum_is_declared_and_unconstrained() {
        let s = json!({ "properties": { "note": {
            "description": "free", "minLength": 1, "pattern": "^x", "default": "x"
        } } });
        assert_eq!(at(&s, &["note"]), untyped());
    }

    // ── Array elements, which the `contains` array row reads ─────────────────────────────────

    #[test]
    fn an_array_path_carries_its_element_facts() {
        let s = json!({ "properties": {
            "tags": { "type": "array", "items": { "type": "string", "enum": ["a", "b"] } },
            "never": { "type": "array", "items": false },
            "any": { "type": "array", "items": true }
        } });
        assert_eq!(
            at(&s, &["tags"]),
            PathResolution::Declared(PathFacts {
                types: types(&[JsonType::Array]),
                values: None,
                items: Some(Box::new(PathFacts {
                    types: types(&[JsonType::String]),
                    values: Some(vec![json!("a"), json!("b")]),
                    items: None,
                })),
            })
        );
        assert_eq!(
            at(&s, &["never"]),
            PathResolution::Declared(PathFacts {
                types: types(&[JsonType::Array]),
                values: None,
                items: Some(Box::new(PathFacts {
                    types: types(&[]),
                    ..PathFacts::default()
                })),
            })
        );
        assert_eq!(at(&s, &["any"]), typed(&[JsonType::Array]));
    }

    #[test]
    fn element_facts_stop_at_the_depth_bound() {
        // An array of arrays of strings, two levels deep, under a depth bound of one.
        let s = json!({ "properties": { "grid": {
            "type": "array", "items": { "type": "array", "items": { "type": "string" } }
        } } });
        let one = WalkBounds {
            max_depth: 1,
            ..WalkBounds::BUILD
        };
        assert_eq!(
            resolve(&s, &path(&["grid"]), &one),
            PathResolution::Declared(PathFacts {
                types: types(&[JsonType::Array]),
                values: None,
                items: Some(Box::new(PathFacts {
                    types: types(&[JsonType::Array]),
                    ..PathFacts::default()
                })),
            })
        );
    }

    // ── Everything else is refused ───────────────────────────────────────────────────────────

    /// Every construct a field tree cannot hold, at the root and nested in each schema position
    /// the walk reads, fails `reads_whole` — so the loader judges the schema uncompilable — and a
    /// walk handed it anyway declares nothing: every path, including one the rest of the schema
    /// declares, is not declared, and the vocabulary is empty. Bites `reads_whole`: with it reading
    /// every schema as whole, the `$ref` rows resolve `a` as declared and unconstrained (what the
    /// walk once did with a `$ref` it did not follow), and the rest resolve `ok` as declared.
    #[test]
    fn a_schema_holding_a_construct_the_walk_does_not_read_declares_nothing() {
        let constructs = [
            json!({ "$ref": "#/$defs/x" }),
            json!({ "$ref": "http://169.254.169.254/latest/meta-data" }),
            json!({ "$defs": { "x": {} } }),
            json!({ "allOf": [{}] }),
            json!({ "anyOf": [{}] }),
            json!({ "oneOf": [{}] }),
            json!({ "not": {} }),
            json!({ "if": {}, "then": {} }),
            json!({ "format": "date" }),
            json!({ "prefixItems": [{}] }),
            json!({ "patternProperties": { "^a": {} } }),
            json!({ "dependentSchemas": { "z": {} } }),
            json!({ "x-extension": true }),
            json!({ "$schema": "http://json-schema.org/draft-07/schema#" }),
            json!({ "$schema": "http://json-schema.org/draft/2020-12/schema" }),
            json!({ "$schema": 7 }),
            json!({ "enum": "x" }),
            json!({ "type": 7 }),
            json!({ "type": ["string", 7] }),
        ];
        for construct in constructs {
            let positions = [
                construct.clone(),
                json!({ "properties": { "a": construct.clone() } }),
                json!({ "properties": { "a": { "items": construct.clone() } } }),
            ];
            for mut s in positions {
                s.as_object_mut()
                    .expect("an object")
                    .entry("properties")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .expect("properties is an object")
                    .insert("ok".to_string(), json!({ "type": "string" }));
                assert!(!reads_whole(&s), "{s}");
                assert_eq!(at(&s, &["ok"]), PathResolution::NotDeclared, "{s}");
                assert_eq!(at(&s, &["a"]), PathResolution::NotDeclared, "{s}");
                assert_eq!(
                    enumerate(&s, &WalkBounds::BUILD),
                    Vocabulary::default(),
                    "{s}"
                );
            }
        }
    }

    #[test]
    fn schema_and_id_are_read_at_the_root_and_refused_below_it() {
        let root = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/shape",
            "properties": { "a": { "type": "string" } }
        });
        assert_eq!(at(&root, &["a"]), typed(&[JsonType::String]));
        let trailing = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema#",
            "properties": { "a": {} }
        });
        assert_eq!(at(&trailing, &["a"]), untyped());
        for keyword in ["$schema", "$id"] {
            let below = json!({ "properties": { "a": {
                keyword: "https://json-schema.org/draft/2020-12/schema"
            } } });
            assert!(!reads_whole(&below), "{below}");
            assert_eq!(at(&below, &["a"]), PathResolution::NotDeclared, "{below}");
        }
    }

    #[test]
    fn a_value_that_is_not_a_schema_where_a_schema_sits_is_refused() {
        for s in [
            json!("string"),
            json!({ "properties": { "a": 7 } }),
            json!({ "properties": ["a"] }),
            json!({ "properties": { "a": { "items": [{}] } } }),
        ] {
            assert!(!reads_whole(&s), "{s}");
            assert_eq!(at(&s, &["a"]), PathResolution::NotDeclared, "{s}");
        }
    }

    /// What `additionalProperties` and the data keywords hold is not a schema the walk reads, so a
    /// construct there is not refused: a `$ref` as a `default` is a value, and the walk never
    /// descends `additionalProperties`. Bites a `reads_whole` that pushed every object it met.
    #[test]
    fn data_and_an_unwalked_additional_properties_are_not_read_for_constructs() {
        let s = json!({
            "properties": {
                "a": {
                    "default": { "$ref": "#" },
                    "enum": [{ "allOf": [] }],
                    "const": { "allOf": [] },
                    "examples": [{ "$ref": "#" }]
                }
            },
            "additionalProperties": { "$ref": "#" }
        });
        assert!(matches!(at(&s, &["a"]), PathResolution::Declared(_)));
    }

    // ── The bounds ───────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_path_deeper_than_the_walk_is_beyond_the_depth_bound() {
        let s = json!({ "properties": { "a": {} } });
        let deep: Vec<String> = (0..=MAX_WALK_DEPTH).map(|i| format!("k{i}")).collect();
        assert_eq!(
            resolve(&s, &deep, &WalkBounds::BUILD),
            PathResolution::BeyondBound(BoundKind::Depth)
        );
    }

    #[test]
    fn a_schema_over_the_byte_bound_is_not_walked() {
        let s = json!({ "properties": { "a": { "description": "x".repeat(64) } } });
        let small = WalkBounds {
            max_schema_bytes: 32,
            ..WalkBounds::BUILD
        };
        assert_eq!(
            resolve(&s, &path(&["a"]), &small),
            PathResolution::BeyondBound(BoundKind::SchemaBytes)
        );
        assert_eq!(enumerate(&s, &small), unwalked(BoundKind::SchemaBytes));
    }

    /// A chain of `n` nested objects, each declaring `v` and `child`.
    fn chain(n: usize) -> Value {
        let mut node = json!({ "properties": { "v": { "type": "integer" } } });
        for _ in 0..n {
            node = json!({ "properties": { "v": { "type": "integer" }, "child": node } });
        }
        node
    }

    #[test]
    fn a_tree_deeper_than_the_walk_yields_its_depth_bounded_paths_and_says_so() {
        let three = WalkBounds {
            max_depth: 3,
            ..WalkBounds::BUILD
        };
        let node = PathFacts::default();
        let v = PathFacts {
            types: types(&[JsonType::Integer]),
            ..PathFacts::default()
        };
        assert_eq!(
            enumerate(&chain(5), &three),
            Vocabulary {
                paths: vec![
                    AskablePath::new(path(&["child"]), node.clone()),
                    AskablePath::new(path(&["v"]), v.clone()),
                    AskablePath::new(path(&["child", "child"]), node.clone()),
                    AskablePath::new(path(&["child", "v"]), v.clone()),
                    AskablePath::new(path(&["child", "child", "child"]), node),
                    AskablePath::new(path(&["child", "child", "v"]), v),
                ],
                truncated: BTreeSet::from([BoundKind::Depth]),
            }
        );
        // A tree exactly as deep as the bound is whole: `child`, `v`, `child.child`, `child.v` and
        // `child.child.v`.
        let whole = enumerate(&chain(2), &three);
        assert!(whole.truncated.is_empty(), "{:?}", whole.truncated);
        assert_eq!(whole.paths.len(), 5);
    }

    #[test]
    fn a_shape_declaring_more_paths_than_are_advertised_is_cut_breadth_first_and_says_so() {
        let fields: Map<String, Value> = (0..=MAX_WALK_PATHS)
            .map(|i| (format!("f{i:05}"), json!({ "properties": { "x": {} } })))
            .collect();
        let s = json!({ "properties": fields });
        let v = enumerate(&s, &WalkBounds::BUILD);
        assert_eq!(v.truncated, BTreeSet::from([BoundKind::Paths]));
        assert_eq!(v.paths.len(), MAX_WALK_PATHS);
        // Breadth-first: every path is at the shallowest level, in byte order.
        assert!(v.paths.iter().all(|p| p.path.len() == 1));
        assert_eq!(v.paths[0].path, path(&["f00000"]));
    }

    #[test]
    fn a_finite_schema_enumerates_whole_with_its_operators() {
        let s = json!({ "properties": {
            "unit": { "type": "string", "enum": ["ms"] },
            "flag": { "type": "boolean" },
            "never": false
        } });
        assert_eq!(
            enumerate(&s, &WalkBounds::BUILD),
            Vocabulary {
                paths: vec![
                    AskablePath {
                        path: path(&["flag"]),
                        facts: PathFacts {
                            types: types(&[JsonType::Boolean]),
                            ..PathFacts::default()
                        },
                        operators: vec![AskOperator::HasKey, AskOperator::Contains],
                    },
                    AskablePath {
                        path: path(&["unit"]),
                        facts: PathFacts {
                            types: types(&[JsonType::String]),
                            values: Some(vec![json!("ms")]),
                            items: None,
                        },
                        operators: vec![
                            AskOperator::HasKey,
                            AskOperator::Contains,
                            AskOperator::Compare
                        ],
                    },
                ],
                truncated: BTreeSet::new(),
            }
        );
    }

    /// A shape whose enums fill the schema is advertised whole: every path with every member, cut
    /// at no bound. The walk's work is bounded by the schema's size alone, so no shape within
    /// [`MAX_SCHEMA_BYTES`] meets a cut but `depth` or `paths`. The fixture is one a field tree can
    /// be — each enum of 256 strings, the most the compile admits — so it bites a per-read work or
    /// copy budget on a shape that compiles: the visit budget this walk replaced charged each such
    /// path about 770 visits and cut this vocabulary at `work` after about 85 of its paths.
    #[test]
    fn a_vocabulary_whose_enums_fill_the_schema_is_advertised_whole() {
        let codes = 256;
        let members: Vec<Value> = (0..codes).map(|c| json!(format!("c{c:05}"))).collect();
        let field = json!({ "enum": members });
        let fields = (MAX_SCHEMA_BYTES - 256) / json_bytes(&json!({ "f0000": field }));
        assert!(fields > 85, "past where the visit budget cut: {fields}");
        let properties: Map<String, Value> = (0..fields)
            .map(|f| (format!("f{f:04}"), field.clone()))
            .collect();
        let s = json!({ "type": "object", "properties": properties });
        assert!(
            within_schema_bound(&s),
            "the fixture is within the walk's bound"
        );
        let vocabulary = enumerate(&s, &WalkBounds::BUILD);
        assert!(
            vocabulary.truncated.is_empty(),
            "{:?}",
            vocabulary.truncated
        );
        assert_eq!(vocabulary.paths.len(), fields);
        for path in &vocabulary.paths {
            let values = path.facts.values.as_ref().expect("an enum declares values");
            assert_eq!(values.len(), codes, "{:?}", path.path);
        }
    }

    /// Every path a vocabulary advertises resolves to the facts it advertises, so a caller who
    /// asks what the vocabulary listed is never told something else. Bites any drift between the
    /// enumeration's descent and `reach`'s.
    #[test]
    fn every_advertised_path_resolves_to_what_it_advertises() {
        let s = json!({ "properties": {
            "a": { "type": "object", "properties": {
                "b": { "enum": [1, 2] },
                "c": { "type": "array", "items": { "const": "x" } },
                "d": true,
                "e": false
            } },
            "f": { "type": ["string", "null"] }
        } });
        let v = enumerate(&s, &WalkBounds::BUILD);
        assert_eq!(v.paths.len(), 5, "{v:?}");
        for advertised in v.paths {
            assert_eq!(
                resolve(&s, &advertised.path, &WalkBounds::BUILD),
                PathResolution::Declared(advertised.facts),
                "{:?}",
                advertised.path
            );
        }
    }

    /// A key too long to ask, beside one that is not, is never advertised and nothing below it is
    /// walked. Bites the key skip in `child_properties`: without it the vocabulary advertises the
    /// long key and the path below it.
    #[test]
    fn a_key_too_long_to_ask_is_never_advertised_nor_walked_below() {
        let mut props = Map::new();
        props.insert(
            "a".repeat(MAX_ARTIFACT_PATH_SEGMENT_BYTES + 1),
            json!({ "properties": { "x": {} } }),
        );
        props.insert("ok".to_string(), json!({ "type": "string" }));
        let s = json!({ "properties": props });
        assert_eq!(
            enumerate(&s, &WalkBounds::BUILD),
            Vocabulary {
                paths: vec![AskablePath::new(
                    path(&["ok"]),
                    PathFacts {
                        types: types(&[JsonType::String]),
                        ..PathFacts::default()
                    }
                )],
                truncated: BTreeSet::new(),
            }
        );
    }

    /// The schema is measured only until it passes the bound. Serializing an array of
    /// 100,000 numbers writes each number on its own, so the count stops within one number of the
    /// limit. Bites the stopping sink in `json_bytes_up_to`: a sink that never stops counts the
    /// whole value, over half a megabyte.
    #[test]
    fn measuring_a_schema_stops_once_it_passes_the_bound() {
        let numbers: Vec<Value> = (0..100_000).map(|i| json!(i)).collect();
        let s = json!({ "enum": numbers });
        let whole = json_bytes(&s);
        assert!(whole > 500_000, "{whole}");
        let measured = json_bytes_up_to(&s, 1_024);
        assert!(measured > 1_024 && measured <= 1_024 + 8, "{measured}");
        assert_eq!(json_bytes_up_to(&s, whole), whole, "exact within the limit");
    }

    /// The borrowed view writes exactly the bytes of the copy it stands for —
    /// every field present only when set, at every items level, over enum members, a `const`, an
    /// enum and a `const` together, and `items: false` — so measuring the view measures the copy.
    /// Bites any drift between `Facts`'s `Serialize` and `PathFacts`'s derived one.
    #[test]
    fn the_borrowed_view_writes_the_wire_bytes() {
        let codes = json!({ "enum": ["ms", 7, null, { "k": [1, 2.5] }, ["x"]] });
        let s = json!({
            "properties": {
                "plain": {},
                "typed": { "type": ["integer", "null"] },
                "coded": codes,
                "constant": { "type": "string", "const": "x" },
                "both": { "enum": ["x", "y"], "const": "x" },
                "grid": { "type": "array", "items": {
                    "type": "array", "items": codes
                } },
                "never": { "type": "array", "items": false }
            }
        });
        let walk = Walk::new(&s, &WalkBounds::BUILD).expect("a field tree under the byte bound");
        let names = [
            "plain", "typed", "coded", "constant", "both", "grid", "never",
        ];
        for name in names {
            let asked = path(&[name]);
            let Resolved::Declared(facts) = walk.reach(&asked) else {
                panic!("{name} is declared");
            };
            let view = AskableView::new(&asked, &facts);
            let wire = AskablePath::new(asked.clone(), facts.to_wire());
            let viewed = serde_json::to_vec(&view).expect("the view serializes");
            assert_eq!(
                viewed,
                serde_json::to_vec(&wire).expect("the wire serializes"),
                "{name}"
            );
            assert_eq!(json_bytes_up_to(&view, usize::MAX), viewed.len());
            assert_eq!(view.into_askable(), wire);
        }
    }
}
