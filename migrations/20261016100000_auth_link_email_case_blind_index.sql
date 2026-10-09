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

SELECT declare_migration(
    20261016100000,
    'additive',
    'One new expression index. Nothing pre-existing is altered; a binary without this migration never reads it.'
);
