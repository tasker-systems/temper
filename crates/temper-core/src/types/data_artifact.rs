//! Wire types for data artifact reads and writes — the shape API, MCP, and CLI surfaces use.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::authorship::ActInput;
use crate::types::ids::{DataArtifactId, ResourceId};
use crate::types::resource::SortOrder;

/// One data artifact, as returned by the API and MCP surfaces.
///
/// This is the readback `RetrievedArtifact` projected onto a wire shape: the typed enums
/// (`intent`, `shape_state`, `kind_owner`) are rendered as their serde-renamed strings so a
/// consumer that does not share the Rust types still gets a self-describing response.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "data_artifact.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ArtifactView {
    pub artifact_id: DataArtifactId,
    pub resource_id: ResourceId,
    /// `"kb_profiles"` or `"kb_teams"` — the namespace half of the family name.
    pub kind_owner_table: String,
    pub kind_owner_id: Uuid,
    pub artifact_kind: String,
    /// `"current"` / `"member"` / `"pinned"` — the closed selection vocabulary.
    pub intent: String,
    pub precedence: f64,
    pub content_hash: String,
    pub content_bytes: i64,
    /// Conformance to the shape in force for the artifact's family in its home, by the staleness
    /// triple (shape id, shape version, content hash): `"never_declared"` when no shape is in
    /// force, else `"declared_satisfied"`, `"declared_not_satisfied"`, or
    /// `"declared_not_yet_checked"` when no verdict matches the triple (`_data_artifact_shape_state`,
    /// `20260822000020`; a batch read's rows carry the same state, computed set-based).
    pub shape_state: String,
    pub is_folded: bool,
    pub created: DateTime<Utc>,
    /// The content payload. Absent, not `null`, when the artifact was committed with no bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
}

/// One row of per-family counts, as returned by the counts endpoint.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "data_artifact.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ArtifactCountRow {
    pub kind_owner_table: String,
    pub kind_owner_id: Uuid,
    pub artifact_kind: String,
    pub count: i64,
    pub total_bytes: i64,
}

/// Query parameters for listing artifacts on a resource.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct ArtifactListParams {
    /// Filter by the bare family name (e.g. `"measurement"`).
    #[serde(default)]
    pub kind: Option<String>,
    /// Filter by selection intent: `"current"`, `"member"`, or `"pinned"`.
    #[serde(default)]
    pub intent: Option<String>,
    /// Include folded (superseded) artifacts in the result. Default: `false`.
    #[serde(default)]
    pub include_folded: Option<bool>,
    /// Return per-family counts instead of full artifacts. No content hydration.
    /// Default: `false` (full artifacts with content). Counts are not paged: `limit` and
    /// `after_*` are not read when this is set.
    #[serde(default)]
    pub counts: Option<bool>,
    /// Return at most this many artifacts. The body is still the bare array; when more remain, the
    /// response carries a `Link: <...>; rel="next"` header whose target is this request with the
    /// next page's position, and the header is absent on the last page. Must be at least 1 and at
    /// most the server ceiling `MAX_ARTIFACT_PAGE_ROWS`, which is 200; a value outside that range
    /// is refused, never clamped. Omit it to receive the whole list (the unbounded form, which a
    /// later release retires).
    #[serde(default)]
    #[cfg_attr(feature = "web-api", param(minimum = 1))]
    pub limit: Option<u32>,
    /// Continue after this position: the `after_created` of the previous page's `Link` target.
    /// Send together with `after_id`, and only with `limit`.
    #[serde(default)]
    pub after_created: Option<DateTime<Utc>>,
    /// Continue after this position: the `after_id` of the previous page's `Link` target. Send
    /// together with `after_created`, and only with `limit`.
    #[serde(default)]
    pub after_id: Option<Uuid>,
    /// The direction of a bounded read over `(created, id)`: `asc`, oldest first, or `desc`,
    /// newest first. Absent is `asc`. Only with `limit`; the `Link` target carries it, so every
    /// page continues in the direction the first was asked in. Any other value is refused.
    #[serde(default)]
    pub order: Option<SortOrder>,
}

