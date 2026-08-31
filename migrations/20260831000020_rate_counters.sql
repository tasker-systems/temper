-- The rate-limit seam's reconcile-channel counter — the minimal state the seam mints
-- where no canonical artifact exists to count from.
--
-- Design authority: temper-artifacts/specs/2026-08-31-tmpr22-rate-limit-seam-design.md
-- (A2, A5). The reconcile pair writes no per-call row (its handlers trace only), so this
-- table is the one deliberate new state. The self-service door needs none: it counts
-- kb_join_requests itself.
--
-- LOAD-BEARING: `route` is the key and `call_count`/`window_started_at` move together in
-- ONE statement (`rate_limit::bump_route`) — single-statement because PgBouncer
-- transaction mode (the deployment's only path to Postgres) cannot hold session state,
-- and because the atomic statement is what makes concurrent callers unable to lose a
-- count or resurrect an expired window.
--
-- Default off: unread and unwritten unless the operator sets RATE_LIMIT_RECONCILE_*.

CREATE TABLE kb_rate_counters (
    route             TEXT PRIMARY KEY,
    window_started_at TIMESTAMPTZ NOT NULL,
    call_count        BIGINT  NOT NULL
);

COMMENT ON TABLE kb_rate_counters IS
  'Per-route windowed call counter for the rate-limit seam''s reconcile channel. One row per '
  'route (keyed on the request path); window_started_at and call_count roll together in the '
  'single atomic upsert in temper-services rate_limit::bump_route, safe under PgBouncer '
  'transaction mode. Unwritten unless RATE_LIMIT_RECONCILE_MAX and RATE_LIMIT_RECONCILE_WINDOW_SECS '
  'are set (default off). The self-service door counts kb_join_requests directly and does not use '
  'this table.';

SELECT declare_migration(
    20260831000020,
    'additive',
    'The rate-limit seam''s reconcile-channel counter (task '
    '01a058de-29cb-7483-b0d6-ee53c19e5ad0). New table kb_rate_counters: route TEXT PRIMARY KEY, '
    'window_started_at TIMESTAMPTZ, call_count BIGINT — one row per rate-limited internal route, '
    'updated by a single atomic upsert so window rollover and counting cannot race under PgBouncer '
    'transaction mode. Minted because the reconcile pair writes no per-call artifact to count from. '
    'Default off: unread and unwritten unless the operator configures the reconcile rate limit. No '
    'existing column, constraint or function is altered.'
);
