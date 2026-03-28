# I3a: Audit Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Address all 6 deferred findings from the 2026-03-28 pre-deployment security and architecture audit.

**Architecture:** Schema-first sequencing — migration, then temper-core types, then service/middleware/config, then route gating. Each task is a self-contained commit. The migration here is additive (not a restructure); migration consolidation happens at end of feature branch per project decision.

**Tech Stack:** Rust, sqlx (Postgres), axum, jsonwebtoken, serde, utoipa

**Note:** This migration will be consolidated into the foundational schema before first deploy. Write it as an incremental ALTER migration for now — the consolidation pass will fold it into the base CREATE.

---

### Task 1: Migration — kb_contexts polymorphic ownership

**Files:**
- Create: `migrations/20260328000002_contexts_ownership.sql`

- [ ] **Step 1: Write the migration SQL**

```sql
-- I3a: Add polymorphic ownership to kb_contexts.
-- Contexts are now scoped to a profile or team via (kb_owner_table, kb_owner_id).

-- Step 1: Add columns as nullable
ALTER TABLE kb_contexts
  ADD COLUMN kb_owner_table VARCHAR(64),
  ADD COLUMN kb_owner_id UUID;

-- Step 2: Backfill existing seed contexts to system profile
UPDATE kb_contexts
  SET kb_owner_table = 'kb_profiles',
      kb_owner_id = '00000000-0000-0000-0004-000000000001';

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

-- Step 6: Index for ownership lookups
CREATE INDEX idx_contexts_owner ON kb_contexts(kb_owner_table, kb_owner_id);

-- Step 7: Function to return contexts visible to a profile
CREATE FUNCTION contexts_visible_to(
    p_profile_id UUID,
    p_team_id UUID DEFAULT NULL
) RETURNS TABLE(id UUID, name VARCHAR(128), kb_owner_table VARCHAR(64), kb_owner_id UUID)
LANGUAGE SQL STABLE AS $$
    -- Contexts I own
    SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id
    FROM kb_contexts c
    WHERE c.kb_owner_table = 'kb_profiles'
      AND c.kb_owner_id = p_profile_id

    UNION

    -- Contexts owned by teams I belong to
    SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id
    FROM kb_contexts c
    JOIN kb_team_members tm ON tm.team_id = c.kb_owner_id
    WHERE c.kb_owner_table = 'kb_teams'
      AND tm.profile_id = p_profile_id
      AND (p_team_id IS NULL OR c.kb_owner_id = p_team_id)
$$;
```

- [ ] **Step 2: Run migration against local database**

Run: `sqlx migrate run --database-url postgresql://temper:temper@localhost:5437/temper_test`
Expected: Migration applied successfully.

- [ ] **Step 3: Verify migration applied correctly**

Run: `psql postgresql://temper:temper@localhost:5437/temper_test -c "SELECT id, name, kb_owner_table, kb_owner_id FROM kb_contexts ORDER BY name;"`
Expected: All 5 contexts show `kb_owner_table = 'kb_profiles'` and `kb_owner_id = '00000000-0000-0000-0004-000000000001'`.

- [ ] **Step 4: Verify contexts_visible_to function works**

Run: `psql postgresql://temper:temper@localhost:5437/temper_test -c "SELECT * FROM contexts_visible_to('00000000-0000-0000-0004-000000000001');"`
Expected: Returns all 5 contexts (owned by system profile).

Run: `psql postgresql://temper:temper@localhost:5437/temper_test -c "SELECT * FROM contexts_visible_to('00000000-0000-0000-0004-000000000002');"`
Expected: Returns 0 rows (anonymous profile owns no contexts).

- [ ] **Step 5: Commit**

```bash
git add migrations/20260328000002_contexts_ownership.sql
git commit -m "migration: add polymorphic ownership to kb_contexts

Adds (kb_owner_table, kb_owner_id) tuple for profile or team ownership.
Replaces global UNIQUE(name) with per-owner UNIQUE constraint.
Adds contexts_visible_to() function for scoped context listing."
```

---

### Task 2: VaultConfig typed struct in temper-core

**Files:**
- Create: `crates/temper-core/src/types/vault_config.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

- [ ] **Step 1: Write tests for VaultConfig serde round-trip**

Create `crates/temper-core/src/types/vault_config.rs`:

```rust
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::config::MergePolicy;

