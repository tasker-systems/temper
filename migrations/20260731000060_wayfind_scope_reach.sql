-- Wayfind reach legibility (issue #585, goal 019fb559-7191-75a3-99d4-879090c60e94; Task 4 of the
-- cross-map-wayfind goal — the `reach-is-legible` / `controls-do-not-mislead` clauses).
--
-- THE DEFECT. A wayfind response's only reach signal is `scope_size`, which the service computes as
-- `scope_ids.len()` — a count of RESOURCES. A monopolized wayfind and a fair one both report a figure
-- in the hundreds, so the caller cannot tell them apart, and the large number reads as health. On the
-- production corpus this was measured directly: before Task 2's round-robin, one map held 12/12 region
-- slots while `scope_size` still reported several hundred resources.
--
-- The second half is the knob. Since Task 2, Stage-1 admits at most ONE region per anchor per round,
-- so the number of anchors a query can reach is bounded by the effective region width. That width is
-- SQL-resident (`default_n` = 3, `max_n` = 20 in wayfind_region_scores) and the caller is told neither
-- the default nor that it was clamped — `--regions`' help says only "default and ceiling are
-- server-side". A caller who passes nothing gets a three-anchor ceiling and no way to observe it.
--
-- THE FIX — report reach in ANCHORS, and report the width actually applied. Not by copying `default_n`
-- / `max_n` into the CLI (that would duplicate the SQL-resident tuning constants this codebase
-- deliberately keeps in one place, and the copy would drift), but by returning the EFFECTIVE width the
-- SQL clamped to, alongside how many anchors were visible and how many actually contributed.
-- `3 of 10 anchors reached, region width 3` is legible where `scope_size: 340` is not.
--
-- WHY A NEW FUNCTION, AND WHY wayfind_scope_ids BECOMES A WRAPPER. `wayfind_scope_ids` returns
-- `SETOF uuid`; widening it to carry the scalars would change its return type, and a SQL function's
-- return type is a wire contract with the running binary (an old binary calling a re-shaped function
-- fails at the call, not at deploy). So the scalars go in a NEW function, and `wayfind_scope_ids` is
-- re-pointed at it with its signature untouched — the same move Task 2 made when it collapsed the
-- duplicated scoring into `wayfind_region_scores`. The alternative (leave `wayfind_scope_ids` alone and
-- call a second function beside it) would run Stage-1 TWICE per wayfind query and leave two copies of
-- the assembly to drift. One home, one scan.
--
-- WHAT "REACHED" MEANS, AND THE ARM THAT IS EASY TO MISS. An anchor contributes to the scope by EITHER
-- path: its region cleared the round-robin cut and its members were dereferenced, OR it is region-less
-- and its directly-homed resources came in through cold-start (§5). Counting only the region winners
-- would UNDERSTATE reach by exactly the cold-start anchors — the fresh, thin ones the cold-start arm
-- exists to keep reachable — and would report a fresh anchor as unreached in the one case where the
-- substrate went out of its way to reach it. Both arms count.
--
-- Deploy safety: purely additive. One new function; one CREATE OR REPLACE of an existing function with
-- an UNCHANGED signature and unchanged semantics (identical id set — the wrapper's body is the previous
-- body verbatim, relocated). `additive` per the schema/binary pairing design.

-- ---------------------------------------------------------------------------
-- 1. wayfind_scope_reach — scope assembly + the reach scalars, in one pass.
--
-- The assembly CTEs (top_regions / region_ids / vanchors / thin_anchors / direct_ids) are
-- 20260731000050's wayfind_scope_ids body VERBATIM; what is added is per-anchor attribution
-- (`contributing`) and the three scalars over it. Returns exactly one row, always — an empty scope is
-- `{}` with zeroed counts, never zero rows, so a caller can always read the reach figures. That
-- matters: "no anchors reached" is precisely the case the caller most needs reported, and a function
-- that returned no rows there would make the honest answer indistinguishable from a missing one.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION wayfind_scope_reach(
    p_principal uuid, p_lens uuid, p_emb vector, p_regions_n int,
    p_anchor_table varchar DEFAULT NULL, p_anchor_id uuid DEFAULT NULL)
RETURNS TABLE(
    scope_ids         uuid[],
    anchors_visible   int,
    anchors_reached   int,
    regions_effective int)
