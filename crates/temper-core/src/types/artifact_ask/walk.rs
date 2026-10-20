//! The bounded schema walk — what a shape declares at a path, decided over a `serde_json::Value`
//! with no JSON Schema validator (spec §2.4, §3.3).
//!
//! **It walks exactly the constructs of the §3.3 table, and nothing more.** `properties` chains;
//! a local `$ref` into `#/$defs/…` or `#/definitions/…`; boolean subschemas; `allOf` (merged);
//! `anyOf` / `oneOf` (unioned). `additionalProperties`, `patternProperties`,
//! `unevaluatedProperties`, `if`/`then`/`else`, `dependentSchemas`, and every `$ref` that is not a
//! local definition are **not walked**, so a path reachable only through them is not declared — the
//! declared hole of spec §11, refused as `path_outside_shape` rather than guessed at.
//!
//! **The walk sees only schemas that compiled.** The loader (spec §2.4) compiled the schema with
//! the no-fetch compiler before handing it here, so an unrecognised `$schema` or a remote `$ref` has
//! already been decided `Uncompilable`. What this module reads `$schema` for is the one place the
//! drafts differ for it: whether keywords beside a `$ref` apply (2019-09 and 2020-12) or are
//! ignored (draft-04/06/07), and whether `const` and `prefixItems` exist. The draft names are the
//! ones `jsonschema` recognises (`referencing::Draft::from_schema_uri`); an absent `$schema` is
//! 2020-12, its default.
//!
//! **Three bounds, each named** ([`BoundKind`]): a schema is written by any author of its home and
//! read by every reader of it, and a cycle guard alone does not bound the walk — a non-cyclic
//! `$defs` graph in which each definition references the next twice declares 2^N paths from a few
//! kilobytes (§3.3, *The walk is bounded*). [`resolve`] walks one path lazily and never enumerates;
//! its work is bounded by memoising each `(subschema, path position)` pair, so a fan-out that
//! repeats a definition costs its size once rather than 2^N times. [`enumerate`] produces the
//! advertised vocabulary and stops at the path budget, saying so.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::reference::jsonb_eq;
use crate::types::query::filter::json_bytes;

/// The deepest path, in object-key segments, the walk descends to. A deeper ask is
/// [`PathResolution::BeyondBound`]`(`[`BoundKind::Depth`]`)`, refused `path_outside_shape` with
/// `teaches` naming the bound (spec §2.5).
///
/// # 16
///
/// It bounds a legitimately recursive schema — a tree through `properties` declares unboundedly
/// many paths, and this is where its vocabulary stops. Structured content in a shape is a record of
/// a few levels; 16 levels of object nesting is far past any shape a person writes, and the bound
/// is what makes a recursive one finite.
pub const MAX_WALK_DEPTH: usize = 16;

/// The most paths [`enumerate`] advertises. Past it the vocabulary is truncated at
/// [`BoundKind::Paths`], visibly (spec §2.5).
///
/// # 1024
///
/// The vocabulary is read whole by `schema show`, `GET /api/shapes/{id}` and
/// `get_data_artifact_shape`, so this bounds a response body and the walk behind it. A path entry
/// is a few hundred bytes, so 1024 of them is a few hundred kilobytes at most. It is far above a
/// shape a person writes — a record with tens of fields — and it is the bound the 2^N `$defs`
/// fan-out meets.
pub const MAX_WALK_PATHS: usize = 1024;

/// The largest schema the walk reads, in compact serialized JSON bytes. Past it nothing is walked:
/// every path is [`PathResolution::BeyondBound`]`(`[`BoundKind::SchemaBytes`]`)` and the
/// vocabulary is empty and says why.
///
/// # 256 KiB
///
/// Today a schema is bounded only by the 25 MiB gated request body (spec §3.3). A JSON Schema for
/// one family of structured records is kilobytes; 256 KiB admits two orders of magnitude more than
/// that and keeps every walk — and every memo the walk keeps — small.
pub const MAX_SCHEMA_BYTES: usize = 256 * 1024;

/// The walk's three bounds. The values the build runs are [`WalkBounds::BUILD`]; tests construct
/// smaller ones to reach a bound without a large schema.
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

