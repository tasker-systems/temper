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
}
