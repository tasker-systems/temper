# Cloud-Only Sync Handling, FindableResource Refactor, and Owner Canonicalization Reversal — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix five bugs (resource-show no API fallback; sync misclassifying missing-as-modified; misleading recovery error; owner-canonicalization wrong direction; C.1 slugify-collapse) by introducing a typed `FindableResource` lookup, a new `ManifestEntryState::LocallyMissing` state, an API fallback in `temper resource show`, and a reversal of PR #70/#72's owner canonicalization (`@me` is canonical for own work; `@<other-slug>` is for team-shared contexts).

**Architecture:** Two work sets in one PR. **Work Set A** (Tasks 1-7) introduces `FindableResource` in a new `temper-cli/src/lookup.rs` module, replaces the stringly-typed `doc_type: &str` lookup with the typed `DocType` enum, adds `@me` + `@<profile.slug>` dual-directory scanning so legacy PR #70/72-era files stay reachable, fixes C.1 by removing the slugify call from the lookup, and adds a silent API fallback in `temper resource show` for cloud-only resources in local mode. **Work Set B** (Tasks 8-14) adds `ManifestEntryState::LocallyMissing`, routes missing-but-tracked files through pull instead of push, reverses `OwnerResolver::resolve` and `resolve_owner_for_frontmatter` to return `@me` for the requester's own resources, and updates the now-unreachable `vault_file_missing_err` message plus CLAUDE.md's recovery guidance.

**Tech Stack:** Rust workspace (`cargo make`, `cargo nextest`), `temper-core` (shared types incl. `DocType`, `Manifest`, `ManifestEntryState`), `temper-cli` (CLI commands + sync actions), `temper-client` (HTTP client for API fallback), `temper-api` (server diff, unchanged in this PR), e2e harness at `tests/e2e/`.

**Spec:** `docs/superpowers/specs/2026-05-10-cloud-only-sync-and-find-resource-design.md`

---

## File Map

**New files:**
- `crates/temper-cli/src/lookup.rs` — `FindableResource`, `ResolvedResource`, `find_resource` (Tasks 1-4)
- `tests/e2e/tests/cloud_only_show_fallback_test.rs` — e2e for show API fallback (Task 7)
- `tests/e2e/tests/locally_missing_recovery_test.rs` — e2e for missing-file pull-recovery (Task 11)

**Modified files:**
- `crates/temper-cli/src/lib.rs` — register the new `lookup` module (Task 1)
- `crates/temper-cli/src/commands/resource.rs` — replace `find_resource_file` callers with `find_resource`; retire `VALID_DOC_TYPES`/`validate_doc_type`; add API fallback in `show_generic` (Tasks 5, 6, 7)
- `crates/temper-cli/src/commands/task.rs` — add API fallback when `find_task` returns None (Task 7)
- `crates/temper-cli/src/commands/session.rs` — add API fallback when `find_session` returns None (Task 7)
- `crates/temper-core/src/types/manifest.rs` — add `LocallyMissing` variant + serde tests (Task 8)
- `crates/temper-cli/src/actions/sync.rs` — `rehash_manifest`, `normalize_all_entries`, `sync_orchestration`, `resolve_owner_for_frontmatter`, `OwnerResolver::resolve`, `vault_file_missing_err` (Tasks 9, 10, 11, 12, 14)
- `crates/temper-cli/src/commands/add.rs` and/or `crates/temper-cli/src/actions/ingest.rs` — own-resource owner-string audit (Task 13)
- `CLAUDE.md` — recovery guidance paragraph rewrite (Task 14)

**Coordination point:** Task 13 (create-path owner audit) overlaps with the sibling B.2 session's work in `commands/add.rs`. Land order determines who rebases. If B.2 lands first, Task 13 verifies their fix is correct and adds any sites they missed. If this plan lands first, B.2 rebases.

---

## Verification Commands (used throughout)

```bash
# Single test (unit)
cargo nextest run -p temper-cli <test_name>

# Single test (with test-db)
cargo nextest run -p temper-cli --features test-db <test_name>

# Full crate suite
cargo nextest run -p temper-cli

# Full workspace (catches feature unification)
cargo nextest run --workspace

# Embed-gated e2e (run before final commit per spec)
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed

# Lint + format
cargo make check

# Auto-fix
cargo make fix
```

---

# Work Set A — Lookup refactor, C.1 fix, show fallback

This work set is read-side only. No manifest state changes, no sync orchestration changes, no write paths touched.

---

### Task 1: Create `lookup.rs` module skeleton with types

**Files:**
- Create: `crates/temper-cli/src/lookup.rs`
- Modify: `crates/temper-cli/src/lib.rs`

- [ ] **Step 1: Add the new module declaration**

Edit `crates/temper-cli/src/lib.rs` and add `pub mod lookup;` after the existing `pub mod` declarations (next to `pub mod actions;`, `pub mod commands;`, etc. — preserve alphabetical/logical ordering already present in the file).

- [ ] **Step 2: Create the type definitions**

Create `crates/temper-cli/src/lookup.rs` with the following content:

```rust
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
use temper_core::types::manifest::Manifest;

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
pub fn find_resource(_req: FindableResource<'_>) -> Result<ResolvedResource> {
    Err(TemperError::Vault("find_resource: not yet implemented".into()))
}
```

- [ ] **Step 3: Verify the module compiles**

Run:
```bash
cargo check -p temper-cli
```

Expected: clean compile (the function returns an error stub but the types are valid). If the `Manifest` import path is wrong, run `grep -n "pub use.*Manifest\b" crates/temper-core/src/lib.rs crates/temper-core/src/types/mod.rs` and adjust.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/lookup.rs crates/temper-cli/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(cli): add FindableResource lookup type skeleton

Introduces the new lookup module with FindableResource, ResolvedResource,
and a stub find_resource that returns an error. Subsequent tasks
implement the matching algorithm, manifest-aware id resolution, and
legacy directory fallback.

Part of the 2026-05-10 cloud-only-sync + find-resource refactor.
EOF
)"
```

---

### Task 2: Implement `find_resource` matching with C.1 regression test

**Files:**
- Modify: `crates/temper-cli/src/lookup.rs`

- [ ] **Step 1: Write failing tests for the matching algorithm**

Add at the bottom of `crates/temper-cli/src/lookup.rs` (after the function stub):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::fs;
    use tempfile::TempDir;

    /// Build a minimal Config rooted at a tempdir, with a single context
    /// "temper" and `@me` as its owner.
    fn test_config(vault_root: &std::path::Path) -> Config {
        let mut c = Config::default();
        c.vault_root = vault_root.to_path_buf();
        c.contexts = vec!["temper".to_string()];
        // owner_for_context returns "@me" by default for unmapped contexts.
        c
    }

    fn write_task(vault_root: &std::path::Path, owner: &str, ctx: &str, slug: &str, body: &str) {
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
        // `a--b` to `a-b`, then matched stem.contains(needle) — so
        // `foo--bar` searched as `foo-bar` against stem `foo--bar`
        // would match accidentally, but `foo--bar.md` searched against
        // a different prefix wouldn't. The new lookup must not
        // normalize the input at all.
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
        assert!(res.path.ends_with("2026-05-09-thread-owner-through-build-vault-path.md"));
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
        assert!(matches!(err, TemperError::Vault(msg) if msg.contains("not found") && msg.contains("nope")));
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
                &format!("---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: {slug}\n---\n\n"),
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
        assert!(matches!(err, TemperError::Vault(msg) if msg.contains("ambiguous")));
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
        // Also write a same-slug file under a different owner to prove
        // we're not accidentally cross-owner matching.
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
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo nextest run -p temper-cli lookup::tests
```

Expected: all 5 tests fail with the stub error `find_resource: not yet implemented`.

- [ ] **Step 3: Implement `find_resource`**

Replace the stub `find_resource` body with:

```rust
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

    let mut matches: Vec<(PathBuf, String, String)> = Vec::new(); // (path, context, owner)

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
            "{} not found: {}",
            doc_type_str, needle
        )));
    }

    // Disambiguate suffix-only matches: if more than one file matches and
    // the user did NOT pass an exact stem or slug-portion, error out with
    // candidates listed.
    if matches.len() > 1 {
        let exact_count = matches
            .iter()
            .filter(|(p, _, _)| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s == needle)
            })
            .count();
        if exact_count == 0 {
            let names: Vec<String> = matches
                .iter()
                .filter_map(|(p, _, _)| p.file_stem().and_then(|s| s.to_str()).map(String::from))
                .collect();
            return Err(TemperError::Vault(format!(
                "ambiguous slug suffix '{}', matches: {}",
                needle,
                names.join(", ")
            )));
        }
    }

    // Sort by path descending so the most recent date-prefixed file wins
    // when there are multiple exact matches.
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    let (path, context, owner) = matches.into_iter().next().unwrap();

    // Frontmatter: best-effort id resolution. A parse failure does not
    // fail the lookup — the caller may still want the path.
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
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-cli lookup::tests
```

Expected: all 5 tests pass.

- [ ] **Step 5: Run lint to confirm clean**

```bash
cargo make check
```

