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

-- Shape-breaking, NOT additive — and the reason is the wire contract, not the schema.
-- Adding a value to invitation_status changes what every query decoding that column can
-- return. The running binary decodes by the frozen type {pending,accepted,declined,expired};
-- a rolling old binary that reads a row written as 'revoked' fails to decode it. That is a
-- caller this change does not update (DEPLOYING.md § "A function's return type is a wire
-- contract with the binary"), so it must ride the operator-run cutover, not auto-deploy.
-- sqlx-schema-crosscheck flags exactly this: the wire cache moved for the invitation_status
-- queries, so the migration carrying the move must own the shape-breaking classification.
SELECT declare_migration(
    20260808000010,
    'shape-breaking',
    'Adds enum value invitation_status.revoked — owner-side withdrawal of a pending team invite, distinct from declined. Shape-breaking: the enum is a wire contract, so every query decoding invitation_status now returns a value the pre-deploy binary cannot decode; requires the cutover runbook (deploy the value-aware binary before any row is written revoked), not auto-deploy.'
);
