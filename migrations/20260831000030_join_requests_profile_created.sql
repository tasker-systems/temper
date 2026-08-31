-- A covering index so the rate-limit seam's self-service guard COUNT stays indexed as
-- history grows.
--
-- The guard (`rate_limit::guard_join_request`) runs a windowed COUNT over
-- kb_join_requests(requesting_profile_id, created) BEFORE any read or write on
-- POST /api/access/requests. The table's only indexes lead with team_id (partial on
-- status='pending') or status — neither usable by that predicate, so the guard was a
-- sequential scan per guarded request over the audit trail, which is unbounded by
-- design (spec A3). Spec A2 authorizes a covering index "if measured necessary": the
-- necessity is structural, not a measurement — the scan's cost grows with history on
-- the exact door the seam bounds, so the bound must not itself be the growing cost.
--
-- Additive; serves no query outside the seam's guard today.

CREATE INDEX idx_join_requests_profile_created
    ON kb_join_requests (requesting_profile_id, created);

SELECT declare_migration(
    20260831000030,
    'additive',
    'Covering index for the rate-limit seam''s self-service guard: idx_join_requests_profile_created '
    'on kb_join_requests (requesting_profile_id, created), so the guard''s windowed COUNT — which '
    'runs before any read or write on POST /api/access/requests — is served by index instead of a '
    'sequential scan whose cost grows with the unbounded audit history. No existing column, '
    'constraint or function is altered.'
);
