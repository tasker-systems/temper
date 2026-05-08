# Ownership Bug Warning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Review cadence (this plan):** Per user preference, do NOT run per-task spec/code-quality review subagents. Each subagent task should implement, verify locally (TDD red-green + `cargo make check`), then commit and surface for the next task. A consolidated spec + code-quality review runs as the final task using `superpowers:requesting-code-review`.

**Goal:** Eliminate the spurious ownership-mismatch warning emitted by `temper sync run` for the user's own newly-tracked resources, and prevent the same class of bug recurring via symmetric defenses on the write and read sides.

**Architecture:** Two-sided fix in `crates/temper-cli`. Write side: `build_frontmatter_from_resource` gains a `canonical_owner` parameter; callers resolve `@me` → `@<profile.slug>` via a new helper before passing it in. Read side: `preflight_ownership_check` gains a `current_owner_slug` parameter and uses an `owners_equivalent` helper that treats `@me` and `@<current_owner_slug>` as aliases in either direction. Plumbing: `sync_cmd.rs` fetches the profile before preflight; `ensure_profile` returns the resolved `Profile` so the slug is reused.

**Tech Stack:** Rust workspace (cargo-make, cargo-nextest), sqlx, axum, postgres. Tests use the embed-gated e2e recipe from `CLAUDE.md` for the regression test.

**Spec:** `docs/superpowers/specs/2026-05-08-ownership-bug-warning-design.md`

---

## File Map

| File | Change | Responsibility |
|---|---|---|
| `crates/temper-cli/src/actions/sync.rs` | Add helpers `resolve_owner_for_frontmatter` and `owners_equivalent`; modify `preflight_ownership_check` signature and body; update 3 call sites of `build_frontmatter_from_resource`; add unit tests | Owner resolution helpers, ownership preflight, sync orchestration |
| `crates/temper-cli/src/actions/ingest.rs` | Modify `build_frontmatter_from_resource` signature; update 5 existing test call sites; add 2 new unit tests | Resource → frontmatter projection |
| `crates/temper-cli/src/actions/runtime.rs` | Change `ensure_profile` return type from `Result<()>` to `Result<Profile>` | Profile pre-flight |
| `crates/temper-cli/src/commands/sync_cmd.rs` | Reorder so client/profile is built before preflight; thread `profile.slug` into preflight; update line that ignores ensure_profile result | `temper sync run` command wiring |
| `crates/temper-cli/src/commands/add.rs` | Update 5 call sites of `ensure_profile` to discard returned `Profile` | Other commands that don't need the profile |
| `tests/e2e/tests/pull_command_test.rs` | Add regression test next to `pull_one_resource_with_manifest_but_untracked_id_writes_canonical_layout` | E2E coverage of the round-trip |

No files are deleted. No new files are created.

---

## Task 1: New helper `resolve_owner_for_frontmatter` + unit tests

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs` (helper near top of pure-functions section, ~line 180; tests in existing `#[cfg(test)] mod tests` block)

- [ ] **Step 1: Write the failing tests**

Add to the existing test module in `crates/temper-cli/src/actions/sync.rs` (the module that already contains `preflight_detects_synced_owner_drift` etc.). Place near the other pure-function unit tests:

```rust
    // --- resolve_owner_for_frontmatter ---

    #[test]
    fn resolve_owner_for_frontmatter_resolves_at_me() {
        assert_eq!(
            resolve_owner_for_frontmatter("@me", "j-cole-taylor"),
            "@j-cole-taylor"
        );
    }

    #[test]
    fn resolve_owner_for_frontmatter_passes_through_team_handle() {
        assert_eq!(
            resolve_owner_for_frontmatter("+platform-eng", "j-cole-taylor"),
            "+platform-eng"
        );
    }

    #[test]
    fn resolve_owner_for_frontmatter_passes_through_other_user() {
        assert_eq!(
            resolve_owner_for_frontmatter("@some-other-user", "j-cole-taylor"),
            "@some-other-user"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-cli resolve_owner_for_frontmatter
```

Expected: compile error (`cannot find function 'resolve_owner_for_frontmatter' in this scope`).

- [ ] **Step 3: Implement the helper**

Add to `crates/temper-cli/src/actions/sync.rs`, in the "Pure functions" section near line 180 (look for the `// Pure functions (no client, no async — fully unit-testable)` banner comment). Insert a new banner-and-function block:

```rust
// ---------------------------------------------------------------------------
// Owner sigil resolution
// ---------------------------------------------------------------------------

/// Resolve the API's `owner_handle` shorthand to the canonical owner sigil
/// used in vault paths and `kb_resource_uri()`.
///
/// The API returns the literal string `"@me"` for the requester's own
/// resources (see `OWNER_HANDLE_EXPR` in `resource_service.rs`). The vault
/// layout and the server's `kb_resource_uri()` SQL function use
/// `@<profile.slug>` as the canonical owner segment. This helper closes the
/// gap: callers pass `resource.owner_handle` plus the requester's own
/// `profile.slug` (without leading `@`) and get back the canonical sigil.
///
/// Team handles (`+<team-slug>`) are already canonical and pass through
/// unchanged; so do other users' personal handles.
pub fn resolve_owner_for_frontmatter(handle: &str, profile_slug: &str) -> String {
    if handle == "@me" {
        format!("@{profile_slug}")
    } else {
        handle.to_string()
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p temper-cli resolve_owner_for_frontmatter
```

