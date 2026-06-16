# Enterprise Self-Host Enablement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a non-`temperkb.io` deployment a first-class, documented path for the temper API + MCP + CLI surfaces by stripping baked-in `temperkb` defaults from the shipped binary, adding a self-host `temper init` flow, and authoring an operator runbook grounded in the live deployment's config.

**Architecture:** The deployable API/MCP are already env-driven; the work is in the CLI/config layer (`temper-core` defaults, `temper-cli` `init`) plus docs. A freshly-built config ships **unconfigured** (no provider, no api_url) instead of pointing at the hosted SaaS. The temperkb constants survive only as a `temper init` "hosted preset," so `temperkb.io` becomes just another deployment configured through the same path an enterprise uses.

**Tech Stack:** Rust (temper-core, temper-cli, clap, dialoguer, validator 0.20, toml, serde), TypeScript (temper-cloud, vitest, Vercel functions), Markdown (mdBook-adjacent docs).

**Spec:** `docs/superpowers/specs/2026-06-16-enterprise-self-host-enablement-design.md`

**Scope:** API + MCP + CLI. The `temper-ui` SvelteKit web app and its Auth0 Regular-Web-App flow are **out of scope** and documented as deferred.

**Commands:** `cargo make check` (clippy+fmt, `SQLX_OFFLINE=true`), `cargo make test` (unit, no DB), `cargo nextest run -p <crate> <test>` (single test). TypeScript: `cd packages/temper-cloud && bun run test` / `bun run typecheck` / `bun run check`.

---

## File Structure

| File | Responsibility | Change |
|------|----------------|--------|
| `crates/temper-core/src/types/config.rs` | Canonical config + defaults | Strip temperkb defaults; relax `api_url` validation to allow empty |
| `crates/temper-client/src/config.rs` | Client config resolution + its tests | Update default-posture test assertions |
| `crates/temper-cli/src/commands/init.rs` | `temper init` wizard + TOML emitter | Add Hosted/SelfHosted/None; hosted-preset constants; self-host emitter |
| `crates/temper-cli/src/cli.rs` | clap command defs | Add self-host flags to `Init` |
| `crates/temper-cli/src/main.rs` | clap dispatch | Thread self-host flags into `init::run` |
| `packages/temper-cloud/src/cli-callback.ts` | Pure CLI-callback relay logic (NEW) | Extract from the Vercel fn; host-neutral parse |
| `packages/temper-cloud/tests/cli-callback.test.ts` | Relay unit tests (NEW) | Cover code/state extraction + redirect |
| `api/auth/cli-callback.ts` | Thin Vercel entry point | Delegate to temper-cloud/src |
| `crates/temper-api/.env.template` | API env contract example | Fix stale Neon-Auth → Auth0, host-neutral |
| `.env.template` | Repo env example | Reorganize; mark UI vars out-of-scope |
| `docs/guides/self-hosting.md` | Operator runbook (NEW) | The lasting deliverable |
| `docs/guides/install.md` | Install guide | Add a pointer to the self-hosting runbook |

---

## Task 1: Strip baked-in temperkb defaults from temper-core config

**Files:**
- Modify: `crates/temper-core/src/types/config.rs`
- Test: same file (`#[cfg(test)] mod tests`)

The default `TemperConfig` must ship **unconfigured**: `auth.provider = "none"`, empty `auth.providers`, empty `cloud.api_url`, empty callback fallback. Because `api_url` carries `#[validate(url)]` (which rejects `""`), validation is relaxed to a custom validator that allows empty (unconfigured) but rejects non-empty garbage.

- [ ] **Step 1: Update the existing default-asserting tests to the new unconfigured posture (write failing tests first)**

In `crates/temper-core/src/types/config.rs`, find the tests that assert temperkb defaults (around lines 585–635: the TOML round-trip fixture and `returns_defaults*`/`default_*` tests). Replace the temperkb assertions with the unconfigured posture. Specifically, locate the test that constructs `TemperConfig::default()` and asserts `config.cloud.api_url == "https://temperkb.io"` (≈line 625) and the provider assertions (≈588–589, 631–632), and rewrite them:

```rust
    #[test]
    fn default_config_is_unconfigured_for_cloud() {
        // No baked-in default: a fresh binary must not point at the hosted SaaS.
        let config = TemperConfig::default();
        assert_eq!(config.auth.provider, "none");
        assert!(
            config.auth.providers.is_empty(),
            "default config must ship with no auth providers"
        );
        assert_eq!(config.cloud.api_url, "", "api_url must be unset by default");
    }

    #[test]
    fn default_config_validates() {
        // The unconfigured default (empty api_url) must still pass validation,
        // so commands that load-then-validate don't choke before `temper init`.
        use validator::Validate;
        TemperConfig::default()
            .validate()
            .expect("unconfigured default config must validate");
    }
```

