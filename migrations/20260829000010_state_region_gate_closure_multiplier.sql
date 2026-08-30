-- State the region gate's team-closure cost where it is paid.
--
-- Region visibility on the wayfind path is recomputed rather than hoisted: every
-- wayfind_region_scores invocation walks visible_region_anchors into the two closure-bearing
-- readers, and a composed plan invokes it once per survey stage. That multiplier was derivable
-- only by walking the chain by hand; it is now stated on the two functions that pay it, and the
-- per-stage count is pinned by crates/temper-substrate/tests/query_plan_compile.rs
-- (survey_s_region_gate_pays_two_team_closures_per_stage_and_the_count_is_pinned), which asserts
-- each hop and fails in either direction if the count moves.
--
-- Comment-only: same objects, same signatures, same bodies, same behaviour. No function body,
-- grant, table or column changes; safe to apply ahead of any binary.

COMMENT ON FUNCTION visible_region_anchors(uuid) IS
    'The anchors a principal may pool regions from, over BOTH kinds (spec §3.7). Replaces '
    'cogmap_visible_maps as wayfind''s admission gate. UNION ALL is safe: the two arms are disjoint '
    'by anchor_table, and each source already de-duplicates within its own kind. COST, stated where '
    'it is paid: every call evaluates BOTH arms — a UNION ALL does not short-circuit — and each arm '
    'computes the recursive team closure (cogmap_visible_maps calls profile_reachable_teams once; '
    'contexts_readable_by, through contexts_readable_by_teams, once), so ONE call costs TWO '
    'recursive team closures. wayfind_region_scores enters this once per invocation and a composed '
    'plan invokes it once per survey stage, so an N-survey-stage statement pays 2N team closures '
    'for region visibility, beside the same statement''s single hoisted resource gate.';

COMMENT ON FUNCTION wayfind_region_scores(uuid, uuid, vector, int, varchar, uuid) IS
    'The ONE home for wayfind Stage-1 region scoring (issue #585 Task 2). Returns one row per candidate '
    'region across the principal''s visible anchors — per-kind sal_norm, NaN-guarded query_cos, the '
    'composite region_score, and in_top_n (cleared the per-MAP round-robin cut). Scoring is T7''s blend '
    '(20260712000090) verbatim; the added step is per-map round-robin selection so no single map can '
    'monopolize the top-N while a sibling''s competitive champion sits below the cut. '
    'wayfind_region_diagnostics (adds map names) and the survey act''s core read from here, so their '
    'selection cannot drift. Visibility-gated; deny ⇒ zero rows. No cold-start / member-deref '
    'here — those are scope assembly. COST: every invocation enters visible_region_anchors once — '
    'two recursive team closures — and a composed plan invokes this once per survey stage, so N '
    'survey stages pay 2N team closures for the region gate beside the statement''s one hoisted '
    'resource gate.';

SELECT declare_migration(
    20260829000010,
    'additive',
    'comment-only: states the region gate''s team-closure cost on the two functions that pay it '
    '(visible_region_anchors, wayfind_region_scores). No body, signature or behaviour change.'
);
