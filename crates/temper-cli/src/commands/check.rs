use serde::Serialize;

use crate::config::Config;
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

pub fn run(config: &Config, quiet: bool, format: Option<String>) -> Result<()> {
    let vault_result = check_vault(config);
    let state_result = check_state(config);

    // Build structured report regardless of quiet flag.
    let checks = vec![
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

    let has_errors = checks.iter().any(|c| c.status == "error");

    let fmt = OutputFormat::resolve(format.as_deref());

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
