-- A replayed refresh token is a recorded event, and the chain it belongs to can be named and ended.
--
-- Four facts the AS needs in order to act on the OAuth 2.0 Security BCP's reading of a rotated
-- token presented again (RFC 6819 5.2.2.3): that a row was retired BY ROTATION, which chain it
-- belonged to, whether that chain has been ended, and that the presentation happened.
--
-- Design, alternatives and measurements:
-- internal/superpowers/specs/2026-08-26-refresh-replay-signal-design.md
-- Task 01a0390d-335a-7440-a27a-565e0a70ccce.

-- `rotated_at` distinguishes rotation from the FOUR other writers of `revoked_at` (two Rust
-- revokers, `revokeRefreshToken`, and this feature's own chain-ending), each of which is held to
-- leaving it NULL by a test. It is set in the rotation guard's own statement; a `rotated_to`
-- descent link could not be, since the successor row does not exist yet.
--
-- `chain_id` is stamped at a chain's first token and inherited unchanged, like `chain_expires_at`
-- (20260825000010). The DEFAULT names a row written by a binary that predates the column, which the
-- paired binary then inherits — that is what keeps a chain reachable across a rolling deploy.
ALTER TABLE kb_oauth_refresh_tokens
    ADD COLUMN rotated_at TIMESTAMPTZ,
    ADD COLUMN chain_id   UUID;

-- Named explicitly rather than by the DEFAULT above. Letting the default name existing rows would
-- rely on `uuid_generate_v7` being VOLATILE — which is what makes PostgreSQL rewrite per row
-- instead of storing one constant — and that function is the pg_uuidv7 extension's on PG17 and this
-- repo's shim on PG18 (20260624000001:48-70), so the deciding property is not checkable from a PG18
-- box. Every row, not just live ones: the column is about to be NOT NULL.
UPDATE kb_oauth_refresh_tokens SET chain_id = id WHERE chain_id IS NULL;

ALTER TABLE kb_oauth_refresh_tokens
    ALTER COLUMN chain_id SET NOT NULL,
    ALTER COLUMN chain_id SET DEFAULT uuid_generate_v7();

COMMENT ON COLUMN kb_oauth_refresh_tokens.rotated_at IS
  'Set in the rotation guard''s own statement when this token was retired BY ROTATION and a '
  'successor was minted. revoked_at cannot say it alone: rotation is one of five writers of that '
  'column and the other four all leave this one NULL. A presented token carrying it is a REPLAY, '
  'recorded in kb_oauth_refresh_replays.';

COMMENT ON COLUMN kb_oauth_refresh_tokens.chain_id IS
  'Identity of the refresh CHAIN, stamped at its first token and inherited unchanged by every '
  'successor, so a response to a replay reaches the chain and not merely the row presented.';

COMMENT ON COLUMN kb_oauth_refresh_tokens.rotated_to IS
  'Unwritten, by design: rotated_at carries what this column was declared for, and can be set in '
  'the guard statement where a descent link cannot. Kept because the chain TOPOLOGY it would record '
  'is a capability chain_id does not provide; read nothing into its NULLs.';

-- Every LIVE token of one chain — the replay responder's access path. Partial for the reason
-- idx_kb_oauth_refresh_tokens_live_profile (20260825000010:89-91) is: the table keeps every token
-- ever issued and the revoker only asks about live rows.
CREATE INDEX idx_kb_oauth_refresh_tokens_live_chain
    ON kb_oauth_refresh_tokens (chain_id)
 WHERE revoked_at IS NULL;

-- Chains ended in response to a replay. Read by `storeRefreshToken` before it mints a successor,
-- which is what makes an ending STICK: rotation is two statements with no transaction across them,
-- so an ending landing in that gap revokes nothing and would otherwise be undone by the successor
-- arriving behind it. Recording the ending rather than its effect closes that. Not written by the
-- administrative revokers, which keep their own stated excursion.
CREATE TABLE kb_oauth_refresh_chain_ends (
    chain_id UUID PRIMARY KEY,
    ended_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_oauth_refresh_chain_ends IS
  'Chains ended because a token of theirs was presented after rotation. Consulted by the AS before '
  'minting a successor, so a rotation already past its own guard cannot revive an ended chain.';

-- ONE ROW PER TOKEN, upserted: the write is reachable by anyone holding a retired token, so an
-- append-per-presentation table would grow without bound. BIGINT for the same reason.
-- `first_age_seconds` is the age the grace judgement was made on, carried from the AS —
-- `first_seen - rotated_at` would be a second clock read a statement later and could contradict it.
CREATE TABLE kb_oauth_refresh_replays (
    token_id          UUID PRIMARY KEY REFERENCES kb_oauth_refresh_tokens(id) ON DELETE CASCADE,
    chain_id          UUID NOT NULL,
    profile_id        UUID REFERENCES kb_profiles(id) ON DELETE CASCADE,
    client_id         TEXT NOT NULL,
    rotated_at        TIMESTAMPTZ NOT NULL,
    first_seen        TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen         TIMESTAMPTZ NOT NULL DEFAULT now(),
    first_age_seconds DOUBLE PRECISION NOT NULL,
    replay_count      BIGINT NOT NULL DEFAULT 1,
    graced_count      BIGINT NOT NULL DEFAULT 0,
    tokens_revoked    BIGINT NOT NULL DEFAULT 0
);

COMMENT ON TABLE kb_oauth_refresh_replays IS
  'Refresh tokens presented again after rotation. One row per token, upserted per presentation. '
  'Read through vw_oauth_refresh_replays.';

-- The operator's read, so "has any chain been replayed" has one statement of what it means and does
-- not need log retention. LEFT JOIN on the profile because profile_id is nullable by design (a
-- fail-open login records no owner) and an unattributable replay is still a replay.
CREATE VIEW vw_oauth_refresh_replays AS
SELECT r.token_id,
       r.chain_id,
       r.profile_id,
       p.handle AS profile_handle,
       r.client_id,
       r.rotated_at,
       r.first_seen,
       r.last_seen,
       make_interval(secs => r.first_age_seconds) AS first_replay_age,
       r.replay_count,
       r.graced_count,
       r.replay_count - r.graced_count AS hostile_count,
       r.tokens_revoked,
       (e.chain_id IS NOT NULL) AS chain_ended,
       e.ended_at AS chain_ended_at
  FROM kb_oauth_refresh_replays r
  LEFT JOIN kb_profiles p ON p.id = r.profile_id
  LEFT JOIN kb_oauth_refresh_chain_ends e ON e.chain_id = r.chain_id;

COMMENT ON VIEW vw_oauth_refresh_replays IS
  'Every refresh token that came back after being rotated: who held the chain, how long after the '
  'rotation it reappeared, how many presentations were judged hostile rather than a client retry, '
  'how many live tokens were revoked, and whether the chain was ended.';

SELECT declare_migration(
    20260826000140,
    'additive',
    'Makes a refresh token presented after rotation a recorded, queryable, actionable event (task 01a0390d-335a-7440-a27a-565e0a70ccce, PR #787, design internal/superpowers/specs/2026-08-26-refresh-replay-signal-design.md). Two columns on kb_oauth_refresh_tokens: rotated_at, which distinguishes rotation from the four other writers of revoked_at and is set in the rotation guard''s own statement where a rotated_to descent link could not be, since the successor row does not yet exist; and chain_id, stamped at a chain''s first token and inherited unchanged exactly as chain_expires_at is, so a response to a replay reaches the chain rather than the row presented. chain_id is NOT NULL with a DEFAULT that names rows written by a lagging binary, keeping a chain reachable across a rolling deploy, but existing rows are named by an explicit UPDATE (chain_id = id): letting the default name them would depend on uuid_generate_v7 being VOLATILE, and that function is the pg_uuidv7 extension''s on PG17 and this repo''s shim on PG18, so the deciding property is not checkable from a PG18 box. Two new tables: kb_oauth_refresh_chain_ends, consulted before minting a successor so that an ending landing between a rotation''s guard and its INSERT is not undone by the successor arriving behind it; and kb_oauth_refresh_replays, one row per token and upserted because the write is reachable by anyone holding a retired token. One partial index on live rows per chain, one view vw_oauth_refresh_replays, five COMMENT. Additive: two ALTER TABLE, one backfill UPDATE, one CREATE INDEX, two CREATE TABLE, one CREATE VIEW; no existing column, constraint, function or view is altered, and a lagging binary names none of the new names -- it omits chain_id and the default supplies one. rotated_at is asserted by the rotation that writes it and never inferred, so it stays NULL on rows already on disk. Policy (what a replay costs) lives in the AS as AS_REFRESH_REPLAY_GRACE_SECONDS, documented in docs/playbooks/self-host-with-saml.md.'
);
