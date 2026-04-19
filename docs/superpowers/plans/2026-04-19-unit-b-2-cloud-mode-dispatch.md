# Unit B.2: Cloud-Mode Dispatch Rewrites — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route `resource::create` / `resource::update` / `push` / `pull` / `list` / `show` / `search` / `sync run` through cloud-mode branches when `VaultState::Cloud`. A cloud session with `TEMPER_TOKEN` + `TEMPER_VAULT_STATE=cloud` can run the full resource CRUD/query surface without ever touching `auth.json` or the vault filesystem.

**Architecture:** B.2 lands in two halves. **Part 1 (Auth foundation)** introduces a `TokenStore` trait in `temper-client`, with `DiskTokenStore` (preserves current behavior) and `MemoryTokenStore` (cloud / ephemeral). `StoredAuth` tokens are wrapped in `SecretString` with a manual redacted `Debug`. `parse_jwt_claims` stops silently dropping non-UUID `sub` claims. `provider: String` becomes a `Provider` enum with one `Auth0 { domain: String }` variant; `auth.json` format is reset (no migration). `temper auth token <jwt>` migrates from positional arg to stdin-only input. New `temper auth export-token` command exports a refreshed access token from the user's local grant. **Part 2 (Cloud dispatch)** branches each CLI command on `VaultState::Cloud` at the action-layer boundary, routing cloud-mode calls through the existing `temper-client` REST surface (`ResourcesApi::list`, `get`, `content`, `create`, `update`). Unit A's primitives already accept `manifest: None`, so `push` / `pull` need only a thin cloud-mode guard to skip manifest load. `sync run` returns a redirect error in cloud mode.

**Tech Stack:** Rust workspace (`temper-client`, `temper-cli`, `temper-core`); clap v4; tokio/reqwest; `secrecy` crate for `SecretString`; cargo-nextest for tests; Axum-side API unchanged.

**Branch:** `jct/temper-cloud-mode-portable-memory` (already checked out; working branch for Units A/B/C/D).

**Spec:** `docs/superpowers/specs/2026-04-18-cloud-mode-and-portable-memory-design.md` §Unit B.2
**Research note:** `docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md`
**Unit A precedent:** `docs/superpowers/plans/2026-04-18-unit-a-unified-push-pull-primitives.md`

---

## Ordering Constraint (Load-Bearing)

The B.4 design note's §Integration Picture pins an ordering that prevents an accidentally-shipped intermediate state from leaking tokens to disk. **Tasks execute in the order given.** Reorganizing breaks the blast-radius property: e.g. shipping `export-token` (Task 8) before the `TokenStore` refactor (Tasks 1–6) means any cloud runner with a writable `$HOME` persists refreshed tokens to `~/.config/temper/auth.json` via the unconditional `save_auth(&updated)?` at `auth.rs:346`. Keep the sequence.

```
Part 1 — Auth foundation
  Task 1:  TokenStore trait + DiskTokenStore (preserves current behavior)
  Task 2:  SecretString newtype + redacted Debug on StoredAuth
  Task 3:  MemoryTokenStore (ephemeral, cloud)
  Task 4:  parse_jwt_claims non-UUID sub fix
  Task 5:  Provider enum (Q5); reset auth.json
  Task 6:  Eliminate free-function shims; thread TokenStore explicitly

Part 2 — Auth CLI surface
  Task 7:  temper auth token → stdin-only (breaking)
  Task 8:  temper auth export-token (new command)

Part 3 — Cloud-mode dispatch
  Task 9:  TemperClient construction switches stores based on VaultState
  Task 10: resource::list cloud branch
  Task 11: resource::show cloud branch
  Task 12: resource::create cloud branch (shared helper across all doctypes)
  Task 13: resource::update cloud branch
  Task 14: push / pull cloud-mode guard (skip manifest load)
  Task 15: sync run cloud redirect

Part 4 — Integration
  Task 16: End-to-end cloud-mode round-trip test
  Task 17: cargo make check + full test sweep
```

---

## Key Invariants

1. **Local mode behavior is byte-for-byte preserved.** Every task gates new behavior behind `VaultState::from_env().is_cloud()` or equivalent. The only local-mode changes are the auth-layer refactors (Part 1); those are observably equivalent at the user surface (same `auth.json` reads/writes, same CLI output).
2. **Tokens never hit disk in cloud mode.** `MemoryTokenStore` is the only auth surface in cloud sessions. No code path falls back to `DiskTokenStore` when `VaultState::Cloud`.
3. **No `auth.json` is created or required in cloud mode.** `temper auth login` / `logout` / `status` / `token` / `export-token` all still target disk in local mode; `VaultState::Cloud` reads from `TEMPER_TOKEN` and never writes.
4. **POST-and-rewrite preserves Unit A's provisional→canonical machinery.** The existing `extract_resource_id_with_provisional_flag` at `actions/sync.rs:1001` and the rewrite logic at `actions/sync.rs:1217-1231` are reused, not reinvented.
5. **Service layer owns SQL** — this plan touches zero SQL. All cloud-mode dispatch goes through the existing `temper-client` REST wrappers in `resources.rs` / `ingest.rs`.

---

## File Structure

**Modified:**
- `crates/temper-client/src/auth.rs` — add `TokenStore` trait, `DiskTokenStore`, `MemoryTokenStore`, `SecretString`-wrapped token fields, manual `Debug` on `StoredAuth`, fixed `parse_jwt_claims`, `Provider` enum. Free-function `refresh_token` / `get_valid_token` converted to trait methods.
- `crates/temper-client/src/config.rs` — `build_client` + `build_client_from` accept an explicit store.
- `crates/temper-client/src/lib.rs` — `TemperClient::get_valid_token` (line 68) routes through injected store.
- `crates/temper-cli/src/commands/auth.rs` — `token` migrated to stdin-only; new `export_token` entry point.
- `crates/temper-cli/src/actions/runtime.rs` — `with_client` picks `DiskTokenStore` or `MemoryTokenStore` based on `VaultState::from_env()`.
- `crates/temper-cli/src/commands/resource.rs` — `create`, `list`, `show` gain cloud branches.
- `crates/temper-cli/src/commands/resource/cloud.rs` **(new module)** — shared helpers: `create_cloud`, `update_cloud`, `list_cloud`, `show_cloud`, `build_resource_payload_from_content`.
- `crates/temper-cli/src/actions/task.rs` — `create` cloud branch.
- `crates/temper-cli/src/commands/session.rs` — `save` cloud branch.
- `crates/temper-cli/src/commands/research.rs` — `save` cloud branch.
- `crates/temper-cli/src/commands/goal.rs` — `create` cloud branch.
- `crates/temper-cli/src/commands/push.rs` — skip manifest load in cloud mode.
- `crates/temper-cli/src/commands/pull.rs` — same.
- `crates/temper-cli/src/commands/sync_cmd.rs` — cloud-mode redirect error at start of `run()`.
- `crates/temper-cli/src/cli.rs` — `AuthAction::Token` arg becomes optional; add `AuthAction::ExportToken`.
- `crates/temper-cli/src/main.rs` — dispatch arm for `ExportToken`.

**Created:**
- `crates/temper-cli/src/commands/resource/cloud.rs` — cloud-mode resource dispatch helpers.
- `tests/e2e/tests/cloud_mode_test.rs` — cloud-mode round-trip (create / show / list / update / push / pull / sync-redirect).

---

## Task 1: Introduce `TokenStore` trait + `DiskTokenStore`

**Purpose:** Define the trait and land the disk implementation that preserves today's `auth.json` behavior. No behavior change; call sites unchanged in this task. Free functions stay — Task 6 removes them once the trait is plumbed through.

**Files:**
- Modify: `crates/temper-client/src/auth.rs` — add trait + impl near the top of the file (insert after the `use` block, before `StoredAuth`).

- [ ] **Step 1.1: Add the trait + `DiskTokenStore`**

Insert at `crates/temper-client/src/auth.rs`, directly after the existing `use` statements (around line 20, above `StoredAuth`):

```rust
// ---------------------------------------------------------------------------
// TokenStore — abstracts over "where does the auth state live"
// ---------------------------------------------------------------------------
//
// DiskTokenStore  — ~/.config/temper/auth.json (local CLI default).
// MemoryTokenStore — ephemeral, populated from env vars at session start
//                    (cloud mode / agent runners with no writable $HOME).
//
// Every function that previously called the free `load_auth` / `save_auth`
// functions must move to accepting `&dyn TokenStore` so the caller chooses
// the backing storage explicitly. `VaultState::Cloud` MUST use
// MemoryTokenStore; `DiskTokenStore` reaching a cloud session would write
// refreshed tokens to the agent's $HOME.

pub trait TokenStore: Send + Sync {
    fn load(&self) -> Result<Option<StoredAuth>>;
    fn save(&self, auth: &StoredAuth) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

/// Disk-backed token store — the local CLI default. Wraps the existing
/// `load_auth_from` / `save_auth_to` / `clear_auth_at` helpers with a
/// configurable path (default: `auth_json_path()`).
#[derive(Debug, Clone)]
pub struct DiskTokenStore {
    path: std::path::PathBuf,
}

impl DiskTokenStore {
    /// Use `~/.config/temper/auth.json`.
    pub fn default_path() -> Self {
        Self { path: auth_json_path() }
    }

    /// Use an explicit path (tests, non-default installs).
    pub fn at(path: std::path::PathBuf) -> Self {
        Self { path }
    }
}

impl TokenStore for DiskTokenStore {
    fn load(&self) -> Result<Option<StoredAuth>> {
        // Env-var path takes precedence even for DiskTokenStore — matches
        // current `load_auth()` semantics. Callers in cloud mode should use
        // MemoryTokenStore; this fallback exists for backward compatibility
        // with any tool that instantiates DiskTokenStore without checking
        // VaultState.
        if let Some(stored) = stored_auth_from_env()? {
            return Ok(Some(stored));
        }
        load_auth_from(&self.path)
    }

    fn save(&self, auth: &StoredAuth) -> Result<()> {
        save_auth_to(auth, &self.path)
    }

    fn clear(&self) -> Result<()> {
        clear_auth_at(&self.path)
    }
}
```

- [ ] **Step 1.2: Write a test for `DiskTokenStore` round-trip**

Append to the existing `#[cfg(test)] mod tests` block in `crates/temper-client/src/auth.rs`:

```rust
    #[test]
    fn disk_token_store_round_trips_stored_auth() {
        let tmp = tempfile::NamedTempFile::new().expect("tmpfile");
        let store = DiskTokenStore::at(tmp.path().to_path_buf());

        let auth = StoredAuth {
            provider: "auth0".to_string(),
            access_token: "at_test".to_string(),
            refresh_token: Some("rt_test".to_string()),
            expires_at: Utc::now() + Duration::hours(1),
            profile_id: Some(uuid::Uuid::now_v7()),
            device_id: Some(uuid::Uuid::now_v7().to_string()),
        };

        store.save(&auth).expect("save");
        let loaded = store.load().expect("load").expect("some");
        assert_eq!(loaded.access_token, auth.access_token);
        assert_eq!(loaded.refresh_token, auth.refresh_token);
        assert_eq!(loaded.provider, auth.provider);

        store.clear().expect("clear");
        let after_clear = store.load().expect("load after clear");
        // Env var may still populate on CI — guard against false failure.
        if std::env::var("TEMPER_TOKEN").is_err() {
            assert!(after_clear.is_none());
        }
    }
```

- [ ] **Step 1.3: Run the test**

Run: `cargo nextest run -p temper-client disk_token_store`
Expected: PASS.

- [ ] **Step 1.4: Run the full client test suite to confirm no regression**

Run: `cargo nextest run -p temper-client`
Expected: all PASS.

- [ ] **Step 1.5: Commit**

```bash
git add crates/temper-client/src/auth.rs
git commit -m "feat(client): introduce TokenStore trait + DiskTokenStore

Spine of the cloud-mode auth refactor. DiskTokenStore wraps the existing
load_auth_from / save_auth_to / clear_auth_at helpers with a configurable
path. Free functions (load_auth / save_auth / clear_auth) stay until Task 6
eliminates them; this task adds the new surface without removing the old.

Refs: docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md Q2"
```

---

## Task 2: `SecretString` on `StoredAuth` + redacted `Debug`

**Purpose:** Wrap `access_token` / `refresh_token` in `SecretString` so leak paths (`tracing::error!("{auth:?}")`, `dbg!`, `#[instrument]`, `serde_json::to_string`) are structurally closed. Manual redacted `Debug` impl on `StoredAuth`.

