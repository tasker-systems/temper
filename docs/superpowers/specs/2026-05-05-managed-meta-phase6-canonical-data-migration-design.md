# Managed-Meta Phase 6 — Canonical Data Migration — Design Spec

**Date:** 2026-05-05
**Context:** `temper`
**Mode:** plan
**Effort:** medium (single session, controller-driven)
**Branch:** `jct/wave1-shared-execution-paths-and-cloud-first-reframe`

**Related work:**
- Umbrella plan task: `2026-05-03-schema-driven-managed-meta-alignment-temper-prefix-everywhere-schemas-as-contract`.
- Backbone spec: `docs/superpowers/specs/2026-05-03-schema-driven-managed-meta-design.md` — defines the canonical key set this migration is bringing legacy data into compliance with.
- Phase 1 plan: `docs/superpowers/plans/2026-05-04-managed-meta-phase1-schema-contract-foundation.md` — landed the schema/serde rename and the `date` drop from managed-tier.
- Phase 5 plan: `docs/superpowers/plans/2026-05-04-managed-meta-phase5-canonical-projection-injection.md` — landed the symmetric send-side + receive-side `ensure_managed_identity_keys` injection that this phase relies on for the post-deploy safety net.
- Adjacent (out of scope): Phase 8 (re-enable `show_cache` tier-2) and Phase 9 (`temper doctor fix` legacy vault rewrite). Both are unblocked once this phase lands; both are separate plans.

---

## Problem

After Phase 1 + Phase 5 landed, every newly-ingested or newly-updated resource produces and stores canonical managed-meta:
- `temper-title` / `temper-slug` keys (not bare `title` / `slug`).
- `date` in `open_meta` (not `managed_meta`) for session / research / decision / concept rows.
- Server-stored `managed_hash` and `open_hash` computed over that canonical shape.

But pre-Phase-1 rows in production were ingested with the old shape and have **never been touched since**. They carry:
- Bare `title:` / `slug:` keys in `kb_resource_manifests.managed_meta` JSONB.
- `date:` inside `kb_resource_manifests.managed_meta` for the four affected doctypes.
- `managed_hash` / `open_hash` values computed over the legacy shape.

These rows present three problems:

1. **Phase 8 blocker.** `show_cache` tier-2 compares the local canonical hash (computed over the canonical local frontmatter) to `kb_resource_manifests.managed_hash`. Any legacy row will mismatch on every show, defeating the cache.
2. **Drift footgun.** Any consumer that reads JSONB directly (search, future analytics, manifest-based diffs) sees inconsistent shapes across rows.
3. **Phase 9 blocker.** `temper doctor fix` will rewrite local vault files to canonical, but the corresponding server rows still hold the legacy shape, so the round-trip won't actually converge.

## The Reframe

This is not a code change. It is a one-time data fix-up against the production database, executed via committed SQL migrations on the same branch as Phases 1 + 5.

The work splits into three pieces, in the order they will land:

1. **Migration A — pure SQL, mechanical.** Rewrites JSONB keys and resets the affected hashes to the empty-string sentinel. Deterministic. Reviewable.
2. **One-shot local helper — throwaway Rust example.** Connects to a snapshot of the post-Migration-A database, computes the correct `managed_hash` / `open_hash` for every row using the canonical-hash code already in `temper-core`, emits a static `UPDATE`-per-row SQL file.
3. **Migration B — generated, committed verbatim.** The helper's output. Restores correct hashes for the bulk of rows in a single bounded operation.

A safety net catches anything Migration B misses: the empty-string hash sentinel forces the next `temper sync run` push for that row to re-stamp the hash via Phase 5's receive-side `ensure_managed_identity_keys` wiring. No silent corruption — empty strings are loud (they fail every hash comparison).

## Decisions Locked (this brainstorm)

