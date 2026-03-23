use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use crate::config::Config;
use crate::embedder::{self, Embedder};
use crate::error::{Result, TemperError};
use crate::format::OutputFormat;
use crate::hnsw::SearchIndex;

#[derive(Debug, Serialize)]
struct ChunkResult {
    score: f32,
    header_path: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct GroupedResult {
    file_path: String,
    note_type: String,
    title: String,
    chunks: Vec<ChunkResult>,
}

#[derive(Debug, Serialize)]
struct NoteDetail {
    path: String,
    title: String,
    tags: Vec<String>,
    content: String,
}

#[derive(Debug, Serialize)]
struct ContextOutput {
    topic: String,
    primary: Option<NoteDetail>,
    related_chunks: Vec<GroupedResult>,
    hop: usize,
}

impl fmt::Display for ContextOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== Context: {} ===", self.topic)?;

        if let Some(ref primary) = self.primary {
            if !primary.tags.is_empty() {
                writeln!(f, "tags: {}", primary.tags.join(", "))?;
            }
            writeln!(f)?;
            writeln!(f, "{}", primary.content)?;
        }

        if !self.related_chunks.is_empty() {
            if self.hop > 1 {
                writeln!(f, "--- Related (hop {}) ---", self.hop)?;
            } else {
                writeln!(f, "--- Related ---")?;
            }
            writeln!(f)?;
            for group in &self.related_chunks {
                let best_score = group.chunks.first().map(|c| c.score).unwrap_or(0.0);
                writeln!(f, "{} ({}) [{:.2}]", group.file_path, group.note_type, best_score)?;
                for chunk in &group.chunks {
                    let header_display = if chunk.header_path.is_empty() {
                        String::new()
                    } else {
                        format!("[{}]", chunk.header_path)
                    };
                    writeln!(f, "  [{:.2}] {}", chunk.score, header_display)?;
                    if let Some(line) = chunk.content.lines().find(|l| !l.trim().is_empty()) {
                        writeln!(f, "    > {line}")?;
                    }
                }
                writeln!(f)?;
            }
        }

        Ok(())
    }
}

/// Resolve a topic to an index entry vector.
///
/// Resolution order:
/// 1. Exact file path match in vault
/// 2. Exact title match among indexed notes
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
        let content = std::fs::read_to_string(&candidate).map_err(|e| {
            TemperError::Io(e)
        })?;
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
    exclude_paths: &std::collections::HashSet<String>,
    limit: usize,
) -> Vec<GroupedResult> {
    let mut groups: HashMap<String, GroupedResult> = HashMap::new();
    for hit in hits {
        if exclude_paths.contains(&hit.entry.file_path) {
            continue;
        }
        let group = groups.entry(hit.entry.file_path.clone()).or_insert_with(|| GroupedResult {
            file_path: hit.entry.file_path.clone(),
            note_type: hit.entry.metadata.note_type.clone(),
            title: hit.entry.metadata.note_type.clone(), // title not stored separately
            chunks: Vec::new(),
        });
        group.chunks.push(ChunkResult {
            score: hit.score,
            header_path: hit.entry.header_path.clone(),
            content: hit.entry.content.clone(),
        });
    }

    let mut related: Vec<GroupedResult> = groups.into_values().collect();
    related.sort_by(|a, b| {
        let a_score = a.chunks.first().map(|c| c.score).unwrap_or(0.0);
        let b_score = b.chunks.first().map(|c| c.score).unwrap_or(0.0);
        b_score.partial_cmp(&a_score).unwrap_or(std::cmp::Ordering::Equal)
    });
    related.truncate(limit);
    related
}

pub fn run(config: &Config, topic: &str, format: &str, depth: usize, limit: usize) -> Result<()> {
    let fmt = OutputFormat::parse(format);

    let state_dir = &config.state_dir;

    let index = match SearchIndex::load(state_dir) {
        Ok(idx) => idx,
        Err(_) => {
            println!("No search index found. Run 'temper index' to build it.");
            return Ok(());
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
            let title = fm.as_ref()
                .and_then(|v| v.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or(topic)
                .to_string();
            let tags: Vec<String> = fm.as_ref()
                .and_then(|v| v.get("tags"))
                .and_then(|v| v.as_sequence())
                .map(|seq| {
                    seq.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            NoteDetail {
                path: rel.clone(),
                title,
                tags,
                content,
            }
        })
    });

    // Hop 1: search for related content
    let mut exclude_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Some(ref p) = primary_path {
        exclude_paths.insert(p.clone());
    }

    let search_limit = limit * 4; // fetch wider set before grouping
    let hits = index.search(&query_vector, search_limit, None);
    let related = group_hits(hits, &exclude_paths, limit);

    // Collect the top-hop paths for depth > 1
    let hop1_paths: Vec<String> = related.iter().map(|g| g.file_path.clone()).collect();

    // Output hop 1
    let output = ContextOutput {
        topic: topic.to_string(),
        primary,
        related_chunks: related,
        hop: 1,
    };
    crate::format::output(&output, fmt);

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

            let hop_output = ContextOutput {
                topic: hop_path.clone(),
                primary: None,
                related_chunks: hop_related,
                hop: 2,
            };
            crate::format::output(&hop_output, fmt);
        }
    }

    Ok(())
}
