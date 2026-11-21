-- The rate-limit seam's per-principal counter: one windowed count per (principal, door), for the
-- doors budgeted per caller (temper-services rate_limit::PrincipalDoor), where a refused or failed
-- call leaves no artifact to count from.
--
-- Its own table, not rows in kb_rate_counters: the key is a profile, so it is a foreign key into
-- kb_profiles that the erasure manifests' fk derivation finds, and a profile's counters go with
-- the profile. kb_rate_counters stays keyed on request paths only.
--
-- LOAD-BEARING: call_count and window_started_at move together in ONE statement
-- (rate_limit::bump_principal), for the same reason as kb_rate_counters: PgBouncer transaction
-- mode holds no session state, and the single upsert is what keeps concurrent callers from losing
-- a count or resurrecting an expired window.
--
-- Default off: unread and unwritten unless the operator sets a door's RATE_LIMIT_* pair.

CREATE TABLE kb_principal_rate_counters (
    profile_id        uuid        NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    door              text        NOT NULL,
    window_started_at timestamptz NOT NULL,
    call_count        bigint      NOT NULL,
    PRIMARY KEY (profile_id, door)
);

COMMENT ON TABLE kb_principal_rate_counters IS
  'Per-principal windowed call counter for the rate-limit seam''s per-principal doors. One row per '
  '(profile, door); door is the PrincipalDoor name (blob_write). window_started_at and call_count '
  'roll together in the single atomic upsert in temper-services rate_limit::bump_principal, safe '
  'under PgBouncer transaction mode. Unwritten unless the operator sets that door''s RATE_LIMIT_* '
  'pair (default off).';

SELECT declare_migration(
    20261201000010,
    'additive',
    'The rate-limit seam''s per-principal counter. New table kb_principal_rate_counters: profile_id '
    'uuid REFERENCES kb_profiles ON DELETE CASCADE, door text, window_started_at timestamptz, '
    'call_count bigint, primary key (profile_id, door), updated by a single atomic upsert. Default '
    'off: unwritten unless the operator configures a per-principal door. No existing table, column, '
    'constraint or function is altered.'
);
