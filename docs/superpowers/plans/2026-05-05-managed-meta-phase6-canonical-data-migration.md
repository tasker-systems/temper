# Managed-Meta Phase 6 — Canonical Data Migration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring legacy `kb_resource_manifests` rows into the canonical managed-meta shape established by Phases 1 + 5, then re-stamp `managed_hash` / `open_hash` so Phase 8 (`show_cache` tier-2) can be safely re-enabled.

**Architecture:** Two committed SQL migrations + one throwaway local Rust helper. Migration A is pure SQL: rename bare `title`/`slug` → `temper-title`/`temper-slug`, move `date` from managed_meta to open_meta for session/research/decision/concept rows, reset affected hashes to the empty-string sentinel. Helper walks the post-Migration-A snapshot via `temper_core::hash::compute_managed_hash` / `compute_open_hash`, emits Migration B as static `UPDATE`-per-row SQL. Phase 5's receive-side `ensure_managed_identity_keys` wiring is the safety net — any row Migration B misses gets re-stamped on next `temper sync run` push.

**Tech Stack:** PostgreSQL 18 with sqlx migrations, `temper-core` canonical-hash functions (`crates/temper-core/src/hash.rs`), `temper-api` integration test fixtures.

**Spec:** `docs/superpowers/specs/2026-05-05-managed-meta-phase6-canonical-data-migration-design.md`

**Umbrella:** `2026-05-03-schema-driven-managed-meta-alignment-temper-prefix-everywhere-schemas-as-contract`

---

## Pre-Flight

Two facts must be verified once before any task touches code. Both are quick controller-driven checks (not subagent tasks).

**P1 — Doctype name spelling.** The Migration A date-move pass filters by `kb_doc_types.name IN ('session', 'research', 'decision', 'concept')`. If the actual stored names differ in case or spelling, the migration silently no-ops. Verify with:

```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "SELECT DISTINCT name FROM kb_doc_types ORDER BY name;"
```

Expected: rows including exactly `concept`, `decision`, `research`, `session` (lowercase, singular). If anything differs, **stop** and update the spec + this plan before proceeding.

**P2 — Hash function signatures (already verified during plan-write):**

- `temper_core::hash::compute_managed_hash(doc_type: &str, managed_meta: &serde_json::Value) -> String` at `crates/temper-core/src/hash.rs:65`
- `temper_core::hash::compute_open_hash(open_meta: &serde_json::Value) -> String` at `crates/temper-core/src/hash.rs:82`
- Returns `"sha256:<hex>"`-prefixed strings.

Both must be reachable from `temper-api` (already a dep) and from a future `temper-api/examples/` binary.

---

## Task 1: Integration tests for Migration A (TDD red)

**Files:**
- Create: `crates/temper-api/tests/phase6_managed_meta_migration_test.rs`

This task writes the failing tests for Migration A. The migration SQL itself does not exist yet; tests must fail because the migration file doesn't exist (compile error if loaded by name) or because the rows aren't mutated when run against an unmigrated DB. The intent is to capture the contract first.

The Phase 5 file `crates/temper-api/tests/managed_hash_invariant_test.rs` is the structural template — same harness imports, same `setup_test_database()` helper, same `sqlx::query!` patterns.

- [ ] **Step 1: Write the failing tests**

