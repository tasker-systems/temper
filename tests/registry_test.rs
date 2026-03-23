use temper_cli::registry::{compute_file_hash, FileRecord, FileSource, Registry};
use std::collections::HashMap;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// test_new_registry
// ---------------------------------------------------------------------------

#[test]
fn test_new_registry() {
    let reg = Registry::new();
    assert_eq!(reg.version, 1);
    assert!(reg.files.is_empty());
}

// ---------------------------------------------------------------------------
// test_save_and_load_round_trip
// ---------------------------------------------------------------------------

#[test]
fn test_save_and_load_round_trip() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path();

    let mut reg = Registry::new();
    reg.files.insert(
        "concepts/test.md".to_string(),
        FileRecord {
            content_hash: "abc123".to_string(),
            chunk_ids: vec!["chunk-0".to_string(), "chunk-1".to_string()],
            source: FileSource::Vault,
            last_indexed: "2026-01-01T00:00:00Z".to_string(),
        },
    );

    reg.save(state_dir).expect("save should succeed");
    let loaded = Registry::load(state_dir).expect("load should succeed");

    assert_eq!(loaded.version, 1);
    assert_eq!(loaded.files.len(), 1);
    let rec = loaded.files.get("concepts/test.md").expect("record should exist");
    assert_eq!(rec.content_hash, "abc123");
    assert_eq!(rec.chunk_ids, vec!["chunk-0", "chunk-1"]);
    matches!(rec.source, FileSource::Vault);
}

// ---------------------------------------------------------------------------
// test_load_missing_returns_empty
// ---------------------------------------------------------------------------

#[test]
fn test_load_missing_returns_empty() {
    let tmp = TempDir::new().unwrap();
    // Nothing written — no registry.json
    let reg = Registry::load(tmp.path()).expect("load missing should return empty, not error");
    assert_eq!(reg.version, 1);
    assert!(reg.files.is_empty());
}

// ---------------------------------------------------------------------------
// test_compute_hash
// ---------------------------------------------------------------------------

#[test]
fn test_compute_hash() {
    let tmp = TempDir::new().unwrap();
    let file_path = tmp.path().join("content.txt");

    std::fs::write(&file_path, b"hello world").unwrap();
    let h1 = compute_file_hash(&file_path).expect("hash should succeed");
    let h2 = compute_file_hash(&file_path).expect("second hash should succeed");
    assert_eq!(h1, h2, "same content should produce same hash");

    std::fs::write(&file_path, b"different content").unwrap();
    let h3 = compute_file_hash(&file_path).expect("hash after change should succeed");
    assert_ne!(h1, h3, "changed content should produce different hash");
}

// ---------------------------------------------------------------------------
// test_diff_new_file
// ---------------------------------------------------------------------------

#[test]
fn test_diff_new_file() {
    let reg = Registry::new();
    let current = vec![("concepts/new.md".to_string(), "deadbeef".to_string())];
    let diff = reg.diff(&current);

    assert_eq!(diff.new_files.len(), 1);
    assert_eq!(diff.new_files[0], "concepts/new.md");
    assert!(diff.changed_files.is_empty());
    assert!(diff.deleted_files.is_empty());
    assert!(diff.unchanged_files.is_empty());
}

// ---------------------------------------------------------------------------
// test_diff_changed_file
// ---------------------------------------------------------------------------

#[test]
fn test_diff_changed_file() {
    let mut reg = Registry::new();
    reg.files.insert(
        "concepts/changed.md".to_string(),
        FileRecord {
            content_hash: "old-hash".to_string(),
            chunk_ids: vec![],
            source: FileSource::Vault,
            last_indexed: "2026-01-01T00:00:00Z".to_string(),
        },
    );

    let current = vec![("concepts/changed.md".to_string(), "new-hash".to_string())];
    let diff = reg.diff(&current);

    assert!(diff.new_files.is_empty());
    assert_eq!(diff.changed_files.len(), 1);
    assert_eq!(diff.changed_files[0], "concepts/changed.md");
    assert!(diff.deleted_files.is_empty());
    assert!(diff.unchanged_files.is_empty());
}

