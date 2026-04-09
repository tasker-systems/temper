-- migrations/20260408000001_resource_for_uri_drop_legacy.sql
-- Drop the legacy no-sigil URI branch from resource_for_uri.
--
-- Before this migration, resource_for_uri accepted both:
--   - kb://@owner/<ctx>/<type>/<ident>   (owner-scoped, current canonical form)
--   - kb://+team/<ctx>/<type>/<ident>    (team-scoped, current canonical form)
--   - kb://<ctx>/<type>/<ident>          (legacy, no-sigil — resolved via ELSE branch)
--
-- The legacy form was kept in Session 3 (20260407000002_owner_scoped_uris.sql)
-- as a development-safety fallback while CLI vaults migrated to the
-- owner-segmented layout. Both Pete's vaults are now on the new layout, so
-- the fallback can be removed. Legacy URIs return an empty result after this
-- migration; clients must upgrade to owner-scoped form.
--
-- Function signature (return columns, parameter types) is identical to the
-- Session 3 definition — only the body changes.

CREATE OR REPLACE FUNCTION resource_for_uri(p_profile_id UUID, p_kb_uri TEXT)
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
    parts       TEXT[];
    ctx_name    TEXT;
    dtype_name  TEXT;
    ident       TEXT;
    resolved_id UUID;
BEGIN
    -- Split kb://... into path segments.
    parts := string_to_array(replace(p_kb_uri, 'kb://', ''), '/');

    -- Require an owner sigil on the first segment. Legacy no-sigil URIs
    -- (kb://<ctx>/<type>/<ident>) are no longer accepted.
    IF array_length(parts, 1) IS NULL
       OR (parts[1] NOT LIKE '@%' AND parts[1] NOT LIKE '+%') THEN
        RETURN;  -- Empty result for legacy or malformed URIs.
    END IF;

    ctx_name   := parts[2];
    dtype_name := parts[3];
    ident      := parts[4];

    -- Resolve identifier: try UUID first, then slug.
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