Keep the existing `validate_rejects_malformed_api_url` test (it sets `api_url = "not a url"` and expects a validation error) — it still must pass after the validator change below. If a test named `default_vault_path_is_documents_temper_vault` exists, leave it unchanged. Delete any now-obsolete test that asserts the temperkb provider URLs/client_id as a *default* (those values now live only in `temper-cli`'s hosted preset — Task 3).

- [ ] **Step 2: Run the new tests to verify they fail**

Run: `cargo nextest run -p temper-core default_config_is_unconfigured_for_cloud default_config_validates`
Expected: FAIL — `default_config_is_unconfigured_for_cloud` fails on `provider == "none"` (currently `"auth0"`) and `api_url == ""` (currently temperkb).

- [ ] **Step 3: Flip `default_auth_provider` and `AuthConfig::default` to unconfigured**

Replace the current `default_auth_provider` (≈line 150) and `AuthConfig::default` (≈lines 154–175):

```rust
fn default_auth_provider() -> String {
    "none".to_string()
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            provider: default_auth_provider(),
            providers: Vec::new(),
            path: None,
        }
    }
}
```

- [ ] **Step 4: Empty the `api_url` default and the callback fallback**

Replace `default_api_url` (≈line 291) and `default_callback_url` (≈line 131):

```rust
fn default_api_url() -> String {
    String::new()
}
```

```rust
fn default_callback_url() -> String {
    // No baked-in host. `temper init` always writes an explicit callback_url
    // (derived from the instance URL); this fallback only applies to a
    // hand-edited config that omits the field, where empty surfaces a clear
    // "callback not configured" failure rather than silently using temperkb.
    String::new()
}
```

- [ ] **Step 5: Relax `api_url` validation to allow empty**

On `CloudSection.api_url` (≈lines 277–280), replace the `#[validate(url(...))]` attribute with a custom validator, and add the validator function below `default_api_url`:

```rust
    /// API base URL (overridden by `TEMPER_API_URL` environment variable).
    /// Empty means "unconfigured" — set by `temper init`.
    #[serde(default = "default_api_url")]
    #[validate(custom(function = "validate_optional_api_url"))]
    pub api_url: String,
```

```rust
/// Allow an empty `api_url` (the unconfigured default) while still rejecting a
/// non-empty value that isn't a valid URL. validator 0.20 exposes
/// `ValidateUrl::validate_url` for `&str`.
fn validate_optional_api_url(value: &str) -> Result<(), validator::ValidationError> {
    use validator::ValidateUrl;
    if value.is_empty() || value.validate_url() {
        Ok(())
    } else {
        Err(validator::ValidationError::new("api_url_invalid"))
    }
}
```

- [ ] **Step 6: Run temper-core config tests**

Run: `cargo nextest run -p temper-core --lib types::config`
Expected: PASS — new default tests pass, `validate_rejects_malformed_api_url` still passes (since `"not a url"` is non-empty and not a URL), round-trip tests pass.

- [ ] **Step 7: Workspace check**

Run: `cargo make check`
Expected: PASS (clippy + fmt clean). If clippy flags the new function as unused in any feature combo, it is referenced by the derive attribute — confirm the `#[validate(custom(...))]` path string matches the function name exactly.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core/src/types/config.rs
git commit -m "feat(config): ship unconfigured by default — strip baked-in temperkb defaults

default TemperConfig now has provider=none, empty providers, empty api_url
(validation relaxed to allow the unconfigured empty value). The OSS binary no
longer points at the hosted SaaS; instance + provider are set via temper init."
```

---

## Task 2: Update temper-client default-posture tests

**Files:**
- Modify: `crates/temper-client/src/config.rs` (the `#[cfg(test)] mod tests`)

These tests currently assert the temperkb defaults (api_url, authorize/token/callback URLs, provider client_id/audience). After Task 1 the defaults are unconfigured. Update them so they document the new contract: a default config has no usable provider, and `oauth_config` returns the "cloud disabled" error.

- [ ] **Step 1: Rewrite the default-asserting tests**

In `crates/temper-client/src/config.rs`, locate the tests asserting `config.cloud.api_url == "https://temperkb.io"` (≈line 165, ≈226) and the provider assertions (≈167–177, ≈367–374). Replace the temperkb expectations:

```rust
    #[test]
    fn default_config_has_no_usable_provider() {
        let config = TemperConfig::default();
        assert_eq!(config.auth.provider, "none");
        assert!(config.auth.providers.is_empty());
        assert_eq!(config.cloud.api_url, "");
    }

    #[test]
    fn oauth_config_errors_for_unconfigured_default() {
        // A fresh, unconfigured vault cannot build an OAuth config — the caller
        // is told to run `temper init`.
        let config = TemperConfig::default();
        let err = oauth_config(&config).expect_err("unconfigured config has no provider");
        let msg = err.to_string();
        assert!(
            msg.contains("cloud sync is disabled") || msg.contains("temper init"),
            "expected a 'run temper init' style error, got: {msg}"
        );
    }
```

For any test that needs a *configured* client (e.g. `build_client_from_uses_config_api_url` ≈line 429, or `api_url` env-precedence tests ≈225–233), keep them but build the config explicitly with a provider/api_url in-test rather than relying on the default. Example for the env-precedence test:

```rust
    #[test]
    fn api_url_env_var_takes_priority() {
        let mut config = TemperConfig::default();
        config.cloud.api_url = "https://config-host.example.com".to_string();
        let url = temp_env::with_var("TEMPER_API_URL", Some("https://env-host.example.com"), || {
            api_url(&config)
        });
        assert_eq!(url, "https://env-host.example.com");
        let url = temp_env::with_var("TEMPER_API_URL", None::<&str>, || api_url(&config));
        assert_eq!(url, "https://config-host.example.com");
    }
```

(Adjust to match the existing test's exact name/shape — the point is: don't assert temperkb as a *default*; set the value explicitly when a configured client is needed.)

- [ ] **Step 2: Run temper-client tests**

Run: `cargo nextest run -p temper-client config`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-client/src/config.rs
git commit -m "test(client): assert unconfigured-default posture instead of temperkb defaults"
```

---

## Task 3: Add the self-host flow to `temper init` (wizard + emitter)

**Files:**
- Modify: `crates/temper-cli/src/commands/init.rs`
- Test: same file (`#[cfg(test)] mod tests`)

Replace the two-way `AuthChoice` (Auth0 | None) with a three-way model: **Hosted** (the temperkb preset — the only place those constants now live), **SelfHosted** (carries instance + Auth0 inputs), **None**. The interactive wizard prompts for the three; the non-interactive path builds `SelfHosted` from flags (Task 4) or falls back to `None`.

- [ ] **Step 1: Write failing tests for the new emitter behavior**

Add these tests to the `tests` module in `init.rs`. They define the new `AuthChoice`/`SelfHostConfig` shape and the emitter contract:

```rust
    #[test]
    fn hosted_preset_emits_temperkb_provider_and_api_url() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::Hosted,
        };
        let toml = render_config_toml(&answers);
        let cfg: TemperConfig = toml::from_str(&toml).expect("hosted toml parses");
        cfg.validate().expect("hosted config validates");
        assert_eq!(cfg.auth.provider, "auth0");
        assert_eq!(cfg.cloud.api_url, "https://temperkb.io");
        let p = &cfg.auth.providers[0];
        assert_eq!(p.authorize_url, "https://temperkb.us.auth0.com/authorize");
        assert_eq!(p.audience, "https://temperkb.io/api");
        assert_eq!(p.callback_url, "https://temperkb.io/api/auth/cli-callback");
    }

    #[test]
    fn self_hosted_emits_derived_urls() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::SelfHosted(SelfHostConfig {
                instance_url: "https://temper.acme.com".into(),
                auth_domain: "acme.us.auth0.com".into(),
                client_id: "AcMeClientId123".into(),
                audience: "https://temper.acme.com/api".into(),
            }),
        };
        let toml = render_config_toml(&answers);
        let cfg: TemperConfig = toml::from_str(&toml).expect("self-hosted toml parses");
        cfg.validate().expect("self-hosted config validates");
        assert_eq!(cfg.cloud.api_url, "https://temper.acme.com");
        assert_eq!(cfg.auth.provider, "auth0");
        let p = &cfg.auth.providers[0];
        assert_eq!(p.authorize_url, "https://acme.us.auth0.com/authorize");
        assert_eq!(p.token_url, "https://acme.us.auth0.com/oauth/token");
        assert_eq!(p.client_id, "AcMeClientId123");
        assert_eq!(p.audience, "https://temper.acme.com/api");
        assert_eq!(p.callback_url, "https://temper.acme.com/api/auth/cli-callback");
        assert_eq!(
            p.scopes,
            vec!["openid", "profile", "email", "offline_access"]
        );
    }

    #[test]
    fn none_choice_omits_cloud_and_providers() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::None,
        };
        let toml = render_config_toml(&answers);
        assert!(toml.contains(r#"provider = "none""#));
        assert!(!toml.contains("[[auth.providers]]"));
        let cfg: TemperConfig = toml::from_str(&toml).expect("none toml parses");
        cfg.validate().expect("none config validates");
        assert_eq!(cfg.cloud.api_url, "");
    }
```

Then update the existing tests that used `AuthChoice::Auth0`: replace every `auth_choice: AuthChoice::Auth0` with `auth_choice: AuthChoice::Hosted`. In `default_answers_generate_complete_config`, the assertion `api_url = "https://temperkb.io"` stays valid under Hosted. In `rendered_toml_parses_and_validates_auth0`, rename to `..._hosted` and use `AuthChoice::Hosted`. In `auth0_writes_array_of_tables_format` and `extra_contexts_go_into_subscriptions` and `mock_ensurer_called_*` and `apply_answers_*`, swap `Auth0` → `Hosted`. In `render_init_summary_json_includes_vault_path`, leave `auth: "auth0"` (summary string for hosted; see Step 4).

- [ ] **Step 2: Run to verify failure**

Run: `cargo nextest run -p temper-cli init`
Expected: FAIL to compile — `AuthChoice::Hosted` / `AuthChoice::SelfHosted` / `SelfHostConfig` don't exist yet.

- [ ] **Step 3: Define the new types and hosted-preset constants**

Replace the `AuthChoice` enum (≈lines 26–30) and add `SelfHostConfig` + preset constants. The constants are the **single home** for the temperkb values removed from temper-core in Task 1:

```rust
/// Hosted-instance preset values (the only place the temperkb.io constants
/// live after the binary stopped baking them into config defaults).
const HOSTED_API_URL: &str = "https://temperkb.io";
const HOSTED_AUTH_DOMAIN: &str = "temperkb.us.auth0.com";
const HOSTED_CLIENT_ID: &str = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF";
const HOSTED_AUDIENCE: &str = "https://temperkb.io/api";

/// Per-instance OAuth inputs for a self-hosted deployment.
#[derive(Debug, Clone)]
pub struct SelfHostConfig {
    /// Instance base URL, e.g. `https://temper.acme.com`.
    pub instance_url: String,
    /// Auth0 tenant domain, e.g. `acme.us.auth0.com`.
    pub auth_domain: String,
    /// Auth0 native-app client_id for the CLI.
    pub client_id: String,
    /// API audience / resource identifier, e.g. `https://temper.acme.com/api`.
    pub audience: String,
}

