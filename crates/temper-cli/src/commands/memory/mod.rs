//! `temper memory` — Claude Code's `MEMORY.md` as a rendered projection of
//! `memory`-typed Temper resources.

use std::path::PathBuf;

use temper_core::types::config::{expand_tilde, MemoryConfig};

pub mod check;
pub mod emit;
mod fetch;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> MemoryConfig {
        MemoryConfig {
            shared_contexts: vec![],
            project_contexts: vec!["@me/temper".to_string()],
            index_path: "~/.claude/projects/p/memory/MEMORY.md".to_string(),
            stale_after_days: 90,
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
}
