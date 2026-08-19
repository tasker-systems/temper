# Owner-Scoped URIs: Phase 2 Database + API + Core Types

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add profile slugs, rewrite URI functions to include owner sigils (`@profile`/`+team`), update core Rust types, and add `temper-owner` to the frontmatter schema.

**Architecture:** Single SQL migration adds `kb_profiles.slug`, rewrites `kb_resource_uri()` and `resource_for_uri()`. Rust types updated in temper-core (`Profile.slug`, `Subscription.owner`). Frontmatter base schema extended with `temper-owner`. Legacy URI fallback in `resource_for_uri()` for development safety.

**Tech Stack:** PostgreSQL (plpgsql), Rust (sqlx, serde), JSON Schema

**Subagent guidance — include verbatim in every subagent prompt:**

```
SG-1: Follow Existing Patterns — before writing anything, read the file you're modifying
AND a sibling in the same module. Match the style: naming, imports, structure, error handling.

SG-2: Single Responsibility — each function does one thing. Follow the project's existing layering.

SG-3: No Logic Duplication — extract only if two implementations would drift. No premature abstractions.

SG-4: Test Strategy — unit tests co-located with code. Integration tests separate. One behavior
per test with descriptive names. Tests must actually run — verify, don't assume.

SG-5: Don't Over-Build — implement exactly what the task says. No speculative features.

SG-6: Verify Before Claiming Done — run the verification command. Read the output.

SG-7: Prefer Native Solutions — use framework/platform tools over hand-rolled alternatives.

SG-8: Front-Load Constraints — before proposing anything: existing abstractions? platform limits?
async/performance requirements?

SG-10: Checkpoint Before Continuing — after each major step, report what's done, what's next.
```

**Project fundamentals (include in subagent prompts):**
- Typed structs over inline JSON — never use `serde_json::json!()` for data with a known structure
- Service layer owns SQL — all SQL lives in `temper-api/src/services/`
- SQL macros — use `sqlx::query!()` / `sqlx::query_as!()` for compile-time verification
- After changing SQL: regenerate cache with `cargo sqlx prepare --workspace -- --all-features`
- Unit tests: `cargo make test` / Integration tests: `cargo make test-db`
- Single test: `cargo nextest run --workspace test_name`
- Lint: `cargo make check` / Fix: `cargo make fix`
- Docker Postgres on port 5437: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`
- Feature flags: `test-db` for integration tests, `web-api` for utoipa, `mcp` for schemars

---

### Task 1: SQL migration — kb_profiles.slug column + backfill

**Files:**
- Create: `migrations/20260407000002_owner_scoped_uris.sql`

This is the first half of the migration file. Task 2 adds the URI functions to the same file.

- [ ] **Step 1: Create the migration file with slug column + backfill**

```sql
-- migrations/20260407000002_owner_scoped_uris.sql
-- Phase 2: Owner-scoped URIs — profile slugs and URI function rewrites

-- 1. Add slug column to kb_profiles
ALTER TABLE kb_profiles ADD COLUMN slug VARCHAR(64);

-- 2. Backfill from display_name: lowercase, replace non-alnum with -, trim dashes
UPDATE kb_profiles
   SET slug = lower(regexp_replace(
       regexp_replace(display_name, '[^a-zA-Z0-9]+', '-', 'g'),
       '^-+|-+$', '', 'g'
   ));

-- 3. Handle any slug collisions (defensive — unlikely with ~2 profiles)
DO $$
DECLARE
    rec RECORD;
    base_slug TEXT;
    new_slug TEXT;
    suffix INT;
BEGIN
    FOR rec IN
        SELECT id, slug
          FROM kb_profiles
         WHERE slug IN (
             SELECT slug FROM kb_profiles GROUP BY slug HAVING count(*) > 1
         )
         ORDER BY created
         OFFSET 1  -- skip the first (oldest) profile, it keeps the clean slug
    LOOP
        suffix := 2;
        base_slug := rec.slug;
        LOOP
            new_slug := base_slug || '-' || suffix;
            EXIT WHEN NOT EXISTS (
                SELECT 1 FROM kb_profiles WHERE slug = new_slug
            );
            suffix := suffix + 1;
        END LOOP;
        UPDATE kb_profiles SET slug = new_slug WHERE id = rec.id;
    END LOOP;