/// User selection for instance + auth provider.
#[derive(Debug, Clone)]
pub enum AuthChoice {
    /// temperkb.io hosted preset.
    Hosted,
    /// A self-hosted instance with its own Auth0 tenant.
    SelfHosted(SelfHostConfig),
    /// Local-only, no cloud sync.
    None,
}
```

Note: `AuthChoice` is no longer `Copy` (it owns `String`s via `SelfHosted`). It remains `Clone`.

- [ ] **Step 4: Update all `AuthChoice` match/compare sites**

`apply_answers` (≈line 287) compares `answers.auth_choice == AuthChoice::Auth0`. Replace with a `matches!` cloud-enabled check:

```rust
        // Cloud-ensure step: only when an instance is configured.
        if !matches!(answers.auth_choice, AuthChoice::None) {
            ensure_server_contexts(answers, ensurer)?;
        }
```

`print_summary` (≈line 229) and `run_non_interactive` (≈line 160) build an auth label. Replace both with:

```rust
    let auth_label = match &answers.auth_choice {
        AuthChoice::Hosted => "auth0",
        AuthChoice::SelfHosted(_) => "auth0 (self-hosted)",
        AuthChoice::None => "none",
    };
```

(For `run_non_interactive`'s `InitSummary.auth` field, use `.to_string()` on the same match.)

- [ ] **Step 5: Implement the per-variant TOML emitter**

Replace `render_config_toml`'s `auth_section` + `[cloud]` construction (≈lines 363–405). Introduce a shared helper that builds the provider block and the api_url from parts, then dispatch by variant:

```rust
/// Build the `[auth]` + `[[auth.providers]]` block and the `[cloud]` api_url
/// line for a configured instance (hosted or self-hosted).
fn provider_and_cloud_sections(
    api_url: &str,
    auth_domain: &str,
    client_id: &str,
    audience: &str,
) -> (String, String) {
    let auth = format!(
        r#"[auth]
provider = "auth0"

[[auth.providers]]
name = "auth0"
authorize_url = "https://{auth_domain}/authorize"
token_url = "https://{auth_domain}/oauth/token"
client_id = "{client_id}"
audience = "{audience}"
callback_url = "{api_url}/api/auth/cli-callback"
scopes = ["openid", "profile", "email", "offline_access"]
"#
    );
    let cloud = format!("[cloud]\napi_url = \"{api_url}\"\n");
    (auth, cloud)
}
```

Then in `render_config_toml`, replace the `auth_section` let-binding and the `[cloud]` line in the final `format!` with variant dispatch:

```rust
    let (auth_section, cloud_section) = match &answers.auth_choice {
        AuthChoice::Hosted => provider_and_cloud_sections(
            HOSTED_API_URL,
            HOSTED_AUTH_DOMAIN,
            HOSTED_CLIENT_ID,
            HOSTED_AUDIENCE,
        ),
        AuthChoice::SelfHosted(sh) => provider_and_cloud_sections(
            &sh.instance_url,
            &sh.auth_domain,
            &sh.client_id,
            &sh.audience,
        ),
        AuthChoice::None => (
            "[auth]\nprovider = \"none\"\n".to_string(),
            String::new(),
        ),
    };
