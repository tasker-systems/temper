-- A refresh chain carries the RFC 8707 resource it was started for.
--
-- The authorization-code grant has minted the flow's authorized audience since PR #829: the
-- /oauth/authorize handler validates the requested `resource` fail-closed against the instance's
-- served set and stamps it on the flow row, consumeCode hands it back, and the mint puts it in the
-- token's `aud`. The refresh grant could not follow, because the chain row carried no record of the
-- audience its login was authorized for — so every rotated MCP session silently switched to the
-- instance audience and survived only through the MCP middleware's dual-accept. This column is the
-- missing carrier.
--
-- Nullable TEXT, no DEFAULT, no backfill — three deliberate choices:
--
-- 1. Nullable. NULL means "no audience recorded" — a chain minted before the paired binary landed,
--    or (today unreachable, admitted for symmetry with the authorize handler's default) a login
--    with no requested resource. The refresh grant reads NULL and mints the instance audience,
--    which is exactly today's fallback; existing chains keep refreshing across the deploy window
--    with no behaviour change.
--
-- 2. No DEFAULT. The column is not shape-breaking without one: an AS binary that predates it omits
--    the column on INSERT and Postgres fills NULL, which the old binary never reads. There is no
--    lagging-binary window to bridge the way chain_expires_at (20260825000010) needed one, because
--    the old binary neither writes nor reads this column.
--
--    The asymmetry that DOES exist runs the other way, and it is worth stating rather than leaving
--    to be discovered: on a NEW→OLD rollback the old binary's rotation INSERT omits the column, so
--    successors come back NULL and those sessions fall back to instance-audience minting behind the
--    MCP middleware's dual-accept — the reliance this change exists to remove, quietly restored for
--    rolled-back chains until a re-login starts a fresh audience-carrying chain. Degrade is
--    graceful (nothing breaks), but it is degrade: a rollback that outlives a session re-introduces
--    the gap for that session.
--
-- 3. No backfill. A chain's first token's flow row is long consumed and nothing links a chain back
--    to the flow that authorized it, so the original audience is unrecoverable for existing rows.
--    NULL is the honest record of that, and the fallback it triggers is the behaviour those rows
--    have always had. New chains carry the audience from their first token.
--
-- The write-time validation against the served audience set lives in the TypeScript writer
-- (storeRefreshToken, packages/temper-cloud/src/oauth/flow.ts), not here: the served set is
-- process env (AS_AUDIENCE / MCP_AUDIENCE), which a migration cannot read — the same reason
-- 20260825000010 duplicated its 90-day bound as a literal. The DB stores what it is told; the
-- fail-closed refusal of an out-of-set audience is the paired binary's job.
ALTER TABLE kb_oauth_refresh_tokens
    ADD COLUMN audience TEXT;

COMMENT ON COLUMN kb_oauth_refresh_tokens.audience IS
  'The RFC 8707 resource indicator this chain''s login was authorized for — what the client '
  'requested at /oauth/authorize and the instance validated against its served set. Stamped at '
  'the chain''s first token and inherited UNCHANGED by every successor, so a refreshed session '
  'mints the audience it originally asked for rather than silently switching to the instance '
  'audience. NULL on chains minted before the paired binary (or with no requested resource): '
  'rotation falls back to the instance audience, which is the pre-existing behaviour. '
  'Fail-closed out-of-set refusal lives in the TypeScript writer, which is the only place the '
  'served set (AS_AUDIENCE / MCP_AUDIENCE) is readable.';

SELECT declare_migration(
    20260831000010,
    'additive',
    'A refresh chain carries the RFC 8707 resource it was started for (task '
    '01a05852-e2e6-7a70-88d0-f68cb9630d68). One nullable TEXT column audience on '
    'kb_oauth_refresh_tokens: stamped at the chain''s first token from the flow the authorization-code '
    'grant redeemed, inherited unchanged by every successor, and read by the refresh grant so a '
    'rotated session mints the audience the flow was authorized for instead of silently switching to '
    'the instance audience behind the MCP middleware''s dual-accept. Nullable with no DEFAULT and no '
    'backfill: NULL (chains minted before this lands, or logins with no requested resource) keeps '
    'today''s fallback — refresh mints the instance audience — so there is no lagging-binary window '
    'to bridge and no behaviour change at deploy. Out-of-set audiences are refused at write time by '
    'the paired binary against the served set (AS_AUDIENCE / MCP_AUDIENCE), which a migration cannot '
    'read. No existing column, constraint or function is altered.'
);
