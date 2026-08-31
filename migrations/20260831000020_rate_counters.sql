-- The reconcile channel's rate counter — the minimal state the rate-limit seam mints
-- where no canonical artifact exists to count from.
--
-- Design authority: temper-artifacts/specs/2026-08-31-tmpr22-rate-limit-seam-design.md
-- (A2, A5). The seam's principle is *count the canonical artifact where one exists; mint
-- counter state only where no artifact exists*. The self-service join-request door counts
-- `kb_join_requests` itself — the audit trail already states "this principal requested
-- recently", so no second bookkeeping beside it. The reconcile pair
-- (`/internal/saml/reconcile`, `/internal/principal/resolve`) writes no per-call row (its
-- handlers trace only), so this table is the one deliberate new state: one row per route,
-- carrying the current window's start and its count.
--
-- Shape, chosen minimal on purpose:
--
-- * `route` PRIMARY KEY. The key is the route path itself — spec A1 for this channel:
--   the caller is anonymous-but-secret-bearing, so the route is the only controlled
--   input, and one row per route keeps the two endpoints' budgets separate.
-- * `window_started_at` + `call_count` move together in one statement
--   (`rate_limit::bump_route`): INSERT .. ON CONFLICT DO UPDATE rolls the window and
--   bumps the count atomically, so concurrent callers can neither lose a count nor
--   resurrect an expired window. Single-statement by design: the deployment reaches
--   Postgres through PgBouncer transaction mode, where session-level state is not
--   reliable, so the counter must be manipulable as one atomic statement.
-- * No cleanup job, no retention. Two opt-in doors means two rows; the window rolls in
--   place and the table cannot grow beyond one row per rate-limited route. The
--   unbounded-growth concern this seam is often mistaken for (kb_join_requests row
--   growth) is a standing-machine constraint, tracked separately — a rate limit slows
--   growth, it never bounds it, and this table does not have that problem at all.
--
-- Default off: an instance that never sets the RATE_LIMIT_RECONCILE_* variables never
-- reads or writes this table. Empty is its steady state for the default posture.

CREATE TABLE kb_rate_counters (
    route             TEXT PRIMARY KEY,
    window_started_at TIMESTAMPTZ NOT NULL,
    call_count        BIGINT  NOT NULL
);

COMMENT ON TABLE kb_rate_counters IS
  'Per-route windowed call counter for the rate-limit seam''s reconcile channel — the one '
  'deliberately-minted counter state, existing only because the reconcile handlers write no '
  'per-call row to count from. One row per route (keyed on the request path, the only '
  'controlled input for a secret-bearing anonymous caller); window_started_at and call_count '
  'roll together in the single atomic upsert in temper-services rate_limit::bump_route, safe '
  'under PgBouncer transaction mode. Unwritten unless the operator sets RATE_LIMIT_RECONCILE_MAX '
  'and RATE_LIMIT_RECONCILE_WINDOW_SECS (default off). The self-service door does not use this '
  'table: it counts kb_join_requests itself, the canonical artifact.';

SELECT declare_migration(
    20260831000020,
    'additive',
    'The rate-limit seam''s reconcile-channel counter (task '
    '01a058de-29cb-7483-b0d6-ee53c19e5ad0). One new table kb_rate_counters: route TEXT PRIMARY '
    'KEY, window_started_at TIMESTAMPTZ, call_count BIGINT — one row per rate-limited internal '
    'route, updated by a single atomic upsert so window rollover and counting cannot race under '
    'PgBouncer transaction mode. Minted because the reconcile pair writes no per-call artifact to '
    'count from; the self-service door counts kb_join_requests directly and does not use this '
    'table. Default off: unread and unwritten unless the operator configures the reconcile rate '
    'limit. No existing column, constraint or function is altered.'
);
