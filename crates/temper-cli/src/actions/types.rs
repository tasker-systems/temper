use serde::{Deserialize, Serialize};

/// Task metadata parsed from frontmatter.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskInfo {
    pub title: String,
    pub slug: String,
    #[serde(alias = "context", alias = "temper-context")]
    pub context: String,
    #[serde(alias = "goal", alias = "temper-goal")]
    pub goal: String,
    #[serde(alias = "stage", alias = "temper-stage")]
    pub stage: String,
    #[serde(alias = "mode", alias = "temper-mode")]
    pub mode: Option<String>,
    #[serde(alias = "effort", alias = "temper-effort")]
    pub effort: Option<String>,
    #[serde(alias = "seq", alias = "temper-seq")]
    pub seq: u32,
    #[serde(alias = "branch", alias = "temper-branch")]
    pub branch: Option<String>,
    #[serde(alias = "pr", alias = "temper-pr")]
    pub pr: Option<String>,
}

/// Goal metadata parsed from frontmatter.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoalInfo {
    pub title: String,
    pub slug: String,
    #[serde(alias = "context", alias = "temper-context")]
    pub context: String,
    #[serde(alias = "seq", alias = "temper-seq")]
    pub seq: u32,
    #[serde(alias = "status", alias = "temper-status")]
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

/// Statistics about a completed index run.
#[derive(Debug, Clone, Serialize)]
pub struct IndexStats {
    pub documents: usize,
    pub chunks: usize,
    pub duration_secs: f64,
}

/// Summary of a normalize run.
#[derive(Debug, Clone, Serialize)]
pub struct NormalizeSummary {
    pub ids_backfilled: u32,
    pub files_moved: u32,
    pub stages_migrated: u32,
    pub slugs_fixed: u32,
    pub frontmatter_fixed: u32,
    pub tasks_without_effort: u32,
}

/// A document in the vault with its content and metadata.
#[derive(Debug, Clone)]
pub struct VaultDocument {
    pub path: String,
    pub note_type: String,
    pub title: String,
    pub frontmatter: serde_yaml::Value,
    pub body: String,
}
