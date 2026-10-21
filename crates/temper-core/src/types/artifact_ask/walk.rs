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
//! **A `$ref` the walk does not follow is `true`, under every draft.** An anchor, a remote one, or
//! any reference made inside a resource the schema embeds under its own `$id` (where `#` means that
//! resource, not the root): the walk cannot see what it says, so the subschema it stands for is
//! declared and unconstrained. That never refuses a declared path; a
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
//! - **Every size an author controls is charged before work proportional to it is done.** The
//!   schema is measured only up to its bound; each `$ref` is decoded once, when the walk is built;
//!   each list a visit reads — branches, `type` names, `enum` members, a frame's property keys —
//!   costs its length; and each advertised path costs its bytes. A property key longer than an
//!   asked segment may be (`MAX_ARTIFACT_PATH_SEGMENT_BYTES`) is never advertised and never walked
//!   below: no ask can name it.
//! - **No recursion over the schema.** A subschema is evaluated from an explicit stack, so a chain
//!   of thousands of `$ref` hops costs heap, not call stack. The only recursion left is over the
//!   [`PathFacts`] value built, whose `items` nesting is held to the depth bound.
//! - **Memoised per walk** by `(subschema, path position)`, so a definition referenced many times
//!   at one position is evaluated once.
//! - **Values cost what they carry.** An `enum` / `const` member is read once per walk and charged
//!   by its size — its bytes and the values nested in it; the set a keyword declares is built once
//!   per walk and shared rather than copied; sets intersect and union by member id in linear time,
//!   charged by their sizes each time; and copying them into an answer is charged by size again.
//!   An ask's paths are never copied at all. So a wide `allOf` over a large enum stops at the work
//!   budget instead of holding millions of values or comparing them pairwise.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::reference::canonical;
use super::validate::MAX_ARTIFACT_PATH_SEGMENT_BYTES;
use crate::types::query::filter::{json_string_bytes, property_value_nodes};

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
/// `get_data_artifact_shape`, so this bounds a response body and the walk behind it. It is far
/// above a shape a person writes — a record with tens of fields — and it is the bound the 2^N
/// `$defs` fan-out meets.
///
/// It bounds how many paths, not how large each is. A path's text is at most [`MAX_WALK_DEPTH`]
/// segments of at most `MAX_ARTIFACT_PATH_SEGMENT_BYTES` — a longer key is never advertised — so
/// 4 KiB; and every path built is charged its bytes against [`MAX_WALK_VISITS`] at
/// [`BYTES_PER_VISIT`], so one walk builds at most about 4 MiB of path text whatever the count. A
/// path's declared values are charged the same way.
pub const MAX_WALK_PATHS: usize = 1024;

/// The largest schema the walk reads, in compact serialized JSON bytes. Past it nothing is walked:
/// every path is [`PathResolution::BeyondBound`]`(`[`BoundKind::SchemaBytes`]`)` and the
/// vocabulary is empty and says why. The measurement itself stops at the bound, so a schema far
/// over it costs no more to refuse than one just over it.
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
    /// The path is deeper than the walk descends.
    Depth,
    /// The vocabulary has more paths than are advertised.
    Paths,
    /// The schema is larger than the walk reads.
    SchemaBytes,
    /// Reading the shape spent its work budget — schema visits, or enum membership steps —
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
    /// The declared JSON types. `None`: no `type` constrains the path. An empty set: the branches
    /// that apply here declare types with nothing in common, so no value is admissible.
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

    /// Whether a schema object opens a resource of its own: an `$id` (`id` in draft-04) that is a
    /// URI rather than a plain-name fragment, which in draft-06/07 is an anchor and changes no base.
    fn opens_resource(self, schema: &Map<String, Value>) -> bool {
        let keyword = if self == Draft::D4 { "id" } else { "$id" };
        schema
            .get(keyword)
            .and_then(Value::as_str)
            .is_some_and(|id| !id.starts_with('#'))
    }
}

/// How many bytes one visit of the budget pays for, wherever the walk carries text the author
/// sized. A member of an `enum` or `const` costs one visit, plus one per this many bytes of its
/// canonical text and of its nested values counted at [`NODE_BYTES`] each, when the walk first
/// reads it and each time it is copied into an answer; a path [`enumerate`] builds costs one
/// visit, plus one per this many bytes of its segments.
///
/// # 64
///
/// A short code, or a path of short keys, costs one visit; a single member of 200 KB costs over
/// three thousand. So no walk copies more than about [`MAX_WALK_VISITS`] × 64 bytes — 4 MiB — of
/// declared values, nor builds more than that of path text.
///
/// # What that leaves a real shape
///
/// An `enum` of `n` short codes costs `2n` visits the first time a walk reads it — `n` for its
/// members, `n` for the set — and nothing more when another path reaches it under the same
/// budget, since both are read once per budget. So:
///
/// - **An ask** reads its paths without copying their values, so every path an ask names over one
///   shared 9,000-code enum costs 18,000 visits for the enum and a few visits for each path —
///   [`MAX_ASK_PREDICATES`](super::validate::MAX_ASK_PREDICATES) such paths fit, in visits. Their
///   `contains` values are checked against the enum under a budget of their own,
///   `MAX_MEMBERSHIP_STEPS`, which they can still spend: 32 predicates of four values each, scanned
///   against 9,000 members, need about 1.15M steps, past 1,048,576, and are refused
///   `path_outside_shape` with `beyond_bound: work`.
/// - **A vocabulary** copies each advertised path's values into the answer, `n` visits a copy, so a
///   shape whose `k` paths all carry the same `n`-code enum costs about `(2 + k) · n`: eleven paths
///   over one 5,000-code enum fit (2 · 5,000 + 11 · 5,000 = 65,000 of 65,536), and the twelfth
///   meets `work`, visibly. Paths over distinct enums each pay the read as well as the copy, `3n`
///   a path, so about four paths over distinct 5,000-code enums fit (4 · 15,000 = 60,000) and the
///   fifth meets `work`. Each copy is memory the answer holds, so it is charged every time.
pub const BYTES_PER_VISIT: usize = 64;

/// The bytes a member's nested value is charged as, beside its canonical text. A value nested in
/// an `enum` member is cloned with the member as its own `Value` — 32 bytes — held in a container
/// that is its own allocation, so its heap cost is several times the one or two bytes its canonical
/// text spends on it (`[[[…]]]` is two bytes a level). Charged as 64 bytes, one visit, a deep or
/// wide member costs what copying it allocates.
pub const NODE_BYTES: usize = 64;