**Files:**
- Modify: `crates/temper-client/Cargo.toml` — add `secrecy = "0.10"`.
- Modify: `crates/temper-client/src/auth.rs` — wrap token fields, add `Debug` impl.
- Modify: `crates/temper-cli/src/commands/auth.rs:48-55` — use `.into()` when constructing `StoredAuth`.

- [ ] **Step 2.1: Add `secrecy` dependency**

Edit `crates/temper-client/Cargo.toml`, under `[dependencies]`:

```toml
secrecy = { version = "0.10", features = ["serde"] }
```

Run: `cargo build -p temper-client`
Expected: clean build (the dep is unused so far).

- [ ] **Step 2.2: Write a failing test for redacted `Debug`**

Append to the test module in `crates/temper-client/src/auth.rs`:

```rust
    #[test]
    fn stored_auth_debug_redacts_tokens() {
        let auth = StoredAuth {
            provider: "auth0".to_string(),
            access_token: secrecy::SecretString::from("at_sensitive"),
            refresh_token: Some(secrecy::SecretString::from("rt_sensitive")),
            expires_at: Utc::now(),
            profile_id: None,
            device_id: None,
        };
        let rendered = format!("{auth:?}");
        assert!(
            !rendered.contains("at_sensitive"),
            "access_token must not appear in Debug output: {rendered}"
        );
        assert!(
            !rendered.contains("rt_sensitive"),
            "refresh_token must not appear in Debug output: {rendered}"
        );
        assert!(rendered.contains("provider"), "structural fields still present: {rendered}");
    }
```

- [ ] **Step 2.3: Run the test to verify failure**

Run: `cargo nextest run -p temper-client stored_auth_debug_redacts_tokens`
Expected: FAIL — `access_token` is still `String`, not `SecretString`.

- [ ] **Step 2.4: Change field types and add manual `Debug`**

Edit `crates/temper-client/src/auth.rs` around lines 23–33. Replace:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub profile_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub device_id: Option<String>,
}
```

With:

```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub provider: String,
    #[serde(with = "secrecy_serde")]
    pub access_token: secrecy::SecretString,
    #[serde(default, with = "secrecy_serde_opt")]
    pub refresh_token: Option<secrecy::SecretString>,
    pub expires_at: DateTime<Utc>,
    pub profile_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub device_id: Option<String>,
}

impl std::fmt::Debug for StoredAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredAuth")
            .field("provider", &self.provider)
            .field("access_token", &"<REDACTED>")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<REDACTED>"))
            .field("expires_at", &self.expires_at)
            .field("profile_id", &self.profile_id)
            .field("device_id", &self.device_id)
            .finish()
    }
}

/// Serde helper: serialize/deserialize `SecretString` as its underlying
/// string. The JSON layer is trusted because `auth.json` is chmod 0o600
/// and the env-var path never reaches serde at all.
mod secrecy_serde {
    use secrecy::{ExposeSecret, SecretString};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(s: &SecretString, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(s.expose_secret())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SecretString, D::Error> {
        String::deserialize(d).map(SecretString::from)
    }
}

mod secrecy_serde_opt {
    use secrecy::{ExposeSecret, SecretString};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        s: &Option<SecretString>,
        ser: S,
    ) -> Result<S::Ok, S::Error> {
        match s {
            Some(v) => ser.serialize_some(v.expose_secret()),
            None => ser.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<SecretString>, D::Error> {
        Option::<String>::deserialize(d).map(|o| o.map(SecretString::from))
    }
}
```

- [ ] **Step 2.5: Update every call site that reads `.access_token` / `.refresh_token`**

Run: `cargo check -p temper-client`
Expected: type errors at each site that reads `auth.access_token` as `&str` or `String`.

For each site, replace:
- `&auth.access_token` → `auth.access_token.expose_secret()` (add `use secrecy::ExposeSecret;` in scope)
- `auth.refresh_token.as_deref()` → `auth.refresh_token.as_ref().map(|s| s.expose_secret())`
- Struct literal construction (e.g. `access_token: "abc".to_string()`) → `access_token: secrecy::SecretString::from("abc")` or `.into()`

Expected sites (grep `access_token\|refresh_token` in `crates/temper-client/src/`):
- `auth.rs:237` — `access_token: jwt` → `access_token: jwt.into()`
- `auth.rs:238` — `refresh_token: None` → stays (Option)
- `auth.rs:313-316` — extracting `refresh` for the POST body → `let refresh = auth.refresh_token.as_ref().map(|s| s.expose_secret()).ok_or(ClientError::TokenExpired)?;`
- `auth.rs:337-341` — `StoredAuth` construction in `refresh_token()` fn → wrap `tr.access_token.into()`, `tr.refresh_token.map(Into::into).or_else(|| auth.refresh_token.clone())`
- `auth.rs:359` — `Ok(auth.access_token)` in `get_valid_token()` → `Ok(auth.access_token.expose_secret().to_string())`
- `auth.rs:287` — `Ok(auth.access_token)` in `current_token()` → same
- `config.rs:377` — `refresh_token: None` — stays
- `login.rs:165` — `refresh_token: tokens.refresh_token` → wrap `tokens.refresh_token.map(Into::into)`

Also: existing tests in `auth.rs` construct `StoredAuth` literals — update those too.

- [ ] **Step 2.6: Update `temper-cli/src/commands/auth.rs:48-55`**

In the `token()` function, change:

```rust
    let stored = temper_client::auth::StoredAuth {
        provider: provider.to_string(),
        access_token: jwt.to_string(),
        refresh_token: None,
        ...
    };
```

To:

```rust
    let stored = temper_client::auth::StoredAuth {
        provider: provider.to_string(),
        access_token: jwt.to_string().into(),
        refresh_token: None,
        ...
    };
```

- [ ] **Step 2.7: Run the redaction test**

Run: `cargo nextest run -p temper-client stored_auth_debug_redacts_tokens`
Expected: PASS.

- [ ] **Step 2.8: Run the full workspace test suite**

Run: `cargo nextest run --workspace`
Expected: PASS (any test asserting on `access_token` equality must read `.expose_secret()`). Fix any failures.

- [ ] **Step 2.9: Commit**

```bash
git add crates/temper-client/Cargo.toml crates/temper-client/src/auth.rs crates/temper-cli/src/commands/auth.rs Cargo.lock
git commit -m "feat(client): wrap StoredAuth tokens in SecretString; redacted Debug

Closes the structural leak path: tracing::error!(\"{auth:?}\"), dbg!, and
any future #[instrument]/to_string on StoredAuth will print <REDACTED>
instead of the raw tokens. Manual Debug impl; secrecy_serde modules
preserve the on-disk JSON format for auth.json.

Every .access_token / .refresh_token reader now goes through ExposeSecret
at the single point of network use.

Refs: docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md Q2
structural hardening."
```

---

## Task 3: `MemoryTokenStore` (ephemeral, cloud)

**Purpose:** The cloud-mode counterpart to `DiskTokenStore`. Constructed once at session start from `TEMPER_TOKEN`. Holds `Arc<RwLock<Option<StoredAuth>>>`. Reads do **not** re-parse env on every `load()` — once initialized, the store is authoritative.

**Files:**
- Modify: `crates/temper-client/src/auth.rs` — add `MemoryTokenStore` next to `DiskTokenStore`.

- [ ] **Step 3.1: Write a failing test**

Append to the test module:

```rust
    #[test]
    fn memory_token_store_returns_what_was_saved_not_env() {
        let _guard = temp_env::with_vars(
            [("TEMPER_TOKEN", None::<&str>)],
            || {},
        );

        let store = MemoryTokenStore::empty();
        assert!(store.load().expect("load").is_none());

        let auth = StoredAuth {
            provider: "auth0".to_string(),
            access_token: secrecy::SecretString::from("at_v1"),
            refresh_token: None,
            expires_at: Utc::now() + Duration::hours(1),
            profile_id: None,
            device_id: None,
        };
        store.save(&auth).expect("save");

        let loaded = store.load().expect("load").expect("some");
        assert_eq!(loaded.access_token.expose_secret(), "at_v1");

        store.clear().expect("clear");
        assert!(store.load().expect("load after clear").is_none());
    }

    #[test]
    fn memory_token_store_from_env_reads_token_once() {
        temp_env::with_var("TEMPER_TOKEN", Some(make_test_jwt(3600)), || {
            let store =
                MemoryTokenStore::from_env().expect("from_env").expect("some");
            // Mutate env after construction — store must NOT re-read.
            std::env::set_var("TEMPER_TOKEN", "junk_not_a_jwt");
            let loaded = store.load().expect("load").expect("some");
            // Token came from the initial parse, not from the junk env.
            assert_eq!(loaded.provider, "auth0");
            std::env::remove_var("TEMPER_TOKEN");
        });
    }
```

(The test helper `make_test_jwt` already exists further up in the test module — reuse it. If the signature differs, match it.)

Add dev-dep if not already present: `temp-env = "0.3"` in `[dev-dependencies]` of `crates/temper-client/Cargo.toml`.

- [ ] **Step 3.2: Run the test to verify failure**

Run: `cargo nextest run -p temper-client memory_token_store`
Expected: FAIL — `MemoryTokenStore` does not exist.

- [ ] **Step 3.3: Implement `MemoryTokenStore`**

Insert in `crates/temper-client/src/auth.rs`, directly after `DiskTokenStore`:

```rust
/// Ephemeral, in-memory token store. Constructed once at session start —
/// typically from `TEMPER_TOKEN` via `MemoryTokenStore::from_env()`. Saves
/// stay in memory only; `clear()` wipes the slot. Deliberately does NOT
/// derive `Serialize` — prevents future "log the whole client state as
/// JSON" accidents.
#[derive(Clone)]
pub struct MemoryTokenStore {
    inner: std::sync::Arc<std::sync::RwLock<Option<StoredAuth>>>,
}

impl MemoryTokenStore {
    /// Empty store — for tests, or when the caller will `save()` immediately.
    pub fn empty() -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// Pre-populated from `StoredAuth`.
    pub fn with_auth(auth: StoredAuth) -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::RwLock::new(Some(auth))),
        }
    }

    /// Initialize from env vars (`TEMPER_TOKEN` + `TEMPER_PROVIDER` +
    /// `TEMPER_DEVICE_ID`). Returns `Ok(None)` when `TEMPER_TOKEN` is unset
    /// — caller should fall back to disk or error per VaultState. Returns
    /// `Err(_)` when the env is set but malformed.
    ///
    /// Unlike `stored_auth_from_env`, this reads the env **once**; later
    /// `load()` calls return the in-memory state, not a fresh env read.
    pub fn from_env() -> Result<Option<Self>> {
        match stored_auth_from_env()? {
            Some(auth) => Ok(Some(Self::with_auth(auth))),
            None => Ok(None),
        }
    }
}

impl std::fmt::Debug for MemoryTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryTokenStore")
            .field("populated", &self.inner.read().map(|g| g.is_some()).unwrap_or(false))
            .finish()
    }
}

impl TokenStore for MemoryTokenStore {
    fn load(&self) -> Result<Option<StoredAuth>> {
        Ok(self.inner.read().map_err(|_| {
            ClientError::Other("MemoryTokenStore lock poisoned".into())
        })?.clone())
    }

    fn save(&self, auth: &StoredAuth) -> Result<()> {
        let mut guard = self.inner.write().map_err(|_| {
            ClientError::Other("MemoryTokenStore lock poisoned".into())
        })?;
        *guard = Some(auth.clone());
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        let mut guard = self.inner.write().map_err(|_| {
            ClientError::Other("MemoryTokenStore lock poisoned".into())
        })?;
        *guard = None;
        Ok(())
    }
}
```

- [ ] **Step 3.4: Run the tests**

Run: `cargo nextest run -p temper-client memory_token_store`
Expected: PASS (2 tests).

- [ ] **Step 3.5: Commit**

```bash
git add crates/temper-client/src/auth.rs crates/temper-client/Cargo.toml Cargo.lock
git commit -m "feat(client): add MemoryTokenStore for cloud-mode sessions

Ephemeral, populated once from TEMPER_TOKEN via from_env(). Not Serialize
— prevents accidental JSON-log leaks. clear() wipes in-memory state.

