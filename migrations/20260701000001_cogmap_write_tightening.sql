-- Deliverable 3b — cogmap-write tightening (Q-A). Design:
-- docs/superpowers/specs/2026-07-01-deliverable-3b-cogmap-write-tightening-design.md
--
-- BEHAVIOR-CHANGING but not big-bang: a single forward CREATE OR REPLACE + a bounded one-time
-- snapshot. Ordered BACKFILL-FIRST then FLIP, committing atomically (one migration = one txn), so
-- there is never a committed state where a current author lacks their grant.
-- Namespace-free (no SET search_path): names resolve against the connection's search_path (public).

-- ============================================================================
-- (1) BACKFILL FIRST — snapshot today's DELIBERATE flat authors as PER-PROFILE can_write grants, so
-- #221's multi-author authoring survives the flip below. PER-PROFILE (a true snapshot; NO ongoing
-- membership-inheritance, which Q-A forbids — a member who LATER joins a snapshotted team does not
-- inherit write). auto_join_role teams (temper-system → the L0 kernel) are EXCLUDED: their membership
-- is the universal "everyone" pool, so snapshotting them would grant the whole userbase write to the
-- operator-governed kernel. The per-binding-row `t.auto_join_role IS NULL` filter means a map joined
-- to BOTH a real team and temper-system snapshots only the real-team members. granted_by = the system
-- profile (an accountable one-time admin event).
-- ============================================================================
INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id,
                              can_read, can_write, granted_by_profile_id)
SELECT DISTINCT 'kb_cogmaps', tc.cogmap_id, 'kb_profiles', tm.profile_id, true, true,
       (SELECT id FROM kb_profiles WHERE handle = 'system')
FROM kb_team_cogmaps tc
JOIN kb_teams t         ON t.id = tc.team_id
JOIN kb_team_members tm ON tm.team_id = tc.team_id
WHERE t.auto_join_role IS NULL
ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING;

-- ============================================================================
-- (2) FLIP — Q-A: cogmap authorship = explicit write grant only (no membership-implies-write).
-- Cogmaps have no owner column, so there is no ownership floor; authority is wholly explicit. Reads
-- stay membership-broad (`cogmap_readable_by_profile`, unchanged). `derived_access_profile`'s
-- cogmap/write arm delegates here BY NAME, so `can(...,'write','kb_cogmaps',…)` follows automatically.
-- Was: the flat read stub (20260629000005:5 → 20260630000002's read predicate).
-- ============================================================================
CREATE OR REPLACE FUNCTION cogmap_authorable_by_profile(p_profile uuid, p_cogmap uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT profile_explicit_grant(p_profile, 'write', 'kb_cogmaps', p_cogmap);
$$;