END $$;

-- 4. Apply constraints
ALTER TABLE kb_profiles ALTER COLUMN slug SET NOT NULL;
ALTER TABLE kb_profiles ADD CONSTRAINT kb_profiles_slug_unique UNIQUE (slug);
```

- [ ] **Step 2: Run the migration against Docker Postgres**

Run: `cargo make docker-up && sqlx migrate run`
Expected: Migration applies cleanly. Verify with:
```bash
psql "$DATABASE_URL" -c "SELECT id, display_name, slug FROM kb_profiles;"
```
Each profile should have a slug derived from its display_name.

- [ ] **Step 3: Commit**

```bash
git add migrations/20260407000002_owner_scoped_uris.sql
git commit -m "feat(db): add kb_profiles.slug column with backfill

Adds slug VARCHAR(64) NOT NULL UNIQUE to kb_profiles, backfilled from
display_name. Collision resolution appends -2, -3 etc."
```

---

### Task 2: SQL migration — kb_resource_uri() and resource_for_uri() rewrites

**Files:**
- Modify: `migrations/20260407000002_owner_scoped_uris.sql` (append to file from Task 1)

- [ ] **Step 1: Append URI function rewrites to the migration file**

Add the following to the end of `migrations/20260407000002_owner_scoped_uris.sql`:

```sql
-- 5. Rewrite kb_resource_uri() to include owner sigil
CREATE OR REPLACE FUNCTION kb_resource_uri(p_resource_id UUID)
RETURNS TEXT
LANGUAGE SQL STABLE AS $$
    SELECT 'kb://' ||
           CASE c.kb_owner_table
               WHEN 'kb_profiles' THEN '@' || p.slug
               WHEN 'kb_teams'    THEN '+' || t.slug
           END ||
           '/' || c.name ||
           '/' || dt.name ||
           '/' || COALESCE(r.slug, r.id::text)
      FROM kb_resources r
      JOIN kb_contexts c ON c.id = r.kb_context_id
      JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
      LEFT JOIN kb_profiles p
          ON p.id = c.kb_owner_id AND c.kb_owner_table = 'kb_profiles'
      LEFT JOIN kb_teams t
          ON t.id = c.kb_owner_id AND c.kb_owner_table = 'kb_teams'
     WHERE r.id = p_resource_id
$$;

-- 6. Rewrite resource_for_uri() with new format + legacy fallback
--    New format: kb://@owner/context/type/identifier
--    Legacy format: kb://context/type/uuid (no sigil)
CREATE OR REPLACE FUNCTION resource_for_uri(p_profile_id UUID, p_kb_uri TEXT)
RETURNS TABLE (
    resource_id  UUID,
    origin_uri   TEXT,
    content_hash VARCHAR(64),
    updated      TIMESTAMPTZ,
    is_active    BOOLEAN,
    access_level VARCHAR(32),
    team_role    team_role
)
LANGUAGE plpgsql STABLE AS $$
DECLARE
    parts      TEXT[];
    owner_seg  TEXT;
    ctx_name   TEXT;
    dtype_name TEXT;
    ident      TEXT;
    resolved_id UUID;
