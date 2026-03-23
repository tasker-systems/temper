// Incremental index registry

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, TemperError};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct Registry {
    pub version: u32,
    pub last_indexed: String,
    pub files: HashMap<String, FileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub content_hash: String,
    pub chunk_ids: Vec<String>,
    pub source: FileSource,
    pub last_indexed: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FileSource {
    #[serde(rename = "vault")]
    Vault,
    #[serde(rename = "external")]
    External { referenced_by: String },
}

pub struct RegistryDiff {
    pub new_files: Vec<String>,
    pub changed_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub unchanged_files: Vec<String>,
}

// ---------------------------------------------------------------------------
// Registry impl
// ---------------------------------------------------------------------------

impl Registry {
    /// Create a new empty registry with version 1.
    pub fn new() -> Self {
        Self {
            version: 1,
            last_indexed: Utc::now().to_rfc3339(),
            files: HashMap::new(),
        }
    }

    /// Load the registry from `<state_dir>/registry.json`.
    /// Returns an empty registry if the file does not exist.
    pub fn load(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join("registry.json");
        if !path.exists() {
            return Ok(Self::new());
        }
        let data = std::fs::read_to_string(&path)?;
        let reg: Self =
            serde_json::from_str(&data).map_err(|e| TemperError::Index(e.to_string()))?;
        Ok(reg)
    }

    /// Atomically save the registry to `<state_dir>/registry.json`.
    /// Writes to a `.tmp` file first, then renames to the final path.
    pub fn save(&self, state_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(state_dir)?;

        let final_path = state_dir.join("registry.json");
        let tmp_path = state_dir.join("registry.json.tmp");

        let json =
            serde_json::to_string_pretty(self).map_err(|e| TemperError::Index(e.to_string()))?;

        {
            let mut f = std::fs::File::create(&tmp_path)?;
            f.write_all(json.as_bytes())?;
            f.flush()?;
        }

        std::fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    /// Compare the given `(path, hash)` pairs against the registry and return a diff.
    pub fn diff(&self, current_files: &[(String, String)]) -> RegistryDiff {
        let mut new_files = Vec::new();
        let mut changed_files = Vec::new();
        let mut unchanged_files = Vec::new();

        for (path, hash) in current_files {
            match self.files.get(path) {
                None => new_files.push(path.clone()),
                Some(rec) if rec.content_hash != *hash => changed_files.push(path.clone()),
                Some(_) => unchanged_files.push(path.clone()),
            }
        }

        // Any path in the registry not present in current_files has been deleted
        let current_set: std::collections::HashSet<&str> =
            current_files.iter().map(|(p, _)| p.as_str()).collect();
        let deleted_files: Vec<String> = self
            .files
            .keys()
            .filter(|p| !current_set.contains(p.as_str()))
            .cloned()
            .collect();

        RegistryDiff {
            new_files,
            changed_files,
            deleted_files,
            unchanged_files,
        }
    }

    /// Return paths of external files whose referencing vault note no longer exists.
    pub fn find_orphaned_externals(&self, existing_vault_files: &[String]) -> Vec<String> {
        let vault_set: std::collections::HashSet<&str> =
            existing_vault_files.iter().map(|s| s.as_str()).collect();

        self.files
            .iter()
            .filter_map(|(path, rec)| {
                if let FileSource::External { referenced_by } = &rec.source {
                    if !vault_set.contains(referenced_by.as_str()) {
                        return Some(path.clone());
                    }
                }
                None
            })
            .collect()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Free function
// ---------------------------------------------------------------------------

/// Compute the SHA-256 hex digest of the file at `path`.
pub fn compute_file_hash(path: &Path) -> Result<String> {
    let data = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let digest = hasher.finalize();
    Ok(format!("{digest:x}"))
}
