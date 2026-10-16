-- Pending invitations to the gating team that a non-admin issued are withdrawn (team-role rank
-- spec R11/C14, C16).
--
-- From this release only a system admin invites onto the gating team: `create_invitation` resolves
-- the caller's `MembershipAuthority`, and on the gating team a non-admin's row resolves to no
-- managing authority at all. But an invitation is redeemed on the authority it was CREATED under —
-- `accept_invitation` re-runs no rank decision, by design (an invitation outlives a later change to
-- its inviter). So a gating-team invitation a non-admin owner or maintainer issued under the old
-- rule would still join the gating team after this deploy, for as long as it stays pending (seven
-- days at most). This withdraws exactly those.
--
-- WHAT IS WITHDRAWN: pending, unrevoked invitations on the configured gating team whose inviter is
-- not a system admin now (`is_system_admin`). Withdrawn the way a manager withdraws one —
-- `revoked_at` set, the status left `pending` — so every listing and both redemption paths treat
-- it as revoked. Nothing else moves: invitations on other teams, expired, declined or accepted
-- rows, and invitations an admin issued.
--
-- An unconfigured instance (`gating_team_slug` NULL) has no gating team, and the join matches
-- nothing.

DO $$
DECLARE
    v_withdrawn integer;
BEGIN
    UPDATE kb_team_invitations i
       SET revoked_at = now()
      FROM kb_teams t
      JOIN kb_system_settings s ON t.slug = s.gating_team_slug
     WHERE i.team_id = t.id
       AND i.status = 'pending'
       AND i.revoked_at IS NULL
       AND NOT is_system_admin(i.invited_by_profile_id);
    GET DIAGNOSTICS v_withdrawn = ROW_COUNT;
    IF v_withdrawn > 0 THEN
        RAISE WARNING 'withdrew % pending gating-team invitation(s) issued by a non-admin', v_withdrawn;
    END IF;
END $$;

SELECT declare_migration(
    20261101300000,
    'additive',
    'Withdraws (sets revoked_at on) pending gating-team invitations whose inviter is not a system '
    'admin, so an invitation issued under the old rule cannot redeem after this release. Data '
    'only: no column, constraint or function changes. A binary without this change reads a '
    'revoked invitation exactly as it reads one a manager withdrew.'
);