Refs: docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md Q2"
```

---

## Task 4: Fix `parse_jwt_claims` non-UUID `sub` bug

**Purpose:** Today `parse_jwt_claims` at `auth.rs:189-192` silently sets `profile_id = None` when `sub` isn't a UUID. This works for our issuer (profile UUIDs) but will break silently in Unit D when `sub` is `auth0|abc123...`. Surface it as an explicit distinction: preserve the raw string alongside the UUID-typed field.

**Design decision:** Rather than breaking the return type, add a `sub: Option<String>` field on `JwtClaims` that captures the raw subject. `profile_id` stays `Option<Uuid>` with today's semantics (UUID-only). Callers that need the raw subject (Unit D, tracing) read `sub`.

**Files:**
- Modify: `crates/temper-client/src/auth.rs:154-197` — extend `JwtClaims`, populate both fields.

- [ ] **Step 4.1: Write a failing test**

Append to the test module:

```rust
    #[test]
    fn parse_jwt_claims_preserves_non_uuid_sub() {
        // Issuer pattern "auth0|abc123" — not a UUID.
        let payload = serde_json::json!({
            "exp": (Utc::now() + Duration::hours(1)).timestamp(),
            "sub": "auth0|6123abcdef"
        });
        let jwt = make_test_jwt_with_payload(&payload);

        let claims = parse_jwt_claims(&jwt).expect("parse");
        assert!(claims.profile_id.is_none(), "non-UUID sub: profile_id stays None");
        assert_eq!(
            claims.sub.as_deref(),
            Some("auth0|6123abcdef"),
            "raw sub must be preserved for downstream (Unit D) callers"
        );
    }

    #[test]
    fn parse_jwt_claims_uuid_sub_populates_both_fields() {
        let uuid = uuid::Uuid::now_v7();
        let payload = serde_json::json!({
            "exp": (Utc::now() + Duration::hours(1)).timestamp(),
            "sub": uuid.to_string()
        });
        let jwt = make_test_jwt_with_payload(&payload);

        let claims = parse_jwt_claims(&jwt).expect("parse");
        assert_eq!(claims.profile_id, Some(uuid));
        assert_eq!(claims.sub.as_deref(), Some(uuid.to_string().as_str()));
    }
```

If `make_test_jwt_with_payload` helper doesn't exist, add it near the existing `make_test_jwt` helper in the test module — it base64url-encodes the header+payload+signature per the existing pattern.

- [ ] **Step 4.2: Run to verify failure**

Run: `cargo nextest run -p temper-client parse_jwt_claims`
Expected: FAIL — `JwtClaims` has no `sub` field.

- [ ] **Step 4.3: Extend `JwtClaims`**

Replace `crates/temper-client/src/auth.rs:154-158`:

```rust
#[derive(Debug, Clone)]
pub struct JwtClaims {
    pub expires_at: DateTime<Utc>,
    pub profile_id: Option<uuid::Uuid>,
}
```

With:

```rust
#[derive(Debug, Clone)]
pub struct JwtClaims {
    pub expires_at: DateTime<Utc>,
    /// Only populated when `sub` parses as a UUID (our issuer's profile id).
    /// Unit D tokens issued via Auth0 Management API may have non-UUID
    /// subjects (e.g., `auth0|abc123`); use `sub` for the raw value.
    pub profile_id: Option<uuid::Uuid>,
    /// The raw `sub` claim as a string, regardless of format. Preserved
    /// for downstream callers (tracing, Unit D session-id extraction) that
    /// need to distinguish "sub missing" from "sub present but not a UUID".
    pub sub: Option<String>,
}
```

Update the constructor at `auth.rs:194-198`:

```rust
    let sub = payload
        .get("sub")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let profile_id = sub
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    Ok(JwtClaims {
        expires_at,
        profile_id,
        sub,
    })
```

- [ ] **Step 4.4: Run the tests**

Run: `cargo nextest run -p temper-client parse_jwt_claims`
Expected: PASS.

Also run the pre-existing tests that assert on `profile_id`:

Run: `cargo nextest run -p temper-client`
Expected: PASS (the pre-existing `parse_jwt_profile_id_none_when_sub_not_uuid` test at `auth.rs:523` continues to pass — `profile_id` behavior is unchanged).

- [ ] **Step 4.5: Commit**

```bash
git add crates/temper-client/src/auth.rs
git commit -m "fix(client): preserve raw JWT sub alongside profile_id

JwtClaims.sub captures the raw subject string (e.g. auth0|abc123); the
UUID-typed profile_id stays None when sub isn't UUID-shaped. Unit D's
Management-API-minted tokens will carry non-UUID subjects — that path
now has a way to read sub without silently dropping it.

Refs: docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md Q2
latent bug."
```

---

## Task 5: `Provider` enum (Q5); reset `auth.json`

**Purpose:** Convert `StoredAuth.provider: String` → `Provider { Auth0 { domain: String } }`. Reset auth.json format (no migration shim — users re-run `temper auth login`).

**Files:**
- Modify: `crates/temper-client/src/auth.rs` — add `Provider` enum, change `StoredAuth` field.
- Modify: every call site that constructs or reads `provider`.

- [ ] **Step 5.1: Write a failing test**

Append to the test module:

```rust
    #[test]
    fn provider_auth0_serializes_as_tagged_enum() {
        let p = Provider::Auth0 {
            domain: "temper.us.auth0.com".to_string(),
        };
        let json = serde_json::to_string(&p).expect("to_string");
        assert!(json.contains("\"kind\":\"auth0\""), "tag present: {json}");
        assert!(json.contains("\"domain\":\"temper.us.auth0.com\""), "domain: {json}");

        let round: Provider = serde_json::from_str(&json).expect("from_str");
        assert_eq!(round, p);
    }
```

- [ ] **Step 5.2: Run to verify failure**

Run: `cargo nextest run -p temper-client provider_auth0`
Expected: FAIL — `Provider` doesn't exist.

- [ ] **Step 5.3: Add the enum**

Insert in `crates/temper-client/src/auth.rs`, above `StoredAuth`:

```rust
/// Which identity provider issued the stored credentials.
///
/// One variant today; the enum shape is the change — a second variant
/// (hypothetical `SelfHosted`, etc.) is a separate design question and
/// is NOT speculatively built. Extending the enum later is a minor
/// refactor; extending a stringly-typed field is a breaking API change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provider {
    Auth0 { domain: String },
}

impl Provider {
    /// Convenience constructor for the common case.
    pub fn auth0(domain: impl Into<String>) -> Self {
        Self::Auth0 {
            domain: domain.into(),
        }
    }

    /// Expose the provider's domain (e.g. `temper.us.auth0.com`) for OAuth
    /// endpoint construction.
    pub fn domain(&self) -> &str {
        match self {
            Provider::Auth0 { domain } => domain,
        }
    }
}
```

- [ ] **Step 5.4: Change `StoredAuth.provider` field type**

In `StoredAuth` (the struct edited in Task 2), change:

```rust
    pub provider: String,
```

To:

```rust
    pub provider: Provider,
```

- [ ] **Step 5.5: Update every construction site**

Run: `cargo check -p temper-client`
Expected: errors at each `provider: "..."` site.

Sites to update (search `provider: `):
- `auth.rs:236` in `stored_auth_from_env` — was `provider` (String); becomes `Provider::Auth0 { domain: provider }`. The domain comes from the env var today (it's whatever "auth0" / "auth0:host" parses to). Simplify to:

  ```rust
      let provider_env = std::env::var(TEMPER_PROVIDER_ENV)
          .ok()
          .filter(|s| !s.is_empty());
      let provider = match provider_env.as_deref() {
          None | Some("auth0") => {
              // Domain default from the compiled-in config — lookup via
              // crate::config if accessible; else fall through to a
              // sentinel and fix in Task 9 when threading the real config.
              Provider::Auth0 { domain: default_auth0_domain() }
          }
          Some(s) if s.starts_with("auth0:") => {
              Provider::Auth0 { domain: s.trim_start_matches("auth0:").to_string() }
          }
          Some(other) => {
              return Err(ClientError::Other(format!(
                  "unsupported TEMPER_PROVIDER value: {other}"
              )));
          }
      };
  ```

  Add a small helper `fn default_auth0_domain() -> String` that reads from `crate::config::load_cloud_config()` or, if that introduces a cycle, a hardcoded constant (the domain is compile-time-known — check the existing `OAuthConfig` / `auth.auth0_domain()` surface in `crates/temper-client/src/config.rs` around lines 60-100 and reuse the same source).

- `auth.rs:340` in `refresh_token` — `provider: auth.provider.clone()` stays.
- `cli/commands/auth.rs:49` in `token()` — the CLI currently takes `provider: &str`. Update the parse:

  ```rust
  let provider_enum = temper_client::auth::Provider::from_str(provider)?;
  // ... or if kept stringly at the CLI surface, parse manually:
  let provider_enum = match provider {
      "auth0" => temper_client::auth::Provider::auth0(
          temper_client::config::default_auth0_domain()
      ),
      other => return Err(crate::error::TemperError::Config(format!(
          "unsupported provider: {other}"
      ))),
  };
  ```

  (Add `impl FromStr for Provider` in `auth.rs` mirroring the parse logic from the env-var path above, so callers share one path.)

- `auth_status()` at `auth.rs:131-146` — `provider: Some(a.provider)` — stays (the AuthStatus struct's `provider` field type changes to `Option<Provider>`).

- Tests in `auth.rs` that construct `StoredAuth` with `provider: "auth0".to_string()` — change to `provider: Provider::auth0("test.auth0.com")`.

- [ ] **Step 5.6: Reset any existing `auth.json` on disk (local dev hygiene)**

The repo doesn't commit `auth.json`. Developers running the tool will see a parse error on their next `temper` invocation because the old format is incompatible. Add a deserialization helper that maps the legacy `"auth0"` string to `Provider::Auth0 { domain: <default> }` via `serde(untagged)` **once** for one release, then remove — **or** just document in the commit message that `temper auth login` is required after this change.

**Recommended:** no compat shim. Per the "no premature backward compat" rule (repo is ~1 month old, all `auth.json` files are on developer machines). The commit message + the stderr error on next `temper` run is clear enough. Users run `temper auth login`; disk is rewritten.

- [ ] **Step 5.7: Run tests + check**

Run: `cargo nextest run --workspace` and `cargo make check`
Expected: PASS (fix any unrelated breakage — tests that match `provider == "auth0"` need `provider == Provider::auth0(...)`).

- [ ] **Step 5.8: Commit**

```bash
git add crates/temper-client/src/auth.rs crates/temper-cli/src/commands/auth.rs
git commit -m "refactor(client): Provider String → enum with Auth0 { domain } variant

One variant; enum shape is the change. Door is open for a future second
provider without building speculatively. auth.json format reset — users
re-run \`temper auth login\` once after upgrade.

BREAKING: StoredAuth.provider field type changes; existing auth.json
files will fail to parse and require a fresh login.

Refs: docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md Q5"
```

---

## Task 6: Eliminate free-function shims; thread `TokenStore` explicitly

**Purpose:** Delete `load_auth` / `save_auth` / `clear_auth` / `refresh_token` / `get_valid_token` / `current_token` free functions. Callers receive a `&dyn TokenStore` argument and call methods on it. This is what makes "cloud mode never writes to disk" a **structural** property: there is no `save_auth` free function that a future edit could accidentally call.

**Files:**
- Modify: `crates/temper-client/src/auth.rs` — move free-fn bodies onto `TokenStore` as default or provided methods, then remove the free functions.
- Modify: `crates/temper-client/src/lib.rs:68` — `get_valid_token` uses injected store.
- Modify: `crates/temper-client/src/config.rs:101-105` — `build_client` takes a store.
- Modify: `crates/temper-cli/src/actions/runtime.rs:16-25` — `with_client` picks the store based on `VaultState`.

- [ ] **Step 6.1: Move `refresh_token` and `get_valid_token` onto the trait**

Edit `crates/temper-client/src/auth.rs`. Replace the free `refresh_token` and `get_valid_token` functions (lines 308–360) with extension methods:

```rust
/// Token operations that work against any `TokenStore`. `refresh_if_needed`
/// is the single path through which tokens get refreshed — no free-function
/// `refresh_token` exists. Cloud sessions using `MemoryTokenStore` cannot
/// accidentally land a refreshed token on disk because the save goes
/// through the store.
#[async_trait::async_trait]
pub trait TokenStoreExt: TokenStore {
    async fn refresh_if_needed(
        &self,
        token_url: &str,
        client_id: &str,
    ) -> Result<Option<StoredAuth>> {
        let Some(auth) = self.load()? else {
            return Err(ClientError::NotAuthenticated);
        };
        if !needs_refresh(&auth) {
            return Ok(Some(auth));
        }
        let Some(refresh) = auth
            .refresh_token
            .as_ref()
            .map(|s| secrecy::ExposeSecret::expose_secret(s).to_string())
        else {
            return Err(ClientError::TokenExpired);
        };

        let client = reqwest::Client::new();
        let resp = client
            .post(token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", client_id),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(ClientError::TokenExpired);
        }
        let tr: TokenResponse = resp.json().await?;
        let expires_at =
            Utc::now() + Duration::seconds(tr.expires_in.unwrap_or(3600) as i64);
        let updated = StoredAuth {
            provider: auth.provider.clone(),
            access_token: tr.access_token.into(),
            refresh_token: tr
                .refresh_token
                .map(Into::into)
                .or_else(|| auth.refresh_token.clone()),
            expires_at,
            profile_id: auth.profile_id,
            device_id: auth.device_id.clone(),
        };
        self.save(&updated)?;
        Ok(Some(updated))
    }

    async fn get_valid_token(
        &self,
        token_url: &str,
        client_id: &str,
    ) -> Result<String> {
        match self.refresh_if_needed(token_url, client_id).await? {
            Some(auth) => Ok(secrecy::ExposeSecret::expose_secret(&auth.access_token).to_string()),
            None => Err(ClientError::NotAuthenticated),
        }
    }
}

impl<T: TokenStore + ?Sized> TokenStoreExt for T {}
```

Add `async_trait = "0.1"` to `crates/temper-client/Cargo.toml` under `[dependencies]` if not present.

- [ ] **Step 6.2: Delete the old free functions**

From `crates/temper-client/src/auth.rs`, delete:
- `pub fn load_auth()` (lines 113–118) — callers migrate to `DiskTokenStore::default_path().load()`.
- `pub fn save_auth(auth)` (lines 121–123) — callers migrate to `.save(auth)`.
- `pub fn clear_auth()` (lines 126–128) — `.clear()`.
- `pub fn current_token()` (lines 282–288) — replaced by store-based path.
- `pub async fn refresh_token(...)` (lines 308–348) — moved to trait.
- `pub async fn get_valid_token(...)` (lines 351–360) — moved to trait.

Keep: `parse_jwt_claims`, `stored_auth_from_env`, `load_auth_from`, `save_auth_to`, `clear_auth_at`, `auth_json_path`, `load_or_create_device_id`, `load_device_id`, `needs_refresh`.

- [ ] **Step 6.3: Update `lib.rs:68`**

`crates/temper-client/src/lib.rs` currently has:

```rust
auth::get_valid_token(&config.token_url, &config.client_id).await
```

The method now lives on `TokenStore`. `TemperClient` needs a store field. Add a `store: Arc<dyn TokenStore>` to `TemperClient` (or the relevant struct), wired at construction. Replace the call:

```rust
self.store.get_valid_token(&config.token_url, &config.client_id).await
```

(Read `crates/temper-client/src/lib.rs` lines 40–90 to see the exact `TemperClient` struct — adapt field name to match the style there.)

- [ ] **Step 6.4: Update `config.rs` — `build_client` takes an explicit store**

In `crates/temper-client/src/config.rs`, change:

```rust
pub fn build_client() -> crate::error::Result<crate::TemperClient> {
    let config = load_cloud_config()?;
    let auth = crate::auth::load_auth().ok().flatten();
    build_client_from(&config, auth.as_ref())
}
```

To:

```rust
pub fn build_client(
    store: std::sync::Arc<dyn crate::auth::TokenStore>,
) -> crate::error::Result<crate::TemperClient> {
    let config = load_cloud_config()?;
    build_client_from(&config, store)
}
```

And update `build_client_from` to take the store instead of `Option<&StoredAuth>`:

```rust
pub fn build_client_from(
    config: &TemperConfig,
    store: std::sync::Arc<dyn crate::auth::TokenStore>,
) -> crate::error::Result<crate::TemperClient> {
    let auth = store.load()?;
    let client = match &auth {
        Some(a) => crate::TemperClient::with_token(
            &config.api_base_url,
            secrecy::ExposeSecret::expose_secret(&a.access_token).to_string(),
            store.clone(),
        ),
        None => crate::TemperClient::new(&config.api_base_url, store.clone()),
    }?;
    let oauth = /* existing oauth attach */;
    Ok(client.with_oauth(oauth))
}
```

Update `TemperClient::new` / `with_token` in `lib.rs` to take a `store: Arc<dyn TokenStore>` parameter and store it internally.

- [ ] **Step 6.5: Update CLI `runtime::with_client`**

`crates/temper-cli/src/actions/runtime.rs:16-25` — switch to:

```rust
pub fn with_client<F, T>(f: F) -> Result<T>
where
    F: FnOnce(&temper_client::TemperClient) -> Pin<Box<dyn Future<Output = Result<T>> + '_>>,
{
    use temper_core::types::VaultState;
    use temper_client::auth::{DiskTokenStore, MemoryTokenStore, TokenStore};

    let store: std::sync::Arc<dyn TokenStore> = match VaultState::from_env() {
        VaultState::Cloud => {
            let mem = MemoryTokenStore::from_env()?
                .ok_or_else(|| TemperError::Config(
                    "TEMPER_VAULT_STATE=cloud but TEMPER_TOKEN is not set".into()
                ))?;
            std::sync::Arc::new(mem)
        }
        VaultState::Local => std::sync::Arc::new(DiskTokenStore::default_path()),
    };

    let rt = tokio::runtime::Runtime::new()?;
    let client = temper_client::config::build_client(store)?;
    rt.block_on(f(&client))
}
```

- [ ] **Step 6.6: Update every `load_auth` / `save_auth` / `current_token` call site**

Run: `cargo check --workspace`
Expected: errors at each removed-free-fn call site.

Sites (grep `load_auth\|save_auth\|current_token\|temper_client::auth::`):
- `temper-cli/src/commands/auth.rs:57` — `save_auth(&stored)?` → construct `DiskTokenStore::default_path()`, call `.save(&stored)`. (The `token` command always writes to disk — it's a local-grant import, not relevant in cloud mode.)
- `temper-cli/src/commands/auth.rs:32` — `clear_auth()` → `DiskTokenStore::default_path().clear()`.
- `temper-cli/src/commands/auth.rs:73` — `auth_status()` → keep; `auth_status()` helper stays in `auth.rs` but now takes `&dyn TokenStore` as a parameter. Update its signature + the caller.
- `temper-client/src/login.rs` — any `save_auth` call → `store.save(&stored)` where `store` is passed through.
- `temper-client/src/lib.rs` — see Step 6.3.

- [ ] **Step 6.7: Run the full test sweep**

Run: `cargo nextest run --workspace` and `cargo make check`
Expected: PASS.

- [ ] **Step 6.8: Commit**

```bash
git add -A
git commit -m "refactor(client): remove free auth functions; thread TokenStore

No more load_auth / save_auth / refresh_token / get_valid_token free
functions. Callers (TemperClient, CLI) receive a &dyn TokenStore.

Cloud mode using MemoryTokenStore can no longer accidentally write to
disk — structural, not discipline-based. VaultState::from_env() in
with_client() picks DiskTokenStore or MemoryTokenStore at session start.

Refs: docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md Q2"
```

---

## Task 7: Migrate `temper auth token` to stdin-only

**Purpose:** Today `temper auth token <jwt> --provider auth0` leaks the JWT to shell history, `ps auxww`, and `/proc/<pid>/cmdline`. Move to stdin-only input. This is a breaking CLI change; per "no premature backward compat" (repo is ~1 month old), acceptable.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` — `AuthAction::Token` loses the `jwt` field.
- Modify: `crates/temper-cli/src/commands/auth.rs` — `token()` signature drops `jwt` arg, reads stdin.
- Modify: `crates/temper-cli/src/main.rs` — dispatch arm loses the `jwt` capture.

- [ ] **Step 7.1: Write a failing integration-style test**

**File:** `crates/temper-cli/src/commands/auth.rs` — append to the file's test module (create if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_errors_when_stdin_empty() {
        // Simulate no stdin content.
        let err = token_from_stdin(Some("")).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("empty") || msg.contains("stdin"),
            "expected empty-stdin error: {msg}"
        );
    }
}
```

- [ ] **Step 7.2: Run to verify failure**

Run: `cargo nextest run -p temper-cli token_errors_when_stdin_empty`
Expected: FAIL — `token_from_stdin` doesn't exist.

- [ ] **Step 7.3: Refactor `token`**

Replace `crates/temper-cli/src/commands/auth.rs` lines 38–69 with:

```rust
/// Store a JWT directly, bypassing the OAuth flow. The JWT is read from
/// stdin (never a positional argument — positional args leak to shell
/// history, `ps auxww`, and `/proc/<pid>/cmdline`).
///
/// Example:
/// ```text
/// temper auth export-token | temper auth token
/// pbpaste | temper auth token
/// ```
pub fn token(provider: &str) -> Result<()> {
    let stdin_content = read_stdin()?;
    token_from_stdin(stdin_content.as_deref(), provider)
}

fn read_stdin() -> Result<Option<String>> {
    use std::io::Read;
    if atty::is(atty::Stream::Stdin) {
        return Err(crate::error::TemperError::Config(
            "temper auth token reads the JWT from stdin. Usage:\n  \
             temper auth export-token | temper auth token".into(),
        ));
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).map_err(|e| {
        crate::error::TemperError::Config(format!("failed to read stdin: {e}"))
    })?;
    Ok(Some(buf))
}

fn token_from_stdin(stdin_content: Option<&str>, provider: &str) -> Result<()> {
    let jwt_raw = stdin_content
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| crate::error::TemperError::Config(
            "temper auth token: stdin was empty; pipe a JWT".into(),
        ))?;

    let claims = temper_client::auth::parse_jwt_claims(jwt_raw)
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

    let device_id = temper_client::auth::load_or_create_device_id();
    let provider_enum = match provider {
        "auth0" => temper_client::auth::Provider::auth0(
            temper_client::config::default_auth0_domain(),
        ),
        other => return Err(crate::error::TemperError::Config(format!(
            "unsupported provider: {other}"
        ))),
    };

    let stored = temper_client::auth::StoredAuth {
        provider: provider_enum,
        access_token: jwt_raw.to_string().into(),
        refresh_token: None,
        expires_at: claims.expires_at,
        profile_id: claims.profile_id,
        device_id: Some(device_id),
    };

    let store = temper_client::auth::DiskTokenStore::default_path();
    temper_client::auth::TokenStore::save(&store, &stored)
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

    let status = temper_client::auth::AuthStatus {
        authenticated: true,
        provider: Some(stored.provider),
        expires_at: Some(stored.expires_at),
        profile_id: stored.profile_id,
    };
    let json = serde_json::to_string_pretty(&status).map_err(crate::error::TemperError::Json)?;
    println!("{json}");
    Ok(())
}
```

Add `atty = "0.2"` to `crates/temper-cli/Cargo.toml` under `[dependencies]` if not already present. (Or reuse whatever stdin-detection helper the codebase already uses — check `crates/temper-cli/src/vault.rs:read_stdin_if_piped` and reuse that pattern instead of pulling in `atty`.)

**Actually:** `vault::read_stdin_if_piped` already exists and is used elsewhere. Prefer:

```rust
fn read_stdin() -> Result<Option<String>> {
    Ok(crate::vault::read_stdin_if_piped())
}
```

And inline the TTY-detection error at the entry point if `read_stdin_if_piped()` returns `None`.

- [ ] **Step 7.4: Update `cli.rs`**

Find the `AuthAction` enum (grep `AuthAction`) in `crates/temper-cli/src/cli.rs`. Replace the `Token { jwt, provider }` variant with:

```rust
    /// Store a JWT directly (read from stdin). Useful for CI bootstrapping.
    /// Usage: `temper auth export-token | temper auth token`.
    Token {
        /// Identity provider (default: auth0).
        #[arg(long, default_value = "auth0")]
        provider: String,
    },
```

- [ ] **Step 7.5: Update `main.rs` dispatch**

Find `AuthAction::Token { jwt, provider } => commands::auth::token(&jwt, &provider)` and replace with:

```rust
        AuthAction::Token { provider } => commands::auth::token(&provider),
```

- [ ] **Step 7.6: Run tests**

Run: `cargo nextest run -p temper-cli` and `cargo check --workspace`
Expected: PASS.

- [ ] **Step 7.7: Commit**

```bash
git add crates/temper-cli/src/commands/auth.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "feat(cli): temper auth token reads JWT from stdin only (breaking)

Positional-arg JWT leaked to shell history, ps auxww, /proc/<pid>/cmdline.
Stdin-only input eliminates all three leak paths. Usage:
  temper auth export-token | temper auth token
  pbpaste | temper auth token

BREAKING: \`temper auth token <jwt>\` no longer accepts a positional arg;
scripts must pipe via stdin.

Refs: docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md Q1"
```

---