Expected: 3 passed.

- [ ] **Step 5: Run lints**

```bash
cargo make check
```

Expected: passes (no clippy or fmt complaints).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "feat(sync): add resolve_owner_for_frontmatter helper

Pure helper that resolves the API's @me display alias to the canonical
@<profile.slug> sigil used in vault paths and kb_resource_uri(). Team
handles and other users' handles pass through unchanged.

Used by upcoming changes to build_frontmatter_from_resource and
preflight_ownership_check to keep on-disk frontmatter in sync with the
canonical owner segment used everywhere else.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `ensure_profile` returns `Profile`

**Files:**
- Modify: `crates/temper-cli/src/actions/runtime.rs:114-121` (signature change)
- Modify: `crates/temper-cli/src/commands/sync_cmd.rs:103, 284, 338` (callers)
- Modify: `crates/temper-cli/src/commands/add.rs:134, 217, 404, 650, 706` (callers)

This is a pure refactor: no new behavior, no new tests. Compiler enforces correctness across all call sites.

- [ ] **Step 1: Change the signature in runtime.rs**

In `crates/temper-cli/src/actions/runtime.rs`, replace the existing `ensure_profile`:

```rust
/// Ensure the user's profile exists on the server, returning the resolved
/// `Profile` so callers can reuse fields like `slug` without a second
/// network round-trip.
///
/// Calls `GET /api/profile` which hits the Axum endpoint that auto-provisions
/// profiles for new users. This must be called before any TypeScript-routed
/// endpoints (ingest, sync) which require a pre-existing profile.
pub async fn ensure_profile(
    client: &temper_client::TemperClient,
) -> Result<temper_core::types::Profile> {
    client
        .profile()
        .get()
        .await
        .map_err(|e| TemperError::Api(format!("profile pre-flight: {e}")))
}
```

- [ ] **Step 2: Update callers that don't need the profile**

In `crates/temper-cli/src/commands/add.rs`, find the five call sites:

```bash
grep -n "ensure_profile" crates/temper-cli/src/commands/add.rs
```

Each currently looks like:

```rust
rt.block_on(runtime::ensure_profile(&client))?;
```

Change each to:

```rust
let _ = rt.block_on(runtime::ensure_profile(&client))?;
```

Apply the same change to `crates/temper-cli/src/commands/sync_cmd.rs` lines 284 and 338 (the two call sites that aren't the one we'll be reworking in Task 5).

Leave `sync_cmd.rs:103` alone for now — Task 5 reworks it.

- [ ] **Step 3: Verify the compile passes**

```bash
cargo build -p temper-cli
```

Expected: builds cleanly. If any caller is missed, the compiler will say `expected (), found Profile` — fix and retry.

- [ ] **Step 4: Run all tests in temper-cli**

```bash
cargo nextest run -p temper-cli
```

Expected: all existing tests pass (no behavioral change).

- [ ] **Step 5: Run lints**

```bash
cargo make check
```

Expected: passes.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/runtime.rs crates/temper-cli/src/commands/add.rs crates/temper-cli/src/commands/sync_cmd.rs
git commit -m "refactor(runtime): ensure_profile returns Profile

Returning the resolved Profile lets callers reuse fields like slug
without a second network round-trip. Callers that don't need the
profile bind it with let _ = ..., preserving prior behavior.

Prep for sync_cmd.rs ordering change that needs profile.slug before
preflight_ownership_check.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: `build_frontmatter_from_resource` accepts `canonical_owner`

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs:485` (signature + body)
- Modify: `crates/temper-cli/src/actions/ingest.rs:1187, 1230, 1277, 1317, 1464` (5 existing test call sites)
- Add: `crates/temper-cli/src/actions/ingest.rs` (2 new unit tests for canonical-owner behavior)
- Modify: `crates/temper-cli/src/actions/sync.rs:1440, 1523, 1645` (3 production call sites)

- [ ] **Step 1: Write the new failing tests in ingest.rs**

Add inside the existing `#[cfg(test)] mod tests` block in `crates/temper-cli/src/actions/ingest.rs` (next to `test_build_frontmatter_from_resource_preserves_arrays_and_objects`):