Expected: clean. Fix any clippy/format issues inline before committing.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/lookup.rs
git commit -m "$(cat <<'EOF'
feat(cli): implement find_resource matching algorithm + C.1 fix

Implements raw equality / slug-portion-after-date-prefix / suffix
matching with no slugify normalization. Closes the C.1 finding from the
2026-05-09 audit sweep: prior `find_resource_file` collapsed `--` to
`-` via slugify(input), making double-hyphen slugs unreachable
(`audit-followups--rationalization` and `deprecate-resource-service--create-after-phase-3b`
were the canaries).

Tests cover: exact slug match, double-hyphen regression, date-prefixed
slug-portion match, not-found error, ambiguous-suffix disambiguation,
and the @me-default-when-owner-None contract.

Manifest-aware ResourceId / provisional_id resolution and legacy
@<profile.slug>/ fallback land in the next two tasks.
EOF
)"
```

---

### Task 3: Add manifest-aware id resolution

**Files:**
- Modify: `crates/temper-cli/src/lookup.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/temper-cli/src/lookup.rs`:

```rust
#[test]
fn find_resource_resolves_resource_id_from_manifest() {
    use std::collections::HashMap;
    use temper_core::types::ids::ResourceId;
    use temper_core::types::manifest::{Manifest, ManifestEntry, ManifestEntryState};
    use uuid::Uuid;

    let tmp = TempDir::new().unwrap();
    let id = ResourceId::from(Uuid::now_v7());
    let id_str = Uuid::from(id).to_string();

    // File with NO temper-id in frontmatter, but in the manifest.
    write_task(
        tmp.path(),
        "@me",
        "temper",
        "tracked",
        "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: tracked\n---\n\n",
    );

    let mut manifest = Manifest::new("device-test".to_string());
    manifest.entries.insert(
        id,
        ManifestEntry {
            path: "@me/temper/task/tracked.md".to_string(),
            body_hash: String::new(),
            remote_body_hash: String::new(),
            managed_hash: String::new(),
            open_hash: String::new(),
            remote_managed_hash: String::new(),
            remote_open_hash: String::new(),
            synced_at: chrono::Utc::now(),
            state: ManifestEntryState::Clean,
            mtime_secs: None,
            provisional: false,
            last_audit_id: None,
        },
    );

    let config = test_config(tmp.path());
    let res = find_resource(FindableResource {
        config: &config,
        manifest: Some(&manifest),
        owner: None,
        context: None,
        doc_type: DocType::Task,
        slug_or_suffix: "tracked".into(),
    })
    .unwrap();

    assert_eq!(res.resource_id, Some(id), "id should resolve from manifest path lookup");
    assert!(res.provisional_id.is_none());
    let _ = id_str; // pin id_str usage so future debugging keeps it
}

#[test]
fn find_resource_picks_up_provisional_id_from_frontmatter() {
    let tmp = TempDir::new().unwrap();
    write_task(
        tmp.path(),
        "@me",
        "temper",
        "unsynced",
        "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: unsynced\ntemper-provisional-id: prov-abc-123\n---\n\n",
    );
    let config = test_config(tmp.path());
    let res = find_resource(FindableResource {
        config: &config,
        manifest: None,
        owner: None,
        context: None,
        doc_type: DocType::Task,
        slug_or_suffix: "unsynced".into(),
    })
    .unwrap();
    assert_eq!(res.provisional_id.as_deref(), Some("prov-abc-123"));
    assert!(res.resource_id.is_none());
}
```

- [ ] **Step 2: Run tests to verify the manifest test fails**

```bash
cargo nextest run -p temper-cli lookup::tests::find_resource_resolves_resource_id_from_manifest
```

Expected: FAIL — `res.resource_id` is `None` because the file has no `temper-id` frontmatter and the current code does not consult the manifest.

The provisional test should already PASS because the existing code reads `temper-provisional-id` from frontmatter.

- [ ] **Step 3: Add manifest-path lookup to id resolution**

In `find_resource`, after the frontmatter-based id resolution block, before constructing `ResolvedResource`, insert a manifest-fallback step:

Replace the block:
```rust
    let (resource_id, provisional_id) = match std::fs::read_to_string(&path)
```
through to the closing of that `match`, with:

```rust
    let (mut resource_id, provisional_id) = match std::fs::read_to_string(&path)
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

    // Manifest fallback: if frontmatter didn't yield an id, look up by
    // path. Manifest entries are keyed by ResourceId; iterate to find
    // the entry whose `path` matches our resolved relative path.
    if resource_id.is_none() {
        if let Some(manifest) = req.manifest {
            if let Ok(rel) = path.strip_prefix(&req.config.vault_root) {
                let rel_str = rel.to_string_lossy().to_string();
                resource_id = manifest
                    .entries
                    .iter()
                    .find(|(_, e)| e.path == rel_str)
                    .map(|(id, _)| *id);
            }
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p temper-cli lookup::tests
```

Expected: all tests pass (7 total now).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/lookup.rs
git commit -m "$(cat <<'EOF'
feat(cli): wire manifest-path fallback for resource_id resolution

When the matched file's frontmatter has no temper-id (legacy file or
fresh write), look up the manifest entry by relative path and pull the
ResourceId from there. Provisional id resolution stays frontmatter-only
since unsynced files aren't in the manifest yet.
EOF
)"
```

---

### Task 4: Add legacy `@<profile.slug>/` directory fallback

**Files:**
- Modify: `crates/temper-cli/src/lookup.rs`

**Background:** PR #70 + PR #72 (May 2026) wrote some pulled-and-newly-tracked own-resource files under `@<profile.slug>/<ctx>/<doctype>/...` instead of `@me/<ctx>/<doctype>/...`. After Work Set B reverses that direction, future writes land at `@me/`, but legacy files from the May 2026 window remain at `@<profile.slug>/`. To keep them reachable without a vault migration, `find_resource` scans both directories when the requested owner is `@me` (the default).

The "what's the user's own slug" lookup is sync-only. We rely on `Config::owner_for_context` returning `@me` for own contexts (already true) and read `profile.slug` from the cached auth state (added in Work Set B). For Task 4 specifically, we accept an `Option<String>` and treat `None` as "no legacy fallback" — the integration with the cached profile slug happens implicitly through callers that already have it.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:

```rust
#[test]
fn find_resource_falls_back_to_legacy_slug_directory() {
    // PR #70/72 wrote some own-resource files under @<profile.slug>/.
    // After the canonical-direction reversal we still want those files
    // reachable without a vault migration. find_resource scans both
    // @me/ and @<profile.slug>/ when the requested owner is @me.
    let tmp = TempDir::new().unwrap();
    write_task(
        tmp.path(),
        "@j-cole-taylor",
        "temper",
        "legacy-pull",
        "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: legacy-pull\ntemper-owner: '@j-cole-taylor'\n---\n\n",
    );

    let mut config = test_config(tmp.path());
    config.profile_slug = Some("j-cole-taylor".to_string());

    let res = find_resource(FindableResource {
        config: &config,
        manifest: None,
        owner: None, // → @me
        context: None,
        doc_type: DocType::Task,
        slug_or_suffix: "legacy-pull".into(),
    })
    .unwrap();

    assert!(
        res.path.ends_with("@j-cole-taylor/temper/task/legacy-pull.md"),
        "expected legacy @<slug>/ path, got {:?}",
        res.path
    );
    // The owner field on the result reflects the directory we found it in,
    // so callers can detect "this was found via the legacy fallback."
    assert_eq!(res.owner, "@j-cole-taylor");
}

#[test]
fn find_resource_prefers_at_me_over_legacy_when_both_exist() {
    let tmp = TempDir::new().unwrap();
    write_task(
        tmp.path(),
        "@me",
        "temper",
        "dual-resident",
        "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: dual-resident\n---\n\nfresh\n",
    );
    write_task(
        tmp.path(),
        "@j-cole-taylor",
        "temper",
        "dual-resident",
        "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: dual-resident\n---\n\nlegacy\n",
    );
    let mut config = test_config(tmp.path());
    config.profile_slug = Some("j-cole-taylor".to_string());

    let res = find_resource(FindableResource {
        config: &config,
        manifest: None,
        owner: None,
        context: None,
        doc_type: DocType::Task,
        slug_or_suffix: "dual-resident".into(),
    })
    .unwrap();
    assert_eq!(res.owner, "@me", "@me/ should be preferred over legacy fallback");
}
```

- [ ] **Step 2: Add `profile_slug` to `Config`**

The tests reference `config.profile_slug`. Add it to `Config` (look in `crates/temper-cli/src/config.rs`):

Find the `Config` struct definition and add:

```rust
    /// The user's profile slug (cached from `client.profile().get()`),
    /// used by `lookup::find_resource` to scan the legacy
    /// `@<profile.slug>/` directory for files written during the
    /// PR #70 / PR #72 window. `None` until first authenticated CLI
    /// invocation populates it.
    #[serde(default)]
    pub profile_slug: Option<String>,
```

If `Config` derives `Default`, `Option<String>` plays nicely. If not, add `profile_slug: None` to the manual `impl Default for Config` body.

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo nextest run -p temper-cli lookup::tests::find_resource_falls_back_to_legacy_slug_directory
```

Expected: FAIL with not-found.

- [ ] **Step 4: Implement legacy fallback**

In `find_resource`, change the directory iteration so that when `owner == "@me"` and `req.config.profile_slug` is `Some`, we also scan the `@<profile.slug>/<ctx>/<doctype>/` directory.

Replace the `for ctx in &contexts {` loop with:

```rust
    for ctx in &contexts {
        let mut dirs_to_scan: Vec<(PathBuf, String)> = Vec::new();
        let primary = vault_layout.doc_type_dir(&owner, ctx, doc_type_str);
        dirs_to_scan.push((primary, owner.clone()));

        // Legacy fallback: when the requested owner is @me, also scan
        // the @<profile.slug>/ directory (files written during the
        // PR #70 / PR #72 window before the canonical direction was
        // reversed).
        if owner == "@me" {
            if let Some(profile_slug) = req.config.profile_slug.as_deref() {
                let legacy_owner = format!("@{profile_slug}");
                let legacy = vault_layout.doc_type_dir(&legacy_owner, ctx, doc_type_str);
                dirs_to_scan.push((legacy, legacy_owner));
            }
        }

        for (dir, dir_owner) in dirs_to_scan {
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
                    matches.push((path, ctx.clone(), dir_owner.clone()));
                }
            }
        }
    }
```

The `prefers_at_me_over_legacy` test relies on the existing path-descending sort, but `@me/` sorts before `@j-cole-taylor/` alphabetically (uppercase `@` then `m` vs `j` — actually `j` < `m` lexicographically, so `@j-cole-taylor/` would sort first under descending). Tighten the result picker to prefer `@me`-resident matches when there are multiple exact matches:

Replace:
```rust
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    let (path, context, owner) = matches.into_iter().next().unwrap();
```

with:

```rust
    // Prefer @me-resident matches over legacy @<slug>/ matches when
    // both exist for the same logical resource.
    matches.sort_by(|a, b| {
        // @me wins
        let a_is_me = a.2 == "@me";
        let b_is_me = b.2 == "@me";
        match (a_is_me, b_is_me) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b.0.cmp(&a.0), // tiebreak: most recent date-prefixed wins
        }
    });
    let (path, context, owner) = matches.into_iter().next().unwrap();
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo nextest run -p temper-cli lookup::tests
```

Expected: all 9 tests pass.

- [ ] **Step 6: `cargo make check`, then commit**

```bash
cargo make check && git add -A && git commit -m "$(cat <<'EOF'
feat(cli): scan legacy @<profile.slug>/ directory in find_resource