```

And update the final `format!` template: remove the hardcoded `[cloud]\napi_url = "https://temperkb.io"` block and interpolate `{cloud_section}` in its place:

```rust
    format!(
        r#"[vault]
path = {vault_path_toml}

[sync.subscriptions]
contexts = [{ctx_list}]

[skill]
output = "~/.claude/skills/temper"

{auth_section}
{cloud_section}
# [cli] — output-presentation defaults (optional; omit for agent-first auto behavior).
# Precedence for each knob: CLI flag > env var > this config > tty-aware default.
#   format: "json" | "toon"            env: TEMPER_FORMAT  (default: toon on a TTY, json otherwise)
#   color:  "auto" | "always" | "never"  env: TEMPER_COLOR  (NO_COLOR honored; default: auto)
# [cli]
# format = "json"
# color = "auto"
"#
    )
```

Note: `client_id`/`audience`/`auth_domain`/`instance_url` are interpolated raw. They come from Auth0 (alphanumeric domain, opaque client_id, URL audience) and the CLI prompt/flags; they contain no TOML metacharacters in practice. The round-trip parse tests in Step 1 guard the output. (The vault path keeps its existing `toml::Value::String` escaping because it can be an arbitrary filesystem path.)

- [ ] **Step 6: Update `gather_answers` for the three-way prompt**

Replace the auth `Select` block (≈lines 198–213) with a three-item select plus conditional self-host prompts:

```rust
    let items = [
        "hosted (temperkb.io cloud sync)",
        "self-hosted (your own instance + Auth0 tenant)",
        "none (local-only, no sync)",
    ];
    let idx = Select::with_theme(&theme)
        .with_prompt("Instance")
        .default(0)
        .items(items)
        .interact()
        .map_err(prompt_err)?;

    let auth_choice = match idx {
        0 => AuthChoice::Hosted,
        1 => {
            let instance_url: String = Input::with_theme(&theme)
                .with_prompt("Instance base URL (e.g. https://temper.acme.com)")
                .interact_text()
                .map_err(prompt_err)?;
            let auth_domain: String = Input::with_theme(&theme)
                .with_prompt("Auth0 tenant domain (e.g. acme.us.auth0.com)")
                .interact_text()
                .map_err(prompt_err)?;
            let client_id: String = Input::with_theme(&theme)
                .with_prompt("Auth0 CLI application client_id")
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
            })
        }
        _ => AuthChoice::None,
    };
