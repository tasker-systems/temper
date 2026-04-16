//! Cluster formation using HNSW nearest-neighbor search.
//!
//! Takes seed phrases from seeds.rs, embeds them, performs HNSW nearest-neighbor
//! search against the index manifest, and groups documents into clusters.

use std::path::PathBuf;

use temper_core::types::config::GraphIndexConfig;
use temper_llm::types::{Cluster, SeedPhrase};

use crate::actions::index_build::IndexManifest;

/// Load the index manifest from `.temper/index.json`.
pub fn load_manifest(temper_dir: &PathBuf) -> Option<IndexManifest> {
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
/// 1. Embed the phrase using `temper-ingest::embed::embed_text`
/// 2. HNSW nearest-neighbor search → candidate chunk set
/// 3. Group chunks by document (one doc may contribute multiple chunks)
/// 4. Apply `cluster_similarity_threshold` and `cluster_max_members` cap
pub fn form_clusters(
    seeds: &[SeedPhrase],
    manifest: &IndexManifest,
    config: &GraphIndexConfig,
) -> Vec<Cluster> {
    let mut clusters = Vec::new();

    for seed in seeds {
        // Embed the seed phrase
        let seed_embedding = match temper_ingest::embed::embed_text(&seed.phrase) {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Find nearest neighbors in the index
        // TODO: Use hnsw_rs for actual nearest-neighbor search once the index is built
        // For now, use doc_embedding cosine similarity
        let mut doc_scores: Vec<(String, f32)> = Vec::new();
        for file in &manifest.files {
            let sim = cosine_similarity(&seed_embedding, &file.doc_embedding);
            if sim >= config.cluster_similarity_threshold {
                doc_scores.push((file.rel_path.clone(), sim));
            }
        }

        // Sort by similarity, cap at cluster_max_members
        doc_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        doc_scores.truncate(config.cluster_max_members);

        if doc_scores.len() >= config.concept_min_members {
            let member_ids: Vec<String> = doc_scores.iter().map(|(id, _)| id.clone()).collect();
            let member_embeddings: Vec<Vec<f32>> = doc_scores
                .iter()
                .filter_map(|(rel_path, _)| {
                    manifest.files.iter()
                        .find(|f| &f.rel_path == rel_path)
                        .map(|f| f.doc_embedding.clone())
                })
                .collect();

            clusters.push(Cluster::new(seed.clone(), member_ids, member_embeddings));
        }
    }

    clusters
}

/// Compute cosine similarity between two vectors.
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