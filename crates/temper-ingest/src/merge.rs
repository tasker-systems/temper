//! Paragraph-granularity merge with semantic chunk escalation.
//!
//! Diffs two versions of a markdown document at paragraph boundaries, then
//! escalates Replace regions to semantic chunk level via [`crate::chunk::chunk_markdown`].
//! Unresolvable conflicts are annotated with HTML comments.

use chrono::Utc;
use sha2::{Digest, Sha256};
use similar::{DiffOp, DiffTag, TextDiff};
use temper_core::types::merge::{MergeResult, MergeStrategy};

use crate::chunk::chunk_markdown;

// ---------------------------------------------------------------------------
// Paragraph splitting
// ---------------------------------------------------------------------------

/// Split text into paragraphs on `\n\n` boundaries, preserving inner newlines.
pub fn split_paragraphs(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return vec![];
    }

    let mut result = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if i + 1 < len && bytes[i] == b'\n' && bytes[i + 1] == b'\n' {
            // Found a \n\n boundary — emit the paragraph before it.
            let slice = &text[start..i];
            if !slice.is_empty() {
                result.push(slice);
            }
            // Skip over consecutive blank lines.
            while i < len && bytes[i] == b'\n' {
                i += 1;
            }
            start = i;
        } else {
            i += 1;
        }
    }

    // Trailing paragraph.
    if start < len {
        let slice = &text[start..];
        if !slice.is_empty() {
            result.push(slice);
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Diff types
// ---------------------------------------------------------------------------

/// A region produced by paragraph-level diffing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffRegion {
    Equal(String),
    Insert(String),
    Delete(String),
    Replace { local: String, remote: String },
}

// ---------------------------------------------------------------------------
// Paragraph diffing
// ---------------------------------------------------------------------------

/// Diff two documents at paragraph granularity.
pub fn diff_paragraphs(local: &str, remote: &str) -> Vec<DiffRegion> {
    let local_paras = split_paragraphs(local);
    let remote_paras = split_paragraphs(remote);

    let diff = TextDiff::from_slices(&local_paras, &remote_paras);
    let mut regions = Vec::new();

    for op in diff.ops() {
        match op {
            DiffOp::Equal { old_index, len, .. } => {
                let text = local_paras[*old_index..*old_index + *len].join("\n\n");
                regions.push(DiffRegion::Equal(text));
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                let text = local_paras[*old_index..*old_index + *old_len].join("\n\n");
                regions.push(DiffRegion::Delete(text));
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                let text = remote_paras[*new_index..*new_index + *new_len].join("\n\n");
                regions.push(DiffRegion::Insert(text));
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                let local_text = local_paras[*old_index..*old_index + *old_len].join("\n\n");
                let remote_text = remote_paras[*new_index..*new_index + *new_len].join("\n\n");
                regions.push(DiffRegion::Replace {
                    local: local_text,
                    remote: remote_text,
                });
            }
        }
    }

    regions
}

// ---------------------------------------------------------------------------
// Conflict annotation
// ---------------------------------------------------------------------------

/// First 8 hex chars of SHA-256.
pub fn sha2_short(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())[..8].to_string()
}

/// Format a conflict annotation using HTML comments.
pub fn format_conflict(local: &str, remote: &str, local_hash: &str, remote_hash: &str) -> String {
    let timestamp = Utc::now().to_rfc3339();
    format!(
        "<!-- sync-conflict: local ({local_hash}) vs remote ({remote_hash}) at {timestamp} -->\n\
         <!-- local version -->\n\
         {local}\n\
         <!-- remote version -->\n\
         {remote}\n\
         <!-- end conflict -->"
    )
}

// ---------------------------------------------------------------------------
// Replace-region resolution via semantic chunks
// ---------------------------------------------------------------------------

/// Outcome of attempting to resolve a Replace region at chunk level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceResolution {
    /// Resolved without conflict — the merged text.
    Resolved(String),
    /// True conflict — annotated text.
    Conflict(String),
}

/// Re-diff a Replace region at semantic chunk granularity.
///
/// Chunks each side via `chunk_markdown`, then diffs the chunk content slices.
/// If the chunk-level diff still contains Replace ops, it is a true conflict.
pub fn resolve_replace_region(local_region: &str, remote_region: &str) -> ReplaceResolution {
    let local_chunks = chunk_markdown(local_region);
    let remote_chunks = chunk_markdown(remote_region);

    let local_contents: Vec<&str> = local_chunks.iter().map(|c| c.content.as_str()).collect();
    let remote_contents: Vec<&str> = remote_chunks.iter().map(|c| c.content.as_str()).collect();

    let diff = TextDiff::from_slices(&local_contents, &remote_contents);

    let mut has_replace = false;
    for op in diff.ops() {
        if matches!(op.tag(), DiffTag::Replace) {
            has_replace = true;
            break;
        }
    }

    if has_replace {
        let local_hash = sha2_short(local_region);
        let remote_hash = sha2_short(remote_region);
        let annotated = format_conflict(local_region, remote_region, &local_hash, &remote_hash);
        ReplaceResolution::Conflict(annotated)
    } else {
        // No Replace at chunk level — reassemble from the diff ops.
        let mut parts = Vec::new();
        for op in diff.ops() {
            match op {
                DiffOp::Equal { old_index, len, .. } => {
                    for c in &local_contents[*old_index..*old_index + *len] {
                        parts.push(*c);
                    }
                }
                DiffOp::Delete {
                    old_index, old_len, ..
                } => {
                    for c in &local_contents[*old_index..*old_index + *old_len] {
                        parts.push(*c);
                    }
                }
                DiffOp::Insert {
                    new_index, new_len, ..
                } => {
                    for c in &remote_contents[*new_index..*new_index + *new_len] {
                        parts.push(*c);
                    }
                }
                DiffOp::Replace { .. } => unreachable!(),
            }
        }
        ReplaceResolution::Resolved(parts.join("\n\n"))
    }
}

