-- Sign-in's email reconciliation matches a verified address case-blind.
--
-- profile_service::reconcile_by_email matched `email = $1`, so one person signing in through two
-- providers that disagree on case (`Alice@x.com`, `alice@x.com`) got two profiles, and an
-- invitation to that address then addressed nobody: vw_invitee_invitations compares lower(email)
-- and refuses an address two verified profiles hold. Reconciliation now compares lower(email), the
-- same rule. This index serves that comparison, as idx_auth_links_email served the exact one.
CREATE INDEX idx_auth_links_email_lower ON kb_profile_auth_links (lower(email));

SELECT declare_migration(
    20261016100000,
    'additive',
    'One new expression index. Nothing pre-existing is altered; a binary without this migration never reads it.'
);
