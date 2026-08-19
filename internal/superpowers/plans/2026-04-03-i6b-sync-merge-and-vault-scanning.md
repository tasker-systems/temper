# I6b: Sync Merge, Vault Scanning, and Progress Reporting — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable intelligent vault scanning, semantic merge with conflict annotation, and progress reporting for `temper sync run`.

**Architecture:** Extend the existing sequential `sync_orchestration` pipeline. Merge logic lives in `temper-ingest` (shared crate). New types in `temper-core`. Vault scanning, progress trait, and orchestration updates in `temper-cli`. No new API endpoints — all server interaction uses existing ingest and sync APIs.

**Tech Stack:** Rust, `similar` crate for diffing, `indicatif` + `anstream`/`anstyle` for progress output, existing `temper-ingest` chunking pipeline.

**Spec:** `docs/superpowers/specs/2026-04-03-i6b-sync-merge-and-vault-scanning-design.md`

---

## File Structure

### New Files
| File | Responsibility |
|------|---------------|
| `crates/temper-ingest/src/merge.rs` | Paragraph diff, change-region classification, semantic escalation, conflict annotation, `attempt_merge()` |
| `crates/temper-cli/src/actions/progress.rs` | `SyncProgress` trait, `TerminalProgress`, `NoopProgress`, `CollectingProgress` |

### Modified Files
| File | Changes |
|------|---------|
| `crates/temper-core/src/types/mod.rs` | Export new merge types |
| `crates/temper-core/src/types/merge.rs` | New: `MergeResult`, `MergeStrategy`, `PushKind` types |
| `crates/temper-ingest/Cargo.toml` | Add `similar` dependency |
| `crates/temper-ingest/src/lib.rs` | Export `merge` module |
| `crates/temper-cli/src/actions/mod.rs` | Export `progress` module |
| `crates/temper-cli/src/actions/ingest.rs` | Extract `infer_context_and_doctype()` helper |
| `crates/temper-cli/src/actions/sync.rs` | Add `scan_vault_for_untracked()`, update orchestration to use progress + merge, expand `SyncResult` |
| `crates/temper-cli/src/commands/sync_cmd.rs` | Wire `TerminalProgress` into orchestration calls |

---

## Task 1: Add Merge Types to temper-core

**Files:**
- Create: `crates/temper-core/src/types/merge.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

- [ ] **Step 1: Create merge types file**

```rust
// crates/temper-core/src/types/merge.rs

use serde::{Deserialize, Serialize};

/// Strategy used to resolve a merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    /// All changes were non-conflicting (resolved at paragraph diff level).
    NonConflicting,
    /// Required semantic chunk escalation to resolve.
    SemanticResolution,
    /// Some regions resolved, some annotated as conflicts.
    Mixed,
}

/// Result of attempting to merge two versions of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeResult {
    /// Successfully merged without conflicts.
    AutoMerged {
        content: String,
        strategy: MergeStrategy,
    },
    /// Merged with conflict annotations embedded in the content.
    ConflictAnnotated {
        content: String,
        conflict_count: usize,
    },
}

/// Classification of a push operation for progress reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushKind {
    /// Newly discovered vault file.
    New,
    /// Locally modified existing resource.
    Modified,
    /// Auto-merged content.
    Merged,
    /// Content with conflict annotations.
    ConflictAnnotated,
}

impl std::fmt::Display for PushKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::New => write!(f, "new"),
            Self::Modified => write!(f, "modified"),
            Self::Merged => write!(f, "merged"),
            Self::ConflictAnnotated => write!(f, "conflict-annotated"),
        }
    }
}
```

- [ ] **Step 2: Export merge types from mod.rs**

Add to `crates/temper-core/src/types/mod.rs`:

```rust
pub mod merge;
pub use merge::{MergeResult, MergeStrategy, PushKind};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-core --all-features`
Expected: clean compilation

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/types/merge.rs crates/temper-core/src/types/mod.rs
git commit -m "feat(temper-core): add MergeResult, MergeStrategy, PushKind types for I6b"
```

---

## Task 2: Add `similar` Dependency and Create Merge Module in temper-ingest

**Files:**
- Modify: `crates/temper-ingest/Cargo.toml`
- Create: `crates/temper-ingest/src/merge.rs`
- Modify: `crates/temper-ingest/src/lib.rs`

- [ ] **Step 1: Add `similar` to temper-ingest Cargo.toml**

Add under `[dependencies]`:
```toml
similar = "2"
```

- [ ] **Step 2: Write failing tests for paragraph diff**

Create `crates/temper-ingest/src/merge.rs` with tests first:

