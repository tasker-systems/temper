# Okta Provider Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Temper's `/userinfo` email fallback work for any OIDC provider (via discovery) and teach `temper init` to emit Okta authorization-server URL shapes — removing the two Auth0-shape workarounds documented in `docs/guides/self-hosting-okta.md`.

**Architecture:** Two independent streams. **Stream A** (temper-api) replaces the hardcoded `{issuer}/userinfo` URL with an OIDC-discovery lookup memoized on `AppState`. **Stream B** (temper-cli) adds an internal `Idp` enum that drives `authorize_url`/`token_url` templating in `temper init`, while keeping the written provider label `auth0`. **Stream C** updates docs and closes the tracking tasks.

**Tech Stack:** Rust, axum, reqwest, serde/serde_json, tokio (`OnceCell`), clap, dialoguer, cargo-nextest.

## Global Constraints

- `--all-features` for all builds and clippy; clippy is `-D warnings`.
- Typed structs over inline JSON; params structs over >5 args; all public types derive `Debug`.
- temper-api carries `sqlx::query!` macros — run its tests with `SQLX_OFFLINE=true` (committed `.sqlx/` cache) so they compile without a live DB.
- nextest gotcha: never run a bare `cargo nextest run -p <crate>` for `temper-api`/`temper-cli` (both are lib+bin; the bin target hangs list-enumeration). Always scope unit tests with `--lib`.
- The written config provider label stays `"auth0"` for both Auth0 and Okta — it is the selector that ties `[auth].provider` to a `[[auth.providers]]` entry, not the IdP identity.
- Conventional-ish commit prefixes; this branch uses `fix(...)`, `feat(...)`, `docs(...)`. Branch: `jct/okta-provider-parity`.

---

## Stream A — Provider-agnostic `/userinfo` (temper-api)

### Task A1: Parse `userinfo_endpoint` from an OIDC discovery document

**Files:**
- Modify: `crates/temper-api/src/middleware/auth.rs` (add struct + pure parse fn + a new `#[cfg(test)] mod tests` at end of file)

**Interfaces:**
- Produces: `fn parse_userinfo_endpoint(body: &str) -> Result<String, String>`; `struct OidcDiscovery { userinfo_endpoint: Option<String> }`

- [ ] **Step 1: Write the failing tests** — append a test module at the end of `crates/temper-api/src/middleware/auth.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_userinfo_endpoint_auth0_shape() {
        let body = r#"{"issuer":"https://t.auth0.com/","userinfo_endpoint":"https://t.auth0.com/userinfo"}"#;
        assert_eq!(
            parse_userinfo_endpoint(body).unwrap(),
            "https://t.auth0.com/userinfo"
        );
    }

    #[test]
    fn parse_userinfo_endpoint_okta_shape() {
        let body = r#"{"issuer":"https://org.okta.com/oauth2/aus1","userinfo_endpoint":"https://org.okta.com/oauth2/aus1/v1/userinfo"}"#;
        assert_eq!(
            parse_userinfo_endpoint(body).unwrap(),
            "https://org.okta.com/oauth2/aus1/v1/userinfo"
        );
    }

    #[test]
    fn parse_userinfo_endpoint_missing_field_errors() {
        let body = r#"{"issuer":"https://x"}"#;
        assert!(parse_userinfo_endpoint(body).is_err());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-api --lib parse_userinfo_endpoint`
Expected: FAIL — `cannot find function parse_userinfo_endpoint` / `cannot find type OidcDiscovery`.

- [ ] **Step 3: Implement the struct + parse fn** — add directly above the existing `UserinfoResponse` struct (around auth.rs:173):

```rust
/// Subset of the OIDC discovery document (`/.well-known/openid-configuration`).
#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    userinfo_endpoint: Option<String>,
}

/// Parse the `userinfo_endpoint` out of an OIDC discovery document body.
fn parse_userinfo_endpoint(body: &str) -> Result<String, String> {
    let doc: OidcDiscovery =
        serde_json::from_str(body).map_err(|e| format!("discovery parse error: {e}"))?;
    doc.userinfo_endpoint
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "discovery document missing userinfo_endpoint".to_string())
}
```

