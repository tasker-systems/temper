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
    /// Files skipped during walk because their frontmatter failed to parse.
    pub skipped_files: usize,
    pub references_resolved: usize,
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

/// A raw reference candidate extracted from markdown body text.
/// Not yet resolved against any owner map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RawRef {
    /// A slug appearing in a wikilink `[[slug]]` (with any variants stripped).
    WikiSlug(String),
    /// A bare UUID appearing in body text.
    BareUuid(Uuid),
    /// A markdown link `[text](path)` pointing at a `.md` file.
    /// The path is the raw `dest_url` from pulldown-cmark.
    MarkdownLink(String),
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

/// Result of walking the vault: the resolution maps, the filtered file
/// list for Pass 2 to scan, and the count of files skipped because their
/// frontmatter failed to parse.
pub(crate) struct VaultDiscovery {
    pub(crate) slugs: SlugMap,
    pub(crate) uuids: UuidMap,
    pub(crate) files: Vec<DiscoveredFile>,
    pub(crate) skipped: usize,
}

/// Walk the vault and build per-owner slug/UUID resolution maps.
///
/// `context_filter` restricts which files appear in the returned
/// `files` list (Pass 2 only scans filtered files), but the
/// maps always include all same-owner files across all contexts so
/// cross-context same-owner references can still resolve.
pub(crate) fn discover_vault(
    config: &Config,
    context_filter: Option<&str>,
) -> Result<VaultDiscovery> {
    use std::fs;
    use temper_core::frontmatter::Frontmatter;
    use temper_core::vault::Vault;

    let mut slugs = SlugMap::default();
    let mut uuids = UuidMap::default();
    let mut filtered_files: Vec<DiscoveredFile> = Vec::new();
    let mut skipped: usize = 0;

    let vault_layout = Vault::new(&config.vault_root);

    for ctx in &config.contexts {
        let owner = config.owner_for_context(ctx);
        let include_in_scan = context_filter.is_none_or(|f| f == ctx);

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
                        skipped += 1;
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

    Ok(VaultDiscovery {
        slugs,
        uuids,
        files: filtered_files,
        skipped,
    })
}

/// Scan a markdown body for raw reference candidates.
///
/// Walks the pulldown-cmark event stream and collects:
/// - `Event::Start(Tag::Link { dest_url, .. })` → `RawRef::MarkdownLink`
///   when `dest_url` ends in `.md` and is not an external URL
/// - Wikilinks `[[...]]` and bare UUIDs inside `Event::Text` events
///   (which are emitted only outside code contexts by pulldown-cmark)
///
/// Does NOT resolve candidates against any owner map — that's the
/// caller's job in Pass 3.
pub(crate) fn scan_body(body: &str) -> Vec<RawRef> {
    use pulldown_cmark::{Event, Parser, Tag};

    // Wikilink regex: [[slug]], [[slug|display]], [[slug#section]],
    // [[slug#section|display]], [[slug.md]]. Rejects folder/ prefixes
    // by disallowing `/` in the slug.
    static WIKILINK_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\[\[([^\]\|#/]+?)(?:\.md)?(?:#[^\]\|]*)?(?:\|[^\]]*)?\]\]").unwrap()
    });

    // UUID regex: 8-4-4-4-12 hex in standard form.
    static UUID_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b",
        )
        .unwrap()
    });

    let mut out: Vec<RawRef> = Vec::new();
    // pulldown-cmark 0.10 splits `[[alpha]]` into individual Text events for
    // each bracket/token. Buffer consecutive Text events so the wikilink regex
    // can match across the concatenated run. Flush whenever a non-Text event
    // arrives (link start, code block start, paragraph end, etc.).
    //
    // Code blocks (fenced and indented) emit Event::Text for their content, so
    // we track `in_code_block` and discard buffered text while inside one.
    let mut text_buf = String::new();
    let mut in_code_block = false;
    let parser = Parser::new(body);

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                flush_text_buf(&mut text_buf, &WIKILINK_RE, &UUID_RE, &mut out);
                in_code_block = true;
            }
            Event::End(pulldown_cmark::TagEnd::CodeBlock) => {
                // Discard anything accumulated inside the code block.
                text_buf.clear();
                in_code_block = false;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                flush_text_buf(&mut text_buf, &WIKILINK_RE, &UUID_RE, &mut out);
                let url = dest_url.as_ref();
                if !is_external_or_anchor(url) && url.ends_with(".md") {
                    out.push(RawRef::MarkdownLink(url.to_string()));
                }
            }
            Event::Text(text) => {
                if !in_code_block {
                    text_buf.push_str(&text);
                }
            }
            _ => {
                if !in_code_block {
                    flush_text_buf(&mut text_buf, &WIKILINK_RE, &UUID_RE, &mut out);
                }
            }
        }
    }
    // Final flush in case the document ends while in a text run.
    flush_text_buf(&mut text_buf, &WIKILINK_RE, &UUID_RE, &mut out);

    out
}