/// Which walk bound was met. A truncated vocabulary and a refusal both name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BoundKind {
    /// [`WalkBounds::max_depth`] — the path is deeper than the walk descends.
    Depth,
    /// [`WalkBounds::max_paths`] — the vocabulary has more paths than are advertised.
    Paths,
    /// [`WalkBounds::max_schema_bytes`] — the schema is larger than the walk reads.
    SchemaBytes,
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
    /// The declared JSON types. `None`: no `type` constrains the path. An empty set: the branches
    /// that apply here declare types with nothing in common, so no value is admissible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<BTreeSet<JsonType>>,
    /// The `enum` / `const` values. `None`: no value set constrains the path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<Value>>,
    /// What an element of an array at this path declares. `None`: elements are unconstrained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "web-api", schema(no_recursion))]
    pub items: Option<Box<PathFacts>>,
}

impl PathFacts {
    /// Whether nothing at all is declared — the facts of `{}` or `true`.
    fn is_unconstrained(&self) -> bool {
        self.types.is_none() && self.values.is_none() && self.items.is_none()
    }

    /// Whether `contains` can admit any value here: the path is untyped, or declares some type.
    pub fn admits_contains(&self) -> bool {
        self.types.as_ref().is_none_or(|t| !t.is_empty())
    }

    /// Whether `compare` is feasible here: the path declares `number`, `integer` or `string`, or
    /// declares no type (spec §3.4, the `compare` row).
    pub fn admits_compare(&self) -> bool {
        self.types.as_ref().is_none_or(|t| {
            t.iter()
                .any(|t| matches!(t, JsonType::Number | JsonType::Integer | JsonType::String))
        })
    }

    /// The operators feasible at this path, in [`AskOperator`] order.
    pub fn operators(&self) -> Vec<AskOperator> {
        let mut ops = vec![AskOperator::HasKey];
        if self.admits_contains() {
            ops.push(AskOperator::Contains);
        }
        if self.admits_compare() {
            ops.push(AskOperator::Compare);
        }
        ops
    }
}

/// The name of a [`PropertyOp`](crate::types::query::PropertyOp) operator, for a vocabulary to
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

/// What a shape lets a caller ask: its askable paths, and whether the list was cut at a bound.
///
/// `truncated` is the walk bound met, never a silent short list (spec §2.5): a recursive tree
/// through `properties` is `Depth`, a 2^N `$defs` fan-out is `Paths`, an oversized schema is
/// `SchemaBytes` with no paths.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Vocabulary {
    pub paths: Vec<AskablePath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<BoundKind>,
}

/// Where one asked path stands against the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathResolution {
    /// Reached through `properties` at every segment, and not forbidden by a `false`.
    Declared(PathFacts),
    /// Not reached through the walked constructs, or forbidden (`false`).
    NotDeclared,
    /// The walk stopped at a bound before it could say.
    BeyondBound(BoundKind),
}

/// Resolve one path against a schema. Lazy: one walk down the schema, never an enumeration.
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
        Err(bound) => Vocabulary {
            paths: Vec::new(),
            truncated: Some(bound),
        },
    }
}

/// The draft a schema is applied under, as far as the walk needs to know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Draft {
    /// draft-04
    D4,
    /// draft-06
    D6,
    /// draft-07
    D7,
    /// 2019-09
    D2019,
    /// 2020-12
    D2020,
}

impl Draft {
    /// The draft `jsonschema` selects for a root schema: `referencing::Draft::from_schema_uri` over
    /// `$schema` with a trailing `#` trimmed, and 2020-12 when there is none. A URI it does not
    /// recognise never reaches the walk (the loader refused it), and is read as 2020-12, which is
    /// also how `referencing` treats its unknown arm.
    fn of(root: &Value) -> Draft {
        let uri = root.get("$schema").and_then(Value::as_str).unwrap_or("");
        match uri.trim_end_matches('#') {
            "https://json-schema.org/draft/2019-09/schema"
            | "http://json-schema.org/draft/2019-09/schema" => Draft::D2019,
            "https://json-schema.org/draft-07/schema"
            | "http://json-schema.org/draft-07/schema" => Draft::D7,
            "https://json-schema.org/draft-06/schema"
            | "http://json-schema.org/draft-06/schema" => Draft::D6,
            "https://json-schema.org/draft-04/schema"
            | "http://json-schema.org/draft-04/schema" => Draft::D4,
            _ => Draft::D2020,
        }
    }

    /// Before 2019-09 a `$ref` replaces its whole schema object; its siblings are ignored.
    fn ref_overrides_siblings(self) -> bool {
        matches!(self, Draft::D4 | Draft::D6 | Draft::D7)
    }

