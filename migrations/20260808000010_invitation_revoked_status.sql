-- Add the 'revoked' invitation status — the owner-side withdrawal of a pending invite.
--
-- Until now the invitation lifecycle had no verb for "the inviter withdrew this before the
-- invitee acted." A mis-typed email or wrong role could only be cleared by waiting out the
-- 7-day expiry (which does not even flip status without an accept attempt) or asking the
-- invitee to decline. `revoked` is that missing terminal state, distinct from `declined`
-- (invitee said no) so the two are never conflated in the record.
--
-- This is its OWN migration on purpose: PostgreSQL commits an `ALTER TYPE … ADD VALUE` before
-- the new value may be *used*, and a value added in a transaction cannot be referenced in that
-- same transaction. sqlx runs each migration in its own transaction and commits between them,
-- so any migration or runtime path that references 'revoked' runs strictly after this commits.
-- `declare_migration` below only records the class; it never references the new value.
ALTER TYPE invitation_status ADD VALUE IF NOT EXISTS 'revoked';

SELECT declare_migration(
    20260808000010,
    'additive',
    'Adds enum value invitation_status.revoked — owner-side withdrawal of a pending team invite, distinct from declined. Additive: a new enum value, no edit to the birth type, no existing caller reads it (only the new revoke path writes it).'
);
