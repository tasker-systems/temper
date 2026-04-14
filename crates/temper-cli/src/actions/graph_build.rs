//! `temper graph build` pipeline implementation.
//!
//! Three-pass additive seeder that walks the vault, scans markdown
//! bodies for explicit references (markdown links, wikilinks, bare
//! UUIDs), resolves them within-owner, and writes the resolved set
//! back into each file's `open_meta.references`.
//!
//! Owner boundaries are enforced by map partitioning: every resolution
//! map is keyed by owner, and a scanning file can only look inside
//! the map for its own owner. Cross-owner references are structurally
//! impossible.

use std::collections::HashMap;
use std::path::PathBuf;

use uuid::Uuid;

use crate::config::Config;
use crate::error::Result;

/// Doc types that live at `{vault}/{owner}/{context}/{doc_type}/`.
/// Matches `actions::doctor::ENTITY_DOC_TYPES`.
const ENTITY_DOC_TYPES: &[&str] = &["task", "goal", "session", "decision", "concept", "research"];

/// Parameters for a graph build run.
#[derive(Debug, Clone)]
pub struct GraphBuildParams {
    /// Optional single-context filter. None means all configured contexts.
    pub context_filter: Option<String>,
    /// If true, do not write any files; report what would change.
    pub dry_run: bool,
    /// If true, emit per-file detail in the report.
    pub verbose: bool,
}

/// Final report from a graph build run.
#[derive(Debug, Default, Clone)]
pub struct GraphBuildReport {
    pub files_walked: usize,
    pub references_found: usize,
    pub files_modified: usize,
    pub references_added: usize,
    pub already_present: usize,
    pub modified_files: Vec<ModifiedFile>,
}

/// Per-file change record for the report.
#[derive(Debug, Clone)]
pub struct ModifiedFile {
    pub rel_path: String,
    pub added: usize,
    /// Only populated when verbose = true
    pub added_refs: Vec<String>,
}

/// Owner sigil-prefixed identifier, e.g. "@me" or "+platform-eng".
pub type Owner = String;
/// Context name, e.g. "temper", "tasker".
pub type Context = String;

/// Slug resolution maps, partitioned by owner AND context for
/// same-context-first resolution. A slug lookup walks "same context
/// first, then cross-context if unique" — never crossing the owner
/// boundary.
#[derive(Debug, Default)]
pub(crate) struct SlugMap {
    inner: HashMap<Owner, HashMap<Context, HashMap<String, PathBuf>>>,
}

/// UUID resolution map, partitioned by owner only (UUIDs are globally
/// unique within the vault and do not need context partitioning).
#[derive(Debug, Default)]
pub(crate) struct UuidMap {
    inner: HashMap<Owner, HashMap<Uuid, PathBuf>>,
}

impl SlugMap {
    /// Register a file at `(owner, context, slug)`.
    pub(crate) fn insert(&mut self, owner: &str, context: &str, slug: &str, path: PathBuf) {
        self.inner
            .entry(owner.to_string())
            .or_default()
            .entry(context.to_string())
            .or_default()
            .insert(slug.to_string(), path);
    }

    /// Resolve a slug for a scanning file.
    ///
    /// - Same-owner same-context: direct match wins.
    /// - Same-owner cross-context: falls back ONLY if exactly one
    ///   other context in the owner contains the slug. Ambiguous
    ///   matches return `None` with a debug trace.
    /// - Cross-owner: never resolves.
    pub(crate) fn resolve(
        &self,
        scanning_owner: &str,
        scanning_context: &str,
        slug: &str,
    ) -> Option<&std::path::Path> {
        let owner_map = self.inner.get(scanning_owner)?;

        // 1. Same-context first
        if let Some(ctx_map) = owner_map.get(scanning_context) {
            if let Some(path) = ctx_map.get(slug) {
                return Some(path.as_path());
            }
        }

        // 2. Cross-context fallback — only if exactly one match exists
        let matches: Vec<&std::path::Path> = owner_map
            .iter()
            .filter(|(ctx, _)| ctx.as_str() != scanning_context)
            .filter_map(|(_, ctx_map)| ctx_map.get(slug))
            .map(|p| p.as_path())
            .collect();

        match matches.len() {
            0 => None,
            1 => Some(matches[0]),
            n => {
                tracing::debug!(
                    owner = %scanning_owner,
                    slug = %slug,
                    n_matches = n,
                    "ambiguous cross-context slug — skipping"
                );
                None
            }
        }
    }
}

