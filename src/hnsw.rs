// HNSW vector search index

use std::collections::HashMap;
use std::path::Path;

use instant_distance::{Builder, HnswMap, Search};
use serde::{Deserialize, Serialize};

use crate::chunker::ChunkMeta;
use crate::error::{Result, TemperError};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub chunk_id: String,
    pub file_path: String,
    pub chunk_index: usize,
    pub header_path: String,
    pub content: String,
    pub metadata: ChunkMeta,
}

pub struct SearchFilter {
    pub note_type: Option<String>,
    pub cluster: Option<String>,
    pub project: Option<String>,
    pub tags: Option<Vec<String>>,
}

pub struct SearchHit {
    pub entry: IndexEntry,
    pub score: f32, // cosine similarity
}

// ---------------------------------------------------------------------------
// Internal: the Point newtype that instant-distance requires
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct EmbeddingPoint(Vec<f32>);

impl instant_distance::Point for EmbeddingPoint {
    fn distance(&self, other: &Self) -> f32 {
        // cosine distance = 1 - cosine_similarity
        // for unit vectors this is just 1 - dot product
        let dot: f32 = self.0.iter().zip(other.0.iter()).map(|(a, b)| a * b).sum();
        1.0 - dot.clamp(-1.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// Serializable index data (entries + vectors; the HNSW graph is rebuilt on load)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct IndexData {
    entries: Vec<IndexEntry>,
    vectors: Vec<Vec<f32>>,
}

// ---------------------------------------------------------------------------
// SearchIndex
// ---------------------------------------------------------------------------

pub struct SearchIndex {
    /// The HNSW map; values are indices into `data.entries`.
    hnsw: Option<HnswMap<EmbeddingPoint, usize>>,
    /// The serialisable half — kept so we can save/reload and serve cached vectors.
    data: IndexData,
}

impl SearchIndex {
    /// Build an index from parallel slices of entries and embedding vectors.
    pub fn build(entries: Vec<IndexEntry>, vectors: Vec<Vec<f32>>) -> Result<Self> {
        if entries.len() != vectors.len() {
            return Err(TemperError::Index(format!(
                "entries ({}) and vectors ({}) lengths differ",
                entries.len(),
                vectors.len()
            )));
        }

        let hnsw = if entries.is_empty() {
            None
        } else {
            let points: Vec<EmbeddingPoint> =
                vectors.iter().map(|v| EmbeddingPoint(v.clone())).collect();
            let values: Vec<usize> = (0..entries.len()).collect();
            Some(Builder::default().build(points, values))
        };

        Ok(Self {
            hnsw,
            data: IndexData { entries, vectors },
        })
    }

    /// Atomically save the index to `<state_dir>/index.bin`.
    pub fn save(&self, state_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(state_dir)?;

        let target = state_dir.join("index.bin");
        let tmp = state_dir.join("index.bin.tmp");

        let bytes = bincode::serialize(&self.data)
            .map_err(|e| TemperError::Index(format!("serialise failed: {e}")))?;

        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &target)?;
        Ok(())
    }

    /// Load from `<state_dir>/index.bin` and rebuild the HNSW graph.
    pub fn load(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join("index.bin");
        let bytes = std::fs::read(&path)
            .map_err(|e| TemperError::Index(format!("could not read {}: {e}", path.display())))?;

        let data: IndexData = bincode::deserialize(&bytes)
            .map_err(|e| TemperError::Index(format!("deserialise failed: {e}")))?;

        let hnsw = if data.entries.is_empty() {
            None
        } else {
            let points: Vec<EmbeddingPoint> = data
                .vectors
                .iter()
                .map(|v| EmbeddingPoint(v.clone()))
                .collect();
            let values: Vec<usize> = (0..data.entries.len()).collect();
            Some(Builder::default().build(points, values))
        };

        Ok(Self { hnsw, data })
    }

    /// Search for nearest neighbours to `query_vector`, applying an optional post-filter.
    pub fn search(
        &self,
        query_vector: &[f32],
        limit: usize,
        filter: Option<&SearchFilter>,
    ) -> Vec<SearchHit> {
        let hnsw = match &self.hnsw {
            Some(h) => h,
            None => return Vec::new(),
        };

        if limit == 0 {
            return Vec::new();
        }

        let query_point = EmbeddingPoint(query_vector.to_vec());
        let mut search = Search::default();

        // Fetch a wider candidate set so filtering doesn't leave us short.
        let candidates: Vec<(usize, f32)> = hnsw
            .search(&query_point, &mut search)
            .map(|item| {
                let idx = *item.value;
                let distance = item.distance; // cosine distance in [0, 2]
                let similarity = 1.0 - distance;
                (idx, similarity)
            })
            .collect();

        let mut hits: Vec<SearchHit> = candidates
            .into_iter()
            .filter(|(idx, _)| {
                let entry = &self.data.entries[*idx];
                filter_matches(entry, filter)
            })
            .take(limit)
            .map(|(idx, score)| SearchHit {
                entry: self.data.entries[idx].clone(),
                score,
            })
            .collect();

        // If filtering left us sparse, widen the search by rescanning all entries
        // sorted by their stored vector similarity.
        if let Some(f) = filter {
            if hits.len() < limit {
                let q: Vec<f32> = query_vector.to_vec();
                let mut all: Vec<(usize, f32)> = self
                    .data
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| filter_matches(e, Some(f)))
                    .map(|(i, _)| {
                        let vec = &self.data.vectors[i];
                        let dot: f32 = q.iter().zip(vec.iter()).map(|(a, b)| a * b).sum();
                        (i, dot)
                    })
                    .collect();

                all.sort_unstable_by(|a, b| {
                    b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                });

                // Rebuild hits from the full brute-force pass
                hits = all
                    .into_iter()
                    .take(limit)
                    .map(|(idx, score)| SearchHit {
                        entry: self.data.entries[idx].clone(),
                        score,
                    })
                    .collect();
            }
        }

        hits
    }

    /// Total number of indexed chunks.
    pub fn entry_count(&self) -> usize {
        self.data.entries.len()
    }

    /// Number of distinct source files in the index.
    pub fn file_count(&self) -> usize {
        self.data
            .entries
            .iter()
            .map(|e| e.file_path.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    /// Find the first indexed entry whose title (case-insensitive) matches `title`.
    pub fn find_by_title(&self, title: &str) -> Option<&IndexEntry> {
        let lower = title.to_lowercase();
        self.data
            .entries
            .iter()
            .find(|e| e.metadata.title.to_lowercase() == lower)
    }

    /// Returns a `chunk_id → vector` map for use in incremental re-indexing.
    pub fn cached_vectors(&self) -> HashMap<String, Vec<f32>> {
        self.data
            .entries
            .iter()
            .zip(self.data.vectors.iter())
            .map(|(e, v)| (e.chunk_id.clone(), v.clone()))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn filter_matches(entry: &IndexEntry, filter: Option<&SearchFilter>) -> bool {
    let f = match filter {
        Some(f) => f,
        None => return true,
    };

    if let Some(ref nt) = f.note_type {
        if &entry.metadata.note_type != nt {
            return false;
        }
    }
    if let Some(ref cluster) = f.cluster {
        match &entry.metadata.cluster {
            Some(c) if c == cluster => {}
            _ => return false,
        }
    }
    if let Some(ref project) = f.project {
        match &entry.metadata.project {
            Some(p) if p == project => {}
            _ => return false,
        }
    }
    if let Some(ref required_tags) = f.tags {
        for tag in required_tags {
            if !entry.metadata.tags.contains(tag) {
                return false;
            }
        }
    }
    true
}