```rust
//! Semantic merge for markdown documents.
//!
//! Uses `similar` as the diff engine at paragraph and chunk granularity.
//! Pipeline: paragraph diff → change-region scan → semantic escalation → annotation.

use similar::{ChangeTag, TextDiff};
use temper_core::types::{MergeResult, MergeStrategy};

use crate::chunk::chunk_markdown;

/// Split text into paragraphs (double-newline boundaries).
fn split_paragraphs(text: &str) -> Vec<&str> {
    // Split on \n\n, keeping empty trailing entries to preserve structure.
    let mut paragraphs = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'\n' && bytes[i + 1] == b'\n' {
            paragraphs.push(&text[start..i]);
            // Skip past the double newline
            i += 2;
            // Skip additional blank lines
            while i < bytes.len() && bytes[i] == b'\n' {
                i += 1;
            }
            start = i;
        } else {
            i += 1;
        }
    }
    if start < text.len() {
        paragraphs.push(&text[start..]);
    }
    paragraphs
}

/// A classified region from the paragraph-level diff.
#[derive(Debug, PartialEq, Eq)]
enum DiffRegion<'a> {
    Equal(&'a str),
    Insert(&'a str),
    Delete(&'a str),
    Replace { local: String, remote: String },
}

/// Diff two texts at paragraph granularity and return classified regions.
fn diff_paragraphs<'a>(local: &'a str, remote: &'a str) -> Vec<DiffRegion<'a>> {
    todo!()
}

/// Attempt to merge two versions of a markdown document.
///
/// Returns `MergeResult::AutoMerged` if all changes are non-conflicting,
/// or `MergeResult::ConflictAnnotated` if any regions could not be resolved.
pub fn attempt_merge(local: &str, remote: &str) -> MergeResult {
    todo!()
}

/// Format a conflict annotation block.
fn format_conflict(local: &str, remote: &str, local_hash: &str, remote_hash: &str) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- split_paragraphs ---

    #[test]
    fn split_paragraphs_basic() {
        let text = "First paragraph.\n\nSecond paragraph.\n\nThird.";
        let paras = split_paragraphs(text);
        assert_eq!(paras, vec!["First paragraph.", "Second paragraph.", "Third."]);
    }

    #[test]
    fn split_paragraphs_single() {
        let text = "Just one paragraph.";
        let paras = split_paragraphs(text);
        assert_eq!(paras, vec!["Just one paragraph."]);
    }

    #[test]
    fn split_paragraphs_empty() {
        let paras = split_paragraphs("");
        assert!(paras.is_empty());
    }

    // --- diff_paragraphs ---

    #[test]
    fn diff_identical_texts() {
        let text = "Hello.\n\nWorld.";
        let regions = diff_paragraphs(text, text);
        assert!(regions.iter().all(|r| matches!(r, DiffRegion::Equal(_))));
    }

    #[test]
    fn diff_append_only() {
        let local = "Para 1.\n\nPara 2.\n\nPara 3 added locally.";
        let remote = "Para 1.\n\nPara 2.";
        let regions = diff_paragraphs(local, remote);
        // Should have Equal regions and an Insert for the appended paragraph
        assert!(regions.iter().any(|r| matches!(r, DiffRegion::Insert(_) | DiffRegion::Delete(_))));
        assert!(!regions.iter().any(|r| matches!(r, DiffRegion::Replace { .. })));
    }

    #[test]
    fn diff_disjoint_changes() {
        let local = "Changed first.\n\nMiddle unchanged.\n\nOriginal third.";
        let remote = "Original first.\n\nMiddle unchanged.\n\nChanged third.";
        let regions = diff_paragraphs(local, remote);
        // Two Replace regions, one Equal in the middle
        let replace_count = regions.iter().filter(|r| matches!(r, DiffRegion::Replace { .. })).count();
        assert_eq!(replace_count, 2);
    }

    // --- attempt_merge ---

    #[test]
    fn merge_identical() {
        let text = "# Title\n\nSome content.\n\nMore content.";
        let result = attempt_merge(text, text);
        match result {
            MergeResult::AutoMerged { content, strategy } => {
                assert_eq!(content, text);
                assert_eq!(strategy, MergeStrategy::NonConflicting);
            }
            _ => panic!("expected AutoMerged for identical texts"),
        }
    }

    #[test]
    fn merge_local_append() {
        let local = "# Title\n\nOriginal.\n\nAppended locally.";
        let remote = "# Title\n\nOriginal.";
        let result = attempt_merge(local, remote);
        match result {
            MergeResult::AutoMerged { content, strategy } => {
                assert!(content.contains("Appended locally."));
                assert!(content.contains("Original."));
                assert_eq!(strategy, MergeStrategy::NonConflicting);
            }
            _ => panic!("expected AutoMerged for append-only"),
        }
    }

    #[test]
    fn merge_remote_append() {
        let local = "# Title\n\nOriginal.";
        let remote = "# Title\n\nOriginal.\n\nAppended remotely.";
        let result = attempt_merge(local, remote);
        match result {
            MergeResult::AutoMerged { content, .. } => {
                assert!(content.contains("Appended remotely."));
            }
            _ => panic!("expected AutoMerged for remote append"),
        }
    }

    #[test]
    fn merge_disjoint_edits() {
        let local = "Changed first.\n\nMiddle.\n\nOriginal third.";
        let remote = "Original first.\n\nMiddle.\n\nChanged third.";
        // Both sides changed different paragraphs — these are Replace regions
        // but at disjoint positions, so semantic escalation should resolve them.
        let result = attempt_merge(local, remote);
        // This may produce AutoMerged or ConflictAnnotated depending on
        // whether semantic escalation can resolve disjoint Replace regions.
        // At minimum it should not panic.
        match &result {
            MergeResult::AutoMerged { content, .. } => {
                assert!(content.contains("Changed first."));
                assert!(content.contains("Changed third."));
            }
            MergeResult::ConflictAnnotated { content, .. } => {
                assert!(content.contains("sync-conflict"));
            }
        }
    }

    #[test]
    fn merge_true_conflict() {
        let local = "# Title\n\nLocal version of this paragraph.";
        let remote = "# Title\n\nRemote version of this paragraph.";
        let result = attempt_merge(local, remote);
        match result {
            MergeResult::ConflictAnnotated { content, conflict_count } => {
                assert!(content.contains("<!-- sync-conflict:"));
                assert!(content.contains("Local version"));
                assert!(content.contains("Remote version"));
                assert!(content.contains("<!-- end conflict -->"));
                assert_eq!(conflict_count, 1);
            }
            _ => panic!("expected ConflictAnnotated for true conflict"),
        }
    }

    // --- format_conflict ---

    #[test]
    fn conflict_annotation_format() {
        let annotation = format_conflict(
            "Local text.",
            "Remote text.",
            "a1b2c3d4",
            "e5f6g7h8",
        );
        assert!(annotation.contains("<!-- sync-conflict: local (a1b2c3d4) vs remote (e5f6g7h8)"));
        assert!(annotation.contains("<!-- local version -->"));
        assert!(annotation.contains("Local text."));
        assert!(annotation.contains("<!-- remote version -->"));
        assert!(annotation.contains("Remote text."));
        assert!(annotation.contains("<!-- end conflict -->"));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p temper-ingest -- merge::tests`