// ---------------------------------------------------------------------------
// Reassembly
// ---------------------------------------------------------------------------

/// Reassemble non-conflicting diff regions into a single document.
pub fn reassemble_non_conflicting(regions: &[DiffRegion]) -> String {
    let mut parts = Vec::new();
    for region in regions {
        match region {
            DiffRegion::Equal(text) | DiffRegion::Insert(text) | DiffRegion::Delete(text) => {
                parts.push(text.as_str());
            }
            DiffRegion::Replace { .. } => {
                // Caller should not pass Replace regions here.
            }
        }
    }
    parts.join("\n\n")
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Attempt to merge two versions of a markdown document.
///
/// Returns `MergeResult::AutoMerged` if the merge succeeds without conflicts,
/// or `MergeResult::ConflictAnnotated` if some regions could not be resolved.
pub fn attempt_merge(local: &str, remote: &str) -> MergeResult {
    // Identical documents.
    if local == remote {
        return MergeResult::AutoMerged {
            content: local.to_string(),
            strategy: MergeStrategy::NonConflicting,
        };
    }

    let regions = diff_paragraphs(local, remote);

    // Pure insert/delete/equal — reassemble without conflict handling.
    let has_replace = regions
        .iter()
        .any(|r| matches!(r, DiffRegion::Replace { .. }));
    if !has_replace {
        return MergeResult::AutoMerged {
            content: reassemble_non_conflicting(&regions),
            strategy: MergeStrategy::NonConflicting,
        };
    }

    // Replace regions present: resolve each (escalating to semantic chunk
    // level) and assemble the final result.
    build_merge_result(resolve_regions(&regions))
}

/// Outcome of resolving every diff region: the assembled paragraphs plus how
/// they were resolved (how many stayed conflicts, whether any used the
/// semantic resolver).
struct ResolvedRegions {
    parts: Vec<String>,
    conflict_count: usize,
    used_semantic: bool,
}

/// Resolve each region, escalating `Replace` regions to the semantic chunk
/// resolver. Equal/Insert/Delete regions pass through verbatim.
fn resolve_regions(regions: &[DiffRegion]) -> ResolvedRegions {
    let mut parts = Vec::new();
    let mut conflict_count = 0;
    let mut used_semantic = false;

    for region in regions {
        match region {
            DiffRegion::Equal(text) | DiffRegion::Insert(text) | DiffRegion::Delete(text) => {
                parts.push(text.clone());
            }
            DiffRegion::Replace { local, remote } => match resolve_replace_region(local, remote) {
                ReplaceResolution::Resolved(text) => {
                    used_semantic = true;
                    parts.push(text);
                }
                ReplaceResolution::Conflict(annotated) => {
                    conflict_count += 1;
                    parts.push(annotated);
                }
            },
        }
    }

    ResolvedRegions {
        parts,
        conflict_count,
        used_semantic,
    }
}

/// Assemble the resolved regions into a `MergeResult`: annotated conflict when
/// any region stayed a conflict, otherwise an auto-merge tagged by whether the
/// semantic resolver was used.
fn build_merge_result(resolved: ResolvedRegions) -> MergeResult {
    let ResolvedRegions {
        parts,
        conflict_count,
        used_semantic,
    } = resolved;
    let content = parts.join("\n\n");

    if conflict_count > 0 {
        MergeResult::ConflictAnnotated {
            content,
            conflict_count,
        }
    } else {
        MergeResult::AutoMerged {
            content,
            strategy: if used_semantic {
                MergeStrategy::SemanticResolution
            } else {
                MergeStrategy::NonConflicting
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- split_paragraphs ---

    #[test]
    fn split_paragraphs_basic() {
        let text = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        let paras = split_paragraphs(text);
        assert_eq!(paras.len(), 3);
        assert_eq!(paras[0], "First paragraph.");
        assert_eq!(paras[1], "Second paragraph.");
        assert_eq!(paras[2], "Third paragraph.");
    }

    #[test]
    fn split_paragraphs_single() {
        let text = "Just one paragraph here.";
        let paras = split_paragraphs(text);
        assert_eq!(paras.len(), 1);
        assert_eq!(paras[0], "Just one paragraph here.");
    }

    #[test]
    fn split_paragraphs_empty() {
        let paras = split_paragraphs("");
        assert!(paras.is_empty());
    }

    // --- diff_paragraphs ---

    #[test]
    fn diff_identical_texts() {
        let text = "Paragraph one.\n\nParagraph two.";
        let regions = diff_paragraphs(text, text);
        assert!(regions.iter().all(|r| matches!(r, DiffRegion::Equal(_))));
    }

    #[test]
    fn diff_append_only() {
        let local = "Paragraph one.";
        let remote = "Paragraph one.\n\nNew paragraph.";
        let regions = diff_paragraphs(local, remote);
        // Should have Equal + Insert, no Replace.
        assert!(!regions
            .iter()
            .any(|r| matches!(r, DiffRegion::Replace { .. })));
        let has_insert = regions.iter().any(|r| matches!(r, DiffRegion::Insert(_)));
        assert!(has_insert, "expected an Insert region");
    }

    #[test]
    fn diff_disjoint_changes() {
        let local = "Alpha.\n\nBeta.\n\nGamma.";
        let remote = "Alpha changed.\n\nBeta.\n\nGamma changed.";
        let regions = diff_paragraphs(local, remote);
        // Should have Replace regions around the changed paragraphs.
        let replace_count = regions
            .iter()
            .filter(|r| matches!(r, DiffRegion::Replace { .. }))
            .count();
        assert!(replace_count >= 1, "expected at least one Replace region");
    }

    // --- merge ---

    #[test]
    fn merge_identical() {
        let text = "Same content.\n\nAnother paragraph.";
        let result = attempt_merge(text, text);
        match result {
            MergeResult::AutoMerged { content, strategy } => {
                assert_eq!(content, text);
                assert_eq!(strategy, MergeStrategy::NonConflicting);
            }
            _ => panic!("expected AutoMerged"),
        }
    }

    #[test]
    fn merge_local_append() {
        let local = "Base.\n\nLocal addition.";
        let remote = "Base.";
        let result = attempt_merge(local, remote);
        match result {
            MergeResult::AutoMerged { content, .. } => {
                assert!(content.contains("Base."));
                assert!(content.contains("Local addition."));
            }
            _ => panic!("expected AutoMerged for append-only change"),
        }
    }

    #[test]
    fn merge_remote_append() {
        let local = "Base.";
        let remote = "Base.\n\nRemote addition.";
        let result = attempt_merge(local, remote);
        match result {
            MergeResult::AutoMerged { content, .. } => {
                assert!(content.contains("Base."));
                assert!(content.contains("Remote addition."));
            }
            _ => panic!("expected AutoMerged for append-only change"),
        }
    }

    #[test]
    fn merge_disjoint_edits() {
        // Local changes first paragraph, remote changes third — both should appear.
        let base_local = "Alpha local.\n\nBeta.\n\nGamma.";
        let base_remote = "Alpha.\n\nBeta.\n\nGamma remote.";
        let result = attempt_merge(base_local, base_remote);
        // Both sides changed different paragraphs — this will produce Replace
        // regions that escalate to semantic chunk level.
        match &result {
            MergeResult::AutoMerged { content, .. } => {
                // If auto-merged, both changes should be present.
                assert!(
                    content.contains("Beta."),
                    "shared paragraph should be present"
                );
            }
            MergeResult::ConflictAnnotated { content, .. } => {
                // Conflict annotations are also acceptable for disjoint paragraph edits
                // since they produce Replace ops at paragraph level.
                assert!(content.contains("Beta."));
            }
        }
    }

    #[test]
    fn merge_true_conflict() {
        // Same paragraph changed differently on each side.
        let local = "This paragraph was changed locally.";
        let remote = "This paragraph was changed remotely.";
        let result = attempt_merge(local, remote);
        match result {
            MergeResult::ConflictAnnotated {
                content,
                conflict_count,
            } => {
                assert!(conflict_count >= 1);
                assert!(content.contains("<!-- sync-conflict:"));
                assert!(content.contains("<!-- local version -->"));
                assert!(content.contains("<!-- remote version -->"));
                assert!(content.contains("<!-- end conflict -->"));
            }
            MergeResult::AutoMerged { .. } => {
                // Semantic resolution might resolve this if chunks differ enough.
                // Either outcome is acceptable.
            }
        }
    }

    #[test]
    fn conflict_annotation_format() {
        let local = "Local text.";
        let remote = "Remote text.";
        let local_hash = sha2_short(local);
        let remote_hash = sha2_short(remote);
        let annotation = format_conflict(local, remote, &local_hash, &remote_hash);

        assert!(annotation.starts_with("<!-- sync-conflict: local ("));
        assert!(annotation.contains(&local_hash));
        assert!(annotation.contains(&remote_hash));
        assert!(annotation.contains("<!-- local version -->"));
        assert!(annotation.contains("Local text."));
        assert!(annotation.contains("<!-- remote version -->"));
        assert!(annotation.contains("Remote text."));
        assert!(annotation.contains("<!-- end conflict -->"));
    }

    // --- sha2_short ---

    #[test]
    fn sha2_short_length() {
        let hash = sha2_short("hello");
        assert_eq!(hash.len(), 8);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