When the requested owner is @me (the default), find_resource also walks
the @<profile.slug>/<ctx>/<doctype>/ directory to find files written
during the PR #70 / PR #72 window. Adds Config::profile_slug as the
cached source of the user's slug; populated lazily by the first authed
CLI invocation (wiring lands in Work Set B).

@me-resident matches always win when the same logical slug exists in
both directories, so legacy fallback never shadows a fresh write.
EOF
)"
```

---

### Task 5: Migrate `find_resource_file` callers to `find_resource`

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs`

The callers (per `grep -n find_resource_file crates/temper-cli/src/commands/resource.rs`):
- Line ~840: in some command path (read context near the line)
- Line ~925: in `resolve_resource_id`
- Line ~956: in another command path
- Line ~1207: the function definition itself
- Line ~1503: in update path

- [ ] **Step 1: Read each callsite to understand its context**

```bash
grep -n "find_resource_file\b" crates/temper-cli/src/commands/resource.rs
```

For each line returned, read 5 lines before and 5 lines after to see what the caller does with the `(PathBuf, String)` return value. Record (in your head or scratch buffer) which fields each caller actually consumes.

- [ ] **Step 2: For each callsite, replace with `find_resource(FindableResource { ... })`**

Example replacement pattern. Where the old code is:

```rust
let (path, ctx) = find_resource_file(config, doc_type, slug, context)?;
```

The new code is:

```rust
let resolved = crate::lookup::find_resource(crate::lookup::FindableResource {
    config,
    manifest: None, // or Some(&manifest) if the caller has loaded one
    owner: None,
    context: context.map(str::to_string),
    doc_type: temper_core::frontmatter::DocType::from_str(doc_type)?,
    slug_or_suffix: slug.to_string(),
})?;
let (path, ctx) = (resolved.path, resolved.context);
```

If the caller also wants the resolved `resource_id` (e.g., the `resolve_resource_id` site at line ~925), use `resolved.resource_id` directly instead of the secondary frontmatter parse. For sites that load a manifest later, pass `manifest: Some(&manifest)` if the manifest is already in scope at the lookup point.

- [ ] **Step 3: Delete the old `find_resource_file` function**

Once all callers are migrated, remove the function definition at line ~1207. Run `cargo check -p temper-cli` to confirm there are no remaining callers (the compiler will flag any).

- [ ] **Step 4: Run the full crate test suite**

```bash
cargo nextest run -p temper-cli
```

Expected: all tests pass. The migration is mechanical so existing tests should be unaffected. If any test fails, it's a sign the lookup behavior changed in a way the test caught — investigate before continuing. **Do not soften error handling to make tests pass** (per `feedback_subagent_escalate_not_soften` in user memory).

- [ ] **Step 5: `cargo make check`, then commit**

```bash
cargo make check && git add -A && git commit -m "$(cat <<'EOF'
refactor(cli): migrate find_resource_file callers to find_resource

Mechanical refactor of all callers in commands/resource.rs to use the
new typed FindableResource lookup. The old find_resource_file function
is deleted; DocType::from_str gates the input string at each call site
(retired wholesale at the clap boundary in the next task).

C.1 fix lands by construction: the new lookup does not call slugify
on its input.
EOF
)"
```

---

### Task 6: Retire `validate_doc_type` in favor of `DocType::from_str` at clap boundary

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs`

The const + function pair at lines 19-29 is the stringly-typed legacy. After Task 5, internal callers go through `DocType::from_str`. Now we move that boundary parse to the clap layer once, and remove the redundant validation helper.

- [ ] **Step 1: Locate the clap entry points**

```bash
grep -n "doc_type\b" crates/temper-cli/src/commands/resource.rs | head -30
```

Identify the public entry functions (likely `pub fn show`, `pub fn create`, `pub fn update`, `pub fn delete` etc.) that take `doc_type: &str` from clap.

- [ ] **Step 2: Change each public entry signature to take `DocType`**

For each entry function, change the parameter type from `doc_type: &str` (or similar) to `doc_type: temper_core::frontmatter::DocType`. The clap layer (in `crates/temper-cli/src/main.rs` or wherever args are parsed) needs to call `DocType::from_str(&raw)?` once before dispatching.

If clap arg parsing is via a derive — check `crates/temper-cli/src/main.rs` for a struct with `#[derive(Parser)]` or `#[derive(Args)]` — add an `#[arg(value_parser = DocType::from_str)]` attribute or convert the field type via a `value_parser`.

If clap parsing is manual — find the dispatch site and add the `DocType::from_str(...)?` call before the function call.

- [ ] **Step 3: Remove `validate_doc_type` and `VALID_DOC_TYPES`**

Delete lines 19-29 (the const + function). Remove all `validate_doc_type(doc_type)?;` calls inside `commands/resource.rs`. The DocType type eliminates the need.

- [ ] **Step 4: Compile, run tests**

```bash
cargo check -p temper-cli
cargo nextest run -p temper-cli
```

Expected: clean. The compiler will flag any missed migration sites; chase them down.

- [ ] **Step 5: `cargo make check`, then commit**

```bash
cargo make check && git add -A && git commit -m "$(cat <<'EOF'
refactor(cli): retire stringly-typed validate_doc_type

Internal CLI APIs now pass DocType directly. Boundary parsing happens
once at the clap layer via DocType::from_str. The VALID_DOC_TYPES const
and validate_doc_type(&str) helper are removed; DocType's exhaustive
enum + from_str is the single source of truth.

Per `feedback_no_stringly_typed_match` in project guidance: match on
enums directly when you own the bounded set.
EOF
)"
```

---

### Task 7: Add API fallback in `temper resource show` for cloud-only resources

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (`show_generic`)
- Modify: `crates/temper-cli/src/commands/task.rs` (`show`)
- Modify: `crates/temper-cli/src/commands/session.rs` (`show`)
- Create: `tests/e2e/tests/cloud_only_show_fallback_test.rs`

Goal: when a `temper resource show <slug>` call in local mode fails to find the file locally, fall back to the API. The user sees the body without an error (silent recovery). On API failure, emit a clearer error than the current "task not found".

- [ ] **Step 1: Write the failing e2e test**

Create `tests/e2e/tests/cloud_only_show_fallback_test.rs`:

