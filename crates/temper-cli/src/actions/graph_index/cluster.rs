//! Cluster formation using document-embedding nearest-neighbor search.
//!
//! Takes seed phrases from [`super::seeds`], embeds them, scores each vault
//! document's `doc_embedding` by cosine similarity, and groups the top matches
//! into clusters suitable for LLM judgment.
//!
//! Reads the `.temper/index.json` sidecar produced by `temper index` directly
//! via a local lightweight view so this module does not depend on the private
//! `IndexManifest` shape in `index_build`.

use std::path::Path;

use serde::Deserialize;

use temper_core::types::config::GraphIndexConfig;
use temper_llm::types::{Cluster, SeedPhrase};

/// Lightweight view of the on-disk index sidecar — only the fields this module
/// needs. Kept separate from `index_build::IndexManifest` to avoid a visibility
/// coupling across modules.
#[derive(Debug, Clone, Deserialize)]
pub struct IndexManifestView {
    pub files: Vec<ManifestFileView>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestFileView {
    pub rel_path: String,
    pub doc_embedding: Vec<f32>,
}

/// Load the index manifest from `.temper/index.json`, or `None` if absent.
pub fn load_manifest(temper_dir: &Path) -> Option<IndexManifestView> {
    let path = temper_dir.join("index.json");
    if !path.exists() {
        return None;
    }
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Form clusters from seed phrases using the index manifest.
///
/// For each seed phrase:
/// 1. Embed the phrase via `temper-ingest::embed::embed_text`.
/// 2. Score every manifest document by cosine similarity.
/// 3. Keep matches above `cluster_similarity_threshold`, capped at
///    `cluster_max_members`.
/// 4. Emit a `Cluster` iff ≥ `concept_min_members` members remain.
pub fn form_clusters(
    seeds: &[SeedPhrase],
    manifest: &IndexManifestView,
    config: &GraphIndexConfig,
) -> Vec<Cluster> {
    let mut clusters = Vec::new();

    for seed in seeds {
        let Ok(seed_embedding) = temper_ingest::embed::embed_text(&seed.phrase) else {
            continue;
        };

        let mut scored: Vec<(&ManifestFileView, f32)> = manifest
            .files
            .iter()
            .map(|f| (f, cosine_similarity(&seed_embedding, &f.doc_embedding)))
            .filter(|(_, sim)| *sim >= config.cluster_similarity_threshold)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(config.cluster_max_members);

        if scored.len() < config.concept_min_members {
            continue;
        }

        let member_ids: Vec<String> = scored.iter().map(|(f, _)| f.rel_path.clone()).collect();
        let member_embeddings: Vec<Vec<f32>> = scored
            .iter()
            .map(|(f, _)| f.doc_embedding.clone())
            .collect();

        clusters.push(Cluster::new(seed.clone(), member_ids, member_embeddings));
    }

    clusters
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}