    /// `const` arrived in draft-06; draft-04 ignores it as an unknown keyword.
    fn has_const(self) -> bool {
        !matches!(self, Draft::D4)
    }

    /// `prefixItems` arrived in 2020-12; earlier drafts ignore it.
    fn has_prefix_items(self) -> bool {
        matches!(self, Draft::D2020)
    }
}

/// One schema, measured and its draft selected, ready to resolve paths against.
#[derive(Debug)]
pub(super) struct Walk<'s> {
    root: &'s Value,
    draft: Draft,
    bounds: WalkBounds,
}

impl<'s> Walk<'s> {
    /// Measure the schema once. Over [`WalkBounds::max_schema_bytes`] nothing is walked.
    pub(super) fn new(schema: &'s Value, bounds: &WalkBounds) -> Result<Walk<'s>, BoundKind> {
        if json_bytes(schema) > bounds.max_schema_bytes {
            return Err(BoundKind::SchemaBytes);
        }
        Ok(Walk {
            root: schema,
            draft: Draft::of(schema),
            bounds: *bounds,
        })
    }

    pub(super) fn resolve(&self, path: &[String]) -> PathResolution {
        if path.len() > self.bounds.max_depth {
            return PathResolution::BeyondBound(BoundKind::Depth);
        }
        let mut resolver = Resolver {
            walk: self,
            path,
            memo: HashMap::new(),
            active: HashSet::new(),
        };
        match resolver.reach(self.root, 0) {
            Reach::Declared(facts) => PathResolution::Declared(facts),
            Reach::Silent | Reach::Forbidden => PathResolution::NotDeclared,
        }
    }

    /// Breadth-first over property keys, shallowest paths first and keys in byte order within a
    /// level, so a truncated vocabulary is the shallow part of the full one.
    pub(super) fn enumerate(&self) -> Vocabulary {
        let mut paths = Vec::new();
        let mut truncated = None;
        let mut queue: VecDeque<(Vec<String>, Vec<&'s Value>)> = VecDeque::new();
        queue.push_back((Vec::new(), self.frame([self.root])));
        while let Some((prefix, frame)) = queue.pop_front() {
            let keys = child_keys(&frame);
            if keys.is_empty() {
                continue;
            }
            if prefix.len() == self.bounds.max_depth {
                // Keys exist below the bound. Whether each is declared is past the bound too, so
                // this says "there may be more" — the honest reading of a depth cut.
                truncated.get_or_insert(BoundKind::Depth);
                continue;
            }
            for key in keys {
                let mut path = prefix.clone();
                path.push(key.to_string());
                let PathResolution::Declared(facts) = self.resolve(&path) else {
                    continue;
                };
                if paths.len() == self.bounds.max_paths {
                    return Vocabulary {
                        paths,
                        truncated: Some(BoundKind::Paths),
                    };
                }
                let children = frame
                    .iter()
                    .copied()
                    .filter_map(|node| node.get("properties").and_then(|p| p.get(key)));
                queue.push_back((path.clone(), self.frame(children)));
                paths.push(AskablePath::new(path, facts));
            }
        }
        Vocabulary { paths, truncated }
    }

    /// Every schema object whose `properties` apply at one location: the given nodes and what
    /// their `$ref`, `allOf`, `anyOf` and `oneOf` reach, each once. Deduplicated by identity, so a
    /// definition referenced from many places joins the frame once.
    fn frame(&self, nodes: impl IntoIterator<Item = &'s Value>) -> Vec<&'s Value> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for node in nodes {
            self.expand(node, &mut out, &mut seen);
        }
        out
    }

    fn expand(&self, node: &'s Value, out: &mut Vec<&'s Value>, seen: &mut HashSet<usize>) {
        let Value::Object(map) = node else {
            return;
        };
        if !seen.insert(identity(node)) {
            return;
        }
        if let Some(target) = map.get("$ref").and_then(|r| self.target(r)) {
            self.expand(target, out, seen);
        }
        if self.draft.ref_overrides_siblings() && map.contains_key("$ref") {
            return;
        }
        out.push(node);
        for keyword in ["allOf", "anyOf", "oneOf"] {
            for branch in map
                .get(keyword)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                self.expand(branch, out, seen);
            }
        }
    }

    /// The subschema a local `$ref` names: `#/$defs/…` or `#/definitions/…`, as a JSON pointer.
    /// Anything else is not walked (§3.3, the remote-`$ref` row).
    fn target(&self, reference: &Value) -> Option<&'s Value> {
        let reference = reference.as_str()?;
        let pointer = reference.strip_prefix('#')?;
        if !(pointer.starts_with("/$defs/") || pointer.starts_with("/definitions/")) {
            return None;
        }
        self.root.pointer(pointer)
    }
}