```rust
//! E2E test: `temper resource show` falls back to the API when the
//! local vault file is missing but the resource exists upstream.

#![cfg(feature = "test-db")]

mod common;

use common::e2e_harness::E2EHarness;

#[tokio::test]
async fn resource_show_falls_back_to_api_when_local_missing() {
    let h = E2EHarness::new().await;
    let user = h.create_user_and_login("show-fallback@example.com").await;

    // Create a resource directly via the API — no local file written.
    let api_body = "# Server-side body\n\nThis exists upstream only.\n";
    let resource = user
        .ingest_task(
            "temper",
            "server-only-task",
            "Server Only Task",
            api_body,
        )
        .await;

    // Ensure no local file exists.
    let vault_layout = h.vault_layout(&user);
    let me_path = vault_layout
        .doc_type_dir("@me", "temper", "task")
        .join("server-only-task.md");
    assert!(
        !me_path.exists(),
        "test setup: local file should not exist before the show call"
    );

    // Run `temper resource show` against the local-mode CLI.
    let output = h
        .run_cli(&user, &["resource", "show", "server-only-task", "--type", "task", "--context", "temper"])
        .await;

    assert!(
        output.status.success(),
        "show should succeed via API fallback; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Server-side body"),
        "API-fetched body should be in stdout; got: {stdout}"
    );

    // Vault should remain untouched — the fallback does not write to disk.
    assert!(
        !me_path.exists(),
        "API fallback should not materialize the file locally; recovery is via `temper sync run`"
    );

    let _ = resource;
}
```

> **Note for the implementer:** the exact harness method names (`create_user_and_login`, `ingest_task`, `vault_layout`, `run_cli`) may differ from the existing e2e harness API. Read `tests/e2e/tests/common/mod.rs` and the most recent e2e test (e.g. find one with `grep -l "ingest_task\|create_user" tests/e2e/tests/*.rs`) to discover the actual method shapes, and adjust the test to match. The test's *intent* is what matters: API-only resource → `temper resource show` succeeds → no local file written. Do NOT change the test to softer assertions; if the harness lacks a method you need, escalate (per `feedback_subagent_escalate_not_soften`).

- [ ] **Step 2: Run the e2e test to verify it fails**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db resource_show_falls_back_to_api_when_local_missing
```

Expected: FAIL with the current error wording from `find_task` / `find_resource_file` ("task not found: server-only-task").

- [ ] **Step 3: Add API fallback to `task::show`**

In `crates/temper-cli/src/commands/task.rs::show`, change the local-mode branch. Currently:

```rust
        VaultState::Local => {
            let task = find_task(config, slug_or_suffix, context)?
                .ok_or_else(|| TemperError::Vault(format!("task not found: {slug_or_suffix}")))?;
            // ... rest of local rendering ...
        }
```

After:

```rust
        VaultState::Local => {
            match find_task(config, slug_or_suffix, context)? {
                Some(task) => {
                    // existing local-rendering logic, unchanged
                }
                None => {
                    // Local lookup miss: fall back to the API. The vault
                    // stays untouched — recovery to disk happens via
                    // `temper sync run`.
                    return show_via_api_fallback(
                        config,
                        "task",
                        slug_or_suffix,
                        context,
                        format,
                    );
                }
            }
        }
```

Refactor the existing local-rendering body into a helper or keep inline as the `Some(task) => { ... }` arm — implementer's choice. Add the new helper at the bottom of `task.rs` or as a `pub(crate)` helper in `commands/resource.rs`:

```rust
/// API fallback for `temper resource show` when the local vault file is
/// missing in local mode. Resolves the resource id via
/// `GET /api/resources/by-uri`, fetches body via
/// `GET /api/resources/{id}/content`, and prints it. Does not write to
/// the vault — recovery is via `temper sync run`.
pub(crate) fn show_via_api_fallback(
    config: &Config,
    doc_type: &str,
    slug_or_suffix: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::runtime;
    use temper_core::types::VaultState;

    let ctx = context.map(str::to_string);
    let slug = slug_or_suffix.to_string();
    let dt = doc_type.to_string();
    let config_clone = config.clone();
    let format_owned = format.to_string();

    let body = runtime::with_client(|client| {
        Box::pin(async move {
            let id = super::resource::resolve_resource_id(
                &config_clone,
                client,
                &dt,
                &slug,
                ctx.as_deref(),
                VaultState::Local,
            )
            .await
            .map_err(|e| {
                // Distinguish offline vs not-found-on-server.
                if let TemperError::Client(_) = &e {
                    TemperError::Vault(format!(
                        "couldn't reach server to verify resource exists; \
                         offline lookup failed for {slug}"
                    ))
                } else {
                    TemperError::Vault(format!(
                        "{dt} not found locally or on server: {slug}"
                    ))
                }
            })?;
            let content = client
                .resources()
                .content(uuid::Uuid::from(id))
                .await
                .map_err(crate::commands::client_err)?;

            if format_owned == "json" {
                Ok(serde_json::to_string_pretty(&content)
                    .map_err(|e| TemperError::Vault(format!("json serialization failed: {e}")))?)
            } else {
                Ok(content.markdown)
            }
        })
    })?;

    print!("{body}");
    Ok(())
}
```

> **Note:** the exact error variant names (`TemperError::Client`, `TemperError::Vault`) and the client method (`content`) need verification against current code. `grep -n "pub enum TemperError\|TemperError::Client" crates/temper-cli/src/error.rs` and `grep -n "pub async fn content" crates/temper-client/src/resources.rs` — adjust if names differ.

- [ ] **Step 4: Add the same fallback to `session::show`**

Mirror the change in `crates/temper-cli/src/commands/session.rs::show`. The structure is the same: when `find_session` (or whatever the equivalent primitive is) returns `None` in local mode, call `show_via_api_fallback(config, "session", slug_or_suffix, context, format)`.

If `commands/session.rs` doesn't have an analogous `find_session` and instead routes through `find_resource_file` in `show_generic`, then Step 5's `show_generic` change handles sessions too — skip this step in that case.

- [ ] **Step 5: Add the fallback to `show_generic`**

In `crates/temper-cli/src/commands/resource.rs::show_generic`, after the call to `find_resource(...)?` (added in Task 5), wrap the call in a `match` and on `Err(TemperError::Vault(msg)) if msg.contains("not found")` route to `show_via_api_fallback`.

The simplest sketch:

```rust
fn show_generic(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    let resolved = match crate::lookup::find_resource(crate::lookup::FindableResource {
        config,
        manifest: None,
        owner: None,
        context: context.map(str::to_string),
        doc_type: temper_core::frontmatter::DocType::from_str(doc_type)?,
        slug_or_suffix: slug.to_string(),
    }) {
        Ok(r) => r,
        Err(TemperError::Vault(msg)) if msg.starts_with(&format!("{doc_type} not found")) => {
            // Local lookup miss. In Local mode, fall back to the API.
            // In Cloud mode the upstream lookup already happened at a
            // higher layer; surface the original error.
            use temper_core::types::VaultState;
            return match VaultState::from_env() {
                VaultState::Local => crate::commands::task::show_via_api_fallback(
                    config, doc_type, slug, context, format,
                ),
                VaultState::Cloud => Err(TemperError::Vault(msg)),
            };
        }
        Err(e) => return Err(e),
    };

    // ... existing render logic using `resolved.path` ...
}
```

Adjust the `show_via_api_fallback` path qualifier (`crate::commands::task::show_via_api_fallback`) to wherever you placed the helper.

- [ ] **Step 6: Run the e2e test to verify it passes**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db resource_show_falls_back_to_api_when_local_missing
```

Expected: PASS.

- [ ] **Step 7: Run the full unit suite + workspace check**

```bash
cargo nextest run -p temper-cli
cargo nextest run --workspace
```

Expected: all green.

- [ ] **Step 8: `cargo make check`, then commit**

```bash
cargo make check && git add -A && git commit -m "$(cat <<'EOF'
feat(cli): API fallback for resource show when local file is missing

In local mode, when find_task / find_session / find_resource returns
NotFound, fall back to GET /api/resources/by-uri + content. The user
sees the body without an error — recovery to disk remains
`temper sync run` territory (no implicit write here).

On API failure, distinguishes offline ("couldn't reach server …") from
genuine-not-found ("<doctype> not found locally or on server: <slug>"),
both clearer than the prior local-only "<doctype> not found" message.

Closes symptom #1 of the cloud-only-sync bug task.
EOF
)"
```

End of Work Set A.

---

# Work Set B — `LocallyMissing` state, sync classifier fix, owner reversal

This work set introduces a new manifest state and changes write paths. Depends on Work Set A's lookup tolerance for legacy `@<profile.slug>/` files to remain reachable after the canonical-direction reversal.

---

### Task 8: Add `ManifestEntryState::LocallyMissing` variant

**Files:**
- Modify: `crates/temper-core/src/types/manifest.rs`

- [ ] **Step 1: Add a failing serde round-trip test**

In the `tests` mod at the bottom of `crates/temper-core/src/types/manifest.rs`, extend the existing `test_manifest_entry_state_serde`:

```rust
    #[test]
    fn test_manifest_entry_state_serde() {
        let states = [
            (ManifestEntryState::Clean, "\"clean\""),
            (ManifestEntryState::LocalModified, "\"local_modified\""),
            (ManifestEntryState::RemoteModified, "\"remote_modified\""),
            (ManifestEntryState::Conflict, "\"conflict\""),
            (ManifestEntryState::Pending, "\"pending\""),
            (ManifestEntryState::LocallyMissing, "\"locally_missing\""),
        ];
        for (state, expected_json) in &states {
            let json = serde_json::to_string(state).unwrap();
            assert_eq!(&json, expected_json);
            let parsed: ManifestEntryState = serde_json::from_str(&json).unwrap();
            assert_eq!(*state, parsed);
        }
    }
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-core test_manifest_entry_state_serde
```