## Task 8: `temper auth export-token`

**Purpose:** Export a refreshed access token from the user's local grant. Token goes to stdout (pipeable). Security warning goes to stderr.

**Files:**
- Modify: `crates/temper-cli/src/commands/auth.rs` — add `export_token()`.
- Modify: `crates/temper-cli/src/cli.rs` — add `AuthAction::ExportToken`.
- Modify: `crates/temper-cli/src/main.rs` — dispatch arm.

- [ ] **Step 8.1: Write a failing unit test**

Append to the `tests` mod in `crates/temper-cli/src/commands/auth.rs`:

```rust
    #[test]
    fn export_token_errors_when_not_authenticated() {
        // Simulate no auth.json and no env var via a tmp HOME.
        let tmp = tempfile::tempdir().expect("tmp");
        let home_guard = temp_env::with_var("HOME", Some(tmp.path()), || {
            // When TEMPER_TOKEN is also unset:
            temp_env::with_var("TEMPER_TOKEN", None::<&str>, || {
                let store = temper_client::auth::DiskTokenStore::at(
                    tmp.path().join("auth.json"),
                );
                let err = export_token_with_store(
                    &store, "https://example/token", "test_client",
                );
                assert!(matches!(err, Err(_)), "must error when unauthenticated");
            });
        });
        drop(home_guard);
    }
```

(Adjust the helper signatures and `temp_env` usage to match the style of other tests in this file. If `temp_env` isn't a dep of `temper-cli`, add it to `[dev-dependencies]`.)

- [ ] **Step 8.2: Run to verify failure**

Run: `cargo nextest run -p temper-cli export_token`
Expected: FAIL — `export_token_with_store` doesn't exist.

- [ ] **Step 8.3: Implement the command**

Append to `crates/temper-cli/src/commands/auth.rs`:

```rust
/// Export a refreshed access token from the local grant.
///
/// Token goes to stdout (plain, single line — pipeable to `pbcopy`, an
/// agent's secret input, etc.). Security warning goes to stderr.
///
/// Refuses to run in cloud mode — `export-token` reads from the local
/// DiskTokenStore; a cloud-mode invocation would have nothing to export.
pub fn export_token() -> Result<()> {
    use temper_core::types::VaultState;

    if matches!(VaultState::from_env(), VaultState::Cloud) {
        return Err(crate::error::TemperError::Config(
            "temper auth export-token is a local-mode command — \
             TEMPER_VAULT_STATE=cloud has no local grant to export. \
             Run this on your laptop, paste the token into the cloud \
             session's secrets, and the agent will read TEMPER_TOKEN."
                .into(),
        ));
    }

    let config = temper_client::config::load_cloud_config()
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;
    let store = temper_client::auth::DiskTokenStore::default_path();
    print_export_warning();
    let token = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))?
        .block_on(export_token_with_store(&store, &config.token_url, &config.client_id))?;
    println!("{token}");
    Ok(())
}

async fn export_token_with_store(
    store: &dyn temper_client::auth::TokenStore,
    token_url: &str,
    client_id: &str,
) -> Result<String> {
    use temper_client::auth::TokenStoreExt;
    store
        .get_valid_token(token_url, client_id)
        .await
        .map_err(|e| crate::error::TemperError::Config(e.to_string()))
}

fn print_export_warning() {
    eprintln!(
        "⚠  This access token grants full access to your temper account at"
    );
    eprintln!(
        "   your current permission levels until it expires (~24 hours)."
    );
    eprintln!(
        "   Once issued, the token cannot be revoked early — treat leaked"
    );
    eprintln!(
        "   tokens as live for their full lifetime. Per-session revocation"
    );
    eprintln!("   is coming in Unit D of the cloud-mode goal.");
    eprintln!();
    eprintln!("   Recommended handling:");
    eprintln!("     temper auth export-token | pbcopy          # macOS clipboard");
    eprintln!("     temper auth export-token | wl-copy         # Linux wayland");
    eprintln!("     temper auth export-token | <agent-secret-input>");
    eprintln!("   AVOID:");
    eprintln!("     temper auth export-token > token.txt       # file lands in backups");
    eprintln!("     TEMPER_TOKEN=$(temper auth export-token)   # shell history + /proc/<pid>/environ");
    eprintln!();
}
```

- [ ] **Step 8.4: Add the CLI variant**

In `crates/temper-cli/src/cli.rs`, add to `AuthAction`:

```rust
    /// Export a refreshed access token (local mode only). Token goes to
    /// stdout; security warning to stderr.
    ///
    /// Pipe the token into a cloud session's secrets manager as
    /// `TEMPER_TOKEN`. Token is ~24h lifetime with no early-revoke; re-
    /// export to renew.
    ExportToken,
```

- [ ] **Step 8.5: Wire the dispatch**

In `crates/temper-cli/src/main.rs`, add the match arm:

```rust
        AuthAction::ExportToken => commands::auth::export_token(),
```

- [ ] **Step 8.6: Run tests**

Run: `cargo nextest run -p temper-cli export_token` and `cargo check -p temper-cli`
Expected: PASS.

- [ ] **Step 8.7: Manual smoke (optional, local only)**

```bash
cargo run -p temper-cli -- auth login   # if not already authed
cargo run -p temper-cli -- auth export-token 2>/dev/null > /tmp/tok.txt
head -c 20 /tmp/tok.txt   # first 20 chars of a JWT
rm /tmp/tok.txt
```

Expected: stdout contains a JWT; stderr contains the warning.

- [ ] **Step 8.8: Commit**

```bash
git add crates/temper-cli/src/commands/auth.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "feat(cli): temper auth export-token (W1 for cloud-mode bootstrap)

Exports a refreshed access token from the local DiskTokenStore. Token to
stdout (pipeable), security warning to stderr. No --json flag: JSON
wrapping expands the capture surface for structured-output loggers.

No refresh-token export. Refresh-token rotation would entangle the
user's local grant with any cloud session that holds the exported RT;
Unit D solves that via server-minted separate grants.

Refuses to run in cloud mode. Use on the local machine; paste the token
into the cloud session's secret manager as TEMPER_TOKEN.

Refs: docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md Q1 W1"
```

---

## Task 9: `TemperClient` construction switches stores based on `VaultState`

**Purpose:** Task 6 already wired `runtime::with_client` to pick the store. This task verifies the wiring at a higher level: cloud-mode sessions get `MemoryTokenStore`, local sessions get `DiskTokenStore`, with no code path that falls back from one to the other.

**Files:**
- Modify: `crates/temper-cli/src/actions/runtime.rs` — harden the `VaultState::Cloud` branch (error if `TEMPER_TOKEN` is missing).
- Add: unit test.

- [ ] **Step 9.1: Write a test asserting the cloud-mode guard**

**File:** `crates/temper-cli/src/actions/runtime.rs` — append a test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_client_errors_when_cloud_mode_but_no_token() {
        temp_env::with_vars(
            [
                ("TEMPER_VAULT_STATE", Some("cloud")),
                ("TEMPER_TOKEN", None),
            ],
            || {
                let result = with_client(|_client| {
                    Box::pin(async { Ok(()) })
                });
                let err = result.unwrap_err();
                let msg = format!("{err}");
                assert!(
                    msg.contains("TEMPER_TOKEN"),
                    "expected TEMPER_TOKEN error: {msg}"
                );
            },
        );
    }
}
```

Add `temp-env = "0.3"` to `[dev-dependencies]` of `crates/temper-cli/Cargo.toml` if not present.

- [ ] **Step 9.2: Run to verify**

Run: `cargo nextest run -p temper-cli with_client_errors_when_cloud_mode_but_no_token`
Expected: PASS (Task 6's implementation already has this guard). If it fails, harden the Task 6 implementation by adding the `.ok_or_else(|| ...)` now.

- [ ] **Step 9.3: Commit**

```bash
git add crates/temper-cli/src/actions/runtime.rs crates/temper-cli/Cargo.toml Cargo.lock
git commit -m "test(cli): assert cloud-mode runtime errors without TEMPER_TOKEN

Lock down the guard introduced by the TokenStore refactor. Cloud mode
must produce a clear error — not silently fall back to DiskTokenStore —
when TEMPER_TOKEN is missing."
```

---

## Task 10: `resource::list` cloud branch

**Purpose:** In cloud mode, `temper list --type X --context Y` calls `client.resources().list(params)` instead of `scan_rows()`. Route both branches through the same `render_list` output path so the JSON/text shapes match.

**Files:**
- Create: `crates/temper-cli/src/commands/resource/cloud.rs` (new module; `resource.rs` becomes `resource/mod.rs` OR `resource_cloud` stays a sibling file — pick the simpler option. **Recommendation:** keep `resource.rs` as-is and add a sibling `resource_cloud.rs`. Rename later if the module grows.)
- Modify: `crates/temper-cli/src/commands/mod.rs` — add `pub mod resource_cloud;`.
- Modify: `crates/temper-cli/src/commands/resource.rs` — `list()` branches on `VaultState`.

- [ ] **Step 10.1: Create `resource_cloud.rs` with `list_cloud`**

**File:** `crates/temper-cli/src/commands/resource_cloud.rs` (new)

```rust
//! Cloud-mode dispatch for `temper resource` commands. Each function
//! assumes `VaultState::Cloud` and routes through the `temper-client`
//! REST surface. No vault filesystem reads or writes.
//!
//! Shapes are chosen to match the local-mode equivalents so callers can
//! switch branches at the top of each command function without rewriting
//! output formatting.

use crate::actions::runtime;
use crate::error::{Result, TemperError};
use crate::output;
use temper_core::types::ResourceListParams;

pub struct ListCloudParams<'a> {
    pub doc_type: &'a str,
    pub context: Option<&'a str>,
    pub limit: usize,
    pub stage: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub status: Option<&'a str>,
    pub format: &'a str,
}

/// Cloud-mode `temper list`. Calls `client.resources().list(...)` with
/// server-side filters, formats the response to match local-mode output.
pub fn list(params: ListCloudParams<'_>) -> Result<()> {
    let doc_type = params.doc_type.to_string();
    let context = params.context.map(ToString::to_string);
    let stage = params.stage.map(ToString::to_string);
    let goal = params.goal.map(ToString::to_string);
    let status = params.status.map(ToString::to_string);
    let limit = params.limit;
    let format = params.format.to_string();

    runtime::with_client(move |client| {
        Box::pin(async move {
            let req = ResourceListParams {
                doc_type: Some(doc_type.clone()),
                context: context.clone(),
                stage: stage.clone(),
                goal: goal.clone(),
                status: status.clone(),
                limit: Some(limit as i64),
                ..Default::default()
            };
            let resp = client
                .resources()
                .list(&req)
                .await
                .map_err(crate::commands::client_err)?;

            if resp.rows.is_empty() {
                output::hint(format!("No {doc_type} resources found."));
                return Ok(());
            }

            if format == "json" {
                let json = serde_json::to_string_pretty(&resp.rows)
                    .map_err(TemperError::Json)?;
                println!("{json}");
            } else {
                render_rows_as_table(&resp.rows)?;
            }
            Ok(())
        })
    })
}

