# Owner-Scoped URIs: Phase 2 CLI + Vault

**Date:** 2026-04-08
**Session:** 4 of the system-access-gate workstream
**Scope:** `Vault` abstraction in temper-core, CLI migration to owner-segmented vault
layout, `temper doctor` ownership validation, `temper sync` preflight ownership checks,
legacy `resource_for_uri` fallback removal
**Research:** R11 — System Access Gate and Owner-Scoped URIs (sections 4.1–4.4, 4.9)
**Prior sessions:**
- Session 1 — Phase 1 data + API (system access gate)
- Session 2 — Phase 1 CLI + MCP (system access gate)
- Session 3 — Phase 2 DB + API + core types (owner-scoped URIs: profile slugs, URI
  function rewrites, Profile.slug, Subscription.owner, temper-owner frontmatter field)

**Branch:** jct/temper-system-access-gate

---

## Summary

Session 3 introduced owner-scoped URIs at the database + core-types layer and added
`Subscription.owner` + `Profile.slug` + `temper-owner` frontmatter field, but left a
legacy fallback in `resource_for_uri()` and did not touch CLI path construction. This
session makes the CLI owner-aware by:

1. Centralizing all vault layout and URI construction in a new `Vault<'a>` abstraction
   in `temper-core`, replacing dispersed `doc_type_dir` / `PathBuf::join` / `format!`
   path arithmetic.
2. Migrating every CLI call site to consume `Vault` for both filesystem and URI
   operations.
3. Adding `temper doctor` validation for the `temper-owner` frontmatter field, scoped
   correctly to what's server-authoritative versus local-authoritative.
4. Adding `temper sync` preflight ownership validation to refuse uploads that would
   silently rewrite ownership via frontmatter edits.
5. Running a one-off shell script (not a CLI command) to physically migrate Pete's two
   alpha vaults into the new owner-segmented layout.
6. Removing the legacy fallback branch in `resource_for_uri()` once vaults are migrated.
7. Merging and deploying the full system-access-gate branch.

## Guiding principles

- **Ownership is server-authoritative.** Local frontmatter, filesystem paths, and
  manifest entries are derived displays. No local edit can unilaterally transfer
  ownership — that requires an explicit server-side action (future team commands).
- **One source of truth for vault layout.** Every filesystem path, manifest entry
  string, and `kb://` URI comes from one helper. Path arithmetic outside `vault.rs` is
  a code-review red flag.
- **Cross-implementation parity.** Rust `Vault::canonical_uri` and SQL
  `kb_resource_uri()` must produce identical outputs for the same inputs, enforced by
  an integration test.
- **Resource types are uniform.** Every doc type traverses through the same path rules.
  No special cases for research or any other type.
- **YAGNI for team primitives.** `--owner` flag, MCP owner parameter, and
  `temper team transfer` wait until there is a second owner to justify them.

## Out of scope

- `--owner` flag on resource-creating commands (deferred to Session 5).
- MCP `create_resource` owner parameter (deferred to Session 5).
- `temper team transfer` command and supporting server API (deferred — requires
  dedicated design work).
- `temper doctor migrate-vault` Rust command (replaced by one-off shell script).
- SvelteKit UI for ownership display, transfer, or owner picker.
- `temper profile update --slug-to` (deferred from Session 3 notes).

---

## 1. Vault abstraction in temper-core

### 1.1 New file: `crates/temper-core/src/vault.rs`

