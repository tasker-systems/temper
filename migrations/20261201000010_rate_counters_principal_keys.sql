-- Present-truth comment for kb_rate_counters, which now holds two kinds of key. Comment only: no
-- column, constraint or function changes.
--
-- The rate-limit seam's per-principal doors (temper-services rate_limit::PrincipalDoor) count in
-- this table beside the reconcile pair: a key is either a request path (the reconcile pair, always
-- beginning with '/') or 'principal:<door>:<profile id>'. Both are bumped by the one atomic upsert,
-- rate_limit::bump_route. A later edit must keep the two key spaces disjoint.

COMMENT ON TABLE kb_rate_counters IS
  'Windowed call counter for the rate-limit seam, where no canonical artifact exists to count from. '
  'One row per key: a request path for the reconcile pair (keyed per route), or '
  '''principal:<door>:<profile id>'' for a door budgeted per principal (the blob write doors). '
  'window_started_at and call_count roll together in the single atomic upsert in temper-services '
  'rate_limit::bump_route, safe under PgBouncer transaction mode. A door''s rows are written only '
  'when the operator sets that door''s RATE_LIMIT_* pair (default off). The self-service join-request '
  'door counts kb_join_requests directly and does not use this table.';

SELECT declare_migration(
    20261201000010,
    'additive',
    'Rewrites the kb_rate_counters table comment to name both key kinds it now holds: request paths '
    'for the reconcile pair and principal:<door>:<profile id> for the per-principal doors. Comment '
    'only; no table, column, constraint, function or index changes.'
);