BEGIN
    -- Split kb://... into path segments
    parts := string_to_array(replace(p_kb_uri, 'kb://', ''), '/');

    IF parts[1] LIKE '@%' OR parts[1] LIKE '+%' THEN
        -- New format: kb://@owner/context/type/identifier
        owner_seg  := parts[1];
        ctx_name   := parts[2];
        dtype_name := parts[3];
        ident      := parts[4];
    ELSE
        -- Legacy format: kb://context/type/uuid
        ctx_name   := parts[1];
        dtype_name := parts[2];
        ident      := parts[3];
    END IF;

    -- Resolve identifier: try UUID first, then slug
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
    SELECT r.id AS resource_id,
           r.origin_uri,
           r.content_hash,
           r.updated,
           r.is_active,
           v.access_level,
           v.team_role
      FROM kb_resources r
      JOIN resources_visible_to(p_profile_id, NULL, ARRAY[resolved_id]) v
        ON v.resource_id = r.id
     WHERE r.id = resolved_id;
END;
$$;
```

- [ ] **Step 2: Re-run migration (reset + re-apply if needed)**

Run: `sqlx migrate run`
If the migration was already applied in Task 1, you need to reset and re-apply:
```bash
sqlx database reset -y && sqlx migrate run
```

Verify the new URI format:
```bash
psql "$DATABASE_URL" -c "
SELECT kb_resource_uri(r.id) as uri
  FROM kb_resources r
 LIMIT 5;
"
```
Expected: URIs like `kb://@petetaylor/temper/research/2026-04-07-...` or similar with `@` or `+` prefix.

- [ ] **Step 3: Regenerate sqlx query cache**

Run: `cargo sqlx prepare --workspace -- --all-features`
Expected: Cache regenerated without errors. The `.sqlx/` directory is updated.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260407000002_owner_scoped_uris.sql .sqlx/
git commit -m "feat(db): rewrite kb_resource_uri and resource_for_uri for owner-scoped URIs