Expected: FAIL — `LocallyMissing` is not a variant.

- [ ] **Step 3: Add the variant**

In `crates/temper-core/src/types/manifest.rs:11`, the enum becomes:

```rust
pub enum ManifestEntryState {
    /// Local hash = manifest hash = remote hash
    Clean,
    /// Local hash != manifest hash (local edits since last sync)
    LocalModified,
    /// Remote hash changed (detected on next sync/status check)
    RemoteModified,
    /// Both sides changed; `.conflict.md` materialized alongside
    Conflict,
    /// Subscribed but not yet materialized (new resource from server)
    Pending,
    /// Manifest entry exists but the local vault file is missing on
    /// disk. Set by rehash/normalize when the file has been removed
    /// (deliberately or accidentally — there is no implicit-delete
    /// path; deletes go through `temper resource delete`). The next
    /// sync run reclassifies this as a pull-recovery target.
    ///
    /// Phase 6 hand-off: when the per-resource state machine lands,
    /// this variant either folds into `Synced` with a transient
    /// recovery flag, or stays as a discrete state with documented
    /// transitions out of it (after-pull → `Synced`; force-delete →
    /// removed from manifest entirely).
    LocallyMissing,
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p temper-core
```

Expected: serde test passes. Other tests continue to pass.

- [ ] **Step 5: `cargo make check`, then commit**

```bash
cargo make check && git add -A && git commit -m "$(cat <<'EOF'
feat(core): add ManifestEntryState::LocallyMissing variant

Set by sync rehash/normalize when a tracked file is missing on disk.
Routed through the pull set by the orchestrator (next task) instead of
being misclassified as LocalModified → push-with-empty-body.

Phase 6 hand-off: when the per-resource state machine lands, this
variant either folds into Synced with a transient recovery flag or
stays as a discrete state with documented transitions.
EOF
)"
```

---

### Task 9: `rehash_manifest` sets `LocallyMissing` for missing files

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:465-528`

- [ ] **Step 1: Write a failing unit test**

Find the `tests` mod in `crates/temper-cli/src/actions/sync.rs` (around line 2683 is one test mod) — add a new test:

```rust
    #[test]
    fn rehash_marks_missing_file_as_locally_missing() {
        use std::collections::HashMap;
        use chrono::Utc;
        use temper_core::types::ids::ResourceId;
        use temper_core::types::manifest::{
            Manifest, ManifestEntry, ManifestEntryState,
        };
        use uuid::Uuid;

        let tmp = tempfile::TempDir::new().unwrap();
        let id = ResourceId::from(Uuid::now_v7());

        let mut manifest = Manifest {
            device_id: "test".to_string(),
            last_sync: None,
            entries: HashMap::new(),
        };
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/task/missing.md".to_string(),
                body_hash: "sha256:knownbody".to_string(),
                remote_body_hash: "sha256:knownremote".to_string(),
                managed_hash: "sha256:knownmanaged".to_string(),
                open_hash: "sha256:knownopen".to_string(),
                remote_managed_hash: "sha256:knownremmanaged".to_string(),
                remote_open_hash: "sha256:knownremopen".to_string(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: Some(1_700_000_000),
                provisional: false,
                last_audit_id: None,
            },
        );

        // The file does NOT exist under tmp.path() — that's the test.
        let _ = rehash_manifest(&mut manifest, tmp.path()).unwrap();

        let entry = manifest.entries.get(&id).unwrap();
        assert_eq!(
            entry.state,
            ManifestEntryState::LocallyMissing,
            "missing file should reclassify as LocallyMissing"
        );
        assert_eq!(
            entry.body_hash, "sha256:knownbody",
            "body_hash should be preserved (not cleared) so server-diff has hash to compare"
        );
        assert_eq!(
            entry.managed_hash, "sha256:knownmanaged",
            "managed_hash should also be preserved"
        );
    }
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-cli rehash_marks_missing_file_as_locally_missing
```

Expected: FAIL — current `rehash_manifest` sets `LocalModified` and clears `body_hash`.

- [ ] **Step 3: Update `rehash_manifest`**

Replace the missing-file branch in `crates/temper-cli/src/actions/sync.rs:469-477`:

```rust
        if !file_path.exists() {
            if entry.state != ManifestEntryState::LocalModified {
                entry.state = ManifestEntryState::LocalModified;
                entry.body_hash = String::new();
                entry.mtime_secs = None;
                changed += 1;
            }
            continue;
        }
```

Becomes:

```rust
        if !file_path.exists() {
            if entry.state != ManifestEntryState::LocallyMissing {
                entry.state = ManifestEntryState::LocallyMissing;
                // Body / managed / open hashes are PRESERVED here — the
                // server-side diff path uses them to confirm we're not
                // sending stale partial state. mtime is cleared because
                // the file no longer exists.
                entry.mtime_secs = None;
                changed += 1;
            }
            continue;
        }
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p temper-cli rehash_marks_missing_file_as_locally_missing
cargo nextest run -p temper-cli
```

Expected: new test passes; full crate passes.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
fix(sync): rehash sets LocallyMissing instead of LocalModified

When a tracked file is missing on disk, rehash_manifest now sets the
entry's state to LocallyMissing and preserves all hashes. The previous
behavior (LocalModified + cleared body_hash) caused the server-diff
phase to treat the empty hash as a local change and route the entry to
the push set, where it would fail with "vault file missing".

The pull-set routing for LocallyMissing entries lands in the next two
tasks (normalize_all_entries + sync orchestration).
EOF
)"
```

---

### Task 10: `normalize_all_entries` sets `LocallyMissing` for missing files

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:365-378`

- [ ] **Step 1: Write a failing unit test**

Add to the same `tests` mod (or wherever `normalize_all_entries` tests live — search with `grep -n "fn.*normalize_all_entries\|normalize_all_entries(" crates/temper-cli/src/actions/sync.rs | grep -i test`):

```rust
    #[test]
    fn normalize_marks_missing_file_as_locally_missing() {
        use std::collections::HashMap;
        use chrono::Utc;
        use temper_core::types::ids::ResourceId;
        use temper_core::types::manifest::{
            Manifest, ManifestEntry, ManifestEntryState,
        };
        use uuid::Uuid;

        let tmp = tempfile::TempDir::new().unwrap();
        let temper_dir = tmp.path().join(".temper");
        std::fs::create_dir_all(&temper_dir).unwrap();
        let id = ResourceId::from(Uuid::now_v7());

        let mut manifest = Manifest {
            device_id: "test".to_string(),
            last_sync: None,
            entries: HashMap::new(),
        };
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/task/missing.md".to_string(),
                body_hash: "sha256:keepme".to_string(),
                remote_body_hash: "sha256:remote".to_string(),
                managed_hash: "sha256:keepmanaged".to_string(),
                open_hash: "sha256:keepopen".to_string(),
                remote_managed_hash: "sha256:rmanaged".to_string(),
                remote_open_hash: "sha256:ropen".to_string(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: Some(1_700_000_000),
                provisional: false,
                last_audit_id: None,
            },
        );

        let _report =
            normalize_all_entries(&mut manifest, tmp.path(), &temper_dir, None).unwrap();

        let entry = manifest.entries.get(&id).unwrap();
        assert_eq!(entry.state, ManifestEntryState::LocallyMissing);
        assert_eq!(
            entry.body_hash, "sha256:keepme",
            "body_hash preserved"
        );
    }
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-cli normalize_marks_missing_file_as_locally_missing
```

Expected: FAIL — current code sets LocalModified.

- [ ] **Step 3: Update `normalize_all_entries`**

In `crates/temper-cli/src/actions/sync.rs:362-379`, the missing-file branch is:

```rust
        let abs_path = vault_root.join(&rel_path);

        // Missing file: mirror rehash_manifest's prior behavior.
        if !abs_path.exists() {
            report.missing += 1;
            if let Some(entry) = manifest.entries.get_mut(id) {
                if entry.state != ManifestEntryState::LocalModified {
                    entry.state = ManifestEntryState::LocalModified;
                }
                entry.body_hash = String::new();
                entry.mtime_secs = None;
            }
            crate::manifest_io::save_manifest(temper_dir, manifest)?;
            if let Some(p) = progress {
                p.rehash_progress(idx + 1, total, 0);
            }
            continue;
        }
```

Replace with:

```rust
        let abs_path = vault_root.join(&rel_path);

        // Missing file: mark as LocallyMissing so the orchestration
        // pull set picks it up. Preserve hashes — the server-diff path
        // uses them to confirm we're not sending stale partial state.
        if !abs_path.exists() {
            report.missing += 1;
            if let Some(entry) = manifest.entries.get_mut(id) {
                if entry.state != ManifestEntryState::LocallyMissing {
                    entry.state = ManifestEntryState::LocallyMissing;
                }
                entry.mtime_secs = None;
            }
            crate::manifest_io::save_manifest(temper_dir, manifest)?;
            if let Some(p) = progress {
                p.rehash_progress(idx + 1, total, 0);
            }
            continue;
        }
