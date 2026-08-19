# Owner-Scoped URIs: Phase 2 Database + API + Core Types

**Date:** 2026-04-07
**Session:** 3 of the system-access-gate workstream
**Scope:** Database migration, URI function rewrites, core Rust types, frontmatter schema
**Research:** R11 — System Access Gate and Owner-Scoped URIs (sections 4.1–4.4, 4.9)
**Branch:** jct/temper-system-access-gate

---

## Summary

This session lands the data layer and shared types for owner-scoped URIs. The canonical
URI format changes from `kb://context/type/uuid` to `kb://@owner/context/type/slug-or-uuid`
where the owner segment uses `@profile-slug` for personal namespaces and `+team-slug` for
team namespaces. All changes are backward-compatible during development via a legacy
fallback in `resource_for_uri()`; the fallback is stripped before merge to main.

Out of scope: CLI vault migration, manifest path migration, `--owner` flags, MCP parameter,
`temper doctor` validation, UI. Those are Session 4+.

---

## 1. Migration: kb_profiles.slug

Add a `slug` column to `kb_profiles` with a backfill from `display_name`.

```sql
ALTER TABLE kb_profiles ADD COLUMN slug VARCHAR(64);

-- Backfill: slugify display_name (lowercase, replace non-alnum with -, trim dashes)
UPDATE kb_profiles
   SET slug = lower(regexp_replace(
       regexp_replace(display_name, '[^a-zA-Z0-9]+', '-', 'g'),
       '^-+|-+$', '', 'g'
   ));

-- Handle any collisions by appending -2, -3, etc.
-- With ~2 profiles in production this is defensive, not expected to fire.
DO $$
DECLARE
    rec RECORD;
    base_slug TEXT;
    new_slug TEXT;
    suffix INT;
BEGIN
    FOR rec IN
        SELECT id, slug FROM kb_profiles
        WHERE slug IN (
            SELECT slug FROM kb_profiles GROUP BY slug HAVING count(*) > 1
        )
        ORDER BY created
    LOOP
        suffix := 2;
        base_slug := rec.slug;
        LOOP
            new_slug := base_slug || '-' || suffix;
            EXIT WHEN NOT EXISTS (
                SELECT 1 FROM kb_profiles WHERE slug = new_slug AND id != rec.id
            );
            suffix := suffix + 1;
        END LOOP;
        UPDATE kb_profiles SET slug = new_slug WHERE id = rec.id;
    END LOOP;
END $$;

ALTER TABLE kb_profiles ALTER COLUMN slug SET NOT NULL;
ALTER TABLE kb_profiles ADD CONSTRAINT kb_profiles_slug_unique UNIQUE (slug);
```

**Slug generation on new profiles:** `profile_service::resolve_from_claims()` calls a
`generate_profile_slug()` helper that slugifies `display_name`, queries for collisions,
and appends `-2`, `-3` etc. if needed.

**No slug update endpoint this session.** Slug mutation is deferred to a future
`temper profile update --slug <slug> --slug-to <new-slug>` command following the
resource update pattern.

---

## 2. Migration: kb_resource_uri() rewrite

Replace the current function that produces `kb://context/type/uuid` with one that
includes the owner segment.

```sql
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
```

**Impact:** `sync_diff_for_device()` calls `kb_resource_uri()` internally and inherits the
new format automatically. No Rust changes needed for URI generation.

---

## 3. Migration: resource_for_uri() rewrite with legacy fallback

The function switches from SQL to plpgsql to support conditional parsing and exception
handling. Two code paths:

- **New format:** First segment after `kb://` starts with `@` or `+` — parse as
  `kb://@owner/context/type/identifier`
- **Legacy format:** First segment has no sigil — parse as `kb://context/type/uuid`
  (infer owner from requesting profile)

The identifier (last segment) is tried as UUID first, then as slug-within-context.

