//! Knowledge graph types — edge types, traversal results, and relationship
//! declarations for the R7 vertex-edge graph stored in `kb_resource_edges`.

use crate::frontmatter::document::DocType;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Edge Type ──────────────────────────────────────────────────────────────

/// Edge type enum — mirrors the Postgres `edge_type` enum exactly.
///
/// All edges are directed: `source_resource_id → target_resource_id`.
/// Symmetric queries (e.g., `relates_to`) union forward + reverse scans.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "edge_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    RelatesTo,
    Extends,
    DependsOn,
    References,
    ParentOf,
    PrecededBy,
    DerivedFrom,
}

impl std::fmt::Display for EdgeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RelatesTo => write!(f, "relates_to"),
            Self::Extends => write!(f, "extends"),
            Self::DependsOn => write!(f, "depends_on"),
            Self::References => write!(f, "references"),
            Self::ParentOf => write!(f, "parent_of"),
            Self::PrecededBy => write!(f, "preceded_by"),
            Self::DerivedFrom => write!(f, "derived_from"),
        }
    }
}

// ─── Structural Edge Typing (SSTorytime four-type taxonomy) ─────────────────

/// Structural edge type — the four Semantic-Spacetime primitives. Each kind
/// carries a distinct traversal algebra:
/// - `Contains`  — transitive (composition / part-of participation)
/// - `LeadsTo`   — antisymmetric, causal/temporal order
/// - `Near`      — symmetric (proximity / similarity)
/// - `Express`   — leaf attribute (has-property)
///
/// Mirrors the Postgres `edge_kind` enum.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "edge_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Express,
    Contains,
    LeadsTo,
    Near,
}

/// Edge direction sign. `source → target` as asserted may run *with* the
/// structural arrow (`Forward`) or *against* it (`Inverse`) — e.g. a
/// `depends_on` edge is asserted source=dependant/target=dependency, but the
/// causal arrow runs dependency→dependant, so it is `Inverse` `LeadsTo`.
/// `Near` is symmetric: always `Forward`.
///
/// Mirrors the Postgres `edge_polarity` enum.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "edge_polarity", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Forward,
    Inverse,
}

impl EdgeType {
    /// Map a legacy flat `EdgeType` to its structural `(EdgeKind, Polarity,
    /// label)` triple. The label is the legacy enum's snake_case name —
    /// preserving the human relation vocabulary as free-text. Used by the
    /// schema-cutover migration and the frontmatter edge-extraction rewire.
    pub fn legacy_mapping(self) -> (EdgeKind, Polarity, &'static str) {
        match self {
            Self::ParentOf => (EdgeKind::Contains, Polarity::Forward, "parent_of"),
            Self::DependsOn => (EdgeKind::LeadsTo, Polarity::Inverse, "depends_on"),
            Self::PrecededBy => (EdgeKind::LeadsTo, Polarity::Inverse, "preceded_by"),
            Self::DerivedFrom => (EdgeKind::LeadsTo, Polarity::Inverse, "derived_from"),
            Self::Extends => (EdgeKind::LeadsTo, Polarity::Inverse, "extends"),
            Self::RelatesTo => (EdgeKind::Near, Polarity::Forward, "relates_to"),
            Self::References => (EdgeKind::Near, Polarity::Forward, "references"),
        }
    }
}

// ─── Target Reference ───────────────────────────────────────────────────────

/// A reference target in frontmatter — either a resolved UUID or a slug string.
/// Used during edge extraction before resolution against the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetRef {
    Id(Uuid),
    Slug(String),
}

impl TargetRef {
    /// Parse a frontmatter reference value into a TargetRef.
    ///
    /// - Valid UUID string → `TargetRef::Id`
    /// - Non-empty, non-URL string → `TargetRef::Slug`
    /// - Empty or URL strings → `None` (external URIs aren't graph edges)
    pub fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(uuid) = Uuid::parse_str(trimmed) {
            return Some(Self::Id(uuid));
        }
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return None;
        }
        Some(Self::Slug(trimmed.to_string()))
    }
}

// ─── Relationship Declarations ──────────────────────────────────────────────

