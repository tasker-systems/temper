# I3a: Audit Remediation — Design Spec

## Overview

Address all 6 deferred findings from the 2026-03-28 pre-deployment security and architecture audit. Schema-first sequencing: migration, then service layer, then middleware/config, then route gating.

## 1. kb_contexts Polymorphic Ownership

### Problem

`kb_contexts` has no ownership model — contexts are global, a single-tenant holdover. The seeded contexts (temper, storyteller, tasker, knowledge, writing) are user-specific project names that should be profile-scoped or team-scoped.

### Design

Add polymorphic ownership via `(kb_owner_table, kb_owner_id)` tuple. Migration sequence handles existing rows:

```sql
-- Step 1: Add columns as nullable
ALTER TABLE kb_contexts
  ADD COLUMN kb_owner_table VARCHAR(64),
  ADD COLUMN kb_owner_id UUID;

-- Step 2: Backfill existing seed contexts to system profile
UPDATE kb_contexts
  SET kb_owner_table = 'kb_profiles',
      kb_owner_id = '00000000-0000-0000-0001-000000000001';

-- Step 3: Set NOT NULL after backfill
ALTER TABLE kb_contexts
  ALTER COLUMN kb_owner_table SET NOT NULL,
  ALTER COLUMN kb_owner_table SET DEFAULT 'kb_profiles',
  ALTER COLUMN kb_owner_id SET NOT NULL;

-- Step 4: Replace global unique with per-owner unique
ALTER TABLE kb_contexts DROP CONSTRAINT kb_contexts_name_key;
ALTER TABLE kb_contexts
  ADD CONSTRAINT kb_contexts_owner_name_unique
  UNIQUE (kb_owner_table, kb_owner_id, name);

-- Step 5: Constrain owner table values
ALTER TABLE kb_contexts
  ADD CONSTRAINT kb_contexts_owner_table_check
  CHECK (kb_owner_table IN ('kb_profiles', 'kb_teams'));
```

Existing seed contexts assigned to the system profile (`00000000-0000-0000-0001-000000000001`) with `kb_owner_table = 'kb_profiles'`.

New SQL function `contexts_visible_to(p_profile_id UUID, p_team_id UUID DEFAULT NULL)` returns contexts where:
- `kb_owner_table = 'kb_profiles' AND kb_owner_id = p_profile_id`, OR
- `kb_owner_table = 'kb_teams' AND kb_owner_id` is a team the profile belongs to

Service code uses this to scope context listing. Resource queries that filter by `kb_context_id` inherit scoping since the context itself is access-controlled.

### CLI Implication

Context name resolves to the authenticated profile's context by default. `--team <name>` resolves against team-owned contexts. This is a future CLI concern (I5/I6) but the schema supports it now.

## 2. Email Reconciliation Hardening

### Problem

`resolve_from_claims()` auto-links a new provider identity to an existing profile if emails match. An attacker controlling a provider that doesn't verify email could hijack profiles.

### Design

Add `email_verified: Option<bool>` to `AuthClaims` in `temper-core/src/types/auth.rs`. Extract from JWT claims in auth middleware. Default to `None` (unknown) when absent.

Gate reconciliation in `resolve_from_claims()`:

1. **Direct link lookup** (unchanged) — `(provider, external_user_id)` already linked, return that profile.
2. **Email reconciliation** (gated) — only auto-link via email match if `email_verified == Some(true)`. If `None` or `Some(false)`, skip to step 3.
3. **Create new profile** (unchanged) — fresh profile + link.

When reconciliation is skipped due to unverified email, log at `warn` with provider name and external_user_id (no PII/email in logs).

Future `temper auth link` command (I5/I6) provides explicit account linking for users whose email couldn't be auto-reconciled.

## 3. Preferences & VaultConfig Validation

### Problem

`ProfileUpdateRequest` accepts arbitrary `serde_json::Value` for `preferences` and `vault_config`. No size, depth, or structure validation.

### Design

**preferences** — stays as `Option<serde_json::Value>`. Size-capped at 64KB in the handler. No structural validation — the UI/UX that populates this hasn't been designed.

