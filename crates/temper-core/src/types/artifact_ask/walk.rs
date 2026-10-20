//! The bounded schema walk — what a shape declares at a path, decided over a `serde_json::Value`
//! with no JSON Schema validator (spec §2.4, §3.3).
//!
//! **It walks exactly the constructs of the §3.3 table, and nothing more.** `properties` chains; a
//! local `$ref` — any `#`-prefixed JSON pointer into the schema, `#/$defs/…` and `#/definitions/…`
//! among them; boolean subschemas; `allOf` (merged); `anyOf` / `oneOf` (unioned).
//! `additionalProperties`, `patternProperties`, `unevaluatedProperties`, `if`/`then`/`else` and
//! `dependentSchemas` are **not walked**, so a path reachable only through them is not declared —
//! the declared hole of spec §11, refused as `path_outside_shape` rather than guessed at.
//!
//! **A `$ref` the walk does not follow is `true`, under every draft.** An anchor, a reference
//! resolved against an embedded `$id`, or a remote one: the walk cannot see what it says, so the
//! subschema it stands for is declared and unconstrained. That never refuses a declared path; a
//! value admitted there may then match nothing, which is an honest empty.
//!
//! **The walk sees only schemas that compiled.** The loader (spec §2.4) compiled the schema with
//! the no-fetch compiler before handing it here, so an unrecognised `$schema` or a remote `$ref` has
//! already been decided `Uncompilable`. What this module reads `$schema` for is where the drafts
//! differ for it: whether keywords beside a `$ref` apply (2019-09 and 2020-12) or are ignored
//! (draft-04/06/07), and whether `const` and `prefixItems` exist. The draft names are the ones
//! `jsonschema` recognises (`referencing::Draft::from_schema_uri`); an absent `$schema` is 2020-12,
//! its default.
//!
//! # Bounded in time, memory and stack (§3.3: the walk "never stalls or exhausts the server")
//!
//! A schema is written by any author of its home and read by every reader of it, and a cycle guard
//! alone does not bound the walk: a non-cyclic `$defs` graph in which each definition references
//! the next twice declares 2^N paths from a few kilobytes, and an `allOf` of tens of thousands of
//! branches makes every visit to its schema expensive. So:
//!
//! - **Four named bounds** ([`BoundKind`]): path depth, advertised paths, schema bytes, and a
//!   **work budget** of subschema visits shared by everything one [`resolve`] or one [`enumerate`]
//!   does. Meeting any of them is reported, never a stall.
//! - **No recursion over the schema.** A subschema is evaluated from an explicit stack, so a chain
//!   of thousands of `$ref` hops costs heap, not call stack. The only recursion left is over the
//!   [`PathFacts`] value built, whose `items` nesting is held to the depth bound.
//! - **Memoised per walk** by `(subschema, path position)`, so a definition referenced many times
//!   at one position is evaluated once.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::reference::jsonb_eq;
use crate::types::query::filter::json_bytes;

/// The deepest path, in object-key segments, the walk descends to. A deeper ask is
/// [`PathResolution::BeyondBound`]`(`[`BoundKind::Depth`]`)`, refused `path_outside_shape` with
/// `teaches` naming the bound (spec §2.5). It also bounds how many array levels deep element facts
/// ([`PathFacts::items`]) are reported; past it an element is unconstrained.
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
/// that, and it bounds how much any one subschema can hold — the widest `allOf` is under 100,000
/// branches.
pub const MAX_SCHEMA_BYTES: usize = 256 * 1024;

/// The most subschema visits one [`resolve`] or one [`enumerate`] makes, counted as each subschema
/// is scheduled for evaluation or expansion. Past it the walk stops at [`BoundKind::Work`].
///
/// # 65,536
///
/// The other three bounds limit the answer, not the work behind it: an `allOf` of ~85,000 empty
/// branches beside a two-way fan-out fits every one of them and costs ~10⁹ visits to enumerate.
/// This one limits the work directly. A real shape needs tens of visits per path — a `properties`
/// step, a `$ref` hop, an `allOf` or two — so the vocabulary of a shape at [`MAX_WALK_PATHS`] with
/// sixty-four visits each fits, and a pathological schema stops after a few milliseconds of work.
pub const MAX_WALK_VISITS: usize = 65_536;