/// The server ceiling on one page of artifacts. A `limit` above it is refused by name
/// ([`ArtifactPageRefusal::LimitAboveCeiling`]), never clamped: a caller that asked for more
/// than it can have is told so, rather than handed a short page it might read as the end.
/// `ArtifactListParams::limit`'s doc, which is the published OpenAPI text, states this value;
/// change the two together.
pub const MAX_ARTIFACT_PAGE_ROWS: u32 = 200;

/// The page size a caller that wants every artifact asks for while draining the list.
/// Well under [`MAX_ARTIFACT_PAGE_ROWS`] because each row carries its content whole.
pub const ARTIFACT_DRAIN_PAGE_ROWS: u32 = 50;

/// The deepest content a batch-read page serves. A served row whose content nests deeper is named
/// in the page's `oversized` in its place, as a row too long for the page is, and the page goes on
/// (temper-services' `artifact_ask_read`). **Depth** counts each array or object level, with the
/// content's root at 1: `{}` and `[]` are 1 deep, `{"a":[0]}` is 2, and a string or a number is 0.
///
/// The batch read's docs state this value in the phrase "more than 96 arrays or objects deep", so
/// change them together. A test reads it in each text they are published in: `openapi.json` (from
/// `OversizedArtifact`, `AskResponse.oversized` and the route's doc), the MCP tool declarations,
/// the CLI reference (from the CLI's help), the skill template, and the MCP skill's hand-written
/// tool reference. The SDKs, the TypeScript types and the MCP skill's projection of the template
/// are generated from those, under their own drift gates.
///
/// # 96: the strictest first-party client's nesting limit, less the levels above the content
///
/// A page a client cannot parse fails for every reader of the ask through that client, and no
/// `limit` or `next` steps past it. The strictest first-party client is the generated Ruby SDK
/// (`clients/temper-rb`), which parses a body as `JSON.parse("[#{body}]")` and sets no
/// `max_nesting`, so Ruby's default of 100 levels applies, and its `[ ]` takes one of them. An ask
/// page puts three levels above each row's content: the `AskResponse` object, its `artifacts`
/// array and the `ArtifactView` object. So content may nest 100 − 1 − 3 = 96 deep. serde_json,
/// which temperkb-client and the MCP relay parse with, allows 127 levels, so a Rust client reads
/// an ask page whose content nests up to 124 deep.
pub const ARTIFACT_ASK_CONTENT_DEPTH: usize = 96;

/// A position in the `(created, id)` keyset order that `artifacts_for_resource` returns rows in.
///
/// It carries position and no authority: each page re-runs the read gate for the asker, so a
/// cursor handed to another principal, or replayed after access is revoked, reveals nothing
/// the read would not. That is why it is a legible object and not an opaque token, and needs
/// no decoder in any SDK. Every bounded artifact read continues with this one type.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "data_artifact.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
// `list_data_artifacts` takes it as `after`, and an MCP tool declaration carries no `$ref`.
#[cfg_attr(feature = "mcp", schemars(inline))]
pub struct ArtifactCursor {
    /// The `created` of the last artifact on the previous page.
    pub created: DateTime<Utc>,
    /// The `artifact_id` of the last artifact on the previous page; breaks ties on `created`.
    pub artifact_id: DataArtifactId,
}

/// The first instant Postgres's `timestamptz` holds, 4714-11-24 00:00:00 BC in UTC, as Unix
/// seconds: Julian day 0, 2,440,588 days before the Unix epoch.
const STORABLE_FROM_UNIX_SECONDS: i64 = -210_866_803_200;

/// The first instant past what `timestamptz` holds, 294277-01-01 00:00:00 in UTC, as Unix seconds:
/// Julian day 109,203,528.
const STORABLE_UNTIL_UNIX_SECONDS: i64 = 9_224_318_016_000;