```

- [ ] **Step 4: Run tests, commit**

```bash
cargo nextest run -p temper-cli normalize_marks_missing_file_as_locally_missing
cargo nextest run -p temper-cli
git add -A && git commit -m "$(cat <<'EOF'
fix(sync): normalize_all_entries sets LocallyMissing for missing files

Mirrors the rehash_manifest fix from the prior commit. The body_hash
is preserved instead of cleared, so the diff path can still compare
against the server's view if needed. mtime is cleared because the
file no longer exists.
EOF
)"
```

---

### Task 11: Sync orchestration routes `LocallyMissing` to pull set

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs::sync_orchestration` (around line 780-908)
- Create: `tests/e2e/tests/locally_missing_recovery_test.rs`

- [ ] **Step 1: Write the failing e2e test**

Create `tests/e2e/tests/locally_missing_recovery_test.rs`:

```rust
//! E2E test: `temper sync run` reclassifies a missing-but-tracked file
//! as LocallyMissing and pulls it back from the server, instead of
//! erroring with "vault file missing".

#![cfg(feature = "test-db")]

mod common;

use common::e2e_harness::E2EHarness;

#[tokio::test]
async fn sync_run_pulls_locally_missing_entries() {
    let h = E2EHarness::new().await;
    let user = h.create_user_and_login("locally-missing@example.com").await;

    // 1. Create a resource locally + push so the manifest tracks it.
    let body = "# Recoverable\n\nThis file gets removed and recovered.\n";
    user.create_local_task("temper", "recoverable", "Recoverable", body)
        .await;
    h.run_sync(&user).await;

    // 2. Remove the local file.
    let vault_layout = h.vault_layout(&user);
    let me_path = vault_layout
        .doc_type_dir("@me", "temper", "task")
        .join("recoverable.md");
    assert!(me_path.exists(), "test setup: file should exist before removal");
    std::fs::remove_file(&me_path).unwrap();

    // 3. Run sync. Should NOT push-fail; should pull the file back.
    let output = h.run_cli(&user, &["sync", "run"]).await;
    assert!(
        output.status.success(),
        "sync should succeed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // 4. File reappears with original content.
    assert!(me_path.exists(), "file should be restored by sync run");
    let restored = std::fs::read_to_string(&me_path).unwrap();
    assert!(
        restored.contains("Recoverable"),
        "restored body should match server-side content; got: {restored}"
    );

    // 5. Push set is empty (the file was never pushed; only pulled).
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("push: 0") || stdout.contains("Push    0"),
        "push count should be zero; stdout: {stdout}"
    );
}
```

> Same harness-API caveat as Task 7: read `tests/e2e/tests/common/mod.rs` and an existing sync e2e test for actual method names. Adjust the test to match without softening the assertions.

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db sync_run_pulls_locally_missing_entries
```

Expected: FAIL — current orchestration tries to push the missing file and errors out.

- [ ] **Step 3: Add post-diff routing of `LocallyMissing` entries**

In `crates/temper-cli/src/actions/sync.rs::sync_orchestration` (line 780+), after the diff request returns (line ~801) and before the push loop (line ~808), insert a synthesis step that builds extra pull items for any `LocallyMissing` manifest entries that aren't already in `diff.to_pull` and excludes them from `diff.to_push`:

```rust
    // ----- LocallyMissing post-diff routing ----------------------------
    //
    // Manifest entries marked LocallyMissing during rehash/normalize
    // need a pull, not a push. The server-side diff doesn't know about
    // this client-side state, so we patch the diff after it returns:
    // (a) drop any to_push items whose manifest entry is LocallyMissing,
    // (b) ensure each LocallyMissing entry appears in to_pull as a
    //     synthesized SyncPullItem with kind=Body.
    let locally_missing_ids: std::collections::HashSet<ResourceId> = manifest
        .entries
        .iter()
        .filter_map(|(id, e)| {
            if e.state == ManifestEntryState::LocallyMissing {
                Some(*id)
            } else {
                None
            }
        })
        .collect();

    let mut diff = diff;
    if !locally_missing_ids.is_empty() {
        diff.to_push.retain(|item| {
            item.resource_id
                .map(|id| !locally_missing_ids.contains(&id))
                .unwrap_or(true)
        });
        let already_in_pull: std::collections::HashSet<ResourceId> =
            diff.to_pull.iter().map(|p| p.resource_id).collect();
        for id in &locally_missing_ids {
            if already_in_pull.contains(id) {
                continue;
            }
            // Synthesize a pull item. Look up the manifest entry to
            // construct the URI.
            if let Some(entry) = manifest.entries.get(id) {
                if let Some(parsed) = Vault::parse_rel(&entry.path) {
                    let uri = Vault::canonical_uri(
                        parsed.owner,
                        parsed.context,
                        parsed.doc_type,
                        &uuid::Uuid::from(*id).to_string(),
                    );
                    diff.to_pull.push(SyncPullItem {
                        resource_id: *id,
                        uri,
                        kind: SyncItemKind::Body,
                        // expected_remote_hash: server's hash if known,
                        // else None — pull_one_resource handles None.
                        content_hash: entry.remote_body_hash.clone(),
                    });
                    tracing::info!(
                        path = %entry.path,
                        "LocallyMissing → pull recovery"
                    );
                }
            }
        }
    }
    let push_count = diff.to_push.len();
    let pull_count = diff.to_pull.len();
```

> **Note:** the exact `SyncPullItem` field names (`resource_id`, `uri`, `kind`, `content_hash`) need verification. Check `grep -n "pub struct SyncPullItem" crates/temper-core/src/types/sync.rs crates/temper-client/src/sync.rs` and adjust. If `content_hash` is required and `entry.remote_body_hash` may be empty, pass an empty string (the server-side hash, which is what `pull_one_resource` writes back as `remote_body_hash` regardless).

- [ ] **Step 4: Move the existing `let push_count = diff.to_push.len();` and `let pull_count = ...` up if they were already present, OR delete the duplicate**

The original code at line 803-804 has these computed BEFORE the post-diff routing. Move them to after, or replace them by the new ones in the patch above. Use a single source of truth — no duplicated `let push_count` lines.

- [ ] **Step 5: Run tests**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db sync_run_pulls_locally_missing_entries
cargo nextest run -p temper-cli
```

Expected: e2e passes; unit suite green. **Run the full workspace** to catch feature-unification surprises:

```bash
cargo nextest run --workspace
```

- [ ] **Step 6: `cargo make check`, then commit**

```bash
cargo make check && git add -A && git commit -m "$(cat <<'EOF'
fix(sync): route LocallyMissing entries through the pull set

After the server diff returns, post-process: drop any to_push items for
manifest entries marked LocallyMissing, and ensure each LocallyMissing
entry is in to_pull (synthesizing a SyncPullItem if needed). The
existing pull_one_resource primitive handles the body fetch + write;
manifest entry's state transitions back to Clean when the pull
completes.

Closes symptoms #2 + #3 of the cloud-only-sync bug task: the user
sees `↓ Pull` (not `push [modified]` followed by an error) and the
file is restored on the next sync run.
EOF
)"
```

---

### Task 12: Reverse PR #72 — own-resource owner canonicalizes to `@me`

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:218-279` (`resolve_owner_for_frontmatter` + `OwnerResolver`)
- Modify: existing tests at `crates/temper-cli/src/actions/sync.rs:3281-3299`

- [ ] **Step 1: Update the existing tests for `resolve_owner_for_frontmatter`**

Find the tests in `crates/temper-cli/src/actions/sync.rs` (search `grep -n "fn resolve_owner_for_frontmatter_resolves_at_me\|fn resolve_owner_for_frontmatter_passes_through" crates/temper-cli/src/actions/sync.rs`):

The existing test at line ~3281 is:

```rust
    #[test]
    fn resolve_owner_for_frontmatter_resolves_at_me() {
        let result = resolve_owner_for_frontmatter("@me", "j-cole-taylor");
        assert_eq!(result, "@j-cole-taylor"); // OLD behavior — wrong direction
    }
```

Replace with:

```rust
    #[test]
    fn resolve_owner_for_frontmatter_keeps_at_me_canonical() {
        // Design intent: @me is the canonical local-vault owner for the
        // user's own private work. Both the API's @me shorthand and an
        // explicit @<profile.slug> for the same user normalize to @me.
        // PR #72's reverse direction (@me → @<slug>) is reverted by
        // this change.
        let result = resolve_owner_for_frontmatter("@me", "j-cole-taylor");
        assert_eq!(result, "@me");
    }

    #[test]
    fn resolve_owner_for_frontmatter_normalizes_explicit_own_slug_to_at_me() {
        // If the API ever returns the user's explicit slug instead of
        // @me, normalize it back to @me to keep frontmatter consistent.
        let result = resolve_owner_for_frontmatter("@j-cole-taylor", "j-cole-taylor");
        assert_eq!(result, "@me");
    }

    #[test]
    fn resolve_owner_for_frontmatter_passes_through_team_handle() {
        let result = resolve_owner_for_frontmatter("+temper-team", "j-cole-taylor");
        assert_eq!(result, "+temper-team");
    }

    #[test]
    fn resolve_owner_for_frontmatter_passes_through_other_user() {
        let result = resolve_owner_for_frontmatter("@someone-else", "j-cole-taylor");
        assert_eq!(result, "@someone-else");
    }
