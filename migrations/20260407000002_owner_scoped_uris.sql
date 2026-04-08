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
--    Must DROP first because language changes from SQL to plpgsql
DROP FUNCTION IF EXISTS resource_for_uri(UUID, TEXT);
CREATE FUNCTION resource_for_uri(p_profile_id UUID, p_kb_uri TEXT)
RETURNS TABLE (
    resource_id  UUID,
    origin_uri   TEXT,
    body_hash    VARCHAR(128),
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
           COALESCE(m.body_hash, '')::VARCHAR(128) AS body_hash,
           r.updated,
           r.is_active,
           v.access_level,
           v.team_role
      FROM kb_resources r
      LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
      JOIN resources_visible_to(p_profile_id, NULL, ARRAY[resolved_id]) v
        ON v.resource_id = r.id
     WHERE r.id = resolved_id;
END;
$$;
