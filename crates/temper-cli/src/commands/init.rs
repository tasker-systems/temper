//! `temper init` — guided config + cloud-context setup.
//!
//! The wizard is split into two parts so that tests can drive the apply
//! step without touching dialoguer: `gather_answers` (interactive) and
//! `apply_answers` (pure config work + optional cloud ensure).

use std::path::{Path, PathBuf};

use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use serde::Serialize;

use crate::config::global_config_path;
use crate::error::{Result, TemperError};
use crate::format::{render, OutputFormat};
use crate::output;

/// Structured summary emitted in non-interactive mode with --format.
#[derive(Debug, Serialize)]
pub(crate) struct InitSummary {
    pub vault_path: String,
    pub contexts: Vec<String>,
    pub auth: String,
}

/// Hosted-instance preset values (the only place the temperkb.io constants
/// live after the binary stopped baking them into config defaults).
const HOSTED_API_URL: &str = "https://temperkb.io";
const HOSTED_AUTH_DOMAIN: &str = "temperkb.us.auth0.com";
const HOSTED_CLIENT_ID: &str = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF";
const HOSTED_AUDIENCE: &str = "https://temperkb.io/api";

/// Which IdP's OAuth endpoint shapes `temper init` should emit. The written
/// provider *label* stays "auth0" regardless — this only selects URL templating.
#[derive(Debug, Clone)]
pub enum Idp {
    /// Auth0 tenant: `https://{domain}/authorize`, `/oauth/token`.
    Auth0,
    /// Okta custom authorization server: `https://{domain}/oauth2/{id}/v1/*`.
    Okta { auth_server_id: String },
}

/// Per-instance OAuth inputs for a self-hosted deployment.
#[derive(Debug, Clone)]
pub struct SelfHostConfig {
    /// Instance base URL, e.g. `https://temper.acme.com`.
    pub instance_url: String,
    /// OAuth provider domain — e.g. `acme.us.auth0.com` (Auth0) or `acme.okta.com` (Okta).
    pub auth_domain: String,
    /// Auth0 native-app client_id for the CLI.
    pub client_id: String,
    /// API audience / resource identifier, e.g. `https://temper.acme.com/api`.
    pub audience: String,
    /// Identity-provider URL shape to emit (Auth0 vs Okta authz server).
    pub idp: Idp,
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

/// Collected wizard answers — produced by `gather_answers` (interactive) or
/// `default_answers` (`--no-interactive`).
#[derive(Debug, Clone)]
pub struct WizardAnswers {
    pub vault_path: String,
    pub extra_contexts: Vec<String>,
    pub auth_choice: AuthChoice,
}

/// Abstraction over server-side context ensuring.
///
/// Introduced so tests can inject a mock without building a real
/// `TemperClient` (and without a running server or `--features test-db`).
///
/// The production implementation wraps `TemperClient::contexts()` calls.
/// The test implementation records calls and returns `Ok(())`.
pub trait ContextEnsurer {
    /// Ensure the named context exists server-side. Implementations must
    /// treat "already exists" (409 Conflict) as success.
    fn ensure_context(&self, name: &str) -> Result<()>;
}

/// Production `ContextEnsurer` built from an already-initialized
/// `TemperClient` + a tokio runtime to drive async calls.
pub struct ClientContextEnsurer<'a> {
    client: &'a temper_client::TemperClient,
    rt: &'a tokio::runtime::Runtime,
    existing_names: Vec<String>,
}

impl<'a> ClientContextEnsurer<'a> {
    pub fn new(
        client: &'a temper_client::TemperClient,
        rt: &'a tokio::runtime::Runtime,
        existing_names: Vec<String>,
    ) -> Self {
        Self {
            client,
            rt,
            existing_names,
        }
    }
}

impl ContextEnsurer for ClientContextEnsurer<'_> {
    fn ensure_context(&self, name: &str) -> Result<()> {
        if self.existing_names.iter().any(|n| n == name) {
            return Ok(());
        }
        let result = self.rt.block_on(self.client.contexts().create(name));
        match result {
            Ok(_) => Ok(()),
            Err(temper_client::error::ClientError::Conflict { .. }) => {
                // 409: already exists on the server — idempotent success.
                Ok(())
            }
            Err(e) => Err(TemperError::Api(format!("create context '{name}': {e}"))),
        }
    }
}