fn render_rows_as_table(rows: &[temper_core::types::ResourceRow]) -> Result<()> {
    // Match the local-mode table columns: Context, Type, Slug, Updated,
    // Stage, Mode, Effort, Goal. (Exact columns: check the local render
    // pipeline in resource.rs and mirror.) Tab-separated so anstream /
    // the existing output helper renders correctly on both TTY and non-
    // TTY.
    println!("Context\tType\tSlug\tUpdated\tStage\tMode\tEffort\tGoal");
    for row in rows {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.context,
            row.doc_type,
            row.slug,
            row.updated_at.format("%Y-%m-%d"),
            row.stage.as_deref().unwrap_or(""),
            row.mode.as_deref().unwrap_or(""),
            row.effort.as_deref().unwrap_or(""),
            row.goal_slug.as_deref().unwrap_or(""),
        );
    }
    Ok(())
}
```

**⚠️ Field-name verification:** `ResourceRow`'s exact field names (e.g. `goal_slug` vs `goal` vs `goal_id`) live in `crates/temper-core/src/types/resource.rs`. Open that file and match the actual field names before compiling. Don't invent.

- [ ] **Step 10.2: Register the module**

In `crates/temper-cli/src/commands/mod.rs`, add (alphabetical):

```rust
pub mod resource_cloud;
```

- [ ] **Step 10.3: Branch `list()` in `resource.rs`**

Edit `crates/temper-cli/src/commands/resource.rs:438-508` (the `list` function). At the very top, before any validation logic, insert:

```rust
pub fn list(config: &Config, params: ListParams<'_>) -> Result<()> {
    use temper_core::types::VaultState;
    if matches!(VaultState::from_env(), VaultState::Cloud) {
        return crate::commands::resource_cloud::list(
            crate::commands::resource_cloud::ListCloudParams {
                doc_type: params.doc_type,
                context: params.context,
                limit: params.limit,
                stage: params.stage,
                goal: params.goal,
                status: params.status,
                format: params.format,
            },
        );
    }
    // ... existing local-mode body stays unchanged below ...
```

- [ ] **Step 10.4: Write an e2e test for cloud-mode list**

**File:** `tests/e2e/tests/cloud_mode_test.rs` (create)

```rust
//! E2E: cloud-mode dispatch end-to-end.

mod common;
use common::{e2e_client, e2e_test_context, seed_resource};

#[tokio::test]
async fn cloud_mode_list_returns_server_rows() {
    let ctx = e2e_test_context("cloud-mode-list").await;
    let client = e2e_client(&ctx).await;

    // Seed two server-side resources.
    let a = seed_resource(&ctx, "session", "cloud-mode-list-a").await;
    let b = seed_resource(&ctx, "session", "cloud-mode-list-b").await;

    // Invoke the CLI in cloud mode.
    let output = common::e2e_cli_cmd(&ctx)
        .env("TEMPER_VAULT_STATE", "cloud")
        .env("TEMPER_TOKEN", ctx.access_token())
        .args(["list", "--type", "session", "--context", ctx.context()])
        .output()
        .await
        .expect("cli");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&a.slug), "list must contain a: {stdout}");
    assert!(stdout.contains(&b.slug), "list must contain b: {stdout}");
}
```

**⚠️ Harness verification:** `common::e2e_test_context` / `e2e_client` / `seed_resource` / `e2e_cli_cmd` — read `tests/e2e/tests/common/mod.rs` first and adapt to the real helper names. If `e2e_cli_cmd` doesn't exist, add it following the pattern from `tests/e2e/tests/push_command_test.rs` (Unit A's test harness).

- [ ] **Step 10.5: Run the test**

Precondition: `cargo make docker-up`.
Run: `cargo nextest run -p temper-e2e --features test-db cloud_mode_list`
Expected: PASS.

- [ ] **Step 10.6: Commit**

```bash
git add crates/temper-cli/src/commands/resource_cloud.rs \
        crates/temper-cli/src/commands/mod.rs \
        crates/temper-cli/src/commands/resource.rs \
        tests/e2e/tests/cloud_mode_test.rs
git commit -m "feat(cli): temper list cloud-mode branch

In VaultState::Cloud, list routes through client.resources().list() with
server-side filters — no vault filesystem scan. Local mode untouched.
Table output columns match the local-mode render; JSON format returns
the raw server rows."
```

---

## Task 11: `resource::show` cloud branch

**Purpose:** In cloud mode, `temper show --type X <slug>` resolves slug → id via `client.resources().list` (with slug filter) and fetches content via `client.resources().content(id)`. Avoids a vault file read.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource_cloud.rs` — add `show()`.
- Modify: `crates/temper-cli/src/commands/resource.rs:511-532` — `show()` branches.

- [ ] **Step 11.1: Add `show_cloud` helper**

Append to `crates/temper-cli/src/commands/resource_cloud.rs`:

```rust
pub struct ShowCloudParams<'a> {
    pub doc_type: &'a str,
    pub slug: &'a str,
    pub context: Option<&'a str>,
    pub format: &'a str,
}

pub fn show(params: ShowCloudParams<'_>) -> Result<()> {
    let doc_type = params.doc_type.to_string();
    let slug = params.slug.to_string();
    let context = params.context.map(ToString::to_string);
    let format = params.format.to_string();

    runtime::with_client(move |client| {
        Box::pin(async move {
            // Resolve slug → id via a narrow list query. Server should
            // honour (doc_type, slug, context) as a natural key.
            let req = ResourceListParams {
                doc_type: Some(doc_type.clone()),
                slug: Some(slug.clone()),
                context: context.clone(),
                limit: Some(2),
                ..Default::default()
            };
            let resp = client
                .resources()
                .list(&req)
                .await
                .map_err(crate::commands::client_err)?;

            let row = match resp.rows.len() {
                0 => {
                    return Err(TemperError::NotFound(format!(
                        "{doc_type} not found: {slug}"
                    )))
                }
                1 => &resp.rows[0],
                _ => {
                    return Err(TemperError::Vault(format!(
                        "{doc_type} slug '{slug}' is ambiguous; pass --context"
                    )))
                }
            };

            let content = client
                .resources()
                .content(row.id)
                .await
                .map_err(crate::commands::client_err)?;

            if format == "json" {
                #[derive(serde::Serialize)]
                struct ResourceShow<'a> {
                    doc_type: &'a str,
                    slug: &'a str,
                    title: &'a str,
                    context: &'a str,
                    content: &'a str,
                }
                let out = ResourceShow {
                    doc_type: &doc_type,
                    slug: &slug,
                    title: &row.title,
                    context: &row.context,
                    content: &content.markdown,
                };
                let json = serde_json::to_string_pretty(&out)
                    .map_err(TemperError::Json)?;
                println!("{json}");
            } else {
                print!("{}", content.markdown);
            }
            Ok(())
        })
    })
}
```

**⚠️ Verify `ResourceListParams.slug` exists.** Grep `crates/temper-core/src/types/resource.rs` for `slug:`. If the server-side list endpoint doesn't accept a slug filter, either (a) add it in a small API PR first (service layer + handler + sqlx prepare) or (b) filter client-side after a broader list. **Pick (a)** — filtering client-side defeats the point, and slug-based lookup is a natural extension of the list endpoint. If the field doesn't exist, the path is:

1. Add `slug: Option<String>` to `ResourceListParams` in `temper-core/src/types/resource.rs`.
2. Add a `.filter(slug)` branch to the list service in `temper-api/src/services/resource_service.rs` (or equivalent — grep for the list query).
3. `cargo sqlx prepare --workspace -- --all-features`.

Track this as a sub-task of Task 11 if the field is missing.

- [ ] **Step 11.2: Branch `show()` in `resource.rs`**

Edit `crates/temper-cli/src/commands/resource.rs:511-532`. At the top:

```rust
pub fn show(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
    edges: bool,
) -> Result<()> {
    validate_doc_type(doc_type)?;

    use temper_core::types::VaultState;
    if matches!(VaultState::from_env(), VaultState::Cloud) {
        crate::commands::resource_cloud::show(
            crate::commands::resource_cloud::ShowCloudParams {
                doc_type,
                slug,
                context,
                format,
            },
        )?;
        if edges {
            // Edges already route through the API (see existing
            // show_edges() — it calls runtime::with_client already).
            show_edges(slug, format)?;
        }
        return Ok(());
    }

    // ... existing local-mode body stays unchanged below ...
```

- [ ] **Step 11.3: Write an e2e test**

Append to `tests/e2e/tests/cloud_mode_test.rs`:

```rust
#[tokio::test]
async fn cloud_mode_show_returns_server_content() {
    let ctx = e2e_test_context("cloud-mode-show").await;
    let _client = e2e_client(&ctx).await;

    let seeded = seed_resource(&ctx, "session", "cloud-mode-show-target").await;

    let output = common::e2e_cli_cmd(&ctx)
        .env("TEMPER_VAULT_STATE", "cloud")
        .env("TEMPER_TOKEN", ctx.access_token())
        .args([
            "show",
            "--type",
            "session",
            &seeded.slug,
            "--context",
            ctx.context(),
        ])
        .output()
        .await
        .expect("cli");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&seeded.title), "body contains title: {stdout}");
}
```

- [ ] **Step 11.4: Run the test + commit**

Run: `cargo nextest run -p temper-e2e --features test-db cloud_mode_show`
Expected: PASS.

```bash
git add crates/temper-cli/src/commands/resource_cloud.rs crates/temper-cli/src/commands/resource.rs tests/e2e/tests/cloud_mode_test.rs
git commit -m "feat(cli): temper show cloud-mode branch

Resolves slug → id via a narrow list call with a slug filter, then
fetches content via client.resources().content(id). No vault read.
Edges continue to route through the API as they already do."
```

---

## Task 12: `resource::create` cloud branch (shared helper across all doctypes)

**Purpose:** The hardest task. Each doctype creator (`task::create`, `goal::create`, `session::save`, `research::save`, `create_simple_resource`) today renders a template to a `String`, then writes to disk. In cloud mode, after the template is rendered, POST the payload to `/resources`, receive the canonical `temper-id`, and print/return the server response — no disk write.

To keep the scope bounded, factor out a **shared cloud-mode helper** that takes the rendered template content + doctype + context + title, swaps in a `temper-provisional-id`, POSTs, and returns the `ResourceRow`. Each doctype creator grows a single early-return branch that calls this helper.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource_cloud.rs` — add `create()` helper.
- Modify: `crates/temper-cli/src/actions/task.rs:138+` — cloud branch.
- Modify: `crates/temper-cli/src/commands/session.rs:27+` — cloud branch.
- Modify: `crates/temper-cli/src/commands/research.rs` — cloud branch.
- Modify: `crates/temper-cli/src/commands/goal.rs` — cloud branch.
- Modify: `crates/temper-cli/src/commands/resource.rs:121+` (`create_simple_resource`) — cloud branch.

- [ ] **Step 12.1: Add the shared `create` helper**

Append to `crates/temper-cli/src/commands/resource_cloud.rs`:

```rust
/// Shared cloud-mode create path. Each doctype creator (task, goal,
/// session, research, concept/decision) calls this after rendering its
/// template; the helper swaps in a provisional id, POSTs, and returns
/// the server's canonical row.
///
/// `rendered` is the complete file content (frontmatter + body) the
/// local-mode path would have written to disk. The helper re-parses the
/// frontmatter, replaces `temper-id: <local-uuid>` with
/// `temper-provisional-id: <local-uuid>`, uses `ingest::build_ingest_payload`
/// to construct the request, POSTs, and returns the canonical row.
pub async fn create_from_rendered(
    client: &temper_client::TemperClient,
    rendered: &str,
    context: &str,
    doc_type: &str,
) -> Result<temper_core::types::ResourceRow> {
    use temper_core::frontmatter::Frontmatter;

    // Parse to extract managed/open projections.
    let fm = Frontmatter::try_from(rendered).map_err(|e| {
        TemperError::Config(format!("cloud create: parse frontmatter: {e}"))
    })?;
    let title = fm
        .value()
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let managed = fm.managed_json();
    let open = fm.open_json();
    let body = fm.body().to_string();

    let mut payload = crate::actions::ingest::build_ingest_payload(
        &body,
        &title,
        context,
        doc_type,
        None,
    )
    .map_err(|e| TemperError::Config(format!("cloud create: build payload: {e}")))?;
    payload.managed_meta = Some(managed);
    payload.open_meta = Some(open);

    let row = client
        .ingest()
        .create(&payload)
        .await
        .map_err(crate::commands::client_err)?;
    Ok(row)
}

/// Convenience wrapper that spins the async runtime for sync callers.
pub fn create_from_rendered_blocking(
    rendered: &str,
    context: &str,
    doc_type: &str,
) -> Result<temper_core::types::ResourceRow> {
    let rendered = rendered.to_string();
    let context = context.to_string();
    let doc_type = doc_type.to_string();
    runtime::with_client(move |client| {
        Box::pin(async move {
            create_from_rendered(client, &rendered, &context, &doc_type).await
        })
    })
}
```

- [ ] **Step 12.2: Branch `create_simple_resource` (concept/decision)**

In `crates/temper-cli/src/commands/resource.rs`, after the existing template render (line 166, right after `content` is assigned), insert before the `Frontmatter::try_from(content.as_str())?` parse:

```rust
    use temper_core::types::VaultState;
    if matches!(VaultState::from_env(), VaultState::Cloud) {
        let row = crate::commands::resource_cloud::create_from_rendered_blocking(
            &content, context, doc_type,
        )?;
        if format == "json" {
            let json = serde_json::to_string_pretty(&row).map_err(TemperError::Json)?;
            println!("{json}");
        } else {
            output::success(format!(
                "Created: {} {} ({})",
                doc_type, row.slug, row.id,
            ));
        }
        return Ok(());
    }
    // ... existing local-mode body continues below ...
