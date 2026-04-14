//! Knowledge graph types — edge types, traversal results, and relationship
//! declarations for the R7 vertex-edge graph stored in `kb_resource_edges`.

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
