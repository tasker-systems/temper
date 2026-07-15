//! `.temper/ingest/<resource_id>.json` — the CLI's local resume manifest for a segmented
//! (multi-block) ingest session (streaming-resumable ingestion, Beat 3).
//!
//! Not vault content — a local-only sidecar (like `.temper/config.toml`), used purely to
//! avoid re-embedding/re-sending already-durable segments after an interrupted `resource
//! create`. The server's ledger (`kb_ingestion_records` + `block_created`/`resource_finalized`
//! events) is the authoritative resume source; this file is a cache of "what we last believed
//! landed," always re-verified against a live `list_blocks` call before resuming
//! (`actions::ingest::run_segmented_create`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Result, TemperError};
use temper_core::types::ingest::SegmentInfo;

/// The current segmenter identity. Bumped whenever `temper_ingest::stream::segment_reader`
/// changes the *bytes* it emits for a given source, so a manifest cut by an older segmenter
/// never resumes against segments the current one would cut differently.
///
/// - **1** — the original normalizing segmenter (`BufRead::lines()` + `join("\n")`, which
///   collapsed CRLF→LF and dropped the trailing newline).
/// - **2** — the verbatim segmenter (`read_line`, terminators retained). W2 PR 2.
pub const SEGMENTER_VERSION: u32 = 2;

/// A manifest written before `segmenter_version` existed was cut by the v1 normalizing
/// segmenter. It deserializes to `1` (not an error), then fails the equality gate in
/// [`find_resumable`] — so the CLI begins a fresh session rather than resuming against
/// bytes the current segmenter would cut differently.
fn legacy_segmenter_version() -> u32 {
    1
}

/// The CLI's local record of a segmented ingest session's progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestManifest {
    pub resource_id: Uuid,
    /// Raw (unprefixed, 64 hex chars) sha256 of the whole source's bytes — a source-integrity
    /// check. A resume whose freshly-computed source hash disagrees with this means the file
    /// changed since the interrupted attempt: the manifest must not be resumed against.
    pub source_hash: String,
    /// The segment budget (bytes) this session's boundaries were cut at. Recorded so a resume
    /// re-derives identical segment boundaries — determinism is load-bearing for diffing
    /// `blocks` against a freshly re-scanned source.
    pub block_budget: u32,
    /// The segmenter identity ([`SEGMENTER_VERSION`]) that cut this session's segments. Part of
    /// the resume identity: a manifest cut by a different segmenter is never resumed against,
    /// because its recorded per-block `content_hash`es would disagree with freshly re-cut
    /// segments while still matching on `(source_hash, block_budget)`.
    #[serde(default = "legacy_segmenter_version")]
    pub segmenter_version: u32,
    pub correlation_id: Uuid,
    /// Landed segments, as last observed (seq + block-merkle `content_hash`, matching
    /// `SegmentInfo`'s wire shape). Always re-verified against a live `list_blocks` call
    /// before being trusted for a resume — this field is a cache, not ground truth.
    pub blocks: Vec<SegmentInfo>,
    pub finalized: bool,
}

/// The on-disk path for a resource's ingest manifest: `<vault>/.temper/ingest/<id>.json`.
pub fn manifest_path(vault: &Path, resource_id: Uuid) -> PathBuf {
    vault
        .join(".temper")
        .join("ingest")
        .join(format!("{resource_id}.json"))
}

/// Load a manifest from `path`, if present. `Ok(None)` for a missing file (the ordinary case
/// for a resource's first ingest attempt); `Err` for a present-but-unreadable or corrupt file
/// — never silently treated as "no manifest," since that would restart over a corrupt-but-real
/// resume state rather than surfacing the problem.
pub fn load(path: &Path) -> Result<Option<IngestManifest>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| TemperError::Project(format!("read ingest manifest {path:?}: {e}")))?;
    let manifest: IngestManifest = serde_json::from_str(&raw)
        .map_err(|e| TemperError::Project(format!("parse ingest manifest {path:?}: {e}")))?;
    Ok(Some(manifest))
}

/// Write `m` to `path`, creating parent directories (`.temper/ingest/`) as needed.
pub fn store(path: &Path, m: &IngestManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| TemperError::Project(format!("create {parent:?}: {e}")))?;
    }
    let raw = serde_json::to_string_pretty(m)
        .map_err(|e| TemperError::Project(format!("serialize ingest manifest: {e}")))?;
    std::fs::write(path, raw)
        .map_err(|e| TemperError::Project(format!("write ingest manifest {path:?}: {e}")))
}