/// Server-side vault configuration stored in `kb_profiles.vault_config`.
///
/// Describes sync subscriptions, per-device overrides, and the vault path.
/// Stored as JSONB — existing empty `{}` values deserialize to defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultConfig {
    /// Managed vault root path
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
    /// What this profile syncs
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscriptions: Vec<Subscription>,
    /// Per-device overrides keyed by X-Temper-Client-Id
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub per_device: HashMap<String, DeviceOverrides>,
}

/// A sync subscription — scopes which resources materialize locally.
///
/// Each subscription is self-contained with its own sync and merge settings.
/// `local_paths` and `repos` enable CWD-to-context inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    /// Which kb_context this subscription targets
    pub context: String,
    /// Team-owned context (None = profile-owned)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    /// Doc type filter (None = all types)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_types: Option<Vec<String>>,
    /// Run local-only manifest pre-flight on every temper command
    #[serde(default)]
    pub auto_sync: bool,
    /// Conflict resolution policy for this subscription
    #[serde(default)]
    pub merge_policy: MergePolicy,
    /// Local directories mapped to this context
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_paths: Vec<String>,
    /// Git repos associated with this context (owner/repo or local paths)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<String>,
}

/// Per-device configuration overrides keyed by X-Temper-Client-Id.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceOverrides {
    /// Device-specific vault location
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
    /// Subscription-level overrides keyed by context name
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub subscription_overrides: HashMap<String, SubscriptionOverride>,
}