```rust
//! Phase 6 — managed_meta canonical data migration tests.
//!
//! Verifies the SQL migration `migrations/<timestamp>_managed_meta_canonical_keys.sql`
//! correctly rewrites legacy JSONB shapes and resets affected hashes to the
//! empty-string sentinel.

use serde_json::json;
use sqlx::PgPool;
use temper_api::testing::setup_test_database;
use uuid::Uuid;

const MIGRATION_A_PATH: &str = "../../migrations/<TIMESTAMP>_managed_meta_canonical_keys.sql";

async fn apply_migration_a(pool: &PgPool) {
    let sql = std::fs::read_to_string(MIGRATION_A_PATH)
        .expect("Migration A SQL file should exist at the expected path");
    sqlx::raw_sql(&sql)
        .execute(pool)
        .await
        .expect("Migration A should apply cleanly");
}

async fn insert_legacy_session_row(
    pool: &PgPool,
    title: &str,
    slug: &str,
    date: &str,
) -> Uuid {
    // Look up the session doctype id, profile id, and a context id
    // from the seeded test fixtures (matches the Phase 5 test pattern).
    let doc_type_id: Uuid = sqlx::query_scalar!(
        "SELECT id FROM kb_doc_types WHERE name = 'session'"
    )
    .fetch_one(pool)
    .await
    .expect("session doctype must be seeded");

    let profile_id: Uuid = sqlx::query_scalar!(
        "SELECT id FROM kb_profiles LIMIT 1"
    )
    .fetch_one(pool)
    .await
    .expect("at least one profile must be seeded");

    let context_id: Uuid = sqlx::query_scalar!(
        "SELECT id FROM kb_contexts LIMIT 1"
    )
    .fetch_one(pool)
    .await
    .expect("at least one context must be seeded");

    let resource_id = Uuid::now_v7();

    sqlx::query!(
        "INSERT INTO kb_resources
         (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
          originator_profile_id, owner_profile_id, created, updated)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $7, now(), now())",
        resource_id,
        context_id,
        doc_type_id,
        format!("phase6-test://{}", resource_id),
        title,
        slug,
        profile_id,
    )
    .execute(pool)
    .await
    .expect("insert kb_resources");

    let legacy_managed = json!({
        "title": title,
        "slug": slug,
        "date": date,
        "temper-stage": "in-progress",
    });

    sqlx::query!(
        "INSERT INTO kb_resource_manifests
         (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
         VALUES ($1, 'sha256:legacy', $2, '{}'::JSONB,
                 'sha256:legacy_managed', 'sha256:legacy_open', now())",
        resource_id,
        legacy_managed,
    )
    .execute(pool)
    .await
    .expect("insert kb_resource_manifests");

    resource_id
}

#[sqlx::test]
async fn phase6_migration_a_renames_legacy_keys_and_moves_date(pool: PgPool) {
    setup_test_database(&pool).await;

    let id = insert_legacy_session_row(&pool, "Legacy Title", "legacy-slug", "2026-01-15").await;

    apply_migration_a(&pool).await;

    let row = sqlx::query!(
        "SELECT managed_meta, open_meta, managed_hash, open_hash
         FROM kb_resource_manifests WHERE resource_id = $1",
        id
    )
    .fetch_one(&pool)
    .await
    .expect("row should still exist");

    assert_eq!(
        row.managed_meta.get("temper-title").and_then(|v| v.as_str()),
        Some("Legacy Title"),
        "temper-title should be present with renamed value"
    );
    assert_eq!(
        row.managed_meta.get("temper-slug").and_then(|v| v.as_str()),
        Some("legacy-slug"),
        "temper-slug should be present with renamed value"
    );
    assert!(
        row.managed_meta.get("title").is_none(),
        "bare `title` should be stripped from managed_meta"
    );
    assert!(
        row.managed_meta.get("slug").is_none(),
        "bare `slug` should be stripped from managed_meta"
    );
    assert!(
        row.managed_meta.get("date").is_none(),
        "`date` should be stripped from managed_meta on session rows"
    );
    assert_eq!(
        row.open_meta.get("date").and_then(|v| v.as_str()),
        Some("2026-01-15"),
        "`date` should be moved into open_meta on session rows"
    );
    assert_eq!(row.managed_hash, "", "managed_hash should be reset to empty-string sentinel");
    assert_eq!(row.open_hash, "", "open_hash should be reset to empty-string sentinel");

    // Other managed-tier keys preserved.
    assert_eq!(
        row.managed_meta.get("temper-stage").and_then(|v| v.as_str()),
        Some("in-progress"),
        "non-renamed managed-tier keys should be preserved"
    );
}

#[sqlx::test]
async fn phase6_migration_a_idempotent_on_already_canonical_rows(pool: PgPool) {
    setup_test_database(&pool).await;

    let doc_type_id: Uuid = sqlx::query_scalar!(
        "SELECT id FROM kb_doc_types WHERE name = 'session'"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let profile_id: Uuid = sqlx::query_scalar!("SELECT id FROM kb_profiles LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let context_id: Uuid = sqlx::query_scalar!("SELECT id FROM kb_contexts LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    let resource_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_resources
         (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
          originator_profile_id, owner_profile_id, created, updated)
         VALUES ($1, $2, $3, $4, 'Canonical', 'canonical', $5, $5, now(), now())",
        resource_id, context_id, doc_type_id,
        format!("phase6-test://{}", resource_id), profile_id,
    )
    .execute(&pool)
    .await
    .unwrap();

    let canonical_managed = json!({
        "temper-title": "Canonical",
        "temper-slug": "canonical",
        "temper-stage": "done",
    });
    let canonical_open = json!({"date": "2026-04-01"});

    sqlx::query!(
        "INSERT INTO kb_resource_manifests
         (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
         VALUES ($1, 'sha256:body', $2, $3, 'sha256:canonical_m', 'sha256:canonical_o', now())",
        resource_id, canonical_managed, canonical_open,
    )
    .execute(&pool)
    .await
    .unwrap();

    apply_migration_a(&pool).await;

    let row = sqlx::query!(
        "SELECT managed_meta, open_meta, managed_hash, open_hash
         FROM kb_resource_manifests WHERE resource_id = $1",
        resource_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.managed_meta, canonical_managed,
        "canonical managed_meta should be untouched by Migration A");
    assert_eq!(row.open_meta, canonical_open,
        "canonical open_meta should be untouched by Migration A");
    assert_eq!(row.managed_hash, "sha256:canonical_m",
        "managed_hash should NOT be reset on canonical rows");
    assert_eq!(row.open_hash, "sha256:canonical_o",
        "open_hash should NOT be reset on canonical rows");
}

#[sqlx::test]
async fn phase6_migration_a_does_not_move_date_for_non_dated_doctypes(pool: PgPool) {
    setup_test_database(&pool).await;

    // A `task` row with a stray `date` in managed_meta should NOT have it moved
    // to open_meta — only session/research/decision/concept doctypes get the move.
    let doc_type_id: Uuid = sqlx::query_scalar!(
        "SELECT id FROM kb_doc_types WHERE name = 'task'"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let profile_id: Uuid = sqlx::query_scalar!("SELECT id FROM kb_profiles LIMIT 1")
        .fetch_one(&pool).await.unwrap();
    let context_id: Uuid = sqlx::query_scalar!("SELECT id FROM kb_contexts LIMIT 1")
        .fetch_one(&pool).await.unwrap();

    let resource_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_resources
         (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
          originator_profile_id, owner_profile_id, created, updated)
         VALUES ($1, $2, $3, $4, 'Stray Date Task', 'stray-date-task', $5, $5, now(), now())",
        resource_id, context_id, doc_type_id,
        format!("phase6-test://{}", resource_id), profile_id,
    )
    .execute(&pool).await.unwrap();

    sqlx::query!(
        "INSERT INTO kb_resource_manifests
         (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
         VALUES ($1, 'sha256:body', $2, '{}'::JSONB,
                 'sha256:legacy_m', 'sha256:legacy_o', now())",
        resource_id,
        json!({"title": "Stray Date Task", "date": "2026-04-01"}),
    )
    .execute(&pool).await.unwrap();

    apply_migration_a(&pool).await;

    let row = sqlx::query!(
        "SELECT managed_meta, open_meta, open_hash
         FROM kb_resource_manifests WHERE resource_id = $1",
        resource_id
    )
    .fetch_one(&pool).await.unwrap();

    // Title rename DID happen.
    assert_eq!(
        row.managed_meta.get("temper-title").and_then(|v| v.as_str()),
        Some("Stray Date Task")
    );
    // But date was NOT moved (task is not in the dated-doctype set).
    assert_eq!(
        row.managed_meta.get("date").and_then(|v| v.as_str()),
        Some("2026-04-01"),
        "date should remain in managed_meta for non-dated doctypes"
    );
    assert!(
        row.open_meta.get("date").is_none(),
        "date should NOT be added to open_meta for non-dated doctypes"
    );
    assert_eq!(
        row.open_hash, "sha256:legacy_o",
        "open_hash should NOT be reset when only managed_meta was changed"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p temper-api --features test-db phase6_migration_a -E 'test(phase6_migration_a)'`

