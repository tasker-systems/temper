use std::path::Path;

use temper_core::types::Manifest;

/// Load manifest from `<temper_dir>/manifest.json`.
/// Returns a new empty manifest if the file doesn't exist.
pub fn load_manifest(temper_dir: &Path, device_id: &str) -> crate::error::Result<Manifest> {
    let path = temper_dir.join("manifest.json");
    if !path.exists() {
        return Ok(Manifest::new(device_id.to_string()));
    }
    let content = std::fs::read_to_string(&path)?;
    // Fall back to a fresh manifest if the file is empty or has an
    // incompatible schema (e.g. bare `{}` from vault init).
    match serde_json::from_str::<Manifest>(&content) {
        Ok(manifest) => Ok(manifest),
        Err(_) => Ok(Manifest::new(device_id.to_string())),
    }
}

/// Save manifest to `<temper_dir>/manifest.json`.
/// Creates the directory if it doesn't exist.
pub fn save_manifest(temper_dir: &Path, manifest: &Manifest) -> crate::error::Result<()> {
    std::fs::create_dir_all(temper_dir)?;
    let path = temper_dir.join("manifest.json");
    let content = serde_json::to_string_pretty(manifest)?;
    std::fs::write(&path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use temper_core::types::{ManifestEntry, ManifestEntryState};
    use uuid::Uuid;

    #[test]
    fn load_manifest_returns_new_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let temper_dir = dir.path().join(".temper");
        // Directory doesn't exist yet — no manifest file
        let manifest = load_manifest(&temper_dir, "device-001").unwrap();
        assert_eq!(manifest.device_id, "device-001");
        assert!(manifest.last_sync.is_none());
        assert!(manifest.entries.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let temper_dir = dir.path().join(".temper");

        let mut original = Manifest::new("device-abc".to_string());
        original.last_sync = Some(Utc::now());

        save_manifest(&temper_dir, &original).unwrap();
        let loaded = load_manifest(&temper_dir, "device-abc").unwrap();

        assert_eq!(loaded.device_id, original.device_id);
        assert!(loaded.last_sync.is_some());
        assert!(loaded.entries.is_empty());
    }

    #[test]
    fn manifest_entries_survive_serialization() {
        let dir = tempfile::tempdir().unwrap();
        let temper_dir = dir.path().join(".temper");

        let mut manifest = Manifest::new("device-xyz".to_string());
        let id = Uuid::nil();
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "temper/notes/test.md".to_string(),
                content_hash: "sha256:deadbeef".to_string(),
                remote_hash: "sha256:deadbeef".to_string(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
            },
        );

        save_manifest(&temper_dir, &manifest).unwrap();
        let loaded = load_manifest(&temper_dir, "device-xyz").unwrap();

        assert_eq!(loaded.entries.len(), 1);
        let entry = loaded.entries.get(&id).unwrap();
        assert_eq!(entry.path, "temper/notes/test.md");
        assert_eq!(entry.content_hash, "sha256:deadbeef");
        assert_eq!(entry.state, ManifestEntryState::Clean);
    }
}
