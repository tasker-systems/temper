//! HNSW index building — walks vault, embeds files, and writes
//! `.temper/index.hnsw.data` + `.temper/index.hnsw.graph` (the on-disk HNSW graph)
//! alongside a `.temper/index.json` sidecar manifest.
//!
//! Incremental: loads existing `.temper/index.json` and skips re-embedding files whose
//! (rel_path, content_hash) hasn't changed since the last run. Cached chunk embeddings
//! are reused — they're reinserted into the freshly-constructed HNSW so every query run
//! sees the full corpus, not just newly-changed files.
//!
//! Streaming + checkpointed: the sidecar manifest is flushed every
//! `CHECKPOINT_EVERY` files and at the end, so a kill mid-walk preserves progress.
//! The HNSW binary is dumped once at the end (hnsw_rs has no streaming append).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[cfg(feature = "hnsw")]
use hnsw_rs::prelude::{AnnT, DistCosine, Hnsw};

use crate::config::Config;
use crate::error::{Result, TemperError};

use super::index::IndexParams;
use super::index::IndexReport;

/// Doc types that live at `{vault}/{owner}/{context}/{doc_type}/`.
const ENTITY_DOC_TYPES: &[&str] = &["task", "goal", "session", "decision", "concept", "research"];

/// How many files between sidecar flushes during the walk. Keeps kill-resilience while
/// avoiding a write-per-file amplification on large vaults.
const CHECKPOINT_EVERY: usize = 25;

/// Model and vector dimension used when building the sidecar. Kept as module-level
/// constants so the checkpoint and final writes produce identical manifests.
const MODEL_NAME: &str = "BAAI/bge-base-en-v1.5";
const DIMENSION: usize = 768;

