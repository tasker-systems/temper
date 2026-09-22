use serde::{Deserialize, Serialize};
use temper_core::types::ids::ResourceId;

/// Task metadata parsed from frontmatter.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskInfo {
    /// The resource id, carried from the list row's top level (not a
    /// frontmatter key). Skipped in (de)serialization of `TaskInfo` itself —
    /// it is threaded in from the listing, not parsed from managed_meta.
    #[serde(skip)]
    pub id: ResourceId,
    #[serde(rename = "temper-title")]
    pub title: String,
    #[serde(rename = "temper-slug")]
    pub slug: String,
    #[serde(rename = "temper-context")]
    pub context: String,
    #[serde(rename = "temper-stage")]
    pub stage: String,
    /// Whether the managed tier carried `temper-stage` at all. `false` beside
    /// `temper-stage: ""` says the empty string is the deprecated legacy rendering
    /// of absence, not a stage value; `true` says the stage was set. Derived at
    /// construction — never a frontmatter key — and always emitted on stdout so
    /// new readers get the fact old renderings bury.
    #[serde(rename = "temper-stage-present", skip_deserializing)]
    pub stage_present: bool,
    #[serde(rename = "temper-mode")]
    pub mode: Option<String>,
    #[serde(rename = "temper-effort")]
    pub effort: Option<String>,
    #[serde(default, rename = "temper-seq")]
    pub seq: Option<u32>,
    #[serde(rename = "temper-branch")]
    pub branch: Option<String>,
    #[serde(rename = "temper-pr")]
    pub pr: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn task_with(stage: Option<&str>) -> TaskInfo {
        TaskInfo {
            id: ResourceId::from(uuid::Uuid::nil()),
            title: "a task".to_string(),
            slug: "a-task".to_string(),
            context: "@me/ctx".to_string(),
            stage: stage.unwrap_or("").to_string(),
            stage_present: stage.is_some(),
            mode: None,
            effort: None,
            seq: None,
            branch: None,
            pr: None,
        }
    }

    /// A stageless task keeps the legacy rendering — `temper-stage: ""` — so every
    /// existing stdout parser keeps parsing, with `temper-stage-present: false`
    /// naming the empty string as the deprecated rendering of absence.
    #[test]
    fn a_stageless_task_keeps_the_empty_string_rendering_beside_its_absence_signal() {
        let json = serde_json::to_string(&task_with(None)).unwrap();
        assert!(json.contains(r#""temper-stage":""#), "{json}");
        assert!(json.contains(r#""temper-stage-present":false"#), "{json}");
    }

    #[test]
    fn a_present_stage_serializes_under_its_canonical_name_with_its_signal() {
        let json = serde_json::to_string(&task_with(Some("in-progress"))).unwrap();
        assert!(json.contains(r#""temper-stage":"in-progress""#), "{json}");
        assert!(json.contains(r#""temper-stage-present":true"#), "{json}");
    }
}
