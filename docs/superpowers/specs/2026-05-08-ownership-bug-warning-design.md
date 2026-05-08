# Ownership Bug Warning — Design

**Date**: 2026-05-08
**Branch**: `jct/ownership-bug-warning`
**Status**: Design approved; ready for implementation plan

## Problem

After PR #70 (`fix(sync): pull untracked ids into canonical layout`, merged 2026-05-08), `temper sync run` emits an ownership-mismatch warning for the user's own resources:

```
! 7 file(s) have ownership mismatches and will be skipped from upload:
!   @j-cole-taylor/temper/task/deprecate-resource-service--create-after-phase-3b.md
!     — frontmatter: @me, manifest: @j-cole-taylor
  ... (6 more)
Ownership transfers require an explicit server action (not yet implemented).
Revert the frontmatter edit or wait for `temper team transfer`.
```

The mismatch is spurious. `@me` is the API's display shorthand for the requester's own
`owner_handle`; `@<profile.slug>` is the canonical owner sigil established by
`kb_resource_uri()` (migrations/20260407000002_owner_scoped_uris.sql). They refer to the
same identity, but `preflight_ownership_check` compares them as opaque strings.

## Root Cause

PR #70's `NewlyTracked` branch in `crates/temper-cli/src/actions/sync.rs:1494-1504`
correctly resolves `@me` → `@<profile.slug>` for the on-disk path and the manifest entry,
matching the server's canonical URI form. But the frontmatter inside the file is written
by `build_frontmatter_from_resource` (`crates/temper-cli/src/actions/ingest.rs:516-521`),
which uses `resource.owner_handle` raw — i.e. the literal `"@me"` shorthand. On the next
sync, `preflight_ownership_check` (`sync.rs:120-158`) reads:

- `manifest_owner` from `Vault::parse_rel(&entry.path)` → `"@j-cole-taylor"`
- `frontmatter_owner` from the file's `temper-owner` field → `"@me"`

String comparison fails → mismatch reported.

## Goals

1. Eliminate the spurious warning for the 7 currently-affected files and any future
   `NewlyTracked` pulls.
2. Prevent the same class of bug from recurring via symmetric defenses (write-side and
   read-side both canonicalize), mirroring the Phase 5 managed-meta pattern documented
   in `temper/CLAUDE.md`.
3. No vault migration. Pre-existing files at `@me/temper/...` paths stay where they are
   and continue to work — they're not the source of this warning.

## Non-Goals

- Migrating existing `@me/<context>/<doc_type>/...` vault files to `@<profile.slug>/...`
  paths. (Option C in brainstorm; declined as YAGNI.)
- Caching profile slug across sync runs. The current per-run fetch is acceptable.
- Auditing other frontmatter writers outside `build_frontmatter_from_resource` for
  similar drift. If any exist, they surface as separate follow-ups.

## Canonical Form (Reference)

| Surface | Form for user's own resource | Source |
|---|---|---|
| Server `kb_resource_uri()` | `kb://@<profile.slug>/...` | migrations/20260407000002_owner_scoped_uris.sql:52-54 |
| Server `owner_handle` field on `ResourceRow` | `"@me"` (display alias) | resource_service.rs OWNER_HANDLE_EXPR |
| Manifest entry `path` | `@<profile.slug>/<ctx>/<type>/<slug>.md` | sync_refresh + PR #70 NewlyTracked |
| File `temper-owner` frontmatter (after fix) | `@<profile.slug>` | this design |
| File `temper-owner` frontmatter (legacy) | `@me` | tolerated on read |
| Team-owned resource | `+<team-slug>` everywhere | unchanged |

## Design

### 1. New helper: `resolve_owner_for_frontmatter`

Location: `crates/temper-cli/src/actions/sync.rs`.

```rust
/// Resolve the API's `owner_handle` shorthand to the canonical owner sigil
/// used in vault paths and `kb_resource_uri()`. The API returns literal
/// `"@me"` for the requester's own resources; everywhere on the storage and
/// URI side, the user's own owner segment is `@<profile.slug>`. Team handles
/// (`+<team-slug>`) are already canonical and pass through.
pub fn resolve_owner_for_frontmatter(handle: &str, profile_slug: &str) -> String {
    if handle == "@me" {
        format!("@{profile_slug}")
    } else {
        handle.to_string()
    }
}
```

Pure, no async, no client. Three unit tests (see Tests).

### 2. Write side: `build_frontmatter_from_resource` accepts a resolved owner

Location: `crates/temper-cli/src/actions/ingest.rs:485`.