/// What one walk may still spend, and the members and value sets it has already read. Shared by
/// everything one `resolve`, one `enumerate`, or one ask's `resolve_all` does.
#[derive(Debug)]
struct Budget<'s> {
    left: usize,
    /// Canonical text → member id: jsonb-equal members share an id.
    ids: HashMap<String, usize>,
    /// A member value in the schema → its id and weight, so each is read once per walk.
    read: HashMap<usize, (usize, usize)>,
    /// An `enum` or `const` keyword in the schema → the value set it declares, built and charged
    /// once per budget and shared by every path that reaches it.
    sets: HashMap<usize, Values<'s>>,
}

impl<'s> Budget<'s> {
    fn new(visits: usize) -> Budget<'s> {
        Budget {
            left: visits,
            ids: HashMap::new(),
            read: HashMap::new(),
            sets: HashMap::new(),
        }
    }

    fn spend(&mut self, visits: usize) -> Result<(), BoundKind> {
        self.left = self.left.checked_sub(visits).ok_or(BoundKind::Work)?;
        Ok(())
    }

    /// One `enum` or `const` member, read once per walk and charged by its size: its canonical
    /// bytes, and its nested values at [`NODE_BYTES`] each. Its canonical text and node count are
    /// built before the charge can be known; because each member node is read once per budget, that
    /// work is at most the schema's own size per budget.
    fn member(&mut self, value: &'s Value) -> Result<Member<'s>, BoundKind> {
        let node = identity(value);
        if let Some(&(id, weight)) = self.read.get(&node) {
            return Ok(Member { id, weight, value });
        }
        let text = canonical(value);
        let nested = property_value_nodes(value) * NODE_BYTES;
        let weight = 1 + (text.len() + nested) / BYTES_PER_VISIT;
        self.spend(weight)?;
        let next = self.ids.len();
        let id = *self.ids.entry(text).or_insert(next);
        self.read.insert(node, (id, weight));
        Ok(Member { id, weight, value })
    }

    /// The value set an `enum` or `const` keyword declares: every member read, and the set
    /// charged once per member — once per budget, then shared, so a path reaching it again costs
    /// nothing more.
    fn members(
        &mut self,
        keyword: &'s Value,
        values: &'s [Value],
    ) -> Result<Values<'s>, BoundKind> {
        let key = identity(keyword);
        if let Some(set) = self.sets.get(&key) {
            return Ok(Rc::clone(set));
        }
        self.spend(values.len())?;
        let members = values
            .iter()
            .map(|v| self.member(v))
            .collect::<Result<Vec<_>, _>>()?;
        let set = Rc::new(members);
        self.sets.insert(key, Rc::clone(&set));
        Ok(set)
    }
}

/// One schema, measured, its draft selected, and its `$ref`s resolved, ready to resolve paths
/// against.
#[derive(Debug)]
pub(super) struct Walk<'s> {
    root: &'s Value,
    draft: Draft,
    bounds: WalkBounds,
    /// Each schema object with a `$ref` the walk follows → the subschema it names. Built once, so a
    /// visit looks a reference up rather than decoding its text again.
    refs: HashMap<usize, &'s Value>,
}

impl<'s> Walk<'s> {
    /// Measure the schema once, no further than the bound. Over [`WalkBounds::max_schema_bytes`]
    /// nothing is walked. Under it, every `$ref` is resolved here, in passes linear in the schema.
    pub(super) fn new(schema: &'s Value, bounds: &WalkBounds) -> Result<Walk<'s>, BoundKind> {
        if json_bytes_up_to(schema, bounds.max_schema_bytes) > bounds.max_schema_bytes {
            return Err(BoundKind::SchemaBytes);
        }
        let draft = Draft::of(schema);
        let embedded = embedded_resources(schema, draft);
        Ok(Walk {
            root: schema,
            draft,
            bounds: *bounds,
            refs: local_references(schema, &embedded),
        })
    }

    /// One path, under a budget of its own.
    pub(super) fn resolve(&self, path: &[String]) -> PathResolution {
        self.resolve_within(path, &mut Budget::new(self.bounds.max_visits))
    }

    /// Several paths under **one** budget, so what reading them costs does not grow with how many
    /// an ask names (§3.3: the walk "never stalls or exhausts the server"). Once the budget is
    /// spent every later path is [`Resolved::BeyondBound`]`(`[`BoundKind::Work`]`)`.
    ///
    /// The facts are the walk's own, borrowed from the schema: no value is copied, so a value set
    /// many paths share is read once (by [`Budget::members`]) and costs nothing per path after.
    pub(super) fn resolve_all(&self, paths: &[&[String]]) -> Vec<Resolved<'s>> {
        let mut budget = Budget::new(self.bounds.max_visits);
        paths
            .iter()
            .map(|path| self.reach(path, &mut budget))
            .collect()
    }

    /// One path, its facts copied onto the wire and charged for the copy.
    fn resolve_within(&self, path: &[String], budget: &mut Budget<'s>) -> PathResolution {
        match self.reach(path, budget) {
            Resolved::Declared(facts) => match into_wire(&facts, budget) {
                Ok(facts) => PathResolution::Declared(facts),
                Err(bound) => PathResolution::BeyondBound(bound),
            },
            Resolved::NotDeclared => PathResolution::NotDeclared,
            Resolved::BeyondBound(bound) => PathResolution::BeyondBound(bound),
        }
    }