/// The property keys any schema object in a frame declares, in byte order.
fn child_keys<'s>(frame: &[&'s Value]) -> BTreeSet<&'s str> {
    frame
        .iter()
        .copied()
        .filter_map(|node| node.get("properties").and_then(Value::as_object))
        .flat_map(|props| props.keys().map(String::as_str))
        .collect()
}

/// A subschema's identity for memoising and cycle-guarding: its address inside the borrowed root,
/// which is stable for the walk's lifetime.
fn identity(node: &Value) -> usize {
    std::ptr::from_ref(node) as usize
}

/// What one subschema says about the remaining path.
#[derive(Debug, Clone)]
enum Reach {
    /// The path can never be present (`false`, or every branch of a union forbids it).
    Forbidden,
    /// This subschema does not reach the path; it neither declares nor constrains it.
    Silent,
    Declared(PathFacts),
}

/// Both hold — `allOf`, and keywords beside a `$ref` under 2019-09 / 2020-12.
fn conjoin(a: Reach, b: Reach) -> Reach {
    match (a, b) {
        (Reach::Forbidden, _) | (_, Reach::Forbidden) => Reach::Forbidden,
        (Reach::Silent, other) | (other, Reach::Silent) => other,
        (Reach::Declared(x), Reach::Declared(y)) => Reach::Declared(conjoin_facts(x, y)),
    }
}

/// One of them holds — `anyOf` / `oneOf` (§3.3 row 6). A branch that does not declare the path
/// takes no part; if none declares it the union is silent, unless every branch forbids it.
fn disjoin(branches: Vec<Reach>) -> Reach {
    let all_forbidden =
        !branches.is_empty() && branches.iter().all(|b| matches!(b, Reach::Forbidden));
    let declared = branches.into_iter().filter_map(|b| match b {
        Reach::Declared(facts) => Some(facts),
        Reach::Silent | Reach::Forbidden => None,
    });
    match declared.reduce(disjoin_facts) {
        Some(facts) => Reach::Declared(facts),
        None if all_forbidden => Reach::Forbidden,
        None => Reach::Silent,
    }
}

fn conjoin_facts(a: PathFacts, b: PathFacts) -> PathFacts {
    PathFacts {
        types: both(a.types, b.types, intersect_types),
        values: both(a.values, b.values, |x, y| {
            x.into_iter()
                .filter(|v| y.iter().any(|w| jsonb_eq(v, w)))
                .collect()
        }),
        items: both(a.items, b.items, |x, y| Box::new(conjoin_facts(*x, *y))),
    }
}

/// A union's facts: unconstrained if either side is — so a value one branch admits is never
/// refused because another branch enumerates (§3.3 row 6).
fn disjoin_facts(a: PathFacts, b: PathFacts) -> PathFacts {
    PathFacts {
        types: either(a.types, b.types, |mut x, y| {
            x.extend(y);
            x
        }),
        values: either(a.values, b.values, |mut x, y| {
            for v in y {
                if !x.iter().any(|w| jsonb_eq(&v, w)) {
                    x.push(v);
                }
            }
            x
        }),
        items: either(a.items, b.items, |x, y| Box::new(disjoin_facts(*x, *y))),
    }
}

/// Both constraints apply: an absent one leaves the other.
fn both<T>(a: Option<T>, b: Option<T>, merge: impl FnOnce(T, T) -> T) -> Option<T> {
    match (a, b) {
        (Some(x), Some(y)) => Some(merge(x, y)),
        (x, None) | (None, x) => x,
    }
}

/// Either constraint applies: an absent one is no constraint at all.
fn either<T>(a: Option<T>, b: Option<T>, merge: impl FnOnce(T, T) -> T) -> Option<T> {
    match (a, b) {
        (Some(x), Some(y)) => Some(merge(x, y)),
        _ => None,
    }
}

