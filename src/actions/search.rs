use crate::actions::types::{SearchHit, SearchResults};
use crate::config::Config;
use crate::embedder::{self, Embedder};
use crate::error::Result;
use crate::hnsw::{SearchFilter, SearchIndex};

/// Execute a semantic search query and return structured results.
///
/// Loads the HNSW index, embeds the query, and returns matching hits.
/// Returns empty results (not an error) when no index exists or the index is empty.
pub fn run(
    config: &Config,
    query: &str,
    note_type: Option<&str>,
    project: Option<&str>,
    limit: usize,
) -> Result<SearchResults> {
    let state_dir = &config.state_dir;

    let index = match SearchIndex::load(state_dir) {
        Ok(idx) => idx,
        Err(_) => {
            return Ok(SearchResults {
                query: query.to_string(),
                hits: vec![],
            });
        }
    };

    if index.entry_count() == 0 {
        return Ok(SearchResults {
            query: query.to_string(),
            hits: vec![],
        });
    }

    let mut embedder = Embedder::new(config.model_cache_dir.clone());
    let preprocessed = embedder::preprocess_chunk(query, "");
    let query_vector = embedder.embed(&preprocessed)?;

    let filter = if note_type.is_some() || project.is_some() {
        Some(SearchFilter {
            note_type: note_type.map(String::from),
            cluster: None,
            project: project.map(String::from),
            tags: None,
        })
    } else {
        None
    };

    let hits = index.search(&query_vector, limit, filter.as_ref());

    let hit_outputs: Vec<SearchHit> = hits
        .into_iter()
        .map(|h| SearchHit {
            score: h.score,
            file_path: h.entry.file_path,
            chunk_index: h.entry.chunk_index,
            note_type: h.entry.metadata.note_type,
            cluster: h.entry.metadata.cluster,
            project: h.entry.metadata.project,
            content: h.entry.content,
        })
        .collect();

    Ok(SearchResults {
        query: query.to_string(),
        hits: hit_outputs,
    })
}
