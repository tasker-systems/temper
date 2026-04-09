# Owner-Scoped URIs Phase 2 CLI + Vault Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land a `Vault` abstraction in temper-core that centralizes filesystem and `kb://` URI layout, migrate the CLI to consume it, add `temper doctor` + `temper sync` ownership validation, and strip the legacy `resource_for_uri` fallback so the system-access-gate branch can merge and deploy.

**Architecture:** A single `Vault<'a>` struct in `temper-core` owns every rule about how `(owner, context, doc_type, slug)` maps to filesystem paths, manifest entries, and canonical kb:// URIs. Every CLI call site that used to build paths by hand or parse them ad-hoc now calls through this helper. Doctor gains ownership validation rules scoped to what's server-authoritative vs local-authoritative. Sync gains a preflight gate that refuses to upload entries whose frontmatter `temper-owner` disagrees with the manifest's understanding. The legacy no-sigil branch in `resource_for_uri()` is stripped last, after both Pete's vaults have been migrated via a one-off shell script.

**Tech Stack:** Rust (temper-core, temper-cli), PostgreSQL (SQL migration), sqlx (query cache), Bash/jq (one-off shell script).

**Refinement from the spec (Section 1.1):** `Vault` methods take `owner: &str, context: &str` as primitives instead of `&Subscription`. This keeps `temper-core::vault` independent of `vault_config::Subscription` — cleaner layering. Callers that have a `Subscription` in hand call `sub.resolved_owner()` at the call site to pass through to `Vault`. Everything else in the spec is unchanged.

**File structure (new + modified):**

- **New:** `crates/temper-core/src/vault.rs` — `Vault`, `ParsedVaultPath`, `ParsedKbUri`
- **New:** `crates/temper-core/tests/vault_parity.rs` — cross-implementation parity integration test (behind `test-db`)
- **New:** `scripts/migrate-vault-to-owner-segmented.sh` — one-off vault migration (not shipped)
- **New:** `migrations/20260408000001_resource_for_uri_drop_legacy.sql`
- **Modified:** `crates/temper-core/src/lib.rs` (export vault module)
- **Modified:** `crates/temper-cli/src/config.rs` (add `subscriptions` field + `subscription_for_context` helper; delete `doc_type_dir`)
- **Modified:** `crates/temper-cli/src/actions/task.rs` (load_tasks, create use Vault)
- **Modified:** `crates/temper-cli/src/actions/goal.rs` (load_goals, ensure_maintenance, create use Vault; hardcoded format! discovery event strings removed)
- **Modified:** `crates/temper-cli/src/commands/resource.rs` (create_simple_resource uses Vault)
- **Modified:** `crates/temper-cli/src/actions/doctor.rs` (scan iterates subscriptions, dead research block deleted, temper-owner validation added to scan_file)
- **Modified:** `crates/temper-cli/src/actions/doctor_fix.rs` (add `FixAction::SetOwnerField`, `owner_backfilled` counter)
- **Modified:** `crates/temper-cli/src/actions/sync.rs` (build_status_request, push_error_context, parse_kb_uri, rel_path computation all use Vault; add preflight_ownership_check)
- **Modified:** `crates/temper-cli/src/actions/ingest.rs` (infer_context_and_doctype replaced by Vault::parse_rel)

---

## Phase 1 — Vault abstraction

### Task 1: Scaffold `vault.rs` with types and module export

**Files:**
- Create: `crates/temper-core/src/vault.rs`
- Modify: `crates/temper-core/src/lib.rs`

- [ ] **Step 1: Create `vault.rs` with type skeletons only (no method bodies)**

```rust
// crates/temper-core/src/vault.rs
//! Vault layout and kb:// URI construction.
//!
//! Centralizes every rule about how `(owner, context, doc_type, slug)` maps to
//! filesystem paths, manifest-relative path strings, and canonical kb:// URIs.
//! Shared by temper-cli, temper-api, and temper-mcp so all three produce
//! byte-identical paths and URIs for the same inputs.

use std::path::{Path, PathBuf};

/// Owns layout rules for a specific vault root. Construct once per operation;
/// methods are pure functions of the inputs.
pub struct Vault<'a> {
    vault_root: &'a Path,
}

/// A parsed vault-relative path. Borrows from the input string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedVaultPath<'a> {
    /// Owner sigil + identifier, e.g., "@me" or "+platform-eng".
    pub owner: &'a str,
    pub context: &'a str,
    pub doc_type: &'a str,
    /// Filename stem (no .md extension).
    pub slug: &'a str,
}

/// A parsed kb:// URI. Borrows from the input string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedKbUri<'a> {
    /// Owner sigil + identifier, e.g., "@me" or "+platform-eng".
    pub owner: &'a str,
    pub context: &'a str,
    pub doc_type: &'a str,
    /// Identifier — slug or UUID string, caller decides.
    pub ident: &'a str,
}

impl<'a> Vault<'a> {
    /// Construct a Vault for a given vault root directory.
    pub fn new(vault_root: &'a Path) -> Self {
        Self { vault_root }
    }
}
```

- [ ] **Step 2: Add `pub mod vault;` export in `lib.rs`**

Find the existing `pub mod` declarations near the top of `crates/temper-core/src/lib.rs` (around lines 7-10):

```rust
pub mod error;
pub mod ids;
pub mod schema;
pub mod types;
```

Add after `pub mod types;`:

