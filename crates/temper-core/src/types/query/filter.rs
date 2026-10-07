//! The predicate layer. Bounds are membership and terms are magnitude; neither can narrow by what
//! a thing IS. Everything carries `kb_properties` (`doc_type`, `tags`, `facet`) and every edge
//! carries BOTH an `edge_kind` and a `label`.
//!
//! Typed slots, deliberately NOT a generic `{field, op, value}` grammar: a general predicate
//! language would be more expressive and would immediately re-open every conflation this contract
//! exists to close.

use serde::{Deserialize, Serialize};

// The four members of the DDL's `edge_kind` enum
// (`migrations/20260624000001_canonical_schema.sql:95`) are ALREADY modelled, and re-used here
// rather than restated. `types::graph::EdgeKind` is `sqlx::Type`-bound to that DDL, so it is the
// copy a schema change breaks — which is exactly why the contract must not carry a second one.
// Its closedness is the fix for the audit's #1 finding: an edge `label` such as `advances` cannot
// be passed here.
use crate::types::graph::EdgeKind;

/// Narrowing over edges. `edge_kinds` and `labels` are DIFFERENT AXES and are never merged: the
/// kind is a closed DDL enum, the label is free text the caller actually sees on every edge.
///
/// # Every field here constrains a HOP, and that is why they live in a container
///
/// `[decided — 2026-08-14, Pete]` *A narrowing that can be expressed as a set must be an act. A
/// narrowing that cannot be a set belongs to the act whose semantics it constrains.* An edge
/// predicate has no set-shaped substitute: binding a walk by *"nodes that participate in an edge
/// matching P"* admits a node because it has a matching edge **somewhere** and then walks it through
/// a different, non-matching one — a different question, returning plausible rows and looking like
/// it narrowed. So these constrain the traversal from inside it, and the only act that traverses an
/// edge is `follow-from`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
pub struct EdgeFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edge_kinds: Vec<EdgeKind>,
    /// Edge labels, OR within the list. Each label is at most 256 bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "web-api", schema(max_items = 256))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 256), inner(length(max = 256))))]
    pub labels: Vec<String>,
    /// `kb_properties` rows owned by the edge itself: open key space, closed operator set.
    /// AND across the list, OR within a [`PropertyOp::Contains`].
    ///
    /// **This is where an edge property predicate lives, and the container is the point.** It moved
    /// off [`super::ActInvocation::properties`], where the same field meant different things
    /// depending on which act carried it — which is what a `PropertySubject` tag existed to
    /// disambiguate. Given a container the tag has no job; the subject is the container.
    ///
    /// **Zero edge-owned properties exist in this deployment** `[measured on prod — 2026-08-14]`,
    /// and the storage has admitted them since the schema's first migration (`kb_properties.
    /// owner_table` includes `'kb_edges'`, whose DDL comment has said *"§4a edges carry facets"*
    /// throughout) with a shipped write path `[verified — 20260727000030]`. So this slot narrows
    /// nothing today by data rather than by design.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<PropertyPredicate>,
}

/// One `kb_properties` facet predicate, at the inner-key grain the facet model uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
pub struct FacetPredicate {
    #[cfg_attr(feature = "web-api", schema(max_length = 256))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 256)))]
    pub key: String,
    /// At most 16384 bytes, and counted toward the composition's 1048576-byte total of
    /// property-predicate values.
    #[cfg_attr(feature = "web-api", schema(max_length = 16384))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 16384)))]
    pub value: String,
}