fn flush_text_buf(
    buf: &mut String,
    wikilink_re: &regex::Regex,
    uuid_re: &regex::Regex,
    out: &mut Vec<RawRef>,
) {
    if !buf.is_empty() {
        scan_text_for_wikilinks(buf, wikilink_re, out);
        scan_text_for_uuids(buf, uuid_re, out);
        buf.clear();
    }
}

fn is_external_or_anchor(url: &str) -> bool {
    url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("mailto:")
        || url.starts_with('#')
}

fn scan_text_for_wikilinks(text: &str, re: &regex::Regex, out: &mut Vec<RawRef>) {
    for caps in re.captures_iter(text) {
        if let Some(m) = caps.get(1) {
            let slug = m.as_str().trim();
            if !slug.is_empty() {
                out.push(RawRef::WikiSlug(slug.to_string()));
            }
        }
    }
}

fn scan_text_for_uuids(text: &str, re: &regex::Regex, out: &mut Vec<RawRef>) {
    for m in re.find_iter(text) {
        if let Ok(id) = Uuid::parse_str(m.as_str()) {
            out.push(RawRef::BareUuid(id));
        }
    }
}

/// Resolve a raw reference candidate against the owner-partitioned
/// maps. Returns the canonical string form to store in
/// `open_meta.references` — either a slug or a UUID string.
///
/// - `scanning_file` is the absolute path of the file whose body is
///   being scanned; used to reject self-references and to resolve
///   relative markdown links.
/// - Wikilinks resolve via `SlugMap::resolve` with same-context-first.
/// - Markdown links resolve the `dest_url` relative to the scanning
///   file's parent directory (lexically, no filesystem access), then
///   look up the resulting stem in `SlugMap`.
/// - Bare UUIDs resolve via `UuidMap::resolve` with owner scoping.
///
/// Self-edges (resolution → scanning_file itself) are rejected: they
/// would produce a source == target edge which `edge_service` already
/// rejects server-side.
pub(crate) fn resolve_ref(
    raw: &RawRef,
    scanning_owner: &str,
    scanning_context: &str,
    scanning_file: &std::path::Path,
    slugs: &SlugMap,
    uuids: &UuidMap,
) -> Option<String> {
    match raw {
        RawRef::WikiSlug(slug) => {
            let target = slugs.resolve(scanning_owner, scanning_context, slug)?;
            if target == scanning_file {
                return None;
            }
            Some(slug.clone())
        }
        RawRef::BareUuid(id) => {
            let target = uuids.resolve(scanning_owner, *id)?;
            if target == scanning_file {
                return None;
            }
            Some(id.to_string())
        }
        RawRef::MarkdownLink(dest) => {
            // Resolve the dest relative to the scanning file's dir.
            let scanning_dir = scanning_file.parent()?;
            let joined = scanning_dir.join(dest);
            let canonical = lexical_clean(&joined);

            // Extract the stem and look it up in the same-owner slug map.
            let stem = canonical.file_stem()?.to_str()?.to_string();
            let target = slugs.resolve(scanning_owner, scanning_context, &stem)?;

            // Verify the resolved target actually matches the literal
            // path we computed. This guards against stem collisions
            // across contexts — SlugMap::resolve might pick a different
            // file that happens to share the stem.
            if target != canonical {
                tracing::debug!(
                    dest = %dest,
                    stem = %stem,
                    resolved = %target.display(),
                    computed = %canonical.display(),
                    "markdown link stem collision — rejecting"
                );
                return None;
            }

            if target == scanning_file {
                return None;
            }

            Some(stem)
        }
    }
}