Expected: `error` — the file `migrations/<TIMESTAMP>_managed_meta_canonical_keys.sql` does not exist, so `apply_migration_a` panics on the file-read. Tests fail at the SQL-load step. **Do not commit yet** — these tests are paired with Migration A in a single commit.

---

## Task 2: Migration A SQL (TDD green)

**Files:**
- Create: `migrations/YYYYMMDDHHMMSS_managed_meta_canonical_keys.sql` (use `date -u +%Y%m%d%H%M%S` for timestamp)
- Modify: `crates/temper-api/tests/phase6_managed_meta_migration_test.rs:14` — replace `<TIMESTAMP>` with the actual filename timestamp

- [ ] **Step 1: Generate the migration timestamp**

Run: `date -u +%Y%m%d%H%M%S`
Capture the output (e.g. `20260505140000`). This becomes part of the filename.

- [ ] **Step 2: Write Migration A**

```sql
-- Phase 6 — Canonical managed_meta keys + date move.
--
-- Brings legacy kb_resource_manifests rows into the canonical shape established
-- by Phases 1 + 5: rename bare `title`/`slug` to `temper-title`/`temper-slug`,
-- move `date` from managed_meta to open_meta for session/research/decision/
-- concept rows, and reset affected hashes to the empty-string sentinel so
-- Phase 5's receive-side wiring re-stamps them on next sync push.
--
-- See: docs/superpowers/specs/2026-05-05-managed-meta-phase6-canonical-data-migration-design.md
-- See: docs/superpowers/plans/2026-05-05-managed-meta-phase6-canonical-data-migration.md
--
-- Idempotent: re-running this migration is safe. Each UPDATE has a guard
-- predicate that skips already-canonical rows. Migration B (separate file,
-- generated by examples/phase6_recompute_hashes.rs) restores correct hashes.

BEGIN;

-- 1a. Rename `title` -> `temper-title` (bare key only).
UPDATE kb_resource_manifests
SET managed_meta = (managed_meta - 'title')
                || jsonb_build_object('temper-title', managed_meta->'title'),
    managed_hash = ''
WHERE managed_meta ? 'title'
  AND NOT (managed_meta ? 'temper-title');

-- 1b. Drop bare `title` if both forms coexist (canonical key wins).
UPDATE kb_resource_manifests
SET managed_meta = managed_meta - 'title',
    managed_hash = ''
WHERE managed_meta ? 'title'
  AND managed_meta ? 'temper-title';

-- 2a. Rename `slug` -> `temper-slug` (bare key only).
UPDATE kb_resource_manifests
SET managed_meta = (managed_meta - 'slug')
                || jsonb_build_object('temper-slug', managed_meta->'slug'),
    managed_hash = ''
WHERE managed_meta ? 'slug'
  AND NOT (managed_meta ? 'temper-slug');

-- 2b. Drop bare `slug` if both forms coexist (canonical key wins).
UPDATE kb_resource_manifests
SET managed_meta = managed_meta - 'slug',
    managed_hash = ''
WHERE managed_meta ? 'slug'
  AND managed_meta ? 'temper-slug';

-- 3a. Move `date` from managed_meta to open_meta for dated doctypes.
UPDATE kb_resource_manifests m
SET open_meta    = m.open_meta || jsonb_build_object('date', m.managed_meta->'date'),
    managed_meta = m.managed_meta - 'date',
    managed_hash = '',
    open_hash    = ''
FROM kb_resources r
JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
WHERE m.resource_id = r.id
  AND dt.name IN ('session', 'research', 'decision', 'concept')
  AND m.managed_meta ? 'date'
  AND NOT (m.open_meta ? 'date');

-- 3b. Drop `date` from managed_meta if open_meta already has one (defensive).
UPDATE kb_resource_manifests m
SET managed_meta = m.managed_meta - 'date',
    managed_hash = ''
FROM kb_resources r
JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
WHERE m.resource_id = r.id
  AND dt.name IN ('session', 'research', 'decision', 'concept')
  AND m.managed_meta ? 'date'
  AND m.open_meta ? 'date';

COMMIT;
```

