-- Deterministic drift sweep across all team-joined cogmaps (goal 019f3220).
--
-- REUSE, not re-implement: the sweep calls the existing per-cogmap `steward_ingest_delta` via a
-- LATERAL join (DRY — one source of truth for "what counts as drift"), reading each map's own
-- watermark internally. Candidate set = team-joined cogmaps (kb_team_cogmaps), scoped through
-- `anchor_readable_by_profile(principal, ...)` — the same read gate every other query uses; the
-- steward app-principal's broad read comes from grants / access_mode=open, NOT a bypass.
--
-- ADDITIVE, additive-only-on-`main`: two new functions; nothing altered.

CREATE FUNCTION steward_candidate_cogmaps(p_principal uuid)
RETURNS TABLE(cogmap_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT DISTINCT tc.cogmap_id
      FROM kb_team_cogmaps tc
     WHERE anchor_readable_by_profile(p_principal, 'kb_cogmaps', tc.cogmap_id);
$$;

CREATE FUNCTION steward_drift_sweep(p_principal uuid, p_threshold bigint)
RETURNS TABLE(cogmap_id uuid, watermark uuid, new_resources bigint, new_events bigint)
LANGUAGE sql STABLE AS $$
    SELECT m.cogmap_id,
           cm.steward_watermark_event_id AS watermark,
           d.new_resources,
           d.new_events
      FROM steward_candidate_cogmaps(p_principal) m
      JOIN kb_cogmaps cm ON cm.id = m.cogmap_id
      CROSS JOIN LATERAL steward_ingest_delta(m.cogmap_id, cm.steward_watermark_event_id) d
     WHERE d.new_resources >= p_threshold
     ORDER BY d.new_resources DESC;
$$;