/// Purely lexical path cleanup — removes `./` and `../` components
/// without touching the filesystem. Used for resolving markdown link
/// paths in `resolve_ref` since test paths may not exist on disk and
/// `std::fs::canonicalize` would fail.
fn lexical_clean(path: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c.as_os_str()),
        }
    }
    out
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

/// Merge discovered references with the existing `references` list,
/// returning the combined list and the count of genuinely new entries.
///
/// Uses a HashSet for O(1) deduplication across both slices while
/// preserving the original insertion order of `existing`.
pub(crate) fn merge_references(existing: &[String], discovered: &[String]) -> (Vec<String>, usize) {
    use std::collections::HashSet;
    let mut seen: HashSet<&str> = existing.iter().map(|s| s.as_str()).collect();
    let mut merged: Vec<String> = existing.to_vec();
    let mut added = 0usize;

    for d in discovered {
        if seen.insert(d.as_str()) {
            merged.push(d.clone());
            added += 1;
        }
    }

    (merged, added)
}

/// Write the merged reference list back into a file's frontmatter,
/// mutating the `references` field and serializing the file to disk.
pub(crate) fn write_back_references(file: &std::path::Path, merged: &[String]) -> Result<()> {
    use serde_yaml::Value;
    use temper_core::frontmatter::Frontmatter;

    let mut fm = Frontmatter::parse_file(file)?;

    let seq: Vec<Value> = merged.iter().map(|s| Value::String(s.clone())).collect();
    let new_value = Value::Sequence(seq);

    let mapping = fm.value_mut().as_mapping_mut().ok_or_else(|| {
        crate::error::TemperError::Project(format!(
            "frontmatter of {} is not a mapping",
            file.display()
        ))
    })?;
    mapping.insert(Value::String("references".to_string()), new_value);

    fm.write_to(file)
}