    /// One path, its facts left as the walk found them.
    fn reach(&self, path: &[String], budget: &mut Budget<'s>) -> Resolved<'s> {
        if path.len() > self.bounds.max_depth {
            return Resolved::BeyondBound(BoundKind::Depth);
        }
        let reached = Resolver {
            walk: self,
            path,
            budget,
            memo: HashMap::new(),
            active: HashSet::new(),
        }
        .run();
        match reached {
            Ok(Reach::Declared(facts)) => Resolved::Declared(facts),
            Ok(Reach::Silent | Reach::Forbidden) => Resolved::NotDeclared,
            Err(bound) => Resolved::BeyondBound(bound),
        }
    }

    /// Breadth-first over property keys, shallowest paths first and keys in byte order within a
    /// level, so a truncated vocabulary is the shallow part of the full one. One budget covers
    /// every frame and every path.
    pub(super) fn enumerate(&self) -> Vocabulary {
        let mut budget = Budget::new(self.bounds.max_visits);
        let mut vocabulary = Vocabulary::default();
        if let Err(bound) = self.enumerate_into(&mut vocabulary, &mut budget) {
            vocabulary.truncated.insert(bound);
        }
        vocabulary
    }

    /// The breadth-first walk behind [`Walk::enumerate`]. A depth cut is recorded and the walk goes
    /// on; any other bound stops it, and is returned.
    ///
    /// **No prefix is held twice.** A queued frame names the path it sits under by its index in
    /// `out.paths`, so a prefix lives only in the path the vocabulary carries; and each candidate
    /// path is charged its bytes before it is built, whether or not it is then advertised.
    fn enumerate_into(
        &self,
        out: &mut Vocabulary,
        budget: &mut Budget<'s>,
    ) -> Result<(), BoundKind> {
        let root = self.frame([self.root], budget)?;
        // (the index in `out.paths` of the path a frame sits under — none for the root; the frame)
        let mut queue: VecDeque<(Option<usize>, Vec<&'s Value>)> = VecDeque::from([(None, root)]);
        while let Some((parent, frame)) = queue.pop_front() {
            let depth = parent.map_or(0, |at| out.paths[at].path.len());
            if depth == self.bounds.max_depth {
                // Whether keys exist below the bound, read without collecting them: a cut costs the
                // frame, not the keys under it. Whether each is declared is past the bound too, so
                // this says "there may be more" — the honest reading of a depth cut.
                if frame.iter().any(|node| declares_properties(node)) {
                    out.truncated.insert(BoundKind::Depth);
                }
                continue;
            }
            for key in child_keys(&frame, budget)? {
                let prefix: &[String] = match parent {
                    Some(at) => &out.paths[at].path,
                    None => &[],
                };
                let bytes = prefix.iter().map(String::len).sum::<usize>() + key.len();
                budget.spend(1 + bytes / BYTES_PER_VISIT)?;
                let mut path = Vec::with_capacity(prefix.len() + 1);
                path.extend_from_slice(prefix);
                path.push(key.to_string());
                let facts = match self.resolve_within(&path, budget) {
                    PathResolution::Declared(facts) => facts,
                    PathResolution::NotDeclared => continue,
                    PathResolution::BeyondBound(bound) => return Err(bound),
                };
                if out.paths.len() == self.bounds.max_paths {
                    return Err(BoundKind::Paths);
                }
                out.paths.push(AskablePath::new(path, facts));
                let at = out.paths.len() - 1;
                // Each frame node is asked for the key, so the child frame costs the frame's size.
                budget.spend(frame.len())?;
                let children = frame
                    .iter()
                    .copied()
                    .filter_map(|node| node.get("properties").and_then(|p| p.get(key)));
                queue.push_back((Some(at), self.frame(children, budget)?));
            }
        }
        Ok(())
    }

    /// The schema objects whose `properties` apply at one location — the given nodes and what their
    /// `$ref`, `allOf`, `anyOf` and `oneOf` reach — and only those: a node without `properties` is
    /// expanded through and not kept. Each node is expanded once, from an explicit stack, and every
    /// node taken off the stack — expanded, already seen, or not a schema object — is spent from
    /// the budget, so a list of branches costs its length however many of them are skipped.
    fn frame(
        &self,
        nodes: impl IntoIterator<Item = &'s Value>,
        budget: &mut Budget<'_>,
    ) -> Result<Vec<&'s Value>, BoundKind> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut stack: Vec<&'s Value> = nodes.into_iter().collect();
        while let Some(node) = stack.pop() {
            budget.spend(1)?;
            let Value::Object(map) = node else {
                continue;
            };
            if !seen.insert(identity(node)) {
                continue;
            }
            if let Some(target) = self.target(node) {
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

    /// The subschema the `$ref` on `from` names, if the walk follows it — read from the references
    /// [`local_references`] resolved when the walk was built, never from the reference's text.
    fn target(&self, from: &Value) -> Option<&'s Value> {
        self.refs.get(&identity(from)).copied()
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

/// Every `$ref` the walk follows, resolved once: each object anywhere in the schema that carries a
/// `#`-prefixed `$ref` → the subschema its fragment names, percent-decoded and read as a JSON
/// pointer from the root (`~0`/`~1` unescaped by `Value::pointer`). `#` alone is the root.
///
/// Every object, not only schema positions, because a `$ref` may point anywhere and the walk then
/// reads what it finds there as a schema. One pass from an explicit stack, and each reference's
/// text is decoded once, so the whole costs time linear in the schema — bounded, like the byte
/// measurement before it, by [`MAX_SCHEMA_BYTES`] — and a visit costs a lookup.
///
/// Not followed, so absent here: an anchor, a remote or `file:` URI (§3.3, the remote-`$ref` row),
/// a pointer that names nothing, and any reference made from inside an embedded resource, where `#`
/// means that resource and not the root.
fn local_references<'s>(root: &'s Value, embedded: &HashSet<usize>) -> HashMap<usize, &'s Value> {
    let mut refs = HashMap::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                let target = map
                    .get("$ref")
                    .and_then(Value::as_str)
                    .and_then(|reference| reference.strip_prefix('#'))
                    .filter(|_| !embedded.contains(&identity(node)))
                    .and_then(percent_decode)
                    .and_then(|pointer| root.pointer(&pointer));
                if let Some(target) = target {
                    refs.insert(identity(node), target);
                }
                stack.extend(map.values());
            }
            Value::Array(elements) => stack.extend(elements),
            _ => {}
        }
    }
    refs
}

