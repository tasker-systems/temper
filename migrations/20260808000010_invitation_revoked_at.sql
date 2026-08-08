-- Add invitation revocation — the owner-side withdrawal of a pending invite — modelled as a
-- nullable timestamp rather than an invitation_status enum value.
--
-- WHY NOT AN ENUM VALUE. Adding 'revoked' to invitation_status moves the wire contract for every
-- query decoding that enum: the running binary decodes by the frozen type, so a rolling old binary
-- that read a 'revoked' row could not decode it (sqlx-schema-crosscheck flags exactly this, and it
-- would force the change onto the operator cutover instead of auto-deploy). A NEW nullable column is
-- additive — the pre-deploy binary neither writes nor selects it, and its rows carry NULL — so this
-- rides auto-deploy on main.
--
-- Revocation is orthogonal to status: a revoked invite keeps status = 'pending' and carries
-- revoked_at IS NOT NULL. The service treats revoked_at as authoritative — it is excluded from the
-- pending uniqueness index (so re-invite is never blocked by a withdrawn invite), from both
-- listings, and from accept/decline.
ALTER TABLE kb_team_invitations ADD COLUMN revoked_at TIMESTAMPTZ;

-- Re-scope the one-pending-per-(team,email) guard so a revoked invite no longer occupies it.
-- For the pre-deploy binary this is a no-op: it never sets revoked_at, so all its rows have
-- revoked_at IS NULL and the index behaves exactly as before.
DROP INDEX idx_invitations_one_pending;
CREATE UNIQUE INDEX idx_invitations_one_pending
    ON kb_team_invitations (team_id, invited_email)
    WHERE status = 'pending' AND revoked_at IS NULL;

SELECT declare_migration(
    20260808000010,
    'additive',
    'Adds nullable column kb_team_invitations.revoked_at and re-scopes idx_invitations_one_pending to (status=pending AND revoked_at IS NULL) — owner-side invite revocation, additively (no enum change, no wire-contract move; the pre-deploy binary neither writes nor reads the column and its rows are NULL).'
);
