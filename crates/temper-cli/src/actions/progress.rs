//! Progress reporting for sync operations.
//!
//! Provides a [`SyncProgress`] trait with three implementations:
//! - [`NoopProgress`] — does nothing (useful as a default)
//! - [`CollectingProgress`] — stores events for test assertions
//! - [`TerminalProgress`] — writes styled output via `crate::output`

use temper_core::types::{MergeResult, PushKind};

use crate::output;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Callback interface for reporting sync progress events.
pub trait SyncProgress {
    /// A vault file was found during scanning.
    fn scan_found(&self, path: &str, context: &str, doc_type: &str);

    /// A vault file was skipped during scanning.
    fn scan_skipped(&self, path: &str, reason: &str);

    /// Progress through the rehash phase.
    fn rehash_progress(&self, processed: usize, total: usize, skipped_by_mtime: usize);

    /// A push operation is starting.
    fn push_start(&self, path: &str, kind: PushKind);

    /// A push operation completed.
    fn push_done(&self, path: &str);

    /// A pull operation is starting.
    fn pull_start(&self, path: &str);

    /// A pull operation completed.
    fn pull_done(&self, path: &str);

    /// A merge result was produced.
    fn merge_result(&self, path: &str, outcome: &MergeResult);

    /// Summary for a completed sync phase.
    fn phase_summary(&self, phase: &str, count: usize);
}

// ---------------------------------------------------------------------------
// NoopProgress
// ---------------------------------------------------------------------------

/// A no-op progress reporter — all methods are empty.
pub struct NoopProgress;

impl SyncProgress for NoopProgress {
    fn scan_found(&self, _path: &str, _context: &str, _doc_type: &str) {}
    fn scan_skipped(&self, _path: &str, _reason: &str) {}
    fn rehash_progress(&self, _processed: usize, _total: usize, _skipped_by_mtime: usize) {}
    fn push_start(&self, _path: &str, _kind: PushKind) {}
    fn push_done(&self, _path: &str) {}
    fn pull_start(&self, _path: &str) {}
    fn pull_done(&self, _path: &str) {}
    fn merge_result(&self, _path: &str, _outcome: &MergeResult) {}
    fn phase_summary(&self, _phase: &str, _count: usize) {}
}

// ---------------------------------------------------------------------------
// CollectingProgress
// ---------------------------------------------------------------------------

/// A recorded progress event — used for test assertions.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    ScanFound {
        path: String,
        context: String,
        doc_type: String,
    },
    ScanSkipped {
        path: String,
        reason: String,
    },
    RehashProgress {
        processed: usize,
        total: usize,
        skipped_by_mtime: usize,
    },
    PushStart {
        path: String,
        kind: PushKind,
    },
    PushDone {
        path: String,
    },
    PullStart {
        path: String,
    },
    PullDone {
        path: String,
    },
    MergeResult {
        path: String,
        auto_merged: bool,
        conflict_count: Option<usize>,
    },
    PhaseSummary {
        phase: String,
        count: usize,
    },
}

/// A progress reporter that collects events for test assertions.
pub struct CollectingProgress {
    events: std::sync::Mutex<Vec<ProgressEvent>>,
}

impl CollectingProgress {
    pub fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Return a snapshot of collected events.
    pub fn events(&self) -> Vec<ProgressEvent> {
        self.events
            .lock()
            .expect("collecting progress mutex poisoned")
            .clone()
    }
}

impl Default for CollectingProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncProgress for CollectingProgress {
    fn scan_found(&self, path: &str, context: &str, doc_type: &str) {
        self.events
            .lock()
            .expect("mutex poisoned")
            .push(ProgressEvent::ScanFound {
                path: path.to_string(),
                context: context.to_string(),
                doc_type: doc_type.to_string(),
            });
    }

    fn scan_skipped(&self, path: &str, reason: &str) {
        self.events
            .lock()
            .expect("mutex poisoned")
            .push(ProgressEvent::ScanSkipped {
                path: path.to_string(),
                reason: reason.to_string(),
            });
    }

    fn rehash_progress(&self, processed: usize, total: usize, skipped_by_mtime: usize) {
        self.events
            .lock()
            .expect("mutex poisoned")
            .push(ProgressEvent::RehashProgress {
                processed,
                total,
                skipped_by_mtime,
            });
    }

    fn push_start(&self, path: &str, kind: PushKind) {
        self.events
            .lock()
            .expect("mutex poisoned")
            .push(ProgressEvent::PushStart {
                path: path.to_string(),
                kind,
            });
    }

    fn push_done(&self, path: &str) {
        self.events
            .lock()
            .expect("mutex poisoned")
            .push(ProgressEvent::PushDone {
                path: path.to_string(),
            });
    }

    fn pull_start(&self, path: &str) {
        self.events
            .lock()
            .expect("mutex poisoned")
            .push(ProgressEvent::PullStart {
                path: path.to_string(),
            });
    }

    fn pull_done(&self, path: &str) {
        self.events
            .lock()
            .expect("mutex poisoned")
            .push(ProgressEvent::PullDone {
                path: path.to_string(),
            });
    }

    fn merge_result(&self, path: &str, outcome: &MergeResult) {
        let (auto_merged, conflict_count) = match outcome {
            MergeResult::AutoMerged { .. } => (true, None),
            MergeResult::ConflictAnnotated { conflict_count, .. } => (false, Some(*conflict_count)),
        };
        self.events
            .lock()
            .expect("mutex poisoned")
            .push(ProgressEvent::MergeResult {
                path: path.to_string(),
                auto_merged,
                conflict_count,
            });
    }