/// Sidecar manifest — written to `.temper/index.json` alongside the binary index.
/// Exposed as pub(crate) for use by graph_index module.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct IndexManifest {
    version: u8,
    run_at: String,
    model: String,
    dimension: usize,
    file_count: usize,
    files: Vec<FileEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct FileEntry {
    rel_path: String,
    content_hash: String,
    mtime_ns: u64,
    doc_embedding: Vec<f32>,
    chunks: Vec<ChunkEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ChunkEntry {
    index: usize,
    header_path: String,
    content_hash: String,
    vector_id: usize,
    /// Per-chunk embedding. Persisted so incremental runs can reinsert cached chunks
    /// into a freshly-constructed HNSW without re-embedding unchanged files.
    embedding: Vec<f32>,
}

/// Run the index build pipeline.
pub fn run(config: &Config, params: IndexParams, temper_dir: &PathBuf) -> Result<IndexReport> {
    // Use config's vault_root
    let vault_root = config.vault_root.clone();

    // Load existing manifest for incremental indexing. Cached entries carry chunk
    // embeddings that will be reinserted into the new HNSW below.
    let existing: HashMap<(String, String), ManifestFile> = load_existing_manifest(temper_dir)?;

    // Discover vault files — hardcode "@me" owner, walk all contexts
    let discovered = discover_vault(&vault_root, params.context_filter.as_deref());

    // Estimate max_elements for HNSW up-front: every cached chunk will be reinserted,
    // and each new file contributes at most ~25 chunks (safe over-estimate using the
    // whole discovered set as an upper bound on "to embed"). hnsw_rs treats
    // max_elements as a sizing hint, not a hard cap.
    let total_files = discovered.len();
    let existing_chunk_count: usize = existing.values().map(|f| f.chunks.len()).sum();
    let estimated_cached = existing.len().min(total_files);
    let to_embed_upper_bound = total_files.saturating_sub(estimated_cached);

    #[cfg(feature = "hnsw")]
    let estimated_max_elements = existing_chunk_count + total_files * 25;
    // `existing_chunk_count` is consumed via the estimate; silence unused-var when hnsw is off.
    #[cfg(not(feature = "hnsw"))]
    let _ = existing_chunk_count;

    tracing::info!(
        total_files,
        cached = estimated_cached,
        to_embed = to_embed_upper_bound,
        "starting index build"
    );

    // Construct the HNSW graph up-front so cached chunks and freshly-embedded chunks
    // feed into the same graph during the walk.
    #[cfg(feature = "hnsw")]
    let hnsw: Hnsw<f32, DistCosine> = Hnsw::new(
        24,                            // max_nb_connection
        estimated_max_elements.max(1), // max_elements (sizing hint)
        16,                            // max_layer — hnsw_rs file_dump requires NB_LAYER_MAX
        400,                           // ef_construction
        DistCosine {},
    );

    let mut report = IndexReport::default();
    let mut new_entries: Vec<FileEntry> = Vec::new();
    let mut vector_id_counter = 0usize;

    for (idx, discovered_file) in discovered.into_iter().enumerate() {
        let rel_path = discovered_file.rel_path.clone();
        let path = discovered_file.path.clone();

        tracing::info!(
            file = %rel_path,
            idx = idx + 1,
            total = total_files,
            "indexing file"
        );

        // Compute content hash from file body (not frontmatter-stripped)
        let (body_hash, mtime_ns) = match compute_body_hash(&path) {
            Ok((h, m)) => (h, m),
            Err(e) => {
                report.errors += 1;
                report.skipped_files.push(format!("{}: {}", rel_path, e));
                continue;
            }
        };

        // Unchanged file: reuse cached manifest entry AND reinsert its chunk embeddings
        // into the HNSW so incremental runs produce a graph covering the whole corpus.
        if let Some(existing_entry) = existing.get(&(rel_path.clone(), body_hash.clone())) {
            let mut reused_chunks: Vec<ChunkEntry> =
                Vec::with_capacity(existing_entry.chunks.len());
            for chunk in &existing_entry.chunks {
                let vid = vector_id_counter;
                vector_id_counter += 1;
                #[cfg(feature = "hnsw")]
                hnsw.insert((&chunk.embedding, vid));
                reused_chunks.push(ChunkEntry {
                    index: chunk.index,
                    header_path: chunk.header_path.clone(),
                    content_hash: chunk.content_hash.clone(),
                    vector_id: vid,
                    embedding: chunk.embedding.clone(),
                });
            }
            report.files_skipped += 1;
            new_entries.push(FileEntry {
                rel_path,
                content_hash: body_hash,
                mtime_ns,
                doc_embedding: existing_entry.doc_embedding.clone(),
                chunks: reused_chunks,
            });

            maybe_checkpoint(idx, temper_dir, &new_entries)?;
            continue;
        }

        // Changed/new file: read, strip frontmatter, chunk, embed.
        let raw = match fs::read_to_string(&path) {
            Ok(r) => r,
            Err(e) => {
                report.errors += 1;
                report
                    .skipped_files
                    .push(format!("{}: read error: {}", rel_path, e));
                continue;
            }
        };

        let body = match strip_frontmatter(&raw) {
            Ok(b) => b,
            Err(e) => {
                report.errors += 1;
                report
                    .skipped_files
                    .push(format!("{}: frontmatter parse error: {}", rel_path, e));
                continue;
            }
        };

        // Chunk the content
        let chunks = temper_ingest::chunk::chunk_markdown(&body);
        if chunks.is_empty() {
            report
                .skipped_files
                .push(format!("{}: no chunks", rel_path));
            report.files_skipped += 1;
            continue;
        }

        // Embed chunks
        let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
        let embeddings: Vec<Vec<f32>> = match temper_ingest::embed::embed_texts(&texts) {
            Ok(e) => e,
            Err(e) => {
                report.errors += 1;
                report
                    .skipped_files
                    .push(format!("{}: embed error: {}", rel_path, e));
                continue;
            }
        };

        // Compute doc_embedding = mean pool of all chunk embeddings
        let dim = embeddings.first().map(|v| v.len()).unwrap_or(0);
        let doc_embedding = if dim > 0 {
            let mut sum = vec![0f32; dim];
            for emb in &embeddings {
                for (i, v) in emb.iter().enumerate() {
                    sum[i] += v;
                }
            }
            let count = embeddings.len() as f32;
            sum.iter_mut().for_each(|v| *v /= count);
            sum
        } else {
            vec![]
        };

        // Build chunk entries — embedding is persisted inline so the next incremental
        // run can reinsert this chunk without re-embedding.
        let mut chunk_entries: Vec<ChunkEntry> = Vec::with_capacity(chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            let chunk_hash = sha256_hex(chunk.content.as_bytes());
            let vid = vector_id_counter;
            vector_id_counter += 1;
            let embedding = embeddings.get(i).cloned().unwrap_or_default();
            #[cfg(feature = "hnsw")]
            if !embedding.is_empty() {
                hnsw.insert((&embedding, vid));
            }
            chunk_entries.push(ChunkEntry {
                index: i,
                header_path: chunk.header_path.clone(),
                content_hash: chunk_hash,
                vector_id: vid,
                embedding,
            });
        }

        report.files_indexed += 1;

        new_entries.push(FileEntry {
            rel_path,
            content_hash: body_hash,
            mtime_ns,
            doc_embedding,
            chunks: chunk_entries,
        });

        maybe_checkpoint(idx, temper_dir, &new_entries)?;
    }

    // Final sidecar write — always happens, even when nothing was indexed, so the
    // manifest file reflects the current vault shape.
    write_sidecar(temper_dir, &new_entries)?;

    // Dump the HNSW graph to `.temper/index.hnsw.{data,graph}`.
    // Skip the dump when there's nothing to index (e.g. an empty vault).
    #[cfg(feature = "hnsw")]
    if !new_entries.is_empty() {
        dump_hnsw(&hnsw, temper_dir)?;
    }

    tracing::info!(
        files_indexed = report.files_indexed,
        files_skipped = report.files_skipped,
        "index build complete"
    );

    Ok(report)
}

/// Flush the sidecar manifest every `CHECKPOINT_EVERY` files. Called after each file
/// is processed. `idx` is 0-based so `(idx + 1) % N == 0` fires on the Nth, 2Nth, …
/// file rather than the first.
fn maybe_checkpoint(idx: usize, temper_dir: &Path, entries: &[FileEntry]) -> Result<()> {
    if (idx + 1).is_multiple_of(CHECKPOINT_EVERY) {
        write_sidecar(temper_dir, entries)?;
    }
    Ok(())
}

/// Write `.temper/index.json` from the current set of entries. Called at checkpoints
/// and at the end of the walk. Overwrites any existing manifest.
fn write_sidecar(temper_dir: &Path, entries: &[FileEntry]) -> Result<()> {
    let manifest = IndexManifest {
        version: 1,
        run_at: chrono::Utc::now().to_rfc3339(),
        model: MODEL_NAME.to_string(),
        dimension: DIMENSION,
        file_count: entries.len(),
        files: entries.to_vec(),
    };

    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| TemperError::Project(format!("serialize index manifest: {e}")))?;
    fs::write(temper_dir.join("index.json"), manifest_json)
        .map_err(|e| TemperError::Project(format!("write index.json: {e}")))?;
    Ok(())
}