kb_resource_uri() now emits kb://@profile-slug/context/type/slug-or-uuid
and kb://+team-slug/context/type/slug-or-uuid. resource_for_uri() parses
both new and legacy (no-sigil) URI formats with slug-based resolution."
```

---

### Task 3: Core type — add slug to Profile struct

**Files:**
- Modify: `crates/temper-core/src/types/profile.rs:20-30`

- [ ] **Step 1: Add slug field to Profile struct**

In `crates/temper-core/src/types/profile.rs`, add `slug` after `display_name`:

```rust
pub struct Profile {
    pub id: Uuid,
    pub display_name: String,
    pub slug: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub preferences: serde_json::Value,
    pub vault_config: serde_json::Value,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check --workspace --all-features`
Expected: May show errors in profile_service.rs where SELECT queries don't include `slug`.
Note these errors — they are fixed in Task 4.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/types/profile.rs
git commit -m "feat(core): add slug field to Profile struct"
```

---

### Task 4: Profile service — update queries + slug generation

**Files:**
- Modify: `crates/temper-api/src/services/profile_service.rs`

- [ ] **Step 1: Write the failing test for generate_profile_slug**

Add this test at the bottom of the `#[cfg(all(test, feature = "test-db"))] mod tests` block in `crates/temper-api/src/services/profile_service.rs`:

```rust
    #[sqlx::test(migrations = "../../migrations")]
    async fn generate_slug_from_display_name(pool: PgPool) {
        let slug = generate_profile_slug(&pool, "Pete Taylor").await.unwrap();
        assert_eq!(slug, "pete-taylor");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn generate_slug_handles_special_chars(pool: PgPool) {
        let slug = generate_profile_slug(&pool, "José García-López").await.unwrap();
        assert_eq!(slug, "jos-garc-a-l-pez");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn generate_slug_handles_collision(pool: PgPool) {
        // Create a profile that will own the "collider" slug
        let claims = AuthClaims {
            provider: "test".to_string(),
            external_user_id: "slug-collision-1".to_string(),
            email: "collider@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile = resolve_from_claims(&pool, &claims).await.unwrap();
        assert_eq!(profile.slug, "collider");

        // Now generate a slug for the same display name — should get -2
        let slug = generate_profile_slug(&pool, "collider").await.unwrap();
        assert_eq!(slug, "collider-2");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p temper-api --features test-db generate_slug`
Expected: FAIL — `generate_profile_slug` is not defined.

- [ ] **Step 3: Implement generate_profile_slug**

Add this function above `resolve_from_claims` in `crates/temper-api/src/services/profile_service.rs`:

```rust
/// Generate a unique profile slug from a display name.
///
/// Slugifies the name (lowercase, non-alnum → dash, trim dashes),
/// then appends -2, -3, etc. if the slug already exists.
pub async fn generate_profile_slug(pool: &PgPool, display_name: &str) -> ApiResult<String> {
    let base: String = display_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let base = base.trim_matches('-').to_string();
    let base = if base.is_empty() {
        "user".to_string()
    } else {
        base
    };

    // Check if the base slug is available
    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM kb_profiles WHERE slug = $1) as \"exists!: bool\"",
        &base,
    )
    .fetch_one(pool)
    .await?;

    if !exists {
        return Ok(base);
    }

    // Find next available suffix
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        let exists = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM kb_profiles WHERE slug = $1) as \"exists!: bool\"",
            &candidate,
        )
        .fetch_one(pool)
        .await?;

        if !exists {
            return Ok(candidate);
        }
        suffix += 1;
    }
}
```

- [ ] **Step 4: Update resolve_from_claims INSERT to include slug**

In `resolve_from_claims`, after `let display_name = ...` (line 97) and before the INSERT (line 100), add slug generation and update the INSERT:

```rust
    // 5: brand new profile + auth link
    let display_name = claims.email.split('@').next().unwrap_or("user").to_string();
    let slug = generate_profile_slug(pool, &display_name).await?;

    let profile_id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO kb_profiles
            (id, display_name, slug, email, avatar_url, preferences, vault_config, is_active, created, updated)
        VALUES ($1, $2, $3, $4, null, '{}', '{}', true, now(), now())
        "#,
        profile_id,
        &display_name,
        &slug,
        &claims.email as &str,
    )
    .execute(pool)
    .await?;
```

- [ ] **Step 5: Update get_by_id SELECT to include slug**

In `get_by_id` (line 148), add `slug` to the SELECT:

```rust
    let profile = sqlx::query_as!(
        Profile,
        r#"
        SELECT id,
               display_name,
               slug,
               email,
               avatar_url,
               preferences as "preferences: serde_json::Value",
               vault_config as "vault_config: serde_json::Value",
               is_active,
               created,
               updated
          FROM kb_profiles
         WHERE id = $1
           AND is_active = true
        "#,
        id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
```

- [ ] **Step 6: Regenerate sqlx cache and run tests**

Run:
```bash
cargo sqlx prepare --workspace -- --all-features
cargo nextest run -p temper-api --features test-db generate_slug
```
Expected: All three `generate_slug_*` tests PASS.

- [ ] **Step 7: Run full test suite to check for breakage**

Run: `cargo make test && cargo make test-db`
Expected: All tests pass. If any SELECT queries elsewhere are missing `slug`, fix them
by adding `slug` to the column list.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/src/services/profile_service.rs .sqlx/
git commit -m "feat(api): generate profile slugs on creation with collision handling

Adds generate_profile_slug() helper and updates resolve_from_claims to
store slug on new profiles. Updates all kb_profiles SELECT queries to
include the slug column. 3 new integration tests."
```

---

### Task 5: Integration tests — URI functions

**Files:**
- Create: `tests/e2e/tests/owner_scoped_uri_test.rs`

These tests verify the SQL functions produce correct URIs after the migration.

- [ ] **Step 1: Write integration tests for kb_resource_uri and resource_for_uri**

Create `tests/e2e/tests/owner_scoped_uri_test.rs`:

```rust
//! Integration tests for owner-scoped URI functions.
//!
//! Verifies that kb_resource_uri() produces the new @owner/+team format
//! and that resource_for_uri() resolves both new and legacy URI formats.

use sqlx::PgPool;
use uuid::Uuid;

/// Helper: create a profile and return (profile_id, slug).
async fn create_test_profile(pool: &PgPool, display_name: &str, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_profiles (id, display_name, slug, email, is_active, created, updated)
         VALUES ($1, $2, $3, $4, true, now(), now())",
    )
    .bind(id)
    .bind(display_name)
    .bind(slug)
    .bind(format!("{slug}@test.com"))
    .execute(pool)
    .await
    .unwrap();
    id
}

