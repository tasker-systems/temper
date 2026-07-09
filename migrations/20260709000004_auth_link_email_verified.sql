-- Migration: persist email verification on auth links (invitation-resolution Option A).
--
-- Task 019f423b (deferred from the invitee-side resolution round, PR #311 / Option B).
-- Option B guards email-matching at query time by discounting any email held by more
-- than one profile — safe, but it leaves the fragmented-unverified-account case on the
-- manual token path, and reconciliation trusts a stored email whose original claims
-- were never verified. Persisting the provider's `email_verified` claim closes both:
-- matching and reconciliation can require a VERIFIED stored email.
--
-- Backfill decision: existing rows stay `false` (strict). Provisioning now refreshes
-- the flag (and the stored email) on every verified sign-in of an existing link, so
-- rows self-heal at each holder's next login rather than being trusted retroactively —
-- a liberal backfill would mark emails verified that never carried the claim.
ALTER TABLE kb_profile_auth_links
    ADD COLUMN email_verified BOOLEAN NOT NULL DEFAULT false;
