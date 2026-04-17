//! Cluster formation using document-embedding nearest-neighbor search.
//!
//! Takes seed phrases from [`super::seeds`], embeds them, scores each vault
//! document's `doc_embedding` by cosine similarity, and groups the top matches
//! into clusters suitable for LLM judgment.
//!
//! Reads the `.temper/index.json` sidecar produced by `temper index` directly
//! via a local lightweight view so this module does not depend on the private
//! `IndexManifest` shape in `index_build`.

use std::collections::HashSet;
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
///
/// After forming all clusters, drops duplicates whose `member_ids` overlap
/// any earlier-accepted cluster by more than `cluster_overlap_threshold`
/// (Jaccard). Because seeds arrive in descending TF-IDF score, the
/// higher-scored cluster always wins.
pub fn form_clusters(
    seeds: &[SeedPhrase],
    manifest: &IndexManifestView,
    config: &GraphIndexConfig,
) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::new();

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

    dedupe_overlapping_clusters(clusters, config.cluster_overlap_threshold)
}

/// Drop clusters whose `member_ids` overlap an earlier-accepted cluster by
/// more than `threshold` (Jaccard of member-id sets). Input order encodes
/// priority — the first-seen cluster wins, later duplicates are dropped.
fn dedupe_overlapping_clusters(clusters: Vec<Cluster>, threshold: f32) -> Vec<Cluster> {
    let mut deduped: Vec<Cluster> = Vec::new();
    for candidate in clusters {
        let candidate_set: HashSet<&str> =
            candidate.member_ids.iter().map(String::as_str).collect();
        let keep = !deduped.iter().any(|accepted| {
            let accepted_set: HashSet<&str> =
                accepted.member_ids.iter().map(String::as_str).collect();
            let intersection = candidate_set.intersection(&accepted_set).count();
            let union = candidate_set.union(&accepted_set).count();
            if union == 0 {
                return false;
            }
            let jaccard = intersection as f32 / union as f32;
            jaccard > threshold
        });
        if keep {
            deduped.push(candidate);
        }
    }
    deduped
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_cluster(phrase: &str, member_ids: &[&str]) -> Cluster {
        let seed = SeedPhrase::new(
            phrase.to_string(),
            member_ids.len(),
            member_ids.iter().map(|s| s.to_string()).collect(),
        );
        let member_ids: Vec<String> = member_ids.iter().map(|s| s.to_string()).collect();
        let member_embeddings = vec![Vec::<f32>::new(); member_ids.len()];
        Cluster::new(seed, member_ids, member_embeddings)
    }

    #[test]
    fn test_form_clusters_dedupes_overlapping() {
        // Three clusters, input in descending-score order:
        //   - "alpha"   : members {1,2,3,4}
        //   - "bravo"   : members {1,2,3,5} — Jaccard with alpha = 3/5 = 0.6 (kept at default 0.8)
        //   - "charlie" : members {1,2,3,4,5} — Jaccard with alpha = 4/5 = 0.8 (kept at 0.8)
        //   - "delta"   : members {1,2,3,4}   — Jaccard with alpha = 4/4 = 1.0 (dropped)
        //   - "echo"    : members {10,11,12}  — Jaccard with all = 0 (kept)
        let clusters = vec![
            mk_cluster("alpha", &["1", "2", "3", "4"]),
            mk_cluster("bravo", &["1", "2", "3", "5"]),
            mk_cluster("charlie", &["1", "2", "3", "4", "5"]),
            mk_cluster("delta", &["1", "2", "3", "4"]),
            mk_cluster("echo", &["10", "11", "12"]),
        ];

        let deduped = dedupe_overlapping_clusters(clusters, 0.8);
        let surviving: Vec<&str> = deduped.iter().map(|c| c.seed.phrase.as_str()).collect();

        // "alpha" (highest score) wins; "delta" is a full duplicate (J=1.0) → dropped.
        // "charlie" vs "alpha" has J=4/5=0.8 which is NOT > 0.8 → kept.
        // "bravo" has J=3/5=0.6 → kept.
        // "echo" disjoint → kept.
        assert_eq!(
            surviving,
            vec!["alpha", "bravo", "charlie", "echo"],
            "expected delta dropped as full duplicate of alpha"
        );
    }

    #[test]
    fn test_form_clusters_dedupes_full_duplicate_keeps_first() {
        // Two clusters with identical member sets — Jaccard=1.0, later one
        // must be dropped regardless of seed phrase.
        let clusters = vec![
            mk_cluster("first", &["a", "b", "c", "d"]),
            mk_cluster("second", &["a", "b", "c", "d"]),
        ];
        let deduped = dedupe_overlapping_clusters(clusters, 0.8);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].seed.phrase, "first");
    }
}