Current signature:
```rust
pub fn build_frontmatter_from_resource(
    resource: &temper_core::types::ResourceRow,
    context: &str,
    doc_type: &str,
    body: String,
    managed_meta: Option<&serde_json::Value>,
    open_meta: Option<&serde_json::Value>,
) -> crate::error::Result<temper_core::frontmatter::Frontmatter>
```

New signature: add `canonical_owner: &str` immediately after `doc_type`. Inside, replace:

```rust
if !resource.owner_handle.is_empty() {
    fm.set_managed_field(
        "temper-owner",
        serde_json::Value::String(resource.owner_handle.clone()),
    );
}
```

with:

```rust
if !canonical_owner.is_empty() {
    fm.set_managed_field(
        "temper-owner",
        serde_json::Value::String(canonical_owner.to_string()),
    );
}
```

The function no longer reads `resource.owner_handle` — the caller is responsible for
passing a canonical value. This pushes the policy decision (resolve vs. pass through)
to the boundary where the client/profile is already in scope.

#### Caller updates

All three production call sites in `sync.rs` resolve via `resolve_owner_for_frontmatter`
before calling. The NewlyTracked branch already computes this string for the path;
reuse it directly.

| Site | File | Notes |
|---|---|---|
| 1 | `sync.rs:1440` (ManifestTracked branch) | Resolve at call site via the helper. |
| 2 | `sync.rs:1523` (NewlyTracked branch) | Reuse the existing `owner` local from line 1500-1504. |
| 3 | `sync.rs:1645` (`apply_pull_meta_only`) | Resolve at call site via the helper. |

Five existing test call sites in `ingest.rs` (lines 1187, 1230, 1277, 1317, 1464)
currently rely on the function reading `resource.owner_handle`. Each needs to pass an
explicit `canonical_owner` argument — for the existing test fixtures (which use `@me`
as the test handle), passing `"@test-user"` or `"@me"` directly is fine; we are not
changing what those tests assert, only adapting them to the new signature.

### 3. Read side: `preflight_ownership_check` accepts current owner slug

Location: `crates/temper-cli/src/actions/sync.rs:120-158`.

Current signature:
```rust
pub fn preflight_ownership_check(
    manifest: &Manifest,
    vault_root: &Path,
) -> Vec<OwnershipMismatch>
```

New signature: add `current_owner_slug: &str` (without the `@` prefix; matches the
shape of `profile.slug`).

Equivalence rule: before the `frontmatter_owner != manifest_owner` check, if
`frontmatter_owner == "@me"`, treat it as `format!("@{current_owner_slug}")` for the
comparison. All other values (other users, team handles) compare as before.

```rust
let frontmatter_owner_canonical = if frontmatter_owner == "@me" {
    format!("@{current_owner_slug}")
} else {
    frontmatter_owner.clone()
};

if frontmatter_owner_canonical != manifest_owner {
    mismatches.push(OwnershipMismatch {
        file_path: entry.path.clone(),
        frontmatter_owner,        // report the raw value the user sees in the file
        manifest_owner,
    });
}
```

Note: when emitting `OwnershipMismatch`, keep the raw `frontmatter_owner` so any
*genuine* mismatch error message still shows the user the literal value in the file.

### 4. Plumbing: `sync_cmd.rs` orders profile fetch before preflight

Location: `crates/temper-cli/src/commands/sync_cmd.rs:53-103`.

Current order:
1. Resolve vault root, load manifest, normalize entries
2. **Run preflight** (line 72) — purely local, no network
3. Build runtime/client (line 100)
4. `ensure_profile` (line 103) — first network call
5. Run sync orchestration (line 105)

New order:
1. Resolve vault root, load manifest, normalize entries
2. Build runtime/client
3. `ensure_profile` → returns the resolved `Profile` so the slug is in scope
4. Run preflight, passing `&profile.slug`
5. Run sync orchestration

Preflight gains a network dependency, but every other branch of this command already
requires connectivity, so users are not losing offline functionality.

#### `ensure_profile` returns `Profile`

Location: `crates/temper-cli/src/actions/runtime.rs:114-121`.

Current:
```rust
pub async fn ensure_profile(client: &temper_client::TemperClient) -> Result<()> {
    client.profile().get().await
        .map_err(|e| TemperError::Api(format!("profile pre-flight: {e}")))?;
    Ok(())
}
```

New:
```rust
pub async fn ensure_profile(client: &temper_client::TemperClient)
    -> Result<temper_core::types::Profile>
{
    client.profile().get().await
        .map_err(|e| TemperError::Api(format!("profile pre-flight: {e}")))
}
```