impl ArtifactCursor {
    /// Whether the store can hold this position: `created` is within Postgres's `timestamptz`
    /// range. A `DateTime<Utc>` reaches further back (to year -262,143), so a position read from a
    /// caller can name an instant no artifact was or could be created at, and binding it to the
    /// keyset fails in the database rather than answering. The batch read refuses a position this
    /// answers `false` for as a body it cannot read, at its door and again in its service read
    /// before anything is read. The high end is past the latest instant a `DateTime<Utc>` holds,
    /// so only the low end can refuse a position that was read.
    pub fn is_storable(&self) -> bool {
        (STORABLE_FROM_UNIX_SECONDS..STORABLE_UNTIL_UNIX_SECONDS)
            .contains(&self.created.timestamp())
    }
}

/// One bounded page of artifacts, in `(created, id)` order in the requested direction.
///
/// What the service layer and `temper-client` hand back once the route's two halves are put
/// together: on the route the body is the bare array and the continuation rides the `Link`
/// response header, so this is not the route's shape and has no OpenAPI or TypeScript derive.
/// It is the shape the CLI's `data-artifact list --limit` and MCP's `list_data_artifacts` with
/// `limit` answer, which have no header to carry the continuation: `{"artifacts": [...],
/// "next": {...} | null}`. `next` is always present and is an explicit `null` on the last page,
/// so a reader can tell the last page from an answer that lost its continuation. `Deserialize`
/// is for SDK consumers reading that envelope back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactPage {
    pub artifacts: Vec<ArtifactView>,
    /// Where to continue, or `None` when this page is the last. Pass it back unchanged as the
    /// next request's position, with the same filters and `order`.
    pub next: Option<ArtifactCursor>,
}

/// A bounded read's request, as [`ArtifactListParams::page_request`] validates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactPageRequest {
    /// Rows wanted, within `1..=MAX_ARTIFACT_PAGE_ROWS`.
    pub limit: u32,
    /// Continue after this position; `None` starts at the beginning.
    pub after: Option<ArtifactCursor>,
    /// `Asc` is oldest first, `Desc` newest first; the position is beyond `after` in this order.
    pub order: SortOrder,
}

/// Why a bounded list request is refused. Each message names the parameter at fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ArtifactPageRefusal {
    #[error("`limit` must be at least 1; a page of zero artifacts cannot make progress")]
    LimitZero,
    #[error(
        "`limit` {limit} is above the ceiling `MAX_ARTIFACT_PAGE_ROWS` ({max}); ask for at most \
         {max} and continue with `after_created` and `after_id`",
        max = MAX_ARTIFACT_PAGE_ROWS
    )]
    LimitAboveCeiling { limit: u32 },
    #[error("`after_created` and `after_id` name one position and must be sent together")]
    AfterIncomplete,
    #[error(
        "`after_created` and `after_id` continue a bounded read and require `limit`; without \
         `limit` the whole list is returned and there is nothing to continue"
    )]
    AfterWithoutLimit,
    #[error(
        "`order` directs a bounded read and requires `limit`; without `limit` the whole list is \
         returned oldest first"
    )]
    OrderWithoutLimit,
}