/// Parsed relationship declarations from YAML frontmatter.
///
/// Each field maps to an edge type. Values are raw strings — either UUIDs
/// or slugs — that get resolved to `kb_resources.id` at ingest time.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceRelationships {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relates_to: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preceded_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_from: Vec<String>,
}

impl ResourceRelationships {
    /// Returns true if no relationships are declared.
    pub fn is_empty(&self) -> bool {
        self.relates_to.is_empty()
            && self.extends.is_empty()
            && self.depends_on.is_empty()
            && self.references.is_empty()
            && self.parent.is_none()
            && self.preceded_by.is_empty()
            && self.derived_from.is_empty()
    }

    /// Extract (EdgeType, TargetRef) pairs from all declared relationships.
    ///
    /// Skips values that don't parse as UUID or slug (e.g., external URLs).
    /// The `parent` field produces a reversed edge: the parent resource gets
    /// a `ParentOf` edge pointing to this resource (handled by the caller).
    pub fn to_edge_declarations(&self) -> Vec<(EdgeType, TargetRef)> {
        let mut edges = Vec::new();
        let field_mappings: &[(&[String], EdgeType)] = &[
            (&self.relates_to, EdgeType::RelatesTo),
            (&self.extends, EdgeType::Extends),
            (&self.depends_on, EdgeType::DependsOn),
            (&self.references, EdgeType::References),
            (&self.preceded_by, EdgeType::PrecededBy),
            (&self.derived_from, EdgeType::DerivedFrom),
        ];
        for (values, edge_type) in field_mappings {
            for value in *values {
                if let Some(target) = TargetRef::parse(value) {
                    edges.push((*edge_type, target));
                }
            }
        }
        // Parent is a single value producing a reversed ParentOf edge
        if let Some(ref parent_val) = self.parent {
            if let Some(target) = TargetRef::parse(parent_val) {
                edges.push((EdgeType::ParentOf, target));
            }
        }
        edges
    }
}

// ─── Database Result Types ──────────────────────────────────────────────────

/// Graph traversal result row — mirrors the `graph_traverse()` SQL function.
///
/// **Single path per resource**: `graph_traverse()` uses `DISTINCT ON (resource_id)`
/// ordered by `depth ASC, path_weight DESC`, so only the shallowest/strongest
/// path to each resource survives. Callers that need all paths should query the
/// raw recursive CTE directly.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GraphTraversalRow {
    pub resource_id: Uuid,
    pub depth: i32,
    pub path: Vec<Uuid>,
    pub edge_type: Option<EdgeType>,
    pub from_resource_id: Option<Uuid>,
    pub path_weight: f64,
}

/// Graph neighbor row — mirrors the `graph_neighbors()` SQL function.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GraphNeighborRow {
    pub resource_id: Uuid,
    pub edge_type: EdgeType,
    pub direction: String,
    pub weight: f64,
    pub metadata: serde_json::Value,
}

/// Edge listing row — mirrors the `graph_resource_edges()` SQL function.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct GraphEdgeRow {
    pub edge_id: Uuid,
    pub peer_resource_id: Uuid,
    pub peer_title: String,
    pub peer_slug: String,
    pub edge_type: EdgeType,
    pub direction: String,
    pub weight: f64,
    pub metadata: serde_json::Value,
    pub created: chrono::DateTime<chrono::Utc>,
}

/// A resolved edge ready for database insertion.
#[derive(Debug, Clone)]
pub struct ResolvedEdge {
    pub source_resource_id: Uuid,
    pub target_resource_id: Uuid,
    pub edge_type: EdgeType,
    pub weight: f64,
    pub metadata: serde_json::Value,
}

/// Result of edge reconciliation after a frontmatter update.
#[derive(Debug, Clone)]
pub struct EdgeReconciliation {
    pub added: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub deferred: usize,
}

// ─── Subgraph Response Types ────────────────────────────────────────────────