```rust
use std::path::{Path, PathBuf};
use crate::types::vault_config::Subscription;

/// Centralizes all vault layout and URI construction rules. Shared by CLI, API, MCP
/// so that filesystem paths, manifest entries, and kb:// URIs all derive from one
/// source of truth.
pub struct Vault<'a> {
    vault_root: &'a Path,
}

impl<'a> Vault<'a> {
    pub fn new(vault_root: &'a Path) -> Self;

    // ----- Filesystem operations (require vault_root) -----

    /// Absolute dir where a (subscription, doc_type)'s files live.
    /// e.g., <vault>/@me/temper/task/
    pub fn doc_type_dir(&self, sub: &Subscription, doc_type: &str) -> PathBuf;

    /// Absolute file path for a specific resource.
    /// e.g., <vault>/@me/temper/task/my-slug.md
    pub fn doc_file(&self, sub: &Subscription, doc_type: &str, slug: &str) -> PathBuf;

    /// Vault-relative path string used in manifest entries and discovery events.
    /// e.g., "@me/temper/task/my-slug.md"
    pub fn rel_path(&self, sub: &Subscription, doc_type: &str, slug: &str) -> String;

    /// Parse a vault-relative path back into components.
    /// Returns None for malformed paths (missing segments, no owner sigil, bad filename).
    pub fn parse_rel<'r>(&self, rel: &'r str) -> Option<ParsedVaultPath<'r>>;

    // ----- URI operations (pure, no vault_root needed) -----
    // Associated functions so API / MCP can use them without a Vault instance.

    /// Build a canonical kb:// URI from components.
    /// e.g., kb://@me/temper/task/my-slug
    pub fn canonical_uri(sub: &Subscription, doc_type: &str, ident: &str) -> String;

    /// Parse a kb:// URI into components. Rejects legacy no-sigil URIs by returning None.
    pub fn parse_uri(uri: &str) -> Option<ParsedKbUri<'_>>;
}

/// Parsed vault-relative path. Lifetime borrows from the input string.
pub struct ParsedVaultPath<'a> {
    pub owner: &'a str,      // "@me", "+platform-eng"
    pub context: &'a str,
    pub doc_type: &'a str,
    pub slug: &'a str,       // filename stem, no extension
}

/// Parsed kb:// URI. Lifetime borrows from the input string.
pub struct ParsedKbUri<'a> {
    pub owner: &'a str,      // "@me" (sigil included)
    pub context: &'a str,
    pub doc_type: &'a str,
    pub ident: &'a str,      // slug or UUID string
}
```

### 1.2 Path rules (codified in Vault)

- **Filesystem layout:** `<vault_root>/<owner>/<context>/<doc_type>/<slug>.md`
- **Manifest relative:** `<owner>/<context>/<doc_type>/<slug>.md`
- **Canonical URI:** `kb://<owner>/<context>/<doc_type>/<ident>` where `ident` is the
  slug (preferred) or the UUID as a fallback.
- **Owner segment:** derived from `Subscription::resolved_owner()` — returns
  `subscription.owner` if set, else `+<team>` if team is set, else `@me`.

### 1.3 Parser rules (`parse_rel`, `parse_uri`)