LANGUAGE sql STABLE AS $$
  WITH
  -- Stage-1 winners from the single scoring home (20260731000050). Nothing is scored here.
  top_regions AS (
    SELECT s.region_id, s.home_anchor_table, s.home_anchor_id
    FROM wayfind_region_scores(p_principal, p_lens, p_emb, p_regions_n, p_anchor_table, p_anchor_id) s
    WHERE s.in_top_n
  ),
  -- Winners dereferenced to their visible members, CARRYING the home anchor so the resource can be
  -- attributed. This is the one addition to the assembly: the previous body discarded the attribution.
  region_ids AS (
    SELECT m.member_id AS resource_id, t.home_anchor_table AS anchor_table, t.home_anchor_id AS anchor_id
    FROM kb_cogmap_region_members m
    JOIN top_regions t ON t.region_id = m.region_id
    WHERE m.member_table = 'kb_resources'
      AND m.member_id IN (SELECT resource_id FROM resources_visible_to(p_principal))
  ),
  vanchors AS (
    SELECT a.anchor_table, a.anchor_id
    FROM visible_region_anchors(p_principal) a
    WHERE (p_anchor_table IS NULL OR a.anchor_table = p_anchor_table)
      AND (p_anchor_id    IS NULL OR a.anchor_id    = p_anchor_id)
  ),
  thin_anchors AS (
    SELECT v.anchor_table, v.anchor_id
    FROM vanchors v
    WHERE (SELECT count(*) FROM kb_cogmap_regions r
           WHERE r.home_anchor_table = v.anchor_table
             AND r.home_anchor_id    = v.anchor_id
             AND NOT r.is_folded) = 0   -- region-less (T7 thin_threshold 0)
  ),
  direct_ids AS (
    SELECT h.resource_id, t.anchor_table, t.anchor_id
    FROM kb_resource_homes h
    JOIN thin_anchors t
      ON t.anchor_table = h.anchor_table AND t.anchor_id = h.anchor_id
    WHERE h.resource_id IN (SELECT resource_id FROM resources_visible_to(p_principal))
  ),
  -- BOTH arms. A region-less anchor reached only through cold-start is reached.
  contributing AS (
    SELECT resource_id, anchor_table, anchor_id FROM region_ids
    UNION
    SELECT resource_id, anchor_table, anchor_id FROM direct_ids
  ),
  -- The id set the wrapper returns. DISTINCT because one resource can be a member of regions in two
  -- anchors (and of a region AND a thin anchor's direct home), which must widen neither the scope nor
  -- the resource count — but DOES count as reaching both anchors, which is why the distinct-anchor
  -- count is taken over `contributing` and not over this.
  ids AS (SELECT DISTINCT resource_id FROM contributing),
  -- The effective width. Re-derives the clamp rather than reading it back, because the clamp is applied
  -- inside wayfind_region_scores' `n` CTE and is not otherwise observable. The literals here MUST track
  -- `k`.default_n / `k`.max_n in 20260731000050; `wayfind_effective_width_tracks_the_scoring_clamp`
  -- (crates/temper-substrate/tests/cogmap_wayfind_scope.rs) pins them to the scoring function's own
  -- behaviour so the two cannot drift silently.
  width AS (SELECT GREATEST(LEAST(COALESCE(p_regions_n, 3), 20), 1) AS regions_effective)
  SELECT
    COALESCE((SELECT array_agg(resource_id) FROM ids), '{}'::uuid[]),
    (SELECT count(*)::int FROM vanchors),
    (SELECT count(DISTINCT (anchor_table, anchor_id))::int FROM contributing),
    (SELECT regions_effective FROM width);
$$;

COMMENT ON FUNCTION wayfind_scope_reach(uuid, uuid, vector, int, varchar, uuid) IS
    'Wayfind scope assembly PLUS the reach scalars a caller needs to tell a monopolized result from a '
    'fair one (issue #585 Task 4). Exactly one row, always — an empty scope is ''{}'' with zeroed '
    'counts, never zero rows. scope_ids is the bounding resource-id set (identical to '
    'wayfind_scope_ids, which is now a wrapper over this); anchors_visible counts the principal''s '
    'visible region anchors after any single-anchor scoping; anchors_reached counts those that actually '
    'contributed a resource, by EITHER the region-winner or the cold-start arm; regions_effective is '
    'the region width after the SQL clamp, so a caller can observe the default and the ceiling without '
    'the tuning constants being copied out of SQL. Visibility-gated at every stage; deny ⇒ empty scope '
    'and zero counts, never an error.';

-- ---------------------------------------------------------------------------
-- 2. wayfind_scope_ids — unchanged signature, unchanged result, now a wrapper.
--
--    The id set is bit-for-bit what the previous body produced (its CTEs are inlined above); this only
--    moves where they live, so a binary that does not carry this migration is unaffected.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION wayfind_scope_ids(
    p_principal uuid, p_lens uuid, p_emb vector, p_regions_n int,
    p_anchor_table varchar DEFAULT NULL, p_anchor_id uuid DEFAULT NULL)
RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
  SELECT unnest(r.scope_ids)
  FROM wayfind_scope_reach(p_principal, p_lens, p_emb, p_regions_n, p_anchor_table, p_anchor_id) r;
$$;

COMMENT ON FUNCTION wayfind_scope_ids(uuid, uuid, vector, int, varchar, uuid) IS
    'The bounding resource-id set for a wayfind pass, over BOTH anchor kinds (spec §3.7). Since Task 4 '
    'a thin wrapper over wayfind_scope_reach, which owns the assembly (region-winner member-deref UNION '
    'cold-start direct homes) and additionally reports the reach scalars; scoring and per-map '
    'round-robin selection live one level further down in wayfind_region_scores (Task 2). Signature and '
    'result unchanged across both moves. Every stage visibility-gated; deny ⇒ zero rows, never an '
    'error. p_anchor_table/p_anchor_id scope the pool to one anchor; NULL ⇒ every visible anchor.';

-- Purely additive: one new function, and one same-signature CREATE OR REPLACE whose body is the
-- previous body relocated (identical id set). Nothing dropped, no shape altered; a binary that does not
-- carry this calls wayfind_scope_ids identically.
SELECT declare_migration(
    20260731000060,
    'additive',
    'Wayfind reach legibility (issue #585 Task 4). New wayfind_scope_reach returns the scope id set plus anchors_visible / anchors_reached / regions_effective; wayfind_scope_ids becomes a same-signature wrapper over it. No scoring or selection change.'
);