Expected: FAIL — `todo!()` panics

- [ ] **Step 4: Implement `diff_paragraphs`**

Replace the `todo!()` in `diff_paragraphs`:

```rust
fn diff_paragraphs<'a>(local: &'a str, remote: &'a str) -> Vec<DiffRegion<'a>> {
    let local_paras = split_paragraphs(local);
    let remote_paras = split_paragraphs(remote);

    let diff = TextDiff::from_slices(&local_paras, &remote_paras);
    let mut regions = Vec::new();

    for op in diff.ops() {
        match op.tag() {
            similar::DiffTag::Equal => {
                for idx in op.old_range() {
                    regions.push(DiffRegion::Equal(local_paras[idx]));
                }
            }
            similar::DiffTag::Delete => {
                for idx in op.old_range() {
                    regions.push(DiffRegion::Delete(local_paras[idx]));
                }
            }
            similar::DiffTag::Insert => {
                for idx in op.new_range() {
                    regions.push(DiffRegion::Insert(remote_paras[idx]));
                }
            }
            similar::DiffTag::Replace => {
                let local_text: String = local_paras[op.old_range()]
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let remote_text: String = remote_paras[op.new_range()]
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
                    .join("\n\n");
                regions.push(DiffRegion::Replace {
                    local: local_text,
                    remote: remote_text,
                });
            }
        }
    }
    regions
}
```

- [ ] **Step 5: Implement `format_conflict`**

```rust
fn format_conflict(local: &str, remote: &str, local_hash: &str, remote_hash: &str) -> String {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    format!(
        "<!-- sync-conflict: local ({local_hash}) vs remote ({remote_hash}) at {now} -->\n\
         <!-- local version -->\n\
         {local}\n\
         <!-- remote version -->\n\
         {remote}\n\
         <!-- end conflict -->"
    )
}
```

Add `chrono` to the dependency list if not already present (check — it may come transitively via `temper-core`). If needed, add to `crates/temper-ingest/Cargo.toml`:

```toml
chrono = { version = "0.4", features = ["serde"] }
```

- [ ] **Step 6: Implement `attempt_merge`**

```rust
pub fn attempt_merge(local: &str, remote: &str) -> MergeResult {
    // Identical — no merge needed.
    if local == remote {
        return MergeResult::AutoMerged {
            content: local.to_string(),
            strategy: MergeStrategy::NonConflicting,
        };
    }

    let regions = diff_paragraphs(local, remote);
    let has_replacements = regions.iter().any(|r| matches!(r, DiffRegion::Replace { .. }));

    // Step 2: If no Replace regions, all changes are non-conflicting.
    if !has_replacements {
        let content = reassemble_non_conflicting(&regions);
        return MergeResult::AutoMerged {
            content,
            strategy: MergeStrategy::NonConflicting,
        };
    }

    // Step 3: Attempt semantic resolution for Replace regions.
    let local_hash = &sha2_short(local);
    let remote_hash = &sha2_short(remote);
    let mut merged_parts: Vec<String> = Vec::new();
    let mut conflict_count = 0;
    let mut needed_semantic = false;

    for region in &regions {
        match region {
            DiffRegion::Equal(text) => merged_parts.push(text.to_string()),
            DiffRegion::Insert(text) => merged_parts.push(text.to_string()),
            DiffRegion::Delete(_) => { /* accepted removal */ }
            DiffRegion::Replace { local: l, remote: r } => {
                match resolve_replace_region(l, r) {
                    ReplaceResolution::Resolved(text) => {
                        needed_semantic = true;
                        merged_parts.push(text);
                    }
                    ReplaceResolution::Conflict => {
                        conflict_count += 1;
                        merged_parts.push(format_conflict(l, r, local_hash, remote_hash));
                    }
                }
            }
        }
    }

    let content = merged_parts.join("\n\n");

    if conflict_count > 0 {
        MergeResult::ConflictAnnotated {
            content,
            conflict_count,
        }
    } else {
        MergeResult::AutoMerged {
            content,
            strategy: if needed_semantic {
                MergeStrategy::SemanticResolution
            } else {
                MergeStrategy::NonConflicting
            },
        }
    }
}

/// Reassemble content from non-conflicting diff regions.
fn reassemble_non_conflicting(regions: &[DiffRegion<'_>]) -> String {
    let parts: Vec<&str> = regions
        .iter()
        .filter_map(|r| match r {
            DiffRegion::Equal(t) | DiffRegion::Insert(t) => Some(*t),
            DiffRegion::Delete(_) => None,
            DiffRegion::Replace { .. } => unreachable!("no replacements in non-conflicting"),
        })
        .collect();
    parts.join("\n\n")
}

enum ReplaceResolution {
    Resolved(String),
    Conflict,
}

/// Attempt semantic-level resolution of a Replace region.
///
/// Uses temper-ingest's markdown chunker to find finer-grained boundaries,
/// then re-diffs at the chunk level.
fn resolve_replace_region(local_region: &str, remote_region: &str) -> ReplaceResolution {
    let local_chunks = chunk_markdown(local_region);
    let remote_chunks = chunk_markdown(remote_region);

    let local_texts: Vec<&str> = local_chunks.iter().map(|c| c.content.as_str()).collect();
    let remote_texts: Vec<&str> = remote_chunks.iter().map(|c| c.content.as_str()).collect();

    let diff = TextDiff::from_slices(&local_texts, &remote_texts);

    // If semantic diff still shows Replace ops, it's a true conflict.
    let has_sub_replacements = diff.ops().iter().any(|op| op.tag() == similar::DiffTag::Replace);

    if has_sub_replacements {
        return ReplaceResolution::Conflict;
    }

    // All changes at chunk level are Insert/Delete/Equal — resolvable.
    let mut parts: Vec<String> = Vec::new();
    for op in diff.ops() {
        match op.tag() {
            similar::DiffTag::Equal => {
                for idx in op.old_range() {
                    parts.push(local_texts[idx].to_string());
                }
            }
            similar::DiffTag::Insert => {
                for idx in op.new_range() {
                    parts.push(remote_texts[idx].to_string());
                }
            }
            similar::DiffTag::Delete => { /* accepted removal */ }
            similar::DiffTag::Replace => unreachable!("checked above"),
        }
    }
    ReplaceResolution::Resolved(parts.join("\n\n"))
}

/// Short SHA-256 hash (first 8 hex chars) for conflict annotations.
fn sha2_short(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let bytes = hasher.finalize();
    bytes.iter().take(4).fold(String::new(), |mut acc, b| {
        acc.push_str(&format!("{b:02x}"));
        acc
    })
}
```

