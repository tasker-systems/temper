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
    pub score: f32,
    pub file_path: String,
    pub chunk_index: usize,
    pub note_type: String,
    pub cluster: Option<String>,
    pub project: Option<String>,
    pub content: String,
}

/// Collection of search results.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub query: String,
    pub hits: Vec<SearchHit>,
}

/// A single chunk result within a grouped context result.
#[derive(Debug, Clone, Serialize)]
pub struct ContextChunk {
    pub score: f32,
    pub header_path: String,
    pub content: String,
}

/// A group of related chunks from the same file.
#[derive(Debug, Clone, Serialize)]
pub struct ContextGroup {
    pub file_path: String,
    pub note_type: String,
    pub title: String,
    pub chunks: Vec<ContextChunk>,
}

/// Detail about the primary note resolved from a topic.
#[derive(Debug, Clone, Serialize)]
pub struct ContextNoteDetail {
    pub path: String,
    pub title: String,
    pub tags: Vec<String>,
    pub content: String,
}

/// A single hop of context results.
#[derive(Debug, Clone, Serialize)]
pub struct ContextHop {
    pub topic: String,
    pub primary: Option<ContextNoteDetail>,
    pub related_chunks: Vec<ContextGroup>,
    pub hop: usize,
}

/// Results from a context lookup (may contain multiple hops).
#[derive(Debug, Clone, Serialize)]
pub struct ContextResults {
    pub hops: Vec<ContextHop>,
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