```

- [ ] **Step 2: Run to verify the new tests fail**

```bash
cargo nextest run -p temper-cli resolve_owner_for_frontmatter
```

Expected: the two new tests (`keeps_at_me_canonical`, `normalizes_explicit_own_slug_to_at_me`) FAIL.

- [ ] **Step 3: Update `resolve_owner_for_frontmatter`**

Replace the body at line 218-224:

```rust
pub fn resolve_owner_for_frontmatter(handle: &str, profile_slug: &str) -> String {
    if handle == "@me" {
        format!("@{profile_slug}")
    } else {
        handle.to_string()
    }
}
```

With:

```rust
/// Resolve the API's `owner_handle` to the canonical local-vault owner
/// sigil. Design intent (clarified 2026-05-10): `@me` is canonical for
/// the user's own private work. Both the API's `@me` shorthand and an
/// explicit `@<profile.slug>` for the same user normalize to `@me`.
/// Other users' personal handles (`@<other-slug>`) and team handles
/// (`+<team-slug>`) pass through unchanged.
///
/// This reverses the direction PR #72 introduced. The
/// `owners_equivalent` helper in this module remains in place to
/// tolerate frontmatter and manifest paths that still carry the
/// old `@<profile.slug>` form (legacy files from the PR #70/72
/// window).
pub fn resolve_owner_for_frontmatter(handle: &str, profile_slug: &str) -> String {
    let own_slug_handle = format!("@{profile_slug}");
    if handle == "@me" || handle == own_slug_handle {
        "@me".to_string()
    } else {
        handle.to_string()
    }
}
```

- [ ] **Step 4: Update `OwnerResolver::resolve`**

Replace lines 264-278:

```rust
    pub async fn resolve(&mut self, handle: &str) -> crate::error::Result<String> {
        if handle != "@me" {
            return Ok(handle.to_string());
        }
        if self.canonical.is_none() {
            let profile = self
                .client
                .profile()
                .get()
                .await
                .map_err(crate::commands::client_err)?;
            self.canonical = Some(format!("@{}", profile.slug));
        }
        Ok(self.canonical.as_deref().unwrap().to_string())
    }
```

With:

```rust
    pub async fn resolve(&mut self, handle: &str) -> crate::error::Result<String> {
        // Quick path: anything that's already not own-user passes through.
        if handle == "@me" {
            return Ok("@me".to_string());
        }
        // For anything else, fetch the profile so we can compare against
        // the user's explicit slug.
        if self.canonical.is_none() {
            let profile = self
                .client
                .profile()
                .get()
                .await
                .map_err(crate::commands::client_err)?;
            self.canonical = Some(profile.slug);
        }
        let own_slug = self.canonical.as_deref().unwrap();
        if handle.strip_prefix('@') == Some(own_slug) {
            Ok("@me".to_string())
        } else {
            Ok(handle.to_string())
        }
    }
```

Note: `self.canonical` now stores the bare `profile.slug` (no leading `@`) so the comparison is unambiguous. Update the comment on the field:

```rust
pub struct OwnerResolver<'c> {
    client: &'c temper_client::TemperClient,
    /// Cached `profile.slug` (without leading `@`). Used to recognize
    /// own-user explicit handles like `@<profile.slug>` and normalize
    /// them to `@me`.
    canonical: Option<String>,
}
```

- [ ] **Step 5: Existing call sites: ensure `canonical_owner` resolves to `@me` for own resources**

Search for `canonical_owner` consumers:

```bash
grep -n "canonical_owner" crates/temper-cli/src/actions/sync.rs | head -20
```

The sites at lines 1546, 1551, 1739, 1748, 1770, 1789, 1799, 1910, 1919, and the test fixture at 4117 all flow through `OwnerResolver::resolve` — the change in Step 4 propagates automatically. The test fixture at line 4117 currently passes `canonical_owner: "@me"` already (or `"@j-cole-taylor"` — verify). If any test asserts `canonical_owner == "@<slug>"`, update it to `"@me"`.

- [ ] **Step 6: Update or remove the e2e regression test from PR #72**

The PR #72 description references `pull_one_resource_newly_tracked_writes_canonical_owner_and_passes_preflight`. Find it:

```bash
grep -rn "pull_one_resource_newly_tracked_writes_canonical_owner" tests/e2e/ crates/temper-cli/
```

If it asserts the pulled file lands at `@<profile.slug>/...`, flip it to assert `@me/...`. Same for any frontmatter-owner assertions: should be `@me`.

- [ ] **Step 7: Run all tests**

```bash
cargo nextest run -p temper-cli
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db
```

Expected: all green. The pull-related tests now assert `@me/`-resident files.

- [ ] **Step 8: `cargo make check`, then commit**

```bash
cargo make check && git add -A && git commit -m "$(cat <<'EOF'
fix(sync): reverse PR #72 — @me is canonical for own resources

Design intent (clarified 2026-05-10 brainstorming): the on-disk vault
is grounded in `@me/` for the user's private work; `@<other-slug>/`
is reserved for team-shared contexts. PR #72 + PR #70 went the wrong
direction, canonicalizing own-resource pulls to `@<profile.slug>/...`
with `temper-owner: '@<profile.slug>'`. This reverses both:

- resolve_owner_for_frontmatter and OwnerResolver::resolve now return
  `@me` for both the API's `@me` shorthand AND for explicit
  `@<profile.slug>` references to the same user.
- `pull_one_resource` and friends (via canonical_owner threading)
  write own-resource files at `@me/<ctx>/<doctype>/...` and
  frontmatter `temper-owner: '@me'`.

owners_equivalent stays load-bearing for legacy `@<profile.slug>/`
files from the PR #70/72 window. find_resource (Work Set A) scans
both `@me/` and `@<profile.slug>/` so nothing becomes unreachable
without a vault migration.
EOF
)"
```

---

### Task 13: Audit create paths for own-resource owner-string sites

**Files:**
- Possibly modify: `crates/temper-cli/src/commands/add.rs`
- Possibly modify: `crates/temper-cli/src/actions/ingest.rs`

**Coordination point:** the sibling B.2 session is touching `commands/add.rs` to thread `owner` through `build_vault_path`. If their work has landed (check `git log --all --oneline | grep -i "thread.*owner\|build_vault_path" | head -5`), this task verifies their change writes `@me` for own resources. If not landed, this task does the audit independently and the B.2 session rebases.

- [ ] **Step 1: Identify own-owner write sites in create paths**

```bash
grep -n "@me\|profile_slug\|owner_handle\|canonical_owner\|temper-owner\|temper_owner" \
    crates/temper-cli/src/commands/add.rs \
    crates/temper-cli/src/actions/ingest.rs
```

Note every site that writes an owner sigil to disk (vault path or frontmatter).

- [ ] **Step 2: Verify each write site uses `@me` for own resources**

For each site, trace where the owner string comes from:
- If from `config.owner_for_context(ctx)` → already returns `@me` for own contexts; verify by reading `crates/temper-cli/src/config.rs::owner_for_context`. If it returns anything else for own contexts, fix it.
- If from `profile.slug` directly → wrong; needs to use `@me` for own resources.
- If from `resource.owner_handle` (API-derived) → routes through `OwnerResolver::resolve`, which now returns `@me` (per Task 12). OK.
- If from a clap argument or interactive prompt → defaults to `@me` (it should already; verify).

If a site needs fixing, change it. If a site is correct already, move on.

- [ ] **Step 3: Add a regression test for the create path**

In `crates/temper-cli/src/commands/add.rs::tests` (or `actions/ingest.rs::tests`, whichever owns the create path):

```rust
    #[test]
    fn local_mode_create_writes_own_resource_under_at_me() {
        // Regression: a `temper resource create` (or equivalent local-mode
        // create path) must write the file under @me/<ctx>/<doctype>/...
        // for own contexts, regardless of whether profile.slug is
        // configured. PR #70/72 broke this; this test pins the fix.
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = Config::default();
        config.vault_root = tmp.path().to_path_buf();
        config.contexts = vec!["temper".to_string()];
        config.profile_slug = Some("j-cole-taylor".to_string());

        // Invoke whichever local-mode create primitive applies. Adjust
        // the call to match the actual API of write_vault_file_and_register
        // or its equivalent. The asserted invariant is the path:
        let path = /* call create primitive */;

        assert!(
            path.starts_with(tmp.path().join("@me")),
            "own-resource create should land under @me/, got {:?}",
            path
        );
    }
```

> **Implementer note:** the exact create primitive depends on what's in `commands/add.rs` after the B.2 session lands. If the B.2 session refactored to a `VaultWritePlan` params struct, use that. Otherwise call `write_vault_file_and_register` (or equivalent) directly with the same arguments the production callers use. The pin is the path invariant.

- [ ] **Step 4: Run the test, fix any sites it surfaces**

```bash
cargo nextest run -p temper-cli local_mode_create_writes_own_resource_under_at_me
```

If FAIL: adjust the offending owner-string source to use `@me`. If PASS: the existing code is correct (likely B.2 already fixed it).

- [ ] **Step 5: Commit**

```bash
cargo make check && git add -A && git commit -m "$(cat <<'EOF'
test(cli): regression test pins @me-resident own-resource creates