- [ ] **Step 7: Export merge module from lib.rs**

Add to `crates/temper-ingest/src/lib.rs`:

```rust
pub mod merge;
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p temper-ingest -- merge::tests`
Expected: all merge tests pass

- [ ] **Step 9: Run full check**

Run: `cargo make check`
Expected: clippy, fmt, typecheck all pass

- [ ] **Step 10: Commit**

```bash
git add crates/temper-ingest/Cargo.toml crates/temper-ingest/src/merge.rs crates/temper-ingest/src/lib.rs
git commit -m "feat(temper-ingest): add merge module with similar-based paragraph diff and semantic escalation"
```

---

## Task 3: Add SyncProgress Trait and Implementations

**Files:**
- Create: `crates/temper-cli/src/actions/progress.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs`

- [ ] **Step 1: Create progress module with trait and implementations**

```rust
// crates/temper-cli/src/actions/progress.rs

//! Progress reporting for sync operations.
//!
//! `SyncProgress` is a callback trait that sync orchestration uses to report
//! per-resource events. Implementations decouple I/O from business logic.

use temper_core::types::{MergeResult, PushKind};

/// Callback trait for sync progress events.
pub trait SyncProgress {
    fn scan_found(&self, path: &str, context: &str, doc_type: &str);
    fn scan_skipped(&self, path: &str, reason: &str);
    fn rehash_progress(&self, processed: usize, total: usize, skipped_by_mtime: usize);
    fn push_start(&self, path: &str, kind: PushKind);
    fn push_done(&self, path: &str);
    fn pull_start(&self, path: &str);
    fn pull_done(&self, path: &str);
    fn merge_result(&self, path: &str, outcome: &MergeResult);
    fn phase_summary(&self, phase: &str, count: usize);
}

/// No-op implementation for tests and non-interactive contexts.
pub struct NoopProgress;

impl SyncProgress for NoopProgress {
    fn scan_found(&self, _: &str, _: &str, _: &str) {}
    fn scan_skipped(&self, _: &str, _: &str) {}
    fn rehash_progress(&self, _: usize, _: usize, _: usize) {}
    fn push_start(&self, _: &str, _: PushKind) {}
    fn push_done(&self, _: &str) {}
    fn pull_start(&self, _: &str) {}
    fn pull_done(&self, _: &str) {}
    fn merge_result(&self, _: &str, _: &MergeResult) {}
    fn phase_summary(&self, _: &str, _: usize) {}
}

/// Collects events into a Vec for test assertions.
#[derive(Debug, Default)]
pub struct CollectingProgress {
    pub events: std::sync::Mutex<Vec<ProgressEvent>>,
}

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    ScanFound { path: String, context: String, doc_type: String },
    ScanSkipped { path: String, reason: String },
    RehashProgress { processed: usize, total: usize, skipped: usize },
    PushStart { path: String, kind: PushKind },
    PushDone { path: String },
    PullStart { path: String },
    PullDone { path: String },
    MergeResult { path: String, conflict_count: Option<usize> },
    PhaseSummary { phase: String, count: usize },
}

impl SyncProgress for CollectingProgress {
    fn scan_found(&self, path: &str, context: &str, doc_type: &str) {
        self.events.lock().unwrap().push(ProgressEvent::ScanFound {
            path: path.to_string(),
            context: context.to_string(),
            doc_type: doc_type.to_string(),
        });
    }
    fn scan_skipped(&self, path: &str, reason: &str) {
        self.events.lock().unwrap().push(ProgressEvent::ScanSkipped {
            path: path.to_string(),
            reason: reason.to_string(),
        });
    }
    fn rehash_progress(&self, processed: usize, total: usize, skipped: usize) {
        self.events.lock().unwrap().push(ProgressEvent::RehashProgress {
            processed,
            total,
            skipped,
        });
    }
    fn push_start(&self, path: &str, kind: PushKind) {
        self.events.lock().unwrap().push(ProgressEvent::PushStart {
            path: path.to_string(),
            kind,
        });
    }
    fn push_done(&self, path: &str) {
        self.events.lock().unwrap().push(ProgressEvent::PushDone {
            path: path.to_string(),
        });
    }
    fn pull_start(&self, path: &str) {
        self.events.lock().unwrap().push(ProgressEvent::PullStart {
            path: path.to_string(),
        });
    }
    fn pull_done(&self, path: &str) {
        self.events.lock().unwrap().push(ProgressEvent::PullDone {
            path: path.to_string(),
        });
    }
    fn merge_result(&self, path: &str, outcome: &MergeResult) {
        let conflict_count = match outcome {
            MergeResult::ConflictAnnotated { conflict_count, .. } => Some(*conflict_count),
            _ => None,
        };
        self.events.lock().unwrap().push(ProgressEvent::MergeResult {
            path: path.to_string(),
            conflict_count,
        });
    }
    fn phase_summary(&self, phase: &str, count: usize) {
        self.events.lock().unwrap().push(ProgressEvent::PhaseSummary {
            phase: phase.to_string(),
            count,
        });
    }
}

/// Terminal progress reporter using indicatif and the existing output module.
pub struct TerminalProgress {
    use_progress: bool,
}

impl TerminalProgress {
    pub fn new() -> Self {
        let use_progress = std::io::IsTerminal::is_terminal(&std::io::stderr());
        Self { use_progress }
    }
}

impl SyncProgress for TerminalProgress {
    fn scan_found(&self, path: &str, context: &str, doc_type: &str) {
        if self.use_progress {
            crate::output::success(format!("New: {path} (context: {context}, doc_type: {doc_type})"));
        }
    }
    fn scan_skipped(&self, path: &str, reason: &str) {
        if self.use_progress {
            crate::output::warning(format!("Skipped: {path} ({reason})"));
        }
    }
    fn rehash_progress(&self, processed: usize, total: usize, skipped_by_mtime: usize) {
        if self.use_progress {
            crate::output::dim(format!(
                "Rehashed {processed}/{total} files ({skipped_by_mtime} unchanged by mtime)"
            ));
        }
    }
    fn push_start(&self, path: &str, kind: PushKind) {
        if self.use_progress {
            crate::output::item(format!("↑ Push: {path} ({kind})"));
        }
    }
    fn push_done(&self, _path: &str) {}
    fn pull_start(&self, path: &str) {
        if self.use_progress {
            crate::output::item(format!("↓ Pull: {path}"));
        }
    }
    fn pull_done(&self, _path: &str) {}
    fn merge_result(&self, path: &str, outcome: &MergeResult) {
        if self.use_progress {
            match outcome {
                MergeResult::AutoMerged { strategy, .. } => {
                    crate::output::item(format!("⟳ Merge: {path} — auto-merged ({strategy:?})"));
                }
                MergeResult::ConflictAnnotated { conflict_count, .. } => {
                    crate::output::warning(format!(
                        "Merge: {path} — {conflict_count} conflict region(s) (annotated)"
                    ));
                }
            }
        }
    }
    fn phase_summary(&self, phase: &str, count: usize) {
        if self.use_progress {
            crate::output::header(format!("{phase}..."));
            if count > 0 {
                crate::output::dim(format!("  {count} items"));
            }
        }
    }
}
```