/// Narrowing over resources. Every field is AND-composed; an unset field narrows nothing.
///
/// **No field here has a closed vocabulary, and none is checked against one.**
/// `[corrected — 2026-08-10, ADJ-10]` This claimed `doc_type`, `stage` and `status` were closed
/// vocabularies whose unknown values raise `RefusalReason::UnknownFilterValue`. None of the three
/// is: `stage` and `status` are free-form `Option<String>` and are refused wholesale by this door as
/// `FilterNotApplicable`, and `doc_type` is a `kb_properties` row a resource may carry any value
/// for. `[2026-08-15]` `UnknownFilterValue` used to be raised here for exactly one thing — an
/// unrecognized `PropertySubject` — and **that reason is now gone with the type**, so nothing on
/// this struct raises it.
///
/// The rule that replaces the old claim: *an unknown value in a genuinely closed set* is a refusal,
/// because it can never match; *a string that may be perfectly legitimate and matches nothing in the
/// scope you asked about* is an honest empty. `doc_type` is the second kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
pub struct ResourceFilter {
    /// `kb_properties` where `property_key = 'doc_type'`. Each value is at most 256 bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "web-api", schema(max_items = 256))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 256), inner(length(max = 256))))]
    pub doc_type: Vec<String>,
    /// `kb_properties` where `property_key = 'tags'`. AND-containment. Each tag is at most 256
    /// bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "web-api", schema(max_items = 256))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 256), inner(length(max = 256))))]
    pub tags: Vec<String>,
    /// `kb_properties` where `property_key = 'facet'`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facets: Vec<FacetPredicate>,
    /// `kb_properties` rows owned by the resource itself: open key space, closed operator set.
    /// AND across the list, OR within a [`PropertyOp::Contains`].
    ///
    /// **The three named fields above reach three keys; this one reaches the rest.** Sixty-seven of
    /// the seventy live property keys were narrowable by nothing on any act
    /// `[measured on prod — 2026-08-14]`.
    ///
    /// **The container is the point, and it is the same container `EdgeFilter` has.** This is where
    /// a resource property predicate lives; it moved off `ActInvocation::properties`, where the same
    /// field meant different things depending on which act carried it. Given a container the subject
    /// tag has no job, which is why the tag no longer exists.
    ///
    /// **`Contains` reads the value WHOLE — `kb_resource_properties`, never
    /// `kb_property_elements`** `[decided — 2026-08-15, Pete; 20260815000040]`. So it means exactly
    /// what [`EdgeFilter::properties`]'s `Contains` means. The element relation would silently
    /// narrow the operator: an array-shaped probe matches the whole value and matches *nothing*
    /// against an exploded element, and a `[]`-valued key is a row in the one and no rows in the
    /// other. The element view continues to serve `tags` and `facets`, whose semantics genuinely
    /// are AND-containment over elements.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<PropertyPredicate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "web-api", schema(max_length = 256))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 256)))]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "web-api", schema(max_length = 256))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 256)))]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "web-api", schema(max_length = 256))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 256)))]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "web-api", schema(max_length = 4096))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 4096)))]
    pub title_contains: Option<String>,
}

/// Which filter slot an act admits. An unadmitted filter is DECLINED, never ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum FilterField {
    Resource,
    Edge,
}

/// A property narrowing operator. CLOSED — the key space is open, the operator set is not. No
/// operator takes a fragment of a query language; all bind their values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum PropertyOp {
    /// The key is present at all. A row-existence check on the `property_key` btree — NOT a jsonb
    /// operator, because `jsonb_path_ops` does not index key-existence and the btree already
    /// answers it.
    HasKey,
    /// `property_value @> $v` for any listed value. OR within the predicate, matching the
    /// established within-field OR of `doc_type` and `EdgeFilter.labels`.
    ///
    /// **Containment is ASYMMETRIC, and the asymmetry runs the useful way.**
    /// `[corrected — 2026-08-14]` This said *"containment does not coerce:
    /// `'["x"]'::jsonb @> '"x"'::jsonb` is FALSE, so a type-unstable key needs both shapes
    /// listed."* Measured against Postgres 18, that expression is **TRUE** — it is the documented
    /// special exception whereby a top-level array contains a primitive. The reverse is the false
    /// one:
    ///
    /// ```text
    ///  '["x"]'::jsonb @> '"x"'::jsonb   -> t     (array contains scalar)
    ///  '"x"'::jsonb   @> '["x"]'::jsonb -> f     (scalar does not contain array)
    /// ```
    ///
    /// The row's value is on the LEFT, so a **scalar** probe matches both the array-shaped rows
    /// and the scalar-shaped ones, while an **array** probe matches only the array-shaped rows.
    /// The conclusion therefore survives inverted and weaker than it was stated: a type-unstable
    /// key needs the scalar shape, not both. Listing both is harmless — the values OR — but it is
    /// not what makes the predicate span the population, and a caller who lists only the array
    /// shape silently answers for one half of it.
    ///
    /// **Size.** Each value is at most 16384 bytes of compact JSON and at most 256 nested array
    /// elements or object members, and every predicate value in one composition totals at most
    /// 1048576 bytes (`property_value_too_large`, `property_value_budget_exceeded`).
    Contains { values: Vec<serde_json::Value> },
    /// `property_value <direction> $value` over jsonb's native ordering, type-guarded.
    ///
    /// **The type is inferred from the caller's bound, never declared on the operator.**
    /// `jsonb_typeof(property_value) = jsonb_typeof($value)` segments by JSON type before the
    /// comparison, so a row whose JSON type differs from the bound's is an **honest empty** rather
    /// than a type-confusion match. jsonb defines a total type ordering
    /// (`null < boolean < number < string < array < object`), so without the guard a numeric
    /// bound against a string-valued key would match **every** string row (`string > number` is
    /// true in jsonb's ordering) — a type-confusion artifact, not an answer. The guard makes each
    /// comparison run only within a homogeneous sub-population.
    ///
    /// **Per-VALUE inference, not per-key — and the distinction is the trap.** `temper-pr` is
    /// 68 string / 7 numeric on ONE key, so no per-key answer exists; each caller sends one bound
    /// with one JSON type, and the guard makes the other-type rows honest empties. A numeric bound
    /// compares the 7 numeric rows; a string bound compares the 68 string rows; neither is wrong.
    ///
    /// **Numbers stored as JSON STRINGS** that need numeric comparison are out of scope: that is a
    /// convention the key should fix (store numbers as JSON numbers), and `temper-seq` (132 numeric
    /// rows) already does. A comparison operator is not a type-coercion mechanism.
    ///
    /// `probe_count` for `Compare` is **1** — one bound, one comparison per row that carries the
    /// key — like `HasKey`, not like `Contains { values }` whose cost is `Σ|values|`. `Between` is
    /// NOT added: a closed range composes from `gte` AND `lte` via the existing AND-across-the-list,
    /// and adding it saves one probe at the cost of a second value slot and a second SQL branch.
    ///
    /// **Size.** The bound is at most 16384 bytes of compact JSON and at most 256 nested array
    /// elements or object members, and counts toward the same 1048576-byte composition total as
    /// `contains` values.
    Compare {
        direction: OrdOp,
        value: serde_json::Value,
    },
}

