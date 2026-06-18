use serde::Serialize;

use crate::config::{Config, GlobalConfig};
use crate::error::Result;
use crate::format::{render, OutputFormat};

/// A single health check result.
#[derive(Debug, Serialize)]
pub(crate) struct CheckItem {
    pub name: String,
    /// "ok" | "error"
    pub status: String,
    pub message: String,
}

/// Full check report — serialized by `render()` for JSON and Toon outputs.
#[derive(Debug, Serialize)]
pub(crate) struct CheckReport {
    pub checks: Vec<CheckItem>,
}

pub fn run(config: &Config, quiet: bool, fmt: OutputFormat) -> Result<()> {
    let vault_result = check_vault(config);
    let state_result = check_state(config);

    // Build structured report regardless of quiet flag.
    let mut checks = vec![
        CheckItem {
            name: "vault".to_string(),
            status: if vault_result.is_ok() {
                "ok".to_string()
            } else {
                "error".to_string()
            },
            message: vault_result
                .as_ref()
                .map(|_| config.vault_root.display().to_string())
                .unwrap_or_else(|e| e.clone()),
        },
        CheckItem {
            name: "state".to_string(),
            status: if state_result.is_ok() {
                "ok".to_string()
            } else {
                "error".to_string()
            },
            message: state_result
                .as_ref()
                .map(|_| config.state_dir.display().to_string())
                .unwrap_or_else(|e| e.clone()),
        },
        CheckItem {
            name: "contexts".to_string(),
            status: if config.contexts.is_empty() {
                "error".to_string()
            } else {
                "ok".to_string()
            },
            message: if config.contexts.is_empty() {
                "none configured".to_string()
            } else {
                config.contexts.join(", ")
            },
        },
    ];

    // Cloud-config checks (api_url + active-provider callback_url). Loaded
    // best-effort: an absent/unparseable global config is already reported by
    // the state check above, so skip silently here rather than double-report.
    if let Ok(global) = crate::config::load_global_config() {
        checks.extend(cloud_checks(&global));
    }

    let has_errors = checks.iter().any(|c| c.status == "error");

    // quiet mode: suppress output, just propagate errors.
    if quiet {
        for check in &checks {
            if check.status == "error" {
                crate::output::error(format!("{}: {}", check.name, check.message));
            }
        }
        return Ok(());
    }

    let report = CheckReport { checks };
    let rendered = render(&report, fmt)?;
    println!("{rendered}");

    if has_errors {
        std::process::exit(1);
    }
    Ok(())
}

/// Health checks for cloud configuration: the API URL and the active OAuth
/// provider's callback URL. Both default to empty (per-instance config is
/// written by `temper init`); an empty value silently breaks every cloud
/// command (`resource list` → reqwest "builder error", `auth login` → Auth0
/// "Oops") so we surface it here with an actionable message.
///
/// Returns no items when cloud sync is disabled (`auth.provider = "none"`),
/// where empty values are the expected state rather than a misconfiguration.
fn cloud_checks(global: &GlobalConfig) -> Vec<CheckItem> {
    if global.auth.provider == "none" {
        return Vec::new();
    }

    let api_empty = global.cloud.api_url.is_empty();
    let api_item = CheckItem {
        name: "cloud-api".to_string(),
        status: if api_empty { "error" } else { "ok" }.to_string(),
        message: if api_empty {
            "cloud API URL not configured — run `temper init` (or set [cloud].api_url)".to_string()
        } else {
            global.cloud.api_url.clone()
        },
    };

    let callback_item = match global
        .auth
        .providers
        .iter()
        .find(|p| p.name == global.auth.provider)
    {
        Some(p) if p.callback_url.is_empty() => CheckItem {
            name: "auth-callback".to_string(),
            status: "error".to_string(),
            message: format!(
                "OAuth callback URL not configured for provider '{}' — run `temper init` \
                 (or set callback_url)",
                p.name
            ),
        },
        Some(p) => CheckItem {
            name: "auth-callback".to_string(),
            status: "ok".to_string(),
            message: p.callback_url.clone(),
        },
        None => CheckItem {
            name: "auth-callback".to_string(),
            status: "error".to_string(),
            message: format!(
                "auth provider '{}' not found in [[auth.providers]] — run `temper config edit`",
                global.auth.provider
            ),
        },
    };

    vec![api_item, callback_item]
}