```

- [ ] **Step 7: Thread self-host args through `run` / `run_non_interactive`**

Change `run` (≈line 122) and `run_non_interactive` (≈line 148) signatures to accept an `Option<SelfHostConfig>` (populated from flags in Task 4):

```rust
pub fn run(
    path: &Path,
    no_interactive: bool,
    register_global: bool,
    format: OutputFormat,
    self_host: Option<SelfHostConfig>,
) -> Result<()> {
    if no_interactive {
        return run_non_interactive(path, register_global, format, self_host);
    }
    let initial_vault = resolve_initial_vault(path);
    let answers = gather_answers(&initial_vault)?;
    print_summary(&answers, register_global);
    let proceed = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Proceed?")
        .default(true)
        .interact()
        .map_err(prompt_err)?;
    if !proceed {
        output::warning("Init cancelled");
        return Ok(());
    }
    apply_answers(&answers, register_global, None)
}
```

```rust
pub fn run_non_interactive(
    path: &Path,
    register_global: bool,
    format: OutputFormat,
    self_host: Option<SelfHostConfig>,
) -> Result<()> {
    let auth_choice = match self_host {
        Some(sh) => AuthChoice::SelfHosted(sh),
        None => AuthChoice::None,
    };
    let answers = WizardAnswers {
        vault_path: resolve_initial_vault(path),
        extra_contexts: Vec::new(),
        auth_choice,
    };
    apply_answers(&answers, register_global, None)?;

    let mut contexts = vec!["default".to_string()];
    contexts.extend(answers.extra_contexts.iter().cloned());
    let auth = match &answers.auth_choice {
        AuthChoice::Hosted => "auth0".to_string(),
        AuthChoice::SelfHosted(_) => "auth0 (self-hosted)".to_string(),
        AuthChoice::None => "none".to_string(),
    };
    let summary = InitSummary {
        vault_path: answers.vault_path.clone(),
        contexts,
        auth,
    };
    let rendered = render(&summary, format)?;
    println!("{rendered}");
    Ok(())
}
```

Update the existing `no_interactive_defaults_and_applies` test to pass `None` for the new arg:

```rust
        run_non_interactive(&vault, false, OutputFormat::Json, None)
            .expect("non-interactive run should succeed");
```

- [ ] **Step 8: Add a non-interactive self-host test**

```rust
    #[test]
    fn non_interactive_self_host_writes_derived_config() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("v");
        let sh = SelfHostConfig {
            instance_url: "https://temper.acme.com".into(),
            auth_domain: "acme.us.auth0.com".into(),
            client_id: "AcMeClientId123".into(),
            audience: "https://temper.acme.com/api".into(),
        };
        // register_global=false: we only assert the emitter path via apply,
        // matching the existing no-register tests.
        run_non_interactive(&vault, false, OutputFormat::Json, Some(sh))
            .expect("self-host non-interactive run should succeed");
        assert!(vault.join(".temper").is_dir());
    }
```

- [ ] **Step 9: Run the init tests**

Run: `cargo nextest run -p temper-cli init`
Expected: PASS (all hosted/self-hosted/none emitter tests + non-interactive).

- [ ] **Step 10: Commit**

```bash
git add crates/temper-cli/src/commands/init.rs
git commit -m "feat(init): three-way instance choice — hosted preset / self-hosted / none

The temperkb constants now live solely as the init hosted preset. Self-hosted
derives authorize/token/callback URLs from an instance URL + Auth0 inputs."
```

---

## Task 4: Wire self-host flags into the `init` CLI command

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (≈lines 60–66, the `Init` variant)
- Modify: `crates/temper-cli/src/main.rs` (≈lines 88–96, the `Commands::Init` arm)

- [ ] **Step 1: Add the flags to the `Init` clap variant**

Replace the `Init` variant in `cli.rs`:

```rust
    /// Initialize a new vault
    Init {
        /// Path for the new vault (default: current directory)
        path: Option<String>,
        /// Skip interactive prompts
        #[arg(long)]
        no_interactive: bool,
        /// Self-host: instance base URL (e.g. https://temper.acme.com)
        #[arg(long, requires_all = ["auth_domain", "auth_client_id", "auth_audience"])]
        instance_url: Option<String>,
        /// Self-host: Auth0 tenant domain (e.g. acme.us.auth0.com)
        #[arg(long)]
        auth_domain: Option<String>,
        /// Self-host: Auth0 CLI application client_id
        #[arg(long)]
        auth_client_id: Option<String>,
        /// Self-host: API audience (e.g. https://temper.acme.com/api)
        #[arg(long)]
        auth_audience: Option<String>,
    },
```

(`requires_all` on `instance_url` makes clap enforce that the four self-host flags are supplied together — passing one requires all.)

- [ ] **Step 2: Build `SelfHostConfig` from the flags in `main.rs`**

Replace the `Commands::Init { path, no_interactive }` arm:

```rust
        Commands::Init {
            path,
            no_interactive,
            instance_url,
            auth_domain,
            auth_client_id,
            auth_audience,
        } => {
            let vault_path = path
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
            // clap's `requires_all` guarantees these are all-or-nothing.
            let self_host = match (instance_url, auth_domain, auth_client_id, auth_audience) {
                (Some(instance_url), Some(auth_domain), Some(client_id), Some(audience)) => {
                    Some(temper_cli::commands::init::SelfHostConfig {
                        instance_url: instance_url.trim_end_matches('/').to_string(),
                        auth_domain,
                        client_id,
                        audience,
                    })
                }
                _ => None,
            };
            temper_cli::commands::init::run(
                &vault_path,
                no_interactive,
                true,
                output_format,
                self_host,
            )
        }