/// Whether a doctype is an *aggregator* (gravity-well node that clusters
/// participants) or a *participant* (leaf node).
///
/// Drives the R11 visual distinction: aggregators render larger, italic, with
/// a soft radial wash; participants render as plain typeset words. Sessions
/// are neither — they're annotations, not graph nodes.
pub fn is_aggregator(doc_type: DocType) -> bool {
    matches!(
        doc_type,
        DocType::Goal | DocType::Concept | DocType::Decision
    )
}

/// One node in a returned subgraph.
///
/// `edge_count` is the resource's **total** edge count in the graph (not just
/// within the returned subgraph) — used client-side to size the visual radius.
/// `session_count` is the number of `session`-typed resources that share any
/// edge with this node; sessions themselves are **not** returned as nodes.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    /// Typed doctype — serializes as `"concept"`, `"research"`, etc.
    pub doc_type: DocType,
    /// Whether this node is an aggregator (goal/concept/decision) vs a
    /// participant (research/task). Derived server-side from `doc_type` so
    /// the client doesn't have to repeat the classification.
    pub aggregator: bool,
    /// Count of all edges touching this resource, regardless of subgraph scope.
    pub edge_count: i32,
    /// Count of `session`-typed resources that share any edge with this node.
    /// Renders as a `⌊N⌋` annotation glyph in the UI.
    pub session_count: i32,
    /// First-paragraph body preview (≤ 280 chars, truncated on a word boundary
    /// with an ellipsis suffix). `None` when the resource has no body text.
    /// Renders as the `EXCERPT` block in the resource peek panel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    /// Task workflow stage (e.g. `"in-progress"`, `"backlog"`). Only populated
    /// for `DocType::Task` rows, sourced from `managed_meta.temper-stage`.
    /// Renders as a small mono-caps tag under the task label at the detail
    /// zoom tier (`node.tier-detail.type-task`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}

/// One directed edge in a returned subgraph.
///
/// Both `source` and `target` are guaranteed to appear as `id` on a node
/// in the same `SubgraphResponse` — edges to/from nodes outside the
/// subgraph are filtered out server-side.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: Uuid,
    pub target: Uuid,
    pub edge_type: EdgeType,
}