/// The ordering direction for [`PropertyOp::Compare`]. A closed sub-enum, the same shape as
/// `Contains`'s `Vec` — a nested closed set inside one `PropertyOp` discriminant.
///
/// All four directions are needed: inclusivity matters for dates (*"on or after 2026-07-01"* is
/// `gte`, not `gt*). `Gt`/`Lt` are the half-open bounds; `Gte`/`Lte` are the closed ones; a
/// `Between` is `gte` AND `lte` composed through the existing AND-across-the-list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
#[serde(rename_all = "snake_case")]
pub enum OrdOp {
    Gt,
    Gte,
    Lt,
    Lte,
}

/// A property predicate: which key, and how. **The subject is the CONTAINER it sits in** — an
/// [`EdgeFilter`] means the edge's own `kb_properties` rows, a [`ResourceFilter`] means the
/// resource's own, and nothing else has to be said.
///
/// `[2026-08-15]` Both containers now exist, so the subject-tagged variant that floated free on the
/// invocation is **deleted**, along with the `PropertySubject` tag it carried and the
/// `UnknownFilterValue` refusal that tag's open arm existed to raise. What survives is
/// [`super::ActInvocation::properties`], retyped to this struct: it is a **tombstone**, refusing
/// with a redirect rather than being removed, because `ActInvocation` carries `deny_unknown_fields`
/// and removing the field would route a stale caller into a deserializer 400 outside the
/// `ErrorBody` shape — a worse answer than the one being replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
pub struct PropertyPredicate {
    #[cfg_attr(feature = "web-api", schema(max_length = 256))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 256)))]
    pub key: String,
    pub op: PropertyOp,
}

/// The most values one narrowing LIST may carry — `doc_type`, `tags`, `labels`.
///
/// # It bounds BYTES, not work, and that is why it is tighter than its neighbours
///
/// `[added — 2026-08-28, found in review]` `MAX_PER_CANDIDATE_PREDICATES` and
/// `MAX_PER_CANDIDATE_PROBES` bound a per-candidate multiplier, and `ResourceFilter::facets`'
/// own doc records why these three fields are not in that family: *"`tags` and `doc_type` do NOT
/// have this shape — array containment and `= ANY` are single operations whatever their length."*
/// That is still true, and it is exactly what left them unbounded: costing nothing per candidate,
/// they had no reason to be capped, and so a caller could declare as many as they liked.
///
/// What they do cost is BODY. Ten thousand one-character labels on each of `MAX_STAGES` stages is
/// a composition `validate` accepted and which serialized to **4.7 MB**
/// `[measured — 2026-08-28]` — past the door's declared body limit, so a plan the contract called
/// legal met a bare 413 with no refusal list. This is the cap that makes that a typed refusal.
///
/// # 256, measured against the live vocabularies rather than guessed
///
/// `[raised from 64 — 2026-08-28, measured on prod `crimson-fog-23541670`]` 64 was chosen by the
/// "far above the question" method its siblings use, and for one of these three fields that was
/// simply **false**:
///
/// | field | live vocabulary | semantics | is 64 above it? |
/// |---|---|---|---|
/// | `doc_type` | 19 distinct values | `= ANY`, an OR | yes, 3.4x |
/// | `tags` | 997 distinct elements | AND-containment | yes — a long list narrows to nothing |
/// | `labels` | **124 distinct** | `= ANY`, an OR | **no — 64 is half the vocabulary** |
///
/// A caller naming more than half the label vocabulary is doing something ordinary, and 4,362
/// resources is a small corpus: these vocabularies GROW with the material, so the field where the
/// cap was already too tight is the one where the margin erodes fastest.
///
/// 256 is chosen against what this actually bounds — **body bytes, not work** — together with
/// [`MAX_FILTER_STRING_BYTES`], which bounds each string: at `MAX_STAGES` stages of 256 labels at
/// that cap the labels are ~4.2 MB, inside the door's declared limit, and the coherence test
/// measures the whole plan at every cap. 256 is twice today's label vocabulary with room for a
/// corpus an order of magnitude larger.
/// It matches `MAX_PER_CANDIDATE_PROBES`' number by arithmetic coincidence rather than by
/// analogy — that one bounds a per-candidate multiplier and this one bounds a serialization.
pub const MAX_FILTER_VALUES: usize = 256;

