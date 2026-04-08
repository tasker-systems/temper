//! `temper config edit` actions — temp-file editor workflow with validation.

use std::path::{Path, PathBuf};

use temper_core::types::config::TemperConfig;
use validator::Validate;

use crate::error::{Result, TemperError};

/// Outcome of parsing + validating edited TOML content.
#[derive(Debug)]
pub enum ParseOutcome {
    Valid(TemperConfig),
    Invalid(String),
}

/// Parse TOML text into `TemperConfig` and run validator rules.
pub fn parse_and_validate(content: &str) -> ParseOutcome {
    let parsed: TemperConfig = match toml::from_str(content) {
        Ok(c) => c,
        Err(e) => return ParseOutcome::Invalid(format!("TOML parse error: {e}")),
    };
    if let Err(errors) = parsed.validate() {
        return ParseOutcome::Invalid(format_errors(&errors));
    }
    ParseOutcome::Valid(parsed)
}

fn format_errors(errors: &validator::ValidationErrors) -> String {
    let mut out = String::from("Configuration is invalid:\n");
    walk_errors(errors, "", &mut out);
    out
}

fn walk_errors(errors: &validator::ValidationErrors, prefix: &str, out: &mut String) {
    for (field, kind) in errors.errors() {
        let path = if prefix.is_empty() {
            field.to_string()
        } else {
            format!("{prefix}.{field}")
        };
        match kind {
            validator::ValidationErrorsKind::Field(field_errors) => {
                for err in field_errors {
                    let msg = err
                        .message
                        .as_ref()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| err.code.to_string());
                    out.push_str(&format!("  - {path}: {msg}\n"));
                }
            }
            validator::ValidationErrorsKind::Struct(nested) => {
                walk_errors(nested, &path, out);
            }
            validator::ValidationErrorsKind::List(items) => {
                for (idx, nested) in items {
                    let list_path = format!("{path}[{idx}]");
                    walk_errors(nested, &list_path, out);
                }
            }
        }
    }
}

/// Build the temp edit-file path (sibling of the target file).
pub fn temp_edit_path(target: &Path) -> PathBuf {
    let mut file_name = target.file_name().unwrap_or_default().to_os_string();
    file_name.push(".edit");
    target.with_file_name(file_name)
}

/// Atomically replace `target` with the contents currently in `edit_path`.
///
/// Uses `std::fs::rename` which is atomic on the same filesystem.
pub fn commit_edit(edit_path: &Path, target: &Path) -> Result<()> {
    std::fs::rename(edit_path, target)
        .map_err(|e| TemperError::Config(format!("cannot commit config edit: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
[vault]
path = "/tmp/v"

[skill]
output = "~/.claude/skills/temper"

[auth]
provider = "auth0"

[[auth.providers]]
name = "auth0"
authorize_url = "https://example.com/a"
token_url = "https://example.com/t"
client_id = "cid"
audience = "https://example.com/api"

[cloud]
api_url = "https://example.com"
"#;

    #[test]
    fn parse_valid_config_returns_valid() {
        match parse_and_validate(VALID) {
            ParseOutcome::Valid(_) => {}
            ParseOutcome::Invalid(msg) => panic!("expected valid, got: {msg}"),
        }
    }

    #[test]
    fn parse_invalid_toml_returns_invalid() {
        match parse_and_validate("not = toml =") {
            ParseOutcome::Invalid(msg) => assert!(msg.contains("TOML parse error")),
            _ => panic!("expected invalid"),
        }
    }

    #[test]
    fn parse_empty_vault_path_returns_invalid() {
        let broken = VALID.replace(r#"path = "/tmp/v""#, r#"path = """#);
        match parse_and_validate(&broken) {
            ParseOutcome::Invalid(msg) => assert!(msg.contains("vault") || msg.contains("path")),
            _ => panic!("expected invalid"),
        }
    }

    #[test]
    fn parse_bad_url_returns_invalid() {
        let broken = VALID.replace(
            r#"api_url = "https://example.com""#,
            r#"api_url = "not a url""#,
        );
        match parse_and_validate(&broken) {
            ParseOutcome::Invalid(msg) => assert!(msg.contains("api_url") || msg.contains("url")),
            _ => panic!("expected invalid"),
        }
    }

    #[test]
    fn temp_edit_path_is_sibling_with_dot_edit() {
        let p = std::path::PathBuf::from("/a/b/config.toml");
        assert_eq!(
            temp_edit_path(&p),
            std::path::PathBuf::from("/a/b/config.toml.edit")
        );
    }

    #[test]
    fn commit_edit_moves_file_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("config.toml");
        let edit = tmp.path().join("config.toml.edit");
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&edit, "new").unwrap();
        commit_edit(&edit, &target).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(!edit.exists());
    }
}