- [ ] **Step 2: Export progress module from actions/mod.rs**

Add to `crates/temper-cli/src/actions/mod.rs`:

```rust
pub mod progress;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-cli --all-features`
Expected: clean compilation

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/progress.rs crates/temper-cli/src/actions/mod.rs
git commit -m "feat(temper-cli): add SyncProgress trait with Terminal, Noop, and Collecting implementations"
```

---

## Task 4: Add Vault Scanning

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs` (extract helper)
- Modify: `crates/temper-cli/src/actions/sync.rs` (add `scan_vault_for_untracked`)

- [ ] **Step 1: Write failing test for `infer_context_and_doctype`**

Add to the test module in `crates/temper-cli/src/actions/ingest.rs`:

```rust
#[test]
fn infer_context_doctype_from_path() {
    let vault = Path::new("/vault");
    let file = Path::new("/vault/temper/research/my-notes.md");
    let (ctx, dt) = infer_context_and_doctype(vault, file, None, None).unwrap();
    assert_eq!(ctx, "temper");
    assert_eq!(dt, "research");
}

#[test]
fn infer_context_doctype_frontmatter_override() {
    let vault = Path::new("/vault");
    let file = Path::new("/vault/temper/research/my-notes.md");
    let (ctx, dt) = infer_context_and_doctype(
        vault, file,
        Some("custom-context"),
        Some("session"),
    ).unwrap();
    assert_eq!(ctx, "custom-context");
    assert_eq!(dt, "session");
}

#[test]
fn infer_context_doctype_rejects_shallow() {
    let vault = Path::new("/vault");
    let file = Path::new("/vault/orphan.md");
    let result = infer_context_and_doctype(vault, file, None, None);
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p temper-cli --lib -- ingest::tests::infer_context`
Expected: FAIL — function not found

- [ ] **Step 3: Implement `infer_context_and_doctype`**

Add to `crates/temper-cli/src/actions/ingest.rs`:

```rust
/// Infer context and doc_type for a vault file.
///
/// Uses frontmatter overrides if provided, otherwise infers from the file's
/// position in the vault directory hierarchy: `{vault}/{context}/{doc_type}/{slug}.md`.
pub fn infer_context_and_doctype(
    vault_root: &Path,
    file_path: &Path,
    fm_context: Option<&str>,
    fm_doc_type: Option<&str>,
) -> Result<(String, String)> {
    let rel = file_path
        .strip_prefix(vault_root)
        .map_err(|_| TemperError::Config(format!(
            "file {} is not inside vault {}",
            file_path.display(),
            vault_root.display()
        )))?;

    let parts: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    // Need at least context/doc_type/file.md (3 components)
    let dir_context = parts.first().copied();
    let dir_doc_type = if parts.len() >= 3 { Some(parts[1]) } else { None };

    let context = fm_context
        .or(dir_context)
        .ok_or_else(|| TemperError::Config(format!(
            "cannot infer context for {}", file_path.display()
        )))?
        .to_string();

    let doc_type = fm_doc_type
        .or(dir_doc_type)
        .ok_or_else(|| TemperError::Config(format!(
            "cannot infer doc_type for {} (file must be at {{context}}/{{doc_type}}/{{slug}}.md)",
            file_path.display()
        )))?
        .to_string();

    Ok((context, doc_type))
}
```

- [ ] **Step 4: Run infer tests**

Run: `cargo test -p temper-cli --lib -- ingest::tests::infer_context`
Expected: PASS

- [ ] **Step 5: Write failing test for `scan_vault_for_untracked`**

Add to test module in `crates/temper-cli/src/actions/sync.rs`:

```rust
#[test]
fn scan_vault_discovers_untracked_files() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path();

    // Create a file in proper vault structure
    let file_dir = vault.join("temper/research");
    fs::create_dir_all(&file_dir).unwrap();
    fs::write(file_dir.join("new-discovery.md"), "# New Discovery\n\nSome content.").unwrap();

    let mut manifest = Manifest::new("device-test".to_string());
    let progress = crate::actions::progress::CollectingProgress::default();

    let found = scan_vault_for_untracked(&mut manifest, vault, &progress).unwrap();
    assert_eq!(found, 1);
    assert_eq!(manifest.entries.len(), 1);

    let events = progress.events.lock().unwrap();
    assert!(events.iter().any(|e| matches!(e,
        crate::actions::progress::ProgressEvent::ScanFound { path, .. }
        if path.contains("new-discovery.md")
    )));
}

#[test]
fn scan_vault_skips_files_already_in_manifest() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path();

    let file_dir = vault.join("temper/research");
    fs::create_dir_all(&file_dir).unwrap();
    fs::write(file_dir.join("existing.md"), "# Existing\n\nContent.").unwrap();

    let mut manifest = Manifest::new("device-test".to_string());
    let id = Uuid::now_v7();
    manifest.entries.insert(id, ManifestEntry {
        path: "temper/research/existing.md".to_string(),
        content_hash: "somehash".to_string(),
        remote_hash: "somehash".to_string(),
        synced_at: Utc::now(),
        state: ManifestEntryState::Clean,
        mtime_secs: None,
    });

    let progress = crate::actions::progress::CollectingProgress::default();
    let found = scan_vault_for_untracked(&mut manifest, vault, &progress).unwrap();
    assert_eq!(found, 0);
}

#[test]
fn scan_vault_skips_unmappable_files() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path();

    // File at vault root — can't infer context/doc_type
    fs::write(vault.join("orphan.md"), "# Orphan").unwrap();

    let mut manifest = Manifest::new("device-test".to_string());
    let progress = crate::actions::progress::CollectingProgress::default();

    let found = scan_vault_for_untracked(&mut manifest, vault, &progress).unwrap();
    assert_eq!(found, 0);

    let events = progress.events.lock().unwrap();
    assert!(events.iter().any(|e| matches!(e,
        crate::actions::progress::ProgressEvent::ScanSkipped { .. }
    )));
}

#[test]
fn scan_vault_respects_frontmatter_override() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path();

    let file_dir = vault.join("temper/research");
    fs::create_dir_all(&file_dir).unwrap();
    fs::write(
        file_dir.join("overridden.md"),
        "---\ncontext: custom\ndoc_type: session\n---\n\n# Overridden\n"
    ).unwrap();

    let mut manifest = Manifest::new("device-test".to_string());
    let progress = crate::actions::progress::CollectingProgress::default();

    let found = scan_vault_for_untracked(&mut manifest, vault, &progress).unwrap();
    assert_eq!(found, 1);

    let events = progress.events.lock().unwrap();
    assert!(events.iter().any(|e| matches!(e,
        crate::actions::progress::ProgressEvent::ScanFound { context, doc_type, .. }
        if context == "custom" && doc_type == "session"
    )));
}
```

- [ ] **Step 6: Run scan tests to verify they fail**

Run: `cargo test -p temper-cli --lib -- sync::tests::scan_vault`
Expected: FAIL — function not found

- [ ] **Step 7: Implement `scan_vault_for_untracked`**

Add to `crates/temper-cli/src/actions/sync.rs`:

```rust
use crate::actions::progress::SyncProgress;

/// Scan the vault directory for untracked markdown files.
///
/// For each discovered file, parses frontmatter to determine context/doc_type
/// (with directory-based inference as fallback), writes frontmatter if missing,
/// and adds the file to the manifest as `Pending`.
///
/// Returns the count of newly discovered files.
pub fn scan_vault_for_untracked(
    manifest: &mut Manifest,
    vault_root: &Path,
    progress: &dyn SyncProgress,
) -> Result<usize> {
    let known_paths: std::collections::HashSet<String> = manifest
        .entries
        .values()
        .map(|e| e.path.clone())
        .collect();

    let mut found = 0;

    for entry in ignore::WalkBuilder::new(vault_root)
        .hidden(true) // skip dotfiles/dirs like .temper
        .build()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        // Only process .md files
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        // Skip .temper directory
        if path.starts_with(vault_root.join(".temper")) {
            continue;
        }

        let rel_path = path
            .strip_prefix(vault_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // Skip files already in manifest
        if known_paths.contains(&rel_path) {
            continue;
        }

        let content = std::fs::read_to_string(path)?;
        let fm = ingest::parse_source_frontmatter(&content);

        let fm_context = fm.as_ref().and_then(|f| f.context.as_deref());
        let fm_doc_type = fm.as_ref().and_then(|f| f.doc_type.as_deref());

        let (context, doc_type) = match ingest::infer_context_and_doctype(
            vault_root, path, fm_context, fm_doc_type,
        ) {
            Ok(pair) => pair,
            Err(e) => {
                progress.scan_skipped(&rel_path, &e.to_string());
                continue;
            }
        };

        // Generate frontmatter if missing
        let resource_id = Uuid::now_v7();
        if fm.is_none() {
            let frontmatter = ingest::build_frontmatter(
                resource_id,
                &ingest::title_from_path(path),
                &context,
                &doc_type,
                None,
                None,
            );
            let new_content = format!("{frontmatter}{content}");
            std::fs::write(path, &new_content)?;
        }

        // Compute body-only hash
        let full_content = std::fs::read_to_string(path)?;
        let body = strip_frontmatter(&full_content);
        let content_hash = ingest::compute_content_hash(body);
        let mtime = file_mtime_secs(path).ok();

        manifest.entries.insert(
            resource_id,
            ManifestEntry {
                path: rel_path.clone(),
                content_hash,
                remote_hash: String::new(),
                synced_at: chrono::Utc::now(),
                state: ManifestEntryState::Pending,
                mtime_secs: mtime,
            },
        );

        progress.scan_found(&rel_path, &context, &doc_type);
        found += 1;
    }

    Ok(found)
}
```

