//! General API types — health, events, search, profile updates.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::types::vault_config::VaultConfig;

/// Response body for the health endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    /// The git commit this binary was built from, when the build knew it.
    ///
    /// `None` means *this build did not record one* — a local `cargo build`, a
    /// `cargo install`, any build outside a Vercel deploy. It never means "unknown
    /// commit": absence is reported as absence rather than as a placeholder, so a
    /// reader can never mistake "we cannot tell" for "we checked".
    pub commit: Option<&'static str>,
}

/// Response body for the event-cursor endpoint: the most recent event id
/// recorded for a context, or `None` if the context has no events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct EventCursorResponse {
    /// Most recent `kb_events.id` for the context, newest by `created`.
    pub latest_event_id: Option<Uuid>,
}

/// Default search config for full-text search.
fn default_search_config() -> String {
    "english".to_string()
}

/// Request body for POST /api/search.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct SearchParams {
    /// Pre-computed 768-dim embedding vector.
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
    /// Plain-text query for full-text search.
    #[serde(default)]
    pub query: Option<String>,
    /// Postgres text-search configuration (default "english").
    ///
    /// NOTE: reserved/inert in Surface A — FTS is hardcoded `'english'` in `search_fts_candidates`
    /// (Beat 1 kept multilingual storage-only); this param does not affect results yet.
    #[serde(default = "default_search_config")]
    pub search_config: String,
    /// Filter by context **ref** (UUID or decorated @owner/slug), resolved server-side.
    pub context_ref: Option<String>,
    /// Filter by document type.
    pub doc_type: Option<String>,
    /// Maximum results (default 10, max 50).
    pub limit: Option<i64>,
    /// Offset for pagination.
    #[serde(default)]
    pub offset: Option<i64>,
    /// Single-map scope (Surface B). Resolved client-side (cogmap refs are trailing-UUID-only).
    /// Mutually exclusive with `context_ref`. When set, the corpus is the map's homed
    /// participants the principal can see.
    ///
    /// Retained for back-compat beside the plural `cogmap_ids`: an older client (temper-rb, a
    /// pre-multi-map CLI) still sends this scalar. When `cogmap_ids` is non-empty it wins; otherwise
    /// a set `cogmap_id` is treated as a one-element set.
    #[serde(default)]
    pub cogmap_id: Option<Uuid>,
    /// Multi-map scope: the corpus is the UNION of each map's homed, visible participants. Additive
    /// beside `cogmap_id` (the sink `unified_search.p_scope_ids` was always a `uuid[]`); an older
    /// server ignores it and falls back to `cogmap_id`. Mutually exclusive with `context_ref`.
    #[serde(default)]
    pub cogmap_ids: Option<Vec<Uuid>>,
}

impl Default for SearchParams {
    fn default() -> Self {
        Self {
            embedding: None,
            query: None,
            search_config: default_search_config(),
            context_ref: None,
            doc_type: None,
            limit: None,
            offset: None,
            cogmap_id: None,
            cogmap_ids: None,
        }
    }
}

/// A single search result.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct SearchResultRow {
    pub resource_id: Uuid,
    pub title: String,
    /// Canonical kb:// URI: kb://context/doc_type/uuid (from kb_resource_uri SQL function)
    pub kb_uri: String,
    /// Original source URL or file reference
    pub origin_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub doc_type: String,
    pub score: f32,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_path: Option<String>,
}

/// Which scope selector produced the search corpus — the `{context_ref, cogmap_id}` pair in
/// [`SearchParams`], plus `Global` for the unrestricted default. Lets an agent branch on *why* a
/// result set is shaped as it is.
///
/// The `Wayfind` variant is gone with the concept. Note for anyone adding one: the temper-rb gem
/// **`raise`s on an enum value it does not know** (`search_scope.rb:39`), so a new variant is a
/// hard-fail break for an older client.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SearchScope {
    /// No scope selector — the whole corpus visible to the principal.
    Global,
    /// `context_ref` was set.
    Context,
    /// `cogmap_id` was set (single-map scope).
    Cogmap,
}

/// Why a search result set is shaped as it is — the load-bearing signal for agents, which
/// otherwise cannot tell "rephrase the query" from "this tool can never see that content"
/// (issue #360). `Ok` ⇒ at least one hit. `NoMatch` ⇒ a non-empty scope with zero hits
/// (rephrase / broaden). `OutOfScope` ⇒ the scope selector resolved to zero candidates —
/// structurally unreachable content (e.g. `wayfind` over content that is context-homed in no
/// cogmap); a different query phrasing will never help.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SearchReason {
    /// At least one result was returned.
    Ok,
    /// Scope was non-empty (or global) but nothing matched the query.
    NoMatch,
    /// The scope selector resolved to zero candidate resources.
    OutOfScope,
}

// The identity fields below are repeated inline on `ExactHit` and `WideHit` rather than shared
// through `#[serde(flatten)]`, because **ts-rs cannot codegen a flattened field** (see
// `ResourceDetail` in temper-workflow, which drops its `TS` derive for exactly this) and these types
// are ts-rs-exported. The alternative — a nested `identity` object — would add a level of nesting on
// the wire to save one in the source.
//
// `slug` is deliberately absent from both. The enrichment loop wrote `String::new()` into it on
// every path, so it has never carried a value.

