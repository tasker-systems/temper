//! Resource lookup primitives for CLI commands.
//!
//! `FindableResource` formalizes the inputs to a vault-file lookup:
//! owner (defaulting to `@me` canonical), context (optional — defaults to
//! every configured context), typed doc_type, and a raw slug-or-suffix
//! string. `find_resource` walks the on-disk vault using the same
//! match-by-stem / match-by-slug-portion / suffix-match rules as
//! `actions::task::find_task`, with no `slugify` normalization (which
//! would silently collapse `--` and break double-hyphen slugs — see C.1
//! in the 2026-05-09 audit sweep).
//!
//! When `manifest` is provided and a match is found, the resolved record
//! also carries `temper-id` (or `temper-provisional-id` for unsynced
//! files) so callers don't need a second frontmatter parse.

use std::path::PathBuf;

use temper_core::frontmatter::DocType;
use temper_core::types::ids::ResourceId;
use temper_core::types::Manifest;

use crate::config::Config;
use crate::error::{Result, TemperError};

/// Lookup request for a single resource by slug-or-suffix.
///
/// `owner: None` defaults to the canonical `@me` directory. Pass
/// `Some("@<other-slug>")` to look up a team-shared or other-user
/// resource explicitly.
///
/// `context: None` scans every configured context in `config.contexts`.
///
/// `manifest`, when provided, is consulted for `slug → ResourceId`
/// resolution if the file's frontmatter doesn't carry a parsed `temper-id`.
pub struct FindableResource<'a> {
    pub config: &'a Config,
    pub manifest: Option<&'a Manifest>,
    pub owner: Option<String>,
    pub context: Option<String>,
    pub doc_type: DocType,
    pub slug_or_suffix: String,
}

/// Result of a successful `find_resource` call.
#[derive(Debug, Clone)]
pub struct ResolvedResource {
    pub path: PathBuf,
    pub context: String,
    pub owner: String,
    pub doc_type: DocType,
    pub resource_id: Option<ResourceId>,
    pub provisional_id: Option<String>,
}