- [ ] **Step 8: Run scan tests**

Run: `cargo test -p temper-cli --lib -- sync::tests::scan_vault`
Expected: PASS

- [ ] **Step 9: Run full check**

Run: `cargo make check`
Expected: all checks pass

- [ ] **Step 10: Commit**

```bash
git add crates/temper-cli/src/actions/ingest.rs crates/temper-cli/src/actions/sync.rs
git commit -m "feat(temper-cli): add vault scanning for untracked files with frontmatter inference"
```

---

## Task 5: Integrate Merge into Sync Orchestration

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs`

- [ ] **Step 1: Expand `SyncResult` to include merge counts**

Update the `SyncResult` struct in `sync.rs`:

```rust
#[derive(Debug)]
pub struct SyncResult {
    pub push_count: usize,
    pub pull_count: usize,
    pub conflict_count: usize,
    pub removed_count: usize,
    pub scan_count: usize,
    pub merge_auto_count: usize,
    pub merge_conflict_count: usize,
}
```

- [ ] **Step 2: Add `merge_and_push_resource` function**

Add to `sync.rs`:

```rust
/// Merge a conflicting resource and push the result.
///
/// Fetches remote content, runs the merge pipeline, writes the merged
/// file back to the vault, re-chunks/re-embeds, and pushes to the server.
async fn merge_and_push_resource(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    item: &temper_core::types::SyncConflictItem,
    progress: &dyn SyncProgress,
) -> Result<MergeResult> {
    let entry = manifest
        .entries
        .get(&item.resource_id)
        .ok_or_else(|| TemperError::NotFound(format!(
            "manifest entry not found: {}", item.resource_id
        )))?;

    let file_path = vault_root.join(&entry.path);
    let local_full = std::fs::read_to_string(&file_path)?;
    let local_body = strip_frontmatter(&local_full);

    // Fetch remote content
    let content_response = client
        .resources()
        .content(item.resource_id)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;
    let remote_body = &content_response.markdown;

    // Run merge pipeline
    let merge_result = temper_ingest::merge::attempt_merge(local_body, remote_body);

    // Get merged content
    let merged_body = match &merge_result {
        temper_core::types::MergeResult::AutoMerged { content, .. } => content.clone(),
        temper_core::types::MergeResult::ConflictAnnotated { content, .. } => content.clone(),
    };

    progress.merge_result(&entry.path, &merge_result);

    // Write merged content back to vault file (preserve frontmatter)
    let frontmatter = extract_frontmatter_block(&local_full);
    let new_content = format!("{frontmatter}{merged_body}");
    std::fs::write(&file_path, &new_content)?;

    // Re-chunk/re-embed and push
    let body = strip_frontmatter(&new_content);
    let parts: Vec<&str> = entry.path.split('/').collect();
    let context = parts.first().copied().unwrap_or("default");
    let doc_type = if parts.len() > 1 { parts[1] } else { "resource" };
    let title = ingest::title_from_path(&file_path);

    let push_kind = match &merge_result {
        temper_core::types::MergeResult::AutoMerged { .. } => temper_core::types::PushKind::Merged,
        temper_core::types::MergeResult::ConflictAnnotated { .. } => temper_core::types::PushKind::ConflictAnnotated,
    };

    let payload = ingest::build_ingest_payload(
        body, &title, context, doc_type, "imported", "text/markdown", None,
    )?;

    progress.push_start(&entry.path, push_kind);

    let resource = client
        .ingest()
        .update(item.resource_id, &payload)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    if let Some(e) = manifest.entries.get_mut(&item.resource_id) {
        e.remote_hash = resource.content_hash.unwrap_or_default();
        e.content_hash = ingest::compute_content_hash(body);
        e.state = ManifestEntryState::Clean;
        e.synced_at = chrono::Utc::now();
        e.mtime_secs = file_mtime_secs(&file_path).ok();
    }

    progress.push_done(&entry.path);

    Ok(merge_result)
}

