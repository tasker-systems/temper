# Cloud-Only Sync Handling, `FindableResource` Refactor, and Owner Canonicalization Reversal

**Status:** Draft for plan-writing
**Date:** 2026-05-10
**Goal:** `path-to-alpha`
**Predecessor PRs (informing this design):**
- PR #70 (`fix(sync): pull untracked ids into canonical layout, not vault root`)
- PR #72 (`fix(sync): canonicalize @me owner across pull frontmatter and preflight`)
- PR #73 (audit sweep — surfaced C.1)

**Sibling task:** `2026-05-09-thread-owner-through-build-vault-path-audit-b-2-followup` (parallel session — touches `commands/add.rs`)

## Summary

Five user-visible bugs converge on two underlying causes:

1. **Inconsistent owner resolution across surfaces.** PR #72 introduced `OwnerResolver` for pull-side canonicalization but in the wrong direction relative to design intent: it canonicalizes `@me` → `@<profile.slug>` for pulled frontmatter and (via PR #70) NewlyTracked vault paths. The intent is the opposite — `@me/` is the canonical local-vault directory for the user's own work, and `@<other-slug>/` is reserved for team-shared contexts. The wrong-direction canonicalization scattered some files under `@<profile.slug>/` while older files remain at `@me/`, and CLI lookup paths only check one of the two.

2. **Sync state machine has no "tracked-but-missing" representation.** `normalize_all_entries` shoehorns a missing local file into `LocalModified`, which routes it onto the push path with an empty `body_hash`. The push fails late, with a confusing error and a recovery suggestion (`temper sync refresh`) that doesn't actually pull missing files.

This design also addresses **C.1** (a small but persistent lookup bug surfaced in the 2026-05-09 audit sweep): `find_resource_file` runs `vault::slugify(slug)` on its input, collapsing `--` to `-` and silently failing on slugs with literal double-hyphens.

