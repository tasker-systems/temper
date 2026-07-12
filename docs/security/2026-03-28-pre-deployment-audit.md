# Pre-Deployment Security & Architecture Audit

> **Historical audit artifact (point-in-time, 2026-03-28).** This document captures a
> snapshot at a specific moment and does **not** reflect the current architecture. Since
> this audit: auth migrated from Neon Auth to Auth0 (production CLI OAuth device flow), and
> iteration **I5a** removed the ratatui TUI (replaced by the SvelteKit web UI). The findings
> below are preserved as-is for the record; do not treat them as a current-state description.

**Date:** 2026-03-28
**Scope:** temper-api crate + full schema, pre-I4 (Vercel deployment)
**Branch:** jcoletaylor/temper-cloud

## Summary

| Severity | Found | Fixed | Deferred |
|----------|-------|-------|----------|
| Critical | 1 | 1 (partially) | Design decision needed |
| High | 1 | 1 | — |
| Medium | 7 | 3 | 4 (schema/config changes) |
| Low | 8 | 0 | Tracked below |

## Critical Findings

### C1: Email reconciliation allows cross-provider account hijacking

**Status: Partially mitigated — design decision needed**

`profile_service::resolve_from_claims()` automatically links a new auth provider identity to an existing profile if the email matches. An attacker who controls an identity provider (or one that doesn't verify email) could present any email and hijack the matching profile.

**Mitigations applied:**
- Email claim is now required (not fabricated from `sub`) — commit `b457221`
- Currently only one provider (`neon_auth`) is configured, which verifies email

**Remaining risk:** When multi-provider support is added, email reconciliation must require `email_verified: true` from the IdP, or be replaced with an explicit user-initiated account linking flow. Track this as a requirement for the I5/I6 implementation tickets.

## High Findings

### H1: Audience validation disabled when AUTH_AUDIENCE unset

**Status: CLOSED (2026-07-12) — structurally, not by documentation.**

When `AUTH_AUDIENCE` was empty or unset, `validate_aud` was set to false, accepting any JWT from the
same issuer regardless of intended audience.

The original mitigation was to *document* the variable as required for production. That is not a
control: it relied on every future operator reading a checklist, and it left the failure silent when
they didn't. The finding stayed exploitable-in-principle for three months behind a `tracing::warn`.

**What actually closed it — two fixes, not one:**

1. **The config can no longer omit an audience.** It is a non-optional `String` on a typed
   `AuthConfig` (`crates/temper-services/src/auth_config.rs`), parsed once at the choke point both
   surfaces call. A missing audience cannot be *represented*, so `JwksKeyStore::validation` has no
   branch left that could set `validate_aud = false`, and an instance that cannot state which
   audience it validates refuses to boot.
2. **The token can no longer omit the claim.** This was the half that nearly shipped as "fixed".
   `set_audience` does **not** make `aud` mandatory: `jsonwebtoken`'s `required_spec_claims` defaults
   to `{"exp"}`, and its docs say plainly *"Validation only happens if `aud` claim is present in the
   token."* A token carrying **no** `aud` at all was still accepted, with `validate_aud = true`.
   `validation()` now sets `required_spec_claims(&["exp", "iss", "aud"])`.

An adversarial review caught (2) after (1) was written, committed, and documented as complete. The
tell was that adding an `aud` claim to every test fixture had changed no test outcome — a suite in
which every token carries the claim can never discover that the claim is optional. Both halves are
now bite-tested: `token_with_no_audience_claim_is_refused_on_both_surfaces` and
`foreign_audience_token_is_refused_on_both_surfaces`
(`tests/e2e/tests/auth_seam_parity_e2e.rs`). See
[the design spec](../superpowers/specs/2026-07-12-audience-issuer-env-coherence-design.md).

**Two lessons worth keeping.** First: this finding was correctly identified, correctly rated High,
and then answered with a doc change. A security control that depends on someone remembering to set an
environment variable is not a control — make the unsafe state unrepresentable, or it will be reached.
Second: *requiring a value to match is not the same as requiring it to exist.* "Reject a wrong `aud`"
and "require this instance's `aud`" are different properties, and only the first one is what
`set_audience` gives you.

## Medium Findings

### M1: JWT/JWKS error details leaked to clients — FIXED

**Commit:** `b457221`

Auth middleware now logs full error details server-side and returns generic messages to clients:
- JWT errors → "Invalid or expired token"
- JWKS errors → "Authentication service unavailable"

### M2: AuthUser extractor returned plain text, not JSON ErrorBody — FIXED

**Commit:** `b457221`

Changed `Rejection` type from `(StatusCode, &'static str)` to `ApiError`, ensuring consistent JSON error responses.

### M3: Email fabricated from `sub` when missing — FIXED

**Commit:** `b457221`

Email claim is now required. Tokens without `email` are rejected with 401.

### M4: Permissive CORS when no origins configured — DEFERRED

When `CORS_ORIGINS` is empty, `CorsLayer::permissive()` is used. This is safe for development but dangerous in production. Production deployment must set explicit origins.

**Action:** I4 deployment must configure `CORS_ORIGINS`. Consider failing startup if unset in a production-like environment.

### M5: `can_modify_resource()` doesn't check `is_active` — DEFERRED

The SQL function allows authorization checks to pass on soft-deleted resources. Callers mitigate this by including `AND is_active = true` in their queries, but the function itself should enforce it.

**Action:** Add `AND is_active = true` to the ownership check inside `can_modify_resource()` in a future migration.

### M6: `preferences`/`vault_config` accept unbounded JSON — DEFERRED

`ProfileUpdateRequest` accepts arbitrary `serde_json::Value` for these fields. No size or depth validation. Could allow storage exhaustion.

**Action:** Add typed structs or size limits when profile management becomes a focus (I5/I6).

### M7: `kb_contexts` lacks profile/team scoping — DEFERRED (KNOWN)

The `kb_contexts` table has no `owner_profile_id` foreign key. Contexts are global, which is a single-tenant holdover. The seeded contexts (temper, storyteller, tasker, etc.) confirm they should be profile-scoped.

**Action:** Add `owner_profile_id` to `kb_contexts` in a future migration. Update `resources_visible_to()` to scope by context ownership. Remove hardcoded seed contexts or convert to per-profile creation. This is the most significant architectural change remaining.

## Low Findings

### L1: Swagger UI publicly accessible
The OpenAPI spec and Swagger UI at `/api-docs/ui` have no auth. Consider feature-gating for production.

### L2: Hardcoded version in health endpoint
`version: "0.1.0"` will drift from Cargo.toml. Use `env!("CARGO_PKG_VERSION")` or remove.

### L3: serde_json errors reveal field names
`From<serde_json::Error>` includes field names and types. Acceptable given OpenAPI spec is public.

### L4: Provider hardcoded to "neon_auth"
When multi-provider support is added, derive from config instead of hardcoding.

### L5: JWKS key selection doesn't filter by key type
`refresh()` accepts any key type from JWKS. Add `kty=OKP`/`crv=Ed25519` filter.

### L6: JWKS cache thundering herd
Multiple concurrent requests at TTL expiry all trigger fetches. Add re-check under write lock.

### L7: Transitive dependency advisories
- `rsa` (sqlx-mysql, not used) — no fix available, not a runtime dep
- `lru` (ratatui) — resolves when TUI is removed
- `bincode` 1.x unmaintained — migrate to 2.x when convenient
- `serde_yaml` deprecated — migrate to `serde_yml`
- `paste` unmaintained — transitive, no action possible

### L8: `tokio` features = "full" in temper-api
Could narrow to specific features needed. Low impact.

## Architecture Notes

### Positive Findings
- **JWT algorithm lock:** `Validation::new(Algorithm::EdDSA)` correctly prevents algorithm confusion
- **Token expiry:** Validated by default in jsonwebtoken crate
- **Route protection:** Clean public/protected router separation — no bypass possible
- **SQL injection:** All queries use bind parameters, no string interpolation
- **Visibility scoping:** Every resource read uses `resources_visible_to()` CTE
- **Mutation authorization:** `can_modify_resource()` checked before all mutations
- **Database error sanitization:** `From<sqlx::Error>` logs details, returns generic messages
- **No unsafe in temper-api:** Single `unsafe` in temper-cli is appropriate (safetensors mmap)
- **No unused dependencies** after machete cleanup

### Schema Entities Audit

| Entity | Scoping | Status |
|--------|---------|--------|
| `kb_contexts` | Unscoped (global) | **Needs profile scoping** |
| `kb_doc_types` | System-level | Correct — intentionally global |
| `kb_behaviors` | System-level | Correct |
| `resources` | `owner_profile_id` | Correct |
| `kb_chunks` | Via resource FK | Correct |
| `kb_profiles` | Self-scoping | Correct |
| `kb_profile_auth_links` | Profile FK | Correct |
| `kb_teams` | `created_by_profile_id` | Correct |
| `kb_team_members` | Team + Profile FK | Correct |
| `kb_team_resources` | Team + Resource FK | Correct |
| `kb_team_invitations` | Team FK | Correct |
| `kb_transfers` | From/To Profile FKs | Correct |
| `kb_device_sync_state` | Profile FK | Correct |
| `kb_events` | Profile FK | Correct |
| `kb_assignable_states` | Resource FK (cascade) | Correct — `author`/`assignee` VARCHAR noted for future |
