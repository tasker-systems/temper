-- SAML Phase 2: profile soft-delete/deactivation as the authn lever.
-- Additive-only. Sibling to (not part of) the SAML reconcile flow — this is the
-- general-purpose account-deactivation gate that `require_auth` enforces on
-- every authenticated request, regardless of auth provider (OAuth or SAML).
-- See docs/superpowers/specs/2026-07-01-saml-phase2-role-team-provisioning-design.md.

ALTER TABLE kb_profiles ADD COLUMN is_active BOOLEAN NOT NULL DEFAULT true;