impl ArtifactListParams {
    /// The bounded request these params ask for, or `None` for the unbounded list.
    ///
    /// `limit` is the demarcation: absent, the caller gets the whole list as a bare array.
    /// Present, it is checked against `1..=MAX_ARTIFACT_PAGE_ROWS` and refused by name outside
    /// it. A continuation position that is half-sent, or sent without `limit`, is refused too:
    /// quietly ignoring it would answer the first page again. So is `order` without `limit`:
    /// ignoring it would answer oldest first to a caller who asked for newest. An absent `order`
    /// is `Asc`, never `SortOrder::default()`, which is `Desc`.
    pub fn page_request(&self) -> Result<Option<ArtifactPageRequest>, ArtifactPageRefusal> {
        let after = match (self.after_created, self.after_id) {
            (None, None) => None,
            (Some(created), Some(id)) => Some(ArtifactCursor {
                created,
                artifact_id: DataArtifactId::from(id),
            }),
            _ => return Err(ArtifactPageRefusal::AfterIncomplete),
        };
        let Some(limit) = self.limit else {
            return match (after, self.order) {
                (Some(_), _) => Err(ArtifactPageRefusal::AfterWithoutLimit),
                (None, Some(_)) => Err(ArtifactPageRefusal::OrderWithoutLimit),
                (None, None) => Ok(None),
            };
        };
        let order = self.order.unwrap_or(SortOrder::Asc);
        match limit {
            0 => Err(ArtifactPageRefusal::LimitZero),
            n if n > MAX_ARTIFACT_PAGE_ROWS => {
                Err(ArtifactPageRefusal::LimitAboveCeiling { limit: n })
            }
            n => Ok(Some(ArtifactPageRequest {
                limit: n,
                after,
                order,
            })),
        }
    }
}

/// The namespace half of the family name — mirrors `temper_substrate::payloads::KindOwner`
/// in a temper-core-native shape so the wire type does not depend on the substrate crate.
/// `None` (the ordinary case) lets the SQL wrapper default the namespace from the owning
/// resource's home.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "data_artifact.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
pub enum KindOwnerInput {
    #[serde(rename = "kb_profiles")]
    Profile(Uuid),
    #[serde(rename = "kb_teams")]
    Team(Uuid),
}

/// Request body for `POST /api/resources/{id}/artifacts` — commit one data artifact.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "data_artifact.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ArtifactCommitRequest {
    /// The bare family name, qualified by `kind_owner` (or defaulted from the resource's home).
    pub kind: String,
    /// Override the namespace half of the family name. Omit to let the server default it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_owner: Option<KindOwnerInput>,
    /// Selection intent: `"current"`, `"member"`, or `"pinned"`.
    pub intent: String,
    /// Ordering among peers. Meaningful for `member`; carried for all. Default: `0.0`.
    #[serde(default)]
    pub precedence: f64,
    /// The structured payload as JSON. Hashed and stored verbatim — the hash is the proof.
    pub content: serde_json::Value,
    /// Artifacts this one replaces, named explicitly by the writer. Empty = replaces nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<DataArtifactId>,
    /// Per-act correlation + authorship flags. Flattened into the request body.
    #[serde(flatten)]
    pub act: ActInput,
}

/// The longest family name (`kind`) a commit or a shape declaration may carry, in bytes.
///
/// # 256
///
/// A family name is an identifier, and it is a key in three btree indexes (the live-shape unique
/// index, `idx_kb_data_artifacts_kind` and `idx_kb_data_artifacts_family`), whose rows hold at most
/// 2704 bytes. Past that the database refuses the write as a fault. 256 is the cap the other
/// identifiers a caller names already carry: a shape property name
/// (`shape_schema::MAX_PROPERTY_NAME_BYTES`) and an ask path segment
/// (`MAX_ARTIFACT_PATH_SEGMENT_BYTES`). It is well inside the index row with the other key columns
/// beside it, compressed or not. Chosen, not measured against stored families.
pub const MAX_ARTIFACT_KIND_BYTES: usize = 256;