| Decision | Choice | Rationale |
|---|---|---|
| Migration scope | All `kb_resource_manifests` rows are evaluated; only legacy rows mutate (`WHERE managed_meta ? 'title'`, `WHERE managed_meta ? 'slug'`, `WHERE doc_type IN (…) AND managed_meta ? 'date'`) | Idempotent — re-running is safe; post-Phase-5 rows are skipped automatically |
| Hash sentinel | Empty string `''` | Matches `kb_resource_manifests` column default (`NOT NULL DEFAULT ''`); cannot be confused with a real `sha256:…`-prefixed hash; existing receive-side code already treats empty as "needs computing" |
| Helper persistence | `git rm`'d in a follow-up commit after validation | Truly one-time; user is the only operator; no need to keep code that won't be re-run |
| Hash compute language | Rust (existing `temper-core` canonical hash) | Zero divergence risk vs. plpgsql sha256 — Phase 5 just fixed the bug class that comes from re-implementing canonicalization elsewhere |
| Migration B authorship | Generated locally, committed verbatim | Audit trail; the SQL that runs on remote is the SQL in the repo; no runtime decisions during deploy |
| Conflict policy on key collision | Canonical key wins (`temper-title` retained, bare `title` dropped) | Should never occur in practice — Phase 5 receive-side stamps canonical even when caller sends bare — but we define behavior explicitly so the migration is safe even on hand-edited data |

## The Migrations

### Migration A — `YYYYMMDDHHMMSS_managed_meta_canonical_keys.sql`

Single transaction. Three `UPDATE` statements scoped by guard predicates so each is idempotent and only touches rows that need it.

**1. Rename `title` → `temper-title`:**

```sql
UPDATE kb_resource_manifests
SET managed_meta = (managed_meta - 'title') || jsonb_build_object('temper-title', managed_meta->'title'),
    managed_hash = ''
WHERE managed_meta ? 'title'
  AND NOT managed_meta ? 'temper-title';
```

(The title-rename and slug-rename `UPDATE`s touch `managed_meta` only and reset `managed_hash` only. `open_meta` and `open_hash` are unaffected by these two passes; only the date-move pass below changes `open_meta` and resets `open_hash`.)

If both `title` and `temper-title` somehow coexist on a row (defensive, should not occur), the canonical key is preserved and the bare key stripped:

```sql
UPDATE kb_resource_manifests
SET managed_meta = managed_meta - 'title',
    managed_hash = ''
WHERE managed_meta ? 'title'
  AND managed_meta ? 'temper-title';
```

**2. Rename `slug` → `temper-slug`:** symmetric to (1), with the same conflict-handling pair.

**3. Move `date` from managed_meta to open_meta** for session / research / decision / concept rows:

```sql
UPDATE kb_resource_manifests m
SET open_meta    = m.open_meta || jsonb_build_object('date', m.managed_meta->'date'),
    managed_meta = m.managed_meta - 'date',
    managed_hash = '',
    open_hash    = ''
FROM kb_resources r
JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
WHERE m.resource_id = r.id
  AND dt.name IN ('session', 'research', 'decision', 'concept')
  AND m.managed_meta ? 'date';
```

If a row already has `date` in `open_meta` (defensive, should not occur post-Phase-1 normalization), the existing `open_meta.date` wins — we strip from `managed_meta` without overwriting:

```sql
UPDATE kb_resource_manifests m
SET managed_meta = m.managed_meta - 'date',
    managed_hash = ''
FROM kb_resources r
JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
WHERE m.resource_id = r.id
  AND dt.name IN ('session', 'research', 'decision', 'concept')
  AND m.managed_meta ? 'date'
  AND m.open_meta ? 'date';
```

**Why empty-string and not NULL:** `kb_resource_manifests.managed_hash` and `open_hash` are declared `VARCHAR(128) NOT NULL DEFAULT ''` (see `migrations/20260404000002_resource_manifests.sql`). Setting NULL would require a column ALTER, which is out of scope and unnecessary — empty string already serves as the recompute sentinel for new rows.

### Helper — `crates/temper-api/examples/phase6_recompute_hashes.rs`

Throwaway. Lives in `temper-api/examples/` because that crate already has the database connection plumbing and `sqlx` macro setup. Run with `DATABASE_URL=… cargo run --example phase6_recompute_hashes -p temper-api > migrations/YYYYMMDDHHMMSS_managed_meta_recompute_hashes.sql`.