Coordination point with the B.2 owner-threading session. Whether B.2
landed first or this PR did, the local-mode create path must write
own-resource files under @me/<ctx>/<doctype>/...; this test pins that
invariant so future refactors don't regress it.

Any owner-string sites that emitted @<profile.slug> for own resources
have been corrected. owners_equivalent (sync.rs) tolerates legacy
files; find_resource (lookup.rs) scans both directories.
EOF
)"
```

---

### Task 14: Update `vault_file_missing_err` message + CLAUDE.md guidance

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:30-37` (`vault_file_missing_err`)
- Modify: `CLAUDE.md`

- [ ] **Step 1: Write a failing snapshot test for the new message**

In `crates/temper-cli/src/actions/sync.rs` tests:

```rust
    #[test]
    fn vault_file_missing_err_points_to_sync_run() {
        let err = vault_file_missing_err("@me/temper/task/foo.md");
        let msg = err.to_string();
        assert!(
            msg.contains("vault file vanished during sync"),
            "expected new wording, got: {msg}"
        );
        assert!(
            msg.contains("temper sync run"),
            "expected recovery to point at sync run, got: {msg}"
        );
        assert!(
            !msg.contains("temper sync refresh"),
            "old wording referenced refresh which doesn't pull; should be removed"
        );
        assert!(
            !msg.contains("temper resource delete"),
            "old wording suggested delete; ambiguous and removed"
        );
    }
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-cli vault_file_missing_err_points_to_sync_run
```

Expected: FAIL.

- [ ] **Step 3: Update the error helper**

Replace lines 23-37 in `crates/temper-cli/src/actions/sync.rs`:

```rust
/// Build the standard "vault file missing for tracked entry" error, with
```
through:
```rust
fn vault_file_missing_err(rel_path: &str) -> TemperError {
    let slug = ...;
    TemperError::Vault(format!(
        "vault file missing for {slug} at {rel_path}\n\nEither:\n  • To delete the resource, run: temper resource delete {slug}\n  • To recover the file from the server, run: temper sync refresh"
    ))
}
```

With:

```rust
/// Build the "vault file vanished during sync" error.
///
/// After the LocallyMissing classifier change (Tasks 9-11), the
/// rehash-time-missing case is reclassified as `LocallyMissing` and
/// routed to the pull set. This helper now covers only the residual
/// race case: the file was present at scan-time but vanished before
/// push could read it. Recovery is `temper sync run` either way.
fn vault_file_missing_err(rel_path: &str) -> TemperError {
    TemperError::Vault(format!(
        "vault file vanished during sync at {rel_path}; run `temper sync run` to recover"
    ))
}
```

The new message no longer derives a slug, so any callers that previously passed extra slug context can stop. Search for callers and adjust:

```bash
grep -n "vault_file_missing_err" crates/temper-cli/src/actions/sync.rs
```

- [ ] **Step 4: Update CLAUDE.md**

Open `CLAUDE.md` and find the paragraph (around line 86-95) starting with:

> "Resource deletion is always explicit ..."

Within that paragraph there's a sentence:

> "There is no implicit-delete-via-`rm` path — removing a tracked vault file outside this command will cause the next `temper sync push` to error with two-pronged guidance: either run `temper resource delete <slug>` to delete the resource, or run `temper sync refresh` to recover the file from the server."

Replace with:

> "There is no implicit-delete-via-`rm` path. To delete a resource, run `temper resource delete <slug>`. To recover a file you removed by accident (or that's missing on a fresh device), just run `temper sync run` — the next sync cycle reclassifies missing-but-tracked files as `LocallyMissing` and pulls them back. `temper sync refresh` is for non-destructive manifest rebuilds against the server's view, not for recovering missing files; do not use it for recovery."

- [ ] **Step 5: Run tests, then commit**

```bash
cargo nextest run -p temper-cli vault_file_missing_err_points_to_sync_run
cargo nextest run -p temper-cli
cargo make check
git add -A && git commit -m "$(cat <<'EOF'
fix(sync): update vault_file_missing_err + CLAUDE.md recovery guidance

After the LocallyMissing classifier change, the rehash-time-missing
case never reaches push. The helper now covers only the residual
race case (file present at scan, gone before push reads it). The new
message points at `temper sync run` for recovery — the prior
two-pronged guidance pointed at `temper sync refresh`, which doesn't
actually pull missing files and was misleading.

CLAUDE.md's deletion paragraph is rewritten to match: explicit
delete via `temper resource delete <slug>`, accidental removal
recovers via `temper sync run`. Refresh is documented as
"non-destructive manifest rebuild" only.
EOF
)"
```

---

### Task 15: Final verification

**Files:** none — verification only.

- [ ] **Step 1: Full `cargo make check`**

```bash
cargo make check
```

Expected: all green (fmt, clippy, docs, machete, TS typecheck, biome).

- [ ] **Step 2: Full `cargo make test-all`**

```bash
cargo make test-all
```

Expected: all green (Rust unit + DB integration + TS tests).

- [ ] **Step 3: Workspace nextest (catches feature unification)**

```bash
cargo nextest run --workspace
```

Expected: green. Per `feedback_workspace_test_surfaces_pipeline_bugs`, this is the canonical "did anything sneak through feature-unification" check.

- [ ] **Step 4: Embed-gated e2e tests**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed
```

Expected: green. Per CLAUDE.md, the Embed CI job is the only place ONNX runtime is wired up; running locally with both features matches the CI tier most likely to surface workspace-feature-unification surprises.

- [ ] **Step 5: Manual smoke test (optional but recommended)**

If a real vault is available:

```bash
# 1. Pick a tracked task file and remove it.
rm /path/to/vault/@me/temper/task/some-task.md

# 2. Confirm `temper resource show <slug>` falls back to API.
temper resource show some-task --type task --context temper
# Expected: body printed, no error, no file written.

# 3. Confirm `temper sync run` recovers the file.
temper sync run
# Expected: line `↓ Pull @me/temper/task/some-task.md`, push count 0,
# file restored on disk.

# 4. Confirm a double-hyphen-slug task is reachable.
temper resource show audit-followups--rationalization-comments-hiding-incomplete-implementations \
    --type task --context temper
# Expected: body printed (C.1 fix).
```

- [ ] **Step 6: Update the bug task and audit-followups task to `done`**

```bash
temper resource update 2026-05-09-fix-sync-handling-of-cloud-only-resources-resource-show-refresh-as-recovery-missing-vs-modified-scan-classification \
    --type task --stage done

temper resource update audit-followups--rationalization-comments-hiding-incomplete-implementations \
    --type task --stage done
```

If either errors with `task not found` because of the C.1 slug-collapse, the lookup change in Task 5 should already have fixed that — the fact that this command now works on its own is the live confirmation of C.1 closure.

- [ ] **Step 7: Final commit (if any uncommitted state remains, e.g., test fixtures)**

```bash
git status --short
# If anything remains, commit with a focused message.
# Otherwise no-op.
```

End of plan.

---

## Self-Review (Plan-Writer Checklist)

Spec coverage check (one bullet per spec section):

- **Architecture > FindableResource lookup type:** Tasks 1-4. Covered.
- **Architecture > LocallyMissing state:** Tasks 8-11. Covered.
- **Architecture > Owner canonicalization reversal:** Task 12. Covered. PR #70's NewlyTracked path-resolution piece is reversed by the OwnerResolver change in Task 12 (since the path is constructed from `canonical_owner = resolver.resolve(...)` per sync.rs:1546-1551). Verified by the test update in Task 12 Step 6.
- **Architecture > Resource show API fallback:** Task 7. Covered.
- **Data Flow > Sync run after fix:** Tasks 9, 10, 11. Covered.
- **Data Flow > Resource show after fix:** Task 7. Covered.
- **Data Flow > Resource create after fix:** Task 13. Covered with coordination-point note.
- **Error Handling & UX:** Task 14. Covered.
- **Testing matrix:** Tasks 2, 3, 4, 7, 9, 10, 11, 12, 13, 14 each include their corresponding row from the spec's testing table.
- **Decomposition > Work Set A vs B:** Tasks 1-7 (A), 8-15 (B).
- **Out-of-scope follow-ups (Phase 6 hand-off, per-resource pull, stringly-typed sweep beyond DocType, OwnerResolver simplification):** captured in spec, not implemented here. Plan does not need explicit tasks.

Placeholder scan: no TBDs or TODOs in step bodies. The "implementer note" callouts in Tasks 7, 11, 13 are pointers to existing code the implementer must read (not deferred work) — acceptable per skill guidance.

Type consistency: `FindableResource`, `ResolvedResource`, `ManifestEntryState::LocallyMissing`, `resolve_owner_for_frontmatter`, `OwnerResolver::resolve`, `vault_file_missing_err` — all named consistently across the tasks where they appear.

Scope check: 15 tasks for one PR is at the upper bound but the spec explicitly chose one-PR-with-two-internal-work-sets. Each task is bite-sized (one TDD red-green-commit cycle).