/// Whether a JSON value holds U+0000 anywhere: in a string, or in an object key, at any depth of
/// arrays and objects. `jsonb` cannot store the character, so no stored artifact or shape holds
/// it, and binding a value that does is a database error rather than a refusal. An ask refuses a
/// predicate value that holds one (`unstorable_value`); a commit or a shape declaration is refused
/// through [`unstorable_nul`]. Recursive to the value's depth, which the JSON parser that read it
/// has bounded.
pub fn holds_nul(value: &serde_json::Value) -> bool {
    use serde_json::Value;
    match value {
        Value::String(s) => s.contains('\0'),
        Value::Array(elements) => elements.iter().any(holds_nul),
        Value::Object(members) => members
            .iter()
            .any(|(key, member)| key.contains('\0') || holds_nul(member)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

/// The refusal for a write that would store U+0000: a commit whose `kind` or `content` holds it, or
/// a shape declaration whose `kind` or `schema` does. It names each field that holds one as the
/// request names it (`value_field` is `"content"` or `"schema"`), and never quotes the value.
/// `None` when neither does. The doors run it before anything is stored, so such a write is
/// refused by name, as `DATA_ARTIFACT_REFUSAL`, rather than failing in the database.
pub fn unstorable_nul(kind: &str, value_field: &str, value: &serde_json::Value) -> Option<String> {
    // A family name is neither a string value nor an object key, so each field gets its remedy.
    let in_value = || format!("every string and object key in `{value_field}`");
    let (named, remedy) = match (kind.contains('\0'), holds_nul(value)) {
        (false, false) => return None,
        (true, false) => ("`kind` holds".to_string(), "`kind`".to_string()),
        (false, true) => (format!("`{value_field}` holds"), in_value()),
        (true, true) => (
            format!("`kind` and `{value_field}` hold"),
            format!("`kind`, and from {}", in_value()),
        ),
    };
    Some(format!(
        "{named} the character U+0000 (NUL), which the database cannot store, so no artifact or \
         shape can hold it; remove it from {remedy}, and send the request again"
    ))
}

/// The refusal for a commit or a shape declaration whose family name is longer than
/// [`MAX_ARTIFACT_KIND_BYTES`]: it names `kind`, its length and the limit, and never quotes the
/// value. `None` when the name is within it. The doors run it beside [`unstorable_nul`], before
/// anything is stored, so a name no index row can hold is refused by name rather than failing in
/// the database.
pub fn overlong_kind(kind: &str) -> Option<String> {
    (kind.len() > MAX_ARTIFACT_KIND_BYTES).then(|| {
        format!(
            "`kind` is {} bytes; the limit is {MAX_ARTIFACT_KIND_BYTES}. A family name is an \
             identifier: shorten it, and send the request again",
            kind.len()
        )
    })
}

/// Response body for `POST /api/resources/{id}/artifacts` — the committed artifact's ID
/// and its full readback view.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "data_artifact.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ArtifactCommitResponse {
    pub artifact_id: DataArtifactId,
    pub artifact: ArtifactView,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor() -> ArtifactCursor {
        ArtifactCursor {
            // Sub-second precision: the keyset compares microseconds, so the wire must carry them.
            created: DateTime::parse_from_rfc3339("2026-10-19T12:30:45.123456Z")
                .expect("literal parses")
                .with_timezone(&Utc),
            artifact_id: DataArtifactId::from(Uuid::from_u128(0x0123_4567_89ab_cdef)),
        }
    }

    fn params_with(limit: Option<u32>, after: Option<ArtifactCursor>) -> ArtifactListParams {
        ArtifactListParams {
            limit,
            after_created: after.map(|c| c.created),
            after_id: after.map(|c| c.artifact_id.uuid()),
            ..Default::default()
        }
    }

    /// A cursor at `instant`, read as a caller's body is read (`FromStr`, which takes a signed
    /// year of any width).
    fn at(instant: &str) -> ArtifactCursor {
        ArtifactCursor {
            created: instant.parse::<DateTime<Utc>>().expect("literal parses"),
            ..cursor()
        }
    }

    /// **A position is storable exactly within `timestamptz`'s range**: the first instant it holds
    /// is, the instant before it is not, and the latest instant a `DateTime<Utc>` holds is. Bites:
    /// the low bound one day later (the first instant refused), or the range made inclusive at
    /// both ends with the low bound one second early (the instant before admitted).
    #[test]
    fn a_position_is_storable_within_the_stores_instants() {
        assert!(
            at("-4713-11-24T00:00:00Z").is_storable(),
            "the first instant held"
        );
        assert!(
            !at("-4713-11-23T23:59:59.999999Z").is_storable(),
            "the instant before it"
        );
        assert!(at("+262142-12-31T23:59:59.999999Z").is_storable());
        assert!(cursor().is_storable());
    }

    #[test]
    fn cursor_round_trips_through_serde_json() {
        let original = cursor();
        let wire = serde_json::to_string(&original).expect("cursor serializes");
        let back: ArtifactCursor = serde_json::from_str(&wire).expect("cursor deserializes");
        assert_eq!(back, original, "wire form was {wire}");
    }

    #[test]
    fn cursor_wire_form_is_a_legible_object_not_an_opaque_token() {
        let wire = serde_json::to_value(cursor()).expect("cursor serializes");
        assert_eq!(wire["created"], "2026-10-19T12:30:45.123456Z");
        assert!(wire["artifact_id"].is_string(), "got {wire}");
    }

    #[test]
    fn no_limit_is_the_unbounded_list() {
        assert_eq!(params_with(None, None).page_request(), Ok(None));
    }

    #[test]
    fn a_limit_within_the_ceiling_is_a_bounded_request() {
        for limit in [1, 2, MAX_ARTIFACT_PAGE_ROWS] {
            assert_eq!(
                params_with(Some(limit), Some(cursor())).page_request(),
                Ok(Some(ArtifactPageRequest {
                    limit,
                    after: Some(cursor()),
                    order: SortOrder::Asc,
                })),
            );
        }
    }

    #[test]
    fn a_limit_above_the_ceiling_is_refused_by_name_and_never_clamped() {
        let over = MAX_ARTIFACT_PAGE_ROWS + 1;
        let refusal = params_with(Some(over), None)
            .page_request()
            .expect_err("over the ceiling");
        assert_eq!(
            refusal,
            ArtifactPageRefusal::LimitAboveCeiling { limit: over }
        );
        let said = refusal.to_string();
        assert!(said.contains("MAX_ARTIFACT_PAGE_ROWS"), "got: {said}");
        assert!(said.contains(&over.to_string()), "got: {said}");
    }

    #[test]
    fn a_limit_of_zero_is_refused_by_name() {
        let refusal = params_with(Some(0), None).page_request().expect_err("zero");
        assert_eq!(refusal, ArtifactPageRefusal::LimitZero);
        assert!(refusal.to_string().contains("`limit`"));
    }

    #[test]
    fn half_a_continuation_position_is_refused() {
        let mut params = params_with(Some(2), Some(cursor()));
        params.after_id = None;
        assert_eq!(
            params.page_request(),
            Err(ArtifactPageRefusal::AfterIncomplete)
        );
        let mut params = params_with(Some(2), Some(cursor()));
        params.after_created = None;
        assert_eq!(
            params.page_request(),
            Err(ArtifactPageRefusal::AfterIncomplete)
        );
    }

    #[test]
    fn a_continuation_position_without_a_limit_is_refused_not_ignored() {
        assert_eq!(
            params_with(None, Some(cursor())).page_request(),
            Err(ArtifactPageRefusal::AfterWithoutLimit)
        );
    }

    #[test]
    fn an_absent_order_is_oldest_first_and_desc_is_carried() {
        let asked = |order| ArtifactListParams {
            order,
            ..params_with(Some(2), None)
        };
        assert_eq!(
            asked(None).page_request().map(|r| r.map(|r| r.order)),
            Ok(Some(SortOrder::Asc)),
            "absent is asc, never SortOrder's own default (desc)"
        );
        assert_eq!(
            asked(Some(SortOrder::Desc))
                .page_request()
                .map(|r| r.map(|r| r.order)),
            Ok(Some(SortOrder::Desc))
        );
    }

    #[test]
    fn an_order_without_a_limit_is_refused_not_ignored() {
        let params = ArtifactListParams {
            order: Some(SortOrder::Desc),
            ..Default::default()
        };
        let refusal = params.page_request().expect_err("order without limit");
        assert_eq!(refusal, ArtifactPageRefusal::OrderWithoutLimit);
        assert!(refusal.to_string().contains("`order`"));
    }

    /// The doors' envelope always carries `next`: an explicit `null` on the last page, the
    /// legible cursor otherwise. FAILS IF `next` is skipped when `None` (a reader could not tell
    /// the last page from an answer that lost its continuation) or renamed.
    #[test]
    fn the_page_envelope_names_next_on_every_page_and_null_on_the_last() {
        let last = serde_json::to_value(ArtifactPage {
            artifacts: Vec::new(),
            next: None,
        })
        .expect("page serializes");
        let keys: Vec<&str> = last
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["artifacts", "next"], "got {last}");
        assert!(last["next"].is_null(), "got {last}");

        let more = serde_json::to_value(ArtifactPage {
            artifacts: Vec::new(),
            next: Some(cursor()),
        })
        .expect("page serializes");
        assert_eq!(
            more["next"],
            serde_json::to_value(cursor()).expect("cursor serializes")
        );
    }

    /// **A family name is admitted at `MAX_ARTIFACT_KIND_BYTES` and refused one byte past it**,
    /// measured in bytes (a multi-byte name over the limit in bytes but not in characters is
    /// refused), naming `kind`, its length and the limit, and quoting none of it.
    /// Bite: `>` replaced by `>=` in `overlong_kind` (the name at the limit is refused).
    #[test]
    fn an_overlong_family_name_is_refused_naming_its_limit() {
        assert_eq!(overlong_kind(&"k".repeat(MAX_ARTIFACT_KIND_BYTES)), None);
        for name in [
            "k".repeat(MAX_ARTIFACT_KIND_BYTES + 1),
            "é".repeat(MAX_ARTIFACT_KIND_BYTES / 2 + 1),
        ] {
            let refusal = overlong_kind(&name).expect("one byte past the limit is refused");
            assert!(
                refusal.starts_with(&format!("`kind` is {} bytes", name.len()))
                    && refusal.contains(&format!("the limit is {MAX_ARTIFACT_KIND_BYTES}"))
                    && !refusal.contains(&name),
                "got: {refusal}"
            );
        }
    }

    /// **U+0000 is found wherever a stored value can carry it, and a write is refused by the field
    /// that holds it, never by its value**: in a string, in an array of arrays, in a nested
    /// object's key, and in a string inside an object inside an array; not in a value whose only
    /// control character is U+0001. The refusal names `content`, `kind`, or both, and quotes
    /// nothing, and a refused `kind` is told to remove it from `kind`, not from a string or a key.
    /// Bites: `key.contains('\0') ||` dropped from `holds_nul` (the nested key is missed);
    /// `kind.contains('\0')` replaced by `false` in `unstorable_nul` (the family name is admitted).
    #[test]
    fn a_nul_is_refused_by_the_field_that_holds_it() {
        use serde_json::json;
        for value in [
            json!("a\u{0}b"),
            json!([[1, ["\u{0}"]]]),
            json!({ "a": { "k\u{0}": 1 } }),
            json!([{ "b": ["x", { "c": "\u{0}" }] }]),
        ] {
            assert!(holds_nul(&value), "{value}");
        }
        let clean = json!({ "a": ["b\u{1}", 1, null, true, { "c\u{1}": {} }] });
        assert!(!holds_nul(&clean));
        assert_eq!(unstorable_nul("family", "content", &clean), None);

        let content = json!({ "secret\u{0}key": "secret value" });
        let said = unstorable_nul("family", "content", &content).expect("content refused");
        assert!(said.starts_with("`content` holds "), "{said}");
        assert!(
            !said.contains("secret"),
            "the value is never quoted: {said}"
        );
        let said = unstorable_nul("fam\u{0}ily", "content", &clean).expect("kind refused");
        assert!(said.starts_with("`kind` holds "), "{said}");
        assert!(!said.contains("fam"), "the name is never quoted: {said}");
        assert!(
            said.contains("remove it from `kind`,") && !said.contains("object key"),
            "a name is neither a string value nor an object key: {said}"
        );
        let said = unstorable_nul("fam\u{0}ily", "schema", &content).expect("both refused");
        assert!(said.starts_with("`kind` and `schema` hold "), "{said}");
    }

    /// Every number `text` states in the phrase "more than N arrays or objects deep", with every
    /// run of whitespace read as one space.
    fn depths_stated(text: &str) -> Vec<String> {
        let words: Vec<&str> = text.split_whitespace().collect();
        words
            .windows(7)
            .filter(|w| w[..2] == ["more", "than"] && w[3..6] == ["arrays", "or", "objects"])
            .filter(|w| w[6].starts_with("deep"))
            .map(|w| w[2].to_string())
            .collect()
    }

    /// Every string in a JSON document, read as one text.
    fn strings_of(document: &serde_json::Value) -> String {
        let mut text = String::new();
        let mut stack = vec![document];
        while let Some(value) = stack.pop() {
            match value {
                serde_json::Value::String(s) => {
                    text.push_str(s);
                    text.push('\n');
                }
                serde_json::Value::Array(items) => stack.extend(items),
                serde_json::Value::Object(map) => stack.extend(map.values()),
                _ => {}
            }
        }
        text
    }

    /// **Every published text that states the batch read's content depth states
    /// [`ARTIFACT_ASK_CONTENT_DEPTH`]**: the committed `openapi.json` (from `OversizedArtifact`,
    /// `AskResponse.oversized` and the route's doc), the MCP tool declarations
    /// (`crates/temper-mcp/src/declarations/tools_list.json`), the CLI reference
    /// (`docs/reference/cli/data-artifact.md`, from the CLI's help), the skill's template
    /// (`crates/temper-cli/templates/shared/data-artifacts.md`), and the MCP skill's tool reference
    /// (`skills/temper-knowledge-base/knowledge-base.md`, which no generator derives). Each states
    /// it at least once, in the phrase "more than N arrays or objects deep", and never another N,
    /// so the constant cannot move without the texts an agent decides by. Read at test time, as
    /// temper-client's registry test reads `openapi.json`, so the published crate references no
    /// file outside its package. Bites: the constant lowered to 95; one text's 96 edited to 95
    /// (`knowledge-base.md`'s, or the CLI reference's, among them).
    #[test]
    fn every_published_text_states_the_content_depth() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let read = |path: &str| {
            let path = root.join(path);
            std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        };
        let json = |path: &str| {
            let document: serde_json::Value =
                serde_json::from_str(&read(path)).unwrap_or_else(|e| panic!("{path} parses: {e}"));
            strings_of(&document)
        };
        let texts = [
            ("openapi.json", json("openapi.json")),
            (
                "tools_list.json",
                json("crates/temper-mcp/src/declarations/tools_list.json"),
            ),
            (
                "the CLI reference",
                read("docs/reference/cli/data-artifact.md"),
            ),
            (
                "the skill template",
                read("crates/temper-cli/templates/shared/data-artifacts.md"),
            ),
            (
                "the MCP skill's tool reference",
                read("skills/temper-knowledge-base/knowledge-base.md"),
            ),
        ];
        let limit = ARTIFACT_ASK_CONTENT_DEPTH.to_string();
        for (name, text) in texts {
            let stated = depths_stated(&text);
            assert!(!stated.is_empty(), "{name} states the content depth");
            assert!(
                stated.iter().all(|n| *n == limit),
                "{name} states {stated:?}; the constant is {limit}"
            );
        }
    }
}