/// Dump the already-populated HNSW graph to disk as
/// `{temper_dir}/index.hnsw.data` + `{temper_dir}/index.hnsw.graph`.
///
/// `hnsw_rs::file_dump` refuses to overwrite existing files — it instead generates a
/// random-suffixed basename — so we explicitly remove any stale dump files first.
#[cfg(feature = "hnsw")]
fn dump_hnsw(hnsw: &Hnsw<f32, DistCosine>, temper_dir: &Path) -> Result<()> {
    // Remove any stale dump so file_dump writes the expected basename rather than appending a
    // random suffix to avoid collisions.
    for ext in ["hnsw.data", "hnsw.graph"] {
        let p = temper_dir.join(format!("index.{ext}"));
        if p.exists() {
            fs::remove_file(&p)
                .map_err(|e| TemperError::Project(format!("remove {}: {e}", p.display())))?;
        }
    }

    hnsw.file_dump(temper_dir, "index")
        .map_err(|e| TemperError::Project(format!("hnsw dump: {e}")))?;

    Ok(())
}

/// Load existing manifest, returning map of (rel_path, content_hash) -> entry.
fn load_existing_manifest(temper_dir: &PathBuf) -> Result<HashMap<(String, String), ManifestFile>> {
    let path = temper_dir.join("index.json");
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| TemperError::Project(format!("read existing index.json: {e}")))?;
    let manifest: IndexManifest = serde_json::from_str(&raw)
        .map_err(|e| TemperError::Project(format!("parse index.json: {e}")))?;
    let mut map = HashMap::new();
    for f in manifest.files {
        map.insert(
            (f.rel_path.clone(), f.content_hash.clone()),
            ManifestFile {
                rel_path: f.rel_path,
                content_hash: f.content_hash,
                doc_embedding: f.doc_embedding,
                chunks: f.chunks,
            },
        );
    }
    Ok(map)
}

#[derive(Debug, Clone)]
#[expect(dead_code)]
struct ManifestFile {
    rel_path: String,
    content_hash: String,
    doc_embedding: Vec<f32>,
    chunks: Vec<ChunkEntry>,
}