/// The schema objects inside a resource the schema embeds: a non-root object that opens one, and
/// every schema object beneath it. One pass from an explicit stack, bounded like the byte
/// measurement beside it by [`MAX_SCHEMA_BYTES`].
///
/// It descends schema positions only. The values of `enum`, `const`, `default` and `examples` are
/// data, and the values of `properties`, `patternProperties`, `$defs`, `definitions`,
/// `dependentSchemas` and `dependencies` are maps of name to schema — so a property named `$id`, or
/// an `enum` member carrying one, is not a resource.
fn embedded_resources(root: &Value, draft: Draft) -> HashSet<usize> {
    let mut marked = HashSet::new();
    // (node, inside an embedded resource, a map of name to schema rather than a schema)
    let mut stack = vec![(root, false, false)];
    while let Some((node, inside, names)) = stack.pop() {
        match node {
            Value::Object(map) if names => {
                stack.extend(map.values().map(|schema| (schema, inside, false)));
            }
            Value::Object(map) => {
                let inside = inside || (!std::ptr::eq(node, root) && draft.opens_resource(map));
                if inside {
                    marked.insert(identity(node));
                }
                for (keyword, child) in map {
                    match keyword.as_str() {
                        "enum" | "const" | "default" | "examples" => {}
                        "properties" | "patternProperties" | "$defs" | "definitions"
                        | "dependentSchemas" | "dependencies" => {
                            stack.push((child, inside, true));
                        }
                        _ => stack.push((child, inside, false)),
                    }
                }
            }
            Value::Array(schemas) => {
                stack.extend(schemas.iter().map(|schema| (schema, inside, false)));
            }
            _ => {}
        }
    }
    marked
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

/// The askable property keys any schema object in a frame declares, in byte order. Every key read
/// is charged one visit before it is collected.
///
/// A key longer than an asked segment may be (`MAX_ARTIFACT_PATH_SEGMENT_BYTES`, the form cap
/// `validate_ask` refuses a longer segment by) is skipped: no ask can name it, so it is never
/// advertised and nothing below it is walked. Its length is read first, which is all a long key
/// costs — escaping only lengthens a key, so one longer than the cap is past it.
fn child_keys<'s>(
    frame: &[&'s Value],
    budget: &mut Budget<'_>,
) -> Result<BTreeSet<&'s str>, BoundKind> {
    let mut keys = BTreeSet::new();
    let declared = frame
        .iter()
        .copied()
        .filter_map(|node| node.get("properties").and_then(Value::as_object));
    for props in declared {
        budget.spend(props.len())?;
        keys.extend(
            props
                .keys()
                .map(String::as_str)
                .filter(|key| askable_segment(key)),
        );
    }
    Ok(keys)
}

/// Whether a key could be one segment of an asked path: within `MAX_ARTIFACT_PATH_SEGMENT_BYTES`,
/// JSON-escaped, as `validate_ask`'s form check measures it.
fn askable_segment(key: &str) -> bool {
    key.len() <= MAX_ARTIFACT_PATH_SEGMENT_BYTES
        && json_string_bytes(key) <= MAX_ARTIFACT_PATH_SEGMENT_BYTES
}

/// Whether a frame node declares any property at all.
fn declares_properties(node: &Value) -> bool {
    node.get("properties")
        .and_then(Value::as_object)
        .is_some_and(|props| !props.is_empty())
}

/// A subschema's identity for memoising and cycle-guarding: its address inside the borrowed root,
/// which is stable for the walk's lifetime.
fn identity(node: &Value) -> usize {
    std::ptr::from_ref(node) as usize
}

/// One `enum` / `const` member as the walk carries it: an id equal for jsonb-equal members, so
/// sets intersect and union by id in linear time; its weight in visits; and the schema value.
#[derive(Debug, Clone, Copy)]
pub(super) struct Member<'s> {
    id: usize,
    weight: usize,
    pub(super) value: &'s Value,
}

/// A value set, shared rather than copied when a result is memoised or reused.
pub(super) type Values<'s> = Rc<Vec<Member<'s>>>;

/// What the walk has found at a path: [`PathFacts`], with its values as members of the schema and
/// every part shared, so carrying it costs nothing per value. `validate_ask` reads an ask's paths
/// in this form and never copies them; only the wire copies, charged by [`into_wire`].
#[derive(Debug, Clone, Default)]
pub(super) struct Facts<'s> {
    pub(super) types: Option<BTreeSet<JsonType>>,
    pub(super) values: Option<Values<'s>>,
    pub(super) items: Option<Rc<Facts<'s>>>,
}

impl Facts<'_> {
    /// Whether nothing at all is declared — the facts of `{}` or `true`.
    fn is_unconstrained(&self) -> bool {
        self.types.is_none() && self.values.is_none() && self.items.is_none()
    }

    /// The facts as the wire carries them, every value copied. Uncharged here: [`into_wire`]
    /// charges a copy before making it, and a refusal's teaching copies only a path it has first
    /// measured, through [`AskableView`], to fit.
    pub(super) fn to_wire(&self) -> PathFacts {
        PathFacts {
            types: self.types.clone(),
            values: self.values.as_ref().map(|members| {
                #[cfg(test)]
                VALUES_COPIED.with(|copied| copied.set(copied.get() + members.len()));
                members.iter().map(|m| m.value.clone()).collect()
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
    pub(super) static VALUES_COPIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
            Some(members) => out.serialize_field("values", &MemberValues(members.as_slice()))?,
            None => out.skip_field("values")?,
        }
        match &self.items {
            Some(element) => out.serialize_field("items", &**element)?,
            None => out.skip_field("items")?,
        }
        out.end()
    }
}

/// A value set written as the wire writes it: its members' values, in order.
struct MemberValues<'a, 's>(&'a [Member<'s>]);

impl Serialize for MemberValues<'_, '_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|m| m.value))
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

/// The facts as the wire carries them, each value copied once and charged by its weight first.
fn into_wire(facts: &Facts<'_>, budget: &mut Budget<'_>) -> Result<PathFacts, BoundKind> {
    charge_copy(facts, budget)?;
    Ok(facts.to_wire())
}

/// What copying facts onto the wire costs: every member's weight, at every array level.
fn charge_copy(facts: &Facts<'_>, budget: &mut Budget<'_>) -> Result<(), BoundKind> {
    if let Some(members) = &facts.values {
        budget.spend(members.iter().map(|m| m.weight).sum())?;
    }
    match &facts.items {
        Some(element) => charge_copy(element, budget),
        None => Ok(()),
    }
}

/// What one subschema says about the remaining path.
#[derive(Debug, Clone)]
enum Reach<'s> {
    /// The path can never be present (`false`, or every branch of a union forbids it).
    Forbidden,
    /// This subschema does not reach the path; it neither declares nor constrains it.
    Silent,
    Declared(Facts<'s>),
}