/// The longest narrowing string — a label, tag, `doc_type`, facet key, property key, `stage`,
/// `status` or `owner` — in JSON-escaped bytes ([`json_string_bytes`]). Refused as
/// [`super::disposition::RefusalReason::FilterStringTooLong`].
///
/// The count caps bound how many of these a stage carries and never how long each is, so before
/// this a plan inside every count cap could exceed the query door's body limit through string
/// length alone (64 stages of 256 two-kilobyte labels is ~33 MB) and meet a bare 413.
///
/// # 256, measured against live data
///
/// The longest of each `[measured — 2026-10-06]`, community / enterprise: edge label 61 / 45 bytes,
/// tag 94 / 74, property key 29 / 31, facet key 39 / 24, stage or status value 32 / 33, profile
/// handle 38 / 33. 256 is 2.7x the longest.
///
/// # Published as characters, enforced as escaped bytes
///
/// Each is published as `max_length`, which JSON Schema counts in characters. A character is never
/// more than its escaped bytes, so a client that checks the schema never refuses a string the
/// server would accept. The converse does not hold: a string of multi-byte or control characters
/// can be schema-valid and still refused here, with this typed reason. That is the same trade
/// `Intention::query`'s published bound makes. Escaped bytes are the measure because they are what
/// the body limit sees: counting decoded bytes, a plan of control-character labels inside every cap
/// serialized to ~53 MB.
pub const MAX_FILTER_STRING_BYTES: usize = 256;

/// The longest `title_contains`, in JSON-escaped bytes. Refused as
/// [`super::disposition::RefusalReason::FilterStringTooLong`]. A substring probe never usefully
/// exceeds the longest title `[measured — 2026-10-06]`: 279 bytes on community production, 2,316 on
/// the enterprise install (p99.9 578). 4096 is 1.8x the longer.
pub const MAX_TITLE_CONTAINS_BYTES: usize = 4096;

/// The largest single property-predicate value — one `contains` value or `compare` bound in
/// compact serialized JSON bytes, or one facet value in JSON-escaped bytes. Refused as
/// [`super::disposition::RefusalReason::PropertyValueTooLarge`].
///
/// # What it bounds, and why the count caps did not
///
/// `MAX_PER_CANDIDATE_PROBES` charges one probe per value whatever its size, so it bounds how
/// MANY values a container carries and says nothing about how LARGE each one is. Each value is
/// bound into the predicate SQL as jsonb and compared against every candidate row that carries
/// the key, so its size multiplies that work and the request's memory.
///
/// # 16 KiB, measured against live data on both installs
///
/// A `contains` probe matches only a stored value that contains it, so no useful probe is larger
/// than the largest stored value `[measured — 2026-10-06]`:
///
/// | install | values | largest | p99.9 | largest array element |
/// |---|---|---|---|---|
/// | community | 25,421 | 1,528 bytes (an object) | ~1.4 KB | 164 bytes |
/// | enterprise | 111,433 | 8,460 bytes (a `claims` array) | 4,935 | 1,020 bytes |
///
/// 16 KiB is 1.9x the largest, so a probe naming any live value whole is admitted.
///
/// JSON Schema has no keyword for the serialized size of an arbitrary value, so this bound is
/// published in the field's documentation rather than as a schema constraint.
pub const MAX_PROPERTY_VALUE_BYTES: usize = 16384;

/// The most nested nodes — array elements and object members, at every depth — one property
/// predicate value may carry. Refused, like an over-long value, as
/// [`super::disposition::RefusalReason::PropertyValueTooLarge`].
///
/// # Why a count as well as bytes
///
/// `stored @> probe` walks the stored value once per node of the probe, so a comparison costs the
/// product of the two sizes, not their bytes. Against a stored 1M-element array, per comparison
/// `[measured on local Postgres — 2026-10-06]`: a ~2,000-node probe (what 4 KiB of small elements
/// holds) took 11.6 s, 256 nodes 1.49 s, 64 nodes 392 ms. This bounds the factor the probe
/// controls. It does not bound the comparison: the stored side is any value a caller can write. The
/// execution bound is the deployment's (`docs/concepts/query-cost-and-bounds.md`).
///
/// # 256, measured against live data on both installs
///
/// A probe matches only a stored value that contains it, so no useful probe has more nodes than the
/// largest stored value `[measured — 2026-10-06]`: 20 on community (p99.9 10), 144 on the
/// enterprise install (p99.9 72; 208 values over 64). 256 is 1.8x the largest.
pub const MAX_PROPERTY_VALUE_NODES: usize = 256;

