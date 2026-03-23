use std::collections::HashMap;
use temper_cli::chunker::ChunkMeta;
use temper_cli::hnsw::{IndexEntry, SearchFilter, SearchIndex};
use tempfile::TempDir;

const DIMS: usize = 384;

/// Generate a deterministic pseudo-random unit vector for the given seed.
fn seeded_unit_vector(seed: u64) -> Vec<f32> {
    // Simple LCG to produce reproducible values without extra deps
    let mut state = seed ^ 0x123456789abcdef;
    let mut raw: Vec<f32> = (0..DIMS)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            // map to [-1.0, 1.0]
            (state as i64 as f32) / (i64::MAX as f32)
        })
        .collect();

    // normalize to unit length
    let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut raw {
            *x /= norm;
        }
    }
    raw
}

fn make_entry(chunk_id: &str, file_path: &str, note_type: &str) -> IndexEntry {
    IndexEntry {
        chunk_id: chunk_id.to_string(),
        file_path: file_path.to_string(),
        chunk_index: 0,
        header_path: String::new(),
        content: format!("Content for {chunk_id}"),
        metadata: ChunkMeta {
            note_type: note_type.to_string(),
            cluster: None,
            project: None,
            tags: vec![],
            title: chunk_id.to_string(),
        },
    }
}

fn make_entry_with_cluster(chunk_id: &str, cluster: &str) -> IndexEntry {
    IndexEntry {
        chunk_id: chunk_id.to_string(),
        file_path: format!("{chunk_id}.md"),
        chunk_index: 0,
        header_path: String::new(),
        content: format!("Content for {chunk_id}"),
        metadata: ChunkMeta {
            note_type: "concept".to_string(),
            cluster: Some(cluster.to_string()),
            project: None,
            tags: vec![],
            title: chunk_id.to_string(),
        },
    }
}

// --- Tests ---

#[test]
fn test_build_and_search() {
    let v0 = seeded_unit_vector(1);
    let v1 = seeded_unit_vector(2);

    let entries = vec![
        make_entry("chunk-0", "a.md", "concept"),
        make_entry("chunk-1", "b.md", "session"),
    ];
    let vectors = vec![v0.clone(), v1.clone()];

    let index = SearchIndex::build(entries, vectors).expect("build should succeed");
    assert_eq!(index.entry_count(), 2);

    let hits = index.search(&v0, 1, None);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry.chunk_id, "chunk-0");
}

#[test]
fn test_save_and_load_round_trip() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path();

    let v0 = seeded_unit_vector(10);
    let v1 = seeded_unit_vector(11);

    let entries = vec![
        make_entry("round-0", "x.md", "concept"),
        make_entry("round-1", "y.md", "concept"),
    ];
    let vectors = vec![v0.clone(), v1.clone()];

    let index = SearchIndex::build(entries, vectors).expect("build");
    index.save(state_dir).expect("save");

    let loaded = SearchIndex::load(state_dir).expect("load");
    assert_eq!(loaded.entry_count(), 2);

    let hits = loaded.search(&v0, 1, None);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry.chunk_id, "round-0");
}

#[test]
fn test_filter_by_note_type() {
    let v0 = seeded_unit_vector(20);
    let v1 = seeded_unit_vector(21);
    let v2 = seeded_unit_vector(22);

    let entries = vec![
        make_entry("note-concept", "a.md", "concept"),
        make_entry("note-session", "b.md", "session"),
        make_entry("note-source", "c.md", "source"),
    ];
    let vectors = vec![v0.clone(), v1.clone(), v2.clone()];

    let index = SearchIndex::build(entries, vectors).expect("build");

    let filter = SearchFilter {
        note_type: Some("session".to_string()),
        cluster: None,
        project: None,
        tags: None,
    };

    let hits = index.search(&v1, 10, Some(&filter));
    assert_eq!(hits.len(), 1, "expected exactly 1 session hit");
    assert_eq!(hits[0].entry.chunk_id, "note-session");
    assert_eq!(hits[0].entry.metadata.note_type, "session");
}

#[test]
fn test_filter_by_cluster() {
    let v0 = seeded_unit_vector(30);
    let v1 = seeded_unit_vector(31);

    let entries = vec![
        make_entry_with_cluster("cluster-a-chunk", "cluster-a"),
        make_entry_with_cluster("cluster-b-chunk", "cluster-b"),
    ];
    let vectors = vec![v0.clone(), v1.clone()];

    let index = SearchIndex::build(entries, vectors).expect("build");

    let filter = SearchFilter {
        note_type: None,
        cluster: Some("cluster-a".to_string()),
        project: None,
        tags: None,
    };

    let hits = index.search(&v0, 10, Some(&filter));
    assert_eq!(hits.len(), 1, "expected exactly 1 hit from cluster-a");
    assert_eq!(hits[0].entry.chunk_id, "cluster-a-chunk");
}

#[test]
fn test_empty_index() {
    let index = SearchIndex::build(vec![], vec![]).expect("build empty");
    assert_eq!(index.entry_count(), 0);
    assert_eq!(index.file_count(), 0);

    let query = seeded_unit_vector(99);
    let hits = index.search(&query, 5, None);
    assert!(hits.is_empty(), "search on empty index should return empty");
}

#[test]
fn test_limit_respected() {
    let entries: Vec<IndexEntry> = (0..20)
        .map(|i| make_entry(&format!("chunk-{i}"), &format!("file-{i}.md"), "concept"))
        .collect();
    let vectors: Vec<Vec<f32>> = (0..20).map(|i| seeded_unit_vector(i + 100)).collect();

    let query = seeded_unit_vector(100); // same as vectors[0]

    let index = SearchIndex::build(entries, vectors).expect("build");
    assert_eq!(index.entry_count(), 20);

    let hits = index.search(&query, 5, None);
    assert_eq!(hits.len(), 5, "limit of 5 should return exactly 5 results");
}

#[test]
fn test_load_nonexistent_returns_error() {
    let tmp = TempDir::new().unwrap();
    // nothing written to the temp dir
    let result = SearchIndex::load(tmp.path());
    assert!(result.is_err(), "loading from empty dir should return Err");
}

#[test]
fn test_cached_vectors() {
    let v0 = seeded_unit_vector(50);
    let v1 = seeded_unit_vector(51);

    let entries = vec![
        make_entry("vec-chunk-0", "p.md", "concept"),
        make_entry("vec-chunk-1", "q.md", "session"),
    ];
    let vectors = vec![v0.clone(), v1.clone()];

    let index = SearchIndex::build(entries, vectors).expect("build");
    let cache: HashMap<String, Vec<f32>> = index.cached_vectors();

    assert_eq!(cache.len(), 2);
    assert!(cache.contains_key("vec-chunk-0"));
    assert!(cache.contains_key("vec-chunk-1"));

    let retrieved = &cache["vec-chunk-0"];
    let cosine: f32 = retrieved.iter().zip(v0.iter()).map(|(a, b)| a * b).sum();
    assert!(
        cosine > 0.999,
        "cached vector should match original (cosine={cosine})"
    );
}