/// Helper: create a team and return team_id.
async fn create_test_team(pool: &PgPool, slug: &str, created_by: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_teams (id, name, slug, description, is_active, created_by_profile_id, created, updated)
         VALUES ($1, $2, $3, $4, true, $5, now(), now())",
    )
    .bind(id)
    .bind(slug)
    .bind(slug)
    .bind(format!("Test team {slug}"))
    .bind(created_by)
    .execute(pool)
    .await
    .unwrap();

    // Add creator as owner
    sqlx::query(
        "INSERT INTO kb_team_members (id, team_id, profile_id, role, created, updated)
         VALUES ($1, $2, $3, 'owner', now(), now())",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(created_by)
    .execute(pool)
    .await
    .unwrap();

    id
}

/// Helper: create a context and return context_id.
async fn create_test_context(
    pool: &PgPool,
    name: &str,
    owner_table: &str,
    owner_id: Uuid,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id, created, updated)
         VALUES ($1, $2, $3, $4, now(), now())",
    )
    .bind(id)
    .bind(name)
    .bind(owner_table)
    .bind(owner_id)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// Helper: create a resource and return resource_id.
async fn create_test_resource(
    pool: &PgPool,
    context_id: Uuid,
    doc_type_name: &str,
    slug: &str,
    profile_id: Uuid,
) -> Uuid {
    let doc_type_id: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_doc_types WHERE name = $1")
            .bind(doc_type_name)
            .fetch_one(pool)
            .await
            .unwrap();

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, title, slug, origin_uri, content_hash, created_by_profile_id, is_active, created, updated)
         VALUES ($1, $2, $3, $4, $5, '', '', $6, true, now(), now())",
    )
    .bind(id)
    .bind(context_id)
    .bind(doc_type_id)
    .bind(slug)
    .bind(slug)
    .bind(profile_id)
    .execute(pool)
    .await
    .unwrap();
    id
}