fn default_vault_path() -> String {
    dirs::home_dir()
        .map(|h| {
            h.join("Documents/temper-vault")
                .to_string_lossy()
                .to_string()
        })
        .unwrap_or_else(|| "./temper-vault".to_string())
}

/// Resolve an initial vault path from a CLI argument: an empty argument
/// falls back to `default_vault_path()`. Shared between interactive and
/// non-interactive entry points so both handle the empty case identically.
fn resolve_initial_vault(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        default_vault_path()
    } else {
        path.to_string_lossy().to_string()
    }
}

/// Convert a dialoguer prompt failure into a `TemperError`. Prompt errors
/// are a configuration-setup problem, not a vault-state problem, so we map
/// to `Config` rather than `Vault`.
fn prompt_err(e: dialoguer::Error) -> TemperError {
    TemperError::Config(format!("prompt error: {e}"))
}

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
                TemperError::Config("--auth-server-id is required when --idp okta".to_string())
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

/// CLI entry point dispatched from `main.rs`.
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

/// Non-interactive path — uses all defaults, emitting a structured summary in
/// the resolved output format.
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

    // Non-interactive mode always emits a structured summary (the TTY wizard
    // uses styled output instead). The format is resolved globally upstream.
    let mut contexts = vec!["default".to_string()];
    contexts.extend(answers.extra_contexts.iter().cloned());
    let auth = match &answers.auth_choice {
        AuthChoice::Hosted => "auth0".to_string(),
        AuthChoice::SelfHosted(sh) => match sh.idp {
            Idp::Auth0 => "auth0 (self-hosted)".to_string(),
            Idp::Okta { .. } => "okta (self-hosted)".to_string(),
        },
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

/// Run the interactive prompts and return collected answers.
fn gather_answers(initial_vault: &str) -> Result<WizardAnswers> {
    let theme = ColorfulTheme::default();

    let vault_path: String = Input::with_theme(&theme)
        .with_prompt("Where should your vault live?")
        .default(initial_vault.to_string())
        .interact_text()
        .map_err(prompt_err)?;

    let contexts_raw: String = Input::with_theme(&theme)
        .with_prompt("Create any contexts now? (comma-separated, or Enter for just 'default')")
        .default(String::new())
        .allow_empty(true)
        .interact_text()
        .map_err(prompt_err)?;

    let extra_contexts: Vec<String> = contexts_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "default")
        .collect();

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
                    .validate_with(|input: &String| -> std::result::Result<(), &str> {
                        if input.trim().is_empty() {
                            Err("Okta authorization server ID cannot be empty")
                        } else {
                            Ok(())
                        }
                    })
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
        _ => AuthChoice::None,
    };

    Ok(WizardAnswers {
        vault_path,
        extra_contexts,
        auth_choice,
    })
}

fn print_summary(answers: &WizardAnswers, register_global: bool) {
    output::blank();
    output::header("Ready to initialize:");
    output::label("Vault", &answers.vault_path);
    let mut ctxs = vec!["default".to_string()];
    ctxs.extend(answers.extra_contexts.iter().cloned());
    output::label("Contexts", ctxs.join(", "));
    let auth_label = match &answers.auth_choice {
        AuthChoice::Hosted => "auth0",
        AuthChoice::SelfHosted(sh) => match &sh.idp {
            Idp::Auth0 => "auth0 (self-hosted)",
            Idp::Okta { .. } => "okta (self-hosted)",
        },
        AuthChoice::None => "none",
    };
    output::label("Auth", auth_label);
    if register_global {
        output::label("Config", global_config_path().display().to_string());
    }
    output::blank();
}