/// The walk's four bounds. The values the build runs are [`WalkBounds::BUILD`]; tests construct
/// smaller ones to reach a bound without a large schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkBounds {
    pub max_depth: usize,
    pub max_paths: usize,
    pub max_schema_bytes: usize,
    pub max_visits: usize,
}

impl WalkBounds {
    /// The bounds every production caller walks under.
    pub const BUILD: WalkBounds = WalkBounds {
        max_depth: MAX_WALK_DEPTH,
        max_paths: MAX_WALK_PATHS,
        max_schema_bytes: MAX_SCHEMA_BYTES,
        max_visits: MAX_WALK_VISITS,
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
    /// [`WalkBounds::max_visits`] — the walk spent its work budget before it could say.
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
    /// The declared JSON types. `None`: no `type` constrains the path. An empty set: the branches
    /// that apply here declare types with nothing in common, so no value is admissible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<BTreeSet<JsonType>>,
    /// The `enum` / `const` values. `None`: no value set constrains the path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<Value>>,
    /// What an element of an array at this path declares. `None`: elements are unconstrained.
    /// Nested at most [`MAX_WALK_DEPTH`] arrays deep.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "web-api", schema(no_recursion))]
    pub items: Option<Box<PathFacts>>,
}

impl PathFacts {
    /// Whether nothing at all is declared — the facts of `{}` or `true`.
    fn is_unconstrained(&self) -> bool {
        self.types.is_none() && self.values.is_none() && self.items.is_none()
    }

    /// Whether `compare` is feasible here: the path declares `number`, `integer` or `string`, or
    /// declares no type (spec §3.4, the `compare` row).
    pub fn admits_compare(&self) -> bool {
        self.types.as_ref().is_none_or(|t| {
            t.iter()
                .any(|t| matches!(t, JsonType::Number | JsonType::Integer | JsonType::String))
        })
    }

