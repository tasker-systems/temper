// temper index — incremental embed pipeline

use std::collections::HashMap;

use crate::config::Config;
use crate::error::Result;
use crate::hnsw::{IndexEntry, SearchIndex};
use crate::registry::{FileRecord, FileSource, Registry};
use crate::{chunker, embedder, registry, vault};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn run(
    config: &Config,
    force: bool,
    paths_filter: Option<&str>,
    sources_override: Option<&str>,
) -> Result<()> {
    let state_dir = &config.state_dir;

    // --- 1. Collect vault files from essential dirs ---
    let mut vault_files: Vec<std::path::PathBuf> = Vec::new();

    for dir in &[&config.sessions_dir, &config.tickets_dir, &config.milestones_dir] {
        if dir.exists() {
            vault::collect_md_files_recursive(dir, &mut vault_files)?;
        }
    }

    // Collect from index.include (relative to vault_root)
    for include_path in &config.index_include {
        let dir = config.vault_root.join(include_path);
        if dir.exists() {
            vault::collect_md_files_recursive(&dir, &mut vault_files)?;
        }
    }

    // --- 2. Collect external source files ---
    let source_paths: Vec<std::path::PathBuf> = match sources_override {
        Some(s) => s
            .split(',')
            .map(|p| std::path::PathBuf::from(p.trim()))
            .collect(),
        None => config.index_sources.clone(),
    };

    let mut external_files: Vec<std::path::PathBuf> = Vec::new();
    for source_path in &source_paths {
        if source_path.is_dir() {
            vault::collect_md_files_recursive(source_path, &mut external_files)?;
        } else if source_path.is_file() {
            external_files.push(source_path.clone());
        }
    }

    // Combine all files
    let all_files: Vec<std::path::PathBuf> = vault_files
        .iter()
        .chain(external_files.iter())
        .cloned()
        .collect();

    // --- 3. Apply --paths filter if provided ---
    let paths_filter_list: Option<Vec<&str>> =
        paths_filter.map(|p| p.split(',').collect());
    let all_files: Vec<std::path::PathBuf> = if let Some(ref filter) = paths_filter_list {
        all_files
            .into_iter()
            .filter(|f| {
                let s = f.to_string_lossy();
                filter.iter().any(|pat| s.contains(pat.trim()))
            })
            .collect()
    } else {
        all_files
    };

    // Apply index.exclude filter
    let all_files: Vec<std::path::PathBuf> = if config.index_exclude.is_empty() {
        all_files
    } else {
        all_files
            .into_iter()
            .filter(|f| {
                let s = f.to_string_lossy();
                !config
                    .index_exclude
                    .iter()
                    .any(|excl| s.contains(excl.as_str()))
            })
            .collect()
    };

    eprintln!("Collecting files: {} total", all_files.len());

    // --- 4. Compute hashes and build (path, hash) pairs ---
    let mut file_hashes: Vec<(String, String)> = Vec::new();
    for file in &all_files {
        let rel = file
            .strip_prefix(&config.vault_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| file.to_string_lossy().to_string());
        match registry::compute_file_hash(file) {
            Ok(hash) => file_hashes.push((rel, hash)),
            Err(e) => eprintln!("  Warning: could not hash {}: {e}", file.display()),
        }
    }

    // --- 5. Load registry ---
    let mut reg = Registry::load(state_dir)?;

    // --- 6. Determine which files need (re-)embedding ---
    let (files_to_embed, unchanged_paths): (Vec<String>, Vec<String>) =
        if force && paths_filter.is_none() {
            // Force without filter: treat everything as new
            let all_paths: Vec<String> = file_hashes.iter().map(|(p, _)| p.clone()).collect();
            (all_paths, Vec::new())
        } else if force && paths_filter.is_some() {
            // Force with filter: treat filtered files as changed, others as unchanged
            let diff = reg.diff(&file_hashes);
            let changed: Vec<String> = diff
                .changed_files
                .into_iter()
                .chain(diff.new_files)
                .collect();
            (changed, diff.unchanged_files)
        } else {
            // Normal: use registry diff
            let diff = reg.diff(&file_hashes);
            let to_embed: Vec<String> = diff
                .new_files
                .into_iter()
                .chain(diff.changed_files)
                .collect();
            (to_embed, diff.unchanged_files)
        };

    eprintln!(
        "Files to embed: {} new/changed, {} unchanged",
        files_to_embed.len(),
        unchanged_paths.len()
    );

    // --- 7. Load cached vectors for unchanged files ---
    let cached_vectors: HashMap<String, Vec<f32>> = if unchanged_paths.is_empty() {
        HashMap::new()
    } else {
        match SearchIndex::load(state_dir) {
            Ok(existing_index) => existing_index.cached_vectors(),
            Err(_) => {
                eprintln!("  Note: no existing index found; all files will be re-embedded");
                HashMap::new()
            }
        }
    };

    // Build path → hash lookup for registry updates
    let hash_lookup: HashMap<String, String> = file_hashes.into_iter().collect();

    // --- 8. Process unchanged files (reuse cached vectors) ---
    let mut all_entries: Vec<IndexEntry> = Vec::new();
    let mut all_vectors: Vec<Vec<f32>> = Vec::new();
    let mut files_reused = 0usize;
    let mut chunks_reused = 0usize;

    for rel_path in &unchanged_paths {
        if let Some(record) = reg.files.get(rel_path) {
            let chunk_ids = &record.chunk_ids;
            let all_found = chunk_ids.iter().all(|id| cached_vectors.contains_key(id));
            if all_found {
                let full_path = if std::path::Path::new(rel_path).is_absolute() {
                    std::path::PathBuf::from(rel_path)
                } else {
                    config.vault_root.join(rel_path)
                };
                match std::fs::read_to_string(&full_path) {
                    Ok(content) => {
                        let chunks = chunker::chunk_document(rel_path, &content);
                        for chunk in &chunks {
                            if let Some(vec) = cached_vectors.get(&chunk.id) {
                                all_entries.push(IndexEntry {
                                    chunk_id: chunk.id.clone(),
                                    file_path: chunk.file_path.clone(),
                                    chunk_index: chunk.chunk_index,
                                    header_path: chunk.header_path.clone(),
                                    content: chunk.content.clone(),
                                    metadata: chunk.metadata.clone(),
                                });
                                all_vectors.push(vec.clone());
                                chunks_reused += 1;
                            }
                        }
                        files_reused += 1;
                    }
                    Err(e) => {
                        eprintln!("  Warning: could not read {rel_path}: {e}");
                    }
                }
            }
        }
    }

    // --- 9. Embed new/changed files ---
    let mut embedder = embedder::Embedder::new(config.model_cache_dir.clone());
    let total_to_embed = files_to_embed.len();
    let mut files_embedded = 0usize;
    let mut chunks_embedded = 0usize;
    let mut registry_updates: Vec<(String, FileRecord)> = Vec::new();

    // Build a set of external file paths for source classification
    let external_paths: std::collections::HashSet<String> = external_files
        .iter()
        .map(|f| f.to_string_lossy().to_string())
        .collect();

    for (i, rel_path) in files_to_embed.iter().enumerate() {
        let full_path = if std::path::Path::new(rel_path).is_absolute() {
            std::path::PathBuf::from(rel_path)
        } else {
            config.vault_root.join(rel_path)
        };

        // Size check
        if let Ok(meta) = std::fs::metadata(&full_path) {
            if meta.len() > 1_000_000 {
                eprintln!("  Note: large file ({}): {}", meta.len(), rel_path);
            }
        }

        // Read content
        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  Warning: skipping {rel_path} (read error: {e})");
                continue;
            }
        };

        eprint!("  [{}/{}] {} ", i + 1, total_to_embed, rel_path);

        let chunks = chunker::chunk_document(rel_path, &content);
        if chunks.is_empty() {
            eprintln!("— skipped (no chunks)");
            continue;
        }

        match embed_chunks(&mut embedder, &chunks) {
            Ok(vecs) => {
                eprintln!("— {} chunks", chunks.len());
                let chunk_ids: Vec<String> = chunks.iter().map(|c| c.id.clone()).collect();

                // Determine file source
                let full_path_str = full_path.to_string_lossy().to_string();
                let source = if external_paths.contains(&full_path_str) {
                    FileSource::External {
                        referenced_by: rel_path.clone(),
                    }
                } else {
                    FileSource::Vault
                };

                let hash = hash_lookup.get(rel_path).cloned().unwrap_or_default();
                registry_updates.push((
                    rel_path.clone(),
                    FileRecord {
                        content_hash: hash,
                        chunk_ids,
                        source,
                        last_indexed: chrono::Utc::now().to_rfc3339(),
                    },
                ));

                for (chunk, vec) in chunks.into_iter().zip(vecs.into_iter()) {
                    all_entries.push(IndexEntry {
                        chunk_id: chunk.id.clone(),
                        file_path: chunk.file_path.clone(),
                        chunk_index: chunk.chunk_index,
                        header_path: chunk.header_path.clone(),
                        content: chunk.content.clone(),
                        metadata: chunk.metadata.clone(),
                    });
                    all_vectors.push(vec);
                    chunks_embedded += 1;
                }
                files_embedded += 1;
            }
            Err(e) => {
                eprintln!("— ERROR: {e}");
            }
        }
    }

    // --- 10. Build fresh HNSW index from ALL entries (cached + new) ---
    eprintln!();
    eprintln!(
        "Building HNSW index: {} total chunks ({} reused, {} new)…",
        all_entries.len(),
        chunks_reused,
        chunks_embedded
    );
    let index = SearchIndex::build(all_entries, all_vectors)?;
    index.save(state_dir)?;

    // --- 11. Update registry ---
    for (path, record) in registry_updates {
        reg.files.insert(path, record);
    }

    // Remove deleted files
    let current_paths: Vec<String> = unchanged_paths
        .iter()
        .chain(files_to_embed.iter())
        .cloned()
        .collect();
    let diff_for_delete = reg.diff(
        &current_paths
            .iter()
            .map(|p| {
                let hash = hash_lookup.get(p).cloned().unwrap_or_default();
                (p.clone(), hash)
            })
            .collect::<Vec<_>>(),
    );
    for deleted in &diff_for_delete.deleted_files {
        reg.files.remove(deleted);
    }

    // Clean orphaned externals
    let orphaned = reg.find_orphaned_externals(&current_paths);
    for orphan in &orphaned {
        reg.files.remove(orphan);
    }

    reg.last_indexed = chrono::Utc::now().to_rfc3339();
    reg.save(state_dir)?;

    // --- 12. Print summary ---
    println!();
    println!(
        "Index complete: {} files processed ({} embedded, {} reused from cache), {} chunks total.",
        files_embedded + files_reused,
        files_embedded,
        files_reused,
        chunks_embedded + chunks_reused
    );
    println!("Index saved to: {}", state_dir.join("index.bin").display());
    println!(
        "Registry saved to: {}",
        state_dir.join("registry.json").display()
    );
    if !orphaned.is_empty() {
        println!("Cleaned {} orphaned external entries.", orphaned.len());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn embed_chunks(
    embedder: &mut embedder::Embedder,
    chunks: &[chunker::Chunk],
) -> Result<Vec<Vec<f32>>> {
    let texts: Vec<String> = chunks
        .iter()
        .map(|c| {
            if c.chunk_index == 0 && c.header_path.is_empty() {
                embedder::preprocess_frontmatter(&c.content)
            } else {
                embedder::preprocess_chunk(&c.content, &c.header_path)
            }
        })
        .collect();
    let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    embedder.embed_batch(&text_refs)
}