// ---------------------------------------------------------------------------
// test_diff_deleted_file
// ---------------------------------------------------------------------------

#[test]
fn test_diff_deleted_file() {
    let mut reg = Registry::new();
    reg.files.insert(
        "concepts/deleted.md".to_string(),
        FileRecord {
            content_hash: "some-hash".to_string(),
            chunk_ids: vec![],
            source: FileSource::Vault,
            last_indexed: "2026-01-01T00:00:00Z".to_string(),
        },
    );

    // current_files is empty — file was deleted
    let diff = reg.diff(&[]);

    assert!(diff.new_files.is_empty());
    assert!(diff.changed_files.is_empty());
    assert_eq!(diff.deleted_files.len(), 1);
    assert_eq!(diff.deleted_files[0], "concepts/deleted.md");
    assert!(diff.unchanged_files.is_empty());
}

// ---------------------------------------------------------------------------
// test_diff_unchanged_file
// ---------------------------------------------------------------------------

#[test]
fn test_diff_unchanged_file() {
    let mut reg = Registry::new();
    reg.files.insert(
        "concepts/same.md".to_string(),
        FileRecord {
            content_hash: "stable-hash".to_string(),
            chunk_ids: vec![],
            source: FileSource::Vault,
            last_indexed: "2026-01-01T00:00:00Z".to_string(),
        },
    );

    let current = vec![("concepts/same.md".to_string(), "stable-hash".to_string())];
    let diff = reg.diff(&current);

    assert!(diff.new_files.is_empty());
    assert!(diff.changed_files.is_empty());
    assert!(diff.deleted_files.is_empty());
    assert_eq!(diff.unchanged_files.len(), 1);
    assert_eq!(diff.unchanged_files[0], "concepts/same.md");
}

// ---------------------------------------------------------------------------
// test_external_source_tracking
// ---------------------------------------------------------------------------

#[test]
fn test_external_source_tracking() {
    let mut reg = Registry::new();
    // An external file referenced by "concepts/parent.md"
    reg.files.insert(
        "/external/path/doc.md".to_string(),
        FileRecord {
            content_hash: "ext-hash".to_string(),
            chunk_ids: vec![],
            source: FileSource::External {
                referenced_by: "concepts/parent.md".to_string(),
            },
            last_indexed: "2026-01-01T00:00:00Z".to_string(),
        },
    );

    // When the referencing vault note exists — not an orphan
    let vault_files = vec!["concepts/parent.md".to_string()];
    let orphans = reg.find_orphaned_externals(&vault_files);
    assert!(orphans.is_empty(), "referencing note exists — should not be orphan");

    // When the referencing vault note is gone — is an orphan
    let vault_files_empty: Vec<String> = vec![];
    let orphans = reg.find_orphaned_externals(&vault_files_empty);
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0], "/external/path/doc.md");
}

// ---------------------------------------------------------------------------
// test_atomic_save
// ---------------------------------------------------------------------------

#[test]
fn test_atomic_save() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path();

    let reg = Registry::new();
    reg.save(state_dir).expect("save should succeed");

    // Verify no .tmp files remain in state dir
    let tmp_files: Vec<_> = std::fs::read_dir(state_dir)
        .expect("state dir should exist")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "tmp")
                .unwrap_or(false)
        })
        .collect();

    assert!(
        tmp_files.is_empty(),
        "no .tmp files should remain after atomic save, found: {:?}",
        tmp_files.iter().map(|e| e.path()).collect::<Vec<_>>()
    );

    // Also verify the registry.json itself was created
    assert!(
        state_dir.join("registry.json").exists(),
        "registry.json should exist after save"
    );
}

// ---------------------------------------------------------------------------
// Unused import suppression helper — keeps the HashMap import used
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn _uses_hashmap(_: HashMap<String, String>) {}