```

Confirm `SelfHostConfig` is exported (it is `pub` in `init.rs`, and `commands::init` is a public module).

- [ ] **Step 3: Build and smoke-test the CLI help**

Run: `cargo build -p temper-cli && ./target/debug/temper init --help`
Expected: PASS; help lists `--instance-url`, `--auth-domain`, `--auth-client-id`, `--auth-audience`, `--no-interactive`.

- [ ] **Step 4: Verify the requires_all guard**

Run: `./target/debug/temper init --no-interactive --instance-url https://x.example.com /tmp/throwaway-vault; echo "exit=$?"`
Expected: clap error — "the following required arguments were not provided: --auth-domain …"; non-zero exit.

- [ ] **Step 5: Workspace check + commit**

Run: `cargo make check`
Expected: PASS.

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "feat(init): --instance-url/--auth-* flags for headless self-host provisioning"
```

---

## Task 5: Extract the CLI-callback relay into testable temper-cloud logic

**Files:**
- Create: `packages/temper-cloud/src/cli-callback.ts`
- Create: `packages/temper-cloud/tests/cli-callback.test.ts`
- Modify: `api/auth/cli-callback.ts` (becomes a thin wrapper)

Per the codebase pattern (api/ = thin entry points, business logic in `temper-cloud/src/`), move the relay logic into `src/` so it's covered by the existing vitest (`tests/**/*.test.ts`). The redirect target is derived from the `state` port, not the host — so parsing becomes host-neutral, removing the hardcoded `temperkb.io` base.

- [ ] **Step 1: Write the failing test**

Create `packages/temper-cloud/tests/cli-callback.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { buildCliCallbackResponse } from "../src/cli-callback.js";