- [ ] **Step 3: Update the test file's MIGRATION_A_PATH constant**

Replace `<TIMESTAMP>` in `crates/temper-api/tests/phase6_managed_meta_migration_test.rs:14` with the actual timestamp from Step 1.

- [ ] **Step 4: Run the integration tests**

Run: `cargo nextest run -p temper-api --features test-db -E 'test(phase6_migration_a)'`

Expected: 3 passed, 0 failed. All three tests (`renames_legacy_keys_and_moves_date`, `idempotent_on_already_canonical_rows`, `does_not_move_date_for_non_dated_doctypes`) pass.

- [ ] **Step 5: Run full temper-api integration suite as regression guard**

Run: `cargo make test-db`

Expected: full pass, no regressions in any other temper-api integration test. (Per memory: never trust nextest's per-binary "Summary" line under `--no-fail-fast` — verify via exit code 0, and grep the output for `error: test run failed` / `FAIL [` to confirm.)

- [ ] **Step 6: Run cargo make check**

Run: `cargo make check`

Expected: clean. Migration SQL doesn't change any Rust code, so this is mostly a formality, but the test file's new code does need to pass clippy.

- [ ] **Step 7: Commit**

```bash
git add migrations/<TIMESTAMP>_managed_meta_canonical_keys.sql \
        crates/temper-api/tests/phase6_managed_meta_migration_test.rs
git commit -m "$(cat <<'EOF'
feat(migrations): canonical managed_meta keys + date move (Phase 6 Migration A)

Rewrites legacy kb_resource_manifests rows: renames bare title/slug to
temper-title/temper-slug, moves date from managed_meta to open_meta for
session/research/decision/concept rows, resets affected hashes to the
empty-string sentinel so Phase 5 receive-side wiring re-stamps them on
next sync push.

Idempotent: re-running is safe; each UPDATE has guard predicates that
skip already-canonical rows. Migration B (separate, generated) restores
correct hashes.

Integration tests in crates/temper-api/tests/phase6_managed_meta_migration_test.rs
cover: legacy-rename happy path, idempotency on canonical rows, and
non-dated-doctype scoping for the date pass.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Throwaway hash-recompute helper

**Files:**
- Create: `crates/temper-api/examples/phase6_recompute_hashes.rs`
- Modify: `crates/temper-api/Cargo.toml` — add `[[example]]` stanza if examples need explicit registration (check existing `[[bin]]` patterns in the file first; sqlx workspaces sometimes require it)

This file is throwaway. It will be `git rm`'d in Task 6 after the branch lands and validates. The point is to walk every row in `kb_resource_manifests`, decode its JSONB, run `temper-core`'s canonical hash functions, and emit one `UPDATE` per row.

- [ ] **Step 1: Write the helper**

```rust
//! Phase 6 — one-shot hash-recompute helper. Throwaway.
//!
//! Connects to the database via DATABASE_URL, walks every row in
//! kb_resource_manifests, computes managed_hash + open_hash via
//! temper_core::hash, and emits a static `UPDATE`-per-row SQL script
//! to stdout for capture into Migration B.
//!
//! Usage:
//!   DATABASE_URL=postgresql://... \
//!     cargo run --example phase6_recompute_hashes -p temper-api \
//!     > migrations/<TIMESTAMP>_managed_meta_recompute_hashes.sql
//!
//! Run AFTER Migration A has been applied to the target database (locally
//! against a snapshot of prod, or directly against prod read-replica).
//!
//! Removed in cleanup commit after Migration B validates on remote.

use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use temper_core::hash::{compute_managed_hash, compute_open_hash};
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await?;

    let rows = sqlx::query!(
        r#"
        SELECT m.resource_id AS "resource_id!: Uuid",
               m.managed_meta AS "managed_meta!: Value",
               m.open_meta    AS "open_meta!: Value",
               dt.name        AS "doc_type!: String"
        FROM kb_resource_manifests m
        JOIN kb_resources r       ON r.id = m.resource_id
        JOIN kb_doc_types dt      ON dt.id = r.kb_doc_type_id
        ORDER BY m.resource_id
        "#
    )
    .fetch_all(&pool)
    .await?;

    let now = chrono::Utc::now().to_rfc3339();
    println!("-- Phase 6 — Migration B: recompute managed_hash + open_hash.");
    println!("--");
    println!("-- Generated by crates/temper-api/examples/phase6_recompute_hashes.rs");
    println!("-- Generated at: {}", now);
    println!("-- Source rows: {}", rows.len());
    println!("--");
    println!("-- Run AFTER Migration A. Each UPDATE matches a row by resource_id and");
    println!("-- writes the canonical managed_hash + open_hash for that row's current");
    println!("-- (post-Migration-A) JSONB shape, computed via temper_core::hash.");
    println!("--");
    println!("-- Any row added/updated between this generation time and deploy time");
    println!("-- will retain the empty-string hash sentinel from Migration A; the");
    println!("-- next `temper sync run` push re-stamps it via Phase 5 receive-side.");
    println!();
    println!("BEGIN;");
    println!();

    for row in &rows {
        let managed_hash = compute_managed_hash(&row.doc_type, &row.managed_meta);
        let open_hash = compute_open_hash(&row.open_meta);
        println!(
            "UPDATE kb_resource_manifests SET managed_hash = '{}', open_hash = '{}' WHERE resource_id = '{}';",
            managed_hash, open_hash, row.resource_id,
        );
    }

    println!();
    println!("COMMIT;");

    eprintln!("Emitted {} UPDATEs.", rows.len());
    Ok(())
}
```

- [ ] **Step 2: Verify the example compiles**

Run: `cargo build --example phase6_recompute_hashes -p temper-api`

Expected: clean build. If `cargo` complains that the example needs to be registered explicitly in `Cargo.toml`, add:

```toml
[[example]]
name = "phase6_recompute_hashes"
path = "examples/phase6_recompute_hashes.rs"
```

(This is rarely needed for files in `examples/` but workspace configs sometimes require it.)

- [ ] **Step 3: Smoke-test the helper against the local dev DB**

Make sure Docker Postgres is up (`cargo make docker-up`) and dev DB is migrated (`sqlx migrate run --source migrations`).

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo run --example phase6_recompute_hashes -p temper-api 2>/dev/null | head -30`