/// Extract the frontmatter block (including delimiters) from file content.
fn extract_frontmatter_block(content: &str) -> &str {
    if let Some(after_open) = content.strip_prefix("---\n") {
        if let Some(end) = after_open.find("\n---\n") {
            return &content[..4 + end + 5]; // "---\n" + content + "\n---\n"
        }
    }
    ""
}
```

- [ ] **Step 3: Update `sync_orchestration` signature and flow**

Update the function to accept a progress reporter and add the new phases:

```rust
pub async fn sync_orchestration(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    context_filter: &[String],
    progress: &dyn SyncProgress,
) -> Result<SyncResult> {
    // Step 1: Scan vault for untracked files
    let scan_count = scan_vault_for_untracked(manifest, vault_root, progress)?;
    progress.phase_summary("Scanning vault", scan_count);

    // Step 2: Rehash manifest
    let rehash_result = rehash_manifest(manifest, vault_root)?;
    let total = manifest.entries.len();
    let skipped = total - rehash_result;
    progress.rehash_progress(rehash_result, total, skipped);

    // Step 3: Request diff
    let request = build_status_request(manifest, context_filter);
    let diff = client
        .sync()
        .status(&request)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    let push_count = diff.to_push.len();
    let pull_count = diff.to_pull.len();
    let removed_count = diff.removed.len();

    // Step 4-5: Push (new + dirty)
    for item in &diff.to_push {
        let entry_id = item.resource_id.unwrap_or_else(|| {
            extract_resource_id(&item.uri).unwrap_or_default()
        });
        let kind = if item.resource_id.is_none() {
            PushKind::New
        } else {
            PushKind::Modified
        };
        if let Some(entry) = manifest.entries.get(&entry_id) {
            progress.push_start(&entry.path, kind);
        }
        push_resource(client, manifest, vault_root, item).await?;
        if let Some(entry) = manifest.entries.get(&entry_id) {
            progress.push_done(&entry.path);
        }
    }

    // Step 6: Pull
    for item in &diff.to_pull {
        if let Some(entry) = manifest.entries.get(&item.resource_id) {
            progress.pull_start(&entry.path);
        }
        pull_resource(client, manifest, vault_root, item).await?;
        if let Some(entry) = manifest.entries.get(&item.resource_id) {
            progress.pull_done(&entry.path);
        }
    }

    // Step 7-8: Merge conflicts and push merged content
    let mut merge_auto_count = 0;
    let mut merge_conflict_count = 0;
    for item in &diff.conflicts {
        let result = merge_and_push_resource(client, manifest, vault_root, item, progress).await?;
        match result {
            temper_core::types::MergeResult::AutoMerged { .. } => merge_auto_count += 1,
            temper_core::types::MergeResult::ConflictAnnotated { .. } => merge_conflict_count += 1,
        }
    }

    // Step 9: Handle removed
    for item in &diff.removed {
        remove_resource(manifest, vault_root, item)?;
    }

    // Step 10: Complete
    let complete_req = build_complete_request(&manifest.device_id, vec![]);
    let complete_resp = client
        .sync()
        .complete(&complete_req)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    // Step 11: Update manifest timestamp
    manifest.last_sync = Some(complete_resp.last_sync_at);

    let result = SyncResult {
        push_count,
        pull_count,
        conflict_count: diff.conflicts.len(),
        removed_count,
        scan_count,
        merge_auto_count,
        merge_conflict_count,
    };

    progress.phase_summary("Done", push_count + pull_count + merge_auto_count + merge_conflict_count);

    Ok(result)
}
```

- [ ] **Step 4: Update `sync_status_check` to accept progress**

```rust
pub async fn sync_status_check(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    context_filter: &[String],
    progress: &dyn SyncProgress,
) -> Result<SyncStatusResponse> {
    scan_vault_for_untracked(manifest, vault_root, progress)?;
    rehash_manifest(manifest, vault_root)?;

    let request = build_status_request(manifest, context_filter);
    client
        .sync()
        .status(&request)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))
}
```

- [ ] **Step 5: Add test for `extract_frontmatter_block`**

```rust
#[test]
fn extract_frontmatter_block_returns_block() {
    let content = "---\ntitle: Test\ncontext: temper\n---\n\n# Body\n";
    let block = extract_frontmatter_block(content);
    assert_eq!(block, "---\ntitle: Test\ncontext: temper\n---\n");
}

#[test]
fn extract_frontmatter_block_returns_empty_for_no_frontmatter() {
    let content = "# No frontmatter\n";
    let block = extract_frontmatter_block(content);
    assert_eq!(block, "");
}
```

- [ ] **Step 6: Run all sync tests**

Run: `cargo test -p temper-cli --lib -- sync::tests`
Expected: all pass

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "feat(temper-cli): integrate merge pipeline and vault scanning into sync orchestration"
```

---

## Task 6: Wire Progress into CLI Commands

**Files:**
- Modify: `crates/temper-cli/src/commands/sync_cmd.rs`

- [ ] **Step 1: Read the current sync_cmd.rs to understand exact call sites**

Read the file for exact line references before modifying.

- [ ] **Step 2: Update `run()` to create and pass `TerminalProgress`**

Update the `sync_orchestration` call to pass a progress reporter:

```rust
use crate::actions::progress::TerminalProgress;

// In run():
let progress = TerminalProgress::new();
let result = rt.block_on(async {
    sync_actions::sync_orchestration(&client, &mut manifest, &vault_root, contexts, &progress).await
})?;
```

Update the results output to include merge counts:

```rust
// After sync completes, update the output to show merge info:
if result.scan_count > 0 {
    output::label("Scanned", format!("{} new files", result.scan_count));
}
output::label("Push", result.push_count);
output::label("Pull", result.pull_count);
if result.merge_auto_count > 0 {
    output::label("Merged", result.merge_auto_count);
}
if result.merge_conflict_count > 0 {
    output::warning(format!("{} conflict(s) annotated", result.merge_conflict_count));
}
output::label("Removed", result.removed_count);
```

- [ ] **Step 3: Update `status()` to pass `NoopProgress`**

```rust
use crate::actions::progress::NoopProgress;

// In status():
let progress = NoopProgress;
let diff = rt.block_on(async {
    sync_actions::sync_status_check(&client, &mut manifest, &vault_root, contexts, &progress).await
})?;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p temper-cli --all-features`
Expected: clean compilation

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/sync_cmd.rs
git commit -m "feat(temper-cli): wire TerminalProgress into sync run and NoopProgress into sync status"
```

---

## Task 7: Full Verification and Cleanup

**Files:**
- All modified files

- [ ] **Step 1: Run full test suite**

Run: `cargo make test`
Expected: all tests pass

- [ ] **Step 2: Run full check (clippy, fmt, typecheck)**

Run: `cargo make check`
Expected: all checks pass

- [ ] **Step 3: Install and verify against remote**

```bash
cargo install --path crates/temper-cli --all-features
temper sync status
```

Expected: clean status with no regressions from previous fix

- [ ] **Step 4: Test vault scanning manually**

Create a test file in the vault and run sync:

```bash
mkdir -p ~/projects/kb-vault/temper/research
echo "# Test Discovery\n\nThis file should be discovered by vault scanning." > ~/projects/kb-vault/temper/research/test-vault-scan.md
temper sync status
```

Expected: should show the new file in push count. Then clean up the test file.

- [ ] **Step 5: Commit any remaining fixes**

If any fixes were needed during verification, commit them.

- [ ] **Step 6: Final commit message**

```bash
git add -A
git commit -m "chore: I6b verification pass — all tests and checks green"
```