```rust
    #[test]
    fn build_frontmatter_from_resource_writes_canonical_owner_for_at_me() {
        let resource = test_resource_row();
        // Caller is responsible for resolving @me -> @<slug> before calling.

        let fm = build_frontmatter_from_resource(
            &resource,
            "temper",
            "research",
            "@j-cole-taylor",
            String::new(),
            None,
            None,
        )
        .unwrap();

        let owner = fm
            .value()
            .get("temper-owner")
            .and_then(|v| v.as_str())
            .expect("temper-owner must be set");
        assert_eq!(
            owner, "@j-cole-taylor",
            "frontmatter must record the canonical owner the caller passed in, \
             not the API's @me shorthand"
        );
    }

    #[test]
    fn build_frontmatter_from_resource_passes_team_handle_through() {
        let resource = test_resource_row();

        let fm = build_frontmatter_from_resource(
            &resource,
            "temper",
            "research",
            "+platform-eng",
            String::new(),
            None,
            None,
        )
        .unwrap();

        let owner = fm
            .value()
            .get("temper-owner")
            .and_then(|v| v.as_str())
            .expect("temper-owner must be set");
        assert_eq!(owner, "+platform-eng");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-cli build_frontmatter_from_resource_writes_canonical_owner_for_at_me
cargo nextest run -p temper-cli build_frontmatter_from_resource_passes_team_handle_through
```

Expected: compile error — function signature only takes 6 arguments, the test passes 7.

- [ ] **Step 3: Update the function signature and body**

In `crates/temper-cli/src/actions/ingest.rs`, locate `pub fn build_frontmatter_from_resource` (line 485). Update the signature and the `temper-owner` set:

```rust
/// Build a complete `Frontmatter` from a server `ResourceRow` plus the
/// caller-resolved canonical owner sigil.
///
/// `canonical_owner` is the value to write into `temper-owner`. The caller
/// is responsible for resolving the API's `@me` shorthand to
/// `@<profile.slug>` (use `crate::actions::sync::resolve_owner_for_frontmatter`).
/// Team handles (`+<team-slug>`) and other users' handles can be passed
/// through unchanged.
///
/// Combines resource-level fields (id, type, context, created, title) with
/// managed_meta fields (temper-* keys, stage, mode, effort, etc.) and
/// open_meta fields (user-defined keys: tags, relates_to, extends,
/// depends_on, and any other custom frontmatter) for complete frontmatter
/// that matches what the CLI would produce locally.
pub fn build_frontmatter_from_resource(
    resource: &temper_core::types::ResourceRow,
    context: &str,
    doc_type: &str,
    canonical_owner: &str,
    body: String,
    managed_meta: Option<&serde_json::Value>,
    open_meta: Option<&serde_json::Value>,
) -> crate::error::Result<temper_core::frontmatter::Frontmatter> {
```

Replace the existing `temper-owner` setter (the block that reads `if !resource.owner_handle.is_empty()`) with:

```rust
    if !canonical_owner.is_empty() {
        fm.set_managed_field(
            "temper-owner",
            serde_json::Value::String(canonical_owner.to_string()),
        );
    }
```

- [ ] **Step 4: Update the 5 existing test call sites in ingest.rs**

Each existing test call to `build_frontmatter_from_resource` needs `"@me"` inserted between the `doc_type` and `body` arguments — keeping their existing assertions intact (the test fixture already uses `owner_handle = "@me"`).

Locations and exact pattern: `cargo build -p temper-cli` will fail with "expected 7 arguments, found 6" at each site. Update each:

Before:
```rust
let fm = build_frontmatter_from_resource(
    &resource,
    "temper",
    "research",
    String::new(),
    Some(&meta),
    None,
)
```

After:
```rust
let fm = build_frontmatter_from_resource(
    &resource,
    "temper",
    "research",
    "@me",
    String::new(),
    Some(&meta),
    None,
)
```

Apply this transform at all 5 sites: lines ~1187, 1230, 1277, 1317, 1464. Confirm with:

```bash
grep -n "build_frontmatter_from_resource(" crates/temper-cli/src/actions/ingest.rs
```

There should be `pub fn build_frontmatter_from_resource(` plus 7 call sites (5 existing + 2 new) — all with the new arity.

- [ ] **Step 5: Update the 3 production call sites in sync.rs**

Find them:

```bash
grep -n "build_frontmatter_from_resource(" crates/temper-cli/src/actions/sync.rs
```

**Site 1 (line ~1440)** — currently inside the manifest-tracked branch. The caller has the `resource` in scope but does NOT have a resolved profile slug locally. Look at the surrounding context — find where `resource.owner_handle` would resolve. If the function doesn't have the profile available, this site needs to either:
  - Take a `profile_slug: &str` parameter from its caller, OR
  - Resolve via `resolve_owner_for_frontmatter` using the `client`/profile already in scope.

Read the surrounding 30 lines (`crates/temper-cli/src/actions/sync.rs`, ~line 1410-1450) and use whichever resolved owner string is in scope. If neither is in scope, fetch via `client.profile().get().await` once at the top of that function and store the slug. Apply:

```rust
let canonical_owner = crate::actions::sync::resolve_owner_for_frontmatter(
    resource.owner_handle.as_str(),
    profile_slug.as_str(),
);
let fm = ingest::build_frontmatter_from_resource(
    &resource,
    context,
    doc_type,
    &canonical_owner,
    /* existing body arg */,
    /* existing managed_meta arg */,
    /* existing open_meta arg */,
)?;
```