```rust
pub mod vault;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-core`
Expected: Clean compile, no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/vault.rs crates/temper-core/src/lib.rs
git commit -m "feat(core): scaffold Vault abstraction for owner-scoped layout"
```

---

### Task 2: Implement `doc_type_dir`, `doc_file`, `rel_path`

**Files:**
- Modify: `crates/temper-core/src/vault.rs`

- [ ] **Step 1: Write failing unit tests in `vault.rs`**

Append to `crates/temper-core/src/vault.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from("/tmp/test-vault")
    }

    #[test]
    fn doc_type_dir_personal_owner() {
        let v = Vault::new(&root());
        assert_eq!(
            v.doc_type_dir("@me", "temper", "task"),
            PathBuf::from("/tmp/test-vault/@me/temper/task")
        );
    }

    #[test]
    fn doc_type_dir_team_owner() {
        let v = Vault::new(&root());
        assert_eq!(
            v.doc_type_dir("+platform-eng", "temper", "task"),
            PathBuf::from("/tmp/test-vault/+platform-eng/temper/task")
        );
    }

    #[test]
    fn doc_file_builds_full_path_with_md_extension() {
        let v = Vault::new(&root());
        assert_eq!(
            v.doc_file("@me", "temper", "task", "my-task"),
            PathBuf::from("/tmp/test-vault/@me/temper/task/my-task.md")
        );
    }

    #[test]
    fn rel_path_returns_vault_relative_string() {
        let v = Vault::new(&root());
        assert_eq!(
            v.rel_path("@me", "temper", "task", "my-task"),
            "@me/temper/task/my-task.md".to_string()
        );
    }

    #[test]
    fn rel_path_team_owner() {
        let v = Vault::new(&root());
        assert_eq!(
            v.rel_path("+team-x", "general", "goal", "q4-launch"),
            "+team-x/general/goal/q4-launch.md".to_string()
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p temper-core vault::tests`
Expected: FAIL with "no method named `doc_type_dir` found" or similar.

- [ ] **Step 3: Implement the three methods on `impl Vault<'a>`**

Add to the `impl<'a> Vault<'a>` block in `vault.rs` (after `new`):

```rust
    /// Absolute directory where files of a given (owner, context, doc_type) live.
    /// Returns `<vault_root>/<owner>/<context>/<doc_type>/`.
    pub fn doc_type_dir(&self, owner: &str, context: &str, doc_type: &str) -> PathBuf {
        self.vault_root.join(owner).join(context).join(doc_type)
    }

    /// Absolute file path for a specific resource.
    /// Returns `<vault_root>/<owner>/<context>/<doc_type>/<slug>.md`.
    pub fn doc_file(&self, owner: &str, context: &str, doc_type: &str, slug: &str) -> PathBuf {
        self.doc_type_dir(owner, context, doc_type)
            .join(format!("{slug}.md"))
    }

    /// Vault-relative path string used in manifest entries and discovery events.
    /// Returns `<owner>/<context>/<doc_type>/<slug>.md`.
    pub fn rel_path(&self, owner: &str, context: &str, doc_type: &str, slug: &str) -> String {
        format!("{owner}/{context}/{doc_type}/{slug}.md")
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p temper-core vault::tests`
Expected: All 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/vault.rs
git commit -m "feat(core): add Vault doc_type_dir/doc_file/rel_path"
```

---

### Task 3: Implement `parse_rel`

**Files:**
- Modify: `crates/temper-core/src/vault.rs`

- [ ] **Step 1: Write failing unit tests**

Append to the `tests` module in `vault.rs`:

```rust
    #[test]
    fn parse_rel_valid_personal_owner() {
        let parsed = Vault::parse_rel("@me/temper/task/my-task.md").unwrap();
        assert_eq!(parsed.owner, "@me");
        assert_eq!(parsed.context, "temper");
        assert_eq!(parsed.doc_type, "task");
        assert_eq!(parsed.slug, "my-task");
    }

    #[test]
    fn parse_rel_valid_team_owner() {
        let parsed = Vault::parse_rel("+platform-eng/temper/goal/q4.md").unwrap();
        assert_eq!(parsed.owner, "+platform-eng");
        assert_eq!(parsed.context, "temper");
        assert_eq!(parsed.doc_type, "goal");
        assert_eq!(parsed.slug, "q4");
    }

    #[test]
    fn parse_rel_rejects_no_sigil() {
        assert!(Vault::parse_rel("temper/task/my-task.md").is_none());
    }

    #[test]
    fn parse_rel_rejects_too_few_segments() {
        assert!(Vault::parse_rel("@me/temper/task").is_none());
        assert!(Vault::parse_rel("@me/temper").is_none());
        assert!(Vault::parse_rel("@me").is_none());
        assert!(Vault::parse_rel("").is_none());
    }

    #[test]
    fn parse_rel_rejects_non_md_extension() {
        assert!(Vault::parse_rel("@me/temper/task/my-task.txt").is_none());
        assert!(Vault::parse_rel("@me/temper/task/my-task").is_none());
    }

    #[test]
    fn parse_rel_round_trips_with_rel_path() {
        let root = root();
        let v = Vault::new(&root);
        let rel = v.rel_path("@me", "temper", "task", "round-trip");
        let parsed = Vault::parse_rel(&rel).unwrap();
        assert_eq!(parsed.owner, "@me");
        assert_eq!(parsed.context, "temper");
        assert_eq!(parsed.doc_type, "task");
        assert_eq!(parsed.slug, "round-trip");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p temper-core vault::tests::parse_rel`
Expected: FAIL with "no method named `parse_rel` found".

- [ ] **Step 3: Implement `parse_rel` on `impl Vault<'a>`**

Add to the `impl<'a> Vault<'a>` block after `rel_path`:

```rust
    /// Parse a vault-relative path back into components.
    /// Returns `None` if the path is malformed (missing owner sigil, wrong segment
    /// count, or non-`.md` filename).
    ///
    /// Associated function — no Vault instance needed. Callers that only parse
    /// manifest paths do not need to construct a Vault.
    pub fn parse_rel(rel: &str) -> Option<ParsedVaultPath<'_>> {
        let parts: Vec<&str> = rel.split('/').collect();
        if parts.len() != 4 {
            return None;
        }
        let owner = parts[0];
        if !(owner.starts_with('@') || owner.starts_with('+')) {
            return None;
        }
        let context = parts[1];
        let doc_type = parts[2];
        let filename = parts[3];
        let slug = filename.strip_suffix(".md")?;
        Some(ParsedVaultPath {
            owner,
            context,
            doc_type,
            slug,
        })
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p temper-core vault::tests`
Expected: All parse_rel tests pass; previous tests still pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/vault.rs
git commit -m "feat(core): add Vault::parse_rel with round-trip tests"
```

---

### Task 4: Implement `canonical_uri` and `parse_uri`

**Files:**
- Modify: `crates/temper-core/src/vault.rs`

- [ ] **Step 1: Write failing unit tests**

Append to the `tests` module:

```rust
    #[test]
    fn canonical_uri_personal_with_slug() {
        assert_eq!(
            Vault::canonical_uri("@me", "temper", "task", "my-task"),
            "kb://@me/temper/task/my-task".to_string()
        );
    }

    #[test]
    fn canonical_uri_team_with_slug() {
        assert_eq!(
            Vault::canonical_uri("+team-x", "general", "goal", "q4"),
            "kb://+team-x/general/goal/q4".to_string()
        );
    }

    #[test]
    fn canonical_uri_with_uuid_ident() {
        assert_eq!(
            Vault::canonical_uri("@me", "temper", "task", "019d6880-5c21-7bb2-86fb-a0cc612b5cf5"),
            "kb://@me/temper/task/019d6880-5c21-7bb2-86fb-a0cc612b5cf5".to_string()
        );
    }

    #[test]
    fn parse_uri_valid_personal() {
        let parsed = Vault::parse_uri("kb://@me/temper/task/my-task").unwrap();
        assert_eq!(parsed.owner, "@me");
        assert_eq!(parsed.context, "temper");
        assert_eq!(parsed.doc_type, "task");
        assert_eq!(parsed.ident, "my-task");
    }

    #[test]
    fn parse_uri_valid_team() {
        let parsed = Vault::parse_uri("kb://+platform/general/goal/q4").unwrap();
        assert_eq!(parsed.owner, "+platform");
        assert_eq!(parsed.context, "general");
        assert_eq!(parsed.doc_type, "goal");
        assert_eq!(parsed.ident, "q4");
    }

    #[test]
    fn parse_uri_rejects_legacy_no_sigil() {
        assert!(Vault::parse_uri("kb://temper/task/my-task").is_none());
    }

    #[test]
    fn parse_uri_rejects_missing_scheme() {
        assert!(Vault::parse_uri("@me/temper/task/my-task").is_none());
        assert!(Vault::parse_uri("http://@me/temper/task/my-task").is_none());
    }

    #[test]
    fn parse_uri_rejects_too_few_segments() {
        assert!(Vault::parse_uri("kb://@me/temper/task").is_none());
        assert!(Vault::parse_uri("kb://@me/temper").is_none());
        assert!(Vault::parse_uri("kb://@me").is_none());
        assert!(Vault::parse_uri("kb://").is_none());
    }

    #[test]
    fn parse_uri_round_trips_with_canonical_uri() {
        let uri = Vault::canonical_uri("@me", "temper", "task", "round-trip");
        let parsed = Vault::parse_uri(&uri).unwrap();
        assert_eq!(parsed.owner, "@me");
        assert_eq!(parsed.context, "temper");
        assert_eq!(parsed.doc_type, "task");
        assert_eq!(parsed.ident, "round-trip");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p temper-core vault::tests::canonical_uri`
Expected: FAIL with "no function named `canonical_uri`".

- [ ] **Step 3: Implement `canonical_uri` and `parse_uri` as associated functions**

Add to the `impl<'a> Vault<'a>` block after `parse_rel`:

```rust
    // ----- URI operations (pure, no vault_root needed) -----

    /// Build a canonical kb:// URI from components.
    /// Returns `kb://<owner>/<context>/<doc_type>/<ident>`.
    ///
    /// Associated function — no Vault instance needed. API/MCP use this without
    /// touching the filesystem.
    pub fn canonical_uri(owner: &str, context: &str, doc_type: &str, ident: &str) -> String {
        format!("kb://{owner}/{context}/{doc_type}/{ident}")
    }

    /// Parse a kb:// URI into components. Rejects legacy no-sigil URIs.
    ///
    /// Associated function — no Vault instance needed.
    pub fn parse_uri(uri: &str) -> Option<ParsedKbUri<'_>> {
        let rest = uri.strip_prefix("kb://")?;
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() != 4 {
            return None;
        }
        let owner = parts[0];
        if !(owner.starts_with('@') || owner.starts_with('+')) {
            return None;
        }
        Some(ParsedKbUri {
            owner,
            context: parts[1],
            doc_type: parts[2],
            ident: parts[3],
        })
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p temper-core vault::tests`
Expected: All Vault unit tests (filesystem + URI + parse + round-trip) pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/vault.rs
git commit -m "feat(core): add Vault canonical_uri and parse_uri with round-trip tests"
```

---

### Task 5: Cross-implementation parity integration test

**Files:**
- Create: `crates/temper-core/tests/vault_parity.rs`

- [ ] **Step 1: Check whether integration test harness exists in temper-core**

Run: `ls crates/temper-core/tests/ 2>/dev/null`

If the `tests/` directory exists and already has other `#[sqlx::test]` files, follow their setup convention. If it does not exist yet, create it as part of this task and mirror the pattern from `crates/temper-api/tests/` (which does have `#[sqlx::test]` integration tests — examine one as a reference).

- [ ] **Step 2: Write the failing parity test**

Create `crates/temper-core/tests/vault_parity.rs`:

```rust
//! Cross-implementation parity: Vault::canonical_uri in Rust must produce
//! byte-identical output to the SQL kb_resource_uri() function in Postgres
//! for the same inputs.
//!
//! Behind the `test-db` feature since it requires a live database.

#![cfg(feature = "test-db")]

use sqlx::PgPool;
use temper_core::vault::Vault;

#[sqlx::test(migrations = "../../migrations")]
async fn vault_canonical_uri_matches_sql_kb_resource_uri(pool: PgPool) {
    // 1. Create a profile with a known slug.
    let profile_id: uuid::Uuid = sqlx::query_scalar!(
        r#"
        INSERT INTO kb_profiles (id, auth0_sub, email, display_name, slug, created, updated)
        VALUES (gen_random_uuid(), 'auth0|parity-test', 'parity@test.local',
                'Parity Test', 'parity-test', NOW(), NOW())
        RETURNING id
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert profile");

    // 2. Create a context owned by that profile.
    let context_id: uuid::Uuid = sqlx::query_scalar!(
        r#"
        INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id, created, updated)
        VALUES (gen_random_uuid(), 'parity-ctx', 'kb_profiles', $1, NOW(), NOW())
        RETURNING id
        "#,
        profile_id,
    )
    .fetch_one(&pool)
    .await
    .expect("insert context");

    // 3. Look up a doc_type by name (seeded by the migrations).
    let doc_type_id: uuid::Uuid = sqlx::query_scalar!(
        r#"SELECT id FROM kb_doc_types WHERE name = 'task' LIMIT 1"#,
    )
    .fetch_one(&pool)
    .await
    .expect("fetch doc_type");

    // 4. Insert a resource with a known slug.
    let resource_id: uuid::Uuid = sqlx::query_scalar!(
        r#"
        INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, slug, title,
                                   content_hash, created, updated, is_active)
        VALUES (gen_random_uuid(), $1, $2, 'parity-resource', 'Parity Resource',
                'sha256:fake', NOW(), NOW(), TRUE)
        RETURNING id
        "#,
        context_id,
        doc_type_id,
    )
    .fetch_one(&pool)
    .await
    .expect("insert resource");

    // 5. Fetch the SQL-generated URI.
    let sql_uri: String = sqlx::query_scalar!(
        r#"SELECT kb_resource_uri($1) AS "uri!""#,
        resource_id,
    )
    .fetch_one(&pool)
    .await
    .expect("call kb_resource_uri");

    // 6. Compute the same URI in Rust.
    let rust_uri = Vault::canonical_uri("@parity-test", "parity-ctx", "task", "parity-resource");

    // 7. They must match exactly.
    assert_eq!(
        sql_uri, rust_uri,
        "SQL kb_resource_uri and Rust Vault::canonical_uri diverged"
    );
}
```

- [ ] **Step 3: Make sure `test-db` feature is wired for temper-core**

Check `crates/temper-core/Cargo.toml`. If there is no `test-db` feature, add one:

```toml
[features]
web-api = ["utoipa"]
typescript = ["ts-rs"]
mcp = ["schemars"]
test-db = []  # enables database integration tests under tests/
```

And ensure `[dev-dependencies]` includes what the test needs:

```toml
[dev-dependencies]
sqlx = { version = "0.8", features = ["postgres", "runtime-tokio-rustls", "macros", "chrono", "uuid"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
uuid = { version = "1", features = ["v7", "serde"] }
```

(If any of these dev-dependencies are already present, leave them alone; only add what's missing.)

- [ ] **Step 4: Run the parity test**

Run: `cargo nextest run -p temper-core --features test-db vault_canonical_uri_matches_sql_kb_resource_uri`
Expected: PASS. If it fails, the SQL function and Rust helper diverge — fix the divergence (usually in the Rust helper; the SQL function was the Session 3 source of truth for URI shape).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/tests/vault_parity.rs crates/temper-core/Cargo.toml
git commit -m "test(core): add cross-implementation parity test for Vault::canonical_uri"
```

---

## Phase 2 — CLI migration to Vault

### Task 6: Add `subscriptions` + `subscription_for_context` to CLI Config

**Files:**
- Modify: `crates/temper-cli/src/config.rs`

- [ ] **Step 1: Read the current CLI Config struct and the function that builds it**

Read `crates/temper-cli/src/config.rs` in full. Identify:
- The `Config` struct definition (around lines 24-32).
- The constructor / builder function (likely `Config::load()` or `Config::from_global(...)`). Note where `contexts: Vec<String>` is populated from `global.sync.subscriptions`.

- [ ] **Step 2: Add `subscriptions` field to `Config`**

Modify the `Config` struct:

```rust
use temper_core::types::vault_config::Subscription;

/// Resolved runtime configuration built from GlobalConfig.
#[derive(Debug, Clone)]
pub struct Config {
    pub vault_root: PathBuf,
    pub state_dir: PathBuf,
    pub contexts: Vec<String>,
    pub subscriptions: Vec<Subscription>,
    pub skill_output: PathBuf,
    pub skill_framework: String,
}
```

- [ ] **Step 3: Populate the `subscriptions` field in the Config builder**

In the function that builds `Config` from `GlobalConfig` (or equivalent), add:

```rust
let subscriptions = global.sync.subscriptions.clone();
// ...
Config {
    vault_root: ...,
    state_dir: ...,
    contexts,
    subscriptions,
    skill_output: ...,
    skill_framework: ...,
}
```

If `global.sync.subscriptions` does not exist at the path you expect, grep for `Subscription` usage in the codebase to find where they're loaded:

```bash
```

Run: `rg 'Vec<Subscription>' crates/ --type rust`
Expected: A source location that shows where subscriptions are currently accessed. Wire `Config.subscriptions` from that same source.

- [ ] **Step 4: Add `subscription_for_context` method on `Config`**

Add to the `impl Config` block (or create one if absent):

```rust
impl Config {
    /// Look up the subscription for a given context name.
    /// Returns `None` if the context has no subscription configured.
    pub fn subscription_for_context(&self, context: &str) -> Option<&Subscription> {
        self.subscriptions.iter().find(|s| s.context == context)
    }

    /// Resolve the owner string for a given context via its subscription.
    /// Falls back to "@me" if no subscription is configured for the context.
    pub fn owner_for_context(&self, context: &str) -> String {
        self.subscription_for_context(context)
            .map(|s| s.resolved_owner())
            .unwrap_or_else(|| "@me".to_string())
    }
}
```

**Do NOT delete `doc_type_dir` yet** — that happens in Task 13 once all call sites are migrated.

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p temper-cli`
Expected: Clean compile. If a borrow-checker or import error appears, fix it before moving on.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/config.rs
git commit -m "feat(cli): add Config::subscription_for_context and owner_for_context"
```

---

### Task 7: Migrate `actions/task.rs` to Vault

**Files:**
- Modify: `crates/temper-cli/src/actions/task.rs`

- [ ] **Step 1: Add Vault import and baseline run of existing task tests**

At the top of `task.rs`, add the import:

```rust
use temper_core::vault::Vault;
```

Run existing tests first to establish a baseline:

Run: `cargo nextest run -p temper-cli actions::task`
Expected: Passes (green baseline).

- [ ] **Step 2: Migrate `load_tasks`**

Replace the two `config.doc_type_dir(...)` calls inside `load_tasks`. Current (lines ~23-30):

```rust
    let dirs: Vec<_> = if let Some(p) = context {
        let d = config.doc_type_dir(p, "task");
        if d.is_dir() {
            vec![d]
        } else {
            vec![]
        }
    } else {
        // Scan all contexts for task subdirectories
        let mut found = Vec::new();
        for ctx in &config.contexts {
            let d = config.doc_type_dir(ctx, "task");
            if d.is_dir() {
                found.push(d);
            }
        }
        found
    };
```

Replace with:

```rust
    let vault = Vault::new(&config.vault_root);
    let dirs: Vec<_> = if let Some(p) = context {
        let owner = config.owner_for_context(p);
        let d = vault.doc_type_dir(&owner, p, "task");
        if d.is_dir() {
            vec![d]
        } else {
            vec![]
        }
    } else {
        let mut found = Vec::new();
        for ctx in &config.contexts {
            let owner = config.owner_for_context(ctx);
            let d = vault.doc_type_dir(&owner, ctx, "task");
            if d.is_dir() {
                found.push(d);
            }
        }
        found
    };
```

- [ ] **Step 3: Migrate `create` (around line 194)**

Current:

```rust
    let dir = config.doc_type_dir(context, "task");
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    let path = dir.join(format!("{slug}.md"));
```

Replace with:

```rust
    let vault = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(context);
    let dir = vault.doc_type_dir(&owner, context, "task");
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    let path = vault.doc_file(&owner, context, "task", &slug);
```

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -p temper-cli actions::task`
Expected: All existing tests still pass. (If a test expected a legacy path like `<vault>/temper/task/x.md`, it needs updating to `<vault>/@me/temper/task/x.md` — this is expected.)

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/task.rs
git commit -m "refactor(cli): migrate task.rs to Vault layout"
```

---

### Task 8: Migrate `actions/goal.rs` to Vault (including discovery events)

**Files:**
- Modify: `crates/temper-cli/src/actions/goal.rs`

- [ ] **Step 1: Add Vault import and baseline test run**

Add at the top of `goal.rs`:

```rust
use temper_core::vault::Vault;
```

Run: `cargo nextest run -p temper-cli actions::goal`
Expected: Green baseline.

- [ ] **Step 2: Migrate `load_goals`**

Same pattern as `load_tasks`. Replace both `config.doc_type_dir(...)` calls with `vault.doc_type_dir(&owner, ctx, "goal")` after computing `let owner = config.owner_for_context(ctx);`. Add `let vault = Vault::new(&config.vault_root);` at the top of the function.

- [ ] **Step 3: Migrate `ensure_maintenance` path + discovery event**

Current (around line 75):

```rust
    let dir = config.doc_type_dir(context, "goal");
    let path = dir.join(format!("{slug}.md"));
```

Replace with:

```rust
    let vault = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(context);
    let dir = vault.doc_type_dir(&owner, context, "goal");
    let path = vault.doc_file(&owner, context, "goal", &slug);
```

Current discovery event (around line 98):

```rust
    let event = discovery::Event::ResourceCreate {
        ts: Local::now().to_rfc3339(),
        doc_type: "goal".to_string(),
        title: "Maintenance".to_string(),
        path: format!("{context}/goal/{slug}.md"),
        context: context.to_string(),
    };
```

Replace the `path:` line with:

```rust
        path: vault.rel_path(&owner, context, "goal", &slug),
```

- [ ] **Step 4: Migrate `create` discovery event (around line 139)**

Current:

```rust
    let event = discovery::Event::ResourceCreate {
        ts: Local::now().to_rfc3339(),
        doc_type: "goal".to_string(),
        title: title.to_string(),
        path: format!("{context}/goal/{slug}.md"),
        context: context.to_string(),
    };
```

Ensure `vault` and `owner` are in scope at this point in the function (add them near the top if not already added for path construction). Replace the `path:` line with:

```rust
        path: vault.rel_path(&owner, context, "goal", &slug),
```

If there is also a `dir = config.doc_type_dir(context, "goal")` earlier in `create`, migrate it the same way as `ensure_maintenance`.

- [ ] **Step 5: Run tests**

Run: `cargo nextest run -p temper-cli actions::goal`
Expected: All goal tests pass. Update any that hardcoded legacy paths.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/goal.rs
git commit -m "refactor(cli): migrate goal.rs to Vault layout including discovery events"
```

---

### Task 9: Migrate `commands/resource.rs` to Vault

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs`

- [ ] **Step 1: Add Vault import**

```rust
use temper_core::vault::Vault;
```

- [ ] **Step 2: Migrate `create_simple_resource` path construction (around lines 170-172)**

Current:

```rust
    let dir = config.doc_type_dir(context, doc_type);
    let path = dir.join(format!("{slug}.md"));
```

Replace with:

```rust
    let vault = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(context);
    let dir = vault.doc_type_dir(&owner, context, doc_type);
    let path = vault.doc_file(&owner, context, doc_type, &slug);
```

- [ ] **Step 3: The discovery event already uses `relative_str` via `path.strip_prefix(vault_root)`**

Inspect lines 184-190. Since `path` now lives under `<vault>/<owner>/<context>/<doc_type>/<slug>.md`, `strip_prefix(vault_root)` will already produce an owner-scoped string. **No change needed to the discovery event body.** This is a bonus of using absolute paths from `Vault::doc_file`.

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -p temper-cli commands::resource`
Expected: Existing resource tests pass (updating any that hardcoded legacy paths).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "refactor(cli): migrate resource command to Vault layout"
```

---

### Task 10: Migrate `actions/doctor.rs::scan` and delete dead research special case

**Files:**
- Modify: `crates/temper-cli/src/actions/doctor.rs`

- [ ] **Step 1: Add Vault import and baseline**

Add:

```rust
use temper_core::vault::Vault;
```

Run: `cargo nextest run -p temper-cli actions::doctor`
Expected: Green baseline.

- [ ] **Step 2: Migrate the `scan` function and delete the dead research block**

Current (lines 37-67):

```rust
pub fn scan(config: &Config, context_filter: Option<&str>) -> Result<DoctorReport> {
    let mut file_results: Vec<ValidationResult> = Vec::new();

    let contexts_to_scan: Vec<String> = if let Some(ctx) = context_filter {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    // Walk standard entity doc type directories
    for doc_type in ENTITY_DOC_TYPES {
        for ctx in &contexts_to_scan {
            let dir = config.doc_type_dir(ctx, doc_type);
            if !dir.is_dir() {
                continue;
            }
            scan_directory(&dir, doc_type, &mut file_results)?;
        }
    }

    // Walk research directory: {vault_root}/research/{context}/
    let research_root = config.vault_root.join("research");
    if research_root.is_dir() {
        for ctx in &contexts_to_scan {
            let dir = research_root.join(ctx);
            if !dir.is_dir() {
                continue;
            }
            scan_directory(&dir, "research", &mut file_results)?;
        }
    }
```

Replace with:

```rust
pub fn scan(config: &Config, context_filter: Option<&str>) -> Result<DoctorReport> {
    let mut file_results: Vec<ValidationResult> = Vec::new();
    let vault = Vault::new(&config.vault_root);

    let contexts_to_scan: Vec<String> = if let Some(ctx) = context_filter {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    // Walk every entity doc_type directory under its owner-scoped path.
    // research is included in ENTITY_DOC_TYPES — no special case.
    for doc_type in ENTITY_DOC_TYPES {
        for ctx in &contexts_to_scan {
            let owner = config.owner_for_context(ctx);
            let dir = vault.doc_type_dir(&owner, ctx, doc_type);
            if !dir.is_dir() {
                continue;
            }
            scan_directory(&dir, doc_type, &mut file_results)?;
        }
    }
```

The old research-special-case block (lines 57-67) is gone. `research` is already listed in `ENTITY_DOC_TYPES` at line 31 so coverage is preserved.

- [ ] **Step 3: Run tests**

Run: `cargo nextest run -p temper-cli actions::doctor`
Expected: Tests pass. If any test set up a fixture at `<vault>/research/<ctx>/...`, update it to `<vault>/@me/<ctx>/research/...`.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/doctor.rs
git commit -m "refactor(cli): migrate doctor scan to Vault and remove dead research special case"
```

---

### Task 11: Migrate `actions/ingest.rs::infer_context_and_doctype`

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs`

- [ ] **Step 1: Add Vault import**

```rust
use temper_core::vault::Vault;
```

- [ ] **Step 2: Replace `infer_context_and_doctype` body**

Current (lines 554-598):

```rust
pub fn infer_context_and_doctype(
    vault_root: &Path,
    file_path: &Path,
    fm_context: Option<&str>,
    fm_doc_type: Option<&str>,
) -> Result<(String, String)> {
    let rel = file_path.strip_prefix(vault_root).map_err(|_| {
        TemperError::Config(format!(
            "file {} is not inside vault {}",
            file_path.display(),
            vault_root.display()
        ))
    })?;

    let parts: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    let dir_context = parts.first().copied();
    let dir_doc_type = if parts.len() >= 3 {
        Some(parts[1])
    } else {
        None
    };

    let context = fm_context
        .or(dir_context)
        .ok_or_else(|| {
            TemperError::Config(format!("cannot infer context for {}", file_path.display()))
        })?
        .to_string();

    let doc_type = fm_doc_type
        .or(dir_doc_type)
        .ok_or_else(|| {
            TemperError::Config(format!(
            "cannot infer doc_type for {} (file must be at {{context}}/{{doc_type}}/{{slug}}.md)",
            file_path.display()
        ))
        })?
        .to_string();

    Ok((context, doc_type))
}
```

Replace with:

```rust
pub fn infer_context_and_doctype(
    vault_root: &Path,
    file_path: &Path,
    fm_context: Option<&str>,
    fm_doc_type: Option<&str>,
) -> Result<(String, String)> {
    let rel = file_path
        .strip_prefix(vault_root)
        .map_err(|_| {
            TemperError::Config(format!(
                "file {} is not inside vault {}",
                file_path.display(),
                vault_root.display()
            ))
        })?
        .to_string_lossy()
        .to_string();

    let dir_parsed = Vault::parse_rel(&rel);

    let context = fm_context
        .map(|s| s.to_string())
        .or_else(|| dir_parsed.as_ref().map(|p| p.context.to_string()))
        .ok_or_else(|| {
            TemperError::Config(format!("cannot infer context for {}", file_path.display()))
        })?;

    let doc_type = fm_doc_type
        .map(|s| s.to_string())
        .or_else(|| dir_parsed.as_ref().map(|p| p.doc_type.to_string()))
        .ok_or_else(|| {
            TemperError::Config(format!(
            "cannot infer doc_type for {} (file must be at {{owner}}/{{context}}/{{doc_type}}/{{slug}}.md)",
            file_path.display()
        ))
        })?;

    Ok((context, doc_type))
}
```

- [ ] **Step 3: Run tests**

Run: `cargo nextest run -p temper-cli actions::ingest`
Expected: Tests pass. Any fixture files placed at `<vault>/<ctx>/<type>/<slug>.md` without an owner prefix will now fail to parse — update fixtures to use `<vault>/@me/<ctx>/<type>/<slug>.md`.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/ingest.rs
git commit -m "refactor(cli): migrate infer_context_and_doctype to Vault::parse_rel"
```

---

### Task 12: Migrate `actions/sync.rs` (parse_kb_uri, manifest path loops, rel_path computation)

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs`

- [ ] **Step 1: Add Vault import and baseline**

Add:

```rust
use temper_core::vault::Vault;
```

Run: `cargo nextest run -p temper-cli actions::sync`
Expected: Green baseline.

- [ ] **Step 2: Replace `parse_kb_uri` (lines 208-220)**

Current:

```rust
pub fn parse_kb_uri(uri: &str) -> Result<(String, String)> {
    let rest = uri
        .strip_prefix("kb://")
        .ok_or_else(|| TemperError::Config(format!("invalid kb:// URI: {uri}")))?;
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() < 2 {
        return Err(TemperError::Config(format!(
            "kb:// URI must have at least context/doc_type: {uri}"
        )));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}
```

Replace with:

```rust
pub fn parse_kb_uri(uri: &str) -> Result<(String, String)> {
    let parsed = Vault::parse_uri(uri).ok_or_else(|| {
        TemperError::Config(format!(
            "invalid kb:// URI (expected kb://<owner>/<context>/<doc_type>/<ident>): {uri}"
        ))
    })?;
    Ok((parsed.context.to_string(), parsed.doc_type.to_string()))
}
```

- [ ] **Step 3: Replace manifest path parsing in `build_status_request` (lines 108-150)**

Current has two inline `entry.path.split('/')` loops for extracting context and doc_type. Rewrite the loop that iterates `manifest.entries` using the associated function `Vault::parse_rel` (no instance needed):

```rust
pub fn build_status_request(manifest: &Manifest, context_filter: &[String]) -> SyncStatusRequest {
    let mut context_map: std::collections::HashMap<String, Vec<SyncManifestEntry>> =
        std::collections::HashMap::new();

    for (id, entry) in &manifest.entries {
        let Some(parsed) = Vault::parse_rel(&entry.path) else {
            // Malformed manifest entry — skip with a warning.
            tracing::warn!("skipping malformed manifest path: {}", entry.path);
            continue;
        };

        let ctx = parsed.context.to_string();
        let doc_type = parsed.doc_type.to_string();

        if !context_filter.is_empty() && !context_filter.contains(&ctx) {
            continue;
        }

        let uri = Vault::canonical_uri(parsed.owner, &ctx, &doc_type, &id.to_string());

        context_map.entry(ctx).or_default().push(SyncManifestEntry {
            uri,
            local_hash: entry.body_hash.clone(),
            remote_hash: entry.remote_body_hash.clone(),
            managed_hash: entry.managed_hash.clone(),
            remote_managed_hash: entry.remote_managed_hash.clone(),
            open_hash: entry.open_hash.clone(),
            remote_open_hash: entry.remote_open_hash.clone(),
        });
    }

    let contexts = context_map
        .into_iter()
        .map(|(name, entries)| SyncContextEntries { name, entries })
        .collect();

    SyncStatusRequest { contexts }
}
```

- [ ] **Step 4: Replace manifest path parsing in `push_error_context` (lines 521-547)**

Current has another inline `entry.path.split('/')` loop. Rewrite:

```rust
fn push_error_context(manifest: &Manifest, item: &SyncPushItem) -> (String, String, String) {
    let entry = item
        .resource_id
        .and_then(|id| manifest.entries.get(&id))
        .or_else(|| {
            extract_resource_id(&item.uri)
                .ok()
                .and_then(|id| manifest.entries.get(&id))
        });

    if let Some(entry) = entry {
        if let Some(parsed) = Vault::parse_rel(&entry.path) {
            return (
                entry.path.clone(),
                parsed.context.to_string(),
                parsed.doc_type.to_string(),
            );
        }
        return (
            entry.path.clone(),
            "unknown".to_string(),
            "unknown".to_string(),
        );
    }

    (
        item.uri.clone(),
        "unknown".to_string(),
        "unknown".to_string(),
    )
}
```

- [ ] **Step 5: `rel_path` computation during vault scan (around line 263)**

Current:

```rust
        let rel_path = path
            .strip_prefix(vault_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
```

This produces a vault-relative string that already includes the owner segment — after the shell script migration files physically live at `<vault>/@me/<ctx>/<type>/<slug>.md`, so `strip_prefix(vault_root)` naturally produces `@me/<ctx>/<type>/<slug>.md`. No code change is required to the string computation itself.

Add a guard immediately after that skips files whose layout is not owner-scoped (protection against unmigrated vaults or stray files):

```rust
        let rel_path = path
            .strip_prefix(vault_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        if Vault::parse_rel(&rel_path).is_none() {
            tracing::warn!(
                "scanned path is not owner-scoped — the vault may need migration: {rel_path}"
            );
            continue;
        }
```

This skips legacy-layout files during sync scans with a warning rather than failing hard. After the shell script runs, no such files exist in the vault, so the warning should never fire in practice.

- [ ] **Step 6: Run tests**

Run: `cargo nextest run -p temper-cli actions::sync`
Expected: Tests pass. Any test that used manifests with legacy path strings needs updating to the owner-scoped form.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "refactor(cli): migrate sync.rs manifest parsing and parse_kb_uri to Vault"
```

---

### Task 13: Delete `Config::doc_type_dir` and verify all call sites are migrated

**Files:**
- Modify: `crates/temper-cli/src/config.rs`

- [ ] **Step 1: Delete the `doc_type_dir` method from `Config`**

Remove this block:

```rust
    /// Compute the directory for a given context + doc_type.
    /// Returns `vault_root/{context}/{doc_type}/`
    pub fn doc_type_dir(&self, context: &str, doc_type: &str) -> PathBuf {
        self.vault_root.join(context).join(doc_type)
    }
```

- [ ] **Step 2: Compile the whole CLI and use errors as a checklist**

Run: `cargo check -p temper-cli`
Expected: Compile errors at any remaining call site that still uses `config.doc_type_dir(...)`. Fix each one the same way as Tasks 7-12.

If there are zero errors, Tasks 7-12 covered all call sites — proceed.

- [ ] **Step 3: Run the full test suite**

Run: `cargo make check && cargo make test`
Expected: All green. Some tests with hardcoded legacy paths will fail — update those fixtures to include `@me/`.

- [ ] **Step 4: Commit**

```bash
git add -u crates/temper-cli
git commit -m "refactor(cli): delete Config::doc_type_dir, Vault is the sole layout helper"
```

---

### Task 14: Full verification gate for Phase 2

**Files:** (none — verification only)

- [ ] **Step 1: Run the full check suite**

Run: `cargo make check`
Expected: Formatting, clippy, docs, machete, TypeScript all green.

- [ ] **Step 2: Run unit tests**

Run: `cargo make test`
Expected: All unit tests green.

- [ ] **Step 3: Run database integration tests**

Run: `cargo make docker-up && cargo make test-db`
Expected: All integration tests green, including the Task 5 parity test.

- [ ] **Step 4: Spot-check for stray path arithmetic in the CLI**

Run: `rg 'config\.doc_type_dir' crates/temper-cli/src --type rust`
Expected: Zero results. (`config.doc_type_dir` was deleted in Task 13.)

Run: `rg 'format!\("\{.*\}/\{.*\}/\{.*\}\.md' crates/temper-cli/src --type rust`
Expected: Zero results.

Run: `rg 'entry\.path\.split' crates/temper-cli/src --type rust`
Expected: Zero results (both ad-hoc parsers collapsed into `Vault::parse_rel`).

Any hit must be reviewed and migrated to Vault before continuing.

- [ ] **Step 5: Commit (if any fixes were needed)**

If no changes, skip. Otherwise:

```bash
git add -u
git commit -m "refactor(cli): mop up stray path arithmetic after Vault migration"
```

---

## Phase 3 — Shell script + dogfood

### Task 15: Write `migrate-vault-to-owner-segmented.sh`

**Files:**
- Create: `scripts/migrate-vault-to-owner-segmented.sh`

- [ ] **Step 1: Verify `jq` is available**

Run: `which jq`
Expected: A path like `/usr/bin/jq` or `/opt/homebrew/bin/jq`. If absent, install with `brew install jq` before running the script.

- [ ] **Step 2: Create the script**

Create `scripts/migrate-vault-to-owner-segmented.sh`:

```bash
#!/usr/bin/env bash
#
# One-off: migrate a temper vault to the owner-segmented layout.
#
#   Before: <vault>/<context>/<doc_type>/<slug>.md
#   After:  <vault>/@me/<context>/<doc_type>/<slug>.md
#
# Also:
#   - rewrites <vault>/.temper/manifest.json entries[*].path to prepend "@me/"
#   - backfills "temper-owner: \"@me\"" into every file's managed frontmatter
#     that does not already have one
#
# Idempotent: re-running on an already-migrated vault is a no-op.
#
# Usage:
#   ./migrate-vault-to-owner-segmented.sh <vault_path>          # dry-run
#   ./migrate-vault-to-owner-segmented.sh <vault_path> --apply  # actually do it

set -euo pipefail

VAULT="${1:-}"
MODE="${2:-dry-run}"

if [[ -z "$VAULT" ]]; then
    echo "usage: $0 <vault_path> [--apply]" >&2
    exit 1
fi

if [[ ! -d "$VAULT" ]]; then
    echo "error: vault not found: $VAULT" >&2
    exit 1
fi

if [[ "$MODE" != "dry-run" && "$MODE" != "--apply" ]]; then
    echo "error: second arg must be omitted (dry-run) or --apply" >&2
    exit 1
fi

DRY=1
if [[ "$MODE" == "--apply" ]]; then
    DRY=0
fi

say() { echo "[migrate-vault] $*"; }
do_cmd() {
    if [[ "$DRY" -eq 1 ]]; then
        echo "  DRY-RUN: $*"
    else
        echo "  $*"
        eval "$@"
    fi
}

say "vault: $VAULT"
say "mode:  $( [[ $DRY -eq 1 ]] && echo dry-run || echo APPLY )"
say ""

# 1. Move top-level context directories into @me/
say "==> Step 1: relocate context directories under @me/"
mkdir -p "$VAULT/@me" 2>/dev/null || true
moved=0
for dir in "$VAULT"/*; do
    [[ -d "$dir" ]] || continue
    name=$(basename "$dir")
    case "$name" in
        "@me"|"@"*|"+"*|".temper"|".git")
            continue  # already migrated or special
            ;;
    esac
    do_cmd "mv \"$dir\" \"$VAULT/@me/$name\""
    moved=$((moved+1))
done
say "  moved $moved context directories"
say ""

# 2. Rewrite manifest.json paths (prepend @me/)
MANIFEST="$VAULT/.temper/manifest.json"
if [[ -f "$MANIFEST" ]]; then
    say "==> Step 2: rewrite manifest paths"
    BAK="$MANIFEST.pre-migration-bak"
    if [[ "$DRY" -eq 0 ]]; then
        cp "$MANIFEST" "$BAK"
        say "  backup: $BAK"
    fi

    # Count paths that need rewriting (do not already start with @ or +)
    need=$(jq '[.entries[]? | .path | select(startswith("@") | not) | select(startswith("+") | not)] | length' "$MANIFEST")
    say "  manifest entries to rewrite: $need"

    if [[ "$need" -gt 0 ]]; then
        if [[ "$DRY" -eq 0 ]]; then
            tmp=$(mktemp)
            jq '(.entries[]? | .path) |= (if (startswith("@") or startswith("+")) then . else "@me/" + . end)' "$MANIFEST" > "$tmp"
            mv "$tmp" "$MANIFEST"
            say "  manifest rewritten"
        else
            say "  DRY-RUN: would rewrite $need entries"
        fi
    fi
else
    say "==> Step 2: no manifest.json — skipping"
fi
say ""

# 3. Backfill temper-owner: "@me" into frontmatter of every file under @me/
say "==> Step 3: backfill temper-owner frontmatter"
backfilled=0
if [[ -d "$VAULT/@me" ]]; then
    while IFS= read -r -d '' file; do
        # Skip files that already have a temper-owner field
        if head -30 "$file" | grep -q '^temper-owner:'; then
            continue
        fi
        # Check the file has frontmatter at all (starts with ---)
        if ! head -1 "$file" | grep -q '^---$'; then
            continue
        fi
        if [[ "$DRY" -eq 1 ]]; then
            echo "  DRY-RUN: would backfill temper-owner in $file"
        else
            # Insert "temper-owner: \"@me\"" after the first --- line
            awk '
                BEGIN { inserted=0 }
                /^---$/ && !inserted && NR > 1 { print "temper-owner: \"@me\""; inserted=1 }
                { print }
                END { if (!inserted) exit 1 }
            ' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
        fi
        backfilled=$((backfilled+1))
    done < <(find "$VAULT/@me" -type f -name '*.md' -print0)
fi
say "  backfilled temper-owner in $backfilled files"
say ""

say "==> Done."
if [[ "$DRY" -eq 1 ]]; then
    say "This was a dry run. Re-run with --apply to execute."
fi
```

- [ ] **Step 3: Make the script executable**

Run: `chmod +x scripts/migrate-vault-to-owner-segmented.sh`

- [ ] **Step 4: Commit**

```bash
git add scripts/migrate-vault-to-owner-segmented.sh
git commit -m "chore: one-off vault migration script for owner-segmented layout"
```

- [ ] **Step 5: Test on a throwaway vault copy (user-driven, not plan-driven)**

```bash
cp -R /Users/petetaylor/projects/kb-vault /tmp/kb-vault-scratch
./scripts/migrate-vault-to-owner-segmented.sh /tmp/kb-vault-scratch           # dry-run
./scripts/migrate-vault-to-owner-segmented.sh /tmp/kb-vault-scratch --apply
ls /tmp/kb-vault-scratch  # should show @me/, .temper/, README.md
head -5 /tmp/kb-vault-scratch/@me/temper/task/*.md | head  # should show temper-owner: "@me"
rm -rf /tmp/kb-vault-scratch
```

Only proceed to Task 16 after the dry-run output looks correct and the applied scratch copy works when pointed to with a temporary temper config.

---

## Phase 4 — Doctor + sync ownership validation

### Task 16: Add `temper-owner` validation rules to doctor

**Files:**
- Modify: `crates/temper-cli/src/actions/doctor.rs`

- [ ] **Step 1: Add helper to extract `temper-owner` from frontmatter**

At the top of `doctor.rs` (after existing imports), add:

```rust
use temper_core::vault::Vault;
```

Then add a helper function above `scan_file`:

```rust
/// Extract the `temper-owner` value from parsed frontmatter, if present.
fn extract_temper_owner(frontmatter: &serde_yaml::Value) -> Option<String> {
    frontmatter
        .get("temper-owner")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Regex-free pattern check for owner sigils: `^[@+][a-z0-9][a-z0-9-]*$`.
fn is_valid_owner_pattern(value: &str) -> bool {
    let mut chars = value.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if first != '@' && first != '+' {
        return false;
    }
    let second = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !second.is_ascii_lowercase() && !second.is_ascii_digit() {
        return false;
    }
    for c in chars {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return false;
        }
    }
    true
}
```

- [ ] **Step 2: Extend `scan_file` to emit `temper-owner` issues**

Modify `scan_file` to accept additional context: the manifest (to determine provisional vs synced) and the file's vault-relative path (to check directory alignment). Signature becomes:

```rust
fn scan_file(
    file_path: &Path,
    dir_doc_type: &str,
    manifest_owner: Option<&str>,    // None = file not in manifest (provisional)
    is_provisional: bool,             // true even if in manifest if flagged provisional
) -> Result<ValidationResult> {
```

All call sites of `scan_file` in `scan_directory` must be updated to pass the manifest owner and provisional flag — see next step.

Inside the function, after the existing Legacy/Schema/Unknown checks and before returning, add (note: `frontmatter` is already in scope from the existing `let Some(ref frontmatter) = fm else { return ... }` early-return at the top of the function):

```rust
    // 5. temper-owner validation
    {
        let owner_opt = extract_temper_owner(frontmatter);
        match owner_opt {
            None => {
                if is_provisional || manifest_owner.is_none() {
                    issues.push(ValidationIssue {
                        path: "temper-owner".to_string(),
                        message: "missing temper-owner (will default to @me on next sync)".to_string(),
                        auto_fixable: true,
                    });
                } else {
                    issues.push(ValidationIssue {
                        path: "temper-owner".to_string(),
                        message: "missing temper-owner on a synced file — run `temper sync run` to reconcile from server".to_string(),
                        auto_fixable: false,
                    });
                }
            }
            Some(ref value) if !is_valid_owner_pattern(value) => {
                issues.push(ValidationIssue {
                    path: "temper-owner".to_string(),
                    message: format!("invalid temper-owner pattern: {value} (expected @<slug> or +<slug>)"),
                    auto_fixable: false,
                });
            }
            Some(ref value) => {
                if let Some(expected) = manifest_owner {
                    if value != expected {
                        issues.push(ValidationIssue {
                            path: "temper-owner".to_string(),
                            message: format!(
                                "temper-owner ({value}) disagrees with manifest ({expected}) — ownership transfers require an explicit server action"
                            ),
                            auto_fixable: false,
                        });
                    }
                }
            }
        }
    } // end temper-owner validation block
```

- [ ] **Step 3: Update `scan_directory` and `scan` to thread manifest owner through**

`scan_directory` needs access to the manifest. Change its signature:

```rust
fn scan_directory(
    dir: &Path,
    dir_doc_type: &str,
    results: &mut Vec<ValidationResult>,
    vault_root: &Path,
    manifest: Option<&temper_core::types::Manifest>,
) -> Result<()> {
```

Inside the loop, for each file:

```rust
    for file_path in md_files {
        let rel = file_path
            .strip_prefix(vault_root)
            .unwrap_or(&file_path)
            .to_string_lossy()
            .to_string();

        // Look up manifest entry by matching on path.
        let manifest_entry = manifest.and_then(|m| {
            m.entries.values().find(|e| e.path == rel)
        });

        let (manifest_owner, is_provisional) = match manifest_entry {
            Some(entry) => {
                let owner = Vault::parse_rel(&entry.path).map(|p| p.owner.to_string());
                (owner, entry.provisional)
            }
            None => (None, true),
        };

        let result = scan_file(
            &file_path,
            dir_doc_type,
            manifest_owner.as_deref(),
            is_provisional,
        )?;
        results.push(result);
    }
```

Update `scan` to load the manifest once and pass it through:

```rust
pub fn scan(config: &Config, context_filter: Option<&str>) -> Result<DoctorReport> {
    let mut file_results: Vec<ValidationResult> = Vec::new();
    let vault = Vault::new(&config.vault_root);
    let manifest = crate::manifest_io::load_or_default(&config.state_dir).ok();

    let contexts_to_scan: Vec<String> = if let Some(ctx) = context_filter {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    for doc_type in ENTITY_DOC_TYPES {
        for ctx in &contexts_to_scan {
            let owner = config.owner_for_context(ctx);
            let dir = vault.doc_type_dir(&owner, ctx, doc_type);
            if !dir.is_dir() {
                continue;
            }
            scan_directory(
                &dir,
                doc_type,
                &mut file_results,
                &config.vault_root,
                manifest.as_ref(),
            )?;
        }
    }

    // (rest of function unchanged)
```

(If `manifest_io::load_or_default` doesn't exist under that name, use whatever loader pattern the existing code uses — grep for `Manifest::load` or `load_manifest` in `temper-cli/src/` to find it.)

- [ ] **Step 4: Write tests**

Add to `crates/temper-cli/src/actions/doctor.rs` at the bottom (or a sibling tests module):

```rust
#[cfg(test)]
mod owner_validation_tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_fixture(dir: &Path, rel: &str, frontmatter: &str) -> PathBuf {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let content = format!("---\n{frontmatter}\n---\n\n# body\n");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn missing_temper_owner_on_provisional_is_auto_fixable() {
        let tmp = TempDir::new().unwrap();
        let file = write_fixture(
            tmp.path(),
            "@me/temper/task/p.md",
            "temper-type: task\ntitle: p\nslug: p",
        );
        let result = scan_file(&file, "task", None, true).unwrap();
        let owner_issue = result
            .issues
            .iter()
            .find(|i| i.path == "temper-owner")
            .unwrap();
        assert!(owner_issue.auto_fixable);
        assert!(owner_issue.message.contains("default to @me"));
    }

    #[test]
    fn missing_temper_owner_on_synced_is_warning_not_fixable() {
        let tmp = TempDir::new().unwrap();
        let file = write_fixture(
            tmp.path(),
            "@me/temper/task/s.md",
            "temper-type: task\ntitle: s\nslug: s",
        );
        let result = scan_file(&file, "task", Some("@me"), false).unwrap();
        let owner_issue = result
            .issues
            .iter()
            .find(|i| i.path == "temper-owner")
            .unwrap();
        assert!(!owner_issue.auto_fixable);
        assert!(owner_issue.message.contains("run `temper sync run`"));
    }

    #[test]
    fn invalid_temper_owner_pattern_is_error() {
        let tmp = TempDir::new().unwrap();
        let file = write_fixture(
            tmp.path(),
            "@me/temper/task/e.md",
            "temper-type: task\ntitle: e\nslug: e\ntemper-owner: \"not-a-sigil\"",
        );
        let result = scan_file(&file, "task", Some("@me"), false).unwrap();
        let owner_issue = result
            .issues
            .iter()
            .find(|i| i.path == "temper-owner")
            .unwrap();
        assert!(!owner_issue.auto_fixable);
        assert!(owner_issue.message.contains("invalid temper-owner pattern"));
    }

    #[test]
    fn directory_mismatch_warns_never_fixes() {
        let tmp = TempDir::new().unwrap();
        let file = write_fixture(
            tmp.path(),
            "@me/temper/task/m.md",
            "temper-type: task\ntitle: m\nslug: m\ntemper-owner: \"+team\"",
        );
        let result = scan_file(&file, "task", Some("@me"), false).unwrap();
        let owner_issue = result
            .issues
            .iter()
            .find(|i| i.path == "temper-owner")
            .unwrap();
        assert!(!owner_issue.auto_fixable);
        assert!(owner_issue.message.contains("disagrees with manifest"));
    }

    #[test]
    fn valid_temper_owner_matching_manifest_emits_no_issue() {
        let tmp = TempDir::new().unwrap();
        let file = write_fixture(
            tmp.path(),
            "@me/temper/task/v.md",
            "temper-type: task\ntitle: v\nslug: v\ntemper-owner: \"@me\"",
        );
        let result = scan_file(&file, "task", Some("@me"), false).unwrap();
        let owner_issues: Vec<_> = result
            .issues
            .iter()
            .filter(|i| i.path == "temper-owner")
            .collect();
        assert!(owner_issues.is_empty(), "got {:?}", owner_issues);
    }
}
```

- [ ] **Step 5: Run the new tests**

Run: `cargo nextest run -p temper-cli actions::doctor::owner_validation_tests`
Expected: All 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/doctor.rs
git commit -m "feat(cli): temper-owner validation in doctor (presence, pattern, directory)"
```

---

### Task 17: Add `FixAction::SetOwnerField` + `owner_backfilled` counter

**Files:**
- Modify: `crates/temper-cli/src/actions/doctor_fix.rs`
- Modify: `crates/temper-cli/src/commands/doctor.rs`

- [ ] **Step 1: Add the FixAction variant**

In `doctor_fix.rs`, extend the `FixAction` enum:

```rust
pub enum FixAction {
    // ... existing variants ...
    /// Set `temper-owner: "@me"` in a file whose frontmatter is missing it.
    /// Only emitted for provisional/unsynced files.
    SetOwnerField {
        path: PathBuf,
        value: String,  // always "@me" today; parameterized for future flexibility
    },
}
```

- [ ] **Step 2: Add the `owner_backfilled` counter to `ApplyReport`**

In `doctor_fix.rs`, extend `ApplyReport`:

```rust
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ApplyReport {
    pub fields_renamed: u32,
    pub fields_set: u32,
    pub files_renamed: u32,
    pub files_relocated: u32,
    pub manifest_updated: u32,
    pub manifest_removed: u32,
    pub owner_backfilled: u32,
}
```

- [ ] **Step 3: Handle the new variant in `apply_plan` and in the phase counter update**

Find the match arms in `apply_plan` that dispatch on `FixAction` variants. Add:

```rust
            FixAction::SetOwnerField { path, value } => {
                if !dry_run {
                    vault::set_frontmatter_field(path, "temper-owner", value)?;
                }
                report.owner_backfilled += 1;
            }
```

(If `vault::set_frontmatter_field` does not exist under that name, reuse whatever helper the existing `SetField` variant calls. The `SetField` variant at the top of the enum already takes `key: String, value: String` and writes to frontmatter — mirror its implementation.)

And the phase assignment for `SetOwnerField`:

```rust
impl FixAction {
    fn phase(&self) -> u8 {
        match self {
            FixAction::RenameField { .. } | FixAction::SetField { .. } | FixAction::SetOwnerField { .. } => 0,
            FixAction::RenameFile { .. } | FixAction::RelocateFile { .. } => 1,
            FixAction::UpdateManifest { .. } | FixAction::RemoveManifest { .. } => 2,
        }
    }
}
```

- [ ] **Step 4: Emit the new variant from the scan-to-plan step**

Find where `scan_file` issues are converted into `FixAction`s in `doctor_fix.rs`. For each `ValidationIssue` with `path == "temper-owner"` and `auto_fixable == true`, push:

```rust
                FixAction::SetOwnerField {
                    path: file_path.clone(),
                    value: "@me".to_string(),
                }
```

(The existing code has a `build_plan` or similar function that does this conversion; look for it by searching for `ValidationIssue` in `doctor_fix.rs`.)

- [ ] **Step 5: Update the CLI output summary (commands/doctor.rs)**

In `crates/temper-cli/src/commands/doctor.rs`, the `run_fix` function prints a summary with counters. Find the existing summary print (around line 40):

```rust
    output::success(format!(
        "Fixed: {} field renames, {} fields set, {} file renames, {} relocations",
        report.fields_renamed, report.fields_set, report.files_renamed, report.files_relocated
    ));
```

Replace with:

```rust
    output::success(format!(
        "Fixed: {} field renames, {} fields set, {} file renames, {} relocations, {} owner backfills",
        report.fields_renamed, report.fields_set, report.files_renamed, report.files_relocated, report.owner_backfilled
    ));
```

And the corresponding dry-run message:

```rust
    let total = report.fields_renamed
        + report.fields_set
        + report.files_renamed
        + report.files_relocated
        + report.manifest_updated
        + report.manifest_removed
        + report.owner_backfilled;
```

- [ ] **Step 6: Write a test for the new fix action**

Add to the tests module in `doctor_fix.rs`:

```rust
    #[test]
    fn set_owner_field_is_phase_0() {
        let action = FixAction::SetOwnerField {
            path: PathBuf::from("/tmp/test.md"),
            value: "@me".to_string(),
        };
        assert_eq!(action.phase(), 0);
    }

    #[test]
    fn apply_plan_backfills_owner_field() {
        // Use the existing tempfile pattern from other tests in this module.
        let tmp = tempfile::TempDir::new().unwrap();
        let file = tmp.path().join("test.md");
        std::fs::write(
            &file,
            "---\ntemper-type: task\ntitle: t\n---\n\nbody\n",
        )
        .unwrap();

        let mut plan = FixPlan::default();
        plan.add(FixAction::SetOwnerField {
            path: file.clone(),
            value: "@me".to_string(),
        });

        let report = apply_plan(&plan, false).unwrap();
        assert_eq!(report.owner_backfilled, 1);

        let content = std::fs::read_to_string(&file).unwrap();
        assert!(content.contains("temper-owner: \"@me\""));
    }
```

- [ ] **Step 7: Run the tests**

Run: `cargo nextest run -p temper-cli actions::doctor_fix`
Expected: All tests pass, including the two new ones.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/actions/doctor_fix.rs crates/temper-cli/src/commands/doctor.rs
git commit -m "feat(cli): SetOwnerField fix action with backfill counter"
```

---

### Task 18: Add `sync::preflight_ownership_check` and wire into `sync::run` / `sync::status`

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs`

- [ ] **Step 1: Add the mismatch struct and preflight function**

In `sync.rs`, after the imports and before `build_status_request`:

```rust
/// An entry whose frontmatter `temper-owner` disagrees with the owner segment
/// of its manifest path. Skipped from upload until resolved.
#[derive(Debug, Clone)]
pub struct OwnershipMismatch {
    pub file_path: String,
    pub frontmatter_owner: String,
    pub manifest_owner: String,
}

/// Validate every non-provisional manifest entry: the file's frontmatter
/// `temper-owner` must match the owner segment of its manifest path.
/// Returns a list of mismatches to skip from the upload set.
pub fn preflight_ownership_check(
    manifest: &Manifest,
    vault_root: &Path,
) -> Vec<OwnershipMismatch> {
    let mut mismatches = Vec::new();

    for entry in manifest.entries.values() {
        if entry.provisional {
            continue; // Rule 2: provisional entries are the source of truth for their own owner
        }

        let Some(parsed) = Vault::parse_rel(&entry.path) else {
            // Malformed manifest path — surfaced elsewhere as a warning
            continue;
        };
        let manifest_owner = parsed.owner.to_string();

        let abs_path = vault_root.join(&entry.path);
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(_) => continue, // file missing — handled by the existing missing-file detection
        };
        let Some(fm) = crate::vault::parse_frontmatter(&content) else {
            continue; // no frontmatter — handled by doctor
        };
        let frontmatter_owner = fm
            .get("temper-owner")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "@me".to_string());

        if frontmatter_owner != manifest_owner {
            mismatches.push(OwnershipMismatch {
                file_path: entry.path.clone(),
                frontmatter_owner,
                manifest_owner,
            });
        }
    }

    mismatches
}
```

- [ ] **Step 2: Wire into `sync::run`**

Find the `run` function in `sync.rs`. Near the top, after the manifest is loaded and before the upload loop, add:

```rust
    let ownership_mismatches = preflight_ownership_check(&manifest, &config.vault_root);
    if !ownership_mismatches.is_empty() {
        output::warning(format!(
            "{} file(s) have ownership mismatches and will be skipped:",
            ownership_mismatches.len()
        ));
        for m in &ownership_mismatches {
            output::warning(format!(
                "  {} — frontmatter: {}, manifest: {}",
                m.file_path, m.frontmatter_owner, m.manifest_owner
            ));
        }
        output::hint(
            "Ownership transfers require an explicit server action (not yet implemented). \
             Revert the frontmatter edit or wait for `temper team transfer`.",
        );
    }

    let mismatch_paths: std::collections::HashSet<String> = ownership_mismatches
        .iter()
        .map(|m| m.file_path.clone())
        .collect();
```

Then, in the loop that builds the upload set, skip any entry whose `entry.path` is in `mismatch_paths`:

```rust
    for (id, entry) in &manifest.entries {
        if mismatch_paths.contains(&entry.path) {
            continue;
        }
        // ... existing push logic
    }
```

(The exact loop location depends on sync.rs internals — look for the code that builds `push_items` or equivalent.)

- [ ] **Step 3: Wire into `sync::status`**

Find the `status` function (or `run_status`). Add a call to `preflight_ownership_check` and print mismatches in a new section before the existing status summary:

```rust
    let ownership_mismatches = preflight_ownership_check(&manifest, &config.vault_root);
    if !ownership_mismatches.is_empty() {
        output::header("Ownership Mismatches");
        for m in &ownership_mismatches {
            output::warning(format!(
                "  {} — frontmatter: {}, manifest: {}",
                m.file_path, m.frontmatter_owner, m.manifest_owner
            ));
        }
        output::blank();
    }
```

- [ ] **Step 4: Write unit tests for `preflight_ownership_check`**

Add to the existing tests module in `sync.rs`:

```rust
    #[test]
    fn preflight_detects_synced_owner_drift() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = tmp.path();

        // Create a file under @me/... with frontmatter claiming +team ownership.
        let file_dir = vault.join("@me").join("temper").join("task");
        std::fs::create_dir_all(&file_dir).unwrap();
        std::fs::write(
            file_dir.join("drifted.md"),
            "---\ntemper-type: task\ntemper-owner: \"+team\"\ntitle: d\nslug: d\n---\n\nbody\n",
        )
        .unwrap();

        let mut manifest = Manifest::new("dev".to_string());
        let id = ResourceId::from(Uuid::now_v7());
        manifest.entries.insert(
            id,
            temper_core::types::ManifestEntry {
                path: "@me/temper/task/drifted.md".to_string(),
                body_hash: "h".to_string(),
                remote_body_hash: "h".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: chrono::Utc::now(),
                state: temper_core::types::ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );

        let mismatches = preflight_ownership_check(&manifest, vault);
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].frontmatter_owner, "+team");
        assert_eq!(mismatches[0].manifest_owner, "@me");
    }

    #[test]
    fn preflight_ignores_provisional_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = tmp.path();

        let file_dir = vault.join("@me").join("temper").join("task");
        std::fs::create_dir_all(&file_dir).unwrap();
        std::fs::write(
            file_dir.join("new.md"),
            "---\ntemper-type: task\ntemper-owner: \"+different\"\ntitle: n\nslug: n\n---\n\nbody\n",
        )
        .unwrap();

        let mut manifest = Manifest::new("dev".to_string());
        let id = ResourceId::from(Uuid::now_v7());
        manifest.entries.insert(
            id,
            temper_core::types::ManifestEntry {
                path: "@me/temper/task/new.md".to_string(),
                body_hash: "h".to_string(),
                remote_body_hash: String::new(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: chrono::Utc::now(),
                state: temper_core::types::ManifestEntryState::Pending,
                mtime_secs: None,
                last_audit_id: None,
                provisional: true,
            },
        );

        let mismatches = preflight_ownership_check(&manifest, vault);
        assert!(mismatches.is_empty(), "provisional entries should be ignored");
    }

    #[test]
    fn preflight_clean_manifest_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = tmp.path();

        let file_dir = vault.join("@me").join("temper").join("task");
        std::fs::create_dir_all(&file_dir).unwrap();
        std::fs::write(
            file_dir.join("clean.md"),
            "---\ntemper-type: task\ntemper-owner: \"@me\"\ntitle: c\nslug: c\n---\n\nbody\n",
        )
        .unwrap();

        let mut manifest = Manifest::new("dev".to_string());
        let id = ResourceId::from(Uuid::now_v7());
        manifest.entries.insert(
            id,
            temper_core::types::ManifestEntry {
                path: "@me/temper/task/clean.md".to_string(),
                body_hash: "h".to_string(),
                remote_body_hash: "h".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: chrono::Utc::now(),
                state: temper_core::types::ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );

        let mismatches = preflight_ownership_check(&manifest, vault);
        assert!(mismatches.is_empty());
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo nextest run -p temper-cli actions::sync::tests`
Expected: All three new preflight tests pass plus existing sync tests.

- [ ] **Step 6: Full Phase 4 verification**

Run: `cargo make check && cargo make test`
Expected: All green.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "feat(cli): sync preflight ownership check with skip-and-warn behavior"
```

---

## Phase 5 — Strip legacy fallback

### Task 19: SQL migration dropping legacy fallback from `resource_for_uri`

**Files:**
- Create: `migrations/20260408000001_resource_for_uri_drop_legacy.sql`

- [ ] **Step 1: Read the Session 3 migration to copy the function signature exactly**

Run: `cat migrations/20260407000002_owner_scoped_uris.sql`

Find the `resource_for_uri` function definition in that file. Copy the CREATE OR REPLACE statement into the new migration, then edit the body to remove the no-sigil branch.

- [ ] **Step 2: Create the new migration**

Create `migrations/20260408000001_resource_for_uri_drop_legacy.sql` with this content, adjusting the body to match Session 3's return columns exactly:

```sql
-- Drop the legacy no-sigil URI branch from resource_for_uri.
-- After this migration, only kb://@owner/<ctx>/<type>/<ident> and
-- kb://+team/<ctx>/<type>/<ident> URIs resolve. Legacy URIs without a sigil
-- return an empty result (clients must upgrade).

CREATE OR REPLACE FUNCTION resource_for_uri(p_profile_id UUID, p_kb_uri TEXT)
RETURNS TABLE (
    resource_id UUID,
    origin_uri TEXT,
    content_hash VARCHAR(64),
    updated TIMESTAMPTZ,
    is_active BOOLEAN,
    access_level VARCHAR(32),
    team_role team_role
)
LANGUAGE plpgsql STABLE AS $$
DECLARE
    parts TEXT[];
    ctx_name TEXT;
    dtype_name TEXT;
    ident TEXT;
    resolved_id UUID;
BEGIN
    parts := string_to_array(replace(p_kb_uri, 'kb://', ''), '/');

    -- Require owner sigil on first segment.
    IF array_length(parts, 1) IS NULL
       OR (parts[1] NOT LIKE '@%' AND parts[1] NOT LIKE '+%') THEN
        RETURN;  -- Empty result for legacy URIs.
    END IF;

    ctx_name   := parts[2];
    dtype_name := parts[3];
    ident      := parts[4];

    BEGIN
        resolved_id := ident::UUID;
    EXCEPTION WHEN invalid_text_representation THEN
        SELECT r.id INTO resolved_id
          FROM kb_resources r
          JOIN kb_contexts c ON c.id = r.kb_context_id
          JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
         WHERE c.name = ctx_name
           AND dt.name = dtype_name
           AND r.slug = ident
         LIMIT 1;
    END;

    RETURN QUERY
    SELECT r.id,
           r.origin_uri,
           rm.body_hash AS content_hash,
           r.updated,
           r.is_active,
           v.access_level,
           v.team_role
      FROM kb_resources r
      JOIN kb_resource_manifests rm ON rm.kb_resource_id = r.id
      JOIN resources_visible_to(p_profile_id, NULL, ARRAY[resolved_id]) v
        ON v.resource_id = r.id
     WHERE r.id = resolved_id;
END;
$$;
```

**Important:** verify the return column list, the `resources_visible_to` call, and the `kb_resource_manifests` join match exactly what Session 3 committed. Diff your new migration's body against the Session 3 function to confirm the ONLY difference is the removal of the `ELSE` branch that handled legacy no-sigil URIs.

- [ ] **Step 3: Apply the migration locally**

Run: `cargo make docker-up && sqlx migrate run --source migrations`
Expected: The new migration applies cleanly on top of the existing schema.

- [ ] **Step 4: Regenerate the sqlx query cache**

Run: `cargo sqlx prepare --workspace -- --all-features`
Expected: `.sqlx/` files updated. Review the diff — only queries touching `resource_for_uri` should change (if any — the function signature is stable so the cache may be untouched).

- [ ] **Step 5: Write an integration test verifying legacy URIs return empty**

First locate the Session 3 test file:

Run: `rg --files-with-matches 'resource_for_uri' crates/temper-api/tests/ crates/temper-core/tests/`
Expected: One or more test files. Pick the one that already tests `resource_for_uri` URI resolution.

Open that file and identify its fixture-setup pattern. The existing Session 3 tests that verify `resource_for_uri_resolves_owner_scoped_uri` and `resource_for_uri_resolves_slug_identifier` will show the canonical way to insert a profile + context + resource. Add a new `#[sqlx::test]` immediately after them that **reuses the same setup code** verbatim — do not rewrite the fixture; copy-paste its body:

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn resource_for_uri_rejects_legacy_no_sigil_uri(pool: PgPool) {
    // Copy the fixture setup from the sibling test
    // `resource_for_uri_resolves_owner_scoped_uri` verbatim: profile insert,
    // context insert, doc_type lookup, resource insert. Keep the same bindings
    // (profile_id, context_id, resource_id) so the rest of this test compiles.

    // -- BEGIN fixture copy (replace with actual fixture from sibling test) --
    // let profile_id = ...;
    // let context_id = ...;
    // let doc_type_id = ...;
    // let resource_id = ...;
    // -- END fixture copy --

    // Legacy-format URI with no owner sigil — the shape Session 3's ELSE branch
    // used to accept via profile-inference fallback.
    let legacy_uri = "kb://test-ctx/task/test-slug".to_string();

    let rows = sqlx::query!(
        r#"SELECT resource_id FROM resource_for_uri($1, $2)"#,
        profile_id,
        legacy_uri,
    )
    .fetch_all(&pool)
    .await
    .expect("call resource_for_uri");

    assert!(
        rows.is_empty(),
        "legacy no-sigil URIs must return empty after drop-legacy migration"
    );
}
```

The fixture-copy comment block is explicit: the engineer working this task must paste the actual fixture code from the sibling test into the marked region. This avoids drifting the fixture while staying DRY in the plan document.

- [ ] **Step 6: Run the integration tests**

Run: `cargo make test-db`
Expected: All integration tests green, including:
- Existing Session 3 tests for owner-scoped URI resolution (still pass)
- Existing Session 3 tests for slug resolution (still pass)
- New legacy-rejection test (passes)

- [ ] **Step 7: Commit**

```bash
git add migrations/20260408000001_resource_for_uri_drop_legacy.sql .sqlx/ crates/temper-api/tests/
git commit -m "feat(db): drop legacy no-sigil fallback from resource_for_uri"
```

---

## Phase 6 — Merge and deploy

### Task 20: Final verification and PR preparation

**Files:** (none — verification and git only)

- [ ] **Step 1: Full clean-state verification**

Run: `cargo make check && cargo make test && cargo make test-db`
Expected: All green across the workspace.

- [ ] **Step 2: Spot-check for Vault-layout discipline**

Run: `rg 'config\.doc_type_dir' crates/ --type rust`
Expected: Zero results anywhere in the workspace.

Run: `rg 'fn doc_type_dir' crates/ --type rust`
Expected: Only the single definition in `crates/temper-core/src/vault.rs`.

Run: `rg 'format!\("\{.*\}/\{.*\}/\{.*\}\.md' crates/temper-cli --type rust`
Expected: Zero results.

Run: `rg 'entry\.path\.split' crates/temper-cli --type rust`
Expected: Zero results.

- [ ] **Step 3: Manual smoke test on the migrated laptop vault**

User-driven:

```bash
./target/debug/temper resource list --context temper
./target/debug/temper resource show <some-slug> --type task
./target/debug/temper sync status
./target/debug/temper sync run
./target/debug/temper search "vault layout"
./target/debug/temper doctor
```

Expected: All commands succeed against the migrated `<vault>/@me/...` layout. Doctor reports zero issues on a clean migrated vault.

- [ ] **Step 4: Smoke test ownership mismatch detection**

User-driven:

```bash
# Introduce a deliberate mismatch
sed -i.bak 's/temper-owner: "@me"/temper-owner: "+fake"/' /Users/petetaylor/projects/kb-vault/@me/temper/task/<some-file>.md

./target/debug/temper doctor
# Expected: yellow warning about temper-owner directory mismatch, not auto-fixable

./target/debug/temper sync status
# Expected: Ownership Mismatches section lists the file

./target/debug/temper sync run
# Expected: runs sync for clean entries, skips the mismatched file with a warning

# Revert
mv /Users/petetaylor/projects/kb-vault/@me/temper/task/<some-file>.md.bak \
   /Users/petetaylor/projects/kb-vault/@me/temper/task/<some-file>.md
```

- [ ] **Step 5: Open PR**

Push branch if not already pushed:

```bash
git push -u origin jct/temper-system-access-gate
```

Create PR via:

```bash
gh pr create --title "feat: owner-scoped URIs Phase 2 CLI + Vault" --body "$(cat <<'EOF'
## Summary
- Introduces `Vault` abstraction in temper-core centralizing filesystem layout and `kb://` URI construction
- Migrates all CLI call sites to consume `Vault`; removes ~5 sites of duplicated path logic
- Deletes dead research path special case from doctor
- Adds `temper doctor` validation for `temper-owner` frontmatter (presence, pattern, directory alignment)
- Adds `temper sync` preflight ownership check — refuses to upload entries whose frontmatter disagrees with manifest
- Strips legacy no-sigil fallback from `resource_for_uri()`

## Migration required
Vaults must be migrated to the owner-segmented layout before this branch is used. Run `scripts/migrate-vault-to-owner-segmented.sh <vault> --apply` on each machine.

## Test plan
- [x] `cargo make check` green
- [x] `cargo make test` green
- [x] `cargo make test-db` green (includes new Vault parity test and legacy URI rejection test)
- [x] Manual smoke test on migrated laptop vault: resource list/show/create, sync status/run, search, doctor
- [x] Ownership mismatch smoke test: deliberate frontmatter drift detected by doctor and sync preflight

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 6: Merge after CI green**

User-driven. Once GitHub Actions reports green on the PR, merge to main. Vercel deploys automatically.

- [ ] **Step 7: Production smoke test**

User-driven:

```bash
temper sync status  # against production API
temper sync run     # should succeed on both machines
```

Expected: Both machines sync cleanly against the newly deployed server.

---

## Success criteria

- [ ] `Vault` abstraction lands in `temper-core` with filesystem + URI operations and full unit test coverage.
- [ ] Cross-implementation parity test green: `Vault::canonical_uri` matches `kb_resource_uri()` for identical inputs.
- [ ] Zero `PathBuf::join` / `format!` path arithmetic for vault layout outside `vault.rs`.
- [ ] Two ad-hoc path parsers (`infer_context_and_doctype`, `sync.rs` split loops) collapsed into single `Vault::parse_rel` calls.
- [ ] Dead research special case removed from `doctor.rs`.
- [ ] `temper doctor` validates `temper-owner` with correct server-authoritative scoping (four test cases cover the branches).
- [ ] `temper sync` preflight refuses to rewrite ownership via frontmatter edits; clean entries still sync.
- [ ] Both laptop and desktop vaults migrated to owner-segmented layout; local CLI works end-to-end against both.
- [ ] `resource_for_uri` legacy fallback stripped; legacy URIs return empty.
- [ ] `cargo make check && cargo make test && cargo make test-db` green.
- [ ] Branch merged to main, deployed to Vercel, production sync works from both machines.