/// Write config (if `register_global`) and ensure server-side contexts exist.
///
/// The `ensurer` parameter accepts an optional mock for tests. When `None`,
/// the production path builds a `TemperClient` from the just-written config.
/// When `register_global == false`, the cloud-ensure step is skipped entirely
/// (no config to load from, and tests don't need it).
pub fn apply_answers(
    answers: &WizardAnswers,
    register_global: bool,
    ensurer: Option<&dyn ContextEnsurer>,
) -> Result<()> {
    let vault = PathBuf::from(&answers.vault_path);

    // Detect an already-initialized config directory and warn.
    // We use the config file itself as the marker rather than a vault sidecar.
    let config_path = global_config_path();
    if config_path.exists() {
        output::warning(format!(
            "Temper config already initialized at {}; re-running init is idempotent. \
             To change settings, run `temper config edit`.",
            config_path.display()
        ));
    }

    // Always create the vault root (local projection cache root).
    std::fs::create_dir_all(&vault)?;

    // Create the .temper/ state dir — the projection cursor sidecar
    // (`projection/<ctx>.json`) lives here. projection.rs does its own
    // create_dir_all lazily, but having .temper/ present after init is
    // harmless and expected by convention.
    let state_dir = vault.join(".temper");
    std::fs::create_dir_all(&state_dir)?;

    if register_global {
        if !config_path.exists() {
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let toml = render_config_toml(answers);
            std::fs::write(&config_path, toml)?;
            output::dim(format!("Wrote global config to {}", config_path.display()));
        } else {
            output::dim("Global config already exists, skipping");
        }

        // Cloud-ensure step: only when an instance is configured.
        if !matches!(answers.auth_choice, AuthChoice::None) {
            ensure_server_contexts(answers, ensurer)?;
        }
    }

    output::success(
        "Temper initialized successfully. Run `temper pull default` to materialize a local projection.",
    );
    Ok(())
}

/// Ensure the default context and any extra contexts exist server-side.
///
/// Accepts an optional injected `ContextEnsurer` for tests. When `None`,
/// builds a production client from the just-written config.
fn ensure_server_contexts(
    answers: &WizardAnswers,
    ensurer: Option<&dyn ContextEnsurer>,
) -> Result<()> {
    let all_contexts: Vec<String> = std::iter::once("default".to_string())
        .chain(answers.extra_contexts.iter().cloned())
        .collect();

    if let Some(e) = ensurer {
        // Test / injected path.
        for ctx in &all_contexts {
            e.ensure_context(ctx)?;
        }
        return Ok(());
    }

    // Production path: build client from the config we just wrote.
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;

    let (_config, _store, client) = crate::actions::runtime::build_config_store_and_client()?;

    // List existing contexts to avoid redundant creates.
    let existing = match rt.block_on(client.contexts().list()) {
        Ok(rows) => rows.into_iter().map(|r| r.name).collect::<Vec<_>>(),
        Err(temper_client::error::ClientError::NotAuthenticated)
        | Err(temper_client::error::ClientError::TokenExpired) => {
            output::warning(
                "Auth required — run `temper auth login` to authenticate, \
                 then `temper init` again to ensure server-side contexts.",
            );
            return Ok(());
        }
        Err(e) => {
            return Err(TemperError::Api(format!("list contexts: {e}")));
        }
    };

    let prod_ensurer = ClientContextEnsurer::new(&client, &rt, existing);
    for ctx in &all_contexts {
        prod_ensurer.ensure_context(ctx)?;
    }

    Ok(())
}

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

/// Build the `[auth]` + `[[auth.providers]]` block and the `[cloud]` api_url
/// line for a configured instance (hosted or self-hosted).
fn provider_and_cloud_sections(
    api_url: &str,
    auth_domain: &str,
    client_id: &str,
    audience: &str,
    idp: &Idp,
) -> (String, String) {
    // Route every interpolated value through `toml::Value::String` (the same
    // escaping the vault path gets) so any character requiring escaping
    // round-trips, rather than relying on these always being metacharacter-free.
    let tv = |s: String| toml::Value::String(s).to_string();
    let (authorize, token) = provider_urls(idp, auth_domain);
    let authorize_url = tv(authorize);
    let token_url = tv(token);
    let callback_url = tv(format!("{api_url}/api/auth/cli-callback"));
    let client_id_toml = tv(client_id.to_string());
    let audience_toml = tv(audience.to_string());

    let auth = format!(
        r#"[auth]
provider = "auth0"

[[auth.providers]]
name = "auth0"
authorize_url = {authorize_url}
token_url = {token_url}
client_id = {client_id_toml}
audience = {audience_toml}
callback_url = {callback_url}
scopes = ["openid", "profile", "email", "offline_access"]
"#
    );
    let cloud = format!("[cloud]\napi_url = {}\n", tv(api_url.to_string()));
    (auth, cloud)
}