/// `integer` is the integral subset of `number`, so `number` ∧ `integer` is `integer`.
fn intersect_types(a: BTreeSet<JsonType>, b: BTreeSet<JsonType>) -> BTreeSet<JsonType> {
    let mut out: BTreeSet<JsonType> = a.intersection(&b).copied().collect();
    let crosses = |x: &BTreeSet<JsonType>, y: &BTreeSet<JsonType>| {
        x.contains(&JsonType::Integer) && y.contains(&JsonType::Number)
    };
    if crosses(&a, &b) || crosses(&b, &a) {
        out.insert(JsonType::Integer);
    }
    out
}

/// One path's walk, memoised by `(subschema, position)` and cycle-guarded on the same key.
///
/// **A cycle that consumes no segment adds nothing.** `A = {allOf: [{$ref: B}]}`,
/// `B = {allOf: [{$ref: A}]}` re-enters `A` at the same position; the re-entry is silent, so the
/// facts are those reached before the cycle closed. The direction that loses is lenient — an
/// admitted value runs and is an honest empty — never a refusal of something declared.
struct Resolver<'w, 's> {
    walk: &'w Walk<'s>,
    path: &'w [String],
    memo: HashMap<(usize, usize), Reach>,
    active: HashSet<(usize, usize)>,
}

impl<'s> Resolver<'_, 's> {
    fn reach(&mut self, node: &'s Value, at: usize) -> Reach {
        let key = (identity(node), at);
        if let Some(known) = self.memo.get(&key) {
            return known.clone();
        }
        if !self.active.insert(key) {
            return Reach::Silent;
        }
        let reached = self.reach_uncached(node, at);
        self.active.remove(&key);
        self.memo.insert(key, reached.clone());
        reached
    }

    fn reach_uncached(&mut self, node: &'s Value, at: usize) -> Reach {
        match node {
            Value::Bool(false) => Reach::Forbidden,
            // `true` declares the path it sits at and nothing below it: a deeper segment is not
            // reached through `properties`.
            Value::Bool(true) if at == self.path.len() => Reach::Declared(PathFacts::default()),
            Value::Object(map) => self.reach_object(map, at),
            _ => Reach::Silent,
        }
    }

    fn reach_object(&mut self, map: &'s Map<String, Value>, at: usize) -> Reach {
        // Copied out of `self` so the segment borrows the path, not the resolver.
        let path = self.path;
        let by_ref = match map.get("$ref") {
            // Before 2019-09 the `$ref` is the whole schema object.
            Some(reference) if self.walk.draft.ref_overrides_siblings() => {
                return self.reach_ref(reference, at);
            }
            Some(reference) => Some(self.reach_ref(reference, at)),
            None => None,
        };
        let mut acc = if at == path.len() {
            Reach::Declared(self.local_facts(map, at))
        } else {
            match map.get("properties").and_then(|p| p.get(path[at].as_str())) {
                Some(child) => self.reach(child, at + 1),
                None => Reach::Silent,
            }
        };
        if let Some(by_ref) = by_ref {
            acc = conjoin(acc, by_ref);
        }
        for branch in map
            .get("allOf")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let branch = self.reach(branch, at);
            acc = conjoin(acc, branch);
        }
        for keyword in ["anyOf", "oneOf"] {
            if let Some(branches) = map.get(keyword).and_then(Value::as_array) {
                let union = branches.iter().map(|b| self.reach(b, at)).collect();
                acc = conjoin(acc, disjoin(union));
            }
        }
        acc
    }

    /// A `$ref` the walk does not follow constrains nothing it can see.
    fn reach_ref(&mut self, reference: &Value, at: usize) -> Reach {
        match self.walk.target(reference) {
            Some(target) => self.reach(target, at),
            None => Reach::Silent,
        }
    }

    /// What one schema object itself declares at the path's end: `type`, `enum` / `const`, and its
    /// element schema.
    fn local_facts(&mut self, map: &'s Map<String, Value>, at: usize) -> PathFacts {
        let types = map.get("type").map(|t| match t {
            Value::String(name) => JsonType::from_keyword(name).into_iter().collect(),
            Value::Array(names) => names
                .iter()
                .filter_map(|n| n.as_str().and_then(JsonType::from_keyword))
                .collect(),
            _ => BTreeSet::new(),
        });
        let enumerated = map.get("enum").and_then(Value::as_array).cloned();
        let constant = map
            .get("const")
            .filter(|_| self.walk.draft.has_const())
            .map(|c| vec![c.clone()]);
        PathFacts {
            types,
            values: both(enumerated, constant, |x, y| {
                x.into_iter()
                    .filter(|v| y.iter().any(|w| jsonb_eq(v, w)))
                    .collect()
            }),
            items: self.item_facts(map, at),
        }
    }

    /// The facts of one array element, when `items` is a single schema that applies to every
    /// element. A tuple (`items` as an array before 2020-12, or `prefixItems` in it) gives each
    /// position its own schema, so an element anywhere is unconstrained here.
    fn item_facts(&mut self, map: &'s Map<String, Value>, at: usize) -> Option<Box<PathFacts>> {
        if self.walk.draft.has_prefix_items() && map.contains_key("prefixItems") {
            return None;
        }
        let items = map
            .get("items")
            .filter(|i| i.is_object() || i.is_boolean())?;
        match self.reach(items, at) {
            // `items: false` admits no element at all.
            Reach::Forbidden => Some(Box::new(PathFacts {
                types: Some(BTreeSet::new()),
                ..PathFacts::default()
            })),
            Reach::Silent => None,
            Reach::Declared(facts) if facts.is_unconstrained() => None,
            Reach::Declared(facts) => Some(Box::new(facts)),
        }
    }
}