Expected: SQL output with header comment, `BEGIN;`, then `UPDATE ...` lines for each row in your dev DB. Stderr reports the count. If your dev DB is empty, the output will be just the header + `BEGIN;` + `COMMIT;` — that's fine, it confirms the helper runs.

- [ ] **Step 4: Run cargo make check**

Run: `cargo make check`

Expected: clean. The helper's `sqlx::query!` macro will be checked against the live dev DB at compile time.

- [ ] **Step 5: Regenerate sqlx offline cache**

Run: `cargo sqlx prepare --workspace -- --all-features`

Expected: cache updated to include the helper's new query. Without this, CI builds (which run with `SQLX_OFFLINE=true`) will fail. Per CLAUDE.md: "After changing any SQL: Regenerate cache".

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/examples/phase6_recompute_hashes.rs \
        .sqlx/
# Only add Cargo.toml if you needed to add the [[example]] stanza in Step 2.
# git add crates/temper-api/Cargo.toml
git commit -m "$(cat <<'EOF'
feat(temper-api): add Phase 6 hash-recompute helper (throwaway)

One-shot example that walks kb_resource_manifests, computes managed_hash
and open_hash for each row via temper_core::hash, and emits a static
UPDATE-per-row SQL script for capture into Phase 6 Migration B.