Logic:

1. Connect via `sqlx::PgPool::connect(env!("DATABASE_URL"))`.
2. `SELECT resource_id, managed_meta, open_meta FROM kb_resource_manifests` — every row, no filter (the canonical hash is well-defined for the empty `{}` case too, so this also stamps any defaulted-but-untouched rows).
3. For each row: invoke the existing `temper-core` canonical hash function (the same one `ingest_service::ingest`, `resource_service::update`, and `meta_service::update_meta` use post-Phase-5) on `managed_meta` and `open_meta`.
4. Emit one line per row to stdout: `UPDATE kb_resource_manifests SET managed_hash = '<hash>', open_hash = '<hash>' WHERE resource_id = '<uuid>';`.
5. Wrap the output in `BEGIN; … COMMIT;` plus a header comment explaining the file is generated by `phase6_recompute_hashes.rs` against the post-Migration-A snapshot from `<date>`.

The helper is committed only long enough for review; it gets `git rm`'d in the cleanup commit after the branch validates on remote. Detailed reasoning for not keeping it: the SQL it emits is what's load-bearing; re-running the helper requires a snapshot that no longer exists; and the same generic shape (Rust binary that walks JSONB and re-stamps hashes) would be cleaner to write fresh against future schemas than to preserve in working order indefinitely.

### Migration B — `YYYYMMDDHHMMSS_managed_meta_recompute_hashes.sql`

The helper's stdout, captured to a file in `migrations/`, committed verbatim. Hardcoded `UPDATE` per row inside a `BEGIN; … COMMIT;` block. Header comment records: branch, date, post-Migration-A row count.

Runs immediately after Migration A on `cargo make migrate` and on the next Vercel deploy — sqlx applies migrations in filename order, so as long as Migration B's timestamp is greater than Migration A's, ordering is guaranteed.

## Execution Workflow

1. **Land Migration A** in a commit. CI runs `cargo make test-db` against a fresh test DB; verifies the SQL parses and the guard predicates hold (no rows mutate in a fresh DB because there are no rows; integration test below adds one).
2. **Author the helper** in `crates/temper-api/examples/phase6_recompute_hashes.rs`. Commit.
3. **User runs the helper locally** against a snapshot of prod that has had Migration A applied. Captures stdout to `migrations/YYYYMMDDHHMMSS_managed_meta_recompute_hashes.sql`.
4. **User reviews Migration B** (sanity-check row count, spot-check a hash against the local canonical hash for the same row).
5. **Commit Migration B.** CI runs again; integration test below ensures the migration applies cleanly even when there are no matching rows (idempotent on already-canonicalized DBs).
6. **Branch lands on `main`**, deploys to remote. Both migrations apply in order. Production hashes converge.
7. **User runs `temper sync run`** from the canonical local vault. Should be a no-op for any row Migration B handled. Any row with empty-string hash (shouldn't exist, but defensive) gets re-stamped on push via Phase 5 receive-side.
8. **Validation:** spot-check via `SELECT count(*) FROM kb_resource_manifests WHERE managed_hash = ''` — should be zero.
9. **Cleanup commit:** `git rm crates/temper-api/examples/phase6_recompute_hashes.rs`.

## Testing Plan

The migrations themselves run against an empty test DB during `cargo make test-db`, which proves the SQL is syntactically valid but tests nothing semantic. Two integration tests close the gap:

1. **`phase6_migration_a_renames_legacy_keys`** in `crates/temper-api/tests/`:
   - Insert a `kb_resources` row + `kb_resource_manifests` row with legacy shape (`{title: "Foo", slug: "foo", date: "2026-01-01"}` for a session-doctype row).
   - Run Migration A's three `UPDATE`s manually (or via a helper that loads the SQL file).
   - Assert the resulting JSONB has `temper-title`, `temper-slug`, no bare `title`/`slug`/`date` in managed_meta, and `date` present in open_meta.
   - Assert `managed_hash` and `open_hash` are empty strings.

2. **`phase6_migration_a_idempotent_on_canonical_rows`** in the same file:
   - Insert a `kb_resource_manifests` row already in canonical shape (`{temper-title, temper-slug}` + open_meta date).
   - Run Migration A.
   - Assert nothing mutated — JSONB unchanged, hashes unchanged.

These tests live in `temper-api/tests/` because they need a real DB. They use the same `setup_test_database` fixture other integration tests use. They are not in `tests/e2e/` because they don't exercise CLI ↔ API paths — pure DB behavior.

The Phase 5 acceptance gate (`phase5_local_canonical_hash_matches_server_managed_hash` in `tests/e2e/tests/show_cache_e2e_test.rs`) provides the end-to-end check after both migrations apply: the local canonical hash for any resource equals the server's `managed_hash`. Already in place; will pass automatically once the migrations land.

The helper's correctness is verified by inspection (the user spot-checks the generated SQL against locally-recomputed hashes for a few resources before committing Migration B). It is not unit-tested because it is throwaway and operates against ephemeral data.

## What This Spec Is Not

- **Not a code change to read paths.** Every consumer already reads canonical keys after Phase 1 + Phase 5. This is purely about catching legacy data up to the canonical shape.
- **Not a hash function change.** The canonical hash function in `temper-core` is unchanged. Migration B applies the same function the live server now uses.
- **Not a `temper doctor fix` invocation.** That's Phase 9's job for vault files on disk. This phase only touches the server database.
- **Not a schema change.** No table ALTERs. No column adds or drops. No constraint changes. JSONB content only.
- **Not a re-enabling of show_cache tier-2.** That's Phase 8's job. This phase only ensures Phase 8's precondition (every row's `managed_hash` matches the canonical hash for its current JSONB) holds for legacy rows.