    /// The operators feasible at this path, in [`AskOperator`] order. `has_key` and `contains`
    /// always are — whether a `contains` value is admissible is the value's question
    /// (`value_outside_shape`), never the operator's.
    pub fn operators(&self) -> Vec<AskOperator> {
        let mut ops = vec![AskOperator::HasKey, AskOperator::Contains];
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

/// What a shape lets a caller ask: its askable paths, and every bound the list was cut at.
///
/// `truncated` names each walk bound met, never a silent short list (spec §2.5): a recursive tree
/// through `properties` is `depth`, a 2^N `$defs` fan-out is `paths`, an oversized schema is
/// `schema_bytes` with no paths, and a walk that spent its budget is `work`. Ordered, and at most
/// one of each.
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
            truncated: BTreeSet::from([bound]),
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

/// The visits one walk may still make. Shared by everything one `resolve` or one `enumerate` does.
#[derive(Debug)]
struct Budget {
    left: usize,
}

impl Budget {
    fn new(visits: usize) -> Budget {
        Budget { left: visits }
    }

    fn spend(&mut self, visits: usize) -> Result<(), BoundKind> {
        self.left = self.left.checked_sub(visits).ok_or(BoundKind::Work)?;
        Ok(())
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

    /// One path, under a budget of its own.
    pub(super) fn resolve(&self, path: &[String]) -> PathResolution {
        self.resolve_within(path, &mut Budget::new(self.bounds.max_visits))
    }

    fn resolve_within(&self, path: &[String], budget: &mut Budget) -> PathResolution {
        if path.len() > self.bounds.max_depth {
            return PathResolution::BeyondBound(BoundKind::Depth);
        }
        let mut resolver = Resolver {
            walk: self,
            path,
            budget,
            memo: HashMap::new(),
            active: HashSet::new(),
        };
        match resolver.run() {
            Ok(Reach::Declared(facts)) => PathResolution::Declared(facts),
            Ok(Reach::Silent | Reach::Forbidden) => PathResolution::NotDeclared,
            Err(bound) => PathResolution::BeyondBound(bound),
        }
    }

    /// Breadth-first over property keys, shallowest paths first and keys in byte order within a
    /// level, so a truncated vocabulary is the shallow part of the full one. One budget covers
    /// every frame and every path.
    pub(super) fn enumerate(&self) -> Vocabulary {
        let mut budget = Budget::new(self.bounds.max_visits);
        let mut paths = Vec::new();
        let mut truncated = BTreeSet::new();
        let root = match self.frame([self.root], &mut budget) {
            Ok(frame) => frame,
            Err(bound) => {
                truncated.insert(bound);
                return Vocabulary { paths, truncated };
            }
        };
        let mut queue: VecDeque<(Vec<String>, Vec<&'s Value>)> =
            VecDeque::from([(Vec::new(), root)]);
        while let Some((prefix, frame)) = queue.pop_front() {
            let keys = child_keys(&frame);
            if keys.is_empty() {
                continue;
            }
            if prefix.len() == self.bounds.max_depth {
                // Keys exist below the bound. Whether each is declared is past the bound too, so
                // this says "there may be more" — the honest reading of a depth cut.
                truncated.insert(BoundKind::Depth);
                continue;
            }
            for key in keys {
                let mut path = prefix.clone();
                path.push(key.to_string());
                let facts = match self.resolve_within(&path, &mut budget) {
                    PathResolution::Declared(facts) => facts,
                    PathResolution::NotDeclared => continue,
                    PathResolution::BeyondBound(bound) => {
                        truncated.insert(bound);
                        return Vocabulary { paths, truncated };
                    }
                };
                if paths.len() == self.bounds.max_paths {
                    truncated.insert(BoundKind::Paths);
                    return Vocabulary { paths, truncated };
                }
                paths.push(AskablePath::new(path.clone(), facts));
                let children = frame
                    .iter()
                    .copied()
                    .filter_map(|node| node.get("properties").and_then(|p| p.get(key)));
                match self.frame(children, &mut budget) {
                    Ok(child_frame) => queue.push_back((path, child_frame)),
                    Err(bound) => {
                        truncated.insert(bound);
                        return Vocabulary { paths, truncated };
                    }
                }
            }
        }
        Vocabulary { paths, truncated }
    }

    /// The schema objects whose `properties` apply at one location — the given nodes and what their
    /// `$ref`, `allOf`, `anyOf` and `oneOf` reach — and only those: a node without `properties` is
    /// expanded through and not kept. Each node is visited once, from an explicit stack, and every
    /// visit is spent from the budget.
    fn frame(
        &self,
        nodes: impl IntoIterator<Item = &'s Value>,
        budget: &mut Budget,
    ) -> Result<Vec<&'s Value>, BoundKind> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut stack: Vec<&'s Value> = nodes.into_iter().collect();
        while let Some(node) = stack.pop() {
            let Value::Object(map) = node else {
                continue;
            };
            if !seen.insert(identity(node)) {
                continue;
            }
            budget.spend(1)?;
            if let Some(target) = map.get("$ref").and_then(|r| self.target(r)) {
                stack.push(target);
            }
            if self.draft.ref_overrides_siblings() && map.contains_key("$ref") {
                continue;
            }
            if map.get("properties").is_some_and(Value::is_object) {
                out.push(node);
            }
            for keyword in ["allOf", "anyOf", "oneOf"] {
                stack.extend(
                    map.get(keyword)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten(),
                );
            }
        }
        Ok(out)
    }

    /// The subschema a local `$ref` names: the fragment of a `#`-prefixed reference,
    /// percent-decoded, read as a JSON pointer from the root (`~0`/`~1` unescaped by
    /// `Value::pointer`). `#` alone is the root. Anything else — an anchor, a remote or `file:`
    /// URI — is not followed (§3.3, the remote-`$ref` row).
    fn target(&self, reference: &Value) -> Option<&'s Value> {
        let fragment = reference.as_str()?.strip_prefix('#')?;
        self.root.pointer(&percent_decode(fragment)?)
    }
}

/// A URI fragment with its `%XX` escapes decoded, or `None` if an escape is malformed or the
/// result is not UTF-8.
fn percent_decode(fragment: &str) -> Option<String> {
    let bytes = fragment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
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

/// The part a subschema plays for the object that applies it.
#[derive(Debug, Clone, Copy)]
enum Role {
    /// The `$ref` target, when the walk follows it.
    Ref,
    /// `properties[segment]`, one segment deeper.
    Property,
    /// `items`, at the end of the path: what an element declares.
    Items,
    AllOf,
    AnyOf,
    OneOf,
}

/// A subschema to evaluate, and where.
type Child<'s> = (&'s Value, usize, Role);

/// One step of the evaluation: evaluate a subschema at a position, or combine an object's
/// children's results once they are all on the value stack.
enum Task<'s> {
    Eval(&'s Value, usize),
    Combine(&'s Map<String, Value>, usize, usize),
}

/// One path's walk: an explicit-stack evaluation, memoised by `(subschema, position)` and
/// cycle-guarded on the same key.
///
/// A position past the path's end is inside `items`: `path.len() + k` is the element schema `k`
/// arrays deep, so element facts are memoised and bounded like everything else.
///
/// **A cycle that consumes no segment adds nothing.** `A = {allOf: [{$ref: B}]}`,
/// `B = {allOf: [{$ref: A}]}` re-enters `A` at the same position; the re-entry is silent, so the
/// facts are those reached before the cycle closed. The direction that loses is lenient — an
/// admitted value runs and is an honest empty — never a refusal of something declared.
struct Resolver<'w, 's> {
    walk: &'w Walk<'s>,
    path: &'w [String],
    budget: &'w mut Budget,
    memo: HashMap<(usize, usize), Reach>,
    active: HashSet<(usize, usize)>,
}

impl<'s> Resolver<'_, 's> {
    fn run(&mut self) -> Result<Reach, BoundKind> {
        self.budget.spend(1)?;
        let mut tasks = vec![Task::Eval(self.walk.root, 0)];
        let mut values: Vec<Reach> = Vec::new();
        while let Some(task) = tasks.pop() {
            match task {
                Task::Eval(node, at) => self.eval(node, at, &mut tasks, &mut values)?,
                Task::Combine(map, at, id) => {
                    let children = self.children(map, at);
                    // Each child's evaluation pushed exactly one value, after this task was pushed
                    // and before it is popped.
                    let results = values.split_off(values.len() - children.len());
                    let reached = self.combine(map, at, &children, results);
                    self.active.remove(&(id, at));
                    self.memo.insert((id, at), reached.clone());
                    values.push(reached);
                }
            }
        }
        Ok(values.pop().unwrap_or(Reach::Silent))
    }

    /// Push one value for a leaf, a memoised object, or a cycle; or schedule an object's children
    /// and its combination, spending one visit per child.
    fn eval(
        &mut self,
        node: &'s Value,
        at: usize,
        tasks: &mut Vec<Task<'s>>,
        values: &mut Vec<Reach>,
    ) -> Result<(), BoundKind> {
        let reached = match node {
            Value::Bool(false) => Reach::Forbidden,
            Value::Bool(true) => self.unconstrained(at),
            Value::Object(map) => {
                let key = (identity(node), at);
                if let Some(known) = self.memo.get(&key) {
                    known.clone()
                } else if self.active.contains(&key) {
                    Reach::Silent
                } else {
                    let children = self.children(map, at);
                    self.budget.spend(children.len())?;
                    self.active.insert(key);
                    tasks.push(Task::Combine(map, at, key.0));
                    tasks.extend(
                        children
                            .iter()
                            .rev()
                            .map(|&(child, child_at, _)| Task::Eval(child, child_at)),
                    );
                    return Ok(());
                }
            }
            _ => Reach::Silent,
        };
        values.push(reached);
        Ok(())
    }

    /// `true`, or a `$ref` the walk does not follow: declared and unconstrained where it sits, and
    /// declaring nothing below it — a deeper segment is not reached through `properties`.
    fn unconstrained(&self, at: usize) -> Reach {
        if at >= self.path.len() {
            Reach::Declared(PathFacts::default())
        } else {
            Reach::Silent
        }
    }

    /// The subschemas an object applies at a position, in a fixed order the combination relies on.
    fn children(&self, map: &'s Map<String, Value>, at: usize) -> Vec<Child<'s>> {
        let mut out = Vec::new();
        if let Some(target) = map.get("$ref").and_then(|r| self.walk.target(r)) {
            out.push((target, at, Role::Ref));
        }
        if self.walk.draft.ref_overrides_siblings() && map.contains_key("$ref") {
            return out;
        }
        match self.path.get(at) {
            Some(segment) => {
                if let Some(child) = map.get("properties").and_then(|p| p.get(segment.as_str())) {
                    out.push((child, at + 1, Role::Property));
                }
            }
            None => {
                if let Some(items) = self.items_schema(map, at) {
                    out.push((items, at + 1, Role::Items));
                }
            }
        }
        for (keyword, role) in [
            ("allOf", Role::AllOf),
            ("anyOf", Role::AnyOf),
            ("oneOf", Role::OneOf),
        ] {
            out.extend(
                map.get(keyword)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|branch| (branch, at, role)),
            );
        }
        out
    }

    /// The element schema, when `items` is a single schema that applies to every element and the
    /// element is within the depth bound. A tuple (`items` as an array before 2020-12, or
    /// `prefixItems` in it) gives each position its own schema, so an element anywhere is
    /// unconstrained here.
    fn items_schema(&self, map: &'s Map<String, Value>, at: usize) -> Option<&'s Value> {
        if at - self.path.len() >= self.walk.bounds.max_depth {
            return None;
        }
        if self.walk.draft.has_prefix_items() && map.contains_key("prefixItems") {
            return None;
        }
        map.get("items").filter(|i| i.is_object() || i.is_boolean())
    }

    /// An object's own reach from its children's. Under draft-04/06/07 an object with a `$ref` is
    /// the reference alone; otherwise its own facts (or its `properties` step) are conjoined with
    /// the reference, every `allOf` branch, and each union.
    fn combine(
        &self,
        map: &'s Map<String, Value>,
        at: usize,
        children: &[Child<'s>],
        results: Vec<Reach>,
    ) -> Reach {
        if self.walk.draft.ref_overrides_siblings() && map.contains_key("$ref") {
            return results
                .into_iter()
                .next()
                .unwrap_or_else(|| self.unconstrained(at));
        }
        let mut step = Reach::Silent;
        let mut items = None;
        let mut conjuncts = Vec::new();
        let mut any_of: Option<Vec<Reach>> = None;
        let mut one_of: Option<Vec<Reach>> = None;
        for ((_, _, role), reach) in children.iter().zip(results) {
            match role {
                Role::Property => step = reach,
                Role::Items => items = Some(reach),
                Role::Ref | Role::AllOf => conjuncts.push(reach),
                Role::AnyOf => any_of.get_or_insert_with(Vec::new).push(reach),
                Role::OneOf => one_of.get_or_insert_with(Vec::new).push(reach),
            }
        }
        let mut acc = if at >= self.path.len() {
            Reach::Declared(self.local_facts(map, items))
        } else {
            step
        };
        for conjunct in conjuncts {
            acc = conjoin(acc, conjunct);
        }
        for union in [any_of, one_of].into_iter().flatten() {
            acc = conjoin(acc, disjoin(union));
        }
        acc
    }

    /// What one schema object itself declares at the path's end: `type`, `enum` / `const`, and
    /// what its element schema reached.
    fn local_facts(&self, map: &'s Map<String, Value>, items: Option<Reach>) -> PathFacts {
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
            items: match items {
                None | Some(Reach::Silent) => None,
                // `items: false` admits no element at all.
                Some(Reach::Forbidden) => Some(Box::new(PathFacts {
                    types: Some(BTreeSet::new()),
                    ..PathFacts::default()
                })),
                Some(Reach::Declared(facts)) if facts.is_unconstrained() => None,
                Some(Reach::Declared(facts)) => Some(Box::new(facts)),
            },
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

    const DRAFT_07: &str = "http://json-schema.org/draft-07/schema#";

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
            "$schema": DRAFT_07,
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
    fn a_root_ref_is_followed_alike_under_every_draft() {
        for schema in [None, Some(DRAFT_07)] {
            let mut s = json!({
                "type": "object",
                "properties": { "child": { "$ref": "#" }, "n": { "type": "integer" } }
            });
            if let (Some(uri), Some(map)) = (schema, s.as_object_mut()) {
                map.insert("$schema".to_string(), json!(uri));
            }
            assert_eq!(at(&s, &["child"]), typed(&[JsonType::Object]), "{s}");
            assert_eq!(at(&s, &["child", "n"]), typed(&[JsonType::Integer]), "{s}");
        }
    }

    #[test]
    fn a_ref_pointer_is_unescaped_and_percent_decoded() {
        let s = json!({
            "$defs": { "a/b": { "type": "string" }, "c%d": { "type": "boolean" } },
            "properties": {
                "slash": { "$ref": "#/$defs/a~1b" },
                "percent": { "$ref": "#/$defs/c%25d" }
            }
        });
        assert_eq!(at(&s, &["slash"]), typed(&[JsonType::String]));
        assert_eq!(at(&s, &["percent"]), typed(&[JsonType::Boolean]));
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

    #[test]
    fn a_long_acyclic_ref_chain_resolves_without_deep_recursion() {
        // 5,000 `$ref` hops at one position. Recursing once per hop would need far more than the
        // 256 KiB this runs on; the explicit stack needs none of it.
        let hops = 5_000;
        let mut defs = Map::new();
        for i in 0..hops {
            defs.insert(
                format!("d{i}"),
                json!({ "$ref": format!("#/$defs/d{}", i + 1) }),
            );
        }
        defs.insert(
            format!("d{hops}"),
            json!({ "properties": { "x": { "type": "integer" } } }),
        );
        let s = json!({ "$defs": defs, "$ref": "#/$defs/d0" });
        let (resolved, vocabulary) = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || (at(&s, &["x"]), enumerate(&s, &WalkBounds::BUILD)))
            .expect("spawn a small-stack thread")
            .join()
            .expect("the walk returns on a 256 KiB stack");
        assert_eq!(resolved, typed(&[JsonType::Integer]));
        assert_eq!(
            vocabulary,
            Vocabulary {
                paths: vec![AskablePath::new(
                    path(&["x"]),
                    PathFacts {
                        types: types(&[JsonType::Integer]),
                        ..PathFacts::default()
                    }
                )],
                truncated: BTreeSet::new(),
            }
        );
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
    fn a_ref_the_walk_does_not_follow_declares_its_path_unconstrained_under_every_draft() {
        for reference in [
            "http://169.254.169.254/latest/meta-data",
            "file:///etc/passwd",
            "other.json#/$defs/x",
            "#an-anchor",
        ] {
            for schema in [None, Some(DRAFT_07)] {
                let mut s = json!({ "properties": { "a": { "$ref": reference } } });
                if let (Some(uri), Some(map)) = (schema, s.as_object_mut()) {
                    map.insert("$schema".to_string(), json!(uri));
                }
                assert_eq!(at(&s, &["a"]), untyped(), "{s}");
                assert_eq!(at(&s, &["a", "x"]), PathResolution::NotDeclared, "{s}");
            }
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
                truncated: BTreeSet::from([BoundKind::SchemaBytes]),
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
        assert_eq!(v.truncated, BTreeSet::from([BoundKind::Paths]));
        assert_eq!(v.paths.len(), MAX_WALK_PATHS);
        // Breadth-first: the shallowest level is whole.
        assert_eq!(v.paths[0].path, path(&["a"]));
        assert_eq!(v.paths[1].path, path(&["b"]));
    }

    /// A self-referencing two-way fan-out whose every visit also pays for a wide `allOf`.
    fn wide_fan_out(width: usize) -> Value {
        let empties: Vec<Value> = (0..width).map(|_| json!({})).collect();
        json!({
            "$defs": { "d": {
                "properties": { "a": { "$ref": "#/$defs/d" }, "b": { "$ref": "#/$defs/d" } },
                "allOf": empties
            } },
            "$ref": "#/$defs/d"
        })
    }

    #[test]
    fn a_wide_all_of_under_a_fan_out_stops_at_the_work_budget_and_says_so() {
        let v = enumerate(&wide_fan_out(100), &WalkBounds::BUILD);
        assert!(v.truncated.contains(&BoundKind::Work), "{:?}", v.truncated);
        assert!(
            !v.paths.is_empty(),
            "the shallow vocabulary is still advertised"
        );
        assert!(v.paths.len() < MAX_WALK_PATHS, "{}", v.paths.len());
    }

    #[test]
    fn an_all_of_wider_than_the_budget_stops_one_path_at_the_work_budget() {
        // Under the schema byte bound, and wider than the visit budget.
        let s = wide_fan_out(MAX_WALK_VISITS + 1);
        assert_eq!(
            resolve(&s, &path(&["a"]), &WalkBounds::BUILD),
            PathResolution::BeyondBound(BoundKind::Work)
        );
        assert_eq!(
            enumerate(&s, &WalkBounds::BUILD),
            Vocabulary {
                paths: vec![],
                truncated: BTreeSet::from([BoundKind::Work]),
            }
        );
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
                truncated: BTreeSet::from([BoundKind::Depth]),
            }
        );
        // And under the build's bound: two paths per level, to the bound and no further.
        let built = enumerate(&s, &WalkBounds::BUILD);
        assert_eq!(built.truncated, BTreeSet::from([BoundKind::Depth]));
        assert_eq!(built.paths.len(), 2 * MAX_WALK_DEPTH);
        assert!(built.paths.iter().all(|p| p.path.len() <= MAX_WALK_DEPTH));
    }

    #[test]
    fn each_cut_names_the_bound_that_made_it() {
        // A three-way recursive fan-out, cut three different ways.
        let s = json!({
            "$defs": { "node": { "properties": {
                "a": { "$ref": "#/$defs/node" },
                "b": { "$ref": "#/$defs/node" },
                "c": { "$ref": "#/$defs/node" }
            } } },
            "$ref": "#/$defs/node"
        });
        let by_paths = WalkBounds {
            max_depth: 1,
            max_paths: 2,
            ..WalkBounds::BUILD
        };
        let v = enumerate(&s, &by_paths);
        assert_eq!(v.paths.len(), 2);
        assert_eq!(v.truncated, BTreeSet::from([BoundKind::Paths]));
        let by_depth = WalkBounds {
            max_depth: 1,
            ..WalkBounds::BUILD
        };
        let v = enumerate(&s, &by_depth);
        assert_eq!(v.paths.len(), 3);
        assert_eq!(v.truncated, BTreeSet::from([BoundKind::Depth]));
        // Depth two needs twelve paths and about a hundred visits; forty is not enough.
        let by_work = WalkBounds {
            max_depth: 2,
            max_visits: 40,
            ..WalkBounds::BUILD
        };
        let v = enumerate(&s, &by_work);
        assert_eq!(v.truncated, BTreeSet::from([BoundKind::Work]));
        assert!(v.paths.len() < 12, "{}", v.paths.len());
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
}
