//! HNSW index building — walks vault, embeds files, writes `.temper/index.json` sidecar.
//!
//! Incremental: loads existing `.temper/index.json` and skips files whose (rel_path, content_hash)
//! hasn't changed since the last run. Actual HNSW index write is TODO — depends on hnsw_rs API.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::error::{Result, TemperError};

use super::index::IndexParams;
use super::index::IndexReport;

/// Doc types that live at `{vault}/{owner}/{context}/{doc_type}/`.
const ENTITY_DOC_TYPES: &[&str] = &["task", "goal", "session", "decision", "concept", "research"];

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
}

/// Run the index build pipeline.
pub fn run(config: &Config, params: IndexParams, temper_dir: &PathBuf) -> Result<IndexReport> {
    // Use config's vault_root
    let vault_root = config.vault_root.clone();

    // Load existing manifest for incremental indexing
    let existing: HashMap<(String, String), ManifestFile> = load_existing_manifest(temper_dir)?;

    // Discover vault files — hardcode "@me" owner, walk all contexts
    let discovered = discover_vault(&vault_root, params.context_filter.as_deref());

    let mut report = IndexReport::default();
    let mut new_entries: Vec<FileEntry> = Vec::new();
    let mut vector_id_counter = 0usize;

    // TODO: Initialize HNSW index once hnsw_rs is available
    // let hnsw_dir = temper_dir.join("index.bin");
    // let mut hnsw_index = hnsw_rs::Index::new(dim: 768, ...);

    for discovered_file in discovered {
        let rel_path = discovered_file.rel_path.clone();
        let path = discovered_file.path.clone();

        // Compute content hash from file body (not frontmatter-stripped)
        let (body_hash, mtime_ns) = match compute_body_hash(&path) {
            Ok((h, m)) => (h, m),
            Err(e) => {
                report.errors += 1;
                report.skipped_files.push(format!("{}: {}", rel_path, e));
                continue;
            }
        };

        // Skip unchanged files
        if let Some(existing_entry) = existing.get(&(rel_path.clone(), body_hash.clone())) {
            report.files_skipped += 1;
            new_entries.push(FileEntry {
                rel_path,
                content_hash: body_hash,
                mtime_ns,
                doc_embedding: existing_entry.doc_embedding.clone(),
                chunks: existing_entry.chunks.clone(),
            });
            continue;
        }

        // Read and strip frontmatter
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

        // Build chunk entries
        let chunk_entries: Vec<ChunkEntry> = chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| {
                let chunk_hash = sha256_hex(chunk.content.as_bytes());
                let vid = vector_id_counter;
                vector_id_counter += 1;
                ChunkEntry {
                    index: i,
                    header_path: chunk.header_path.clone(),
                    content_hash: chunk_hash,
                    vector_id: vid,
                }
            })
            .collect();

        // TODO: Add chunk embeddings to HNSW index
        // for emb in embeddings {
        //     hnsw_index.add_vector(emb, vector_id);
        // }

        report.files_indexed += 1;

        new_entries.push(FileEntry {
            rel_path,
            content_hash: body_hash,
            mtime_ns,
            doc_embedding,
            chunks: chunk_entries,
        });
    }

    // Write HNSW index
    // TODO: hnsw_index.write_index(temper_dir.join("index.bin"))?;

    // Write sidecar manifest
    let manifest = IndexManifest {
        version: 1,
        run_at: chrono::Utc::now().to_rfc3339(),
        model: "BAAI/bge-base-en-v1.5".to_string(),
        dimension: 768,
        file_count: new_entries.len(),
        files: new_entries,
    };

    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| TemperError::Project(format!("serialize index manifest: {e}")))?;
    fs::write(temper_dir.join("index.json"), manifest_json)
        .map_err(|e| TemperError::Project(format!("write index.json: {e}")))?;

    Ok(report)
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
                            "{}/{}/{}",
                            owner,
                            context_owned,
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
    rel_path: String,
    owner: String,
    context: String,
}