After this work ships:
- `temper resource show <slug>` falls back to the API in local mode when the local file is missing.
- `temper sync run` reclassifies missing-but-tracked files as `LocallyMissing` and routes them to the pull set.
- All write paths (create, pull, NewlyTracked) write own-resource files to `@me/<ctx>/<doctype>/...` and own-resource frontmatter as `temper-owner: '@me'`.
- A new `FindableResource` lookup type formalizes ownership, replaces the stringly-typed `doc_type: &str` with the typed `DocType` enum, and handles slug→`temper-id` (and `temper-provisional-id`) resolution from the manifest.
- The pre-existing `owners_equivalent` tolerance (PR #72) remains in place so legacy `@<slug>/`-resident files from the PR #70/72 window stay reachable without a vault migration.

## Scope

In scope:
- New module `crates/temper-cli/src/lookup.rs` with `FindableResource`, `ResolvedResource`, and `find_resource()`.
- Migration of all `find_resource_file` callers in `crates/temper-cli/src/commands/resource.rs` (and `task::show` / `session::show`) to the new lookup type.
- Retirement of the stringly-typed `VALID_DOC_TYPES` const + `validate_doc_type(&str)` in favor of `temper_core::frontmatter::DocType` parsed at the clap boundary.
- New manifest state `ManifestEntryState::LocallyMissing` in `temper-core/src/sync/manifest.rs`.
- Sync rehash + orchestration changes so `LocallyMissing` is set when the file is gone and joins the pull set.
- Reversal of PR #72's pull-frontmatter direction and PR #70's NewlyTracked path resolution: own-resource writes land at `@me/...` with `temper-owner: '@me'`.
- Audit + correction of own-resource owner strings in create paths (`commands/add.rs`, `actions/ingest.rs`) — coordinated with the sibling B.2 session.
- Update `vault_file_missing_err` message and `CLAUDE.md` two-pronged guidance.
- API fallback in `task::show` / `session::show` / `show_generic` when local lookup fails in local mode.
- Regression tests per symptom + C.1 + lookup contract.

Out of scope:
- Vault layout migration. The user's design intent is to ground the on-disk vault in `@me/` for private work and `@<other-slug>/` for team-shared contexts. Files left at `@<profile.slug>/` from the PR #70/72 window stay there — `owners_equivalent` plus the lookup's legacy-fallback directory keep them reachable. No bulk move.
- Phase 6's per-resource state machine (`Local`, `Synced`, `Conflicting`, `PendingPush`). `LocallyMissing` is a transitional Wave-1-bounded state. The Phase 6 plan must enumerate it and decide its mapping (collapse into `Synced` with a recovery flag, or keep as a discrete state).
- Per-resource recovery command (`temper resource pull <slug>`). Deferred — `temper sync run` is sufficient after the classifier fix.
- Other stringly-typed CLI surfaces beyond `doc_type`. Captured as a follow-up; not done here to keep scope tight.
- `temper sync refresh` semantics changes. Refresh continues to be "non-destructive manifest rebuild from server"; recovery is `sync run`.

## Architecture

### `FindableResource` lookup type

New module `crates/temper-cli/src/lookup.rs`:

```rust
use temper_core::frontmatter::DocType;
use temper_core::types::ids::ResourceId;
use temper_core::sync::Manifest;

pub struct FindableResource<'a> {
    pub config: &'a Config,
    pub manifest: Option<&'a Manifest>,  // for slug→temper-id mapping incl. provisional ids
    pub owner: Option<String>,           // None → @me canonical
    pub context: Option<String>,         // None → scan all configured contexts
    pub doc_type: DocType,
    pub slug_or_suffix: String,          // raw — no slugify
}

pub struct ResolvedResource {
    pub path: PathBuf,
    pub context: String,
    pub owner: String,
    pub doc_type: DocType,
    pub resource_id: Option<ResourceId>,    // resolved from frontmatter or manifest
    pub provisional_id: Option<String>,     // fallback for unsynced files
}

pub fn find_resource(req: FindableResource<'_>) -> Result<ResolvedResource>;
```

**Lookup logic:**

1. Owner = `req.owner.unwrap_or_else(|| "@me".into())`.
2. Contexts to scan = `req.context.map(|c| vec![c]).unwrap_or_else(|| config.contexts.clone())`.
3. For each context, scan `<owner>/<ctx>/<doctype.as_str()>/` directory. If owner is `@me`, additionally scan `@<profile.slug>/<ctx>/<doctype>/` as legacy fallback for PR #70/72-era files.
4. Within each scanned directory: raw equality on stem → equality on slug-portion-after-date-prefix (`YYYY-MM-DD-<slug>` → `<slug>`) → suffix match. **No `slugify`** anywhere — the on-disk filename is canonical and the input is matched against it byte-for-byte. Mirrors `actions::task::find_task`. **This is the C.1 fix.**
5. If `req.manifest` is provided and a match is found, populate `resource_id` from frontmatter `temper-id` (or, if absent, from the manifest's slug→id index) and `provisional_id` from `temper-provisional-id` for unsynced files.
6. Multiple matches: prefer the most recently-prefixed file (existing behavior); ambiguous suffix-only matches return an error listing candidates (mirrors `find_task`).

**Boundary parsing of `doc_type`:** clap derives parse the user-facing string into `DocType` once at the CLI argument boundary using `DocType::from_str(s)?`. The existing `VALID_DOC_TYPES` const + `validate_doc_type(&str)` pair in `crates/temper-cli/src/commands/resource.rs:19-29` are removed. Internal CLI APIs operate on `DocType` from there inward.

### `ManifestEntryState::LocallyMissing`

New variant in `crates/temper-core/src/sync/manifest.rs`:

```rust
pub enum ManifestEntryState {
    Synced,
    LocalCreated,
    LocalModified,
    LocallyMissing,   // new: manifest knows about this file, vault file is gone
    // ... existing variants
}
```

**Set by:** `normalize_all_entries` in `crates/temper-cli/src/actions/sync.rs:365-378` when the file is missing on disk. The previous behavior (`LocalModified` + cleared `body_hash`) is replaced. `body_hash` is preserved so that the server-diff phase can still compare hashes against the server's view if needed.

**Consumed by:** sync orchestration's push-set and pull-set builders. `LocallyMissing` entries are excluded from the push set and added to the pull set with no special-cased download logic — the existing `pull_one_resource` primitive handles the body fetch + write. `temper sync run` output line is a regular `↓ Pull <path>`; no distinct glyph.

**Phase 6 hand-off:** when the Phase 6 plan is written, it must enumerate `LocallyMissing` as input and decide its placement in the per-resource state machine. Likely options: (a) collapse into `Synced` with a transient "recovery" flag, or (b) keep as a discrete state with documented transitions.

### Owner canonicalization direction reversal

**Design intent (clarified during brainstorming):** the on-disk vault is grounded in `@me/` for private work; `@<other-slug>/` directories represent team-shared contexts. PR #70 and PR #72 went the wrong direction: they canonicalize own-resource pulls to `@<profile.slug>/...` with `temper-owner: '@<profile.slug>'` in frontmatter. This design reverses that.

**Reversal points:**

- `build_frontmatter_from_resource` (called from `pull_one_resource` and the NewlyTracked branch): the `canonical_owner: &str` parameter introduced in PR #72 changes meaning. For own resources (where the API's `owner_handle` is `@me` or matches `profile.slug`), the resolved value is now `@me`, not `@<profile.slug>`. For other-owner resources (handle is `@<other-slug>`), the resolved value is the actual `@<other-slug>` handle. Callers update their resolution helper accordingly; the function signature does not need to change.
- NewlyTracked path resolution in `pull_one_resource_newly_tracked_*` and around `sync.rs:561` (`Vault::canonical_uri(parsed.owner, ...)`): own resources resolve to `@me/<ctx>/<doctype>/<file>.md`, not `@<profile.slug>/...`.
- `OwnerResolver` (sync.rs:227) is preserved as the API-shorthand-to-explicit-handle resolver but its return value for "own user" becomes `@me`, not `@<profile.slug>`. The cached `profile.slug` remains accessible for code paths that need the explicit slug (e.g., constructing API request paths where the API requires the explicit owner).
- Create paths: audit `commands/add.rs` and `actions/ingest.rs` for any `temper-owner` writes that emit `@<profile.slug>` for own resources; correct to `@me`. **Coordination point with the sibling B.2 session** which is also touching `commands/add.rs` to thread `owner` through `build_vault_path` — the two changes are complementary; we rebase or they rebase, depending on landing order.
- `owners_equivalent` (sync.rs:121) is **kept** — it is now load-bearing for the legacy direction. Files at `@<profile.slug>/` with `temper-owner: '@<profile.slug>'` from the PR #70/72 window remain valid; the symmetric tolerance keeps them passing preflight.

**Why no vault migration:** rewriting `@<profile.slug>/` to `@me/` would touch every file landed during the PR #70/72 window, change manifest paths, and risk corrupting in-flight syncs. The `owners_equivalent` + lookup-with-legacy-fallback approach makes the dual form coexist cleanly.

### Resource show API fallback

`crates/temper-cli/src/commands/{task,session,resource}.rs::show` — when `find_resource` returns `NotFound` and `vault_state == VaultState::Local`:

1. Build `(@me, ctx, doc_type, slug_or_suffix)` and call `client.resources().resolve_by_uri(...)` (the existing slow-path used in cloud mode).
2. On success: `client.resources().get(id)` for the body and render in the requested format.
3. On client error (network failure, auth failure, server 404): emit a clearer error than the current "task not found" — distinguish the offline case (`couldn't reach server to verify resource exists; offline lookup failed for <slug>`) from the genuine-not-found case (`<doctype> not found locally or on server: <slug>`).

The fallback is invisible to the user when it succeeds — the same rendered output as the local path. No write to disk, no manifest mutation. The vault stays untouched; recovery to disk happens via `temper sync run`.

## Data Flow

### Sync run (after fix)

```
temper sync run
  │
  ├─ rehash phase (normalize_all_entries)
  │    for each manifest entry:
  │      if file present     → existing normalize logic, sets state appropriately
  │      if file missing     → state = LocallyMissing, body_hash preserved
  │
  ├─ scan-untracked phase    → unchanged; finds new local files not in manifest
  │
  ├─ server manifest fetch   → unchanged
  │
  ├─ diff classification
  │    push set ← LocalModified ∪ LocalCreated
  │    pull set ← server-newer ∪ LocallyMissing
  │
  ├─ pull execution
  │    for each pull entry:
  │      resolve canonical local path:
  │        own resource (handle == @me or == profile.slug) → @me/<ctx>/<doctype>/<file>.md
  │        other-owner resource (@<other-slug>)            → @<other-slug>/<ctx>/<doctype>/...
  │      write file via pull_one_resource
  │      frontmatter temper-owner: same rule (@me for own, @<other-slug> for other)
  │
  └─ push execution           → unchanged for genuine LocalModified / LocalCreated
                                 missing files no longer reach this phase
```

### Resource show (after fix, local mode)

```
temper resource show <slug> --type <T> [--context <ctx>]
  │
  ├─ DocType::from_str(<T>)? at clap boundary
  │
  ├─ find_resource(FindableResource {
  │     config,
  │     manifest: load_manifest_or_skip(),
  │     owner: None,                  // → @me
  │     context: <ctx-or-None>,
  │     doc_type: <DocType>,
  │     slug_or_suffix: <slug>,
  │   })
  │     ├─ Found (@me/ or legacy @<slug>/) → read file + render (current path)
  │     │
  │     └─ NotFound + VaultState::Local:
  │           client.resources().resolve_by_uri(@me, ctx, doctype, slug)
  │             ├─ Ok(id)             → client.resources().get(id) → render
  │             ├─ NetworkErr         → "couldn't reach server …"
  │             └─ NotFound on server → "<doctype> not found locally or on server: <slug>"
```

### Resource create (after fix)

```
temper resource create --type <T> --title <Y> --context <ctx>
  │
  ├─ DocType::from_str(<T>)?
  │
  ├─ build_ingest_payload — managed_meta with temper-owner: '@me' (own context)
  │
  ├─ Local mode:                                  Cloud mode:
  │    write to @me/<ctx>/<doctype>/<slug>.md       POST /api/ingest
  │    push to API as tail action                   (server uses Auth0 sub → profile,
  │                                                  stores resource owned by user)
  │                                                 no local file written
```

The create-path audit step in PR plan ensures every owner-string in the local-mode write is `@me`, not `@<profile.slug>`. B.2 sibling session may have already corrected most sites; verify against the bug task's affected slug.

## Error Handling & UX

**`vault_file_missing_err` (sync.rs:30):**

After this fix lands, the rehash-time-missing case is reclassified as `LocallyMissing` and never reaches push. Keep the helper for the residual race case (file present at scan, vanishes before push reads it). New message body:

```
vault file vanished during sync at {rel_path}; run `temper sync run` to recover
```

The two-pronged guidance (delete vs refresh) is removed — both branches were misleading after the classifier fix.

**`CLAUDE.md` update:**

Find the paragraph beginning `"There is no implicit-delete-via-`rm` path"`. Rewrite to:

```
There is no implicit-delete-via-`rm` path. To delete a resource, run
`temper resource delete <slug>`. To recover a file you removed by accident
(or that's missing on a fresh device), just run `temper sync run` — the next
sync cycle reclassifies missing-but-tracked files as `LocallyMissing` and
pulls them back. `temper sync refresh` is for manifest rebuilds and does not
pull missing files; do not use it for recovery.
```

**`temper resource show` errors:**

| Situation | Error |
|---|---|
| Local lookup miss, API has it, render succeeds | (no error — silent fallback) |
| Local lookup miss, API auth/network failure | `couldn't reach server to verify resource exists; offline lookup failed for <slug>` |
| Local lookup miss, API confirms not-found | `<doctype> not found locally or on server: <slug>` |
| Local lookup miss, cloud mode (no fallback needed — already API-only) | unchanged |

**`temper sync run` output:**

`LocallyMissing` entries appear under the regular `↓ Pull <path>` line. The phase summary's `pull: N` count includes them. No new glyph or summary section.

**Logs:**

- `LocallyMissing` reclassification: `tracing::info!` with the path. Normal recovery, not a warning.
- Owner canonicalization mismatch (legacy `@<slug>/` files): `tracing::debug!` only. Permanent tolerance via `owners_equivalent`.

## Testing

| ID | Test name | Layer | Asserts |
|---|---|---|---|
| 1a | `resource_show_falls_back_to_api_when_local_missing` | e2e | API-only resource renders via `temper resource show` in local mode |
| 1b | `resource_show_errors_clearly_when_offline_and_local_missing` | unit (cli) | Network failure produces offline-specific error message |
| 2a | `rehash_marks_missing_file_as_locally_missing` | unit (sync.rs) | Missing file → state `LocallyMissing`, body_hash preserved |
| 2b | `sync_run_pulls_locally_missing_entries` | e2e | Removed local file reappears after `temper sync run`; push set empty |
| 3 | `vault_file_missing_err_points_to_sync_run` | unit | Snapshot of new error wording |
| 4a | `pull_writes_own_resource_to_at_me_directory` | unit (sync.rs) | API `owner_handle: @me` → file at `@me/...`, frontmatter `@me` |
| 4b | `resource_create_writes_own_resource_to_at_me_directory` | unit (commands/add) | Local-mode create writes under `@me/...` regardless of profile slug |
| 4c | `find_resource_falls_back_to_legacy_slug_directory` | unit (lookup.rs) | Legacy `@<profile.slug>/<ctx>/...` file is found by `find_resource` when owner is `None` |
| C.1 | `find_resource_matches_double_hyphen_slug` | unit (lookup.rs) | Slug `foo--bar` matches file `foo--bar.md` (regression: prior slugify collapsed it) |
| Lookup-1 | `find_resource_resolves_resource_id_from_manifest` | unit (lookup.rs) | `ResolvedResource.resource_id` populated from manifest; `provisional_id` populated when only that is present |
| Lookup-2 | `find_resource_defaults_to_at_me_when_owner_none` | unit (lookup.rs) | `FindableResource { owner: None, ... }` searches `@me/` first |

**Workspace verification** (per `feedback_workspace_test_surfaces_pipeline_bugs`): the plan's verification step must run `cargo nextest run --workspace` to surface any feature-unification surprises, even though the fix is CLI-side.

**No `test-embed` gating:** none of these tests touch the embed pipeline. `--features test-db` only.

## Decomposition

**One PR**, internally split into two distinct work sets that share verification at the end. The work sets share enough conceptual surface (lookup correctness across the dual `@me/`-vs-`@<slug>/` reality) to be reviewed together; system-level value is closely linked.

### Work Set A — `FindableResource` refactor + C.1 + show fallback

Pure read-side improvements + the lookup type. No manifest state changes, no sync-orchestration changes.

1. Add `FindableResource` + `ResolvedResource` + `find_resource` in `temper-cli/src/lookup.rs`. Tests for owner default, doc_type typing, manifest resolution, double-hyphen-slug regression (C.1). Stringly-typed `validate_doc_type` retired in favor of `DocType::from_str` at clap boundary.
2. Migrate all `find_resource_file` callers in `commands/resource.rs` to `find_resource`. Mechanical refactor.
3. Add `@me` + `@<profile.slug>` legacy-fallback directory scan in `find_resource`. Test with mixed-form fixture.
4. `task::show` / `session::show` / `show_generic` API fallback when local lookup fails in local mode. Add `client.resources().get` plumbing if absent.
5. Verify Work Set A: `cargo make check`, `cargo make test`, targeted lookup unit tests pass.

### Work Set B — `LocallyMissing` state + sync classifier + owner direction reversal

State-machine and write-path changes. Depends on Work Set A's lookup tolerance for legacy file resolution.

6. Add `ManifestEntryState::LocallyMissing` variant in `temper-core/src/sync/manifest.rs`. Test serde round-trip.
7. `normalize_all_entries` sets `LocallyMissing` for missing files (preserves `body_hash`). Unit test.
8. Sync orchestration: `LocallyMissing` joins pull set, skips push set. Update push/pull set builders. Unit + e2e tests.
9. Reverse PR #72: `build_frontmatter_from_resource` + call sites write `@me` for own resources. Update unit tests.
10. Reverse PR #70 NewlyTracked path resolution: own-resource pulls land at `@me/<ctx>/<doctype>/...`. Update unit tests.
11. Audit create paths (`commands/add.rs`, `actions/ingest.rs`) for own-resource owner-string sites. Coordinate with the B.2 sibling session — rebase order determined at landing time.
12. Update `vault_file_missing_err` message + `CLAUDE.md` two-pronged guidance.
13. Final verification: `cargo make check`, `cargo make test-all`, `cargo nextest run --workspace`, embed-gated CI tier (Embed CI job is the only place ONNX runtime is wired up; per `feedback_workspace_test_surfaces_pipeline_bugs`, run with `--features test-db,test-embed` against `tests/e2e/Cargo.toml` if local).

### Out-of-scope follow-ups (capture as task stubs)

- Phase 6 plan must enumerate `LocallyMissing` and decide its mapping into the per-resource state machine.
- `temper resource pull <slug>` per-resource recovery command — deferred (decision #3 — `sync run` is sufficient).
- `temper-cli` stringly-typed sweep beyond `doc_type` — audit other places that should use enums (audit-followups territory).
- Decide whether `OwnerResolver` is still needed after the direction reversal, or whether its responsibilities collapse into the cached `profile.slug` + `@me` constant.
