-- Cognitive-map analytics read side (task 019ee5a4, WS7). Additive: two gated canonical read
-- functions, siblings to cogmap_shape. The access gate lives INSIDE each function (cogmap_shape's
-- "no view from nowhere" pattern): a principal who cannot read the map gets zero rows, never an error.

-- Per-region analytics tier: the five materialized scalar readouts, read from the stored
-- kb_cogmap_regions columns (NOT recomputed). Gate + lens filter IDENTICAL to cogmap_shape.
CREATE FUNCTION cogmap_region_metrics(
    p_cogmap uuid, p_principal_kind text, p_principal_id uuid, p_lens uuid DEFAULT NULL)
RETURNS TABLE(region_id uuid, lens_id uuid, centrality double precision,
              content_cohesion double precision, internal_tension double precision,
              reference_standing double precision, telos_alignment double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.lens_id, reg.centrality, reg.content_cohesion,
           reg.internal_tension, reg.reference_standing, reg.telos_alignment
    FROM kb_cogmap_regions reg
    WHERE reg.cogmap_id = p_cogmap
      AND NOT reg.is_folded
      AND (p_lens IS NULL OR reg.lens_id = p_lens)
      AND (
        (p_principal_kind = 'profile' AND cogmap_readable_by_profile(p_principal_id, p_cogmap))
        OR (p_principal_kind = 'cogmap' AND p_principal_id = p_cogmap)
      );
$$;

-- Map-level analytics: telos charter id + staleness + the regulation set, composed from the existing
-- canonical functions in one gated row. cogmap_staleness yields exactly one row, so the map-readable
-- gate in WHERE makes the whole function deny → zero rows. regulation defaults to [] (never SQL-null).
CREATE FUNCTION cogmap_analytics(p_cogmap uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(telos_resource_id uuid, materialized_at timestamptz,
              latest_touch timestamptz, is_stale boolean, regulation jsonb)
LANGUAGE sql STABLE AS $$
    SELECT cogmap_telos(p_cogmap),
           s.materialized_at, s.latest_touch, s.is_stale,
           COALESCE(
             (SELECT json_agg(r) FROM cogmap_regulation(p_cogmap, p_principal_kind, p_principal_id) r),
             '[]'::json)::jsonb
    FROM cogmap_staleness(p_cogmap) s
    WHERE (p_principal_kind = 'profile' AND cogmap_readable_by_profile(p_principal_id, p_cogmap))
       OR (p_principal_kind = 'cogmap' AND p_principal_id = p_cogmap);
$$;
