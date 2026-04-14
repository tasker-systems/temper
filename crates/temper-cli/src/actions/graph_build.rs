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

/// A file captured by the vault walk. Keeps the parsed frontmatter
/// so Pass 2 doesn't re-read it.
pub(crate) struct DiscoveredFile {
    pub(crate) path: PathBuf,
    pub(crate) rel_path: String,
    pub(crate) owner: String,
    pub(crate) context: String,
    pub(crate) frontmatter: temper_core::frontmatter::Frontmatter,
}

/// Walk the vault and build per-owner slug/UUID resolution maps.
///
/// `context_filter` restricts which files appear in the returned
/// `DiscoveredFile` list (Pass 2 only scans filtered files), but the
/// maps always include all same-owner files across all contexts so
/// cross-context same-owner references can still resolve.
pub(crate) fn discover_vault(
    config: &Config,
    context_filter: Option<&str>,
) -> Result<(SlugMap, UuidMap, Vec<DiscoveredFile>)> {
    use std::fs;
    use temper_core::frontmatter::Frontmatter;
    use temper_core::vault::Vault;

    let mut slugs = SlugMap::default();
    let mut uuids = UuidMap::default();
    let mut filtered_files: Vec<DiscoveredFile> = Vec::new();

    let vault_layout = Vault::new(&config.vault_root);

    for ctx in &config.contexts {
        let owner = config.owner_for_context(ctx);
        let include_in_scan = context_filter.map_or(true, |f| f == ctx);

        for doc_type in ENTITY_DOC_TYPES {
            let dir = vault_layout.doc_type_dir(&owner, ctx, doc_type);
            if !dir.is_dir() {
                continue;
            }

            let entries = match fs::read_dir(&dir) {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!(
                        dir = %dir.display(),
                        error = %e,
                        "could not read doc_type dir, skipping"
                    );
                    continue;
                }
            };

            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }

                let frontmatter = match Frontmatter::parse_file(&path) {
                    Ok(fm) => fm,
                    Err(e) => {
                        tracing::debug!(
                            path = %path.display(),
                            error = %e,
                            "unparseable frontmatter, skipping"
                        );
                        continue;
                    }
                };

                let slug = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };

                slugs.insert(&owner, ctx, &slug, path.clone());

                if let Some(id_str) = frontmatter
                    .value()
                    .get("temper-id")
                    .and_then(|v| v.as_str())
                {
                    if let Ok(id) = Uuid::parse_str(id_str) {
                        uuids.insert(&owner, id, path.clone());
                    }
                }

                if include_in_scan {
                    let rel_path = path
                        .strip_prefix(&config.vault_root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string();

                    filtered_files.push(DiscoveredFile {
                        path: path.clone(),
                        rel_path,
                        owner: owner.clone(),
                        context: ctx.clone(),
                        frontmatter,
                    });
                }
            }
        }
    }

    filtered_files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    Ok((slugs, uuids, filtered_files))
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

    use std::fs;
    use tempfile::TempDir;

    /// Create a minimal vault structure under `tmp` and write a file
    /// with valid frontmatter. Returns the absolute file path.
    fn write_vault_file(
        tmp: &TempDir,
        owner: &str,
        context: &str,
        doc_type: &str,
        slug: &str,
        temper_id: Option<&str>,
        body: &str,
    ) -> PathBuf {
        let dir = tmp.path().join(owner).join(context).join(doc_type);
        fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join(format!("{slug}.md"));
        let id_line = temper_id
            .map(|id| format!("temper-id: {id}\n"))
            .unwrap_or_default();
        let content = format!(
            "---\n\
             temper-context: {context}\n\
             temper-type: {doc_type}\n\
             temper-owner: '{owner}'\n\
             {id_line}\
             title: {slug}\n\
             slug: {slug}\n\
             ---\n\
             {body}\n"
        );
        fs::write(&file_path, content).unwrap();
        file_path
    }

    fn fixture_config(tmp: &TempDir, contexts: &[&str]) -> Config {
        Config {
            vault_root: tmp.path().to_path_buf(),
            state_dir: tmp.path().join(".temper"),
            contexts: contexts.iter().map(|s| s.to_string()).collect(),
            subscriptions: Vec::new(),
            skill_output: tmp.path().join(".skill"),
        }
    }

    #[test]
    fn discover_vault_builds_slug_and_uuid_maps() {
        let tmp = TempDir::new().unwrap();
        write_vault_file(
            &tmp,
            "@me",
            "temper",
            "task",
            "alpha",
            Some("019d1d24-2000-7379-8f26-ae4ae87bc5c6"),
            "body of alpha",
        );
        write_vault_file(&tmp, "@me", "tasker", "task", "beta", None, "body of beta");
        let config = fixture_config(&tmp, &["temper", "tasker"]);

        let (slugs, uuids, files) = discover_vault(&config, None).unwrap();

        assert_eq!(files.len(), 2, "expected 2 files in walk");
        assert!(slugs.resolve("@me", "temper", "alpha").is_some());
        assert!(slugs.resolve("@me", "tasker", "beta").is_some());

        let alpha_uuid = uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6");
        assert!(uuids.resolve("@me", alpha_uuid).is_some());
    }

    #[test]
    fn discover_vault_skips_unparseable_files_silently() {
        let tmp = TempDir::new().unwrap();
        write_vault_file(&tmp, "@me", "temper", "task", "good", None, "");
        let bad_dir = tmp.path().join("@me").join("temper").join("task");
        fs::write(
            bad_dir.join("bad.md"),
            "not a real frontmatter\nno yaml here\n",
        )
        .unwrap();

        let config = fixture_config(&tmp, &["temper"]);
        let result = discover_vault(&config, None);

        assert!(result.is_ok(), "unparseable files should not fail the walk");
        let (slugs, _uuids, files) = result.unwrap();

        assert_eq!(files.len(), 1);
        assert!(slugs.resolve("@me", "temper", "good").is_some());
        assert!(slugs.resolve("@me", "temper", "bad").is_none());
    }

    #[test]
    fn discover_vault_respects_context_filter() {
        let tmp = TempDir::new().unwrap();
        write_vault_file(&tmp, "@me", "temper", "task", "in-temper", None, "");
        write_vault_file(&tmp, "@me", "tasker", "task", "in-tasker", None, "");
        let config = fixture_config(&tmp, &["temper", "tasker"]);

        let (_slugs, _uuids, files) = discover_vault(&config, Some("temper")).unwrap();

        assert_eq!(files.len(), 1, "context filter should restrict walk");
        assert!(files[0].path.ends_with("in-temper.md"));
    }
}