describe("buildCliCallbackResponse", () => {
  it("redirects the code to the localhost port from state, regardless of host", () => {
    const res = buildCliCallbackResponse(
      "/api/auth/cli-callback?code=abc123&state=51789",
      "temper.acme.com",
    );
    expect(res.status).toBe(302);
    expect(res.headers.get("location")).toBe(
      "http://localhost:51789?code=abc123",
    );
  });

  it("works when req.url is absolute (host ignored for the target)", () => {
    const res = buildCliCallbackResponse(
      "https://temper.acme.com/api/auth/cli-callback?code=xy%2Fz&state=40000",
      null,
    );
    expect(res.headers.get("location")).toBe(
      "http://localhost:40000?code=xy%2Fz",
    );
  });

  it("returns 400 when code or state is missing", () => {
    const res = buildCliCallbackResponse("/api/auth/cli-callback?code=abc", "h");
    expect(res.status).toBe(400);
  });

  it("returns 400 for an out-of-range port", () => {
    const res = buildCliCallbackResponse(
      "/api/auth/cli-callback?code=abc&state=80",
      "h",
    );
    expect(res.status).toBe(400);
  });

  it("surfaces an Auth0 error param as 400", () => {
    const res = buildCliCallbackResponse(
      "/api/auth/cli-callback?error=access_denied&error_description=nope",
      "h",
    );
    expect(res.status).toBe(400);
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/temper-cloud && bun run test cli-callback`
Expected: FAIL — `../src/cli-callback.js` does not exist.

- [ ] **Step 3: Create the pure relay module**

Create `packages/temper-cloud/src/cli-callback.ts`:

```ts
/**
 * CLI auth callback relay logic. Auth0 redirects here with ?code=…&state={port};
 * we redirect the code to the CLI's localhost server on that port. The redirect
 * target depends only on `state`, so this is host-neutral — the `host` argument
 * is used solely as a parse base when `rawUrl` is relative.
 */

function plain(body: string, status: number): Response {
  return new Response(body, {
    status,
    headers: { "Content-Type": "text/plain" },
  });
}

export function buildCliCallbackResponse(
  rawUrl: string,
  host: string | null,
): Response {
  const base = `https://${host ?? "localhost"}`;
  const url = new URL(rawUrl, base);

  const error = url.searchParams.get("error");
  if (error) {
    const description = url.searchParams.get("error_description") ?? "unknown error";
    return plain(`Authentication failed: ${error} — ${description}`, 400);
  }

  const code = url.searchParams.get("code");
  const state = url.searchParams.get("state");
  if (!code || !state) {
    return plain("Missing code or state parameter", 400);
  }

  const port = Number.parseInt(state, 10);
  if (Number.isNaN(port) || port < 1024 || port > 65535) {
    return plain("Invalid port in state parameter", 400);
  }

  const target = `http://localhost:${port}?code=${encodeURIComponent(code)}`;
  return Response.redirect(target, 302);
}
```

- [ ] **Step 4: Slim the Vercel entry point to delegate**

Replace the body of `api/auth/cli-callback.ts`:

```ts
/**
 * CLI auth callback relay (Vercel entry point). Thin wrapper — relay logic and
 * tests live in `packages/temper-cloud/src/cli-callback.ts`.
 */

import { buildCliCallbackResponse } from "../../packages/temper-cloud/src/cli-callback.js";

export function GET(req: Request): Response {
  return buildCliCallbackResponse(req.url, req.headers.get("host"));
}
```

Note: confirm the relative import path resolves under the Vercel build (the repo root `api/` to `packages/temper-cloud/src/`). If the Vercel bundler cannot follow the cross-package path, fall back to inlining the same `buildCliCallbackResponse` body directly in this file (it is dependency-free) — but try the import first to keep one source of truth.

- [ ] **Step 5: Run the test + typecheck**

Run: `cd packages/temper-cloud && bun run test cli-callback && bun run typecheck`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add packages/temper-cloud/src/cli-callback.ts packages/temper-cloud/tests/cli-callback.test.ts api/auth/cli-callback.ts
git commit -m "refactor(cli-callback): host-neutral relay extracted to temper-cloud/src + tests"
```

---

## Task 6: Fix the stale env templates

**Files:**
- Modify: `crates/temper-api/.env.template`
- Modify: `.env.template`

- [ ] **Step 1: Replace the stale Neon-Auth API template with the Auth0 contract**

Overwrite `crates/temper-api/.env.template`:

```sh
# temper-api environment contract (host-neutral; fill in for your instance).
# For temperkb.io these are set in the Vercel project; for a self-hosted
# deployment see docs/guides/self-hosting.md.

# --- Database (Neon) ---
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_test
# Direct (unpooled) URL used for migrations:
DATABASE_URL_UNPOOLED=postgresql://temper:temper@localhost:5437/temper_test

# --- Auth0 (API resource server) ---
# Issuer = https://<your-auth0-domain>/  (trailing slash required)
AUTH_ISSUER=https://<your-tenant>.auth0.com/
JWKS_URL=https://<your-tenant>.auth0.com/.well-known/jwks.json
# Audience = the Auth0 API identifier for this instance:
AUTH_AUDIENCE=https://<your-instance>/api
AUTH_PROVIDER_NAME=auth0

# --- HTTP ---
CORS_ORIGINS=http://localhost:3000,http://localhost:5173
PORT=3000
```

- [ ] **Step 2: Reorganize the root `.env.template` to separate the self-host contract from UI-only vars**

Edit `.env.template`: keep the existing API/MCP-relevant vars, but add a clear banner at the top and move the UI/session vars under an explicit out-of-scope heading. Replace the `# Auth0 (temper-web Regular Web Application)` and `# Session cookie encryption` sections' leading comments with:

```sh
# ---------------------------------------------------------------------------
# UI-only (temper-ui web app) — OUT OF SCOPE for self-host (see
# docs/guides/self-hosting.md). Required only if you also deploy the web UI.
# ---------------------------------------------------------------------------
```

Leave the variable lines themselves intact (they document the UI flow), only re-label the section so an operator reading top-to-bottom knows the API/MCP contract ends and UI-only begins. Do not change the API/MCP var block.

- [ ] **Step 3: Verify nothing references removed vars**

Run: `grep -rn "neonauth" crates/ api/ packages/ || echo "no stale neonauth refs"`
Expected: `no stale neonauth refs` (the api/.env.template no longer mentions it).

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/.env.template .env.template
git commit -m "docs(env): fix stale Neon-Auth template → Auth0; label UI-only vars out-of-scope"
```

---

## Task 7: Author the self-hosting runbook

**Files:**
- Create: `docs/guides/self-hosting.md`

Write the operator runbook. Use the live-grounded values from the spec's "Introspection findings": **Postgres 17** on Neon (not 18 — that's local Docker), extensions `vector` + `pg_uuidv7`, the confirmed Production env contract, and the Auth0 shape (1 API + 1 CLI native app + 1 MCP native app). Do **not** include real secret values, the temperkb client_ids, or the stale `NEON_AUTH_*`/`VITE_NEON_AUTH_*` vars.

- [ ] **Step 1: Write the runbook**

Create `docs/guides/self-hosting.md` with these sections (full prose, not an outline):

1. **Overview & topology** — one Vercel project serves `temper-api` (Axum via `temper-cloud`, catch-all route) and `temper-mcp` (`/mcp`, `/.well-known/*`, `/oauth/*` routes); a Neon Postgres database; an Auth0 tenant. State the scope boundary: the `temper-ui` web app is **not covered** here.
2. **Provision Neon** — create a project; **Postgres 17** (Neon's GA; the repo's local Docker is 18); enable extensions `vector` (pgvector) and `pg_uuidv7`; capture both the pooled `DATABASE_URL` and the direct `DATABASE_URL_UNPOOLED`; run migrations as a deploy step (`DATABASE_URL_UNPOOLED` + `sqlx migrate run`), never at runtime. Mention the Neon×Vercel integration auto-provisions per-preview branch URLs.
3. **Provision Auth0** — value contract (not click-by-click). One **API / resource server** → its identifier is your `AUTH_AUDIENCE`/`MCP_AUDIENCE` (e.g. `https://<instance>/api`). One **native application for the CLI** → Authorization Code + PKCE, allowed callback `https://<instance>/api/auth/cli-callback`; its `client_id` is what `temper init --auth-client-id` takes. One **native application for MCP clients** → its `client_id` is `MCP_CLIENT_ID`; callbacks for your MCP clients (e.g. `https://claude.ai/api/mcp/auth_callback`, `http://localhost`). Map each value to its env var: `AUTH_ISSUER` (=`https://<domain>/`), `JWKS_URL` (=`https://<domain>/.well-known/jwks.json`), `AUTH_AUDIENCE`, `AUTH_PROVIDER_NAME=auth0`, `MCP_AUDIENCE`, `MCP_CLIENT_ID`, `MCP_BASE_URL`. Note that these can be read from a live tenant with the `auth0` CLI (`auth0 apis list`, `auth0 apps list`) or the Auth0 MCP server.
4. **Deploy to Vercel** — the env-var contract table. Columns: Variable, Surface (api/mcp/build), Required, Notes. Rows (the live-confirmed Production set): `DATABASE_URL`, `DATABASE_URL_UNPOOLED`, `AUTH_ISSUER`, `JWKS_URL`, `AUTH_AUDIENCE`, `AUTH_PROVIDER_NAME`, `MCP_BASE_URL`, `MCP_AUDIENCE`, `MCP_CLIENT_ID`, `API_BASE_URL`, `BLOB_READ_WRITE_TOKEN` (Vercel Blob, upload/extract pipeline), `ENABLE_SWAGGER`, `SQLX_OFFLINE=true` (build), `CORS_ORIGINS`. Flag that `CORS_ORIGINS` must be set for any cross-origin client (the API denies cross-origin when unset; `*` for permissive dev). State the `vercel.json` routing contract (`/mcp` + discovery → `api/mcp`, catch-all → `api/axum`).
5. **Configure the CLI** — interactive: `temper init` → choose **self-hosted** → enter instance URL, Auth0 domain, client_id, audience. Headless: `temper init --no-interactive --instance-url https://<instance> --auth-domain <tenant>.auth0.com --auth-client-id <id> --auth-audience https://<instance>/api`. Show the resulting `config.toml` `[auth]`/`[[auth.providers]]`/`[cloud]` blocks. Document `TEMPER_API_URL`, `TEMPER_PROVIDER_ENV`, `TEMPER_TOKEN` for CI/agents.
6. **Connect MCP clients** — point the client at `https://<instance>/mcp`; OAuth discovery served at `/.well-known/*`; the MCP native app registration.
7. **Verify** — `curl https://<instance>/api/health`; `temper login`; create + read back a resource end-to-end.
8. **Not covered / deferred** — the `temper-ui` web app and its Auth0 Regular-Web-App flow; multi-region/HA Neon; alternative messaging. Single-instance self-host is the supported target today.

- [ ] **Step 2: Lint the doc**

Run: `npx markdownlint-cli2 docs/guides/self-hosting.md` (or `cargo make lint` from `tasker-book` conventions if a markdownlint config governs this repo). Fix any line-length/heading issues.
Expected: clean (or only pre-existing-config-driven warnings).

- [ ] **Step 3: Commit**

```bash
git add docs/guides/self-hosting.md
git commit -m "docs(guide): self-hosting runbook for enterprise non-temperkb.io deployments"
```

---

## Task 8: Cross-link the runbook

**Files:**
- Modify: `docs/guides/install.md`

- [ ] **Step 1: Add a pointer from install.md**

At the end of `docs/guides/install.md`, add:

```markdown
## Running your own instance

The steps above install the `temper` CLI and (by default) leave it
unconfigured. To point it at the hosted service, run `temper init` and choose
the hosted option. To stand up your **own** Temper instance on Vercel + Neon +
Auth0 (API + MCP + CLI), see [Self-Hosting](./self-hosting.md).
```

- [ ] **Step 2: Commit**

```bash
git add docs/guides/install.md
git commit -m "docs(install): link the self-hosting runbook"
```

---

## Final Verification

- [ ] **Step 1: Full Rust check + unit tests**

Run: `cargo make check && cargo make test`
Expected: PASS.

- [ ] **Step 2: TypeScript check**

Run: `cd packages/temper-cloud && bun run test && bun run check && bun run typecheck`
Expected: PASS.

- [ ] **Step 3: End-to-end manual smoke of `temper init` self-host (non-interactive)**

Run:
```bash
TMP=$(mktemp -d)
HOME="$TMP" ./target/debug/temper init --no-interactive \
  --instance-url https://temper.acme.com \
  --auth-domain acme.us.auth0.com \
  --auth-client-id AcMeClientId123 \
  --auth-audience https://temper.acme.com/api \
  "$TMP/vault"
cat "$TMP/.config/temper/config.toml"
```
Expected: config.toml has `[cloud] api_url = "https://temper.acme.com"`, an `[[auth.providers]]` block with `authorize_url = "https://acme.us.auth0.com/authorize"` and `callback_url = "https://temper.acme.com/api/auth/cli-callback"`, and parses (no temperkb references).

- [ ] **Step 4: Confirm no temperkb defaults remain in the binary's config layer**

Run: `grep -rn "temperkb" crates/temper-core/src crates/temper-client/src && echo "REVIEW: temperkb refs above" || echo "clean: no temperkb in core/client src"`
Expected: `clean: no temperkb in core/client src` (the only temperkb constants now live in `temper-cli/src/commands/init.rs` as the hosted preset).

---

## Self-Review Notes

- **Spec coverage:** §1 strip defaults → Task 1; §2 init instance step (incl. `--no-interactive` flags, resolved decision) → Tasks 3+4; §3 cli-callback → Task 5 (downgraded to a host-neutral refactor — see note below); §4 env templates → Task 6; §5 runbook → Task 7 + 8. Introspection findings (PG17, extensions, env contract, Auth0 shape) → Task 7 content.
- **§3 scope note:** investigation showed the hardcoded `temperkb.io` base in `cli-callback.ts` is only a parse base for relative URLs; the redirect target derives from `state`, so it was never a functional blocker. Task 5 still removes the temperkb reference and, more valuably, relocates the logic to the tested `temper-cloud/src` layer per the codebase's thin-entry-point convention.
- **Type consistency:** `AuthChoice` (Hosted/SelfHosted/None) and `SelfHostConfig { instance_url, auth_domain, client_id, audience }` are used identically across init.rs, main.rs, and tests. `buildCliCallbackResponse(rawUrl, host)` signature matches between src, test, and the api wrapper.
- **No baked-in default — validation:** `validate_optional_api_url` allows `""` (unconfigured) and rejects non-empty non-URLs, preserving the `validate_rejects_malformed_api_url` test.