```sql
CREATE OR REPLACE FUNCTION resource_for_uri(p_profile_id UUID, p_kb_uri TEXT)
RETURNS TABLE (
    resource_id UUID, origin_uri TEXT, content_hash VARCHAR(64),
    updated TIMESTAMPTZ, is_active BOOLEAN, access_level VARCHAR(32),
    team_role team_role
)
LANGUAGE plpgsql STABLE AS $$
DECLARE
    parts TEXT[];
    owner_seg TEXT;
    ctx_name TEXT;
    dtype_name TEXT;
    ident TEXT;
    resolved_id UUID;
BEGIN
    parts := string_to_array(replace(p_kb_uri, 'kb://', ''), '/');

    IF parts[1] LIKE '@%' OR parts[1] LIKE '+%' THEN
        owner_seg  := parts[1];
        ctx_name   := parts[2];
        dtype_name := parts[3];
        ident      := parts[4];
    ELSE
        ctx_name   := parts[1];
        dtype_name := parts[2];
        ident      := parts[3];
    END IF;

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
    SELECT r.id, r.origin_uri, r.content_hash, r.updated, r.is_active,
           v.access_level, v.team_role
      FROM kb_resources r
      JOIN resources_visible_to(p_profile_id, NULL, ARRAY[resolved_id]) v
        ON v.resource_id = r.id
     WHERE r.id = resolved_id;
END;
$$;
```

The owner segment is parsed but used for disambiguation, not access control —
`resources_visible_to` already handles authorization.

**Before merge:** The legacy branch (no-sigil path) is removed, leaving only the new
format parser.

---

## 4. Core types: Profile.slug

Add `slug: String` to the `Profile` struct in `crates/temper-core/src/types/profile.rs`.
The field is non-optional since the migration backfills all rows and sets NOT NULL.

All `SELECT` queries against `kb_profiles` in the profile service need `slug` added to
their column lists.

A `generate_profile_slug(pool, display_name) -> String` async helper in
`profile_service.rs` handles slugification and collision resolution for new profiles.

---

## 5. Core types: Subscription.owner

Add `owner: Option<String>` to the `Subscription` struct in
`crates/temper-core/src/types/vault_config.rs`. Resolution logic:

| `owner` | `team` | Resolved owner |
|---------|--------|----------------|
| `Some("@pete")` | any | `@pete` |
| `None` | `Some("platform-eng")` | `+platform-eng` |
| `None` | `None` | `@me` |

A `resolved_owner(&self) -> String` method on `Subscription` encapsulates this so callers
don't touch the fallback logic directly.

The `team` field stays for TOML deserialization compat. Removal is a Session 4 concern
alongside `temper doctor` config migration.

---

## 6. Frontmatter: temper-owner in base schema

Add to `crates/temper-core/schemas/base.schema.json`:

```json
"temper-owner": {
  "type": "string",
  "pattern": "^[@+][a-z0-9][a-z0-9-]*$",
  "description": "Owner sigil: @profile-slug or +team-slug. Defaults to @me."
}
```

The field is optional. When absent, the CLI and sync layer treat it as `@me`. Existing
vault content remains valid without migration.

Add `"temper-owner"` to the `SYSTEM_MANAGED_FIELDS` constant in temper-core. It is tracked
by `meta_hash` alongside all other `temper-*` fields (per R11 Q4).

Validation (`temper doctor`) and backfill (`temper doctor migrate-vault`) are Session 4.

---

## 7. sqlx query cache

After all SQL changes, regenerate with `cargo sqlx prepare --workspace -- --all-features`
and commit the updated `.sqlx/` cache.

---

## Testing strategy

- **Migration tests:** Integration tests (feature `test-db`) that run the migration against
  Docker Postgres and verify:
  - Profile slugs are generated correctly from display names
  - `kb_resource_uri()` returns the new format with owner sigils
  - `resource_for_uri()` resolves both new-format and legacy-format URIs
  - Slug-based resolution works when resource has a slug
  - UUID-based resolution still works as fallback

- **Profile service tests:** Unit/integration tests for `generate_profile_slug()`:
  - Basic slugification (spaces, special chars)
  - Collision resolution (appends `-2`, `-3`)
  - Edge cases (empty display name, all-special-chars)

- **Core type tests:** Unit tests for `Subscription::resolved_owner()`:
  - All three resolution paths (owner set, team fallback, default @me)

---

## What this session does NOT touch

- CLI vault layout or path construction
- Manifest path migration
- `--owner` flag on CLI commands
- MCP `create_resource` owner parameter
- `temper doctor` validation or `migrate-vault`
- SvelteKit UI
- Removing the legacy fallback in `resource_for_uri()` (done before merge)