/// Produce the TOML body for `config.toml` from the collected answers.
///
/// Both the vault path and each context name are routed through
/// `toml::Value::String` so that characters requiring escaping (backslashes,
/// double quotes, control characters) round-trip through `TemperConfig`
/// parsing — including Windows-style paths.
pub fn render_config_toml(answers: &WizardAnswers) -> String {
    let mut ctxs = vec!["default".to_string()];
    ctxs.extend(answers.extra_contexts.iter().cloned());
    let ctx_list = ctxs
        .iter()
        .map(|c| toml::Value::String(c.clone()).to_string())
        .collect::<Vec<_>>()
        .join(", ");

    let (auth_section, cloud_section) = match &answers.auth_choice {
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
        AuthChoice::None => ("[auth]\nprovider = \"none\"\n".to_string(), String::new()),
    };

    // toml::Value::String already includes the surrounding quotes, so the
    // `path =` line below does NOT wrap `{vault_path_toml}` in its own quotes.
    let vault_path_toml = toml::Value::String(answers.vault_path.clone()).to_string();

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use temper_core::types::config::TemperConfig;
    use validator::Validate;

    /// Mock `ContextEnsurer` that records which context names were ensured.
    struct MockEnsurer {
        ensured: RefCell<Vec<String>>,
    }

    impl MockEnsurer {
        fn new() -> Self {
            Self {
                ensured: RefCell::new(Vec::new()),
            }
        }

        fn ensured_names(&self) -> Vec<String> {
            self.ensured.borrow().clone()
        }
    }

    impl ContextEnsurer for MockEnsurer {
        fn ensure_context(&self, name: &str) -> Result<()> {
            self.ensured.borrow_mut().push(name.to_string());
            Ok(())
        }
    }

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
                idp: Idp::Auth0,
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
        assert_eq!(
            p.callback_url,
            "https://temper.acme.com/api/auth/cli-callback"
        );
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

    /// Round-trip guard: the rendered TOML must parse into a valid
    /// `TemperConfig` for every auth choice. This catches template drift
    /// that string-contains assertions would miss.
    #[test]
    fn rendered_toml_parses_and_validates_hosted() {
        let answers = WizardAnswers {
            vault_path: "/tmp/roundtrip".into(),
            extra_contexts: vec!["one".into(), "two".into()],
            auth_choice: AuthChoice::Hosted,
        };
        let rendered = render_config_toml(&answers);
        let cfg: TemperConfig =
            toml::from_str(&rendered).expect("rendered TOML should parse into TemperConfig");
        cfg.validate().expect("rendered config should validate");
    }

    #[test]
    fn rendered_toml_parses_and_validates_auth_none() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::None,
        };
        let rendered = render_config_toml(&answers);
        let cfg: TemperConfig =
            toml::from_str(&rendered).expect("rendered TOML should parse into TemperConfig");
        cfg.validate().expect("rendered config should validate");
    }

    #[test]
    fn rendered_toml_escapes_backslashes_in_vault_path() {
        // Windows-style path — backslashes MUST survive the render/parse
        // round-trip without breaking the TOML or being dropped.
        let answers = WizardAnswers {
            vault_path: r"C:\Users\alice\Documents\temper-vault".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::None,
        };
        let rendered = render_config_toml(&answers);
        let cfg: TemperConfig = toml::from_str(&rendered).expect("escaped path must parse");
        assert_eq!(cfg.vault.path, r"C:\Users\alice\Documents\temper-vault");
    }

    #[test]
    fn rendered_toml_escapes_double_quotes_in_vault_path() {
        // Pathological but valid on Unix: a path containing a double quote
        // must not break the TOML basic string it's interpolated into.
        let answers = WizardAnswers {
            vault_path: r#"/tmp/weird"name"#.into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::None,
        };
        let rendered = render_config_toml(&answers);
        let cfg: TemperConfig = toml::from_str(&rendered).expect("escaped quote must parse");
        assert_eq!(cfg.vault.path, r#"/tmp/weird"name"#);
    }

    #[test]
    fn default_answers_generate_complete_config() {
        let answers = WizardAnswers {
            vault_path: "/tmp/my-vault".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::Hosted,
        };
        let toml = render_config_toml(&answers);
        assert!(toml.contains(r#"path = "/tmp/my-vault""#));
        assert!(toml.contains("[auth]"));
        assert!(toml.contains(r#"provider = "auth0""#));
        assert!(toml.contains("[[auth.providers]]"));
        assert!(toml.contains(r#"name = "auth0""#));
        assert!(toml.contains("[cloud]"));
        assert!(toml.contains(r#"api_url = "https://temperkb.io""#));
        // The [cli] output-defaults section ships commented-out (documentation
        // only): the template must not ACTIVATE any cli setting, so a fresh
        // config keeps agent-first auto behavior. Parsing confirms it stays None.
        let cfg: TemperConfig = toml::from_str(&toml).expect("rendered config parses");
        assert!(
            cfg.cli.format.is_none() && cfg.cli.color.is_none(),
            "commented [cli] template must not activate format/color"
        );
        assert!(
            toml.contains("# [cli]"),
            "cli docs should be present (commented)"
        );
        assert!(
            !toml.contains("framework ="),
            "skill.framework should not be written"
        );
    }

    #[test]
    fn auth_none_writes_provider_none_marker() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::None,
        };
        let toml = render_config_toml(&answers);
        assert!(toml.contains(r#"provider = "none""#));
        // no auth0 provider entry when none chosen
        assert!(!toml.contains("[[auth.providers]]"));
    }

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

    #[test]
    fn auth0_writes_array_of_tables_format() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::Hosted,
        };
        let toml = render_config_toml(&answers);
        // Must use the new array-of-tables format, NOT the old dotted-map form
        assert!(toml.contains("[[auth.providers]]"));
        assert!(toml.contains(r#"name = "auth0""#));
        assert!(
            !toml.contains("[auth.providers.auth0]"),
            "must not use old dotted form"
        );
    }

    #[test]
    fn apply_answers_warns_on_existing_vault_but_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("existing");
        std::fs::create_dir_all(&vault).unwrap();
        let answers = WizardAnswers {
            vault_path: vault.to_string_lossy().to_string(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::Hosted,
        };
        // register_global=false — the config-file marker lives at
        // global_config_path() (not in the tmpdir), so we can't pre-create
        // it reliably in a unit test. We just verify apply succeeds and
        // that .temper/ is created even with no register_global.
        apply_answers(&answers, false, None).expect("should warn but succeed");
        // .temper/ state dir is created regardless of register_global.
        assert!(vault.join(".temper").is_dir());
        // No manifest or events sidecars.
        assert!(!vault.join(".temper/manifest.json").exists());
        assert!(!vault.join(".temper/events.jsonl").exists());
    }

    #[test]
    fn apply_answers_creates_vault_structure() {
        let tmp = tempfile::tempdir().unwrap();
        let vault_path = tmp.path().join("vault");
        let answers = WizardAnswers {
            vault_path: vault_path.to_string_lossy().to_string(),
            extra_contexts: vec!["writing".into()],
            auth_choice: AuthChoice::None,
        };
        apply_answers(&answers, false, None).expect("apply should succeed");
        // .temper/ state dir exists.
        assert!(vault_path.join(".temper").is_dir());
        // No manifest, no events sidecars.
        assert!(!vault_path.join(".temper/manifest.json").exists());
        assert!(!vault_path.join(".temper/events.jsonl").exists());
        // No per-context subdirectories.
        assert!(!vault_path.join("default").exists());
        assert!(!vault_path.join("writing").exists());
    }

    #[test]
    fn no_interactive_defaults_and_applies() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("v");
        run_non_interactive(&vault, false, OutputFormat::Json, None)
            .expect("non-interactive run should succeed");
        // .temper/ created.
        assert!(vault.join(".temper").is_dir());
        // No per-context subdirectory.
        assert!(!vault.join("default").exists());
    }

    #[test]
    fn extra_contexts_go_into_subscriptions() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec!["temper".into(), "writing".into()],
            auth_choice: AuthChoice::Hosted,
        };
        let toml = render_config_toml(&answers);
        assert!(toml.contains(r#"contexts = ["default", "temper", "writing"]"#));
    }

    #[test]
    fn mock_ensurer_called_for_default_and_extra_contexts() {
        let tmp = tempfile::tempdir().unwrap();
        let vault_path = tmp.path().join("vault");
        let answers = WizardAnswers {
            vault_path: vault_path.to_string_lossy().to_string(),
            extra_contexts: vec!["writing".into()],
            auth_choice: AuthChoice::Hosted,
        };
        let mock = MockEnsurer::new();
        // register_global=false means the cloud-ensure block is skipped,
        // but the ensurer is passed directly to ensure_server_contexts via
        // the injected path in apply_answers when register_global=true.
        // Test the helper directly to verify mock dispatch works.
        ensure_server_contexts(&answers, Some(&mock))
            .expect("ensure_server_contexts should succeed");
        let names = mock.ensured_names();
        assert!(names.contains(&"default".to_string()));
        assert!(names.contains(&"writing".to_string()));
    }

    #[test]
    fn render_init_summary_json_includes_vault_path() {
        let summary = InitSummary {
            vault_path: "/tmp/my-vault".to_string(),
            contexts: vec!["default".to_string(), "writing".to_string()],
            auth: "auth0".to_string(),
        };
        let out = crate::format::render(&summary, crate::format::OutputFormat::Json)
            .expect("json render");
        assert!(out.contains("\"vault_path\""), "json: {out}");
        assert!(out.contains("/tmp/my-vault"), "json: {out}");
        assert!(out.contains("\"contexts\""), "json: {out}");
        assert!(out.contains("\"auth\""), "json: {out}");
        assert!(out.contains("auth0"), "json: {out}");
    }

    #[test]
    fn non_interactive_self_host_applies_without_error() {
        // register_global=false, so no config file is written here — this only
        // exercises the self-host non-interactive apply path (vault scaffold).
        // The derived-config emission itself is covered by `self_hosted_emits_derived_urls`.
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("v");
        let sh = SelfHostConfig {
            instance_url: "https://temper.acme.com".into(),
            auth_domain: "acme.us.auth0.com".into(),
            client_id: "AcMeClientId123".into(),
            audience: "https://temper.acme.com/api".into(),
            idp: Idp::Auth0,
        };
        run_non_interactive(&vault, false, OutputFormat::Json, Some(sh))
            .expect("self-host non-interactive run should succeed");
        assert!(vault.join(".temper").is_dir());
    }

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

    #[test]
    fn flags_unknown_idp_errors() {
        assert!(self_host_from_flags(
            Some("https://x.com".into()),
            Some("d.auth0.com".into()),
            Some("cid".into()),
            Some("https://x.com/api".into()),
            Some("saml".into()),
            None,
        )
        .is_err());
    }

    #[test]
    fn provider_urls_auth0_and_okta_shapes() {
        let (authorize, token) = provider_urls(&Idp::Auth0, "acme.us.auth0.com");
        assert_eq!(authorize, "https://acme.us.auth0.com/authorize");
        assert_eq!(token, "https://acme.us.auth0.com/oauth/token");

        let (authorize, token) = provider_urls(
            &Idp::Okta {
                auth_server_id: "aus1a2b3c".into(),
            },
            "acme.okta.com",
        );
        assert_eq!(
            authorize,
            "https://acme.okta.com/oauth2/aus1a2b3c/v1/authorize"
        );
        assert_eq!(token, "https://acme.okta.com/oauth2/aus1a2b3c/v1/token");
    }
}