**vault_config** — typed struct in `temper-core`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultConfig {
    pub vault_path: Option<String>,
    pub subscriptions: Vec<Subscription>,
    pub per_device: HashMap<String, DeviceOverrides>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub context: String,
    pub team: Option<String>,
    pub doc_types: Option<Vec<String>>,
    pub auto_sync: bool,
    pub merge_policy: MergePolicy,
    pub local_paths: Vec<String>,
    pub repos: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceOverrides {
    pub vault_path: Option<String>,
    pub subscription_overrides: HashMap<String, SubscriptionOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionOverride {
    pub auto_sync: Option<bool>,
    pub merge_policy: Option<MergePolicy>,
    pub local_paths: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum MergePolicy {
    #[default]
    Manual,
    Auto,
}
```

Key design choices:
- **No IndexConfig** — offline indexing is dropped; vault is a managed path.
- **Sync config lives inside subscriptions** — no top-level SyncConfig. Each subscription is self-contained with its own `auto_sync` and `merge_policy`.
- **`local_paths` and `repos` on Subscription** enable CWD-to-context inference. When temper runs in a directory that falls within a subscription's `local_paths`, it infers the context automatically.
- **DeviceOverrides keyed by X-Temper-Client-Id** — per-device vault location and subscription behavior overrides.

`ProfileUpdateRequest` changes `vault_config: Option<Value>` to `vault_config: Option<VaultConfig>`. Serde validates structure at deserialization — invalid payloads get 422. Overall request body size capped at 64KB.

DB column stays JSONB. Existing empty `{}` values deserialize to `VaultConfig::default()`.

## 4. Swagger UI Access Control

### Problem

OpenAPI spec and Swagger UI publicly accessible. Useful for development, exposes full API surface in production. API Dog available for public-facing API docs.

### Design

`ENABLE_SWAGGER` env var in `ApiConfig`, parsed as bool, default `false`.

In `routes.rs`, conditionally merge `SwaggerUi` route only when `config.enable_swagger` is true. When disabled, paths return 404 (don't exist). No auth complexity.

When enabled, log at `info`: "Swagger UI enabled at /api-docs/ui" — visible in startup output.

## 5. JWKS Key Type Filtering

### Problem

`JwksKeyStore::refresh()` accepts any key type from JWKS endpoint. Should filter by `kty=OKP` and `crv=Ed25519` to match the EdDSA algorithm restriction.

### Design

In `refresh()`, filter `jwks.keys` to only those with `kty: "OKP"` and matching Ed25519 curve before calling `DecodingKey::from_jwk()`. Use the `jsonwebtoken::jwk::Jwk` struct's `algorithm` and `key_type` fields.

If no matching keys found after filtering, return a clear error: "no Ed25519 keys found in JWKS response". This improves diagnostics during key rotation vs the current generic parse failure.

## 6. Provider Name from Config

### Problem

`auth.rs:91` hardcodes `provider: "neon_auth"`. Must come from configuration for multi-provider support.

### Design

Add `auth_provider_name: String` to `ApiConfig`, sourced from `AUTH_PROVIDER_NAME` env var, defaulting to `"neon_auth"`. Replace hardcoded string in auth middleware with `state.config.auth_provider_name`.

Unblocks multi-provider in I5/I6 without designing the full system now.

## Sequencing

1. **Migration** — kb_contexts ownership columns, unique constraint change, `contexts_visible_to()` function, seed data update
2. **Service layer** — context queries enforce ownership scoping; `resolve_from_claims()` email_verified gate; `VaultConfig` typed struct; preferences size cap
3. **Middleware/config** — `email_verified` in AuthClaims; `auth_provider_name` in ApiConfig; JWKS key type filtering
4. **Route gating** — Swagger UI behind `ENABLE_SWAGGER`

## Dependencies

- I3 complete (done)
- Produces schema and types consumed by I4 (Vercel deployment), I5 (temper-client), I6 (sync protocol)

## Test Strategy

- Migration: validate `contexts_visible_to()` returns correct scoping via SQL tests
- Email reconciliation: unit test `resolve_from_claims()` with verified/unverified/missing email_verified
- VaultConfig: serde round-trip tests (serialize/deserialize), rejection of invalid payloads
- Swagger UI: integration test confirming 404 when disabled, 200 when enabled
- JWKS filtering: unit test with mixed key types in JWKS response
- Provider name: verify config value propagates to AuthClaims
