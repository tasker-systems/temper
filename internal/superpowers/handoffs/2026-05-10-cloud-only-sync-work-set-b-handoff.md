# Handoff — Cloud-only sync fix, Work Set B

**For:** the next session resuming `jct/cloud-only-sync-and-find-resource`.
**Date written:** 2026-05-10
**Branch (pushed):** `jct/cloud-only-sync-and-find-resource`

## TL;DR

Mid-implementation pause after Work Set A. The branch holds 10 commits ahead of main: 2 docs (spec + plan) and 8 implementation commits. All Work Set A tasks (1-7 + a follow-up wiring commit) shipped with green tests and lint. Eight tasks remain in Work Set B — five inline, three subagent. PR not yet opened; user is reviewing the branch visually first.

## Where to start

Read in order:

1. **Spec:** `docs/superpowers/specs/2026-05-10-cloud-only-sync-and-find-resource-design.md` — the architecture and decisions.
2. **Plan:** `docs/superpowers/plans/2026-05-10-cloud-only-sync-and-find-resource.md` — 15-task implementation plan with concrete code and verification per task.
3. **Session note:** `@me/temper/session/2026-05-10-2026-05-10-cloud-only-sync-bug-brainstorm-spec-plan-work-set-a-landed.md` — what happened, full decision log.
4. **This handoff** — what's next.

Then check the branch state:

```bash
git checkout jct/cloud-only-sync-and-find-resource
git log --oneline origin/main..HEAD
```

## Work Set A — landed (commits, top-down)

| Commit | Purpose |
|---|---|
| `61f17a2` | profile_slug `OnceLock` cache + `ensure_profile` wires it; manifest plumbing at delete cleanup site (follow-up after review) |
| `0c043fa` | API fallback in `temper resource show` (subagent — Task 7) |
| `72e9c7d` | Retire stringly-typed `validate_doc_type` (Task 6) |
| `00ee22c` | Migrate `find_resource_file` callers to `find_resource` (Task 5) |
| `15b2a4a` | Legacy `@<profile.slug>/` directory fallback + `Config::profile_slug` field (Task 4) |
| `d957236` | Manifest-aware id resolution in `find_resource` (Task 3) |
| `4088b67` | `find_resource` matching algorithm + C.1 regression test (Task 2) |
| `54c0955` | `lookup.rs` module skeleton (Task 1) |

Spec + plan commits (`42983c9`, `85d6452`) live on main if the user pushed during the session — verify with `git log --oneline main..HEAD` vs `git log --oneline origin/main..HEAD`.

**Net effect:**
- `crates/temper-cli/src/lookup.rs` exists. `FindableResource` lookup type with typed `DocType`, optional owner (defaults to `@me`), manifest awareness, legacy `@<profile.slug>/` directory fallback driven by `Config::profile_slug` or the new `OnceLock` cache populated by `ensure_profile`.
- C.1 fixed by construction (no slugify on lookup input).
- `temper resource show` falls back to API in local mode for cloud-only resources via `show_via_api_fallback` (in `commands/resource.rs:947`), called from `task::show`, `session::show`, and `show_generic`.
- Stringly-typed `validate_doc_type` retired; `DocType::from_str` is the single source of truth at validation boundaries.

**Verification (Work Set A):**
- `cargo nextest run -p temper-cli` — 474 tests pass.
- `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db resource_show_falls_back_to_api_when_local_missing` — passes.
- `cargo make check` — fmt/clippy/docs/biome all clean.

## Work Set B — 8 tasks remaining

State machine, sync orchestration, and owner-direction reversal. **This is the riskier half** — it touches existing PR #72/#70 tests that need rewriting and the live sync orchestration code path.

### Inline tasks (5)

**Task 8 — Add `ManifestEntryState::LocallyMissing` variant.**
File: `crates/temper-core/src/types/manifest.rs` (lines 11-22).
Add the new variant + extend the existing `test_manifest_entry_state_serde` test. ~5 minutes. See plan Task 8 for concrete code.

**Task 9 — `rehash_manifest` sets `LocallyMissing` for missing files.**
File: `crates/temper-cli/src/actions/sync.rs` (lines 469-477).
Replace the existing missing-file branch that clears `body_hash` and sets `LocalModified`. New behavior: preserves hashes (server-diff still has them to compare), sets `LocallyMissing`. Unit test gating both the state and hash preservation.

**Task 10 — `normalize_all_entries` sets `LocallyMissing` for missing files.**
File: `crates/temper-cli/src/actions/sync.rs` (lines 365-378).
Same pattern as Task 9 in the parallel function. Plan has both diff blocks side by side.

**Task 14 — Update `vault_file_missing_err` + CLAUDE.md guidance.**
After Tasks 9-11 land, the rehash-time-missing case is unreachable. Helper covers only the residual race case. New message points at `temper sync run`. CLAUDE.md's deletion paragraph (around line 86-95) rewrites to drop the misleading two-pronged guidance.

**Task 15 — Final verification.**
Full `cargo make test-all`, workspace nextest, embed-gated e2e tier. Manual smoke test if a real vault is handy.

### Subagent tasks (3) — highest risk, dispatch carefully

**Task 11 — Sync orchestration routes `LocallyMissing` to pull set.**
The post-diff routing logic at `sync.rs:780-908`. Drop `to_push` items whose manifest entry is `LocallyMissing`; synthesize `SyncPullItem` for missing entries that aren't already in `to_pull`. **Verify `SyncPullItem` field names** (`resource_id`, `uri`, `kind`, `content_hash`) before generating code — plan note flags this. New e2e test `locally_missing_recovery_test.rs` asserts remove-then-sync-run restores the file with push count zero.