Run locally against a snapshot of prod that has had Migration A applied:

  DATABASE_URL=... cargo run --example phase6_recompute_hashes -p temper-api \
    > migrations/<TIMESTAMP>_managed_meta_recompute_hashes.sql

Helper is removed in a follow-up commit after Migration B validates on
remote.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Generate and commit Migration B

This task is partly user-runbook (the user must point the helper at a real database with prod data). The deliverable is a committed SQL file.

**Files:**
- Create: `migrations/YYYYMMDDHHMMSS_managed_meta_recompute_hashes.sql`

- [ ] **Step 1: Acquire a production snapshot in a local Postgres**

The user needs prod data locally to generate accurate hashes. Options (user picks):

a. **Neon branch** — `neonctl branches create --project-id <id> --name phase6-snapshot` and connect to that branch's URL.
b. **`pg_dump | psql`** from prod into a fresh local database.
c. **Read-replica direct** — point `DATABASE_URL` straight at a prod read-replica if one exists.

Whichever path, the result is a `DATABASE_URL` pointing at a database that has the same row contents as prod.

- [ ] **Step 2: Apply Migration A to the snapshot**

```bash
DATABASE_URL=<snapshot_url> sqlx migrate run --source migrations
```

Expected: Migration A applies; legacy rows mutated.

- [ ] **Step 3: Generate Migration B's filename timestamp**

Run: `date -u +%Y%m%d%H%M%S`

The timestamp must be **strictly greater** than Migration A's timestamp, so sqlx applies Migration A first. If you're running this within a minute of Migration A's commit, add at least one second.

- [ ] **Step 4: Run the helper against the snapshot, capturing to Migration B**

```bash
DATABASE_URL=<snapshot_url> cargo run --example phase6_recompute_hashes -p temper-api \
  > migrations/<TIMESTAMP>_managed_meta_recompute_hashes.sql
```

Expected: stdout captured to file; stderr reports `Emitted N UPDATEs.` where N matches your prod row count for `kb_resource_manifests`.

- [ ] **Step 5: Sanity-check Migration B**

