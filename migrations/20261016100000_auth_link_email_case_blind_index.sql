-- Sign-in's email reconciliation matches a verified address case-blind.
--
-- profile_service::reconcile_by_email matched `email = $1`, so one person signing in through two
-- providers that disagree on case (`Alice@x.com`, `alice@x.com`) got two profiles, and an
-- invitation to that address then addressed nobody: vw_invitee_invitations compares lower(email)
-- and refuses an address two verified profiles hold. Reconciliation now compares lower(email)
-- under the C collation: ASCII letters fold, nothing else does, because this match links an
-- identity to an existing profile and the database collation's lower() also folds look-alikes
-- (U+212A KELVIN SIGN to `k`). This index serves exactly that expression, as idx_auth_links_email
-- served the exact one.
CREATE INDEX idx_auth_links_email_lower_c ON kb_profile_auth_links (lower(email COLLATE "C"));

-- vw_invitee_invitations decides who an invitation is addressed to, and so who may redeem it. It is
-- restated with the same fold: under the database collation it folded look-alikes too, so an
-- invitation to `kate@x.com` was addressed to the verified holder of the U+212A spelling when
-- nobody held `kate@x.com`. One fold, ASCII only, for every match that confers anything: linking,
-- invitation addressing, and the admin lookup by verified email. The body is otherwise its
-- definition in 20260826000010 verbatim, and its columns are unchanged.
CREATE OR REPLACE VIEW vw_invitee_invitations AS
SELECT i.id,
       i.team_id,
       t.slug AS team_slug,
       t.name AS team_name,
       i.invited_email,
       i.invited_by_profile_id,
       i.role,
       i.token,
       i.status,
       i.expires_at,
       i.created,
       inv.profile_id AS invitee_profile_id
  FROM kb_team_invitations i
  JOIN kb_teams t ON t.id = i.team_id
  JOIN (
        SELECT DISTINCT al.profile_id, lower(al.email COLLATE "C") AS email
          FROM kb_profile_auth_links al
         WHERE al.email IS NOT NULL
           AND al.email_verified
           AND (SELECT COUNT(DISTINCT al2.profile_id)
                  FROM kb_profile_auth_links al2
                 WHERE lower(al2.email COLLATE "C") = lower(al.email COLLATE "C")
                   AND al2.email_verified) = 1
       ) inv ON inv.email = lower(i.invited_email COLLATE "C")
 WHERE i.status = 'pending'
   AND i.expires_at > now()
   AND i.revoked_at IS NULL
   AND t.is_active;

SELECT declare_migration(
    20261016100000,
    'additive',
    'One new expression index, and vw_invitee_invitations restated with an ASCII-only fold, columns unchanged. A binary without this migration never reads the index and selects the same view columns.'
);
