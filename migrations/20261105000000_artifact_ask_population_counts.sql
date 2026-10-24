-- What one artifact ask counted, as one row: the counted set by shape state, how many matched, and
-- the version of the asked shape's lineage in force. `/api/query` reads it per (stage, ask) beside
-- the stage, so an empty composable stage says whether nothing was governed, nothing matched, or
-- the ask narrowed by a verdict state reconciliation has not reached.
-- Spec: temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md §2.3, §4.5,
-- §5, §6.3.
--
-- What a later edit must not break:
--   1. EXACTLY ONE ROW, whatever the population. The counts are `count(*) FILTER (WHERE ...)` with
--      no GROUP BY, so an empty population is a row of zeros; the in-force pair is a LEFT JOIN of
--      at most one row (`uq_kb_data_artifact_shapes_live`, 20260822000010). The assembler fails a
--      response whose stage asked and has no row, so a missing row is never read as zero.
--   2. CALL THE COUNTED SET, NEVER RESTATE IT. Every count ranges over
--      `__temper_ungated_artifact_ask_population` (20261104000000), the set the selection's asks
--      conjunct reads: the visible set (NULL admits nothing), the ingest floor, live artifacts, the
--      home re-check and the intent narrowing are applied there and nowhere here.
--   3. THE BY-STATE COUNTS IGNORE `shape_states`; `matched` applies the whole ask. The counted set
--      narrows by intent in its WHERE and by state only inside `matched`, which is what lets an ask
--      narrowed to `declared_satisfied` still name its unchecked rows (spec §5 item 3).
--   4. THE IN-FORCE LOOKUP IS GATED BY THE SAME HOME RE-CHECK, read through COALESCE(..., false):
--      an unreadable home, or an asked id naming no row, is a NULL pair beside zero counts. Home
--      and family are read from the asked row by id; no caller passes them.
--
-- Additive: one CREATE FUNCTION. No DROP, no CREATE OR REPLACE, no DEFAULT.

CREATE FUNCTION __temper_ungated_artifact_ask_counts(
    p_visible_ids uuid[],
    p_reader      uuid,
    p_ask         jsonb)
RETURNS TABLE (in_force_shape_id uuid, in_force_shape_version int,
               declared_satisfied bigint, declared_not_satisfied bigint,
               declared_not_yet_checked bigint, matched bigint)
LANGUAGE sql STABLE AS $$
    SELECT f.id, f.shape_version,
           n.declared_satisfied, n.declared_not_satisfied, n.declared_not_yet_checked, n.matched
      FROM (SELECT count(*) FILTER (WHERE p.shape_state = 'declared_satisfied')       AS declared_satisfied,
                   count(*) FILTER (WHERE p.shape_state = 'declared_not_satisfied')   AS declared_not_satisfied,
                   count(*) FILTER (WHERE p.shape_state = 'declared_not_yet_checked') AS declared_not_yet_checked,
                   count(*) FILTER (WHERE p.matched)                                  AS matched
              FROM __temper_ungated_artifact_ask_population(p_visible_ids, p_reader, p_ask) AS p) AS n
      LEFT JOIN (SELECT live.id, live.shape_version
                   FROM kb_data_artifact_shapes asked
                   JOIN kb_data_artifact_shapes live
                     ON live.home_anchor_table = asked.home_anchor_table
                    AND live.home_anchor_id    = asked.home_anchor_id
                    AND live.kind_owner_table  = asked.kind_owner_table
                    AND live.kind_owner_id     = asked.kind_owner_id
                    AND live.artifact_kind     = asked.artifact_kind
                    AND NOT live.is_folded
                  WHERE asked.id = (p_ask->>'shape')::uuid
                    AND COALESCE(anchor_readable_by_profile(p_reader, asked.home_anchor_table,
                                                            asked.home_anchor_id), false)) AS f
        ON true
$$;

COMMENT ON FUNCTION __temper_ungated_artifact_ask_counts(uuid[], uuid, jsonb) IS
    'UNGATED. Exactly one row for one artifact ask (one element of ValidatedAsk::bind): over its counted set, __temper_ungated_artifact_ask_population (20261104000000), how many artifacts are declared_satisfied, declared_not_satisfied and declared_not_yet_checked (the ask''s intent applied, its shape_states not), and how many matched (the whole ask applied); beside them the live row of the asked shape''s home and family, its id and shape_version, while p_reader can read that home. An empty population is zeros; an unreadable home or an absent shape is zeros and a NULL pair. The caller owes the verdict (p_visible_ids, NULL admits none) and the reader, as for the counted set. (20261105000000)';

SELECT declare_migration(
    20261105000000,
    'additive',
    'New function __temper_ungated_artifact_ask_counts(uuid[], uuid, jsonb): one row of counts over the existing counted set __temper_ungated_artifact_ask_population and the asked shape''s version in force, read by the compiled /api/query statement. No existing object is altered or replaced, and no parameter carries a DEFAULT; a deployed binary that does not call it is unaffected.'
);