(`serde_json` is a dependency of temper-api; the fully-qualified path needs no new import. `Deserialize` is already imported at auth.rs:7.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-api --lib parse_userinfo_endpoint`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/middleware/auth.rs
git commit -m "feat(auth): parse userinfo_endpoint from OIDC discovery doc"
```

---

### Task A2: Resolve userinfo via memoized discovery and rewire the fallback

**Files:**
- Modify: `crates/temper-api/src/state.rs:152-182` (add `userinfo_endpoint` field + init)
- Modify: `crates/temper-api/src/middleware/auth.rs` (add `discover_userinfo_endpoint`, change `fetch_email_from_userinfo` signature, rewire call site at ~auth.rs:105)

**Interfaces:**
- Consumes: `parse_userinfo_endpoint` (Task A1)
- Produces: `async fn discover_userinfo_endpoint(issuer: &str) -> Result<String, String>`; `AppState.userinfo_endpoint: Arc<tokio::sync::OnceCell<String>>`; `fetch_email_from_userinfo(userinfo_url: &str, access_token: &str)`

- [ ] **Step 1: Add the memo field to `AppState`** — in `crates/temper-api/src/state.rs`, add the field to the struct (after `backend_selection`, line 159):

```rust
    /// OIDC userinfo endpoint, resolved once per process via discovery on the
    /// first email-fallback. Lazy (not boot-time) so there is no startup
    /// coupling to the IdP; shared across `AppState` clones via `Arc`.
    pub userinfo_endpoint: Arc<tokio::sync::OnceCell<String>>,
```

And initialize it in `AppState::new` (inside the `Self { … }` literal, after `backend_selection`):

```rust
            userinfo_endpoint: Arc::new(tokio::sync::OnceCell::new()),
```

- [ ] **Step 2: Add the discovery wrapper** — in `crates/temper-api/src/middleware/auth.rs`, add directly below `parse_userinfo_endpoint`:

```rust
/// Resolve the OIDC userinfo endpoint for `issuer` via discovery.
async fn discover_userinfo_endpoint(issuer: &str) -> Result<String, String> {
    let base = issuer.trim_end_matches('/');
    let url = format!("{base}/.well-known/openid-configuration");
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("discovery request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("discovery returned status {}", resp.status()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("discovery read error: {e}"))?;
    parse_userinfo_endpoint(&body)
}
```

- [ ] **Step 3: Change `fetch_email_from_userinfo` to take a resolved URL** — replace its signature and the first two lines of its body (auth.rs:185-190). New version:

```rust
/// Fetch the user's email from a resolved OIDC `/userinfo` endpoint.
async fn fetch_email_from_userinfo(
    userinfo_url: &str,
    access_token: &str,
) -> Result<(String, Option<bool>), String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(userinfo_url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("userinfo request failed: {e}"))?;
```

(Leave the rest of the function — status check, json parse, email extraction — unchanged. Delete the old `let url = format!(...)` line.)

- [ ] **Step 4: Rewire the call site** — in `require_auth`, replace the `None =>` arm of the email match (currently auth.rs:105-112, the `fetch_email_from_userinfo(&state.config.auth_issuer, &token)` call) with:

```rust
                None => {
                    let endpoint = state
                        .userinfo_endpoint
                        .get_or_try_init(|| discover_userinfo_endpoint(&state.config.auth_issuer))
                        .await
                        .map_err(|e| {
                            tracing::warn!("OIDC discovery failed: {e}");
                            ApiError::Unauthorized(
                                "Token missing email claim and userinfo lookup failed".to_string(),
                            )
                        })?;
                    fetch_email_from_userinfo(endpoint, &token)
                        .await
                        .map_err(|e| {
                            tracing::warn!("Failed to fetch email from userinfo: {e}");
                            ApiError::Unauthorized(
                                "Token missing email claim and userinfo lookup failed".to_string(),
                            )
                        })?
                }
```

- [ ] **Step 5: Verify it compiles and existing tests pass**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-api --lib`
Expected: PASS — the A1 parse tests plus the existing `state.rs` auth tests still pass (adding a field with a `new()` default keeps them compiling).
Then: `cargo make check`
Expected: clippy clean (`-D warnings`), fmt clean.

(No new unit test here: `discover_userinfo_endpoint` and the call-site wiring are a thin network wrapper with no HTTP mock available in this crate. Coverage is the A1 parse tests plus the compile/clippy gate. This is an intentional, noted gap — not a silent one.)

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/state.rs crates/temper-api/src/middleware/auth.rs
git commit -m "fix(auth): resolve /userinfo via memoized OIDC discovery (provider-agnostic)"
```

---

## Stream B — `temper init` learns Okta URL shapes (temper-cli)

### Task B1: `Idp` enum + `provider_urls` templating

**Files:**
- Modify: `crates/temper-cli/src/commands/init.rs` (add `Idp` enum, `idp` field on `SelfHostConfig`, `provider_urls` fn, thread `idp` through `provider_and_cloud_sections` + `render_config_toml`; fix every `SelfHostConfig` literal)
- Modify: `crates/temper-cli/src/main.rs:102-107` (add `idp` to the literal)

**Interfaces:**
- Produces: `enum Idp { Auth0, Okta { auth_server_id: String } }`; `SelfHostConfig.idp: Idp`; `fn provider_urls(idp: &Idp, domain: &str) -> (String, String)`

- [ ] **Step 1: Write the failing test** — add to the `tests` module in `crates/temper-cli/src/commands/init.rs` (alongside `self_hosted_emits_derived_urls`):

```rust
    #[test]
    fn self_hosted_okta_emits_v1_urls_and_keeps_auth0_label() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::SelfHosted(SelfHostConfig {
                instance_url: "https://temper.acme.com".into(),
                auth_domain: "acme.okta.com".into(),
                client_id: "OktaCli123".into(),
                audience: "https://temper.acme.com/api".into(),
                idp: Idp::Okta {
                    auth_server_id: "aus1a2b3c".into(),
                },
            }),
        };
        let toml = render_config_toml(&answers);
        let cfg: TemperConfig = toml::from_str(&toml).expect("okta toml parses");
        cfg.validate().expect("okta config validates");
        assert_eq!(cfg.auth.provider, "auth0");
        let p = &cfg.auth.providers[0];
        assert_eq!(p.name, "auth0");
        assert_eq!(
            p.authorize_url,
            "https://acme.okta.com/oauth2/aus1a2b3c/v1/authorize"
        );
        assert_eq!(
            p.token_url,
            "https://acme.okta.com/oauth2/aus1a2b3c/v1/token"
        );
        assert_eq!(
            p.callback_url,
            "https://temper.acme.com/api/auth/cli-callback"
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p temper-cli --lib self_hosted_okta_emits`
Expected: FAIL — `cannot find type Idp` / missing field `idp`.

- [ ] **Step 3: Add the `Idp` enum** — in `init.rs`, directly above `SelfHostConfig` (init.rs:32):

```rust
/// Which IdP's OAuth endpoint shapes `temper init` should emit. The written
/// provider *label* stays "auth0" regardless — this only selects URL templating.
#[derive(Debug, Clone)]
pub enum Idp {
    /// Auth0 tenant: `https://{domain}/authorize`, `/oauth/token`.
    Auth0,
    /// Okta custom authorization server: `https://{domain}/oauth2/{id}/v1/*`.
    Okta { auth_server_id: String },
}
```

- [ ] **Step 4: Add the `idp` field to `SelfHostConfig`** — add to the struct (after `audience`, init.rs:42):

```rust
    /// Identity-provider URL shape to emit (Auth0 vs Okta authz server).
    pub idp: Idp,
