use std::collections::{HashMap, HashSet};

use crate::actions::types::{
    ContextChunk, ContextGroup, ContextHop, ContextNoteDetail, ContextResults,
};
use crate::config::Config;
use crate::embedder::{self, Embedder};
use crate::error::{Result, TemperError};
use crate::hnsw::SearchIndex;

/// Resolve a topic to an index entry vector.
///
/// Resolution order:
/// 1. Exact file path match in vault
/// 2. Exact title match among indexed notes
/// 3. Fallback: embed the topic as a query string
///
/// Returns (path_hint, embedding_vector)
fn resolve_topic(
    config: &Config,
    index: &SearchIndex,
    embedder: &mut Embedder,
    topic: &str,
) -> Result<(Option<String>, Vec<f32>)> {
    // Step 1: exact file path match in vault
    let candidate = config.vault_root.join(topic);
    if candidate.exists() {
        let content = std::fs::read_to_string(&candidate).map_err(TemperError::Io)?;
        let preprocessed = embedder::preprocess_chunk(&content, "");
        let vec = embedder.embed(&preprocessed)?;
        let rel = candidate
            .strip_prefix(&config.vault_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| topic.to_string());
        return Ok((Some(rel), vec));
    }

    // Step 2: exact title match in indexed notes
    if let Some(entry) = index.find_by_title(topic) {
        let preprocessed = embedder::preprocess_chunk(&entry.content, &entry.header_path);
        let vec = embedder.embed(&preprocessed)?;
        return Ok((Some(entry.file_path.clone()), vec));
    }

    // Fallback: embed the topic as a query string
    let preprocessed = embedder::preprocess_chunk(topic, "");
    let vec = embedder.embed(&preprocessed)?;
    Ok((None, vec))
}

/// Group search hits by file, excluding a set of already-seen paths.
fn group_hits(
    hits: Vec<crate::hnsw::SearchHit>,
    exclude_paths: &HashSet<String>,
    limit: usize,
) -> Vec<ContextGroup> {
    let mut groups: HashMap<String, ContextGroup> = HashMap::new();
    for hit in hits {
        if exclude_paths.contains(&hit.entry.file_path) {
            continue;
        }
        let group = groups
            .entry(hit.entry.file_path.clone())
            .or_insert_with(|| ContextGroup {
                file_path: hit.entry.file_path.clone(),
                note_type: hit.entry.metadata.note_type.clone(),
                title: hit.entry.metadata.note_type.clone(), // title not stored separately
                chunks: Vec::new(),
            });
        group.chunks.push(ContextChunk {
            score: hit.score,
            header_path: hit.entry.header_path.clone(),
            content: hit.entry.content.clone(),
        });
    }

    let mut related: Vec<ContextGroup> = groups.into_values().collect();
    related.sort_by(|a, b| {
        let a_score = a.chunks.first().map(|c| c.score).unwrap_or(0.0);
        let b_score = b.chunks.first().map(|c| c.score).unwrap_or(0.0);
        b_score
            .partial_cmp(&a_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    related.truncate(limit);
    related
}

/// Execute a context lookup with multi-hop traversal and return structured results.
///
/// Returns empty results (not an error) when no index exists.
pub fn run(config: &Config, topic: &str, depth: usize, limit: usize) -> Result<ContextResults> {
    let state_dir = &config.state_dir;

    let index = match SearchIndex::load(state_dir) {
        Ok(idx) => idx,
        Err(_) => {
            return Ok(ContextResults { hops: vec![] });
        }
    };

    let mut embedder = Embedder::new(config.model_cache_dir.clone());

    // Resolve topic to a primary note + embedding vector
    let (primary_path, query_vector) = resolve_topic(config, &index, &mut embedder, topic)?;

    // Load primary note detail if we found a file
    let primary = primary_path.as_ref().and_then(|rel| {
        let full = config.vault_root.join(rel);
        std::fs::read_to_string(&full).ok().map(|content| {
            let fm = crate::vault::parse_frontmatter(&content);
            let title = fm
                .as_ref()
                .and_then(|v| v.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or(topic)
                .to_string();
            let tags: Vec<String> = fm
                .as_ref()
                .and_then(|v| v.get("tags"))
                .and_then(|v| v.as_sequence())
                .map(|seq| {
                    seq.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            ContextNoteDetail {
                path: rel.clone(),
                title,
                tags,
                content,
            }
        })
    });

    // Hop 1: search for related content
    let mut exclude_paths: HashSet<String> = HashSet::new();
    if let Some(ref p) = primary_path {
        exclude_paths.insert(p.clone());
    }

    let search_limit = limit * 4; // fetch wider set before grouping
    let hits = index.search(&query_vector, search_limit, None);
    let related = group_hits(hits, &exclude_paths, limit);

    // Collect the top-hop paths for depth > 1
    let hop1_paths: Vec<String> = related.iter().map(|g| g.file_path.clone()).collect();

    let mut hops = vec![ContextHop {
        topic: topic.to_string(),
        primary,
        related_chunks: related,
        hop: 1,
    }];

    // Depth > 1: fan-out from hop 1 results
    if depth > 1 {
        for hop_path in &hop1_paths {
            exclude_paths.insert(hop_path.clone());
        }

        for hop_path in &hop1_paths {
            // Load and embed the hop-1 note
            let full = config.vault_root.join(hop_path);
            let content = match std::fs::read_to_string(&full) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let preprocessed = embedder::preprocess_chunk(&content, "");
            let hop_vector = match embedder.embed(&preprocessed) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let hop_hits = index.search(&hop_vector, search_limit, None);
            let hop_related = group_hits(hop_hits, &exclude_paths, limit);

            if hop_related.is_empty() {
                continue;
            }

            hops.push(ContextHop {
                topic: hop_path.clone(),
                primary: None,
                related_chunks: hop_related,
                hop: 2,
            });
        }
    }

    Ok(ContextResults { hops })
}