## Risks and Mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| Helper's snapshot drifts from production state between snapshot time and deploy time (new resources ingested, existing resources updated) | Likely (you keep working between snapshot and deploy) | Empty-hash safety net: any unhandled row is re-stamped on next `temper sync run` push via Phase 5 receive-side. Worst case is a transient cache miss on those rows. |
| Migration A fails partway through (network blip during Vercel deploy) | Unlikely (sqlx wraps in a transaction) | sqlx's transactional migration handling rolls back; re-running is safe per the idempotent guards. |
| Migration B has stale hash for a row that was updated post-snapshot | Possible | Same as row 1 — Phase 5 receive-side wiring catches it on the next `update`. Migration B is best-effort bulk; Phase 5 wiring is the correctness guarantee. |
| Helper produces a hash that diverges from what `temper-core` will compute on next ingest | Very unlikely (helper uses the exact same function) | Helper imports `temper-core::operations` directly. Any divergence would also break Phase 5's send-side, which is already verified. |
| Doctype names in `kb_doc_types.name` don't match the strings `'session'`, `'research'`, `'decision'`, `'concept'` | Unlikely (they're constants) | Plan task to verify before writing the SQL: `SELECT DISTINCT name FROM kb_doc_types`. Confirm exact spelling matches. |

## Out of Scope

- Phase 8 (`show_cache` tier-2 re-enable + companion test).
- Phase 9 (`temper doctor fix` legacy vault file rewrite).
- Wire-shape collapse (dropping top-level `title` / `slug` from `IngestPayload` / `ResourceUpdateRequest`); per Phase 5 spec, deferred indefinitely unless drift observed.
- Generated columns for `kb_resources.title` / `kb_resources.slug`; per backbone spec, deferred.
- Adjustments to the canonical hash function itself.

## Open Questions

None at brainstorm close. All design decisions are locked. Implementation plan should verify two facts before writing SQL:

1. **Doctype name spelling** — query `SELECT DISTINCT name FROM kb_doc_types` against dev DB to confirm `'session'`, `'research'`, `'decision'`, `'concept'` are the exact stored names.
2. **`temper-core` canonical-hash entry point** — confirm the function signature and module path the Phase 5 wiring uses (likely `temper_core::operations::ensure_managed_identity_keys` adjacent), so the helper can call into the same source-of-truth function.