**Task 12 — Reverse PR #72 — `@me` is canonical for own resources.**
Changes `resolve_owner_for_frontmatter` (sync.rs:218) and `OwnerResolver::resolve` (sync.rs:264) to return `@me` for the API's `@me` shorthand AND for explicit `@<profile.slug>` references to the same user. Existing PR #72 tests at sync.rs:3281-3299 need rewriting; the `pull_one_resource_newly_tracked_writes_canonical_owner_and_passes_preflight` e2e test needs its path assertion flipped from `@<slug>/` to `@me/`.

**Task 13 — Audit create paths + B.2 coordination.**
Files: `crates/temper-cli/src/commands/add.rs`, `crates/temper-cli/src/actions/ingest.rs`. Ensure every own-resource owner-string write emits `@me`, not `@<profile.slug>`. The sibling B.2 session (`2026-05-09-thread-owner-through-build-vault-path-audit-b-2-followup`) is also touching `commands/add.rs` — check `git log --all --oneline | grep -i "thread.*owner\|build_vault_path"` to see if their PR has landed. If yes, rebase against it and verify their fix is correct; if no, the audit lands here and they rebase later.

## Decisions that constrain Work Set B

1. **Reverse PR #72 + PR #70 directionally — no vault migration.** Files at `@<profile.slug>/...` stay. Lookups scan both directories (Task 4 + Task 12). `owners_equivalent` (PR #72's helper) is load-bearing and **must not be removed**.

2. **`@me` is canonical for own resources, `@<other-slug>` for team-shared.** Don't second-guess this if a sub-task seems to push the other way — escalate.

3. **`LocallyMissing` is a transitional state.** Phase 6's per-resource state machine will fold it in. Spec section "Phase 6 hand-off" captures this. Don't try to over-design the state machine here.

4. **One PR with two internal work sets.** Don't split into two PRs — the user said system-level value is closely related.

5. **Hybrid execution (per project_hybrid_execution_skill_idea).** Inline for mechanical tasks, subagent only for high-cognitive-load work. Don't dispatch subagents for Tasks 8, 9, 10, 14 — they're a few lines each.

## Open questions to raise at session start

- **Has the B.2 sibling session landed?** Affects Task 13 rebase strategy. Check `git log --all --oneline | grep -i "thread.*owner\|build_vault_path"` and the `2026-05-09-thread-owner-through-build-vault-path-audit-b-2-followup` task's stage.

- **PR opening — when?** User wants visual review of the branch first. Confirm PR creation timing after Tasks 8-15 land (likely as a single PR with the full diff, marked draft until verification passes).

- **Phase 4 plan after this PR?** Task #3 in the harness task tracker is still pending. Confirm whether to start Phase 4 plan-writing immediately after this PR ships or in a later session.

## Gotchas surfaced during Work Set A

- **`cargo nextest` re-runs sometimes hung at 0% CPU for 4+ minutes in-session.** Killing and re-running worked; cause unknown. `cargo test` worked fine. If it recurs in Work Set B, kill the process and use `cargo test` for ad-hoc filtered runs.

- **`OnceLock` test contamination.** The `find_resource_legacy_fallback_uses_process_wide_cache` test in `lookup.rs` is defensive about set-once contention; subsequent tests touching `cached_profile_slug` should follow the same pattern (gate assertions on whether the test won the race) or set the cache via Config field instead.

- **Test fixtures with `Config { ... }` struct literals** — there are ~20 of these across the crate. Task 4's commit (`15b2a4a`) added `profile_slug: None` to all current sites. New Work Set B work that adds another Config fixture should follow the pattern. A `#[derive(Default)]` on Config could simplify this, but is out of scope.

- **The bug task's affected file (`2026-05-09-fix-sync-handling-of-cloud-only-resources-...`) lives at `@j-cole-taylor/temper/task/` on the user's vault** — exactly the legacy directory case Task 4 + the wiring commit make reachable. After Task 12 lands and a sync runs, future pulls of that file will land at `@me/`. Verify this end-to-end during Task 15's manual smoke test.

- **PR #72 e2e regression test** at `tests/e2e/tests/pull_command_test.rs::pull_one_resource_newly_tracked_writes_canonical_owner_and_passes_preflight` will fail until Task 12's reversal is complete. That's expected — the test asserts the old direction. Rewrite it as part of Task 12.

## Verification cheat sheet

```bash
# Quick unit pass during inline development
cargo test -p temper-cli --lib <module>::<test>

# Full crate (use nextest)
cargo nextest run -p temper-cli --no-fail-fast

# Full workspace (catches feature-unification surprises)
cargo nextest run --workspace --no-fail-fast

# E2E with test-db
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db

# E2E with embed (only place ONNX is wired up; required before final commit)
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed

# Lint + format + everything
cargo make check

# Auto-fix
cargo make fix
```

## When Work Set B is complete

1. Final commit message wraps the entire PR's story (read the plan's Task 15 commit guidance).
2. Open PR via `gh pr create` (or per user preference).
3. Close these vault tasks to `done`:
   - `2026-05-09-fix-sync-handling-of-cloud-only-resources-...` (the bug task being fixed)
   - `audit-followups--rationalization-comments-hiding-incomplete-implementations` (C.1 is the last open item)
4. Mention in the PR body that PR #70 and PR #72's direction is being reversed (with no vault migration), and `OwnerResolver` semantics changed.