/// One hit from the **exact** arm: you could quote the words.
///
/// Ordered by `fts_norm` and carrying no other quantity. A second number here would be something to
/// rank it against, which is the category error this whole shape exists to make unrepresentable.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ExactHit {
    pub resource_id: Uuid,
    pub title: String,
    /// Canonical kb:// URI.
    pub kb_uri: String,
    /// Original source URL or file reference.
    pub origin_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub doc_type: String,
    /// Slug of the home context (the natural-key half of `@owner/slug`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_slug: Option<String>,
    /// Already-sigil'd owner of the home context (`@<handle>` or `+<team-slug>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_owner_ref: Option<String>,
    /// `ts_rank` flag 33 — `rank/(rank+1)` normalization plus log-length division — in `[0,1)`.
    pub fts_norm: f32,
}

/// One hit from the **wide** arm: you had the idea, not the words.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct WideHit {
    pub resource_id: Uuid,
    pub title: String,
    pub kb_uri: String,
    pub origin_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub doc_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_owner_ref: Option<String>,
    /// The pgvector cosine DISTANCE (span `[0,2]`) rescaled as `1 - d/2`, landing in `[0,1]`.
    ///
    /// **Not the same quantity as `wayfind_region_scores.query_cos`**, which rescales the identical
    /// operator as `1 - d` and therefore spans `[-1,1]`. Two rescales of one distance; neither
    /// column name discloses which it is.
    pub vec_norm: f32,
}

/// The **exact** arm and its own disposition.
///
/// The `reason` is per-arm because one shared reason has to describe both arms at once, and would
/// report `NoMatch` for a response whose other arm returned hits — a rollup that reads as an answer
/// about the question asked when it is not.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ExactArm {
    pub hits: Vec<ExactHit>,
    pub reason: SearchReason,
    /// One-liner explaining a non-`Ok` reason and suggesting a next step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// The **wide** arm, its disposition, and whether its signal was available at all.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct WideArm {
    pub hits: Vec<WideHit>,
    pub reason: SearchReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// True when the server had to embed the query and could not — error, panic, or budget timeout.
    ///
    /// **This belongs to this arm and not to the response.** A failed embed leaves the exact arm
    /// entirely unaffected and makes the wide arm *impossible*; reporting it at response level
    /// described a blend that no longer exists. An arm that could not run says so here rather than
    /// returning an empty list that reads like an answer.
    pub degraded: bool,
}

/// What bounded the corpus — shared by both arms, because both were asked the same question of the
/// same scope.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct SearchScopeInfo {
    /// Which selector produced the corpus.
    pub kind: SearchScope,
    /// Candidate resources the scope admitted, when cheaply knowable. `None` for `global` and
    /// `context`, whose corpus is not a bounded id-set at scope-resolution time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
}

/// The `POST /api/search` wire body: **two arms that are never combined**, plus the scope they
/// share.
///
/// There is no field anywhere in this shape that ranks one arm against the other, and no single
/// ordered list into which they could be merged. That is the point — see decision
/// `019fd25a-ef4c-7473-b72e-265a7d36dd65`.
///
/// Diagnostics live here in the body. They previously rode an additive
/// `x-temper-search-diagnostics` response header, whose stated reason was keeping the `200` contract
/// a bare `Vec<UnifiedSearchResultRow>`; this shape is an object, so that reason is gone, and the
/// per-arm dispositions belong beside the arms they describe rather than somewhere a reader of the
/// body cannot see.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct SearchResponse {
    pub exact: ExactArm,
    pub wide: WideArm,
    pub scope: SearchScopeInfo,
}

/// Request body for updating a profile.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ProfileUpdateRequest {
    pub display_name: Option<String>,
    pub preferences: Option<serde_json::Value>,
    pub vault_config: Option<VaultConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_params_deserializes_query_only() {
        let json = r#"{"query": "hello world"}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.query.as_deref(), Some("hello world"));
        assert!(params.embedding.is_none());
        assert_eq!(params.search_config, "english");
        assert!(params.context_ref.is_none());
        assert!(params.doc_type.is_none());
        assert!(params.limit.is_none());
        assert!(params.offset.is_none());
    }

    #[test]
    fn search_params_deserializes_embedding_only() {
        let json = r#"{"embedding": [0.1, 0.2, 0.3]}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert!(params.query.is_none());
        assert_eq!(params.embedding.unwrap(), vec![0.1, 0.2, 0.3]);
        assert_eq!(params.search_config, "english");
    }

    #[test]
    fn search_params_carries_no_graph_or_wayfind_knob() {
        // The graph arm and wayfind are gone, so their parameters are too. A field that deserializes
        // and then reaches nothing is the "flag that silently does nothing" this phase set out to
        // remove; `deny_unknown_fields` is deliberately NOT set, so an older client still sending
        // `graph_expand` is ignored rather than rejected.
        let json = r#"{"query": "hello", "graph_expand": false, "wayfind": true, "regions": 5}"#;
        let params: SearchParams = serde_json::from_str(json).expect("stale knobs are ignored");
        assert_eq!(params.query.as_deref(), Some("hello"));
    }

    #[test]
    fn search_params_deserializes_both() {
        let json = r#"{
            "query": "test query",
            "embedding": [1.0, 2.0],
            "search_config": "simple"
        }"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.query.as_deref(), Some("test query"));
        assert_eq!(params.embedding.unwrap(), vec![1.0, 2.0]);
        assert_eq!(params.search_config, "simple");
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn event_cursor_response_round_trips_some_and_none() {
        let some = EventCursorResponse {
            latest_event_id: Some(Uuid::nil()),
        };
        let none = EventCursorResponse {
            latest_event_id: None,
        };
        for value in [some, none] {
            let json = serde_json::to_string(&value).unwrap();
            let back: EventCursorResponse = serde_json::from_str(&json).unwrap();
            assert_eq!(back.latest_event_id, value.latest_event_id);
        }
    }
}
