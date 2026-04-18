//! `temper index` action — orchestrates HNSW index build.
//!
//! Parameters and report types for the index build pipeline.

use serde::{Deserialize, Serialize};

/// Parameters for an index build run.
#[derive(Debug, Clone)]
pub struct IndexParams {
    /// Optional single-context filter. None means all configured contexts.
    pub context_filter: Option<String>,
    /// If true, delete existing index and do a full rebuild.
    pub full: bool,
}

/// Final report from an index build run.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct IndexReport {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub skipped_files: Vec<String>,
    pub errors: usize,
}

/// Run the index build pipeline.
pub fn run(
    config: &crate::config::Config,
    params: IndexParams,
) -> crate::error::Result<IndexReport> {
    let vault_root = &config.vault_root;
    let temper_dir = vault_root.join(".temper");

    // Handle --full rebuild: delete the HNSW dump (data + graph) and the sidecar manifest.
    if params.full {
        for name in ["index.hnsw.data", "index.hnsw.graph", "index.json"] {
            let p = temper_dir.join(name);
            if p.exists() {
                std::fs::remove_file(&p).map_err(|e| {
                    crate::error::TemperError::Project(format!("remove {name}: {e}"))
                })?;
            }
        }
    }

    // Ensure .temper/ directory exists
    if !temper_dir.exists() {
        std::fs::create_dir_all(&temper_dir)
            .map_err(|e| crate::error::TemperError::Project(format!("create .temper dir: {e}")))?;
    }

    crate::actions::index_build::run(config, params, &temper_dir)
}
