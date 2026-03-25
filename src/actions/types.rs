use serde::{Deserialize, Serialize};

/// Ticket metadata parsed from frontmatter.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TicketInfo {
    pub title: String,
    pub slug: String,
    pub project: String,
    pub milestone: String,
    pub stage: String,
    pub scope: Option<String>,
    pub seq: u32,
    pub branch: Option<String>,
    pub pr: Option<String>,
}

/// Milestone metadata parsed from frontmatter.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MilestoneInfo {
    pub title: String,
    pub slug: String,
    pub project: String,
    pub seq: u32,
    pub status: String,
}

/// A single search hit with score and metadata.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub title: String,
    pub score: f32,
    pub snippet: String,
}

/// Collection of search results.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub query: String,
}

/// A neighbor document in the context graph.
#[derive(Debug, Clone, Serialize)]
pub struct ContextNeighbor {
    pub path: String,
    pub title: String,
    pub score: f32,
    pub link_type: String,
}

/// Results from a context lookup.
#[derive(Debug, Clone, Serialize)]
pub struct ContextResults {
    pub source: String,
    pub neighbors: Vec<ContextNeighbor>,
}

/// Statistics about the vault index.
#[derive(Debug, Clone, Serialize)]
pub struct IndexStats {
    pub total_documents: usize,
    pub total_chunks: usize,
    pub indexed_documents: usize,
}

/// Summary of a normalize run.
#[derive(Debug, Clone, Serialize)]
pub struct NormalizeSummary {
    pub ids_backfilled: u32,
    pub files_moved: u32,
    pub stages_migrated: u32,
    pub slugs_fixed: u32,
    pub frontmatter_fixed: u32,
    pub unscoped_tickets: u32,
}

/// A document in the vault with its content and metadata.
#[derive(Debug, Clone, Serialize)]
pub struct VaultDocument {
    pub path: String,
    pub title: String,
    pub content: String,
}