/// The nested nodes of one property-predicate value, as [`MAX_PROPERTY_VALUE_NODES`] counts them:
/// every array element and object member at every depth. A scalar has none.
pub fn property_value_nodes(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Array(xs) => {
            xs.len() + xs.iter().map(property_value_nodes).sum::<usize>()
        }
        serde_json::Value::Object(m) => {
            m.len() + m.values().map(property_value_nodes).sum::<usize>()
        }
        _ => 0,
    }
}

/// The summed size of every property-predicate value in one composition, in the same measure as
/// [`MAX_PROPERTY_VALUE_BYTES`]. Refused as
/// [`super::disposition::RefusalReason::PropertyValueBudgetExceeded`].
///
/// The per-value cap alone does not keep the contract coherent. The count caps admit up to
/// 16,384 values across `MAX_STAGES` stages, and 16,384 values at 16 KiB is ~268 MB, past the
/// query door's 25 MB body limit. A plan the contract called legal would then meet a bare 413
/// rather than a refusal list. This budget keeps the largest legal composition well inside that
/// limit (`the_largest_legal_composition_fits_inside_the_declared_body_limit` holds it). It is
/// also the ceiling on predicate bytes one request can make Postgres compare: 1 MiB, against the
/// ~25 MB the body limit alone would admit.
pub const MAX_COMPOSITION_PROPERTY_VALUE_BYTES: usize = 1024 * 1024;

/// The size of one property-predicate value, as [`MAX_PROPERTY_VALUE_BYTES`] measures it: compact
/// serialized JSON bytes. Counted through a sink, so measuring a large value allocates nothing.
pub fn property_value_bytes(value: &serde_json::Value) -> usize {
    json_bytes(value)
}

/// The caller text one composition may carry — every narrowing string, facet value, predicate
/// value and question — counted at [`worst_case_string_bytes`]. Refused as
/// [`super::disposition::RefusalReason::TextBudgetExceeded`].
///
/// # Why a budget at the worst encoder, not the per-string caps alone
///
/// The per-string and per-value caps count the minimal JSON encoding, so a 256-byte tag can still
/// hold 128 accented letters. But clients do not all encode minimally: Python's `json.dumps`
/// escapes every non-ASCII character as `\uXXXX` by default (up to 3x), and Go's `encoding/json`
/// escapes `<`, `>` and `&` the same way (6x), Gson also `=` and `'`, and .NET's
/// `System.Text.Json` also `+`, `` ` `` and `"`. Against the per-string caps alone, a legal plan of
/// accented tags sent through our own Python SDK was ~31 MB, past the body limit. The count here
/// charges every character an encoder could escape at its full `\uXXXX` width, so a plan inside
/// this budget fits the body limit under any encoder that escapes per character; the coherence test
/// measures that worst case.
///
/// # 8 MiB
///
/// Far above any plan a person or agent writes (the longest live title on either install is 2,316
/// bytes), and with the non-text parts of the largest legal plan at their caps, still over 2x
/// under the 25 MB body limit.
pub const MAX_COMPOSITION_TEXT_BYTES: usize = 8 * 1024 * 1024;

/// A string's bytes inside its quotes under the most expansive JSON encoder that escapes per
/// character: an ASCII letter, digit or space is one byte, because no encoder escapes those; every
/// other character could be written as a six-byte `\uXXXX` (serde: controls; Python's
/// `ensure_ascii`: all non-ASCII; Go: `<>&`; Gson: also `=` and `'`; .NET: also `+`, `` ` `` and
/// `"`), and a character outside the Basic Multilingual Plane as a twelve-byte surrogate pair.
pub fn worst_case_string_bytes(s: &str) -> usize {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | ' ' => 1,
            c if (c as u32) > 0xFFFF => 12,
            _ => 6,
        })
        .sum()
}

/// A JSON value's serialized bytes under the same most expansive encoder: every string (object
/// keys included) at [`worst_case_string_bytes`] plus its quotes, and every number at the widest
/// a client could have written it.
///
/// **Numbers are not encoder-invariant.** An integer past 64 bits is parsed to an `f64` and
/// serde re-writes it short (`1e300`), while Python's `json.dumps(10**300)` sent all 301 digits; a
/// float may come back with a longer exponent or more digits than serde writes. So an `f64` that is
/// integral and beyond 2^63 counts as `MAX_F64_DIGITS` (310) bytes, and any other `f64` as at least
/// `MAX_F64_REPR_BYTES` (24). Integers within 64 bits print the same digits everywhere.
pub fn worst_case_value_bytes(value: &serde_json::Value) -> usize {
    use serde_json::Value;
    let separators = |n: usize| n.saturating_sub(1);
    match value {
        Value::String(s) => worst_case_string_bytes(s) + 2,
        Value::Array(xs) => {
            2 + separators(xs.len()) + xs.iter().map(worst_case_value_bytes).sum::<usize>()
        }
        Value::Object(m) => {
            2 + separators(m.len())
                + m.iter()
                    .map(|(k, v)| worst_case_string_bytes(k) + 3 + worst_case_value_bytes(v))
                    .sum::<usize>()
        }
        Value::Number(n) if n.is_f64() => {
            let x = n.as_f64().unwrap_or(0.0);
            if x.fract() == 0.0 && x.abs() >= 9.223_372_036_854_776e18 {
                MAX_F64_DIGITS
            } else {
                json_bytes(value).max(MAX_F64_REPR_BYTES)
            }
        }
        other => json_bytes(other),
    }
}