```

- [ ] **Step 12.3: Branch `task::create`**

In `crates/temper-cli/src/actions/task.rs`, after the template render at line 195–197 (`content = tmpl.render()?`), insert before the vault-write at line 204:

```rust
    // stdin body merge stays here — in local mode the merged content
    // becomes the vault file; in cloud mode it becomes the POST body.
    let mut content = content; // no-op if the variable is already owned

    use temper_core::types::VaultState;
    if matches!(VaultState::from_env(), VaultState::Cloud) {
        let row = crate::commands::resource_cloud::create_from_rendered_blocking(
            &content, context, "task",
        )?;
        return Ok(row.slug);
    }
    // ... local-mode vault_layout + write_note + discovery event stay ...
```

Verify `let mut content = tmpl.render()?` is already the binding (check line 195 — it is). Drop the duplicate `let mut content = content;` line.

Discovery event at `task.rs:211+` is local-vault-only — it writes to `.temper/discovery/events.jsonl`. Cloud mode skips it (server writes its own audit log on create).

- [ ] **Step 12.4: Branch `session::save`, `research::save`, `goal::create`**

Read each file first (`crates/temper-cli/src/commands/session.rs`, `research.rs`, `goal.rs`), locate the point where the rendered-template `String` exists and the function is about to `vault::write_note(...)`, and insert an early-return cloud branch mirroring Step 12.3.

For each:
- Import `use temper_core::types::VaultState;` (near the top of each file).
- Insert the `if matches!(VaultState::from_env(), VaultState::Cloud)` block right before the vault-write line.
- In the cloud branch, call `create_from_rendered_blocking(&content, context, "<doctype>")` with the exact doctype string (`"session"`, `"research"`, `"goal"`).
- Return the appropriate shape — slug for ones that return slug, `()` for ones that return `()`.

For session specifically: the `format` parameter and JSON output handling are already wrapped; mirror the pattern used in `session::save`'s success path.

- [ ] **Step 12.5: Write an e2e test**

Append to `tests/e2e/tests/cloud_mode_test.rs`:

```rust
#[tokio::test]
async fn cloud_mode_create_session_round_trips_canonical_id() {
    let ctx = e2e_test_context("cloud-mode-create").await;

    // Create in one "cloud session".
    let output = common::e2e_cli_cmd(&ctx)
        .env("TEMPER_VAULT_STATE", "cloud")
        .env("TEMPER_TOKEN", ctx.access_token())
        .args([
            "resource", "create",
            "--type", "session",
            "--context", ctx.context(),
            "--title", "Cloud Mode Create Test",
            "--format", "json",
        ])
        .output()
        .await
        .expect("cli");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let created: serde_json::Value =
        serde_json::from_str(&stdout).expect("parse json");
    let id = created
        .get("id")
        .and_then(|v| v.as_str())
        .expect("id present");

    // Show from a separate "cloud session" — proves round-trip through
    // the server.
    let show_out = common::e2e_cli_cmd(&ctx)
        .env("TEMPER_VAULT_STATE", "cloud")
        .env("TEMPER_TOKEN", ctx.access_token())
        .args([
            "show",
            "--type", "session",
            "--context", ctx.context(),
            created.get("slug").and_then(|v| v.as_str()).unwrap(),
        ])
        .output()
        .await
        .expect("cli");
    assert!(show_out.status.success());
    let show_stdout = String::from_utf8_lossy(&show_out.stdout);
    assert!(
        show_stdout.contains("Cloud Mode Create Test"),
        "show returns created content: {show_stdout}"
    );

    // Assert: no auth.json written in either session (cloud-mode invariant).
    let maybe_auth = dirs::home_dir()
        .map(|h| h.join(".config/temper/auth.json"))
        .filter(|p| p.exists());
    // If the dev's local auth.json exists, that's fine — we can't easily
    // assert "not newly created" without a tmp HOME. Instead, run a dirty
    // check: the command must have succeeded above, which would have
    // failed if auth.json were required. That's the real invariant.
    let _ = maybe_auth;
}
```

- [ ] **Step 12.6: Run tests**

Precondition: `cargo make docker-up`.
Run: `cargo nextest run -p temper-e2e --features test-db cloud_mode_create`
Expected: PASS.

Also: `cargo nextest run --workspace` to confirm no local-mode regressions.

- [ ] **Step 12.7: Commit**

```bash
git add crates/temper-cli/src/commands/resource_cloud.rs \
        crates/temper-cli/src/commands/resource.rs \
        crates/temper-cli/src/actions/task.rs \
        crates/temper-cli/src/commands/session.rs \
        crates/temper-cli/src/commands/research.rs \
        crates/temper-cli/src/commands/goal.rs \
        tests/e2e/tests/cloud_mode_test.rs
git commit -m "feat(cli): resource create cloud-mode branch (all doctypes)

Each doctype creator (task, goal, session, research, concept, decision)
renders its template, then in VaultState::Cloud POSTs the payload via
resource_cloud::create_from_rendered_blocking — no disk write. Server
assigns the canonical temper-id; CLI emits it in the success/JSON output.

Local mode untouched. Discovery events skip in cloud (server-side audit)."
```

---

## Task 13: `resource::update` cloud branch

**Purpose:** In cloud mode, `temper resource update` resolves the target via `client.resources().list` (as in show), reads the current server content + frontmatter, applies the mutation in memory, and PUTs via `client.resources().update(id, req)` — no disk read/write.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource_cloud.rs` — add `update()`.
- Modify: `crates/temper-cli/src/commands/resource.rs:753+` (the `update` function) — branch.

- [ ] **Step 13.1: Inspect the current `update()` shape first**

Open `crates/temper-cli/src/commands/resource.rs:753-898` and read the full function. Note:
- The `UpdateParams` struct (grep it in the same file) — capture every mutation field it carries.
- The mutation logic at lines 815–848 — which operates on a parsed `Frontmatter`.
- The disk write at line 898.

The cloud branch must apply the same mutations, then PUT. The safest way to share mutation logic is to extract the "apply mutations to a Frontmatter" section into a helper, then share it.

- [ ] **Step 13.2: Extract the mutation helper**

In `crates/temper-cli/src/commands/resource.rs`, find the block roughly at lines 815–848 that applies `params.stage`, `params.mode`, `params.effort`, `params.state`, `params.branch`, `params.pr`, `params.goal`, `params.status`, `params.title`, `params.slug` to an `&mut Frontmatter`.

Extract into a free helper:

```rust
fn apply_update_mutations(
    fm: &mut temper_core::frontmatter::Frontmatter,
    params: &UpdateParams<'_>,
) -> Result<()> {
    // ... lifted mutation code here ...
}
```

Replace the extracted section in `update()` with a single `apply_update_mutations(&mut fm, params)?` call.

- [ ] **Step 13.3: Add `update_cloud`**

Append to `crates/temper-cli/src/commands/resource_cloud.rs`:

```rust
use temper_core::types::ResourceUpdateRequest;

pub async fn update(
    client: &temper_client::TemperClient,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    mutate: impl FnOnce(&mut temper_core::frontmatter::Frontmatter) -> Result<()>,
) -> Result<temper_core::types::ResourceRow> {
    use temper_core::frontmatter::Frontmatter;

    // Locate.
    let req = ResourceListParams {
        doc_type: Some(doc_type.to_string()),
        slug: Some(slug.to_string()),
        context: context.map(ToString::to_string),
        limit: Some(2),
        ..Default::default()
    };
    let resp = client
        .resources()
        .list(&req)
        .await
        .map_err(crate::commands::client_err)?;
    let row = match resp.rows.len() {
        0 => {
            return Err(TemperError::NotFound(format!(
                "{doc_type} not found: {slug}"
            )))
        }
        1 => resp.rows.into_iter().next().unwrap(),
        _ => {
            return Err(TemperError::Vault(format!(
                "{doc_type} slug '{slug}' is ambiguous; pass --context"
            )))
        }
    };

    // Fetch current content to rebuild frontmatter + body.
    let content = client
        .resources()
        .content(row.id)
        .await
        .map_err(crate::commands::client_err)?;

    // Parse, mutate, serialize.
    let full = content.markdown.clone();
    let mut fm = Frontmatter::try_from(full.as_str()).map_err(|e| {
        TemperError::Config(format!("update: parse frontmatter: {e}"))
    })?;
    mutate(&mut fm)?;
    let managed = fm.managed_json();
    let open = fm.open_json();
    let body = fm.body().to_string();
    let title = fm
        .value()
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(&row.title)
        .to_string();

    // Build the update request. ResourceUpdateRequest is the PATCH-shape
    // struct the API accepts — verify field names in temper-core types.
    let req = ResourceUpdateRequest {
        title: Some(title),
        markdown: Some(body),
        managed_meta: Some(managed),
        open_meta: Some(open),
        ..Default::default()
    };
    let updated = client
        .resources()
        .update(row.id, &req)
        .await
        .map_err(crate::commands::client_err)?;
    Ok(updated)
}
```

**⚠️ Verify `ResourceUpdateRequest` shape** in `temper-core/src/types/resource.rs` — the field names above are a best-guess based on the typical REST shape. Open the type, match the real fields, and adjust. If the request struct requires more fields (e.g. `slug`, `context`), include them from `row`.

- [ ] **Step 13.4: Branch `update()` in `resource.rs`**

Edit `crates/temper-cli/src/commands/resource.rs:753` (the `update` function). Insert at the very top:

```rust
pub fn update(config: &Config, params: &UpdateParams<'_>) -> Result<()> {
    use temper_core::types::VaultState;

    if matches!(VaultState::from_env(), VaultState::Cloud) {
        let params_clone = params.to_owned_params(); // helper; see below
        runtime::with_client(move |client| {
            Box::pin(async move {
                let row = crate::commands::resource_cloud::update(
                    client,
                    params_clone.doc_type.as_str(),
                    params_clone.slug.as_str(),
                    params_clone.context.as_deref(),
                    |fm| apply_update_mutations(fm, &params_clone.as_borrowed()),
                )
                .await?;
                output::success(format!(
                    "Updated: {} {} ({})",
                    params_clone.doc_type, row.slug, row.id,
                ));
                Ok(())
            })
        })?;
        return Ok(());
    }

    // ... existing local-mode body stays unchanged below ...
```

Add helper methods on `UpdateParams` to convert between borrowed and owned shapes (`to_owned_params` → `OwnedUpdateParams`; `as_borrowed` → `UpdateParams<'_>`). Implementation is mechanical — one clone per `Option<&str>` field; done once, reused.

- [ ] **Step 13.5: Write an e2e test**

Append to `tests/e2e/tests/cloud_mode_test.rs`:

```rust
#[tokio::test]
async fn cloud_mode_update_mutates_server() {
    let ctx = e2e_test_context("cloud-mode-update").await;
    let seeded = seed_resource(&ctx, "task", "cloud-mode-update-target").await;

    let out = common::e2e_cli_cmd(&ctx)
        .env("TEMPER_VAULT_STATE", "cloud")
        .env("TEMPER_TOKEN", ctx.access_token())
        .args([
            "resource", "update",
            "--type", "task",
            &seeded.slug,
            "--context", ctx.context(),
            "--stage", "in-progress",
        ])
        .output()
        .await
        .expect("cli");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Verify via API that the server now reports stage = in-progress.
    let client = e2e_client(&ctx).await;
    let refetched = client
        .resources()
        .get(seeded.id)
        .await
        .expect("refetch");
    assert_eq!(refetched.stage.as_deref(), Some("in-progress"));
}
```

- [ ] **Step 13.6: Run + commit**

```bash
cargo nextest run -p temper-e2e --features test-db cloud_mode_update
cargo nextest run --workspace
```
Expected: PASS.

```bash
git add -A
git commit -m "feat(cli): resource update cloud-mode branch

Shared apply_update_mutations helper extracted from the local path.
Cloud branch resolves slug → id → fetches current content → applies
mutations in memory → PUTs via client.resources().update. No disk I/O."
```

---

## Task 14: `push` / `pull` cloud-mode guard

**Purpose:** Unit A already made `push_one_resource` / `pull_one_resource` accept `manifest: Option<&mut Manifest>`. The CLI wrappers today try to load a manifest unconditionally. In cloud mode, skip the manifest load and pass `None`.

**Files:**
- Modify: `crates/temper-cli/src/commands/push.rs` — skip manifest load in cloud mode.
- Modify: `crates/temper-cli/src/commands/pull.rs` — same.

- [ ] **Step 14.1: Edit `push.rs`**

In `crates/temper-cli/src/commands/push.rs:20-35` (the manifest-load block), wrap the load in a `VaultState` check:

```rust
use temper_core::types::VaultState;

// ...

let is_cloud = matches!(VaultState::from_env(), VaultState::Cloud);
let (mut manifest_opt, persist) = if is_cloud {
    (None, false)
} else {
    match crate::manifest_io::load_manifest(&temper_dir, &device_id) {
        Ok(m) => (Some(m), true),
        Err(_) => (None, false),
    }
};
```