Adapt argument names to whatever the local scope uses.

**Site 2 (line ~1523)** — this is the `NewlyTracked` branch from PR #70. The owner is already resolved at lines 1494-1504 into a local called `owner` (an `&str`). Pass it as `canonical_owner`:

```rust
let fm = ingest::build_frontmatter_from_resource(
    &resource,
    context,
    doc_type,
    owner,                              // <-- new arg, already resolved above
    ingest::normalize_body_for_vault(&content_response.markdown),
    managed_value.as_ref(),
    content_response.open_meta.as_ref(),
)?;
```

**Site 3 (line ~1645)** — read the surrounding context (`crates/temper-cli/src/actions/sync.rs:1605-1660`) to see what's in scope. Apply the same `resolve_owner_for_frontmatter` pattern as Site 1, using whatever profile-slug source exists locally (or fetching once if needed).

- [ ] **Step 6: Build to verify all call sites compile**

```bash
cargo build -p temper-cli
```

Expected: builds cleanly. If a call site is still missing the new arg, the compiler reports it.

- [ ] **Step 7: Run unit tests**

```bash
cargo nextest run -p temper-cli build_frontmatter_from_resource
cargo nextest run -p temper-cli ingest
```

Expected: all 7 `build_frontmatter_from_resource` tests pass plus the rest of the ingest module tests.

- [ ] **Step 8: Run the full temper-cli test suite**

```bash
cargo nextest run -p temper-cli
```