/// The most decimal characters an integral `f64` can need: 309 digits for `f64::MAX`, and a sign.
const MAX_F64_DIGITS: usize = 310;

/// The most characters a shortest round-trip `f64` takes in any common encoder: 17 significant
/// digits, a sign, a point, and a signed three-digit exponent (`-1.2345678901234567e-308`).
const MAX_F64_REPR_BYTES: usize = 24;

/// The bytes a string occupies inside its JSON quotes on the wire: escapes counted, so a control
/// character costs its six-byte `\u00XX`. Every narrowing string and facet value is measured this
/// way, because the body limit sees the escaped form; counting decoded bytes let a plan inside
/// every cap serialize to several times its counted size.
pub fn json_string_bytes(s: &str) -> usize {
    // The serializer writes the opening and closing quote; they are not the string's.
    json_bytes(s) - 2
}

fn json_bytes<T: Serialize + ?Sized>(value: &T) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    // Writing to an infallible sink cannot fail for a string or a `Value`: every map key is a
    // string.
    serde_json::to_writer(&mut count, value).expect("a string or Value always serializes");
    count.0
}

impl PropertyOp {
    /// The caller-supplied values this operator binds into SQL: every `contains` value, the one
    /// `compare` bound, and none for `has_key`.
    pub fn values(&self) -> &[serde_json::Value] {
        match self {
            PropertyOp::HasKey => &[],
            PropertyOp::Contains { values } => values,
            PropertyOp::Compare { value, .. } => std::slice::from_ref(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worst_case_measure_charges_every_character_some_encoder_escapes() {
        // One byte only where no encoder escapes; six for anything Gson, .NET, Go or Python's
        // ensure_ascii may write as \uXXXX; twelve for a surrogate pair.
        assert_eq!(worst_case_string_bytes("aZ9 "), 4);
        for c in [
            "=", "'", "+", "`", "\"", "\\", "/", "<", "&", "é", "\u{1}", "\u{2028}",
        ] {
            assert_eq!(worst_case_string_bytes(c), 6, "{c:?}");
        }
        assert_eq!(worst_case_string_bytes("😀"), 12);
    }

    #[test]
    fn a_number_counts_at_the_widest_a_client_could_have_written_it() {
        // Python sends 10**300 as 301 digits; serde parses it to an f64 and would re-write `1e300`.
        let big: serde_json::Value =
            serde_json::from_str(&format!("1{}", "0".repeat(300))).unwrap();
        assert!(
            worst_case_value_bytes(&big) >= 301,
            "{}",
            worst_case_value_bytes(&big)
        );
        // A float is charged its longest shortest-round-trip form.
        assert_eq!(
            worst_case_value_bytes(&serde_json::json!(0.5)),
            MAX_F64_REPR_BYTES
        );
        // An integer within 64 bits prints the same digits everywhere.
        assert_eq!(worst_case_value_bytes(&serde_json::json!(12345)), 5);
    }

    #[test]
    fn edge_kind_is_closed_at_the_four_the_ddl_declares() {
        // migrations/20260624000001_canonical_schema.sql:95
        //   CREATE TYPE edge_kind AS ENUM ('express', 'contains', 'leads_to', 'near');
        //
        // Regression cover over the INCUMBENT `types::graph::EdgeKind`, which this contract
        // re-uses rather than restates. These assertions are what make the re-use safe: they fail
        // if the shared type ever stops having the properties the contract depends on.
        for (k, j) in [
            (EdgeKind::Express, "\"express\""),
            (EdgeKind::Contains, "\"contains\""),
            (EdgeKind::LeadsTo, "\"leads_to\""),
            (EdgeKind::Near, "\"near\""),
        ] {
            assert_eq!(serde_json::to_string(&k).unwrap(), j);
        }
    }

    #[test]
    fn a_label_cannot_be_passed_as_an_edge_kind() {
        // THE audit's #1 finding, fixed at the type level. `advances` is a real LABEL that appears
        // on real edges; it is not an edge_kind. Today `--edge-type advances` silently narrows to
        // nothing with reason: ok. Here it cannot be constructed at all.
        assert!(serde_json::from_str::<EdgeKind>("\"advances\"").is_err());
        assert!(serde_json::from_str::<EdgeKind>("\"derived_from\"").is_err());
    }

    #[test]
    fn labels_and_edge_kinds_are_separate_fields_on_the_filter() {
        // Separate slots, different types — so the caller who means "advances" has exactly one
        // place to put it, and it is the right one.
        let f = EdgeFilter {
            edge_kinds: vec![EdgeKind::LeadsTo],
            labels: vec!["advances".to_string()],
            properties: vec![],
        };
        let back: EdgeFilter = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
        assert_eq!(back.edge_kinds, vec![EdgeKind::LeadsTo]);
        assert_eq!(back.labels, vec!["advances".to_string()]);
    }

    #[test]
    fn an_empty_filter_serializes_to_nothing() {
        let f = ResourceFilter::default();
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "{}", "an unset filter must not emit empty arrays");
        assert_eq!(serde_json::from_str::<ResourceFilter>("{}").unwrap(), f);
    }

    #[test]
    fn resource_filters_compose_and_round_trip() {
        // filters-compose-to-narrow: several predicates on one request, AND semantics.
        let f = ResourceFilter {
            doc_type: vec!["task".to_string()],
            tags: vec!["search".to_string(), "ci".to_string()],
            facets: vec![FacetPredicate {
                key: "domain".to_string(),
                value: "search".to_string(),
            }],
            stage: Some("in-progress".to_string()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::from_str::<ResourceFilter>(&serde_json::to_string(&f).unwrap()).unwrap(),
            f
        );
    }

    #[test]
    fn filter_fields_name_the_two_slots_an_act_may_admit() {
        assert_eq!(
            serde_json::to_string(&FilterField::Resource).unwrap(),
            "\"resource\""
        );
        assert_eq!(
            serde_json::to_string(&FilterField::Edge).unwrap(),
            "\"edge\""
        );
    }

    #[test]
    fn a_resource_property_predicate_names_no_subject_because_its_container_is_one() {
        // The whole argument for the container, at the type level, on the half that closed it. The
        // subject enum is DELETED — there is nowhere to put a tag, so a resource predicate cannot
        // claim to be about an edge, and `PropertySubject::Other` has no arm left to be unknown in.
        let f = ResourceFilter {
            properties: vec![PropertyPredicate {
                key: "derived_from".to_string(),
                op: PropertyOp::Contains {
                    values: vec![serde_json::json!("spec-a")],
                },
            }],
            ..Default::default()
        };
        let back: ResourceFilter =
            serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
        // The open-key slot is a THIRD thing beside the three named keys, not a spelling of one.
        assert_eq!(back.properties.len(), 1);
        assert!(back.tags.is_empty() && back.doc_type.is_empty() && back.facets.is_empty());
    }

    #[test]
    fn a_resource_filter_with_no_properties_still_serializes_to_nothing() {
        // `skip_serializing_if`, so adding the open-key slot did not start emitting an empty array
        // on every resource filter that has none.
        let f = ResourceFilter::default();
        assert_eq!(serde_json::to_string(&f).unwrap(), "{}");
        assert_eq!(serde_json::from_str::<ResourceFilter>("{}").unwrap(), f);
    }

    #[test]
    fn both_containers_carry_the_same_predicate_type_so_contains_cannot_diverge() {
        // `[2026-08-15]` The point of the ruling, asserted at the type level rather than described:
        // one `PropertyPredicate` in both containers means `contains` serializes identically for
        // both, and both fragments read `property_value @> v` — the value WHOLE. If the resource
        // half had taken the element grain, these two would still typecheck and would MEAN
        // different things, which is the divergence the container design exists to remove.
        let pred = PropertyPredicate {
            key: "derived_from".to_string(),
            op: PropertyOp::Contains {
                values: vec![serde_json::json!("spec-a")],
            },
        };
        let in_resource = ResourceFilter {
            properties: vec![pred.clone()],
            ..Default::default()
        };
        let in_edge = EdgeFilter {
            properties: vec![pred],
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&in_resource.properties).unwrap(),
            serde_json::to_value(&in_edge.properties).unwrap(),
            "one predicate type, so the wire shape cannot drift between the two containers"
        );
    }

    #[test]
    fn an_edge_property_predicate_names_no_subject_because_its_container_is_one() {
        // The whole argument for the container, at the type level: there is nowhere to put a
        // subject tag, so an edge predicate cannot claim to be about a resource.
        let f = EdgeFilter {
            edge_kinds: vec![EdgeKind::LeadsTo],
            labels: vec![],
            properties: vec![PropertyPredicate {
                key: "confidence".to_string(),
                op: PropertyOp::Contains {
                    values: vec![serde_json::json!("high")],
                },
            }],
        };
        let back: EdgeFilter = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
        // And the three axes stay separable — a property predicate is not a label by another name.
        assert_eq!(back.properties.len(), 1);
        assert!(back.labels.is_empty());
    }

    #[test]
    fn an_edge_filter_with_no_properties_still_serializes_to_nothing() {
        // `skip_serializing_if`, so adding a third axis did not start emitting an empty array on
        // every edge filter that has none — the property `an_empty_filter_serializes_to_nothing`
        // asserts for `ResourceFilter`, now owed by this type too.
        let f = EdgeFilter::default();
        assert_eq!(serde_json::to_string(&f).unwrap(), "{}");
        assert_eq!(serde_json::from_str::<EdgeFilter>("{}").unwrap(), f);
    }

    #[test]
    fn has_key_contains_and_compare_are_the_whole_vocabulary() {
        // No operator takes a fragment of a query language. All three bind.
        //
        // `[2026-08-16]` Renamed from `has_key_and_contains_are_the_whole_v1_vocabulary` when
        // `Compare` joined the closed set — the "v1" framing is gone with the second addition.
        let hk = PropertyPredicate {
            key: "keywords".to_string(),
            op: PropertyOp::HasKey,
        };
        let ct = PropertyPredicate {
            key: "confidence".to_string(),
            op: PropertyOp::Contains {
                values: vec![serde_json::json!("high")],
            },
        };
        let cmp = PropertyPredicate {
            key: "date".to_string(),
            op: PropertyOp::Compare {
                direction: OrdOp::Gte,
                value: serde_json::json!("2026-07-01"),
            },
        };
        for p in [hk, ct, cmp] {
            assert_eq!(
                serde_json::from_str::<PropertyPredicate>(&serde_json::to_string(&p).unwrap())
                    .unwrap(),
                p
            );
        }
    }

    #[test]
    fn compare_serializes_internally_tagged_and_round_trips_all_four_directions() {
        // The wire shape the SQL fragment parses: `{"op":"compare","direction":"gte","value":...}`.
        // `OrdOp` is `rename_all = "snake_case"`, so `Gte` → `"gte"` (no ambiguity with `Gt`).
        for (direction, wire) in [
            (OrdOp::Gt, "gt"),
            (OrdOp::Gte, "gte"),
            (OrdOp::Lt, "lt"),
            (OrdOp::Lte, "lte"),
        ] {
            let p = PropertyPredicate {
                key: "date".to_string(),
                op: PropertyOp::Compare {
                    direction,
                    value: serde_json::json!("2026-07-01"),
                },
            };
            let serialized = serde_json::to_string(&p).unwrap();
            assert_eq!(
                serialized,
                format!(
                    r#"{{"key":"date","op":{{"op":"compare","direction":"{wire}","value":"2026-07-01"}}}}"#,
                ),
                "direction {direction:?} did not serialize to the expected wire shape"
            );
            assert_eq!(
                serde_json::from_str::<PropertyPredicate>(&serialized).unwrap(),
                p,
                "round-trip failed for direction {direction:?}"
            );
        }
    }

    #[test]
    fn a_stale_body_still_carrying_a_subject_parses_so_the_redirect_can_fire() {
        // **The tombstone's whole reason, asserted rather than assumed.** `ActInvocation` carries
        // `deny_unknown_fields` and serde short-circuits before `validate`, so deleting the field
        // would answer a stale caller with a deserializer 400 OUTSIDE `ErrorBody`. Retyping it to
        // `PropertyPredicate` — which carries no `deny_unknown_fields` — keeps the old body
        // parsing: the now-meaningless `subject` is ignored and the capability pass's redirect
        // still reaches the caller.
        let wire = r#"{"subject":"edge","key":"confidence","op":{"op":"has_key"}}"#;
        let p: PropertyPredicate =
            serde_json::from_str(wire).expect("a stale subject tag must not break the parse");
        assert_eq!(p.key, "confidence");
        assert_eq!(p.op, PropertyOp::HasKey);
    }

    #[test]
    fn contains_carries_a_list_so_one_predicate_spans_several_values() {
        // `[re-argued — 2026-08-14]` This was named
        // `..._spans_a_type_unstable_key` and rested on *"containment does not coerce, so a
        // single-shape predicate silently answers for one population and not the other."* Measured
        // against Postgres 18, that is backwards — `'["x"]' @> '"x"'` is TRUE, so the SCALAR probe
        // alone already spans both populations of a type-unstable key. See `PropertyOp::Contains`.
        //
        // The list survives because its real job is the one the old rationale never mentioned:
        // OR across genuinely DIFFERENT values, matching the within-field OR that `doc_type` and
        // `EdgeFilter.labels` already have. `derived_from` (an array on 112 resources, a string on
        // 21) is still the fixture, because it is the case that would have gone wrong under the
        // old reading — a caller listing only the array shape answers for 112 and silently misses
        // 21.
        let p = PropertyPredicate {
            key: "derived_from".to_string(),
            op: PropertyOp::Contains {
                values: vec![serde_json::json!("abc"), serde_json::json!(["abc"])],
            },
        };
        let PropertyOp::Contains { values } = &p.op else {
            panic!("wrong op")
        };
        assert_eq!(values.len(), 2);
    }
}
