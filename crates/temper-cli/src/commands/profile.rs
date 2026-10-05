//! `temper profile show|update` — the caller's own profile, one server call each.

use crate::cli::ProfileAction;
use crate::error::{Result, TemperError};
use crate::format::OutputFormat;
use temper_core::types::api::ProfileUpdateRequest;

pub fn run(action: ProfileAction, fmt: OutputFormat) -> Result<()> {
    match action {
        ProfileAction::Show => crate::actions::runtime::render_read(fmt, move |client| {
            Box::pin(async move { client.profile().get().await })
        }),
        ProfileAction::AuthLinks => crate::actions::runtime::render_read(fmt, move |client| {
            Box::pin(async move { client.profile().auth_links().await })
        }),
        ProfileAction::Update {
            display_name,
            preferences,
        } => {
            let request = update_request(display_name, preferences.as_deref())?;
            crate::actions::runtime::render_read(fmt, move |client| {
                Box::pin(async move { client.profile().update(&request).await })
            })
        }
    }
}

/// Build the PATCH body, refusing an update that would change nothing and preferences that are not
/// a JSON object — both before the wire.
fn update_request(
    display_name: Option<String>,
    preferences: Option<&str>,
) -> Result<ProfileUpdateRequest> {
    if display_name.is_none() && preferences.is_none() {
        return Err(TemperError::BadRequest(
            "nothing to update: pass --display-name and/or --preferences".to_string(),
        ));
    }
    let preferences = preferences
        .map(|raw| {
            let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| {
                TemperError::BadRequest(format!("--preferences is not valid JSON: {e}"))
            })?;
            if value.is_object() {
                Ok(value)
            } else {
                Err(TemperError::BadRequest(
                    "--preferences must be a JSON object".to_string(),
                ))
            }
        })
        .transpose()?;
    Ok(ProfileUpdateRequest {
        display_name,
        preferences,
        vault_config: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_that_changes_nothing_is_refused() {
        assert!(matches!(
            update_request(None, None),
            Err(TemperError::BadRequest(m)) if m.contains("nothing to update")
        ));
    }

    #[test]
    fn preferences_must_be_a_json_object() {
        assert!(update_request(None, Some("[1]")).is_err());
        assert!(update_request(None, Some("not json")).is_err());
        let ok = update_request(Some("Pete".to_string()), Some(r#"{"theme":"dark"}"#)).unwrap();
        assert_eq!(ok.preferences, Some(serde_json::json!({"theme": "dark"})));
    }
}