- Reject strings with fewer than four segments after splitting on `/`.
- Reject first segment that does not start with `@` or `+`.
- `parse_rel` expects the last segment to end in `.md`; stem becomes `slug`.
- `parse_uri` expects exactly `kb://` prefix; the identifier segment is returned as-is
  (caller decides whether it's slug or UUID).
- Legacy no-sigil URIs (`kb://context/type/ident`) return `None` — they are invalid
  after the Phase 2 server migration lands.

### 1.4 Tests (co-located in `vault.rs`)

- `doc_type_dir`: three owners (@me, @other, +team), three doc types.
- `doc_file`: round-trips through `parse_rel`.
- `rel_path`: produces strings consistent with `doc_file.strip_prefix(vault_root)`.
- `parse_rel`: accepts well-formed, rejects no-sigil, rejects too-few segments, rejects
  non-`.md` filename.
- `canonical_uri`: three owners, slug and UUID idents.
- `parse_uri`: accepts well-formed, rejects no-sigil, rejects missing `kb://`, rejects
  too-few segments.
- Round-trip: `parse_uri(canonical_uri(...))` recovers inputs.

### 1.5 Cross-implementation parity test

Behind `test-db` feature, in `crates/temper-core/tests/vault_parity.rs` (or wherever
integration tests live for temper-core):

```rust
#[sqlx::test]
async fn vault_canonical_uri_matches_sql_kb_resource_uri(pool: PgPool) {
    // 1. Insert a profile, context, doc_type, resource.
    // 2. SELECT kb_resource_uri(resource.id) FROM kb_resources.
    // 3. Vault::canonical_uri(&subscription, &doc_type.name, &resource.slug).
    // 4. assert_eq!.
}
```

The test covers @profile owner and (once team fixtures exist) +team owner.

### 1.6 Module export

Add `pub mod vault;` to `crates/temper-core/src/lib.rs` alongside existing modules.

---

## 2. CLI migration to Vault

### 2.1 Delete dispersed helpers

- Remove `Config::doc_type_dir` from `crates/temper-core/src/config.rs`. Compile
  failures become the migration checklist for every call site.
- Remove any other `PathBuf::join` / `format!` path arithmetic related to vault layout
  that appears in the compile errors.

### 2.2 Call-site migrations

Every site takes a `&Subscription` where it previously took `context: &str`. A small
`Config::subscription_for_context(&self, context: &str) -> Result<&Subscription>`
accessor is added (or reused if one already exists) so callers can cheaply look up
the right subscription when they only have a context name.

| File | Lines (approx) | Change |
|------|----------------|--------|
| `temper-cli/src/actions/task.rs` | 23-39, 196 | `load_tasks`, `save_task` use `Vault::doc_type_dir` + `doc_file` |
| `temper-cli/src/actions/goal.rs` | 30-38, 74-75, 98, 139 | same pattern; hardcoded `format!("{context}/goal/{slug}.md")` strings replaced with `Vault::rel_path` |
| `temper-cli/src/commands/resource.rs` | 171-172, 182 | `Vault::doc_file` for concept/decision; `Vault::rel_path` for discovery event |
| `temper-cli/src/actions/doctor.rs` | 47-67 | iterate subscriptions instead of context names; delete dead research special case (57-67) |
| `temper-cli/src/actions/sync.rs` | 114-119, 125-130, 209-219, 248-267, 333, 532-537 | manifest path parsing and writing goes through `Vault::parse_rel` / `Vault::rel_path`; `parse_kb_uri` replaced by `Vault::parse_uri` |
| `temper-cli/src/actions/ingest.rs` | 554-598 | `infer_context_and_doctype` reduced to a call to `Vault::parse_rel`, returning `ParsedVaultPath` |
| `temper-cli/src/actions/doctor_fix.rs` | 671, 691, 899 | manifest `entry.path` reads/writes transit owner-aware strings via `Vault::rel_path` |

### 2.3 Subscription lookup

A few action functions receive just `context: &str` from command parsing. They gain a
one-line `let sub = config.subscription_for_context(context)?;` at the top. Discovery
event emitters that are called from deep action code get the subscription passed
through rather than reconstructing it.

### 2.4 Acceptance criteria

- No hand-composed vault-layout paths remain outside `vault.rs`. The only permissible
  `PathBuf::join` calls in `temper-cli/src/actions` are on absolute paths returned by
  `Vault::doc_type_dir` / `Vault::doc_file`, or on `vault_root` for non-layout
  concerns (e.g., `.temper/manifest.json`). No `format!("{}/{}/{}", ...)` strings
  compose owner / context / doc_type / slug segments anywhere outside `vault.rs`.
- Two parsers (`infer_context_and_doctype`, `sync.rs` split loops) collapse into one
  call site each going through `Vault::parse_rel`.
- Dead research special case at `doctor.rs:57-67` is deleted.
- `cargo make check && cargo make test` passes.

---

## 3. `temper doctor` ownership validation

### 3.1 Rules

Ownership has three local representations: manifest path's owner segment (derived from
server), filesystem path (should match manifest), and frontmatter `temper-owner`
(derived display). Doctor enforces the right invariants without pretending to know
what the server has not told it.

**Missing `temper-owner` frontmatter**
- **Provisional/unsynced** (not in manifest, or `manifest_entry.provisional == true`):
  auto-fix to `@me`. Safe — no server truth to contradict.