/// Discover all vault markdown files, partitioned by owner/context.
/// Exposed as pub(crate) for use by graph_index module.
pub(crate) fn discover_vault(
    vault_root: &PathBuf,
    context_filter: Option<&str>,
) -> Vec<DiscoveredFile> {
    let mut files = Vec::new();
    let owner = "@me";

    let contexts_to_walk: Vec<(PathBuf, String)> = if let Some(ctx) = context_filter {
        let path = vault_root.join(owner).join(ctx);
        if path.exists() {
            vec![(path, ctx.to_string())]
        } else {
            vec![]
        }
    } else {
        let owner_root = vault_root.join(owner);
        let mut result = Vec::new();
        if let Ok(entries) = fs::read_dir(&owner_root) {
            for entry in entries.filter_map(|e| e.ok()) {
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    if let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) {
                        if !name.starts_with('.') {
                            result.push((entry_path.clone(), name.to_string()));
                        }
                    }
                }
            }
        }
        result
    };

    for (ctx_root, context) in contexts_to_walk {
        let context_owned = context.clone();
        for doc_type in ENTITY_DOC_TYPES {
            let type_dir = ctx_root.join(doc_type);
            if !type_dir.exists() {
                continue;
            }
            if let Ok(entries) = fs::read_dir(&type_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("md") {
                        let rel_path = format!(
                            "{}/{}/{}/{}",
                            owner,
                            context_owned,
                            doc_type,
                            path.file_name().unwrap().to_str().unwrap()
                        );

                        // Verify it's parseable as markdown with frontmatter
                        if let Ok(raw) = fs::read_to_string(&path) {
                            let frontmatter =
                                temper_core::frontmatter::Frontmatter::try_from(raw.as_str());
                            if frontmatter.is_err() {
                                continue;
                            }
                            files.push(DiscoveredFile {
                                path,
                                rel_path,
                                owner: owner.to_string(),
                                context: context_owned.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

    files
}

/// Strip YAML frontmatter from markdown, returning just the body.
fn strip_frontmatter(raw: &str) -> Result<String> {
    let mut lines = raw.lines();

    // Check for opening `---`
    if lines.next() != Some("---") {
        return Ok(raw.to_string());
    }

    let mut frontmatter_lines = Vec::new();
    for line in lines.by_ref() {
        if line == "---" {
            break;
        }
        frontmatter_lines.push(line);
    }

    // Parse frontmatter to verify it's valid YAML
    let frontmatter_str = frontmatter_lines.join("\n");
    let _: serde_yaml::Value = serde_yaml::from_str(&frontmatter_str)
        .map_err(|e| TemperError::Project(format!("frontmatter parse error: {e}")))?;

    Ok(lines.collect::<Vec<_>>().join("\n").trim().to_string())
}

/// Compute SHA256 hash of the raw file bytes and return mtime in nanoseconds.
fn compute_body_hash(path: &PathBuf) -> Result<(String, u64)> {
    let metadata =
        fs::metadata(path).map_err(|e| TemperError::Project(format!("metadata: {e}")))?;
    let mtime_ns = metadata
        .modified()
        .map_err(|e| TemperError::Project(format!("mtime: {e}")))?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| TemperError::Project(format!("mtime duration: {e}")))?
        .as_nanos() as u64;

    let bytes = fs::read(path).map_err(|e| TemperError::Project(format!("read: {e}")))?;
    let hash = sha256_hex(&bytes);

    Ok((hash, mtime_ns))
}

/// Compute SHA256 hex of a byte slice.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[expect(dead_code)]
pub(crate) struct DiscoveredFile {
    path: PathBuf,
    pub(crate) rel_path: String,
    owner: String,
    context: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `discover_vault` must preserve the doctype segment in `rel_path` so that
    /// downstream consumers (graph-index materialize, clustering) can locate the
    /// file by rel_path alone. Previously the rel_path was
    /// `@me/{context}/{file}` which dropped the doctype, breaking `vault_root.join(rel_path)`.
    #[test]
    fn test_discover_vault_rel_path_includes_doctype() {
        let tmp = tempfile::tempdir().unwrap();
        let task_dir = tmp.path().join("@me").join("temper").join("task");
        fs::create_dir_all(&task_dir).unwrap();

        let doc = "---\n\
temper-id: \"01900000-0000-7000-8000-000000000001\"\n\
temper-type: task\n\
temper-context: temper\n\
temper-created: \"2026-01-01T00:00:00Z\"\n\
temper-owner: \"@me\"\n\
title: \"Foo\"\n\
temper-stage: backlog\n\
slug: foo\n\
---\n\
\n\
body\n";
        fs::write(task_dir.join("foo.md"), doc).unwrap();

        let discovered = discover_vault(&tmp.path().to_path_buf(), None);
        assert_eq!(discovered.len(), 1, "one file discovered");
        assert_eq!(
            discovered[0].rel_path, "@me/temper/task/foo.md",
            "rel_path preserves doctype segment"
        );
    }
}