/// Full response body for `GET /api/graph/subgraph`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubgraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── TargetRef::parse ────────────────────────────────────────────────

    #[test]
    fn parse_valid_uuid() {
        let input = "019d1d24-2000-7379-8f26-ae4ae87bc5c6";
        let result = TargetRef::parse(input);
        assert_eq!(
            result,
            Some(TargetRef::Id(
                Uuid::parse_str("019d1d24-2000-7379-8f26-ae4ae87bc5c6").unwrap()
            ))
        );
    }

    #[test]
    fn parse_slug() {
        assert_eq!(
            TargetRef::parse("r2-data-model"),
            Some(TargetRef::Slug("r2-data-model".to_string()))
        );
    }

    #[test]
    fn parse_slug_with_whitespace_trimming() {
        assert_eq!(
            TargetRef::parse("  some-slug  "),
            Some(TargetRef::Slug("some-slug".to_string()))
        );
    }

    #[test]
    fn parse_empty_returns_none() {
        assert_eq!(TargetRef::parse(""), None);
        assert_eq!(TargetRef::parse("   "), None);
    }

    #[test]
    fn parse_http_url_returns_none() {
        assert_eq!(TargetRef::parse("https://neon.tech/docs"), None);
        assert_eq!(TargetRef::parse("http://example.com"), None);
    }

    // ── ResourceRelationships ───────────────────────────────────────────

    #[test]
    fn empty_relationships() {
        let rels = ResourceRelationships::default();
        assert!(rels.is_empty());
        assert!(rels.to_edge_declarations().is_empty());
    }

    #[test]
    fn to_edge_declarations_extracts_all_types() {
        let rels = ResourceRelationships {
            extends: vec!["r2-data-model".to_string()],
            depends_on: vec![
                "019d1d24-2000-7379-8f26-ae4ae87bc5c6".to_string(),
                "r3-platform-eval".to_string(),
            ],
            references: vec![
                "r1-workflow-vision".to_string(),
                "https://external.com/doc".to_string(), // should be skipped
            ],
            parent: Some("milestone-q2".to_string()),
            ..Default::default()
        };

        assert!(!rels.is_empty());

        let declarations = rels.to_edge_declarations();
        // extends: 1, depends_on: 2, references: 1 (URL skipped), parent: 1 = 5
        assert_eq!(declarations.len(), 5);

        // Verify edge types
        assert!(declarations.iter().any(|(t, _)| *t == EdgeType::Extends));
        assert_eq!(
            declarations
                .iter()
                .filter(|(t, _)| *t == EdgeType::DependsOn)
                .count(),
            2
        );
        // URL reference should be filtered out
        assert_eq!(
            declarations
                .iter()
                .filter(|(t, _)| *t == EdgeType::References)
                .count(),
            1
        );
        // Parent produces ParentOf
        assert!(declarations.iter().any(|(t, _)| *t == EdgeType::ParentOf));
    }

    // ── EdgeType serde ──────────────────────────────────────────────────

    #[test]
    fn edge_type_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&EdgeType::DependsOn).unwrap(),
            "\"depends_on\""
        );
        assert_eq!(
            serde_json::to_string(&EdgeType::RelatesTo).unwrap(),
            "\"relates_to\""
        );
    }

    #[test]
    fn edge_type_deserializes_from_snake_case() {
        let result: EdgeType = serde_json::from_str("\"derived_from\"").unwrap();
        assert_eq!(result, EdgeType::DerivedFrom);
    }

    #[test]
    fn edge_type_display() {
        assert_eq!(EdgeType::ParentOf.to_string(), "parent_of");
        assert_eq!(EdgeType::RelatesTo.to_string(), "relates_to");
    }

    // ── EdgeKind / Polarity ─────────────────────────────────────────────

    #[test]
    fn edge_kind_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&EdgeKind::LeadsTo).unwrap(),
            "\"leads_to\""
        );
        assert_eq!(serde_json::to_string(&EdgeKind::Near).unwrap(), "\"near\"");
    }

    #[test]
    fn polarity_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&Polarity::Inverse).unwrap(),
            "\"inverse\""
        );
    }

    #[test]
    fn legacy_mapping_covers_all_seven_edge_types() {
        // Every legacy EdgeType maps to a (kind, polarity, label).
        for et in [
            EdgeType::RelatesTo,
            EdgeType::Extends,
            EdgeType::DependsOn,
            EdgeType::References,
            EdgeType::ParentOf,
            EdgeType::PrecededBy,
            EdgeType::DerivedFrom,
        ] {
            let (kind, polarity, label) = et.legacy_mapping();
            assert!(!label.is_empty());
            // depends_on / extends / preceded_by / derived_from are inverse leads_to
            if matches!(
                et,
                EdgeType::DependsOn
                    | EdgeType::Extends
                    | EdgeType::PrecededBy
                    | EdgeType::DerivedFrom
            ) {
                assert_eq!(kind, EdgeKind::LeadsTo);
                assert_eq!(polarity, Polarity::Inverse);
            }
        }
        assert_eq!(EdgeType::ParentOf.legacy_mapping().0, EdgeKind::Contains);
        assert_eq!(EdgeType::RelatesTo.legacy_mapping().0, EdgeKind::Near);
        assert_eq!(EdgeType::References.legacy_mapping().0, EdgeKind::Near);
    }

    // ── ResourceRelationships serde round-trip ──────────────────────────

    #[test]
    fn relationships_serde_round_trip() {
        let rels = ResourceRelationships {
            extends: vec!["r2-data-model".to_string()],
            depends_on: vec!["r3-platform".to_string()],
            ..Default::default()
        };
        let json = serde_json::to_string(&rels).unwrap();
        let parsed: ResourceRelationships = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.extends, rels.extends);
        assert_eq!(parsed.depends_on, rels.depends_on);
        assert!(parsed.relates_to.is_empty());
    }

    #[test]
    fn relationships_deserialize_skips_missing_fields() {
        let json = r#"{"extends": ["foo"]}"#;
        let parsed: ResourceRelationships = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.extends, vec!["foo"]);
        assert!(parsed.depends_on.is_empty());
        assert!(parsed.parent.is_none());
    }
}