/// Segments still to send = local segments whose `(seq, content_hash)` is not already present
/// in `landed`. A landed `seq` whose hash differs from the local recomputation is treated as
/// missing too (a pure set difference on the pair, not just on `seq`) — in ordinary operation
/// this never happens for an unchanged source (chunking is deterministic), but the diff itself
/// stays a dumb set difference rather than assuming that invariant. Order-preserving in
/// `local`'s own seq order.
pub fn resume_gap(local: &[SegmentInfo], landed: &[SegmentInfo]) -> Vec<u32> {
    let landed_set: HashSet<(u32, &str)> = landed
        .iter()
        .map(|s| (s.seq, s.content_hash.as_str()))
        .collect();
    local
        .iter()
        .filter(|s| !landed_set.contains(&(s.seq, s.content_hash.as_str())))
        .map(|s| s.seq)
        .collect()
}

/// Scan `<vault>/.temper/ingest/*.json` for an incomplete (`finalized == false`) manifest
/// whose `source_hash`/`block_budget`/`segmenter_version` match the source about to be
/// ingested (segmenter identity included so a manifest cut by an older, normalizing segmenter
/// is never resumed against — see [`SEGMENTER_VERSION`]).
///
/// This is the local-only mechanism `run_segmented_create` uses to recognize "this exact
/// `resource create` was already begun and interrupted" on a bare re-run that has no resource
/// ref of its own to key a resume off (the resource doesn't exist yet on a fresh attempt, and
/// isn't known to the caller on a retried one). A non-matching or already-finalized manifest is
/// not an error — it simply isn't a resume candidate — and is left untouched on disk (an
/// unrelated in-progress manifest, or one whose source changed since the interrupted attempt,
/// is someone else's concern; nothing here reaps or deletes it).
pub fn find_resumable(
    vault: &Path,
    source_hash: &str,
    block_budget: u32,
) -> Result<Option<(Uuid, IngestManifest)>> {
    let dir = vault.join(".temper").join("ingest");
    if !dir.is_dir() {
        return Ok(None);
    }
    let entries =
        std::fs::read_dir(&dir).map_err(|e| TemperError::Project(format!("read {dir:?}: {e}")))?;
    for entry in entries {
        let entry = entry.map_err(|e| TemperError::Project(format!("read {dir:?} entry: {e}")))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(manifest) = load(&path)? else {
            continue;
        };
        if !manifest.finalized
            && manifest.source_hash == source_hash
            && manifest.block_budget == block_budget
            && manifest.segmenter_version == SEGMENTER_VERSION
        {
            return Ok(Some((manifest.resource_id, manifest)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(seq: u32, hash: &str) -> SegmentInfo {
        SegmentInfo {
            seq,
            content_hash: hash.to_string(),
        }
    }

    fn sample_manifest(resource_id: Uuid, source_hash: &str, finalized: bool) -> IngestManifest {
        IngestManifest {
            resource_id,
            source_hash: source_hash.to_string(),
            block_budget: 262_144,
            segmenter_version: SEGMENTER_VERSION,
            correlation_id: Uuid::now_v7(),
            blocks: vec![seg(0, "h0"), seg(1, "h1")],
            finalized,
        }
    }

    // --- resume_gap ---

    #[test]
    fn resume_gap_returns_only_missing_seqs() {
        let local = vec![seg(0, "h0"), seg(1, "h1"), seg(2, "h2")];
        let landed = vec![seg(0, "h0"), seg(1, "h1")];
        assert_eq!(resume_gap(&local, &landed), vec![2]);
    }

    #[test]
    fn resume_gap_empty_when_fully_landed() {
        let local = vec![seg(0, "h0"), seg(1, "h1")];
        let landed = vec![seg(0, "h0"), seg(1, "h1")];
        assert!(resume_gap(&local, &landed).is_empty());
    }

    #[test]
    fn resume_gap_empty_when_nothing_local() {
        assert!(resume_gap(&[], &[seg(0, "h0")]).is_empty());
    }

    #[test]
    fn resume_gap_returns_every_seq_when_nothing_landed() {
        let local = vec![seg(0, "h0"), seg(1, "h1")];
        assert_eq!(resume_gap(&local, &[]), vec![0, 1]);
    }

    #[test]
    fn resume_gap_resends_a_seq_whose_hash_disagrees() {
        // A landed seq with a hash that disagrees with the local recomputation is treated as
        // missing — a pure set difference on (seq, hash), not just on seq.
        let local = vec![seg(0, "h0-new")];
        let landed = vec![seg(0, "h0-old")];
        assert_eq!(resume_gap(&local, &landed), vec![0]);
    }

    // --- manifest_path ---

    #[test]
    fn manifest_path_is_under_dot_temper_ingest() {
        let vault = Path::new("/vault");
        let id = Uuid::now_v7();
        let path = manifest_path(vault, id);
        assert_eq!(
            path,
            vault
                .join(".temper")
                .join("ingest")
                .join(format!("{id}.json"))
        );
    }

    // --- load/store round trip ---

    #[test]
    fn manifest_round_trips_through_store_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let resource_id = Uuid::now_v7();
        let path = manifest_path(dir.path(), resource_id);
        let manifest = sample_manifest(resource_id, &"a".repeat(64), false);

        store(&path, &manifest).unwrap();
        let loaded = load(&path).unwrap().expect("manifest should load");

        assert_eq!(loaded.resource_id, manifest.resource_id);
        assert_eq!(loaded.source_hash, manifest.source_hash);
        assert_eq!(loaded.block_budget, manifest.block_budget);
        assert_eq!(loaded.correlation_id, manifest.correlation_id);
        assert_eq!(loaded.blocks.len(), 2);
        assert_eq!(loaded.blocks[1].seq, 1);
        assert!(!loaded.finalized);
    }

    #[test]
    fn load_missing_manifest_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = manifest_path(dir.path(), Uuid::now_v7());
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn load_corrupt_manifest_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = manifest_path(dir.path(), Uuid::now_v7());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json").unwrap();
        assert!(load(&path).is_err());
    }

    // --- find_resumable ---

    #[test]
    fn find_resumable_matches_on_source_hash_and_budget() {
        let dir = tempfile::tempdir().unwrap();
        let resource_id = Uuid::now_v7();
        let source_hash = "b".repeat(64);
        let manifest = sample_manifest(resource_id, &source_hash, false);
        store(&manifest_path(dir.path(), resource_id), &manifest).unwrap();

        let found = find_resumable(dir.path(), &source_hash, 262_144)
            .unwrap()
            .expect("should find the matching manifest");
        assert_eq!(found.0, resource_id);
    }

    #[test]
    fn find_resumable_ignores_a_finalized_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let resource_id = Uuid::now_v7();
        let source_hash = "c".repeat(64);
        let manifest = sample_manifest(resource_id, &source_hash, true);
        store(&manifest_path(dir.path(), resource_id), &manifest).unwrap();

        assert!(find_resumable(dir.path(), &source_hash, 262_144)
            .unwrap()
            .is_none());
    }

    #[test]
    fn find_resumable_ignores_a_source_hash_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let resource_id = Uuid::now_v7();
        let manifest = sample_manifest(resource_id, &"d".repeat(64), false);
        store(&manifest_path(dir.path(), resource_id), &manifest).unwrap();

        // A different source hash (the file changed since the interrupted attempt) must not
        // resume against the stale manifest.
        assert!(find_resumable(dir.path(), &"e".repeat(64), 262_144)
            .unwrap()
            .is_none());
    }

    #[test]
    fn find_resumable_ignores_a_segmenter_version_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let resource_id = Uuid::now_v7();
        let source_hash = "a1".repeat(32);
        // A manifest cut by an older, normalizing segmenter (v1) matches on source_hash +
        // budget but must NOT resume — its segments' bytes would differ from a fresh cut.
        let mut manifest = sample_manifest(resource_id, &source_hash, false);
        manifest.segmenter_version = 1;
        store(&manifest_path(dir.path(), resource_id), &manifest).unwrap();

        assert!(find_resumable(dir.path(), &source_hash, 262_144)
            .unwrap()
            .is_none());
    }

    #[test]
    fn legacy_manifest_without_segmenter_version_deserializes_as_v1_and_never_resumes() {
        // A pre-versioning manifest on disk (no `segmenter_version` key) must deserialize
        // (not error) and default to v1, so `find_resumable` skips it and the CLI begins a
        // fresh session rather than surfacing a parse error on resume.
        let dir = tempfile::tempdir().unwrap();
        let resource_id = Uuid::now_v7();
        let source_hash = "b2".repeat(32);
        let path = manifest_path(dir.path(), resource_id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let legacy_json = format!(
            r#"{{"resource_id":"{resource_id}","source_hash":"{source_hash}","block_budget":262144,"correlation_id":"{cid}","blocks":[],"finalized":false}}"#,
            cid = Uuid::now_v7()
        );
        std::fs::write(&path, legacy_json).unwrap();

        let loaded = load(&path)
            .unwrap()
            .expect("legacy manifest should load, not error");
        assert_eq!(loaded.segmenter_version, 1, "absent field defaults to v1");
        assert!(find_resumable(dir.path(), &source_hash, 262_144)
            .unwrap()
            .is_none());
    }

    #[test]
    fn find_resumable_ignores_a_block_budget_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let resource_id = Uuid::now_v7();
        let source_hash = "f".repeat(64);
        let manifest = sample_manifest(resource_id, &source_hash, false);
        store(&manifest_path(dir.path(), resource_id), &manifest).unwrap();

        assert!(find_resumable(dir.path(), &source_hash, 8192)
            .unwrap()
            .is_none());
    }

    #[test]
    fn find_resumable_with_no_ingest_dir_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_resumable(dir.path(), "anything", 262_144)
            .unwrap()
            .is_none());
    }
}