impl UuidMap {
    pub(crate) fn insert(&mut self, owner: &str, id: Uuid, path: PathBuf) {
        self.inner
            .entry(owner.to_string())
            .or_default()
            .insert(id, path);
    }

    pub(crate) fn resolve(&self, scanning_owner: &str, id: Uuid) -> Option<&std::path::Path> {
        self.inner
            .get(scanning_owner)?
            .get(&id)
            .map(|p| p.as_path())
    }
}

/// Top-level entry point. Walks the vault, scans bodies, merges
/// references into open_meta, writes files back.
pub fn run(config: &Config, params: GraphBuildParams) -> Result<GraphBuildReport> {
    let _ = (config, params);
    Err(crate::error::TemperError::Project(
        "graph_build::run: not yet implemented".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn path(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn slug_map_resolves_same_context_first() {
        let mut map = SlugMap::default();
        map.insert(
            "@me",
            "temper",
            "foo",
            path("/vault/@me/temper/task/foo.md"),
        );
        map.insert(
            "@me",
            "tasker",
            "foo",
            path("/vault/@me/tasker/task/foo.md"),
        );

        let resolved = map.resolve("@me", "temper", "foo");
        assert_eq!(
            resolved,
            Some(path("/vault/@me/temper/task/foo.md").as_path())
        );
    }

    #[test]
    fn slug_map_falls_back_cross_context_when_unique() {
        let mut map = SlugMap::default();
        map.insert(
            "@me",
            "tasker",
            "only-there",
            path("/vault/@me/tasker/task/only-there.md"),
        );

        let resolved = map.resolve("@me", "temper", "only-there");
        assert_eq!(
            resolved,
            Some(path("/vault/@me/tasker/task/only-there.md").as_path())
        );
    }

    #[test]
    fn slug_map_skips_ambiguous_cross_context() {
        let mut map = SlugMap::default();
        map.insert(
            "@me",
            "tasker",
            "ambiguous",
            path("/vault/@me/tasker/task/ambiguous.md"),
        );
        map.insert(
            "@me",
            "general",
            "ambiguous",
            path("/vault/@me/general/task/ambiguous.md"),
        );

        let resolved = map.resolve("@me", "temper", "ambiguous");
        assert_eq!(resolved, None);
    }

    #[test]
    fn slug_map_rejects_cross_owner() {
        let mut map = SlugMap::default();
        map.insert(
            "+team-x",
            "shared",
            "leaked",
            path("/vault/+team-x/shared/task/leaked.md"),
        );

        let resolved = map.resolve("@me", "temper", "leaked");
        assert_eq!(resolved, None);
    }

    #[test]
    fn slug_map_returns_none_for_unknown_slug() {
        let map = SlugMap::default();
        assert_eq!(map.resolve("@me", "temper", "nonexistent"), None);
    }

    fn uuid(s: &str) -> Uuid {
        Uuid::parse_str(s).unwrap()
    }

    #[test]
    fn uuid_map_resolves_within_owner() {
        let mut map = UuidMap::default();
        let id = uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6");
        map.insert("@me", id, path("/vault/@me/temper/task/foo.md"));

        let resolved = map.resolve("@me", id);
        assert_eq!(
            resolved,
            Some(path("/vault/@me/temper/task/foo.md").as_path())
        );
    }

    #[test]
    fn uuid_map_rejects_cross_owner() {
        let mut map = UuidMap::default();
        let id = uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6");
        map.insert("+team-x", id, path("/vault/+team-x/shared/task/leaked.md"));

        let resolved = map.resolve("@me", id);
        assert_eq!(resolved, None);
    }

    #[test]
    fn uuid_map_returns_none_for_unknown_uuid() {
        let map = UuidMap::default();
        let id = uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6");
        assert_eq!(map.resolve("@me", id), None);
    }
}