fn check_vault(config: &Config) -> std::result::Result<(), String> {
    if !config.vault_root.exists() {
        return Err(format!(
            "vault root does not exist: {}",
            config.vault_root.display()
        ));
    }
    Ok(())
}

fn check_state(config: &Config) -> std::result::Result<(), String> {
    if !config.state_dir.exists() {
        return Err(format!(
            "not initialized — run 'temper init' ({})",
            config.state_dir.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_check_report_json_includes_checks_array() {
        let report = CheckReport {
            checks: vec![CheckItem {
                name: "db".to_string(),
                status: "ok".to_string(),
                message: "connected".to_string(),
            }],
        };
        let out = render(&report, OutputFormat::Json).expect("json render");
        assert!(out.contains("\"checks\""), "json: {out}");
        assert!(out.contains("\"status\": \"ok\""), "json: {out}");
        assert!(out.contains("\"name\": \"db\""), "json: {out}");
    }

    use temper_core::types::config::AuthProvider;

    fn provider(name: &str, callback_url: &str) -> AuthProvider {
        AuthProvider {
            name: name.to_string(),
            authorize_url: "https://id.example.com/authorize".to_string(),
            token_url: "https://id.example.com/oauth/token".to_string(),
            client_id: "client-abc".to_string(),
            audience: "https://api.example.com".to_string(),
            callback_url: callback_url.to_string(),
            scopes: vec!["openid".to_string()],
        }
    }

    fn item<'a>(checks: &'a [CheckItem], name: &str) -> &'a CheckItem {
        checks
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no '{name}' check in {checks:?}"))
    }

    #[test]
    fn cloud_checks_skipped_when_provider_none() {
        let global = GlobalConfig::default(); // provider = "none"
        assert!(cloud_checks(&global).is_empty());
    }

    #[test]
    fn cloud_checks_ok_when_fully_configured() {
        let mut global = GlobalConfig::default();
        global.auth.provider = "auth0".to_string();
        global.auth.providers.push(provider(
            "auth0",
            "https://api.example.com/api/auth/cli-callback",
        ));
        global.cloud.api_url = "https://api.example.com".to_string();

        let checks = cloud_checks(&global);
        assert_eq!(item(&checks, "cloud-api").status, "ok");
        assert_eq!(item(&checks, "auth-callback").status, "ok");
    }

    #[test]
    fn cloud_checks_flags_empty_api_url() {
        let mut global = GlobalConfig::default();
        global.auth.provider = "auth0".to_string();
        global.auth.providers.push(provider(
            "auth0",
            "https://api.example.com/api/auth/cli-callback",
        ));
        // cloud.api_url left empty (the regression default)

        let checks = cloud_checks(&global);
        let api = item(&checks, "cloud-api");
        assert_eq!(api.status, "error");
        assert!(api.message.contains("temper init"), "{}", api.message);
    }

    #[test]
    fn cloud_checks_flags_empty_callback_url() {
        let mut global = GlobalConfig::default();
        global.auth.provider = "auth0".to_string();
        global.auth.providers.push(provider("auth0", "")); // empty callback (the regression)
        global.cloud.api_url = "https://api.example.com".to_string();

        let checks = cloud_checks(&global);
        let cb = item(&checks, "auth-callback");
        assert_eq!(cb.status, "error");
        assert!(cb.message.contains("temper init"), "{}", cb.message);
    }

    #[test]
    fn cloud_checks_flags_missing_provider_entry() {
        let mut global = GlobalConfig::default();
        global.auth.provider = "auth0".to_string();
        // No matching [[auth.providers]] entry, but cloud is enabled.
        global.cloud.api_url = "https://api.example.com".to_string();

        let checks = cloud_checks(&global);
        let cb = item(&checks, "auth-callback");
        assert_eq!(cb.status, "error");
        assert!(cb.message.contains("not found"), "{}", cb.message);
    }

    #[test]
    fn render_check_report_json_message_field() {
        let report = CheckReport {
            checks: vec![CheckItem {
                name: "vault".to_string(),
                status: "error".to_string(),
                message: "vault root does not exist".to_string(),
            }],
        };
        let out = render(&report, OutputFormat::Json).expect("json render");
        assert!(out.contains("\"message\""), "json: {out}");
        assert!(out.contains("vault root does not exist"), "json: {out}");
    }
}