    fn phase_summary(&self, phase: &str, count: usize) {
        self.events
            .lock()
            .expect("mutex poisoned")
            .push(ProgressEvent::PhaseSummary {
                phase: phase.to_string(),
                count,
            });
    }
}

// ---------------------------------------------------------------------------
// TerminalProgress
// ---------------------------------------------------------------------------

/// A progress reporter that writes styled output to the terminal.
pub struct TerminalProgress {
    use_progress: bool,
}

impl TerminalProgress {
    pub fn new() -> Self {
        use std::io::IsTerminal as _;
        Self {
            use_progress: std::io::stderr().is_terminal(),
        }
    }
}

impl Default for TerminalProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncProgress for TerminalProgress {
    fn scan_found(&self, path: &str, context: &str, doc_type: &str) {
        output::success(format!("found {path} [{context}/{doc_type}]"));
    }

    fn scan_skipped(&self, path: &str, reason: &str) {
        output::warning(format!("skipped {path}: {reason}"));
    }

    fn rehash_progress(&self, processed: usize, total: usize, skipped_by_mtime: usize) {
        if self.use_progress {
            output::dim(format!(
                "rehash {processed}/{total} ({skipped_by_mtime} skipped by mtime)"
            ));
        }
    }

    fn push_start(&self, path: &str, kind: PushKind) {
        output::item(format!("push [{kind}] {path}"));
    }

    fn push_done(&self, path: &str) {
        output::item(format!("pushed {path}"));
    }

    fn pull_start(&self, path: &str) {
        output::item(format!("pull {path}"));
    }

    fn pull_done(&self, path: &str) {
        output::item(format!("pulled {path}"));
    }

    fn merge_result(&self, path: &str, outcome: &MergeResult) {
        match outcome {
            MergeResult::AutoMerged { strategy, .. } => {
                output::item(format!("auto-merged {path} [{strategy:?}]"));
            }
            MergeResult::ConflictAnnotated { conflict_count, .. } => {
                output::warning(format!(
                    "conflict-annotated {path} ({conflict_count} conflict(s))"
                ));
            }
        }
    }

    fn phase_summary(&self, phase: &str, count: usize) {
        output::header(format!("{phase}: {count}"));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::MergeStrategy;

    #[test]
    fn noop_progress_compiles_and_runs() {
        let p = NoopProgress;
        p.scan_found("a.md", "temper", "task");
        p.scan_skipped("b.md", "no frontmatter");
        p.rehash_progress(1, 10, 3);
        p.push_start("c.md", PushKind::New);
        p.push_done("c.md");
        p.pull_start("d.md");
        p.pull_done("d.md");
        p.merge_result(
            "e.md",
            &MergeResult::AutoMerged {
                content: String::new(),
                strategy: MergeStrategy::NonConflicting,
            },
        );
        p.phase_summary("push", 5);
    }

    #[test]
    fn collecting_progress_records_scan_found() {
        let p = CollectingProgress::new();
        p.scan_found("notes/task.md", "notes", "task");
        let events = p.events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ProgressEvent::ScanFound {
                path,
                context,
                doc_type,
            } => {
                assert_eq!(path, "notes/task.md");
                assert_eq!(context, "notes");
                assert_eq!(doc_type, "task");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn collecting_progress_records_merge_auto_merged() {
        let p = CollectingProgress::new();
        p.merge_result(
            "doc.md",
            &MergeResult::AutoMerged {
                content: "merged".to_string(),
                strategy: MergeStrategy::NonConflicting,
            },
        );
        let events = p.events();
        match &events[0] {
            ProgressEvent::MergeResult {
                auto_merged,
                conflict_count,
                ..
            } => {
                assert!(*auto_merged);
                assert!(conflict_count.is_none());
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn collecting_progress_records_merge_conflict_annotated() {
        let p = CollectingProgress::new();
        p.merge_result(
            "doc.md",
            &MergeResult::ConflictAnnotated {
                content: "<<<< conflict >>>>".to_string(),
                conflict_count: 2,
            },
        );
        let events = p.events();
        match &events[0] {
            ProgressEvent::MergeResult {
                auto_merged,
                conflict_count,
                ..
            } => {
                assert!(!*auto_merged);
                assert_eq!(*conflict_count, Some(2));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn collecting_progress_records_phase_summary() {
        let p = CollectingProgress::new();
        p.phase_summary("pull", 7);
        let events = p.events();
        match &events[0] {
            ProgressEvent::PhaseSummary { phase, count } => {
                assert_eq!(phase, "pull");
                assert_eq!(*count, 7);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn collecting_progress_records_multiple_events_in_order() {
        let p = CollectingProgress::new();
        p.push_start("a.md", PushKind::Modified);
        p.push_done("a.md");
        p.pull_start("b.md");
        p.pull_done("b.md");
        let events = p.events();
        assert_eq!(events.len(), 4);
        assert!(matches!(events[0], ProgressEvent::PushStart { .. }));
        assert!(matches!(events[1], ProgressEvent::PushDone { .. }));
        assert!(matches!(events[2], ProgressEvent::PullStart { .. }));
        assert!(matches!(events[3], ProgressEvent::PullDone { .. }));
    }
}