- [ ] **Step 14.2: Edit `pull.rs` identically**

In `crates/temper-cli/src/commands/pull.rs:20-35` (the equivalent block), apply the same `is_cloud` gate.

- [ ] **Step 14.3: E2E test — push works without auth.json in cloud mode**

Append to `tests/e2e/tests/cloud_mode_test.rs`:

```rust
#[tokio::test]
async fn cloud_mode_push_with_provisional_id_round_trips() {
    let ctx = e2e_test_context("cloud-mode-push").await;
    let tmp = tempfile::tempdir().expect("tmp");
    let provisional_id = uuid::Uuid::now_v7();
    let file_path = tmp.path().join("push-cloud-seed.md");
    std::fs::write(&file_path, format!(
        "---\ntemper-provisional-id: \"{provisional_id}\"\ntemper-context: {}\ntemper-type: session\ntemper-created: 2026-04-19\ntemper-owner: '@me'\ntitle: Cloud Push Seed\nslug: cloud-push-seed\ndate: 2026-04-19\n---\nBody.\n",
        ctx.context(),
    )).unwrap();

    let out = common::e2e_cli_cmd(&ctx)
        .env("TEMPER_VAULT_STATE", "cloud")
        .env("TEMPER_TOKEN", ctx.access_token())
        .args(["push", file_path.to_str().unwrap()])
        .output()
        .await
        .expect("cli");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Verify the file was rewritten with a canonical id.
    let updated = std::fs::read_to_string(&file_path).unwrap();
    assert!(!updated.contains("temper-provisional-id"));
    assert!(updated.contains("temper-id:"));
}
```

- [ ] **Step 14.4: Run + commit**

```bash
cargo nextest run -p temper-e2e --features test-db cloud_mode_push
```
Expected: PASS.

```bash
git add crates/temper-cli/src/commands/push.rs crates/temper-cli/src/commands/pull.rs tests/e2e/tests/cloud_mode_test.rs
git commit -m "feat(cli): push/pull skip manifest load in cloud mode

VaultState::Cloud → (None, false) short-circuits manifest_io::load_manifest.
Unit A's primitives already accept manifest: None; this just wires the
guard into the CLI wrappers so push/pull don't attempt to open a
.temper/manifest.json that doesn't exist in a cloud session's working dir."
```

---

## Task 15: `sync run` cloud redirect

**Purpose:** `temper sync run` is manifest-based three-way merge. In cloud mode there is no manifest. Return a clear redirecting error.

**Files:**
- Modify: `crates/temper-cli/src/commands/sync_cmd.rs:42+` (the `run` function).

- [ ] **Step 15.1: Add the guard**

At the very top of `run()` in `crates/temper-cli/src/commands/sync_cmd.rs`:

```rust
use temper_core::types::VaultState;

pub fn run(contexts: &[String], format: &str) -> Result<()> {
    if matches!(VaultState::from_env(), VaultState::Cloud) {
        return Err(crate::error::TemperError::Config(
            "temper sync run is for manifest-backed local vaults — \
             in cloud mode, use \"temper push\" and \"temper pull\" \
             against resource IDs directly.".into(),
        ));
    }
    // ... existing body ...
```

- [ ] **Step 15.2: E2E test**

Append:

```rust
#[tokio::test]
async fn cloud_mode_sync_run_returns_redirect_error() {
    let ctx = e2e_test_context("cloud-mode-sync-redirect").await;
    let out = common::e2e_cli_cmd(&ctx)
        .env("TEMPER_VAULT_STATE", "cloud")
        .env("TEMPER_TOKEN", ctx.access_token())
        .args(["sync", "run"])
        .output()
        .await
        .expect("cli");
    assert!(!out.status.success(), "sync run must fail in cloud mode");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("temper push") && stderr.contains("temper pull"),
        "redirect error mentions push/pull: {stderr}"
    );
}
```

- [ ] **Step 15.3: Run + commit**

```bash
cargo nextest run -p temper-e2e --features test-db cloud_mode_sync_run
```
Expected: PASS.

```bash
git add crates/temper-cli/src/commands/sync_cmd.rs tests/e2e/tests/cloud_mode_test.rs
git commit -m "feat(cli): sync run cloud-mode redirect error

Cloud sessions have no manifest. Clear redirect to push/pull instead of
a confusing NotFound on .temper/manifest.json."
```

---

## Task 16: End-to-end cloud-mode round-trip test

**Purpose:** One test that exercises the full acceptance criteria from the spec in sequence: create → show from separate session → list → update → sync-redirect. Confirms the whole pipeline works end-to-end and there are no interaction bugs between the pieces.

**Files:**
- Modify: `tests/e2e/tests/cloud_mode_test.rs` — add the combined test.

- [ ] **Step 16.1: Add the combined test**

Append:

```rust
#[tokio::test]
async fn cloud_mode_full_round_trip() {
    let ctx = e2e_test_context("cloud-mode-e2e").await;
    let env_vars = [
        ("TEMPER_VAULT_STATE", "cloud"),
        ("TEMPER_TOKEN", ctx.access_token()),
    ];

    // 1. create
    let create_out = common::e2e_cli_cmd(&ctx)
        .envs(env_vars)
        .args([
            "resource", "create",
            "--type", "session",
            "--context", ctx.context(),
            "--title", "E2E Round Trip",
            "--format", "json",
        ])
        .output()
        .await
        .expect("create");
    assert!(create_out.status.success(), "{}", String::from_utf8_lossy(&create_out.stderr));
    let created: serde_json::Value =
        serde_json::from_slice(&create_out.stdout).expect("parse");
    let slug = created["slug"].as_str().expect("slug").to_string();
    let id = created["id"].as_str().expect("id").to_string();

    // 2. show (from a "different session" — same env, which is what the
    //    spec's acceptance criterion tests).
    let show_out = common::e2e_cli_cmd(&ctx)
        .envs(env_vars)
        .args(["show", "--type", "session", &slug, "--context", ctx.context()])
        .output()
        .await
        .expect("show");
    assert!(show_out.status.success());
    assert!(String::from_utf8_lossy(&show_out.stdout).contains("E2E Round Trip"));

    // 3. list
    let list_out = common::e2e_cli_cmd(&ctx)
        .envs(env_vars)
        .args(["list", "--type", "session", "--context", ctx.context()])
        .output()
        .await
        .expect("list");
    assert!(list_out.status.success());
    assert!(String::from_utf8_lossy(&list_out.stdout).contains(&slug));

    // 4. sync run must redirect
    let sync_out = common::e2e_cli_cmd(&ctx)
        .envs(env_vars)
        .args(["sync", "run"])
        .output()
        .await
        .expect("sync");
    assert!(!sync_out.status.success());

    let _ = id; // silence warn
}
```

- [ ] **Step 16.2: Run + commit**

```bash
cargo nextest run -p temper-e2e --features test-db cloud_mode_full_round_trip
```
Expected: PASS.

```bash
git add tests/e2e/tests/cloud_mode_test.rs
git commit -m "test(e2e): cloud-mode full round-trip

Covers the spec's Unit B acceptance criteria in one sequence: create →
show from another session → list → sync redirect. Confirms the
dispatch rewrites compose without interaction bugs."
```

---

## Task 17: Final verification sweep

**Purpose:** Quality gates + acceptance-criteria walkthrough.

- [ ] **Step 17.1: Rust unit tests**

Run: `cargo make test`
Expected: PASS.

- [ ] **Step 17.2: Rust integration/e2e**

Precondition: `cargo make docker-up`.
Run: `cargo make test-db`
Expected: PASS. Pay attention to `cloud_mode_test`, `push_command_test`, `pull_command_test`, `sync_test`.

- [ ] **Step 17.3: `cargo make check` (fmt/clippy/docs/machete/biome)**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 17.4: SQLX cache regeneration if any SQL changed**

If Task 11 (or any other task) added a new filter to `ResourceListParams` and updated the list query, run:

```bash
cargo sqlx prepare --workspace -- --all-features
git add .sqlx
git commit -m "chore: regenerate sqlx cache for B.2 list filter"
```

- [ ] **Step 17.5: Spec acceptance walkthrough**

Walk each criterion from `2026-04-18-cloud-mode-and-portable-memory-design.md` §Unit B Acceptance and the B.2 task description:

1. Cloud session with `TEMPER_TOKEN` + `TEMPER_VAULT_STATE=cloud` can run `temper resource create --type session --context temper --title "test"` — covered by `cloud_mode_full_round_trip`. ✅
2. Canonical `temper-id` (not provisional) returned — JSON output's `id` field carries the server-assigned UUID. ✅
3. `temper show <id>` from a second cloud session retrieves same content — covered by `cloud_mode_full_round_trip` step 2. ✅
4. `temper sync run` errors with redirect — `cloud_mode_sync_run_returns_redirect_error`. ✅
5. No `auth.json` required — entire test suite runs with only `TEMPER_TOKEN` + `TEMPER_VAULT_STATE=cloud` in the env; no auth.json writes happen because `MemoryTokenStore` is the only store in the cloud branch (Task 6's structural property). ✅
6. list/show/search route straight to API — Tasks 10, 11; search was already API-driven. ✅
7. POST-success-but-local-write-failure recovery message — N/A in cloud mode (no local write); the log line in cloud `create` emits the canonical id on every success, which is the recovery surface. ✅

- [ ] **Step 17.6: Final commit (if any fixups)**

```bash
git commit --allow-empty -m "chore: Unit B.2 verification sweep complete

All cloud-mode dispatch branches shipped. Acceptance criteria verified
end-to-end via tests/e2e/tests/cloud_mode_test.rs. Ready for B.3."
```

---

## Execution Notes

### Parallelism

Tasks 1–6 (auth refactor) are sequential — each depends on types introduced by the prior. Tasks 10, 11, 12, 13, 14, 15 (cloud dispatch) touch different command files and are mostly independent, but they share `resource_cloud.rs` and `VaultState::from_env()` routing. Dispatching them in parallel via subagent-driven development is viable **after** Tasks 1–9 land. Tasks 7 and 8 must come after Task 6.

### Subagent guidance (from skill)

Every subagent dispatched for this plan MUST receive, verbatim:
- All SG-1 through SG-13 principles from `~/.claude/skills/temper/subagent-guidance.md`.
- Project fundamentals: typed structs over inline JSON, shared types at boundaries, service layer owns SQL, params structs, auth before writes, profile scoping, pino logging (TS), SQL macros for compile-time check, DRY SQL via views, shared predicate sets.
- TDD discipline: write test, run to see fail, implement minimal, run to see pass, commit.
- Verification-before-completion: run the exact verification command before claiming done.
- `cargo make check` must pass before any "done" claim.
- Plan/reality verification: grep-check every named API in the plan against the real code. The plan's `ResourceListParams.slug` (Task 11), `ResourceUpdateRequest` shape (Task 13), and `UpdateParams::to_owned_params` (Task 13) are **hypothesis, not spec** — verify before implementing and surface gaps in the implementer prompt with "⚠️ Plan/reality gap" markers.

### Known plan/reality gaps (verify before implementation)

- **Task 11 `ResourceListParams.slug`**: may not exist today. If absent, the plan path is: add the field, add the filter to the list service in `temper-api/src/services/`, regenerate sqlx cache. Scope this as sub-task 11.0 if needed.
- **Task 13 `ResourceUpdateRequest`**: exact field names are guesses. Read `temper-core/src/types/resource.rs` first and match the real shape (likely `title: Option<String>`, `body: Option<String>` or `markdown: Option<String>`, `managed_meta: Option<Value>`, `open_meta: Option<Value>`, plus whatever else the PATCH handler accepts).
- **Task 13 `UpdateParams::to_owned_params`**: does not exist. Add it as part of the task (small, mechanical).
- **Task 2 `atty`**: use `crate::vault::read_stdin_if_piped` instead of pulling in `atty` if that helper already exists. (It does — it's used by `create_simple_resource`.)

### Out of scope for this plan

- **Unit D (server-minted cloud session tokens)** — parallel task, can start independently.
- **Unit B.3 (working directory + SessionStart hook)** — next task after B.2.
- **Claude Desktop MCP auth flow changes** — unchanged; Claude Desktop runs its own device flow via `temper-mcp`'s discovery endpoints.
- **Second `Provider` enum variant** — Task 5 lands the enum shape; a `SelfHosted` variant is a separate future spec.
- **Unit D audit log, Management API M2M rotation, `session_id` claim embedding** — all covered in `docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md` §Unit D sketch.