/// Both hold — `allOf`, and keywords beside a `$ref` under 2019-09 / 2020-12.
fn conjoin<'s>(
    a: Reach<'s>,
    b: Reach<'s>,
    budget: &mut Budget<'_>,
) -> Result<Reach<'s>, BoundKind> {
    Ok(match (a, b) {
        (Reach::Forbidden, _) | (_, Reach::Forbidden) => Reach::Forbidden,
        (Reach::Silent, other) | (other, Reach::Silent) => other,
        (Reach::Declared(x), Reach::Declared(y)) => Reach::Declared(conjoin_facts(&x, &y, budget)?),
    })
}

/// One of them holds — `anyOf` / `oneOf` (§3.3 row 6). A branch that does not declare the path
/// takes no part; if none declares it the union is silent, unless every branch forbids it.
fn disjoin<'s>(branches: Vec<Reach<'s>>, budget: &mut Budget<'_>) -> Result<Reach<'s>, BoundKind> {
    let all_forbidden =
        !branches.is_empty() && branches.iter().all(|b| matches!(b, Reach::Forbidden));
    let mut union: Option<Facts<'s>> = None;
    for branch in branches {
        if let Reach::Declared(facts) = branch {
            union = Some(match union {
                Some(acc) => disjoin_facts(&acc, &facts, budget)?,
                None => facts,
            });
        }
    }
    Ok(match union {
        Some(facts) => Reach::Declared(facts),
        None if all_forbidden => Reach::Forbidden,
        None => Reach::Silent,
    })
}

fn conjoin_facts<'s>(
    a: &Facts<'s>,
    b: &Facts<'s>,
    budget: &mut Budget<'_>,
) -> Result<Facts<'s>, BoundKind> {
    let values = match (&a.values, &b.values) {
        (Some(x), Some(y)) => Some(intersect(x, y, budget)?),
        (x, None) | (None, x) => x.clone(),
    };
    let items = match (&a.items, &b.items) {
        (Some(x), Some(y)) => Some(Rc::new(conjoin_facts(x, y, budget)?)),
        (x, None) | (None, x) => x.clone(),
    };
    Ok(Facts {
        types: both(a.types.clone(), b.types.clone(), intersect_types),
        values,
        items,
    })
}

/// A union's facts: unconstrained if either side is — so a value one branch admits is never
/// refused because another branch enumerates (§3.3 row 6).
fn disjoin_facts<'s>(
    a: &Facts<'s>,
    b: &Facts<'s>,
    budget: &mut Budget<'_>,
) -> Result<Facts<'s>, BoundKind> {
    let values = match (&a.values, &b.values) {
        (Some(x), Some(y)) => Some(union(x, y, budget)?),
        _ => None,
    };
    let items = match (&a.items, &b.items) {
        (Some(x), Some(y)) => Some(Rc::new(disjoin_facts(x, y, budget)?)),
        _ => None,
    };
    Ok(Facts {
        types: either(a.types.clone(), b.types.clone(), |mut x, y| {
            x.extend(y);
            x
        }),
        values,
        items,
    })
}

/// The members of `x` some member of `y` equals, in `x`'s order. Linear, and charged by both sizes.
fn intersect<'s>(
    x: &Values<'s>,
    y: &Values<'s>,
    budget: &mut Budget<'_>,
) -> Result<Values<'s>, BoundKind> {
    budget.spend(x.len() + y.len())?;
    let keep: HashSet<usize> = y.iter().map(|m| m.id).collect();
    Ok(Rc::new(
        x.iter().filter(|m| keep.contains(&m.id)).copied().collect(),
    ))
}

/// `x`, then the members of `y` it lacks. Linear, and charged by both sizes.
fn union<'s>(
    x: &Values<'s>,
    y: &Values<'s>,
    budget: &mut Budget<'_>,
) -> Result<Values<'s>, BoundKind> {
    budget.spend(x.len() + y.len())?;
    let mut seen: HashSet<usize> = x.iter().map(|m| m.id).collect();
    let mut out: Vec<Member<'s>> = x.to_vec();
    out.extend(y.iter().filter(|m| seen.insert(m.id)).copied());
    Ok(Rc::new(out))
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
    Combine(&'s Value, &'s Map<String, Value>, usize),
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
    budget: &'w mut Budget<'s>,
    memo: HashMap<(usize, usize), Reach<'s>>,
    active: HashSet<(usize, usize)>,
}

