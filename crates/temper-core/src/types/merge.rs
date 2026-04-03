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