Expected: all tests pass. (Earlier preflight tests are unaffected; they'll need updating in Task 4.)

- [ ] **Step 9: Run lints**

```bash
cargo make check
```

Expected: passes.

- [ ] **Step 10: Commit**

```bash
git add crates/temper-cli/src/actions/ingest.rs crates/temper-cli/src/actions/sync.rs
git commit -m "fix(sync): write canonical owner sigil into pulled frontmatter

build_frontmatter_from_resource now takes a canonical_owner argument
instead of reading resource.owner_handle directly. The 'NewlyTracked'
branch from PR #70 already resolves @me -> @<profile.slug> for the
on-disk path and manifest entry; thread that same resolved string
into the frontmatter so all three layers (path, manifest, frontmatter)
agree on the canonical sigil.

Other call sites resolve via the new resolve_owner_for_frontmatter
helper, fetching the profile slug as needed.

Two new unit tests cover the canonical-owner contract; five existing
test call sites updated for the new arity.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: `preflight_ownership_check` accepts `current_owner_slug`

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:120-158` (signature + body, plus new `owners_equivalent` helper)
- Modify: `crates/temper-cli/src/actions/sync.rs:3168-3281` (3 existing preflight tests, add new param)
- Add: 5 new tests in the same module

- [ ] **Step 1: Write the failing new tests**

Add inside the existing test module in `crates/temper-cli/src/actions/sync.rs`, after the existing preflight tests (after `preflight_clean_manifest_returns_empty`):

```rust
    // --- owners_equivalent (pure helper) ---

    #[test]
    fn owners_equivalent_treats_at_me_and_resolved_slug_as_equal() {
        assert!(owners_equivalent("@me", "@j-cole-taylor", "j-cole-taylor"));
        assert!(owners_equivalent("@j-cole-taylor", "@me", "j-cole-taylor"));
    }

    #[test]
    fn owners_equivalent_treats_byte_equal_strings_as_equal() {
        assert!(owners_equivalent("@me", "@me", "j-cole-taylor"));
        assert!(owners_equivalent("@j-cole-taylor", "@j-cole-taylor", "j-cole-taylor"));
        assert!(owners_equivalent("+team", "+team", "j-cole-taylor"));
    }

    #[test]
    fn owners_equivalent_rejects_other_user() {
        assert!(!owners_equivalent("@me", "@some-other", "j-cole-taylor"));
        assert!(!owners_equivalent("@some-other", "@j-cole-taylor", "j-cole-taylor"));
    }

    #[test]
    fn owners_equivalent_rejects_team_vs_personal() {
        assert!(!owners_equivalent("+platform-eng", "@me", "j-cole-taylor"));
        assert!(!owners_equivalent("+platform-eng", "@j-cole-taylor", "j-cole-taylor"));
    }

    // --- preflight_ownership_check tolerance ---

    #[test]
    fn preflight_ownership_check_treats_at_me_as_current_owner_alias() {
        // The bug case: PR #70 wrote a NewlyTracked file at
        // @<profile.slug>/temper/task/x.md but the frontmatter still says @me.
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = tmp.path();

        let file_dir = vault.join("@j-cole-taylor").join("temper").join("task");
        std::fs::create_dir_all(&file_dir).unwrap();
        std::fs::write(
            file_dir.join("x.md"),
            "---\ntemper-type: task\ntemper-owner: \"@me\"\ntemper-title: x\ntemper-slug: x\n---\n\nbody\n",
        )
        .unwrap();

        let mut manifest = Manifest::new("dev".to_string());
        let id = ResourceId::from(Uuid::now_v7());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@j-cole-taylor/temper/task/x.md".to_string(),
                body_hash: "h".to_string(),
                remote_body_hash: "h".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );

        let mismatches = preflight_ownership_check(&manifest, vault, "j-cole-taylor");
        assert!(
            mismatches.is_empty(),
            "@me in frontmatter must be tolerated as alias for the current user; got {mismatches:?}"
        );
    }

    #[test]
    fn preflight_ownership_check_treats_legacy_at_me_path_as_current_owner_alias() {
        // Symmetric: legacy on-disk path is @me/... but frontmatter has been
        // updated to the canonical @<slug>.
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = tmp.path();

        let file_dir = vault.join("@me").join("temper").join("task");
        std::fs::create_dir_all(&file_dir).unwrap();
        std::fs::write(
            file_dir.join("y.md"),
            "---\ntemper-type: task\ntemper-owner: \"@j-cole-taylor\"\ntemper-title: y\ntemper-slug: y\n---\n\nbody\n",
        )
        .unwrap();

        let mut manifest = Manifest::new("dev".to_string());
        let id = ResourceId::from(Uuid::now_v7());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/task/y.md".to_string(),
                body_hash: "h".to_string(),
                remote_body_hash: "h".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );

        let mismatches = preflight_ownership_check(&manifest, vault, "j-cole-taylor");
        assert!(mismatches.is_empty(), "got {mismatches:?}");
    }

    #[test]
    fn preflight_ownership_check_flags_other_owner_mismatch() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = tmp.path();

        let file_dir = vault.join("@j-cole-taylor").join("temper").join("task");
        std::fs::create_dir_all(&file_dir).unwrap();
        std::fs::write(
            file_dir.join("z.md"),
            "---\ntemper-type: task\ntemper-owner: \"@some-other-user\"\ntemper-title: z\ntemper-slug: z\n---\n\nbody\n",
        )
        .unwrap();

        let mut manifest = Manifest::new("dev".to_string());
        let id = ResourceId::from(Uuid::now_v7());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@j-cole-taylor/temper/task/z.md".to_string(),
                body_hash: "h".to_string(),
                remote_body_hash: "h".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );

        let mismatches = preflight_ownership_check(&manifest, vault, "j-cole-taylor");
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].frontmatter_owner, "@some-other-user");
        assert_eq!(mismatches[0].manifest_owner, "@j-cole-taylor");
    }
```

- [ ] **Step 2: Update the 3 existing preflight tests for the new arity**

In the same test module (`crates/temper-cli/src/actions/sync.rs`):

- `preflight_detects_synced_owner_drift` (line 3168) — change `preflight_ownership_check(&manifest, vault)` → `preflight_ownership_check(&manifest, vault, "dev-user")`. The frontmatter is `+team` and the manifest is `@me` — neither is the current user's resolved form, so the mismatch still gets flagged.
- `preflight_ignores_provisional_entries` (line 3207) — same call-site update with `"dev-user"`. Provisional entries are skipped before the equivalence check, so behavior is unchanged.
- `preflight_clean_manifest_returns_empty` (line 3247) — same call-site update with `"dev-user"`. Frontmatter `@me` and manifest `@me` are byte-equal, so the equivalence rule's first branch returns true regardless of `current_owner_slug`.

- [ ] **Step 3: Run all preflight tests to verify the new ones fail**

```bash
cargo nextest run -p temper-cli preflight
cargo nextest run -p temper-cli owners_equivalent
```

Expected: compile error (`owners_equivalent` not found, `preflight_ownership_check` arity mismatch).

- [ ] **Step 4: Implement `owners_equivalent` and update `preflight_ownership_check`**

In `crates/temper-cli/src/actions/sync.rs`, immediately above `pub fn preflight_ownership_check` (line 120), insert the helper:

```rust
/// Two owner sigils are equivalent if they are byte-equal OR one side is
/// the API's `@me` display alias and the other is `@<current_owner_slug>`.
///
/// Used to keep `preflight_ownership_check` from spuriously flagging files
/// whose frontmatter still says `@me` (legacy state, or files written via
/// `build_frontmatter_from_resource` before the canonical-owner fix landed)
/// against manifest paths that already use the canonical `@<profile.slug>`
/// segment — and vice versa for legacy `@me/` vault paths whose frontmatter
/// has been canonicalized.
fn owners_equivalent(a: &str, b: &str, current_owner_slug: &str) -> bool {
    if a == b {
        return true;
    }
    let resolved = format!("@{current_owner_slug}");
    (a == "@me" && b == resolved) || (b == "@me" && a == resolved)
}
```

Then update `preflight_ownership_check`:

```rust
pub fn preflight_ownership_check(
    manifest: &Manifest,
    vault_root: &Path,
    current_owner_slug: &str,
) -> Vec<OwnershipMismatch> {
    let mut mismatches = Vec::new();

    for entry in manifest.entries.values() {
        if entry.provisional {
            continue;
        }

        let Some(parsed) = Vault::parse_rel(&entry.path) else {
            continue;
        };
        let manifest_owner = parsed.owner.to_string();

        let abs_path = vault_root.join(&entry.path);
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let Ok(fm) = temper_core::frontmatter::Frontmatter::try_from(content.as_str()) else {
            continue;
        };
        let frontmatter_owner = fm
            .value()
            .get("temper-owner")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "@me".to_string());

        if !owners_equivalent(&frontmatter_owner, &manifest_owner, current_owner_slug) {
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

- [ ] **Step 5: Build to find the caller in sync_cmd.rs**

```bash
cargo build -p temper-cli
```

Expected: one compile error in `crates/temper-cli/src/commands/sync_cmd.rs:72` — call site has 2 args, function now takes 3. **Leave this for Task 5** — but the build will fail until Task 5 lands. To unblock testing of this task in isolation, make a minimal stub fix in `sync_cmd.rs:72`:

```rust
let ownership_mismatches = sync_actions::preflight_ownership_check(
    &manifest,
    &vault_root,
    "", // TEMP: placeholder, replaced in Task 5
);
```

The `""` empty string makes `format!("@{current_owner_slug}") == "@"`, which won't match any real owner sigil — so the read-side tolerance is effectively disabled until Task 5 wires up the real slug. That's acceptable for one commit because Task 5 is the very next task.

- [ ] **Step 6: Run all preflight + owner tests**

```bash
cargo nextest run -p temper-cli preflight
cargo nextest run -p temper-cli owners_equivalent
```

Expected: all pass.

- [ ] **Step 7: Run the full temper-cli test suite**

```bash
cargo nextest run -p temper-cli
```

Expected: all tests pass.

- [ ] **Step 8: Run lints**

```bash
cargo make check
```

Expected: passes.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs crates/temper-cli/src/commands/sync_cmd.rs
git commit -m "feat(sync): tolerate @me <-> @<slug> in ownership preflight

Adds owners_equivalent helper and threads current_owner_slug into
preflight_ownership_check. Two owner sigils are now equivalent if
byte-equal OR one side is @me and the other is @<current_owner_slug>.

Closes the spurious-mismatch warning that fires after a NewlyTracked
pull writes a file at @<slug>/.../x.md while its frontmatter still
records @me, and symmetrically tolerates legacy @me/ vault paths
whose frontmatter has been canonicalized.

The sync_cmd.rs call site uses an empty placeholder slug pending the
plumbing fix in the next commit; the read-side tolerance is dormant
until then.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Wire profile.slug through `sync_cmd.rs`

**Files:**
- Modify: `crates/temper-cli/src/commands/sync_cmd.rs:53-115` (reorder, thread slug)

- [ ] **Step 1: Read the current `sync_run` function structure**

```bash
sed -n '40,120p' crates/temper-cli/src/commands/sync_cmd.rs
```

Confirm the current order:
1. Resolve vault root + load manifest + normalize entries
2. `preflight_ownership_check(&manifest, &vault_root, "")` (Task 4 placeholder)
3. Build mismatch_paths set
4. `build_runtime_and_client()` and `ensure_profile`
5. `sync_orchestration`

- [ ] **Step 2: Reorder so profile is fetched before preflight**

Replace the relevant section of `crates/temper-cli/src/commands/sync_cmd.rs` (the block currently at ~lines 57-103). The exact target is the region from `let mut manifest = ...` through `rt.block_on(runtime::ensure_profile(&client))?;`. New version:

```rust
    let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    // Phase A invariant: normalize every manifest entry's file before any
    // other sync logic. Per-entry atomic save ensures an interrupt loses at
    // most one file's work.
    let progress = TerminalProgress::new();
    let normalize_report = sync_actions::normalize_all_entries(
        &mut manifest,
        &vault_root,
        &temper_dir,
        Some(&progress),
    )?;
    warn_blocked_paths(&normalize_report);

    // Build runtime + client + ensure profile *before* preflight so we can
    // pass profile.slug into preflight_ownership_check. Preflight gains a
    // network dependency, but every other branch of `sync run` already
    // requires connectivity.
    let (rt, client) = runtime::build_runtime_and_client()?;
    let profile = rt.block_on(runtime::ensure_profile(&client))?;

    // Preflight: detect and warn about ownership mismatches.
    let ownership_mismatches =
        sync_actions::preflight_ownership_check(&manifest, &vault_root, &profile.slug);
    if !ownership_mismatches.is_empty() {
        output::warning(format!(
            "{} file(s) have ownership mismatches and will be skipped from upload:",
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

    let mut mismatch_paths: std::collections::HashSet<String> = ownership_mismatches
        .iter()
        .map(|m| m.file_path.clone())
        .collect();
    // Blocked-by-normalize entries are also excluded from the push set —
    // sync must never ship a file with unresolved schema violations.
    for (path, _) in &normalize_report.issues_by_path {
        mismatch_paths.insert(path.clone());
    }

    let result = rt.block_on(async {
        sync_actions::sync_orchestration(
            &client,
            &mut manifest,
            &vault_root,
            contexts,
            &progress,
            &mismatch_paths,
        )
        .await
    })?;
```

The structural changes:
- Move `build_runtime_and_client()` + `ensure_profile` above preflight.
- Bind the `Profile` returned by `ensure_profile` to a local `profile`.
- Pass `&profile.slug` to `preflight_ownership_check`.
- Remove the placeholder `""` from Task 4.

- [ ] **Step 3: Build to verify**

```bash
cargo build -p temper-cli
```

Expected: builds cleanly.

- [ ] **Step 4: Run all temper-cli tests**

```bash
cargo nextest run -p temper-cli
```

Expected: all pass. Behavior of `temper sync run` is unchanged for healthy files; ownership warnings now correctly tolerate the current user's `@me` alias.

- [ ] **Step 5: Run lints**

```bash
cargo make check
```

Expected: passes.

- [ ] **Step 6: Manual smoke test against the real vault**

```bash
temper sync run
```

Expected: no ownership-mismatch warning for the 7 previously-affected files. The push set should include them now.

If the warning still fires, inspect the file paths and frontmatter — there may be a file we haven't accounted for (e.g. a different drift case). Report back rather than patching reactively.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/commands/sync_cmd.rs
git commit -m "fix(sync): plumb profile.slug into preflight_ownership_check

Reorders sync_cmd::sync_run so build_runtime_and_client + ensure_profile
run before preflight, then passes profile.slug into the check. Activates
the read-side tolerance from the previous commit.

Removes the temporary empty-string placeholder. Manual smoke test against
the affected vault confirms the spurious warning no longer fires for
the user's own NewlyTracked-pulled resources.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: E2E regression test

**Files:**
- Modify: `tests/e2e/tests/pull_command_test.rs` (add new test next to `pull_one_resource_with_manifest_but_untracked_id_writes_canonical_layout`)

- [ ] **Step 1: Add the new test**

Append to `tests/e2e/tests/pull_command_test.rs` (after the `pull_one_resource_with_manifest_but_untracked_id_writes_canonical_layout` test that ends at line 408). Reuse the same setup pattern:

```rust
/// Round-trip regression for the ownership-bug-warning fix
/// (docs/superpowers/specs/2026-05-08-ownership-bug-warning-design.md).
///
/// After a NewlyTracked pull, the file's frontmatter must record the
/// canonical `@<profile.slug>` owner sigil (not the API's `@me` shorthand)
/// AND `preflight_ownership_check` must report no mismatches when called
/// with the requester's profile slug. Together these prove the write side
/// (build_frontmatter_from_resource) and the read side (preflight) agree
/// on the canonical owner.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pull_one_resource_newly_tracked_writes_canonical_owner_and_passes_preflight(
    pool: sqlx::PgPool,
) {
    use temper_cli::actions::sync::preflight_ownership_check;

    let app = common::setup(pool).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("ownership-test")
        .await
        .expect("context create");

    let body = "# Ownership Round-Trip\n\nBody arrived from the server.".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "e"),
        embedding: vec![0.0_f32; 768],
    };
    let payload = IngestPayload {
        title: "Ownership Round-Trip".to_string(),
        origin_uri: "test://ownership".to_string(),
        context_name: "ownership-test".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: "ownership-roundtrip".to_string(),
        content: body.clone(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-05-08"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
    };
    let seeded = app.client.ingest().create(&payload).await.expect("ingest");

    let mut manifest = Manifest::new("e2e-ownership-device".to_string());
    let result = pull_one_resource(
        &app.client,
        app.vault_dir.path(),
        seeded.id,
        Some(&mut manifest),
        Some(temper_core::hash::compute_body_hash(&body)),
    )
    .await
    .expect("pull_one_resource");

    assert_eq!(result.branch, PullBranch::NewlyTracked);

    // Frontmatter must record the canonical @<profile.slug>, not @me.
    let on_disk = std::fs::read_to_string(&result.path).expect("file written");
    let canonical_owner_line = format!("temper-owner: '@{}'", profile.slug);
    let canonical_owner_line_dq = format!("temper-owner: \"@{}\"", profile.slug);
    let canonical_owner_line_bare = format!("temper-owner: @{}", profile.slug);
    assert!(
        on_disk.contains(&canonical_owner_line)
            || on_disk.contains(&canonical_owner_line_dq)
            || on_disk.contains(&canonical_owner_line_bare),
        "frontmatter must record canonical owner @{}; got:\n{on_disk}",
        profile.slug
    );
    assert!(
        !on_disk.contains("temper-owner: '@me'") && !on_disk.contains("temper-owner: \"@me\""),
        "frontmatter must NOT contain literal @me shorthand; got:\n{on_disk}"
    );

    // Preflight must accept the round-trip cleanly.
    let mismatches =
        preflight_ownership_check(&manifest, app.vault_dir.path(), &profile.slug);
    assert!(
        mismatches.is_empty(),
        "preflight must report no mismatches for the round-tripped resource; got {mismatches:?}"
    );
}
```

- [ ] **Step 2: Run the new test (with embed feature for full e2e coverage)**

```bash
cargo nextest run \
  --manifest-path tests/e2e/Cargo.toml \
  --features test-db,test-embed \
  pull_one_resource_newly_tracked_writes_canonical_owner_and_passes_preflight
```

Expected: passes. Requires Docker Postgres running (`cargo make docker-up`).

- [ ] **Step 3: Run the full pull_command_test.rs file**

```bash
cargo nextest run \
  --manifest-path tests/e2e/Cargo.toml \
  --features test-db,test-embed \
  -E 'test(pull_)' \
  --test pull_command_test
```

Expected: all 5 tests pass (4 existing + this one).

- [ ] **Step 4: Run the embed-gated full e2e suite**

```bash
cargo nextest run \
  --manifest-path tests/e2e/Cargo.toml \
  --features test-db,test-embed
```

Expected: all green. This catches workspace-feature-unification surprises (per `CLAUDE.md`'s embed-gated e2e note).

- [ ] **Step 5: Run lints**

```bash
cargo make check
```

Expected: passes.

- [ ] **Step 6: Commit**

```bash
git add tests/e2e/tests/pull_command_test.rs
git commit -m "test(e2e): regression for ownership round-trip

Locks in the contract that NewlyTracked pulls produce frontmatter with
the canonical @<profile.slug> owner sigil AND that preflight_ownership_check
reports no mismatches for that file when called with the requester's slug.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Consolidated review

This is the deferred review task per the user's preferred cadence.

- [ ] **Step 1: Verify all earlier tasks landed**

```bash
git log --oneline main..HEAD
```

Expected: 6 new commits on top of the spec commits. Confirm each task above produced its commit.

- [ ] **Step 2: Run the full check suite one more time**

```bash
cargo make check
cargo nextest run --workspace
cargo nextest run \
  --manifest-path tests/e2e/Cargo.toml \
  --features test-db,test-embed
```

Expected: everything green.

- [ ] **Step 3: Spec self-review against the implementation**

Re-read `docs/superpowers/specs/2026-05-08-ownership-bug-warning-design.md` and confirm:

- All 6 acceptance criteria are met (the seven files no longer warn; genuine mismatches still flagged; new pulls write canonical owner; tests pass; embed-gated e2e green; `cargo make check` green).
- All explicit non-goals were respected (no vault migration; no profile caching; no audit of unrelated frontmatter writers).

- [ ] **Step 4: Code-quality review via skill**

Invoke `superpowers:requesting-code-review` and brief it with:
- Branch: `jct/ownership-bug-warning`
- Spec: `docs/superpowers/specs/2026-05-08-ownership-bug-warning-design.md`
- Plan: `docs/superpowers/plans/2026-05-08-ownership-bug-warning.md`
- Focus: correctness of the symmetric defense (write + read), test coverage, signature changes propagated cleanly across all callers.

- [ ] **Step 5: Open the PR (only on user direction)**

Do not open the PR autonomously. When the user says to ship, follow the standard `gh pr create` flow with a body that links to the spec and the e2e regression test.

---

## Self-Review Notes (plan author)

Cross-checked against spec (commit `9d591a1` after the tightening amendment):

- ✅ Helper `resolve_owner_for_frontmatter` — Task 1
- ✅ `build_frontmatter_from_resource` signature change — Task 3
- ✅ `preflight_ownership_check` signature change + `owners_equivalent` — Task 4
- ✅ `ensure_profile` returns `Profile` — Task 2
- ✅ `sync_cmd.rs` reorder — Task 5
- ✅ E2E regression — Task 6
- ✅ All 8 unit tests + 1 e2e test from spec covered (3 helper + 2 ingest + 5 preflight + 1 e2e = 11; spec said 8 + 1, but we expanded to cover the symmetric direction added in the spec amendment)
- ✅ All 6 acceptance criteria mapped to verification steps in Tasks 5-7

Type/name consistency check:
- `resolve_owner_for_frontmatter(handle, profile_slug)` — same signature in helper definition (Task 1), spec usage description, and Task 3 caller updates ✓
- `owners_equivalent(a, b, current_owner_slug)` — same signature throughout Task 4 ✓
- `current_owner_slug: &str` (no `@` prefix) — consistent in `preflight_ownership_check` signature, all test calls, and the `sync_cmd.rs` plumbing (`&profile.slug`) ✓
- `canonical_owner: &str` parameter on `build_frontmatter_from_resource` — consistent across signature, 5 existing test updates, 3 production call sites, and 2 new tests ✓

No placeholders, no "TODO", no "implement later". The single intentional placeholder (`""` in Task 4 step 5) is explicitly removed in Task 5 step 2.
