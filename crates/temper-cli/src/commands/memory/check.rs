//! `temper memory check` — the LOCAL drift gate.
//!
//! Why local, not CI: the rendered index lives at `~/.claude/projects/<project>/memory/MEMORY.md`
//! — outside the repo, and per-machine. `.github/scripts/check-skills-drift.sh` (the sibling gate
//! for the `agent-skills/` projection) works only because that tree is tracked by git, so CI can
//! diff committed-vs-regenerated. There is nothing here for CI to diff; this is a command a
//! person or a hook runs, and its exit code is the gate (`main.rs` maps `Drifted` to
//! `std::process::exit(1)`).
//!
//! [`compare_index`] is the pure core: given a fresh render and what (if anything) is on disk, it
//! decides the verdict with no I/O and no clock, so it is unit-testable without a filesystem.
//! [`check`] is the async I/O shell: it re-renders through [`super::emit::build_index`] — the
//! same function `emit` uses — so a malformed memory fails `check` too, deliberately: `emit` is
//! the only enforcement point for `open_meta.status`/`open_meta.verified`, and `check` renders
//! through that same gate rather than growing a second one.

use temper_core::types::config::TemperConfig;

use super::emit::render_current;
use crate::error::{Result, TemperError};

/// The result of comparing a fresh render against what is on disk.
///
/// `Absent` and `Drifted` are deliberately distinct: a machine that has never run
/// `temper memory emit` has not had its index tampered with, and collapsing the two would tell an
/// adopting user their (nonexistent) index was hand-edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftVerdict {
    /// The on-disk file is byte-identical to a fresh render.
    Match,
    /// The on-disk file exists and differs from a fresh render — `diff` is a unified diff, on-disk
    /// first, fresh render second.
    Drifted { diff: String },
    /// No file is on disk yet. Not drift — nothing has ever been emitted to drift from.
    Absent,
}

/// Pure comparison: no I/O, no clock. `on_disk` is `None` when the index file does not exist.
pub fn compare_index(rendered: &str, on_disk: Option<&str>) -> DriftVerdict {
    match on_disk {
        None => DriftVerdict::Absent,
        Some(existing) if existing == rendered => DriftVerdict::Match,
        Some(existing) => {
            let diff = similar::TextDiff::from_lines(existing, rendered)
                .unified_diff()
                .header("on-disk", "fresh render")
                .to_string();
            DriftVerdict::Drifted { diff }
        }
    }
}

/// The async I/O shell. Assumes the caller has already checked `[memory]` is configured (the CLI
/// dispatch does, mirroring `emit`'s `EmitOutcome` gate), but re-checks defensively rather than
/// trusting the caller silently — same posture as `emit`.
///
/// `path_override` mirrors `emit`'s `--path`: a machine mid-adoption that ran
/// `emit --path <p>` must be able to gate on that same file, not the configured `index_path` —
/// otherwise the exit code (`main.rs` maps `Drifted` to `process::exit(1)`) is a verdict about a
/// different file than the one that was written.
pub async fn check(config: &TemperConfig, path_override: Option<&str>) -> Result<DriftVerdict> {
    let mem = config.memory.as_ref().ok_or_else(|| {
        TemperError::Config(
            "no [memory] section in config.toml — the memory projection is off".to_string(),
        )
    })?;

    let (rendered, path) = render_current(mem, path_override).await?;
    let on_disk = std::fs::read_to_string(&path).ok();

    Ok(compare_index(&rendered, on_disk.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_reports_match_when_the_file_equals_a_fresh_render() {
        let rendered = "…";
        assert_eq!(compare_index(rendered, Some(rendered)), DriftVerdict::Match);
    }

    #[test]
    fn check_reports_drift_when_the_file_was_hand_edited() {
        let v = compare_index("generated", Some("generated + a human edit"));
        assert!(matches!(v, DriftVerdict::Drifted { .. }));
    }

    /// **The gate must survive the union.** A hand edit to an un-migrated entry is the one drift
    /// this design could plausibly have gone blind to: had the render harvested its titles from
    /// the index, the re-render would have read the edit back in and reproduced it as a `Match`.
    /// It does not, because `build_local_index` is given the files and has no way to reach the
    /// index at all — so the fresh render still carries the file's title, and the edit shows up.
    ///
    /// This mutates the *feature*, not a detail of it: with the union removed the local line
    /// would not be in either string and the assertion could not discriminate.
    #[test]
    fn a_hand_edited_unmigrated_entry_still_drifts() {
        use super::super::emit::{build_index, build_local_index};
        use super::super::migrate::ScannedFile;
        use temper_core::types::config::MemoryConfig;

        let cfg = MemoryConfig {
            shared_contexts: vec![],
            project_contexts: vec!["@me/temper".to_string()],
            index_path: "~/x/MEMORY.md".to_string(),
            stale_after_days: 90,
            reinforced_min: None,
        };
        let scanned = [ScannedFile {
            filename: "project_x.md".to_string(),
            content: "---\ntitle: the file's title\nname: n\ndescription: d\nmetadata:\n  type: project\n---\n\nbody\n"
                .to_string(),
            mtime: chrono::NaiveDate::from_ymd_opt(2026, 7, 14).expect("date"),
        }];
        let local = build_local_index(&scanned, &std::collections::HashSet::new());
        let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 1).expect("date");

        let rendered = build_index(&cfg, &[], &local, &[], today).expect("must render");
        assert!(
            rendered.contains("the file's title"),
            "the union must actually be in force for this test to discriminate: {rendered}"
        );

        let hand_edited = rendered.replace("the file's title", "a title someone typed");

        assert!(
            matches!(
                compare_index(&rendered, Some(&hand_edited)),
                DriftVerdict::Drifted { .. }
            ),
            "an edit to an un-migrated entry must be visible to the gate, not absorbed by it"
        );
    }

    #[test]
    fn check_reports_absent_rather_than_drifted_when_there_is_no_file() {
        assert_eq!(
            compare_index("generated", None),
            DriftVerdict::Absent,
            "a machine that has never emitted has not DRIFTED; the two must not be conflated"
        );
    }
}