impl<'s> Resolver<'_, 's> {
    fn run(mut self) -> Result<Reach<'s>, BoundKind> {
        self.budget.spend(1)?;
        let mut tasks = vec![Task::Eval(self.walk.root, 0)];
        let mut values: Vec<Reach<'s>> = Vec::new();
        while let Some(task) = tasks.pop() {
            match task {
                Task::Eval(node, at) => self.eval(node, at, &mut tasks, &mut values)?,
                Task::Combine(node, map, at) => {
                    let children = self.children(node, map, at);
                    // Each child's evaluation pushed exactly one value, after this task was pushed
                    // and before it is popped.
                    let results = values.split_off(values.len() - children.len());
                    let reached = self.combine(map, at, &children, results)?;
                    let key = (identity(node), at);
                    self.active.remove(&key);
                    self.memo.insert(key, reached.clone());
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
        values: &mut Vec<Reach<'s>>,
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
                    let children = self.children(node, map, at);
                    self.budget.spend(children.len())?;
                    self.active.insert(key);
                    tasks.push(Task::Combine(node, map, at));
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
    fn unconstrained(&self, at: usize) -> Reach<'s> {
        if at >= self.path.len() {
            Reach::Declared(Facts::default())
        } else {
            Reach::Silent
        }
    }

    /// The subschemas an object applies at a position, in a fixed order the combination relies on.
    fn children(&self, node: &'s Value, map: &'s Map<String, Value>, at: usize) -> Vec<Child<'s>> {
        let mut out = Vec::new();
        if let Some(target) = self.walk.target(node) {
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
        &mut self,
        map: &'s Map<String, Value>,
        at: usize,
        children: &[Child<'s>],
        results: Vec<Reach<'s>>,
    ) -> Result<Reach<'s>, BoundKind> {
        if self.walk.draft.ref_overrides_siblings() && map.contains_key("$ref") {
            return Ok(results
                .into_iter()
                .next()
                .unwrap_or_else(|| self.unconstrained(at)));
        }
        let mut step = Reach::Silent;
        let mut items = None;
        let mut conjuncts = Vec::new();
        let mut any_of: Option<Vec<Reach<'s>>> = None;
        let mut one_of: Option<Vec<Reach<'s>>> = None;
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
            Reach::Declared(self.local_facts(map, items)?)
        } else {
            step
        };
        for conjunct in conjuncts {
            acc = conjoin(acc, conjunct, self.budget)?;
        }
        for branches in [any_of, one_of].into_iter().flatten() {
            let union = disjoin(branches, self.budget)?;
            acc = conjoin(acc, union, self.budget)?;
        }
        Ok(acc)
    }

    /// What one schema object itself declares at the path's end: `type`, `enum` / `const`, and
    /// what its element schema reached. A list of `type` names costs its length, like an `enum`.
    fn local_facts(
        &mut self,
        map: &'s Map<String, Value>,
        items: Option<Reach<'s>>,
    ) -> Result<Facts<'s>, BoundKind> {
        let types = match map.get("type") {
            None => None,
            Some(Value::String(name)) => Some(JsonType::from_keyword(name).into_iter().collect()),
            Some(Value::Array(names)) => {
                self.budget.spend(names.len())?;
                Some(
                    names
                        .iter()
                        .filter_map(|n| n.as_str().and_then(JsonType::from_keyword))
                        .collect(),
                )
            }
            Some(_) => Some(BTreeSet::new()),
        };
        let enumerated = match map.get("enum") {
            Some(keyword) => match keyword.as_array() {
                Some(members) => Some(self.budget.members(keyword, members)?),
                None => None,
            },
            None => None,
        };
        let constant = match map.get("const").filter(|_| self.walk.draft.has_const()) {
            Some(value) => Some(self.budget.members(value, std::slice::from_ref(value))?),
            None => None,
        };
        let values = match (enumerated, constant) {
            (Some(x), Some(y)) => Some(intersect(&x, &y, self.budget)?),
            (x, None) | (None, x) => x,
        };
        let items = match items {
            None | Some(Reach::Silent) => None,
            // `items: false` admits no element at all.
            Some(Reach::Forbidden) => Some(Rc::new(Facts {
                types: Some(BTreeSet::new()),
                ..Facts::default()
            })),
            Some(Reach::Declared(facts)) if facts.is_unconstrained() => None,
            Some(Reach::Declared(facts)) => Some(Rc::new(facts)),
        };
        Ok(Facts {
            types,
            values,
            items,
        })
    }
}

#[cfg(test)]
mod tests {
    //! The §3.3 table, row by row, each with a hand-written expected [`PathResolution`].

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

    /// `$defs.E` is an enum of 8,000 numbers; `a` is a union or intersection of 9,000 references
    /// to it. About 230 KB and 18,000 subschema visits, so it fits both the byte and the visit
    /// bound.
    fn wide_enum_references(keyword: &str) -> Value {
        let members: Vec<Value> = (0..8_000).map(|i| json!(i)).collect();
        let references: Vec<Value> = (0..9_000).map(|_| json!({ "$ref": "#/$defs/E" })).collect();
        json!({
            "$defs": { "E": { "enum": members } },
            "properties": { "a": { keyword: references } }
        })
    }

    /// Catches the regression where values were cloned per reference and intersected pairwise
    /// outside the budget (round-3 review N1): ~72M `Value`s held at once and ~10¹¹ comparisons
    /// for this schema, under every bound the walk then had.
    #[test]
    fn values_carried_through_a_wide_all_of_or_any_of_are_charged_and_stop_at_the_work_budget() {
        for keyword in ["allOf", "anyOf"] {
            let s = wide_enum_references(keyword);
            assert!(
                json_bytes(&s) <= MAX_SCHEMA_BYTES,
                "{keyword}: under the byte bound"
            );
            assert_eq!(
                at(&s, &["a"]),
                PathResolution::BeyondBound(BoundKind::Work),
                "{keyword}"
            );
            assert_eq!(
                enumerate(&s, &WalkBounds::BUILD),
                Vocabulary {
                    paths: vec![],
                    truncated: BTreeSet::from([BoundKind::Work]),
                },
                "{keyword}"
            );
        }
    }

    #[test]
    fn a_ref_inside_an_embedded_resource_is_not_resolved_against_the_root() {
        // `#/$defs/x` inside `child` means child's own `$defs/x` (an integer), not the root's (a
        // string). The walk does not follow it, so it says nothing it cannot see.
        let s = json!({
            "$defs": { "x": { "type": "string" } },
            "properties": {
                "child": {
                    "$id": "https://example.com/child",
                    "$defs": { "x": { "type": "integer" } },
                    "properties": { "y": { "$ref": "#/$defs/x" }, "self": { "$ref": "#" } }
                },
                "z": { "$ref": "#/$defs/x" }
            }
        });
        assert_eq!(at(&s, &["child", "y"]), untyped());
        assert_eq!(at(&s, &["child", "self"]), untyped());
        assert_eq!(at(&s, &["child", "self", "y"]), PathResolution::NotDeclared);
        // Outside the resource the root's definition still applies.
        assert_eq!(at(&s, &["z"]), typed(&[JsonType::String]));
    }

