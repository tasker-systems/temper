//! `temper memory` — the harness-loaded index (`MEMORY.md` and siblings) as a rendered
//! projection of `memory`-typed Temper resources. The projection writes files; whichever
//! harness config points a session at them decides how they load.

use std::collections::HashMap;
use std::path::PathBuf;

use temper_core::types::config::{expand_tilde, MemoryConfig};

use crate::error::{Result, TemperError};

pub mod check;
pub mod emit;
mod fetch;
pub mod harvest;
pub mod migrate;
pub mod render;
pub mod status;

pub use check::check;
pub use emit::emit;
pub use status::status;

/// Resolve the effective index path: `path_override` when given, else `mem.index_path`,
/// tilde-expanded. Shared by `emit` (what it writes) and `check` (what it reads and diffs
/// against) so a caller running `emit --path <p>` then `check --path <p>` always gets a verdict
/// about the same file — the two must never independently drift on how an override is applied.
pub fn resolve_index_path(mem: &MemoryConfig, path_override: Option<&str>) -> PathBuf {
    let path_str = path_override.unwrap_or(mem.index_path.as_str());
    expand_tilde(path_str)
}

/// Read the memory index and harvest its link titles, refusing to read an
/// unreadable index as "no titles".
///
/// An index that reads back empty is a genuine reading — no links, so no
/// titles — and proceeds. A read that fails (absent file, misconfigured path,
/// undecodable bytes) is not: treating it as empty fabricates the answer
/// "nothing in the index has a title" for every file the scan found, which
/// `harvest` would report per file as a skip and `migrate` would act on on its
/// way to writing.
pub(crate) fn read_index_titles(index_path: &std::path::Path) -> Result<HashMap<String, String>> {
    let content = std::fs::read_to_string(index_path).map_err(|e| {
        TemperError::Config(format!(
            "cannot read memory index {}: {e}",
            index_path.display()
        ))
    })?;
    Ok(migrate::harvest_titles(&content))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> MemoryConfig {
        MemoryConfig {
            shared_contexts: vec![],
            project_contexts: vec!["@me/temper".to_string()],
            index_path: "~/.claude/projects/p/memory/MEMORY.md".to_string(),
            shared_index_path: None,
            stale_after_days: 90,
            reinforced_min: None,
        }
    }

    #[test]
    fn resolve_index_path_uses_configured_index_path_when_no_override() {
        let path = resolve_index_path(&cfg(), None);
        assert_eq!(path, expand_tilde("~/.claude/projects/p/memory/MEMORY.md"));
    }

    #[test]
    fn resolve_index_path_prefers_the_override() {
        let path = resolve_index_path(&cfg(), Some("/tmp/OTHER.md"));
        assert_eq!(path, PathBuf::from("/tmp/OTHER.md"));
    }

    /// An index that cannot be read must be refused, naming the configured
    /// path — an empty title map here would fabricate "no link names any file"
    /// for a whole run of `harvest`/`migrate`.
    #[test]
    fn unreadable_memory_index_is_refused_not_read_as_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("MEMORY.md");

        let err = read_index_titles(&missing).expect_err("an unreadable index must be refused");

        let msg = format!("{err}");
        assert!(
            msg.contains(missing.to_str().unwrap()),
            "the refusal must name the index path: {msg}"
        );
    }

    /// The other half: an index that reads back empty is a genuine reading of
    /// its content — no links, no titles — and must proceed, not refuse.
    #[test]
    fn a_genuinely_empty_index_reads_as_no_titles_and_proceeds() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = dir.path().join("MEMORY.md");
        std::fs::write(&index, "").unwrap();

        let titles = read_index_titles(&index).expect("an empty index is a real reading");

        assert!(titles.is_empty());
    }
}