/// Locate a resource on disk. See module-level docs for the matching
/// algorithm.
///
/// Errors:
/// - `TemperError::Vault("<doctype> not found: <slug>")` when no file matches.
/// - `TemperError::Vault("ambiguous slug suffix '<input>', matches: ...")`
///   when more than one file matches by suffix-only (mirroring `find_task`).
pub fn find_resource(req: FindableResource<'_>) -> Result<ResolvedResource> {
    use temper_core::frontmatter::Frontmatter;
    use temper_core::vault::Vault;

    let owner = req.owner.unwrap_or_else(|| "@me".into());
    let contexts: Vec<String> = match req.context {
        Some(c) => vec![c],
        None => req.config.contexts.clone(),
    };

    let vault_layout = Vault::new(&req.config.vault_root);
    let doc_type_str = req.doc_type.as_str();
    let needle = req.slug_or_suffix.as_str();

    // (path, context, owner)
    let mut matches: Vec<(PathBuf, String, String)> = Vec::new();

    for ctx in &contexts {
        let dir = vault_layout.doc_type_dir(&owner, ctx, doc_type_str);
        if !dir.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&dir).map_err(|e| TemperError::Vault(e.to_string()))? {
            let entry = entry.map_err(|e| TemperError::Vault(e.to_string()))?;
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            // Slug portion after `YYYY-MM-DD-` prefix, if present.
            let slug_portion = if stem.len() > 11
                && stem.as_bytes().get(4) == Some(&b'-')
                && stem.as_bytes().get(7) == Some(&b'-')
                && stem.as_bytes().get(10) == Some(&b'-')
            {
                &stem[11..]
            } else {
                stem.as_str()
            };

            if stem == needle || slug_portion == needle || stem.ends_with(needle) {
                matches.push((path, ctx.clone(), owner.clone()));
            }
        }
    }

    if matches.is_empty() {
        return Err(TemperError::Vault(format!(
            "{doc_type_str} not found: {needle}"
        )));
    }

    // Disambiguate suffix-only matches: if more than one file matches and
    // none is an exact-stem or exact-slug-portion hit, error with
    // candidates listed (mirrors `actions::task::find_task`).
    if matches.len() > 1 {
        let exact_count = matches
            .iter()
            .filter(|(p, _, _)| {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
                let slug_portion = if stem.len() > 11
                    && stem.as_bytes().get(4) == Some(&b'-')
                    && stem.as_bytes().get(7) == Some(&b'-')
                    && stem.as_bytes().get(10) == Some(&b'-')
                {
                    &stem[11..]
                } else {
                    stem
                };
                stem == needle || slug_portion == needle
            })
            .count();
        if exact_count == 0 {
            let names: Vec<String> = matches
                .iter()
                .filter_map(|(p, _, _)| p.file_stem().and_then(|s| s.to_str()).map(String::from))
                .collect();
            return Err(TemperError::Vault(format!(
                "ambiguous slug suffix '{needle}', matches: {}",
                names.join(", ")
            )));
        }
    }

    // Sort by path descending so the most recent date-prefixed file wins
    // when multiple files match.
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    let (path, context, owner) = matches.into_iter().next().unwrap();

    // Best-effort id resolution from frontmatter; parse failure is not a
    // lookup failure (caller may still want the path).
    let (resource_id, provisional_id) = match std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| Frontmatter::try_from(content.as_str()).ok())
    {
        Some(fm) => {
            let id = fm
                .value()
                .get("temper-id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(ResourceId::from);
            let prov = fm
                .value()
                .get("temper-provisional-id")
                .and_then(|v| v.as_str())
                .map(String::from);
            (id, prov)
        }
        None => (None, None),
    };

    Ok(ResolvedResource {
        path,
        context,
        owner,
        doc_type: req.doc_type,
        resource_id,
        provisional_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Build a minimal Config rooted at a tempdir, with a single context
    /// "temper" that maps to `@me` (no subscription configured).
    fn test_config(vault_root: &Path) -> Config {
        Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: vault_root.join(".temper"),
            contexts: vec!["temper".to_string()],
            subscriptions: Vec::new(),
            skill_output: PathBuf::from("/tmp/skills"),
        }
    }

    fn write_task(vault_root: &Path, owner: &str, ctx: &str, slug: &str, body: &str) {
        let dir = vault_root.join(owner).join(ctx).join("task");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{slug}.md")), body).unwrap();
    }

    #[test]
    fn find_resource_matches_exact_slug_under_at_me() {
        let tmp = TempDir::new().unwrap();
        write_task(
            tmp.path(),
            "@me",
            "temper",
            "my-task",
            "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: my-task\n---\n\n",
        );
        let config = test_config(tmp.path());
        let res = find_resource(FindableResource {
            config: &config,
            manifest: None,
            owner: None,
            context: None,
            doc_type: DocType::Task,
            slug_or_suffix: "my-task".into(),
        })
        .unwrap();
        assert_eq!(res.context, "temper");
        assert_eq!(res.owner, "@me");
        assert_eq!(res.doc_type, DocType::Task);
        assert!(res.path.ends_with("@me/temper/task/my-task.md"));
    }

    #[test]
    fn find_resource_matches_double_hyphen_slug_regression_c1() {
        // C.1: prior find_resource_file ran slugify(slug) which collapsed
        // `--` to `-`, then matched stem.contains(needle). Slugs with
        // literal double-hyphens were unreachable. The new lookup must
        // not normalize the input.
        let tmp = TempDir::new().unwrap();
        write_task(
            tmp.path(),
            "@me",
            "temper",
            "audit-followups--rationalization",
            "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: audit-followups--rationalization\n---\n\n",
        );
        let config = test_config(tmp.path());
        let res = find_resource(FindableResource {
            config: &config,
            manifest: None,
            owner: None,
            context: None,
            doc_type: DocType::Task,
            slug_or_suffix: "audit-followups--rationalization".into(),
        })
        .unwrap();
        assert!(res.path.ends_with("audit-followups--rationalization.md"));
    }

    #[test]
    fn find_resource_matches_slug_portion_after_date_prefix() {
        let tmp = TempDir::new().unwrap();
        write_task(
            tmp.path(),
            "@me",
            "temper",
            "2026-05-09-thread-owner-through-build-vault-path",
            "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: 2026-05-09-thread-owner-through-build-vault-path\n---\n\n",
        );
        let config = test_config(tmp.path());
        let res = find_resource(FindableResource {
            config: &config,
            manifest: None,
            owner: None,
            context: None,
            doc_type: DocType::Task,
            slug_or_suffix: "thread-owner-through-build-vault-path".into(),
        })
        .unwrap();
        assert!(res
            .path
            .ends_with("2026-05-09-thread-owner-through-build-vault-path.md"));
    }

    #[test]
    fn find_resource_errors_when_no_match() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(tmp.path());
        let err = find_resource(FindableResource {
            config: &config,
            manifest: None,
            owner: None,
            context: None,
            doc_type: DocType::Task,
            slug_or_suffix: "nope".into(),
        })
        .unwrap_err();
        match err {
            TemperError::Vault(msg) => {
                assert!(
                    msg.contains("not found") && msg.contains("nope"),
                    "got: {msg}"
                );
            }
            other => panic!("expected Vault error, got: {other:?}"),
        }
    }

    #[test]
    fn find_resource_errors_on_ambiguous_suffix() {
        let tmp = TempDir::new().unwrap();
        for slug in ["aaa-finish", "bbb-finish"] {
            write_task(
                tmp.path(),
                "@me",
                "temper",
                slug,
                &format!(
                    "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: {slug}\n---\n\n"
                ),
            );
        }
        let config = test_config(tmp.path());
        let err = find_resource(FindableResource {
            config: &config,
            manifest: None,
            owner: None,
            context: None,
            doc_type: DocType::Task,
            slug_or_suffix: "finish".into(),
        })
        .unwrap_err();
        match err {
            TemperError::Vault(msg) => assert!(msg.contains("ambiguous"), "got: {msg}"),
            other => panic!("expected Vault error, got: {other:?}"),
        }
    }

    #[test]
    fn find_resource_defaults_to_at_me_when_owner_none() {
        let tmp = TempDir::new().unwrap();
        write_task(
            tmp.path(),
            "@me",
            "temper",
            "private-work",
            "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: private-work\n---\n\n",
        );
        // Same-slug file under a different owner must not match the
        // implicit @me default.
        write_task(
            tmp.path(),
            "@someone-else",
            "temper",
            "private-work",
            "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: private-work\ntemper-owner: '@someone-else'\n---\n\n",
        );
        let config = test_config(tmp.path());
        let res = find_resource(FindableResource {
            config: &config,
            manifest: None,
            owner: None,
            context: None,
            doc_type: DocType::Task,
            slug_or_suffix: "private-work".into(),
        })
        .unwrap();
        assert_eq!(res.owner, "@me");
        assert!(res.path.starts_with(tmp.path().join("@me")));
    }
}
