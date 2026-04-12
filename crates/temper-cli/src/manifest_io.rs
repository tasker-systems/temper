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

/// Save manifest to `<temper_dir>/manifest.json` atomically.
///
/// Writes the serialized JSON to a sibling `manifest.json.tmp` and then
/// renames it into place. POSIX `rename` (and Rust's `std::fs::rename` on
/// Windows when the target exists) is atomic, so a crash mid-write can
/// never leave `manifest.json` truncated to partial JSON. A stray
/// `manifest.json.tmp` from a crashed save is harmless — the next
/// successful save will overwrite it before the rename.
pub fn save_manifest(temper_dir: &Path, manifest: &Manifest) -> crate::error::Result<()> {
    std::fs::create_dir_all(temper_dir)?;
    let path = temper_dir.join("manifest.json");
    let tmp_path = temper_dir.join("manifest.json.tmp");
    let content = serde_json::to_string_pretty(manifest)?;
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use temper_core::types::{ManifestEntry, ManifestEntryState, ResourceId};
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
        let id = ResourceId::from(Uuid::nil());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "temper/notes/test.md".to_string(),
                body_hash: "sha256:deadbeef".to_string(),
                remote_body_hash: "sha256:deadbeef".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );

        save_manifest(&temper_dir, &manifest).unwrap();
        let loaded = load_manifest(&temper_dir, "device-xyz").unwrap();

        assert_eq!(loaded.entries.len(), 1);
        let entry = loaded.entries.get(&id).unwrap();
        assert_eq!(entry.path, "temper/notes/test.md");
        assert_eq!(entry.body_hash, "sha256:deadbeef");
        assert_eq!(entry.state, ManifestEntryState::Clean);
    }

    #[test]
    fn save_manifest_is_atomic_no_tmp_file_after_success() {
        let dir = tempfile::tempdir().unwrap();
        let temper_dir = dir.path().join(".temper");

        let manifest = Manifest::new("device-atomic".to_string());
        save_manifest(&temper_dir, &manifest).unwrap();

        assert!(temper_dir.join("manifest.json").exists());
        assert!(
            !temper_dir.join("manifest.json.tmp").exists(),
            "stray manifest.json.tmp left after successful save"
        );
    }

    #[test]
    fn save_manifest_overwrites_stale_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let temper_dir = dir.path().join(".temper");
        std::fs::create_dir_all(&temper_dir).unwrap();

        // Pre-seed a stray tmp file from a hypothetical crashed prior save.
        let tmp_path = temper_dir.join("manifest.json.tmp");
        std::fs::write(&tmp_path, b"not json").unwrap();

        let manifest = Manifest::new("device-stale".to_string());
        save_manifest(&temper_dir, &manifest).unwrap();

        // Canonical manifest is valid and parseable.
        let loaded = load_manifest(&temper_dir, "device-stale").unwrap();
        assert_eq!(loaded.device_id, "device-stale");

        // The stale tmp was overwritten and renamed away.
        assert!(
            !tmp_path.exists(),
            "stale manifest.json.tmp survived save_manifest"
        );
    }

    #[test]
    fn load_manifest_ignores_stray_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let temper_dir = dir.path().join(".temper");

        // Write a valid canonical manifest with one entry.
        let mut manifest = Manifest::new("device-ignore-tmp".to_string());
        let id = ResourceId::from(Uuid::nil());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "temper/notes/keep.md".to_string(),
                body_hash: "sha256:cafef00d".to_string(),
                remote_body_hash: "sha256:cafef00d".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );
        save_manifest(&temper_dir, &manifest).unwrap();

        // Drop a garbage tmp next to it. load_manifest must ignore it.
        std::fs::write(temper_dir.join("manifest.json.tmp"), b"not json").unwrap();

        let loaded = load_manifest(&temper_dir, "device-ignore-tmp").unwrap();
        assert_eq!(loaded.device_id, "device-ignore-tmp");
        assert_eq!(loaded.entries.len(), 1);
        let entry = loaded.entries.get(&id).unwrap();
        assert_eq!(entry.path, "temper/notes/keep.md");
        assert_eq!(entry.body_hash, "sha256:cafef00d");
    }

    #[test]
    fn save_manifest_crash_simulation_preserves_prior_state() {
        let dir = tempfile::tempdir().unwrap();
        let temper_dir = dir.path().join(".temper");

        // V1: a valid prior state successfully written.
        let mut v1 = Manifest::new("device-crash".to_string());
        let id = ResourceId::from(Uuid::nil());
        v1.entries.insert(
            id,
            ManifestEntry {
                path: "temper/notes/v1.md".to_string(),
                body_hash: "sha256:v1v1v1v1".to_string(),
                remote_body_hash: "sha256:v1v1v1v1".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );
        save_manifest(&temper_dir, &v1).unwrap();

        // Simulate a mid-write crash on V2: partial garbage in the tmp,
        // never reaching the atomic rename.
        std::fs::write(
            temper_dir.join("manifest.json.tmp"),
            b"{ \"device_id\": \"dev",
        )
        .unwrap();

        // load_manifest must still see V1 — manifest.json was untouched.
        let loaded = load_manifest(&temper_dir, "device-crash").unwrap();
        assert_eq!(loaded.device_id, "device-crash");
        assert_eq!(loaded.entries.len(), 1);
        let entry = loaded.entries.get(&id).unwrap();
        assert_eq!(entry.path, "temper/notes/v1.md");
        assert_eq!(entry.body_hash, "sha256:v1v1v1v1");
    }
}