    #[test]
    fn data_and_name_maps_carrying_an_id_open_no_resource() {
        // A property named `$id`, and an `$id` inside an `enum` member, are not resources: the
        // references beside them still resolve against the root.
        let s = json!({
            "$defs": { "x": { "type": "string" } },
            "properties": {
                "$id": { "$ref": "#/$defs/x" },
                "e": { "enum": [{ "$id": "https://example.com/data" }], "$ref": "#/$defs/x" }
            }
        });
        assert_eq!(at(&s, &["$id"]), typed(&[JsonType::String]));
        assert_eq!(
            at(&s, &["e"]),
            PathResolution::Declared(PathFacts {
                types: types(&[JsonType::String]),
                values: Some(vec![json!({ "$id": "https://example.com/data" })]),
                items: None,
            })
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

    // ── Every size an author controls is charged (RG-2) ───────────────────────────────────────

    /// The least visit budget under which `enumerate` finishes without a `work` cut, under a depth
    /// bound. A run spends the same visits whatever its budget, and finishes exactly when they fit,
    /// so this is what the walk charged — a deterministic measure, not a timing.
    fn least_visits(schema: &Value, max_depth: usize) -> usize {
        let finishes = |max_visits| {
            let bounds = WalkBounds {
                max_depth,
                max_visits,
                ..WalkBounds::BUILD
            };
            !enumerate(schema, &bounds)
                .truncated
                .contains(&BoundKind::Work)
        };
        assert!(finishes(MAX_WALK_VISITS), "fits the build's budget");
        let (mut lo, mut hi) = (0, MAX_WALK_VISITS);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if finishes(mid) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        lo
    }

    /// RG-2 F1, the reported attack, unscaled: two keys of 131,000 bytes, each referencing the
    /// root, fit the byte bound and declare a two-way fan-out every path of which carries them.
    /// Bites the key skip in `child_keys`: without it the vocabulary advertises paths built of
    /// those keys — about 1 GB of path text at the path bound with no byte charge, a `work` cut
    /// after about a dozen paths of 131 KB or more with it — where with the skip there is only
    /// `ok`.
    #[test]
    fn a_key_too_long_to_ask_is_never_advertised_nor_walked_below() {
        let mut props = Map::new();
        props.insert("a".repeat(131_000), json!({ "$ref": "#" }));
        props.insert("b".repeat(131_000), json!({ "$ref": "#" }));
        props.insert("ok".to_string(), json!({ "type": "string" }));
        let s = json!({ "properties": props });
        assert!(json_bytes(&s) <= MAX_SCHEMA_BYTES, "{}", json_bytes(&s));
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

    fn two_way_fan_out(a: &str, b: &str) -> Value {
        json!({ "properties": { a: { "$ref": "#" }, b: { "$ref": "#" } } })
    }

    /// RG-2 F1, the byte charge. Keys of `MAX_ARTIFACT_PATH_SEGMENT_BYTES` (256) are askable, so
    /// the skip keeps them and only the charge sees their length. Under a depth bound of 3 the
    /// fan-out builds 2 paths of one segment, 4 of two and 8 of three; each 256-byte segment costs
    /// 256 / 64 = 4 visits more than a one-byte one, so the long fan-out costs
    /// 2·4 + 4·8 + 8·12 = 136 visits more. Bites the per-path charge in `enumerate_into`: without
    /// it the two cost the same.
    #[test]
    fn every_path_built_is_charged_its_bytes() {
        let short = two_way_fan_out("a", "b");
        let long = two_way_fan_out(
            &"a".repeat(MAX_ARTIFACT_PATH_SEGMENT_BYTES),
            &"b".repeat(MAX_ARTIFACT_PATH_SEGMENT_BYTES),
        );
        assert_eq!(least_visits(&long, 3) - least_visits(&short, 3), 136);
        let three = WalkBounds {
            max_depth: 3,
            ..WalkBounds::BUILD
        };
        let v = enumerate(&long, &three);
        assert_eq!(v.paths.len(), 14);
        assert_eq!(v.truncated, BTreeSet::from([BoundKind::Depth]));
    }

    /// RG-2 F2(A), the key charge: each key a frame declares costs one visit as it is read, even
    /// one too long to ask, which is never walked. One hundred such keys beside `ok` cost exactly
    /// one hundred visits more. Bites the charge in `child_keys`: without it they cost nothing.
    #[test]
    fn every_key_a_frame_declares_is_charged_even_one_too_long_to_ask() {
        let ok = json!({ "properties": { "ok": {} } });
        let mut props = Map::new();
        props.insert("ok".to_string(), json!({}));
        for i in 0..100 {
            props.insert(
                format!("{i:03}{}", "k".repeat(MAX_ARTIFACT_PATH_SEGMENT_BYTES)),
                json!({}),
            );
        }
        let crowded = json!({ "properties": props });
        assert_eq!(
            enumerate(&crowded, &WalkBounds::BUILD),
            enumerate(&ok, &WalkBounds::BUILD),
            "the long keys are not advertised"
        );
        assert_eq!(
            least_visits(&crowded, MAX_WALK_DEPTH) - least_visits(&ok, MAX_WALK_DEPTH),
            100
        );
    }

    /// RG-2 F2(A), the depth cut: a frame at the depth bound is read for whether it declares any
    /// property, never for its keys, so a thousand keys under the cut cost what one does. Bites the
    /// order in `enumerate_into`: collecting (and charging) keys before checking the depth would
    /// cost 999 visits more here.
    #[test]
    fn a_depth_cut_costs_nothing_per_key_below_it() {
        let under = |n: usize| {
            let keys: Map<String, Value> = (0..n).map(|i| (format!("k{i}"), json!({}))).collect();
            json!({ "properties": { "a": { "properties": keys } } })
        };
        let (one, many) = (under(1), under(1_000));
        assert_eq!(least_visits(&many, 1), least_visits(&one, 1));
        let v = enumerate(
            &many,
            &WalkBounds {
                max_depth: 1,
                ..WalkBounds::BUILD
            },
        );
        assert_eq!(v.paths.len(), 1);
        assert_eq!(v.truncated, BTreeSet::from([BoundKind::Depth]));
    }

    /// RG-2 sweep: a frame expansion charges every node it takes off its stack, so an `allOf` of
    /// `true`s — not schema objects, never expanded — still costs its length. One thousand of them
    /// cost 2,000 visits more: 1,000 in the root's frame and 1,000 as the root's children when `x`
    /// is resolved. Bites the per-pop charge in `frame`: charging only the objects it expands, the
    /// frame's thousand pops are free and the difference is 1,000.
    #[test]
    fn a_frame_charges_every_branch_it_takes_even_one_it_skips() {
        let with = |n: usize| {
            let trues: Vec<Value> = (0..n).map(|_| json!(true)).collect();
            json!({ "properties": { "x": {} }, "allOf": trues })
        };
        assert_eq!(
            least_visits(&with(1_000), MAX_WALK_DEPTH) - least_visits(&with(0), MAX_WALK_DEPTH),
            2_000
        );
    }

    /// RG-2 sweep: a `type` list costs its length each time it is read, like an `enum`. One
    /// thousand names cost 999 visits more than one. Bites the charge in `local_facts`: without it
    /// the list is read free at every visit.
    #[test]
    fn a_list_of_type_names_is_charged_its_length() {
        let with = |n: usize| {
            let names: Vec<Value> = (0..n).map(|_| json!("string")).collect();
            json!({ "properties": { "x": { "type": names } } })
        };
        assert_eq!(
            least_visits(&with(1_000), MAX_WALK_DEPTH) - least_visits(&with(1), MAX_WALK_DEPTH),
            999
        );
    }

    /// RG-2 F2(B): a `$ref` is decoded and resolved once, when the walk is built, and a visit reads
    /// that resolution — never the reference's text. Bites resolving per visit: a lookup that read
    /// the text would resolve the stranger below, which the walk was never built over.
    #[test]
    fn a_ref_is_resolved_when_the_walk_is_built_not_when_it_is_visited() {
        let s = json!({
            "$defs": { "x": { "type": "string" } },
            "properties": { "a": { "$ref": "#/$defs/x" } }
        });
        let walk = Walk::new(&s, &WalkBounds::BUILD).expect("under the byte bound");
        assert!(walk
            .target(&s["properties"]["a"])
            .is_some_and(|target| std::ptr::eq(target, &s["$defs"]["x"])));
        let stranger = json!({ "$ref": "#/$defs/x" });
        assert_eq!(walk.target(&stranger), None);
    }

    /// RG-2 F4: the schema is measured only until it passes the bound. Serializing an array of
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

    // ── Copies cost what they allocate, and a shared set is read once (RG-2, round 2) ─────────

    /// RG-2 round 2, N2: a member is charged for the values nested in it, not only for its text. A
    /// member 31 arrays deep has 62 canonical bytes, as a 60-letter string does, but holds 30
    /// nested values, each cloned as its own `Value` in its own allocation; charged at `NODE_BYTES`
    /// each it weighs 1 + (62 + 30 · 64) / 64 = 31 visits against the string's 1. Enumerating its
    /// one path reads the member once and copies it once: 2 · 30 = 60 visits more. Bites the node
    /// charge in `Budget::member`: without it the two cost the same.
    #[test]
    fn a_member_is_charged_for_its_nested_values_not_only_its_bytes() {
        let mut deep = json!([]);
        for _ in 0..30 {
            deep = json!([deep]);
        }
        let text = json!("x".repeat(60));
        assert_eq!(canonical(&deep).len(), 62);
        assert_eq!(canonical(&text).len(), 62);
        let at_x = |member: &Value| json!({ "properties": { "x": { "enum": [member] } } });
        assert_eq!(
            least_visits(&at_x(&deep), MAX_WALK_DEPTH) - least_visits(&at_x(&text), MAX_WALK_DEPTH),
            60
        );
    }

    /// The least visit budget under which `resolve_all` declares every path — what it charged.
    fn least_visits_to_resolve(schema: &Value, paths: &[&[String]]) -> usize {
        let resolves = |max_visits| {
            let bounds = WalkBounds {
                max_visits,
                ..WalkBounds::BUILD
            };
            let walk = Walk::new(schema, &bounds).expect("under the byte bound");
            walk.resolve_all(paths)
                .iter()
                .all(|r| matches!(r, Resolved::Declared(_)))
        };
        assert!(resolves(MAX_WALK_VISITS), "fits the build's budget");
        let (mut lo, mut hi) = (0, MAX_WALK_VISITS);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if resolves(mid) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        lo
    }

    /// RG-2 round 2, N3: eight paths over one shared 5,000-member enum cost what one does, plus a
    /// few visits a path. One path reads the enum — 5,000 members and the 5,000-member set — so it
    /// costs at least 10,000; seven more paths cost under 100 more, where reading the set again or
    /// copying it for each would cost 5,000 or more a path. Bites the per-budget set memo in
    /// `Budget::members`, and an ask's `resolve_all` copying its values onto the wire.
    #[test]
    fn paths_sharing_one_enum_read_it_once_under_one_budget() {
        let members: Vec<Value> = (0..5_000).map(|i| json!(i)).collect();
        let props: Map<String, Value> = (0..8)
            .map(|i| (format!("p{i}"), json!({ "$ref": "#/$defs/e" })))
            .collect();
        let s = json!({ "$defs": { "e": { "enum": members } }, "properties": props });
        let owned: Vec<Vec<String>> = (0..8).map(|i| path(&[format!("p{i}").as_str()])).collect();
        let paths: Vec<&[String]> = owned.iter().map(Vec::as_slice).collect();
        let one = least_visits_to_resolve(&s, &paths[..1]);
        let eight = least_visits_to_resolve(&s, &paths);
        assert!(one >= 10_000, "{one}");
        assert!(eight - one < 100, "{one} for one path, {eight} for eight");
    }

    /// RG-2 round 3, R3-1: the borrowed view writes exactly the bytes of the copy it stands for —
    /// every field present only when set, at every items level, over enum members, a `const`, an
    /// enum and a `const` together, and `items: false` — so measuring the view measures the copy.
    /// Bites any drift between `Facts`'s `Serialize` and `PathFacts`'s derived one.
    #[test]
    fn the_borrowed_view_writes_the_wire_bytes() {
        let s = json!({
            "$defs": { "codes": { "enum": ["ms", 7, null, { "k": [1, 2.5] }, ["x"]] } },
            "properties": {
                "plain": {},
                "typed": { "type": ["integer", "null"] },
                "coded": { "$ref": "#/$defs/codes" },
                "constant": { "type": "string", "const": "x" },
                "both": { "enum": ["x", "y"], "const": "x" },
                "grid": { "type": "array", "items": {
                    "type": "array", "items": { "$ref": "#/$defs/codes" }
                } },
                "never": { "type": "array", "items": false }
            }
        });
        let walk = Walk::new(&s, &WalkBounds::BUILD).expect("under the byte bound");
        let names = [
            "plain", "typed", "coded", "constant", "both", "grid", "never",
        ];
        let owned: Vec<Vec<String>> = names.iter().map(|name| path(&[*name])).collect();
        let paths: Vec<&[String]> = owned.iter().map(Vec::as_slice).collect();
        for (asked, resolved) in paths.iter().zip(walk.resolve_all(&paths)) {
            let Resolved::Declared(facts) = resolved else {
                panic!("{asked:?} is declared");
            };
            let view = AskableView::new(asked, &facts);
            let wire = AskablePath::new(asked.to_vec(), facts.to_wire());
            let viewed = serde_json::to_vec(&view).expect("the view serializes");
            assert_eq!(
                viewed,
                serde_json::to_vec(&wire).expect("the wire serializes"),
                "{asked:?}"
            );
            assert_eq!(json_bytes_up_to(&view, usize::MAX), viewed.len());
            assert_eq!(view.into_askable(), wire);
        }
    }
}
