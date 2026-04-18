//! Shared types for LLM-assisted graph indexing.
//!
//! These types are used across `temper-cli` (graph index pipeline) and
//! `temper-llm` (agent harness). Defined here to avoid circular dependencies.

use serde::{Deserialize, Serialize};

/// A candidate seed phrase extracted via TF-IDF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedPhrase {
    /// The raw phrase text.
    pub phrase: String,
    /// Number of documents this phrase appears in.
    pub doc_frequency: usize,
    /// Top document IDs by TF-IDF score within this phrase's context.
    pub top_doc_ids: Vec<String>,
}

impl SeedPhrase {
    pub fn new(phrase: String, doc_frequency: usize, top_doc_ids: Vec<String>) -> Self {
        Self {
            phrase,
            doc_frequency,
            top_doc_ids,
        }
    }
}

/// A cluster of documents around a seed phrase, formed via HNSW nearest-neighbor search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    /// The seed this cluster was formed around.
    pub seed: SeedPhrase,
    /// Document IDs in this cluster.
    pub member_ids: Vec<String>,
    /// Per-member embeddings used for similarity scoring.
    pub member_embeddings: Vec<Vec<f32>>,
}

impl Cluster {
    pub fn new(
        seed: SeedPhrase,
        member_ids: Vec<String>,
        member_embeddings: Vec<Vec<f32>>,
    ) -> Self {
        Self {
            seed,
            member_ids,
            member_embeddings,
        }
    }

    pub fn member_count(&self) -> usize {
        self.member_ids.len()
    }
}

/// A concept proposal from LLM judgment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptProposal {
    /// Whether this cluster represents a coherent concept.
    pub is_concept: bool,
    /// Proposed slug (kebab-case, URL-safe).
    pub slug: Option<String>,
    /// Human-readable title.
    pub title: Option<String>,
    /// Markdown body for the Concept file.
    pub body_markdown: Option<String>,
    /// Edges to add to member documents.
    pub member_edges: Vec<MemberEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberEdge {
    /// The canonical rel_path of the member document (e.g.
    /// `"@me/temper/task/foo.md"`). The LLM echoes back exactly the path
    /// it was handed in the judgment prompt — materialization looks the
    /// file up by joining this onto `vault_root`.
    pub target_path: String,
    pub edge_type: String,
}