```

- [ ] **Step 5: Add `provider_urls` and thread it through `provider_and_cloud_sections`** — add the pure fn above `provider_and_cloud_sections` (init.rs:410):

```rust
/// Map an `Idp` + domain to its (authorize_url, token_url) pair.
fn provider_urls(idp: &Idp, domain: &str) -> (String, String) {
    match idp {
        Idp::Auth0 => (
            format!("https://{domain}/authorize"),
            format!("https://{domain}/oauth/token"),
        ),
        Idp::Okta { auth_server_id } => (
            format!("https://{domain}/oauth2/{auth_server_id}/v1/authorize"),
            format!("https://{domain}/oauth2/{auth_server_id}/v1/token"),
        ),
    }
}
```

Change `provider_and_cloud_sections` to accept `idp: &Idp` as its last parameter, and replace its two URL lines (init.rs:420-421):

```rust
fn provider_and_cloud_sections(
    api_url: &str,
    auth_domain: &str,
    client_id: &str,
    audience: &str,
    idp: &Idp,
) -> (String, String) {
    let tv = |s: String| toml::Value::String(s).to_string();
    let (authorize, token) = provider_urls(idp, auth_domain);
    let authorize_url = tv(authorize);
    let token_url = tv(token);
    // ... (callback_url, client_id_toml, audience_toml, and the format! blocks unchanged)
```

(The `provider = "auth0"` / `name = "auth0"` lines in the `format!` stay exactly as they are.)

- [ ] **Step 6: Update `render_config_toml` callers** — in the `match &answers.auth_choice` (init.rs:459), pass the idp to each arm:

```rust
        AuthChoice::Hosted => provider_and_cloud_sections(
            HOSTED_API_URL,
            HOSTED_AUTH_DOMAIN,
            HOSTED_CLIENT_ID,
            HOSTED_AUDIENCE,
            &Idp::Auth0,
        ),
        AuthChoice::SelfHosted(sh) => provider_and_cloud_sections(
            &sh.instance_url,
            &sh.auth_domain,
            &sh.client_id,
            &sh.audience,
            &sh.idp,
        ),
```

- [ ] **Step 7: Fix every other `SelfHostConfig` literal** — the new required field breaks the interactive wizard, the dispatch site, and existing tests. Build to get the exact list, then add `idp: Idp::Auth0,` to each:

Run: `cargo build -p temper-cli --lib 2>&1 | grep -A1 "missing field"`
Then add `idp: Idp::Auth0,` to:
- `gather_answers` (the `SelfHosted(SelfHostConfig { … })` literal at init.rs:264)
- each test literal in the `tests` module (e.g. `self_hosted_emits_derived_urls`, and any other `SelfHostConfig { … }`)

And in `crates/temper-cli/src/main.rs` (the literal at main.rs:102-107), add as the last field:

```rust
                        idp: temper_cli::commands::init::Idp::Auth0,
```

- [ ] **Step 8: Run the Okta test + full lib suite**

Run: `cargo nextest run -p temper-cli --lib`
Expected: PASS — the new Okta test plus all existing init tests.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-cli/src/commands/init.rs crates/temper-cli/src/main.rs
git commit -m "feat(init): Idp enum drives Okta vs Auth0 URL templating"
```

---

### Task B2: Headless `--idp` / `--auth-server-id` flags + validated assembly

**Files:**
- Modify: `crates/temper-cli/src/commands/init.rs` (add `self_host_from_flags`, update `run_non_interactive` summary label)
- Modify: `crates/temper-cli/src/cli.rs:60-78` (add `idp`, `auth_server_id` flags)
- Modify: `crates/temper-cli/src/main.rs:88-110` (call `self_host_from_flags`)

**Interfaces:**
- Consumes: `Idp`, `SelfHostConfig` (Task B1)
- Produces: `pub fn self_host_from_flags(instance_url, auth_domain, client_id, audience, idp, auth_server_id: Option<String>) -> Result<Option<SelfHostConfig>>`

- [ ] **Step 1: Write the failing tests** — add to the `tests` module in `init.rs`:

```rust
    #[test]
    fn flags_auth0_builds_auth0_idp_and_trims_slash() {
        let sh = self_host_from_flags(
            Some("https://x.com/".into()),
            Some("d.auth0.com".into()),
            Some("cid".into()),
            Some("https://x.com/api".into()),
            Some("auth0".into()),
            None,
        )
        .unwrap()
        .unwrap();
        assert!(matches!(sh.idp, Idp::Auth0));
        assert_eq!(sh.instance_url, "https://x.com");
    }

    #[test]
    fn flags_okta_without_server_id_errors() {
        let res = self_host_from_flags(
            Some("https://x.com".into()),
            Some("o.okta.com".into()),
            Some("cid".into()),
            Some("https://x.com/api".into()),
            Some("okta".into()),
            None,
        );
        assert!(res.is_err());
    }

    #[test]
    fn flags_okta_with_server_id_builds_okta_idp() {
        let sh = self_host_from_flags(
            Some("https://x.com".into()),
            Some("o.okta.com".into()),
            Some("cid".into()),
            Some("https://x.com/api".into()),
            Some("okta".into()),
            Some("aus9".into()),
        )
        .unwrap()
        .unwrap();
        assert!(matches!(sh.idp, Idp::Okta { .. }));
    }

    #[test]
    fn flags_none_when_instance_missing() {
        let res = self_host_from_flags(None, None, None, None, Some("auth0".into()), None).unwrap();
        assert!(res.is_none());
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo nextest run -p temper-cli --lib flags_`
Expected: FAIL — `cannot find function self_host_from_flags`.

- [ ] **Step 3: Implement `self_host_from_flags`** — add to `init.rs` (near `run_non_interactive`). `TemperError` and `Result` are already imported (init.rs:13):

```rust
/// Assemble a `SelfHostConfig` from `--no-interactive` flags. Returns
/// `Ok(None)` when the instance quad is absent (local-only init). Errors when
/// `--idp okta` is missing `--auth-server-id`, or `--idp` is unrecognized.
pub fn self_host_from_flags(
    instance_url: Option<String>,
    auth_domain: Option<String>,
    client_id: Option<String>,
    audience: Option<String>,
    idp: Option<String>,
    auth_server_id: Option<String>,
) -> Result<Option<SelfHostConfig>> {
    let (instance_url, auth_domain, client_id, audience) =
        match (instance_url, auth_domain, client_id, audience) {
            (Some(i), Some(d), Some(c), Some(a)) => (i, d, c, a),
            _ => return Ok(None),
        };
    let idp = match idp.as_deref() {
        None | Some("auth0") => Idp::Auth0,
        Some("okta") => {
            let id = auth_server_id.filter(|s| !s.is_empty()).ok_or_else(|| {
                TemperError::Config(
                    "--auth-server-id is required when --idp okta".to_string(),
                )
            })?;
            Idp::Okta { auth_server_id: id }
        }
        Some(other) => {
            return Err(TemperError::Config(format!(
                "unknown --idp '{other}' (expected 'auth0' or 'okta')"
            )))
        }
    };
    Ok(Some(SelfHostConfig {
        instance_url: instance_url.trim_end_matches('/').to_string(),
        auth_domain,
        client_id,
        audience,
        idp,
    }))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p temper-cli --lib flags_`
Expected: PASS (4 tests).

- [ ] **Step 5: Add the clap flags** — in `crates/temper-cli/src/cli.rs`, inside the `Init { … }` variant (after `auth_audience`, cli.rs:77):

```rust
        /// Self-host: identity provider URL shape (default: auth0)
        #[arg(long, default_value = "auth0")]
        idp: String,
        /// Self-host: Okta authorization server ID (required with --idp okta)
        #[arg(long)]
        auth_server_id: Option<String>,
```

- [ ] **Step 6: Wire the dispatch** — in `crates/temper-cli/src/main.rs`, replace the `Commands::Init { … }` destructure + the `self_host` match block (main.rs:88-110) with:

```rust
        Commands::Init {
            path,
            no_interactive,
            instance_url,
            auth_domain,
            auth_client_id,
            auth_audience,
            idp,
            auth_server_id,
        } => {
            let vault_path = path
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
            let self_host = temper_cli::commands::init::self_host_from_flags(
                instance_url,
                auth_domain,
                auth_client_id,
                auth_audience,
                Some(idp),
                auth_server_id,
            )?;
            temper_cli::commands::init::run(
                &vault_path,
                no_interactive,
                true,
                output_format,
                self_host,
            )
        }
```

- [ ] **Step 7: Update the non-interactive summary label** — in `run_non_interactive` (init.rs:194-198), make the self-hosted label idp-aware:

```rust
    let auth = match &answers.auth_choice {
        AuthChoice::Hosted => "auth0".to_string(),
        AuthChoice::SelfHosted(sh) => match sh.idp {
            Idp::Auth0 => "auth0 (self-hosted)".to_string(),
            Idp::Okta { .. } => "okta (self-hosted)".to_string(),
        },
        AuthChoice::None => "none".to_string(),
    };
```

- [ ] **Step 8: Verify the whole crate builds + lib tests pass**

Run: `cargo nextest run -p temper-cli --lib`
Expected: PASS.
Then: `cargo build -p temper-cli` (builds the bin too, exercising the new main.rs dispatch).
Expected: clean build.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-cli/src/commands/init.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "feat(init): --idp/--auth-server-id headless flags with okta validation"
```

---

### Task B3: Interactive Okta selection in the wizard

**Files:**
- Modify: `crates/temper-cli/src/commands/init.rs` (`gather_answers` self-hosted branch init.rs:247-270; `print_summary` label init.rs:288-293)

**Interfaces:**
- Consumes: `Idp`, `SelfHostConfig` (Task B1)

- [ ] **Step 1: Add the IdP select + Okta prompts** — in `gather_answers`, replace the self-hosted arm (the `1 => { … }` block at init.rs:247-270) with:

```rust
        1 => {
            let instance_url: String = Input::with_theme(&theme)
                .with_prompt("Instance base URL (e.g. https://temper.acme.com)")
                .interact_text()
                .map_err(prompt_err)?;
            let idp_idx = Select::with_theme(&theme)
                .with_prompt("Identity provider")
                .default(0)
                .items(["Auth0", "Okta"])
                .interact()
                .map_err(prompt_err)?;
            let auth_domain: String = Input::with_theme(&theme)
                .with_prompt(if idp_idx == 1 {
                    "Okta org domain (e.g. acme.okta.com)"
                } else {
                    "Auth0 tenant domain (e.g. acme.us.auth0.com)"
                })
                .interact_text()
                .map_err(prompt_err)?;
            let idp = if idp_idx == 1 {
                let auth_server_id: String = Input::with_theme(&theme)
                    .with_prompt("Okta authorization server ID (e.g. aus1a2b3c)")
                    .interact_text()
                    .map_err(prompt_err)?;
                Idp::Okta {
                    auth_server_id: auth_server_id.trim().to_string(),
                }
            } else {
                Idp::Auth0
            };
            let client_id: String = Input::with_theme(&theme)
                .with_prompt("CLI application client_id")
                .interact_text()
                .map_err(prompt_err)?;
            let audience: String = Input::with_theme(&theme)
                .with_prompt("API audience (e.g. https://temper.acme.com/api)")
                .interact_text()
                .map_err(prompt_err)?;
            AuthChoice::SelfHosted(SelfHostConfig {
                instance_url: instance_url.trim().trim_end_matches('/').to_string(),
                auth_domain: auth_domain.trim().to_string(),
                client_id: client_id.trim().to_string(),
                audience: audience.trim().to_string(),
                idp,
            })
        }
```

- [ ] **Step 2: Make `print_summary` idp-aware** — replace the `auth_label` match (init.rs:288-293):

```rust
    let auth_label = match &answers.auth_choice {
        AuthChoice::Hosted => "auth0",
        AuthChoice::SelfHosted(sh) => match &sh.idp {
            Idp::Auth0 => "auth0 (self-hosted)",
            Idp::Okta { .. } => "okta (self-hosted)",
        },
        AuthChoice::None => "none",
    };
```

- [ ] **Step 3: Verify build + lib tests**

Run: `cargo build -p temper-cli && cargo nextest run -p temper-cli --lib`
Expected: clean build, all lib tests pass. (The interactive prompts have no automated test — dialoguer drives a TTY. Manual smoke is in Step 4.)

- [ ] **Step 4: Manual smoke (optional but recommended)**

Run: `cargo run -p temper-cli -- init /tmp/okta-smoke-vault`
Walk: self-hosted → Okta → enter `acme.okta.com`, server id `aus1a2b3c`, a client_id, audience. Decline at "Proceed?" (or proceed and inspect `~/.config/temper/config.toml`). Confirm the summary shows `okta (self-hosted)` and the would-be `authorize_url` carries `/oauth2/aus1a2b3c/v1/authorize`.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/init.rs
git commit -m "feat(init): interactive Okta provider selection in the wizard"
```

---

## Stream C — Docs + task closure

### Task C1: Update the Okta guide and close the tracking tasks

**Files:**
- Modify: `docs/guides/self-hosting-okta.md` (email-claim section, Verify note, CLI section, Known limitations)

- [ ] **Step 1: Downgrade the email-claim requirement** — change the heading `### Add an `email` claim to the access token (required)` to `### Add an `email` claim to the access token (recommended)`, and replace its first paragraph with:

```markdown
Temper resolves the user's email from the access token's `email` claim. When that claim is absent it falls back to the OIDC `/userinfo` endpoint, which Temper now resolves via discovery (`{issuer}/.well-known/openid-configuration`), so the fallback works against Okta. Putting `email` directly on the access token is still **recommended** — it's the fast path and avoids a per-process discovery + userinfo round-trip — but it is no longer mandatory.
```

Update the sentence beginning "Without this claim, login fails…" to:

```markdown
With neither the claim nor a reachable `/userinfo` (e.g. the token lacks the `email` scope), login fails with `Token missing email claim and userinfo lookup failed`.
```

- [ ] **Step 2: Fix the Verify troubleshooting note** — in the `## Verify` section, replace the `If login fails with …` paragraph with:

```markdown
If login fails with `Token missing email claim and userinfo lookup failed`, either add the access-token `email` claim (above) or ensure the CLI's granted scopes include `email` so the `/userinfo` fallback can return it.
```

- [ ] **Step 3: Replace the hand-written-config directive** — in `## Configure the CLI (Okta)`, replace the leading blockquote (`> **`temper init` is Auth0-specific.** …`) with:

```markdown
> **`temper init` now supports Okta.** Interactively, choose **self-hosted → Okta** and enter your authorization server ID. Headless, pass `--idp okta --auth-server-id <authServerId>` alongside the existing self-host flags:
>
> ```sh
> temper init --no-interactive \
>   --instance-url https://<instance> \
>   --auth-domain <okta-domain> \
>   --idp okta --auth-server-id <authServerId> \
>   --auth-client-id <cli-app-client-id> \
>   --auth-audience <custom-auth-server-audience>
> ```
>
> The hand-written block below remains valid as a reference (note `provider`/`name` stay `auth0`).
```

- [ ] **Step 4: Remove the resolved limitations** — delete the entire `## Known limitations` section (both the `/userinfo` bullet and the `temper init` bullet, plus the heading and its intro sentence), since both are now fixed. Remove the in-page link to `#known-limitations` from the email-claim section (change `See [Known limitations](#known-limitations) for the underlying code issue.` to a period-terminated sentence without the link).

- [ ] **Step 5: Verify the doc** — confirm no dangling anchors and the file reads cleanly:

Run: `grep -n "known-limitations\|(required)" docs/guides/self-hosting-okta.md`
Expected: no matches (the anchor link and the `(required)` heading are both gone).

- [ ] **Step 6: Commit**

```bash
git add docs/guides/self-hosting-okta.md
git commit -m "docs(self-hosting-okta): userinfo + init fixes land; drop workarounds"
```

- [ ] **Step 7: Close the tracking vault tasks**

```bash
temper resource update okta-userinfo-019ef9b1-c6c6-73b0-b165-d85f74552bb4 --stage done
temper resource update temper-init-019ef9b1-d409-7092-bbed-e124c201e0cf --stage done
```

---

## Final gate (after all tasks)

- [ ] **Full workspace check + unit tests**

Run: `cargo make check` then `cargo make test`
Expected: fmt + clippy (`-D warnings`) + machete clean; all unit tests pass.

- [ ] **Consolidated review** — request a code review of the full branch diff against this plan and the spec, then address findings before opening the PR.

---

## Self-Review

**Spec coverage:**
- Task A (userinfo discovery) → A1 (parse) + A2 (discover + memo + rewire). ✓
- Memoization via `Arc<OnceCell>` on `AppState`, lazy → A2 Steps 1 & 4. ✓
- No `{issuer}/userinfo` fallback retained → A2 Step 3 deletes the old URL line; Step 4 routes only through discovery. ✓
- Task B (init Okta) label stays `auth0` → B1 Step 5 (format! unchanged) + B1 test asserts `provider=="auth0"`. ✓
- `Idp` enum + `provider_urls` → B1. Headless flags + validation → B2. Interactive → B3. ✓
- Docs: email mandatory→recommended, init path, remove both limitations → C1. Task closure → C1 Step 7. ✓
- Out-of-scope (temper-client Provider enum, server env) — untouched by all tasks. ✓

**Placeholder scan:** No TBD/TODO; every code step shows full code; the one "build to list missing-field sites" step (B1 Step 7) is a deterministic compiler-driven enumeration, not a vague instruction.

**Type consistency:** `Idp`, `SelfHostConfig.idp`, `provider_urls(&Idp, &str)`, `self_host_from_flags(...) -> Result<Option<SelfHostConfig>>`, `parse_userinfo_endpoint(&str) -> Result<String,String>`, `discover_userinfo_endpoint(&str)`, `fetch_email_from_userinfo(&str,&str)`, `AppState.userinfo_endpoint: Arc<OnceCell<String>>` — names/signatures consistent across producing and consuming tasks.