#[cfg(test)]
mod tests {
    //! The §3.3 table, row by row, each with a hand-written expected [`PathResolution`].

    use super::*;
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

    // ── Row 1: `properties` chains ───────────────────────────────────────────────────────────

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

    // ── Row 2: draft selection, and siblings beside `$ref` ────────────────────────────────────

    #[test]
    fn under_draft_07_keywords_beside_a_ref_are_ignored() {
        let s = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "definitions": { "d": { "properties": { "x": { "type": "string" } } } },
            "properties": {
                "a": { "$ref": "#/definitions/d", "properties": { "y": { "type": "number" } } }
            }
        });
        assert_eq!(at(&s, &["a", "x"]), typed(&[JsonType::String]));
        assert_eq!(at(&s, &["a", "y"]), PathResolution::NotDeclared);
    }

    #[test]
    fn under_2020_12_keywords_beside_a_ref_apply_with_it() {
        let s = json!({
            "$defs": { "d": { "properties": { "x": { "type": "string" } } } },
            "properties": {
                "a": { "$ref": "#/$defs/d", "properties": { "y": { "type": "number" } } }
            }
        });
        assert_eq!(at(&s, &["a", "x"]), typed(&[JsonType::String]));
        assert_eq!(at(&s, &["a", "y"]), typed(&[JsonType::Number]));
    }

    #[test]
    fn under_2019_09_keywords_beside_a_ref_apply_with_it() {
        let s = json!({
            "$schema": "https://json-schema.org/draft/2019-09/schema",
            "$defs": { "d": { "type": "string" } },
            "properties": { "a": { "$ref": "#/$defs/d", "enum": ["x"] } }
        });
        assert_eq!(
            at(&s, &["a"]),
            PathResolution::Declared(PathFacts {
                types: types(&[JsonType::String]),
                values: Some(vec![json!("x")]),
                items: None,
            })
        );
    }

    #[test]
    fn const_is_a_value_set_from_draft_06_and_ignored_by_draft_04() {
        let draft6 = json!({
            "$schema": "http://json-schema.org/draft-06/schema#",
            "properties": { "a": { "const": "x" } }
        });
        assert_eq!(
            at(&draft6, &["a"]),
            PathResolution::Declared(PathFacts {
                values: Some(vec![json!("x")]),
                ..PathFacts::default()
            })
        );
        let draft4 = json!({
            "$schema": "http://json-schema.org/draft-04/schema#",
            "properties": { "a": { "const": "x" } }
        });
        assert_eq!(at(&draft4, &["a"]), untyped());
    }

    // ── Row 3: local `$ref`, with a cycle guard ───────────────────────────────────────────────

    #[test]
    fn a_ref_into_defs_is_followed() {
        let s = json!({
            "$defs": { "unit": { "type": "string", "enum": ["ms", "s"] } },
            "properties": { "u": { "$ref": "#/$defs/unit" } }
        });
        assert_eq!(
            at(&s, &["u"]),
            PathResolution::Declared(PathFacts {
                types: types(&[JsonType::String]),
                values: Some(vec![json!("ms"), json!("s")]),
                items: None,
            })
        );
    }

    #[test]
    fn a_ref_cycle_that_consumes_no_segment_terminates() {
        let s = json!({
            "$defs": {
                "a": { "$ref": "#/$defs/b", "type": "string" },
                "b": { "$ref": "#/$defs/a" }
            },
            "properties": { "p": { "$ref": "#/$defs/a" } }
        });
        assert_eq!(at(&s, &["p"]), typed(&[JsonType::String]));
    }

    #[test]
    fn a_definition_doubled_at_every_level_resolves_one_path_without_fanning_out() {
        // Each definition conjoins the next one twice: 2^64 visits unmemoised.
        let mut defs = Map::new();
        for i in 0..64 {
            defs.insert(
                format!("d{i}"),
                json!({ "allOf": [
                    { "$ref": format!("#/$defs/d{}", i + 1) },
                    { "$ref": format!("#/$defs/d{}", i + 1) }
                ] }),
            );
        }
        defs.insert(
            "d64".to_string(),
            json!({ "properties": { "x": { "type": "integer" } } }),
        );
        let s = json!({ "$defs": defs, "$ref": "#/$defs/d0" });
        assert_eq!(at(&s, &["x"]), typed(&[JsonType::Integer]));
    }

    // ── Row 4: boolean subschemas ─────────────────────────────────────────────────────────────

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

    // ── Row 5: `allOf` merges ─────────────────────────────────────────────────────────────────

    #[test]
    fn all_of_declares_every_branchs_paths_and_intersects_their_constraints() {
        let s = json!({ "allOf": [
            { "properties": { "a": { "type": "number", "enum": [1, 2] } } },
            { "properties": { "a": { "type": "integer", "enum": [2.0, 3] }, "b": { "type": "string" } } }
        ] });
        assert_eq!(
            at(&s, &["a"]),
            PathResolution::Declared(PathFacts {
                types: types(&[JsonType::Integer]),
                values: Some(vec![json!(2)]),
                items: None,
            })
        );
        assert_eq!(at(&s, &["b"]), typed(&[JsonType::String]));
    }

    #[test]
    fn an_all_of_branch_forbidding_a_path_forbids_it() {
        let s = json!({ "allOf": [
            { "properties": { "a": { "type": "string" } } },
            { "properties": { "a": false } }
        ] });
        assert_eq!(at(&s, &["a"]), PathResolution::NotDeclared);
    }

    // ── Row 6: `anyOf` / `oneOf` union ────────────────────────────────────────────────────────

    #[test]
    fn any_of_unions_types_and_enums_across_the_branches_that_declare_the_path() {
        let s = json!({ "anyOf": [
            { "properties": { "a": { "type": "string", "enum": ["x"] } } },
            { "properties": { "a": { "type": "number", "enum": [1] } } },
            { "properties": { "b": { "type": "boolean" } } }
        ] });
        assert_eq!(
            at(&s, &["a"]),
            PathResolution::Declared(PathFacts {
                types: types(&[JsonType::Number, JsonType::String]),
                values: Some(vec![json!("x"), json!(1)]),
                items: None,
            })
        );
        assert_eq!(at(&s, &["b"]), typed(&[JsonType::Boolean]));
    }

    #[test]
    fn one_of_is_unconstrained_in_value_when_any_declaring_branch_has_no_enum() {
        let s = json!({ "oneOf": [
            { "properties": { "a": { "type": "string", "enum": ["x"] } } },
            { "properties": { "a": { "type": "string" } } }
        ] });
        assert_eq!(at(&s, &["a"]), typed(&[JsonType::String]));
    }

    #[test]
    fn a_union_branch_with_no_type_leaves_the_union_untyped() {
        let s = json!({ "anyOf": [
            { "properties": { "a": { "type": "string" } } },
            { "properties": { "a": {} } }
        ] });
        assert_eq!(at(&s, &["a"]), untyped());
    }

    // ── Row 7: constructs not walked ──────────────────────────────────────────────────────────

    #[test]
    fn a_path_reachable_only_through_an_unwalked_construct_is_not_declared() {
        for s in [
            json!({ "additionalProperties": { "type": "string" } }),
            json!({ "patternProperties": { "^a": { "type": "string" } } }),
            json!({ "unevaluatedProperties": { "type": "string" } }),
            json!({ "if": { "properties": { "a": {} } },
                    "then": { "properties": { "a": { "type": "string" } } },
                    "else": { "properties": { "a": { "type": "number" } } } }),
            json!({ "dependentSchemas": { "z": { "properties": { "a": { "type": "string" } } } } }),
        ] {
            assert_eq!(at(&s, &["a"]), PathResolution::NotDeclared, "{s}");
        }
    }

    // ── Row 8: never fetched ──────────────────────────────────────────────────────────────────

    #[test]
    fn a_remote_or_file_ref_is_not_followed() {
        for reference in [
            "http://169.254.169.254/latest/meta-data",
            "file:///etc/passwd",
            "other.json#/$defs/x",
        ] {
            let s = json!({ "properties": { "a": { "$ref": reference } } });
            assert_eq!(
                at(&s, &["a", "x"]),
                PathResolution::NotDeclared,
                "{reference}"
            );
        }
    }

    // ── Row 9: no type and no enum ────────────────────────────────────────────────────────────

    #[test]
    fn a_path_with_no_type_and_no_enum_is_declared_and_unconstrained() {
        let s = json!({ "properties": { "note": { "description": "free" } } });
        assert_eq!(at(&s, &["note"]), untyped());
    }

    // ── Array elements, which the `contains` array row reads ─────────────────────────────────

    #[test]
    fn an_array_path_carries_its_element_facts() {
        let s = json!({ "properties": {
            "tags": { "type": "array", "items": { "type": "string", "enum": ["a", "b"] } }
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
    }

    #[test]
    fn a_tuple_leaves_its_elements_unconstrained() {
        let s = json!({ "properties": {
            "pair": { "type": "array", "prefixItems": [{ "type": "string" }], "items": { "type": "number" } }
        } });
        assert_eq!(at(&s, &["pair"]), typed(&[JsonType::Array]));
    }

    // ── The bounds ────────────────────────────────────────────────────────────────────────────

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
        assert_eq!(
            enumerate(&s, &small),
            Vocabulary {
                paths: vec![],
                truncated: Some(BoundKind::SchemaBytes)
            }
        );
    }

    #[test]
    fn a_defs_fan_out_terminates_under_the_path_budget_and_says_it_was_truncated() {
        // Each definition declares two properties that both reference the next one: 2^N paths
        // from a schema of a few kilobytes (§3.3, *bounded*).
        let n = 40;
        let mut defs = Map::new();
        for i in 0..n {
            let next = format!("#/$defs/d{}", i + 1);
            defs.insert(
                format!("d{i}"),
                json!({ "properties": { "a": { "$ref": next }, "b": { "$ref": next } } }),
            );
        }
        defs.insert(format!("d{n}"), json!({ "type": "string" }));
        let s = json!({ "$defs": defs, "$ref": "#/$defs/d0" });
        let v = enumerate(&s, &WalkBounds::BUILD);
        assert_eq!(v.truncated, Some(BoundKind::Paths));
        assert_eq!(v.paths.len(), MAX_WALK_PATHS);
        // Breadth-first: the shallowest level is whole.
        assert_eq!(v.paths[0].path, path(&["a"]));
        assert_eq!(v.paths[1].path, path(&["b"]));
    }

    #[test]
    fn a_recursive_properties_tree_yields_its_depth_bounded_paths_and_says_so() {
        let s = json!({
            "$defs": { "node": { "properties": {
                "v": { "type": "integer" },
                "child": { "$ref": "#/$defs/node" }
            } } },
            "$ref": "#/$defs/node"
        });
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
            enumerate(&s, &three),
            Vocabulary {
                paths: vec![
                    AskablePath::new(path(&["child"]), node.clone()),
                    AskablePath::new(path(&["v"]), v.clone()),
                    AskablePath::new(path(&["child", "child"]), node.clone()),
                    AskablePath::new(path(&["child", "v"]), v.clone()),
                    AskablePath::new(path(&["child", "child", "child"]), node),
                    AskablePath::new(path(&["child", "child", "v"]), v),
                ],
                truncated: Some(BoundKind::Depth),
            }
        );
        // And under the build's bound: two paths per level, to the bound and no further.
        let built = enumerate(&s, &WalkBounds::BUILD);
        assert_eq!(built.truncated, Some(BoundKind::Depth));
        assert_eq!(built.paths.len(), 2 * MAX_WALK_DEPTH);
        assert!(built.paths.iter().all(|p| p.path.len() <= MAX_WALK_DEPTH));
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
                truncated: None,
            }
        );
    }
}