/// Top-level entry point. Walks the vault, scans bodies, merges
/// references into open_meta, writes files back.
pub fn run(config: &Config, params: GraphBuildParams) -> Result<GraphBuildReport> {
    // Pass 1: walk + maps
    let VaultDiscovery {
        slugs,
        uuids,
        files,
        skipped: skipped_files,
    } = discover_vault(config, params.context_filter.as_deref())?;
    let files_walked = files.len();

    // Pass 2: scan + resolve, accumulating per-file discovered refs
    let mut discovered: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let mut references_resolved = 0usize;

    for file in &files {
        let raw_refs = scan_body(file.frontmatter.body());
        for raw in &raw_refs {
            if let Some(resolved) =
                resolve_ref(raw, &file.owner, &file.context, &file.path, &slugs, &uuids)
            {
                references_resolved += 1;
                discovered
                    .entry(file.path.clone())
                    .or_default()
                    .push(resolved);
            }
        }
    }

    // Pass 3: merge + write back (or simulate for dry-run)
    let mut report = GraphBuildReport {
        files_walked,
        skipped_files,
        references_resolved,
        ..Default::default()
    };

    let file_by_path: HashMap<&std::path::Path, &DiscoveredFile> =
        files.iter().map(|f| (f.path.as_path(), f)).collect();

    let mut paths: Vec<&PathBuf> = discovered.keys().collect();
    paths.sort();

    for path in paths {
        let disc_refs = &discovered[path];
        let file = file_by_path
            .get(path.as_path())
            .expect("discovered path not in walk");

        let existing = existing_references(&file.frontmatter);
        let (merged, added) = merge_references(&existing, disc_refs);

        // Count already-present: unique items in discovered that already existed.
        // disc_refs may contain duplicates (same slug mentioned twice in body).
        // We count each unique ref once per existing item it matches.
        use std::collections::HashSet;
        let existing_set: HashSet<&str> = existing.iter().map(|s| s.as_str()).collect();
        let already = disc_refs
            .iter()
            .filter(|d| existing_set.contains(d.as_str()))
            .count();
        report.already_present += already;

        if added == 0 {
            continue;
        }

        if !params.dry_run {
            write_back_references(path, &merged)?;
        }

        report.files_modified += 1;
        report.references_added += added;

        let added_refs: Vec<String> = if params.verbose {
            merged
                .iter()
                .filter(|r| !existing_set.contains(r.as_str()))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        report.modified_files.push(ModifiedFile {
            rel_path: file.rel_path.clone(),
            added,
            added_refs,
        });
    }

    Ok(report)
}

/// Read the existing `open_meta.references` field from a parsed
/// Frontmatter as a `Vec<String>`. Missing, null, or wrong-typed
/// fields yield an empty vec.
fn existing_references(fm: &temper_core::frontmatter::Frontmatter) -> Vec<String> {
    fm.value()
        .get("references")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
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

        let discovery = discover_vault(&config, None).unwrap();

        assert_eq!(discovery.files.len(), 2, "expected 2 files in walk");
        assert_eq!(discovery.skipped, 0);
        assert!(discovery.slugs.resolve("@me", "temper", "alpha").is_some());
        assert!(discovery.slugs.resolve("@me", "tasker", "beta").is_some());

        let alpha_uuid = uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6");
        assert!(discovery.uuids.resolve("@me", alpha_uuid).is_some());
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
        let discovery = result.unwrap();

        assert_eq!(discovery.files.len(), 1);
        assert_eq!(discovery.skipped, 1, "bad.md should be counted as skipped");
        assert!(discovery.slugs.resolve("@me", "temper", "good").is_some());
        assert!(discovery.slugs.resolve("@me", "temper", "bad").is_none());
    }

    #[test]
    fn discover_vault_respects_context_filter() {
        let tmp = TempDir::new().unwrap();
        write_vault_file(&tmp, "@me", "temper", "task", "in-temper", None, "");
        write_vault_file(&tmp, "@me", "tasker", "task", "in-tasker", None, "");
        let config = fixture_config(&tmp, &["temper", "tasker"]);

        let discovery = discover_vault(&config, Some("temper")).unwrap();

        assert_eq!(
            discovery.files.len(),
            1,
            "context filter should restrict walk"
        );
        assert!(discovery.files[0].path.ends_with("in-temper.md"));
    }

    // ── Pass 2: body scanning ───────────────────────────────────────

    #[test]
    fn scan_body_extracts_markdown_link() {
        let refs = scan_body("See [alpha](alpha.md) for details.");
        assert_eq!(refs, vec![RawRef::MarkdownLink("alpha.md".to_string())]);
    }

    #[test]
    fn scan_body_extracts_wikilink_bare() {
        let refs = scan_body("See [[alpha]] for details.");
        assert_eq!(refs, vec![RawRef::WikiSlug("alpha".to_string())]);
    }

    #[test]
    fn scan_body_extracts_wikilink_with_pipe_display() {
        let refs = scan_body("See [[alpha|Alpha Doc]] for details.");
        assert_eq!(refs, vec![RawRef::WikiSlug("alpha".to_string())]);
    }

    #[test]
    fn scan_body_extracts_wikilink_with_anchor() {
        let refs = scan_body("See [[alpha#section]] for details.");
        assert_eq!(refs, vec![RawRef::WikiSlug("alpha".to_string())]);
    }

    #[test]
    fn scan_body_extracts_wikilink_with_anchor_and_pipe() {
        let refs = scan_body("See [[alpha#section|display]] for details.");
        assert_eq!(refs, vec![RawRef::WikiSlug("alpha".to_string())]);
    }

    #[test]
    fn scan_body_extracts_wikilink_with_md_suffix() {
        let refs = scan_body("See [[alpha.md]] for details.");
        assert_eq!(refs, vec![RawRef::WikiSlug("alpha".to_string())]);
    }

    #[test]
    fn scan_body_extracts_bare_uuid() {
        let refs = scan_body("See 019d1d24-2000-7379-8f26-ae4ae87bc5c6 for details.");
        assert_eq!(
            refs,
            vec![RawRef::BareUuid(uuid(
                "019d1d24-2000-7379-8f26-ae4ae87bc5c6"
            ))]
        );
    }

    #[test]
    fn scan_body_rejects_external_urls() {
        let refs = scan_body("See [example](https://example.com).");
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_rejects_mailto() {
        let refs = scan_body("See [contact](mailto:foo@bar.com).");
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_rejects_intra_doc_anchors() {
        let refs = scan_body("See [jump](#section).");
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_rejects_non_md_extensions() {
        let refs = scan_body("See [data](data.json).");
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_rejects_extensionless_paths() {
        let refs = scan_body("See [bare](foo).");
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_skips_code_blocks() {
        let body = "\
Regular text [[real-ref]].

```
Inside code [[fake-ref]] and `[[also-fake]]`.
```

Back to prose [[another-real]].
";
        let refs = scan_body(body);
        assert_eq!(
            refs,
            vec![
                RawRef::WikiSlug("real-ref".to_string()),
                RawRef::WikiSlug("another-real".to_string())
            ]
        );
    }

    #[test]
    fn scan_body_skips_inline_code() {
        let refs = scan_body("The token `[[not-a-ref]]` is inline code.");
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_skips_indented_code_block() {
        let body = "Prose line.\n\n    [[fake-ref-in-indented-block]]\n\nMore prose.";
        let refs = scan_body(body);
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_multiple_references_in_reading_order() {
        let body = "First [[alpha]], then [beta](beta.md), then [[gamma]].";
        let refs = scan_body(body);
        assert_eq!(
            refs,
            vec![
                RawRef::WikiSlug("alpha".to_string()),
                RawRef::MarkdownLink("beta.md".to_string()),
                RawRef::WikiSlug("gamma".to_string()),
            ]
        );
    }

    // ── Pass 2: resolution ──────────────────────────────────────────

    fn build_test_maps() -> (SlugMap, UuidMap) {
        let mut slugs = SlugMap::default();
        let mut uuids = UuidMap::default();
        slugs.insert(
            "@me",
            "temper",
            "alpha",
            path("/v/@me/temper/task/alpha.md"),
        );
        slugs.insert("@me", "tasker", "beta", path("/v/@me/tasker/task/beta.md"));
        uuids.insert(
            "@me",
            uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6"),
            path("/v/@me/temper/task/alpha.md"),
        );
        slugs.insert(
            "+team-x",
            "shared",
            "leaked",
            path("/v/+team-x/shared/task/leaked.md"),
        );
        (slugs, uuids)
    }

    #[test]
    fn resolve_ref_wikislug_same_owner_same_context() {
        let (slugs, uuids) = build_test_maps();
        let resolved = resolve_ref(
            &RawRef::WikiSlug("alpha".to_string()),
            "@me",
            "temper",
            std::path::Path::new("/v/@me/temper/task/other.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(resolved, Some("alpha".to_string()));
    }

    #[test]
    fn resolve_ref_wikislug_cross_owner_rejected() {
        let (slugs, uuids) = build_test_maps();
        let resolved = resolve_ref(
            &RawRef::WikiSlug("leaked".to_string()),
            "@me",
            "temper",
            std::path::Path::new("/v/@me/temper/task/other.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(resolved, None, "cross-owner must not resolve");
    }

    #[test]
    fn resolve_ref_bare_uuid_same_owner() {
        let (slugs, uuids) = build_test_maps();
        let resolved = resolve_ref(
            &RawRef::BareUuid(uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6")),
            "@me",
            "temper",
            std::path::Path::new("/v/@me/temper/task/other.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(
            resolved,
            Some("019d1d24-2000-7379-8f26-ae4ae87bc5c6".to_string())
        );
    }

    #[test]
    fn resolve_ref_markdown_link_relative_md() {
        let (slugs, uuids) = build_test_maps();
        // Scanning from a file in /v/@me/temper/task/, linking to ./alpha.md
        let resolved = resolve_ref(
            &RawRef::MarkdownLink("alpha.md".to_string()),
            "@me",
            "temper",
            std::path::Path::new("/v/@me/temper/task/other.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(resolved, Some("alpha".to_string()));
    }

    #[test]
    fn resolve_ref_markdown_link_unresolvable_returns_none() {
        let (slugs, uuids) = build_test_maps();
        let resolved = resolve_ref(
            &RawRef::MarkdownLink("nonexistent.md".to_string()),
            "@me",
            "temper",
            std::path::Path::new("/v/@me/temper/task/other.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(resolved, None);
    }

    #[test]
    fn resolve_ref_self_reference_returns_none() {
        let (slugs, uuids) = build_test_maps();
        // Scanning file IS alpha.md; a wikilink to [[alpha]] from inside
        // alpha would create a self-edge, which is meaningless.
        let resolved = resolve_ref(
            &RawRef::WikiSlug("alpha".to_string()),
            "@me",
            "temper",
            std::path::Path::new("/v/@me/temper/task/alpha.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(resolved, None, "self-reference should be rejected");
    }

    #[test]
    fn resolve_ref_bare_uuid_self_reference_returns_none() {
        let (slugs, uuids) = build_test_maps();
        // The UUID for alpha is in the map pointing to alpha.md; scanning
        // from alpha.md with that UUID as a bare ref is a self-edge.
        let resolved = resolve_ref(
            &RawRef::BareUuid(uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6")),
            "@me",
            "temper",
            std::path::Path::new("/v/@me/temper/task/alpha.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(
            resolved, None,
            "bare-uuid self-reference should be rejected"
        );
    }

    #[test]
    fn resolve_ref_markdown_link_self_reference_returns_none() {
        let (slugs, uuids) = build_test_maps();
        // Scanning alpha.md with a relative link to ./alpha.md is a self-edge.
        let resolved = resolve_ref(
            &RawRef::MarkdownLink("alpha.md".to_string()),
            "@me",
            "temper",
            std::path::Path::new("/v/@me/temper/task/alpha.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(
            resolved, None,
            "markdown-link self-reference should be rejected"
        );
    }

    // ── Pass 3: merge and write-back ─────────────────────────────────

    #[test]
    fn merge_references_union_preserves_existing_order() {
        let existing = vec!["foo".to_string(), "bar".to_string()];
        let discovered = vec!["baz".to_string()];
        let (merged, added) = merge_references(&existing, &discovered);
        assert_eq!(merged, vec!["foo", "bar", "baz"]);
        assert_eq!(added, 1);
    }

    #[test]
    fn merge_references_dedupes_across_existing_and_discovered() {
        let existing = vec!["foo".to_string(), "bar".to_string()];
        let discovered = vec!["foo".to_string(), "baz".to_string()];
        let (merged, added) = merge_references(&existing, &discovered);
        assert_eq!(merged, vec!["foo", "bar", "baz"]);
        assert_eq!(added, 1);
    }

    #[test]
    fn merge_references_no_new_entries_reports_zero_added() {
        let existing = vec!["foo".to_string(), "bar".to_string()];
        let discovered = vec!["foo".to_string()];
        let (merged, added) = merge_references(&existing, &discovered);
        assert_eq!(merged, vec!["foo", "bar"]);
        assert_eq!(added, 0);
    }

    #[test]
    fn merge_references_empty_existing() {
        let existing: Vec<String> = vec![];
        let discovered = vec!["foo".to_string(), "bar".to_string()];
        let (merged, added) = merge_references(&existing, &discovered);
        assert_eq!(merged, vec!["foo", "bar"]);
        assert_eq!(added, 2);
    }

    #[test]
    fn merge_references_discovered_duplicates_deduped() {
        let existing: Vec<String> = vec![];
        let discovered = vec!["foo".to_string(), "foo".to_string(), "bar".to_string()];
        let (merged, added) = merge_references(&existing, &discovered);
        assert_eq!(merged, vec!["foo", "bar"]);
        assert_eq!(added, 2);
    }

    #[test]
    fn write_back_adds_new_references_field_when_missing() {
        let tmp = TempDir::new().unwrap();
        let file = write_vault_file(&tmp, "@me", "temper", "task", "alpha", None, "body");

        let merged = vec!["beta".to_string(), "gamma".to_string()];
        write_back_references(&file, &merged).unwrap();

        let fm = temper_core::frontmatter::Frontmatter::parse_file(&file).unwrap();
        let refs = fm
            .value()
            .get("references")
            .and_then(|v| v.as_sequence())
            .unwrap();
        let refs_strs: Vec<&str> = refs.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(refs_strs, vec!["beta", "gamma"]);
    }

    #[test]
    fn write_back_updates_existing_references_field() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("@me").join("temper").join("task");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("alpha.md");
        let content = "---\n\
temper-context: temper\n\
temper-type: task\n\
temper-owner: '@me'\n\
title: alpha\n\
slug: alpha\n\
references:\n  - existing1\n  - existing2\n\
---\nbody\n";
        fs::write(&file, content).unwrap();

        let merged = vec![
            "existing1".to_string(),
            "existing2".to_string(),
            "new".to_string(),
        ];
        write_back_references(&file, &merged).unwrap();

        let fm = temper_core::frontmatter::Frontmatter::parse_file(&file).unwrap();
        let refs: Vec<String> = fm
            .value()
            .get("references")
            .and_then(|v| v.as_sequence())
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        assert_eq!(refs, vec!["existing1", "existing2", "new"]);
    }

    #[test]
    fn write_back_preserves_body() {
        let tmp = TempDir::new().unwrap();
        let body_text = "# Heading\n\nSome content with [[alpha]] reference.\n";
        let file = write_vault_file(&tmp, "@me", "temper", "task", "bravo", None, body_text);

        write_back_references(&file, &["alpha".to_string()]).unwrap();

        let fm = temper_core::frontmatter::Frontmatter::parse_file(&file).unwrap();
        assert!(fm.body().contains("# Heading"));
        assert!(fm.body().contains("[[alpha]]"));
    }

    // ── End-to-end run() tests ───────────────────────────────────────

    #[test]
    fn run_end_to_end_seeds_references_from_wikilinks() {
        let tmp = TempDir::new().unwrap();
        write_vault_file(&tmp, "@me", "temper", "task", "alpha", None, "");
        write_vault_file(&tmp, "@me", "temper", "task", "beta", None, "");
        let source = write_vault_file(
            &tmp,
            "@me",
            "temper",
            "task",
            "source",
            None,
            "This references [[alpha]] and [[beta]] explicitly.",
        );
        let config = fixture_config(&tmp, &["temper"]);
        let params = GraphBuildParams {
            context_filter: None,
            dry_run: false,
            verbose: false,
        };
        let report = run(&config, params).unwrap();

        assert_eq!(report.files_modified, 1);
        assert_eq!(report.references_added, 2);

        let fm = temper_core::frontmatter::Frontmatter::parse_file(&source).unwrap();
        let refs: Vec<String> = fm
            .value()
            .get("references")
            .and_then(|v| v.as_sequence())
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        assert_eq!(refs, vec!["alpha", "beta"]);
    }

    #[test]
    fn run_end_to_end_idempotent() {
        let tmp = TempDir::new().unwrap();
        write_vault_file(&tmp, "@me", "temper", "task", "alpha", None, "");
        write_vault_file(
            &tmp,
            "@me",
            "temper",
            "task",
            "source",
            None,
            "See [[alpha]].",
        );
        let config = fixture_config(&tmp, &["temper"]);

        let first = run(
            &config,
            GraphBuildParams {
                context_filter: None,
                dry_run: false,
                verbose: false,
            },
        )
        .unwrap();
        assert_eq!(first.files_modified, 1);

        let second = run(
            &config,
            GraphBuildParams {
                context_filter: None,
                dry_run: false,
                verbose: false,
            },
        )
        .unwrap();
        assert_eq!(second.files_modified, 0, "second run must be a no-op");
        assert_eq!(second.references_added, 0);
    }

    #[test]
    fn run_dry_run_does_not_write() {
        let tmp = TempDir::new().unwrap();
        write_vault_file(&tmp, "@me", "temper", "task", "alpha", None, "");
        let source = write_vault_file(
            &tmp,
            "@me",
            "temper",
            "task",
            "source",
            None,
            "See [[alpha]].",
        );
        let content_before = std::fs::read_to_string(&source).unwrap();

        let config = fixture_config(&tmp, &["temper"]);
        let report = run(
            &config,
            GraphBuildParams {
                context_filter: None,
                dry_run: true,
                verbose: false,
            },
        )
        .unwrap();
        assert_eq!(report.files_modified, 1, "report counts as if written");

        let content_after = std::fs::read_to_string(&source).unwrap();
        assert_eq!(content_before, content_after, "dry-run must not write");
    }
}