#[sqlx::test(migrations = "../migrations")]
async fn kb_resource_uri_includes_profile_owner(pool: PgPool) {
    let profile_id = create_test_profile(&pool, "Alice Test", "alice-test").await;
    let ctx_id = create_test_context(&pool, "myproject", "kb_profiles", profile_id).await;
    let resource_id =
        create_test_resource(&pool, ctx_id, "research", "test-research", profile_id).await;

    let uri: String =
        sqlx::query_scalar("SELECT kb_resource_uri($1)")
            .bind(resource_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(uri, "kb://@alice-test/myproject/research/test-research");
}

#[sqlx::test(migrations = "../migrations")]
async fn kb_resource_uri_includes_team_owner(pool: PgPool) {
    let profile_id = create_test_profile(&pool, "Bob Test", "bob-test").await;
    let team_id = create_test_team(&pool, "platform-eng", profile_id).await;
    let ctx_id = create_test_context(&pool, "general", "kb_teams", team_id).await;
    let resource_id =
        create_test_resource(&pool, ctx_id, "decision", "test-decision", profile_id).await;

    let uri: String =
        sqlx::query_scalar("SELECT kb_resource_uri($1)")
            .bind(resource_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(uri, "kb://+platform-eng/general/decision/test-decision");
}

#[sqlx::test(migrations = "../migrations")]
async fn kb_resource_uri_falls_back_to_uuid_when_no_slug(pool: PgPool) {
    let profile_id = create_test_profile(&pool, "Carol Test", "carol-test").await;
    let ctx_id = create_test_context(&pool, "myctx", "kb_profiles", profile_id).await;

    // Create resource with NULL slug
    let doc_type_id: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_doc_types WHERE name = 'research'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let resource_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, title, slug, origin_uri, content_hash, created_by_profile_id, is_active, created, updated)
         VALUES ($1, $2, $3, 'No Slug', NULL, '', '', $4, true, now(), now())",
    )
    .bind(resource_id)
    .bind(ctx_id)
    .bind(doc_type_id)
    .bind(profile_id)
    .execute(&pool)
    .await
    .unwrap();

    let uri: String =
        sqlx::query_scalar("SELECT kb_resource_uri($1)")
            .bind(resource_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(
        uri.starts_with("kb://@carol-test/myctx/research/"),
        "URI should start with owner-scoped prefix, got: {uri}"
    );
    assert!(
        uri.ends_with(&resource_id.to_string()),
        "URI should end with UUID when no slug, got: {uri}"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn resource_for_uri_resolves_new_format(pool: PgPool) {
    let profile_id = create_test_profile(&pool, "Dan Test", "dan-test").await;
    let ctx_id = create_test_context(&pool, "proj", "kb_profiles", profile_id).await;
    let resource_id =
        create_test_resource(&pool, ctx_id, "research", "my-research", profile_id).await;

    let uri = "kb://@dan-test/proj/research/my-research";

    let resolved: Option<Uuid> = sqlx::query_scalar(
        "SELECT resource_id FROM resource_for_uri($1, $2)",
    )
    .bind(profile_id)
    .bind(uri)
    .fetch_optional(&pool)
    .await
    .unwrap();

    assert_eq!(resolved, Some(resource_id));
}

#[sqlx::test(migrations = "../migrations")]
async fn resource_for_uri_resolves_legacy_format(pool: PgPool) {
    let profile_id = create_test_profile(&pool, "Eve Test", "eve-test").await;
    let ctx_id = create_test_context(&pool, "proj", "kb_profiles", profile_id).await;
    let resource_id =
        create_test_resource(&pool, ctx_id, "research", "legacy-test", profile_id).await;

    // Legacy format uses UUID directly
    let legacy_uri = format!("kb://proj/research/{resource_id}");

    let resolved: Option<Uuid> = sqlx::query_scalar(
        "SELECT resource_id FROM resource_for_uri($1, $2)",
    )
    .bind(profile_id)
    .bind(&legacy_uri)
    .fetch_optional(&pool)
    .await
    .unwrap();

    assert_eq!(resolved, Some(resource_id));
}

#[sqlx::test(migrations = "../migrations")]
async fn resource_for_uri_resolves_by_uuid_in_new_format(pool: PgPool) {
    let profile_id = create_test_profile(&pool, "Frank Test", "frank-test").await;
    let ctx_id = create_test_context(&pool, "proj", "kb_profiles", profile_id).await;
    let resource_id =
        create_test_resource(&pool, ctx_id, "research", "uuid-test", profile_id).await;

    // New format but using UUID instead of slug
    let uri = format!("kb://@frank-test/proj/research/{resource_id}");

    let resolved: Option<Uuid> = sqlx::query_scalar(
        "SELECT resource_id FROM resource_for_uri($1, $2)",
    )
    .bind(profile_id)
    .bind(&uri)
    .fetch_optional(&pool)
    .await
    .unwrap();

    assert_eq!(resolved, Some(resource_id));
}
```

- [ ] **Step 2: Check the e2e test crate's Cargo.toml for the right setup**

Read `tests/e2e/Cargo.toml` to confirm it has `sqlx` with the `runtime-tokio` feature and
the `migrations` feature. The test file uses `#[sqlx::test]` which requires these.

- [ ] **Step 3: Run the URI integration tests**

Run: `cargo nextest run -p temper-e2e --features test-db owner_scoped_uri`
Expected: All 6 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/tests/owner_scoped_uri_test.rs
git commit -m "test: add integration tests for owner-scoped URI functions

6 tests covering kb_resource_uri() with profile/team owners,
UUID fallback, and resource_for_uri() with new format, legacy
format, and UUID-in-new-format resolution."
```

---

### Task 6: Core type — Subscription.owner + resolved_owner()

**Files:**
- Modify: `crates/temper-core/src/types/vault_config.rs:31-52`

- [ ] **Step 1: Write failing tests for resolved_owner**

Add these tests inside the existing `#[cfg(test)] mod tests` block in `crates/temper-core/src/types/vault_config.rs`:

```rust
    #[test]
    fn resolved_owner_uses_explicit_owner() {
        let sub = Subscription {
            context: "temper".to_string(),
            owner: Some("@alice".to_string()),
            team: Some("ignored-team".to_string()),
            doc_types: None,
            auto_sync: false,
            merge_policy: MergePolicy::Manual,
            local_paths: vec![],
            repos: vec![],
        };
        assert_eq!(sub.resolved_owner(), "@alice");
    }

    #[test]
    fn resolved_owner_falls_back_to_team() {
        let sub = Subscription {
            context: "general".to_string(),
            owner: None,
            team: Some("platform-eng".to_string()),
            doc_types: None,
            auto_sync: false,
            merge_policy: MergePolicy::Manual,
            local_paths: vec![],
            repos: vec![],
        };
        assert_eq!(sub.resolved_owner(), "+platform-eng");
    }

    #[test]
    fn resolved_owner_defaults_to_at_me() {
        let sub = Subscription {
            context: "temper".to_string(),
            owner: None,
            team: None,
            doc_types: None,
            auto_sync: false,
            merge_policy: MergePolicy::Manual,
            local_paths: vec![],
            repos: vec![],
        };
        assert_eq!(sub.resolved_owner(), "@me");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p temper-core resolved_owner`
Expected: FAIL — `owner` field does not exist on `Subscription`.

- [ ] **Step 3: Add owner field and resolved_owner method**

In `crates/temper-core/src/types/vault_config.rs`, update the `Subscription` struct to add the `owner` field after `team`:

```rust
pub struct Subscription {
    /// Which kb_context this subscription targets
    pub context: String,
    /// Owner sigil (@profile-slug or +team-slug). Defaults to @me.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Team-owned context (None = profile-owned). Deprecated: use owner instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    /// Doc type filter (None = all types)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_types: Option<Vec<String>>,
    /// Run local-only manifest pre-flight on every temper command
    #[serde(default)]
    pub auto_sync: bool,
    /// Conflict resolution policy for this subscription
    #[serde(default)]
    pub merge_policy: MergePolicy,
    /// Local directories mapped to this context
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_paths: Vec<String>,
    /// Git repos associated with this context (owner/repo or local paths)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<String>,
}

impl Subscription {
    /// Resolve the effective owner for this subscription.
    ///
    /// Priority: explicit `owner` > legacy `team` (as `+team`) > `@me`.
    pub fn resolved_owner(&self) -> String {
        if let Some(owner) = &self.owner {
            return owner.clone();
        }
        if let Some(team) = &self.team {
            return format!("+{team}");
        }
        "@me".to_string()
    }
}
```

- [ ] **Step 4: Fix existing tests that construct Subscription without owner**

Update the four existing tests in the same file that construct `Subscription` to include `owner: None`:

In `full_config_round_trips` (two Subscription instances):
```rust
                Subscription {
                    context: "temper".to_string(),
                    owner: None,
                    team: None,
                    // ... rest unchanged
                },
                Subscription {
                    context: "storyteller".to_string(),
                    owner: None,
                    team: Some("narrative-team".to_string()),
                    // ... rest unchanged
                },
```

In `subscription_skips_none_fields`:
```rust
        let sub = Subscription {
            context: "temper".to_string(),
            owner: None,
            team: None,
            // ... rest unchanged
        };
```

- [ ] **Step 5: Fix any other Subscription construction sites across the codebase**

Run: `cargo check --workspace --all-features`
Any compilation error about missing `owner` field in a Subscription construction — add `owner: None`.

- [ ] **Step 6: Run tests**

Run: `cargo nextest run -p temper-core resolved_owner`
Expected: All 3 `resolved_owner_*` tests PASS.

Run: `cargo make test`
Expected: Full unit test suite passes.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/src/types/vault_config.rs
git commit -m "feat(core): add Subscription.owner field with resolved_owner() method

Adds optional owner field (sigil format: @profile or +team) with
resolution priority: explicit owner > legacy team as +team > @me.
Existing team field preserved for TOML compat."
```

---

### Task 7: Frontmatter schema — add temper-owner

**Files:**
- Modify: `crates/temper-core/schemas/base.schema.json`
- Modify: `crates/temper-core/src/schema.rs:271-280`

- [ ] **Step 1: Add temper-owner to base.schema.json**

In `crates/temper-core/schemas/base.schema.json`, add `temper-owner` after the `temper-source` property (after line 32):

```json
    "temper-owner": {
      "type": "string",
      "pattern": "^[@+][a-z0-9][a-z0-9-]*$",
      "description": "Owner sigil: @profile-slug or +team-slug. Defaults to @me."
    },
```

- [ ] **Step 2: Add temper-owner to SYSTEM_MANAGED_FIELDS**

In `crates/temper-core/src/schema.rs`, update the `SYSTEM_MANAGED_FIELDS` array to include `temper-owner`:

```rust
pub static SYSTEM_MANAGED_FIELDS: &[&str] = &[
    "temper-id",
    "temper-provisional-id",
    "temper-type",
    "temper-context",
    "temper-owner",
    "temper-created",
    "temper-updated",
    "temper-source",
    "temper-legacy-id",
    "slug",
];
```

Note: `temper-context` was already in the schema but missing from SYSTEM_MANAGED_FIELDS — add it too since it's a system-managed `temper-*` field that should not be user-editable.

- [ ] **Step 3: Verify compilation and run tests**

Run: `cargo make check && cargo make test`
Expected: All checks pass, all tests pass. The schema change is backward-compatible since the field is optional.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/schemas/base.schema.json crates/temper-core/src/schema.rs
git commit -m "feat(core): add temper-owner to frontmatter schema and SYSTEM_MANAGED_FIELDS

Optional field with pattern ^[@+][a-z0-9][a-z0-9-]*$, defaults to @me
when absent. Also adds temper-context to SYSTEM_MANAGED_FIELDS."
```

---

### Task 8: Full verification + sqlx cache

**Files:**
- Modify: `.sqlx/` (regenerated cache)

- [ ] **Step 1: Regenerate sqlx query cache**

Run: `cargo sqlx prepare --workspace -- --all-features`
Expected: Cache regenerated successfully.

- [ ] **Step 2: Run cargo make check**

Run: `cargo make check`
Expected: All formatting, clippy, docs, TypeScript checks pass.

- [ ] **Step 3: Run full test suite**

Run: `cargo make test && cargo make test-db`
Expected: All unit and integration tests pass.

- [ ] **Step 4: Run e2e tests**

Run: `cargo make test-e2e`
Expected: All e2e tests pass, including the new owner_scoped_uri tests.

- [ ] **Step 5: Commit any remaining sqlx cache updates**

```bash
git add .sqlx/
git commit -m "chore: regenerate sqlx query cache for owner-scoped URI changes"
```

---

## Task Dependency Order

```
Task 1 (migration: slug) → Task 2 (migration: URI functions) → Task 3 (Profile.slug type)
    → Task 4 (profile service) → Task 5 (URI integration tests)
Task 6 (Subscription.owner) — independent, can run in parallel with Tasks 3-5
Task 7 (frontmatter schema) — independent, can run in parallel with Tasks 3-6
Task 8 (full verification) — depends on all previous tasks
```

Parallelizable groups:
- **Sequential:** Tasks 1 → 2 → 3 → 4 → 5
- **Independent:** Task 6 (after Task 2 for sqlx cache), Task 7 (any time)
- **Final gate:** Task 8