```bash
# Row count check
wc -l migrations/<TIMESTAMP>_managed_meta_recompute_hashes.sql
# Should be ~N + ~12 (header + BEGIN/COMMIT scaffolding).

# Visual spot-check
head -15 migrations/<TIMESTAMP>_managed_meta_recompute_hashes.sql
tail -5 migrations/<TIMESTAMP>_managed_meta_recompute_hashes.sql

# Confirm every UPDATE has a sha256: prefix on both hashes
grep -c "managed_hash = 'sha256:" migrations/<TIMESTAMP>_managed_meta_recompute_hashes.sql
grep -c "open_hash = 'sha256:" migrations/<TIMESTAMP>_managed_meta_recompute_hashes.sql
# Both counts should equal N.
```

If counts mismatch or any line lacks the `sha256:` prefix, **stop** — the helper has a bug and Migration B is unsafe.

- [ ] **Step 6: Apply Migration B to the snapshot to verify it parses and runs**

```bash
DATABASE_URL=<snapshot_url> sqlx migrate run --source migrations
```

Expected: Migration B applies cleanly. Re-running is safe (each `UPDATE` is idempotent for the same input shape).

- [ ] **Step 7: Verify hash consistency on the snapshot**

```bash
DATABASE_URL=<snapshot_url> psql -c "SELECT count(*) FROM kb_resource_manifests WHERE managed_hash = '' OR open_hash = '';"
```

Expected: `0`. Every row has both hashes populated.

- [ ] **Step 8: Commit Migration B**