- **Synced** (manifest entry exists, not provisional): yellow warning, **not
  auto-fixable**. Hint: "run `temper sync run` to reconcile ownership from server".

**Pattern violation** (`temper-owner` present but doesn't match
`^[@+][a-z0-9][a-z0-9-]*$`)
- Red error, never auto-fixable. User must correct by hand or revert.

**Directory mismatch** (`temper-owner` in frontmatter disagrees with the owner segment
of the file's manifest path)
- Yellow warning, **never auto-fixed**. Message: "ownership disagrees with manifest —
  if this was an accidental edit, revert it; ownership transfer requires an explicit
  server-side action (not yet implemented)".

### 3.2 Implementation

- Extend `scan_file` in `temper-cli/src/actions/doctor.rs` with three new issue kinds:
  `MissingTemperOwner`, `InvalidTemperOwnerPattern`, `OwnerDirectoryMismatch`.
- Add `FixAction::SetOwnerField { file: PathBuf, value: String }` to
  `temper-cli/src/actions/doctor_fix.rs`, alongside the existing `FixAction` variants.
  Only emitted for the provisional-missing case.
- Extend `DoctorReport` with `owner_backfilled: u32` counter. Printed in the summary
  when non-zero.
- Reads the manifest during scan to determine provisional vs synced. (The existing
  doctor code already loads the manifest for relocate actions, so this is a reuse.)

### 3.3 Tests

`crates/temper-cli/tests/` (integration) + unit tests in doctor modules:

- `missing_temper_owner_on_provisional_autofixes_to_me`
- `missing_temper_owner_on_synced_warns_not_fixable`
- `invalid_temper_owner_pattern_errors`
- `owner_directory_mismatch_warns_never_fixes`
- `doctor_fix_backfills_owner_field_and_increments_counter`

---

## 4. `temper sync` preflight ownership validation

### 4.1 Rules

Before any upload, validate every manifest entry:

1. **Owner drift on synced files.** For entries with `provisional: false`:
   - Read the file's frontmatter `temper-owner`.
   - Parse the manifest entry's path via `Vault::parse_rel` to get its owner segment.
   - On disagreement, skip this entry from the upload set and record a mismatch.
2. **Provisional owner freeze.** Provisional entries (being POSTed for the first
   time) are **not** checked by preflight. Their frontmatter `temper-owner` IS the
   owner claim sent to the server — there is no server truth to drift from yet. Once
   the POST is accepted and the entry is rekeyed from provisional to real ID, rule 1
   applies from the next sync onward.
3. **Resource-UUID / path-owner alignment.** If the file's `temper-id` matches a
   manifest entry whose path owner disagrees with the file's `temper-owner`, treat as
   the same mismatch category as rule 1.

### 4.2 Implementation

- New function in `temper-cli/src/actions/sync.rs`:

  ```rust
  struct OwnershipMismatch {
      file_path: String,
      frontmatter_owner: String,
      manifest_owner: String,
  }

  fn preflight_ownership_check(
      manifest: &Manifest,
      vault: &Vault<'_>,
      vault_root: &Path,
  ) -> Result<Vec<OwnershipMismatch>>;
  ```

- Called at the top of `sync::run` and `sync::status`.
- `sync status` prints mismatches under a new `Ownership Mismatches` section (yellow).
  Clean entries still display normally.
- `sync run` skips mismatched entries from the upload set, processes clean entries
  normally, and reports the skip count at the end with remediation hint.
- Not a hard abort on either command — mismatches coexist with clean state.

### 4.3 Tests

- `preflight_detects_synced_owner_drift`
- `preflight_ignores_provisional_entries`
- `preflight_detects_resource_uuid_owner_mismatch`
- `sync_run_skips_mismatched_entries_and_continues`
- `sync_status_reports_mismatches_without_blocking`

---

## 5. Strip legacy fallback from `resource_for_uri`

### 5.1 Migration

New file: `migrations/20260408NNNNNN_resource_for_uri_drop_legacy.sql`

The migration `CREATE OR REPLACE`s `resource_for_uri` with the same signature and
return type as the Session 3 version (seven columns: `resource_id`, `origin_uri`,
`content_hash`, `updated`, `is_active`, `access_level`, `team_role`) but with the
legacy no-sigil branch removed. Specifically, the body changes:

- **Remove** the `ELSE` branch in the `IF parts[1] LIKE '@%' OR parts[1] LIKE '+%'`
  conditional that infers owner from the requesting profile for three-segment URIs.
- **Replace** that `ELSE` branch with `RETURN;` so legacy URIs yield an empty result.

Everything else (UUID-or-slug identifier resolution, `resources_visible_to` join,
return row shape) stays identical to the Session 3 implementation at
`migrations/<session-3-file>.sql`.

### 5.2 sqlx cache

After migration, regenerate and commit:

```bash
cargo sqlx prepare --workspace -- --all-features
git add .sqlx/
```

### 5.3 Tests

Extend the existing `resource_for_uri` integration tests from Session 3:

- `resource_for_uri_resolves_owner_scoped_uri` (already passes, verify still green)
- `resource_for_uri_resolves_slug_identifier` (already passes, verify)
- `resource_for_uri_rejects_legacy_no_sigil_uri` (new — was previously resolved via
  fallback, should now return empty)

---

## 6. One-off shell script

**Location:** `scripts/migrate-vault-to-owner-segmented.sh` (or a Python equivalent
if easier to make idempotent).

**Behavior:**
1. Dry-run mode by default. `--apply` to execute. Refuse to run without `--apply` in
   any automation context.
2. Discover every top-level directory in `<vault>` that is not `.temper` and not
   already an owner directory (starting with `@` or `+`). These are context
   directories in the old layout.
3. Move each one into `<vault>/@me/<context>/`. `mkdir -p @me` if needed.
4. Rewrite `<vault>/.temper/manifest.json`: for every `entries[*].path`, prepend
   `@me/`. Use `jq` with an in-place edit, backed up to
   `manifest.json.pre-migration-bak`.
5. Walk every `.md` file under `<vault>/@me/**`. If its frontmatter lacks
   `temper-owner`, insert `temper-owner: "@me"` in the managed field block (above
   any open fields). If the field is already present, leave it alone.
6. Print a summary of moved directories, rewritten manifest entries, and backfilled
   files.
7. Idempotent: re-running the script on an already-migrated vault is a no-op with a
   "nothing to do" message.

**Execution plan:**
1. Test on a throwaway copy of the laptop vault first (just `cp -R` the vault to a
   scratch location and run there).
2. Review the diff in the scratch vault.
3. Run `--apply` on the real laptop vault.
4. Smoke test: `temper resource list --context temper`, `temper sync status`,
   `temper sync run`. Everything should work end-to-end against the local binary.
5. Commit the migrated `kb-vault` repo. Push.
6. On the desktop: pull the migrated repo (which now has `@me/` layout). Manifest
   regeneration from pull should put the manifest in sync automatically; if it
   doesn't, re-run the script there too.
7. Smoke test on desktop.

**Not shipped in the CLI.** Committed as an execution artifact under `scripts/` for
record-keeping only. Will not be maintained after this session.

---

## 7. Implementation sequence

Each phase is a logical commit boundary. `cargo make check && cargo make test &&
cargo make test-db` runs green between phases.

**Phase 1 — Vault abstraction (foundation)**
1. Create `crates/temper-core/src/vault.rs` with all types and methods.
2. Expose `pub mod vault` in `temper-core/src/lib.rs`.
3. Unit tests in `vault.rs` for filesystem and URI operations.
4. Cross-implementation parity integration test.

**Phase 2 — CLI migration to Vault**
5. Delete `Config::doc_type_dir`; use compile errors as migration checklist.
6. Migrate `task.rs`, `goal.rs`, `commands/resource.rs` path construction and
   discovery events.
7. Migrate `doctor.rs::scan` to iterate subscriptions; delete dead research case.
8. Migrate `ingest.rs::infer_context_and_doctype` to `Vault::parse_rel`.
9. Migrate `sync.rs` path parsing and `parse_kb_uri` to `Vault::parse_uri`.
10. Migrate `doctor_fix.rs` manifest entry path touches.
11. Green verification suite.

**Phase 3 — Shell script + dogfood**
12. Write `scripts/migrate-vault-to-owner-segmented.sh`.
13. Test on throwaway vault copy.
14. Run on laptop; smoke test.
15. Commit migrated kb-vault to GitHub.
16. Pull on desktop, reconcile, smoke test.

**Phase 4 — Doctor + sync ownership validation**
17. Extend `doctor.rs::scan_file` + `doctor_fix.rs` with `temper-owner` rules.
18. Add `sync::preflight_ownership_check` and wire into `sync::run` / `sync::status`.
19. Tests for all doctor and sync ownership branches.
20. Green verification suite.

**Phase 5 — Strip legacy fallback**
21. New SQL migration `resource_for_uri_drop_legacy.sql`.
22. Regenerate sqlx cache; commit `.sqlx/`.
23. Integration tests confirming legacy URIs return empty.
24. Final green verification suite.

**Phase 6 — Merge + deploy**
25. Open PR against main. Self-review for stray path arithmetic outside `vault.rs`.
26. Merge. Vercel picks up the new server code.
27. Verify production `temper sync run` from both machines.

## Rollback story by phase

- **Phases 1–2:** revert commits, no data touched.
- **Phase 3:** `git reset` the kb-vault repo; old layout returns.
- **Phase 4:** revert commits, no schema changes.
- **Phase 5:** one-way door at the schema level, but a new up-migration that re-adds
  the fallback branch is always possible if a client regression emerges in production.

---

## 8. Verification gates

Every phase ends with:

```bash
cargo make check       # fmt + clippy + docs + machete + TS typecheck + biome
cargo make test        # unit tests (no DB)
cargo make test-db     # integration tests (Docker Postgres required)
```

Manual smoke tests after Phase 3 (migrated vault) and Phase 5 (legacy fallback strip):

- `temper resource list --context temper`
- `temper resource create --type task --title "smoke test"`
- `temper sync status`
- `temper sync run`
- `temper search "vault layout"`
- `temper doctor` (expect green on migrated vault)
- Create a deliberate frontmatter mismatch; confirm doctor warns and sync skips.

## 9. Success criteria

- `Vault` abstraction lands in `temper-core` with full test coverage.
- Cross-implementation parity test green: `Vault::canonical_uri` matches
  `kb_resource_uri()` for identical inputs.
- Zero `PathBuf::join` / `format!` path arithmetic outside `vault.rs`.
- Two parsers (`infer_context_and_doctype`, `sync.rs` split loops) collapsed into one
  `Vault::parse_rel` call each.
- Dead research special case removed from `doctor.rs`.
- `temper doctor` validates `temper-owner` with correct server-authoritative scoping.
- `temper sync` preflight refuses to rewrite ownership via frontmatter edits.
- Both laptop and desktop vaults migrated to owner-segmented layout; local CLI works
  end-to-end against both.
- `resource_for_uri` legacy fallback stripped; legacy URIs return empty.
- `cargo make check && cargo make test && cargo make test-db` green.
- Branch merged to main, deployed to Vercel, production sync works.

## 10. Explicitly deferred

- `temper team transfer <resource> --to @other` command and server API — future
  session, part of team commands.
- `temper doctor migrate-vault` Rust command — never (shell script replaces it for
  alpha).
- `--owner` flag on `temper resource create` and related commands — Session 5.
- MCP `create_resource` owner parameter — Session 5.
- SvelteKit UI for ownership display, transfer, or owner picker — Session 6+.
- `temper profile update --slug-to` — deferred from Session 3.