/// Overrides for a specific subscription on a specific device.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubscriptionOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_sync: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_policy: Option<MergePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_paths: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_json_deserializes_to_default() {
        let config: VaultConfig = serde_json::from_str("{}").unwrap();
        assert!(config.vault_path.is_none());
        assert!(config.subscriptions.is_empty());
        assert!(config.per_device.is_empty());
    }

    #[test]
    fn full_config_round_trips() {
        let config = VaultConfig {
            vault_path: Some("~/projects/knowledge".to_string()),
            subscriptions: vec![
                Subscription {
                    context: "temper".to_string(),
                    team: None,
                    doc_types: None,
                    auto_sync: true,
                    merge_policy: MergePolicy::Manual,
                    local_paths: vec!["~/projects/tasker-systems/temper".to_string()],
                    repos: vec!["tasker-systems/temper".to_string()],
                },
                Subscription {
                    context: "storyteller".to_string(),
                    team: Some("narrative-team".to_string()),
                    doc_types: Some(vec!["research".to_string(), "concept".to_string()]),
                    auto_sync: false,
                    merge_policy: MergePolicy::Auto,
                    local_paths: vec![],
                    repos: vec![],
                },
            ],
            per_device: HashMap::from([(
                "macbook-abc123".to_string(),
                DeviceOverrides {
                    vault_path: Some("/alt/vault".to_string()),
                    subscription_overrides: HashMap::from([(
                        "temper".to_string(),
                        SubscriptionOverride {
                            auto_sync: Some(false),
                            merge_policy: None,
                            local_paths: None,
                        },
                    )]),
                },
            )]),
        };

        let json = serde_json::to_string(&config).unwrap();
        let roundtripped: VaultConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtripped.vault_path, config.vault_path);
        assert_eq!(roundtripped.subscriptions.len(), 2);
        assert_eq!(roundtripped.subscriptions[0].context, "temper");
        assert!(roundtripped.subscriptions[0].auto_sync);
        assert_eq!(
            roundtripped.subscriptions[1].team.as_deref(),
            Some("narrative-team")
        );
        assert_eq!(roundtripped.per_device.len(), 1);
        assert!(roundtripped.per_device.contains_key("macbook-abc123"));
    }

    #[test]
    fn default_serializes_to_empty_object() {
        let config = VaultConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn subscription_skips_none_fields() {
        let sub = Subscription {
            context: "temper".to_string(),
            team: None,
            doc_types: None,
            auto_sync: false,
            merge_policy: MergePolicy::Manual,
            local_paths: vec![],
            repos: vec![],
        };
        let json = serde_json::to_string(&sub).unwrap();
        assert!(!json.contains("team"));
        assert!(!json.contains("doc_types"));
        assert!(!json.contains("local_paths"));
        assert!(!json.contains("repos"));
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p temper-core vault_config`
Expected: All 4 tests pass.

- [ ] **Step 3: Add module to types/mod.rs**

Add to `crates/temper-core/src/types/mod.rs`:
- Add `pub mod vault_config;` in the module list
- Add `pub use vault_config::{DeviceOverrides, Subscription, SubscriptionOverride, VaultConfig};` in the re-exports

- [ ] **Step 4: Run full temper-core tests**

Run: `cargo test -p temper-core`
Expected: All existing tests + new vault_config tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/vault_config.rs crates/temper-core/src/types/mod.rs
git commit -m "feat(core): add VaultConfig typed struct for profile vault configuration

Subscriptions with per-device overrides, CWD-to-context inference via
local_paths/repos, self-contained sync and merge policy per subscription."
```

---

### Task 3: Add email_verified to AuthClaims

**Files:**
- Modify: `crates/temper-core/src/types/auth.rs`

- [ ] **Step 1: Add email_verified field to AuthClaims**

In `crates/temper-core/src/types/auth.rs`, add to the `AuthClaims` struct after the `email` field:

```rust
    /// Whether the identity provider has verified the user's email.
    /// `None` means the provider didn't include the claim.
    pub email_verified: Option<bool>,
```

- [ ] **Step 2: Fix all compilation errors from the new field**

Every place that constructs `AuthClaims` must now include `email_verified`. There are two locations:

In `crates/temper-api/src/middleware/auth.rs` (the `require_auth` function), update the `AuthClaims` construction to include:

```rust
        email_verified: token_data.claims.email_verified,
```

Also add `email_verified` to the `JwtClaims` struct in the same file:

```rust
#[derive(Debug, Deserialize)]
struct JwtClaims {
    sub: String,
    email: Option<String>,
    email_verified: Option<bool>,
    exp: i64,
    iat: i64,
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p temper-core -p temper-api`
Expected: Compiles without errors.

- [ ] **Step 4: Update test JWT generation to include email_verified**

In `crates/temper-api/tests/common/mod.rs`, add `email_verified: bool` to `TestClaims`:

```rust
#[derive(Debug, Serialize, Deserialize)]
struct TestClaims {
    sub: String,
    email: String,
    email_verified: bool,
    iss: String,
    iat: i64,
    exp: i64,
}
```

Update `generate_test_jwt` to include `email_verified: true` in the claims construction.

Update `generate_expired_jwt` similarly.

- [ ] **Step 5: Run all tests**

Run: `cargo test -p temper-api`
Expected: All existing tests pass (email_verified is always true in test JWTs, matching current behavior).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/types/auth.rs crates/temper-api/src/middleware/auth.rs crates/temper-api/tests/common/mod.rs
git commit -m "feat(core): add email_verified to AuthClaims

Extracted from JWT claims as Option<bool>. None when provider doesn't
include the claim. Enables gated email reconciliation in next task."
```

---

### Task 4: Gate email reconciliation on email_verified

**Files:**
- Modify: `crates/temper-api/src/services/profile_service.rs`
- Test: `crates/temper-api/src/services/profile_service.rs` (inline test module)

- [ ] **Step 1: Write the failing test — unverified email skips reconciliation**

Add a `#[cfg(test)]` module at the bottom of `crates/temper-api/src/services/profile_service.rs`. These are unit tests that run against the test database.

Since these tests need a database, gate them with `#[cfg(feature = "test-db")]` and add the module:

```rust
#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn test_pool() -> PgPool {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://temper:temper@localhost:5437/temper_test".to_string());
        let pool = PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn verified_email_reconciles_to_existing_profile() {
        let pool = test_pool().await;

        // Create first identity
        let claims_a = AuthClaims {
            provider: "provider_a".to_string(),
            external_user_id: "user-recon-verified-a".to_string(),
            email: "recon-verified@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_a = resolve_from_claims(&pool, &claims_a).await.unwrap();

        // Second identity with same email, verified — should reconcile
        let claims_b = AuthClaims {
            provider: "provider_b".to_string(),
            external_user_id: "user-recon-verified-b".to_string(),
            email: "recon-verified@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_b = resolve_from_claims(&pool, &claims_b).await.unwrap();

        assert_eq!(profile_a.id, profile_b.id, "verified email should reconcile to same profile");

        // Cleanup
        sqlx::query("DELETE FROM kb_profile_auth_links WHERE auth_provider IN ('provider_a', 'provider_b')")
            .execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM kb_profiles WHERE id = $1")
            .bind(profile_a.id).execute(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn unverified_email_creates_separate_profile() {
        let pool = test_pool().await;

        // Create first identity with verified email
        let claims_a = AuthClaims {
            provider: "provider_a".to_string(),
            external_user_id: "user-recon-unverified-a".to_string(),
            email: "recon-unverified@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_a = resolve_from_claims(&pool, &claims_a).await.unwrap();

        // Second identity with same email but NOT verified — should NOT reconcile
        let claims_b = AuthClaims {
            provider: "provider_b".to_string(),
            external_user_id: "user-recon-unverified-b".to_string(),
            email: "recon-unverified@example.com".to_string(),
            email_verified: Some(false),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_b = resolve_from_claims(&pool, &claims_b).await.unwrap();

        assert_ne!(profile_a.id, profile_b.id, "unverified email should create separate profile");

        // Cleanup
        sqlx::query("DELETE FROM kb_profile_auth_links WHERE auth_provider IN ('provider_a', 'provider_b')")
            .execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM kb_profiles WHERE id IN ($1, $2)")
            .bind(profile_a.id).bind(profile_b.id).execute(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn missing_email_verified_creates_separate_profile() {
        let pool = test_pool().await;

        // Create first identity
        let claims_a = AuthClaims {
            provider: "provider_a".to_string(),
            external_user_id: "user-recon-none-a".to_string(),
            email: "recon-none@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_a = resolve_from_claims(&pool, &claims_a).await.unwrap();

        // Second identity with email_verified = None — should NOT reconcile
        let claims_b = AuthClaims {
            provider: "provider_b".to_string(),
            external_user_id: "user-recon-none-b".to_string(),
            email: "recon-none@example.com".to_string(),
            email_verified: None,
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_b = resolve_from_claims(&pool, &claims_b).await.unwrap();

        assert_ne!(profile_a.id, profile_b.id, "None email_verified should create separate profile");

        // Cleanup
        sqlx::query("DELETE FROM kb_profile_auth_links WHERE auth_provider IN ('provider_a', 'provider_b')")
            .execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM kb_profiles WHERE id IN ($1, $2)")
            .bind(profile_a.id).bind(profile_b.id).execute(&pool).await.unwrap();
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p temper-api --features test-db -- profile_service::tests`
Expected: `verified_email_reconciles_to_existing_profile` passes (current behavior). The other two FAIL because reconciliation currently always happens regardless of email_verified.

- [ ] **Step 3: Gate reconciliation on email_verified**

In `resolve_from_claims()`, change the email reconciliation section (step 3). Replace the unconditional email lookup with a guarded version:

Replace the block starting at "3: email reconciliation" (around line 36-68) with:

```rust
    // 3: email reconciliation — only if the new identity's email is verified
    if claims.email_verified == Some(true) {
        let reconciled_link = sqlx::query_as::<_, ProfileAuthLink>(
            r#"
            SELECT id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at
              FROM kb_profile_auth_links
             WHERE email = $1
             LIMIT 1
            "#,
        )
        .bind(&claims.email)
        .fetch_optional(pool)
        .await?;

        if let Some(existing) = reconciled_link {
            // 4: create new auth link for this provider pointing to the existing profile
            let new_link_id = Uuid::now_v7();
            sqlx::query(
                r#"
                INSERT INTO kb_profile_auth_links
                    (id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at)
                VALUES ($1, $2, $3, $4, $5, false, now())
                "#,
            )
            .bind(new_link_id)
            .bind(existing.profile_id)
            .bind(&claims.provider)
            .bind(&claims.external_user_id)
            .bind(&claims.email)
            .execute(pool)
            .await?;

            return get_by_id(pool, existing.profile_id).await;
        }
    } else {
        tracing::warn!(
            provider = %claims.provider,
            external_user_id = %claims.external_user_id,
            "Skipping email reconciliation: email_verified is not true"
        );
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p temper-api --features test-db -- profile_service::tests`
Expected: All 3 tests pass.

- [ ] **Step 5: Run full test suite**

Run: `cargo make test`
Expected: All existing tests still pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/services/profile_service.rs
git commit -m "security: gate email reconciliation on email_verified claim

Only auto-link a new provider identity to an existing profile when
email_verified == Some(true). Unverified or unknown verification status
creates a separate profile. Prevents account takeover via unverified
email from a malicious identity provider."
```

---

### Task 5: Preferences size validation in profile handler

**Files:**
- Modify: `crates/temper-api/src/handlers/profiles.rs`
- Modify: `crates/temper-api/src/services/profile_service.rs`

- [ ] **Step 1: Write failing test — oversized preferences rejected**

Add to the `#[cfg(all(test, feature = "test-db"))]` module in `profile_service.rs`:

```rust
    #[test]
    fn oversized_preferences_rejected() {
        // Generate a JSON value > 64KB
        let large_value: serde_json::Value = serde_json::Value::String("x".repeat(65_537));
        let result = validate_preferences_size(Some(&large_value));
        assert!(result.is_err());
    }

    #[test]
    fn normal_preferences_accepted() {
        let small_value: serde_json::Value = serde_json::json!({"theme": "dark"});
        let result = validate_preferences_size(Some(&small_value));
        assert!(result.is_ok());
    }

    #[test]
    fn none_preferences_accepted() {
        let result = validate_preferences_size(None);
        assert!(result.is_ok());
    }
```

- [ ] **Step 2: Add validation function to profile_service.rs**

Add at the top of `profile_service.rs`, below the imports:

```rust
/// Maximum serialized size for the preferences JSON field (64KB).
const MAX_PREFERENCES_BYTES: usize = 65_536;

/// Validate that preferences JSON does not exceed the size limit.
pub fn validate_preferences_size(preferences: Option<&Value>) -> ApiResult<()> {
    if let Some(prefs) = preferences {
        let size = serde_json::to_string(prefs)
            .map(|s| s.len())
            .unwrap_or(0);
        if size > MAX_PREFERENCES_BYTES {
            return Err(ApiError::BadRequest(format!(
                "preferences exceeds maximum size of {MAX_PREFERENCES_BYTES} bytes"
            )));
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p temper-api --features test-db -- profile_service::tests`
Expected: All tests pass (including the 3 from Task 4 and 3 new ones).

- [ ] **Step 4: Change ProfileUpdateRequest to use VaultConfig**

In `crates/temper-api/src/handlers/profiles.rs`, update the import and struct:

Replace:
```rust
use serde_json::Value;
```
with:
```rust
use serde_json::Value;
use temper_core::types::VaultConfig;
```

Update `ProfileUpdateRequest`:
```rust
#[derive(Debug, Deserialize, ToSchema)]
pub struct ProfileUpdateRequest {
    pub display_name: Option<String>,
    pub preferences: Option<Value>,
    pub vault_config: Option<VaultConfig>,
}
```

Update the `update` handler to validate preferences and serialize vault_config to Value for the service layer:

```rust
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ProfileUpdateRequest>,
) -> ApiResult<Json<Profile>> {
    profile_service::validate_preferences_size(req.preferences.as_ref())?;

    let vault_config_value = req
        .vault_config
        .as_ref()
        .map(|vc| serde_json::to_value(vc))
        .transpose()
        .map_err(|e| ApiError::BadRequest(format!("Invalid vault_config: {e}")))?;

    profile_service::update(
        &state.pool,
        auth.0.profile.id,
        req.display_name.as_deref(),
        req.preferences.as_ref(),
        vault_config_value.as_ref(),
    )
    .await
    .map(Json)
}
```

- [ ] **Step 5: Verify compilation and tests**

Run: `cargo test -p temper-api`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/handlers/profiles.rs crates/temper-api/src/services/profile_service.rs
git commit -m "security: validate preferences size and type vault_config

Preferences capped at 64KB. VaultConfig uses typed struct from
temper-core — serde validates structure at deserialization, invalid
payloads get 422."
```

---

### Task 6: Provider name from config

**Files:**
- Modify: `crates/temper-api/src/config.rs`
- Modify: `crates/temper-api/src/middleware/auth.rs`
- Modify: `crates/temper-api/tests/common/mod.rs`

- [ ] **Step 1: Add auth_provider_name to ApiConfig**

In `crates/temper-api/src/config.rs`, add the field to `ApiConfig`:

```rust
pub struct ApiConfig {
    pub database_url: String,
    pub jwks_url: String,
    pub auth_issuer: String,
    pub auth_audience: Option<String>,
    pub auth_provider_name: String,
    pub cors_origins: Vec<String>,
    pub port: u16,
}
```

In `from_env()`, add before the `Ok(Self {`:

```rust
        let auth_provider_name = env::var("AUTH_PROVIDER_NAME")
            .unwrap_or_else(|_| "neon_auth".to_string());
```

And include `auth_provider_name` in the returned struct.

- [ ] **Step 2: Replace hardcoded provider name in auth middleware**

In `crates/temper-api/src/middleware/auth.rs`, replace:

```rust
        provider: "neon_auth".to_string(),
```

with:

```rust
        provider: state.config.auth_provider_name.clone(),
```

- [ ] **Step 3: Update test config**

In `crates/temper-api/tests/common/mod.rs`, add `auth_provider_name` to the test `ApiConfig`:

```rust
    let config = ApiConfig {
        database_url: database_url.clone(),
        jwks_url: "unused".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
    };
```

- [ ] **Step 4: Verify compilation and tests**

Run: `cargo test -p temper-api`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/config.rs crates/temper-api/src/middleware/auth.rs crates/temper-api/tests/common/mod.rs
git commit -m "config: derive auth provider name from AUTH_PROVIDER_NAME env var

Defaults to 'neon_auth'. Replaces hardcoded string in auth middleware.
Unblocks multi-provider support in I5/I6."
```

---

### Task 7: JWKS key type filtering

**Files:**
- Modify: `crates/temper-api/src/state.rs`

- [ ] **Step 1: Write failing test — non-OKP keys are filtered out**

Add to the existing `#[cfg(test)] mod tests` in `state.rs`:

```rust
    #[test]
    fn is_ed25519_okp_accepts_valid_key() {
        use jsonwebtoken::jwk::{
            AlgorithmParameters, EllipticCurve, OctetKeyPairParameters, OctetKeyPairType,
        };
        let params = AlgorithmParameters::OctetKeyPair(OctetKeyPairParameters {
            key_type: OctetKeyPairType::OctetKeyPair,
            curve: EllipticCurve::Ed25519,
            x: "test".to_string(),
        });
        assert!(is_ed25519_okp(&params));
    }

    #[test]
    fn is_ed25519_okp_rejects_rsa() {
        use jsonwebtoken::jwk::{AlgorithmParameters, RSAKeyParameters, RSAKeyType};
        let params = AlgorithmParameters::RSA(RSAKeyParameters {
            key_type: RSAKeyType::RSA,
            n: "test".to_string(),
            e: "test".to_string(),
        });
        assert!(!is_ed25519_okp(&params));
    }

    #[test]
    fn is_ed25519_okp_rejects_wrong_curve() {
        use jsonwebtoken::jwk::{
            AlgorithmParameters, EllipticCurve, OctetKeyPairParameters, OctetKeyPairType,
        };
        let params = AlgorithmParameters::OctetKeyPair(OctetKeyPairParameters {
            key_type: OctetKeyPairType::OctetKeyPair,
            curve: EllipticCurve::P256,
            x: "test".to_string(),
        });
        assert!(!is_ed25519_okp(&params));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p temper-api -- is_ed25519_okp`
Expected: FAIL — `is_ed25519_okp` function does not exist.

- [ ] **Step 3: Add key type filter function and update refresh()**

Add the filter function above the `impl JwksKeyStore` block:

```rust
use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve};

/// Check if a JWK is an OKP key with Ed25519 curve (the only key type
/// we accept for EdDSA verification).
fn is_ed25519_okp(params: &AlgorithmParameters) -> bool {
    matches!(
        params,
        AlgorithmParameters::OctetKeyPair(p) if p.curve == EllipticCurve::Ed25519
    )
}
```

Update the key finding logic in `refresh()`. Replace:

```rust
        let decoding_key = jwks
            .keys
            .iter()
            .find_map(|jwk| DecodingKey::from_jwk(jwk).ok())
            .ok_or_else(|| "No usable EdDSA key found in JWKS".to_string())?;
```

with:

```rust
        let decoding_key = jwks
            .keys
            .iter()
            .filter(|jwk| is_ed25519_okp(&jwk.algorithm))
            .find_map(|jwk| DecodingKey::from_jwk(jwk).ok())
            .ok_or_else(|| "No Ed25519 (OKP) key found in JWKS response".to_string())?;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p temper-api -- is_ed25519_okp`
Expected: All 3 tests pass.

Run: `cargo test -p temper-api`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/state.rs
git commit -m "security: filter JWKS keys to Ed25519 OKP only

Rejects RSA, EC, and non-Ed25519 OKP keys before attempting to build
a DecodingKey. Improves error diagnostics during key rotation."
```

---

### Task 8: Swagger UI gating

**Files:**
- Modify: `crates/temper-api/src/config.rs`
- Modify: `crates/temper-api/src/routes.rs`
- Modify: `crates/temper-api/tests/common/mod.rs`

- [ ] **Step 1: Add enable_swagger to ApiConfig**

In `crates/temper-api/src/config.rs`, add to the struct:

```rust
    pub enable_swagger: bool,
```

In `from_env()`, add:

```rust
        let enable_swagger = env::var("ENABLE_SWAGGER")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
```

And include `enable_swagger` in the returned struct.

Add the info log inside `from_env()`, after the `enable_swagger` parse:

```rust
        if enable_swagger {
            tracing::info!("Swagger UI enabled at /api-docs/ui");
        }
```

- [ ] **Step 2: Conditionally mount Swagger UI in routes.rs**

In `crates/temper-api/src/routes.rs`, replace the unconditional merge:

```rust
    Router::new()
        .merge(public)
        .merge(protected)
        .merge(SwaggerUi::new("/api-docs/ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
```

with:

```rust
    let mut app = Router::new().merge(public).merge(protected);

    if state.config.enable_swagger {
        app = app.merge(
            SwaggerUi::new("/api-docs/ui").url("/api-docs/openapi.json", ApiDoc::openapi()),
        );
    }

    app.layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
```

- [ ] **Step 3: Update test config — swagger disabled by default**

In `crates/temper-api/tests/common/mod.rs`, add `enable_swagger: false` to the test `ApiConfig`.

- [ ] **Step 4: Write integration test — swagger 404 when disabled**

Add a test in `crates/temper-api/tests/swagger_gating.rs`:

```rust
#[cfg(feature = "test-db")]
mod common;

/// Swagger UI is disabled by default (ENABLE_SWAGGER not set).
#[cfg(feature = "test-db")]
#[tokio::test]
async fn swagger_ui_returns_404_when_disabled() {
    let app = common::setup_test_app().await;
    let resp = app.client.get(app.url("/api-docs/ui")).send().await.unwrap();
    assert_eq!(resp.status(), 404);
}

/// OpenAPI JSON is also disabled.
#[cfg(feature = "test-db")]
#[tokio::test]
async fn openapi_json_returns_404_when_disabled() {
    let app = common::setup_test_app().await;
    let resp = app
        .client
        .get(app.url("/api-docs/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}
```

- [ ] **Step 5: Run integration tests**

Run: `cargo test -p temper-api --features test-db -- swagger_gating`
Expected: Both tests pass (swagger disabled by default in test config).

- [ ] **Step 6: Update existing OpenAPI integration tests**

The existing `openapi.rs` integration tests expect Swagger UI to be accessible. They test the spec structure, not the gating. The unit test in `openapi.rs` (which tests the spec itself via `ApiDoc::openapi()`) doesn't need a running server, so it still passes.

Check if any integration tests in `crates/temper-api/tests/` hit `/api-docs/*` — if so, they need an `enable_swagger: true` variant of `setup_test_app`, or they should be converted to unit tests. The existing `openapi.rs` lib test (`openapi_spec_is_valid`) is a unit test that doesn't go through the router, so it's fine.

- [ ] **Step 7: Verify full test suite**

Run: `cargo make test`
Expected: All tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/src/config.rs crates/temper-api/src/routes.rs crates/temper-api/tests/common/mod.rs crates/temper-api/tests/swagger_gating.rs
git commit -m "security: gate Swagger UI behind ENABLE_SWAGGER env var

Default false — paths return 404 when disabled. API Dog handles
public-facing API documentation."
```

---

### Task 9: Final verification and cleanup

**Files:**
- No new files — verification only.

- [ ] **Step 1: Run cargo make check**

Run: `cargo make check`
Expected: fmt, clippy, docs, machete all pass.

- [ ] **Step 2: Run full test suite including test-db**

Run: `cargo make test-all`
Expected: All tests pass.

- [ ] **Step 3: Verify OpenAPI spec still valid**

Run: `cargo test -p temper-api -- openapi_spec_is_valid`
Expected: Pass. The spec unit test validates structure independent of routing.

- [ ] **Step 4: Review git log for commit history**

Run: `git log --oneline -10`
Expected: 8 clean commits covering migration, types, email hardening, preferences validation, provider config, JWKS filtering, swagger gating.