```bash
git add migrations/<TIMESTAMP>_managed_meta_recompute_hashes.sql
git commit -m "$(cat <<'EOF'
feat(migrations): recompute managed_hash + open_hash post-canonical-rewrite (Phase 6 Migration B)

Generated SQL file: one UPDATE per kb_resource_manifests row, restoring
canonical managed_hash + open_hash after Migration A's JSONB rewrite.

Generated by crates/temper-api/examples/phase6_recompute_hashes.rs against
a prod snapshot that had Migration A applied. Sanity-checked: row count
matches prod manifest count, every UPDATE has sha256: hash prefixes,
applies cleanly to the snapshot, post-apply count of empty-hash rows is
zero.

Any row created/updated between snapshot time and remote deploy time
retains the empty-string sentinel from Migration A; Phase 5 receive-side
wiring re-stamps it on the next `temper sync run` push.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Branch validation (pre-merge)

Before this branch lands on `main`, run all quality gates one more time. Per memory `feedback_nextest_summary_lies.md`: never trust the trailing "Summary" line — verify exit code 0 and grep for `error: test run failed` / `FAIL [`.

- [ ] **Step 1: Reset local dev DB and re-apply all migrations from scratch**

```bash
cargo make docker-down && cargo make docker-up
sqlx migrate run --source migrations
```

Expected: all migrations apply in order, including A then B.

- [ ] **Step 2: Run full unit + integration suite**

Run: `cargo make test-all`

Expected: full pass. Confirm by `echo $?` returning 0 and grepping output for failures.

- [ ] **Step 3: Run e2e suite**

Run: `cargo make test-e2e`

Expected: full pass, including the existing Phase 5 acceptance gate `phase5_local_canonical_hash_matches_server_managed_hash` in `tests/e2e/tests/show_cache_e2e_test.rs`.

- [ ] **Step 4: Run cargo make check**

Run: `cargo make check`

Expected: clean.

- [ ] **Step 5: Confirm sqlx offline cache is up to date**

Run: `cargo sqlx prepare --workspace --check -- --all-features`

Expected: no diff. CI builds will succeed in `SQLX_OFFLINE=true` mode.

---

## Task 6: Post-deploy validation + helper cleanup

This task runs **after** the branch has been merged to `main` and deployed to remote (Vercel + Neon). The helper is removed only after Migration B is confirmed working in production.

- [ ] **Step 1: Confirm Migration A + B applied on remote**

```bash
DATABASE_URL=<prod_url> psql -c "SELECT version FROM _sqlx_migrations ORDER BY version DESC LIMIT 5;"
```

Expected: both Migration A's and Migration B's timestamps appear among the most recent applied migrations.

- [ ] **Step 2: Confirm zero empty-hash rows on remote**

```bash
DATABASE_URL=<prod_url> psql -c "SELECT count(*) FROM kb_resource_manifests WHERE managed_hash = '' OR open_hash = '';"
```

Expected: `0`. If non-zero, those are rows that were created/updated between snapshot time and deploy time. Run `temper sync run` from the canonical local vault to re-stamp them via Phase 5 receive-side.

- [ ] **Step 3: Confirm Phase 5 acceptance gate still passes against prod**

Re-run `tests/e2e/tests/show_cache_e2e_test.rs::phase5_local_canonical_hash_matches_server_managed_hash` against a prod-pointed test config (or trust the standing `cargo make test-e2e` against the dev DB plus the empty-hash count check above as proxy).

- [ ] **Step 4: Run `temper sync run` from canonical local vault**

```bash
cd /Users/petetaylor/projects/kb-vault
temper sync run
```

Expected: should report a clean sync — no pushes needed for the bulk of resources (Migration B handled them), or a small number of pushes for any rows that were missed. After it completes, every prod row has a canonical hash.

- [ ] **Step 5: Re-confirm zero empty-hash rows on remote**

```bash
DATABASE_URL=<prod_url> psql -c "SELECT count(*) FROM kb_resource_manifests WHERE managed_hash = '' OR open_hash = '';"
```

Expected: `0`.

- [ ] **Step 6: Remove the throwaway helper**

```bash
git rm crates/temper-api/examples/phase6_recompute_hashes.rs
# If you added a [[example]] stanza in Task 3 Step 2, also remove it from Cargo.toml.
```

- [ ] **Step 7: Regenerate sqlx offline cache (helper's query is gone)**

Run: `cargo sqlx prepare --workspace -- --all-features`

Expected: cache shrinks (entries for the removed query are gone).

- [ ] **Step 8: Run cargo make check**

Run: `cargo make check`

Expected: clean.

- [ ] **Step 9: Commit cleanup**

```bash
git add -u  # picks up the rm and the sqlx cache changes
git commit -m "$(cat <<'EOF'
chore: remove one-shot Phase 6 hash recompute helper

Migration B is applied on remote and verified — every kb_resource_manifests
row has canonical managed_hash + open_hash. The throwaway helper's job is
done; removing it keeps the workspace clean.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Mark Phase 6 done in umbrella task

- [ ] **Step 1: Update the umbrella task's plan-status (if it tracks per-phase status)**

Check `2026-05-03-schema-driven-managed-meta-alignment-temper-prefix-everywhere-schemas-as-contract` for any "Phase X — done" annotation pattern from prior phases. If present, append a Phase 6 entry pointing at the spec + plan + commit shas.

If the umbrella task uses a separate plan-status section: edit it. If not: skip this step — the session note for this work will record the same information.

---

## Self-Review

**1. Spec coverage:**
- Migration A → Tasks 1+2 (tests + SQL).
- Helper → Task 3.
- Migration B (generated) → Task 4.
- Empty-hash sentinel + Phase 5 safety net → woven through (Task 4 Steps 5+7, Task 6 Steps 2+5).
- Integration tests for Migration A (renames, idempotency, doctype scoping) → Task 1's three tests.
- Cleanup commit (`git rm` helper) → Task 6 Steps 6+9.
- Verification SQL (`SELECT count(*) WHERE managed_hash = ''`) → Task 4 Step 7 (snapshot), Task 6 Steps 2+5 (prod).
- Phase 5 acceptance gate as end-to-end check → Task 5 Step 3, Task 6 Step 3.
- Doctype name verification (open question P1) → Pre-Flight P1.
- Hash function entry points (open question P2) → Pre-Flight P2 (already done at plan-write).

**2. Placeholder scan:**
- Migration filename timestamps left as `<TIMESTAMP>` placeholders intentionally — Task 2 Step 1 + Task 4 Step 3 capture the actual `date` command and substitute. The test file is updated explicitly in Task 2 Step 3.
- No TBD/TODO markers in design content.
- No "fill in details" or "similar to Task N" steps — all code shown in full.

**3. Type / function consistency:**
- `compute_managed_hash(doc_type: &str, managed_meta: &serde_json::Value) -> String` — used identically in Task 3.
- `compute_open_hash(open_meta: &serde_json::Value) -> String` — used identically in Task 3.
- Doctype string set `('session', 'research', 'decision', 'concept')` — used identically in spec, Migration A, and integration tests.
- Sentinel `''` for hashes — used identically in Migration A, integration tests, and validation queries.