Other callers of `ensure_profile` that don't need the return value (`add.rs:134, 217,
404, 650, 706` and `sync_cmd.rs:284, 338`) replace `?;` with `?; let _ = …` or simply
discard via `let _ = rt.block_on(...)?;`. No behavioral change for them.

This is a single-line ergonomic improvement, not feature creep — the alternative is
calling `client.profile().get()` a second time before preflight, which is wasteful.

## Tests

TDD red-green for each change. All tests follow the existing patterns in their
respective files.

### Unit — ingest.rs

| Test | Setup | Assertion |
|---|---|---|
| `build_frontmatter_from_resource_writes_canonical_owner_for_at_me` | `ResourceRow.owner_handle = "@me"`, `canonical_owner = "@j-cole-taylor"` | `fm.value()["temper-owner"] == "@j-cole-taylor"` |
| `build_frontmatter_from_resource_passes_team_handle_through` | `canonical_owner = "+platform-eng"` | `fm.value()["temper-owner"] == "+platform-eng"` |

### Unit — sync.rs

| Test | Setup | Assertion |
|---|---|---|
| `resolve_owner_for_frontmatter_resolves_at_me` | `("@me", "j-cole-taylor")` | `== "@j-cole-taylor"` |
| `resolve_owner_for_frontmatter_passes_through_team_handle` | `("+platform-eng", "j-cole-taylor")` | `== "+platform-eng"` |
| `resolve_owner_for_frontmatter_passes_through_other_user` | `("@some-other-user", "j-cole-taylor")` | `== "@some-other-user"` |
| `preflight_ownership_check_treats_at_me_as_current_owner_alias` | manifest path `@j-cole-taylor/temper/task/x.md`, file frontmatter `temper-owner: '@me'`, `current_owner_slug = "j-cole-taylor"` | `mismatches.is_empty()` |
| `preflight_ownership_check_flags_other_owner_mismatch` | manifest path `@j-cole-taylor/...`, frontmatter `temper-owner: '@some-other-user'`, `current_owner_slug = "j-cole-taylor"` | one mismatch reported, `frontmatter_owner == "@some-other-user"` |
| `preflight_ownership_check_flags_team_handle_mismatch_when_path_is_personal` | manifest `@j-cole-taylor/...`, frontmatter `+platform-eng`, current `j-cole-taylor` | flagged (defends against over-tolerance) |

### E2E regression — pull_command_test.rs

Add a sibling test next to PR #70's
`pull_one_resource_with_manifest_but_untracked_id_writes_canonical_layout` (line 277):

- Same NewlyTracked pull setup.
- After the pull, read the file and assert `temper-owner` matches the canonical
  `@<profile.slug>` form.
- Then call `preflight_ownership_check` with the test profile's slug and assert no
  mismatches are reported. This locks in the round-trip invariant for the bug
  reported in this design.

## Self-Heal Behavior for Existing 7 Files

After this fix lands:

- The next `temper sync run` no longer emits the warning for these files (read-side
  tolerance treats their `@me` frontmatter as equivalent to `@j-cole-taylor`).
- Their frontmatter still contains `temper-owner: '@me'` until any pull-after-edit
  cycle re-invokes `build_frontmatter_from_resource`, at which point it gets rewritten
  to `temper-owner: '@j-cole-taylor'`.
- Files that are never re-pulled keep `@me` in frontmatter indefinitely. This is
  harmless because of the read-side tolerance, and matches the migration philosophy
  established by Phase 5 ("pre-existing files without these fields stay valid until
  their next round-trip; new writes never produce them").

If the user wants to force-rewrite immediately, they can run `temper resource update
<slug> --type <type>` on each (touches the file → triggers re-publish on next sync) or
delete the local copy and let `sync_refresh` re-pull. Neither is required.

## Risks

- **`build_frontmatter_from_resource` signature change**: a public function (already
  imported across `sync.rs`). Compiler enforces all call sites are updated.
- **`preflight_ownership_check` signature change**: only one caller (`sync_cmd.rs:72`).
  Compiler catches any new call site that misses the parameter.
- **`ensure_profile` return type change**: ~7 call sites; each either needs the
  profile or can discard via `let _ = …`. Compiler catches misuse.
- **Profile fetch ordering in `sync_cmd.rs`**: moves a network call before what was
  previously a local-only step. Acceptable because every other branch of `sync run`
  already requires the network.

## Acceptance Criteria

1. The seven files listed in the bug report no longer produce ownership-mismatch
   warnings on `temper sync run`.
2. A genuine ownership mismatch (different user, or wrong team handle on a personal
   path) is still reported.
3. New `NewlyTracked` pulls produce frontmatter with `temper-owner: '@<profile.slug>'`
   from the start.
4. All existing unit, integration, and e2e tests still pass.
5. New unit and e2e tests pass in both `cargo make test` and the embed-gated
   `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`.
6. `cargo make check` passes (clippy + format + machete).
