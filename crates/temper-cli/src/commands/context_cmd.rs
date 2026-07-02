use uuid::Uuid;

use crate::commands::resource::inject_context_ref;
use crate::config::{self, Config};
use crate::error::{Result, TemperError};
use crate::output;
use temper_core::context_ref::ContextOwnerRef;
use temper_core::types::context::ShareContextRequest;

/// Parse the `--owner` CLI value into a typed owner descriptor.
///
/// Accepts `@me` (the caller's own profile) or `+<team-slug>` (a team). Anything
/// else — including `@<handle>` — is rejected here; the server would refuse a
/// foreign-profile owner anyway.
fn parse_owner(owner: &str) -> Result<ContextOwnerRef> {
    if owner == "@me" {
        Ok(ContextOwnerRef::Me)
    } else if let Some(slug) = owner.strip_prefix('+') {
        if slug.is_empty() {
            Err(TemperError::BadRequest(
                "context owner `+<team-slug>` is missing the team slug".to_owned(),
            ))
        } else {
            Ok(ContextOwnerRef::Team(slug.to_owned()))
        }
    } else {
        Err(TemperError::BadRequest(format!(
            "invalid context owner {owner:?}: use `@me` or `+<team-slug>`"
        )))
    }
}

/// Add a context to sync.subscriptions.contexts in the global config.
pub fn add(name: &str) -> Result<()> {
    let config_path = config::global_config_path();

    config::safe_write(&config_path, |content| {
        // Check if the context already exists
        if content.contains(&format!("\"{name}\"")) {
            return content;
        }
        // Find the contexts line and append
        let mut result = String::new();
        for line in content.lines() {
            if line.trim_start().starts_with("contexts") && line.contains('[') {
                // Parse existing array and add new context
                if let Some(bracket_start) = line.find('[') {
                    if let Some(bracket_end) = line.find(']') {
                        let existing = &line[bracket_start + 1..bracket_end];
                        let trimmed = existing.trim();
                        let new_line = if trimmed.is_empty() {
                            format!("{}[\"{name}\"]", &line[..bracket_start])
                        } else {
                            format!("{}[{}, \"{name}\"]", &line[..bracket_start], trimmed)
                        };
                        result.push_str(&new_line);
                        result.push('\n');
                        continue;
                    }
                }
            }
            result.push_str(line);
            result.push('\n');
        }
        result
    })?;

    output::success(format!("Added context '{name}'"));
    Ok(())
}

/// Remove a context from sync.subscriptions.contexts in the global config.
pub fn remove(name: &str) -> Result<()> {
    let config_path = config::global_config_path();

    config::safe_write(&config_path, |content| {
        let mut result = String::new();
        for line in content.lines() {
            if line.trim_start().starts_with("contexts") && line.contains('[') {
                if let Some(bracket_start) = line.find('[') {
                    if let Some(bracket_end) = line.find(']') {
                        let existing = &line[bracket_start + 1..bracket_end];
                        // Parse items, filter out the one to remove
                        let items: Vec<&str> = existing
                            .split(',')
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .filter(|s| {
                                let unquoted = s.trim_matches('"');
                                unquoted != name
                            })
                            .collect();
                        let new_line = format!("{}[{}]", &line[..bracket_start], items.join(", "));
                        result.push_str(&new_line);
                        result.push('\n');
                        continue;
                    }
                }
            }
            result.push_str(line);
            result.push('\n');
        }
        result
    })?;

    output::success(format!("Removed context '{name}'"));
    Ok(())
}

/// Create a context on the remote server and render the resulting context row
/// with an injected `ref` field (`{owner_ref}/{slug}`) for copy-paste addressing.
pub async fn create_remote(
    client: &temper_client::TemperClient,
    name: &str,
    owner: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let owner = owner.map(parse_owner).transpose()?;
    let context = client
        .contexts()
        .create(name, owner)
        .await
        .map_err(crate::commands::client_err)?;

    let mut row = serde_json::to_value(&context)
        .map_err(|e| crate::error::TemperError::Api(format!("context serialize: {e}")))?;
    inject_context_ref(&mut row);

    let rendered = crate::format::render(&row, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// List configured contexts.
pub fn list(config: &Config, fmt: crate::format::OutputFormat) -> Result<()> {
    let mut names = config.contexts.clone();
    names.sort();

    let rendered = crate::format::render(&names, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Resolve a context ref (a bare UUID, or the `@handle/slug` / `+team-slug/slug` form that
/// `context list` renders) to its context id. `@me` shorthand is NOT resolved here — an operator
/// sharing a context addresses it by the concrete owner shown in the list (or by UUID).
pub async fn resolve_context_id(
    client: &temper_client::TemperClient,
    context: &str,
) -> Result<Uuid> {
    if let Ok(id) = Uuid::parse_str(context) {
        return Ok(id);
    }
    let (owner, slug) = context.split_once('/').ok_or_else(|| {
        TemperError::BadRequest(format!(
            "invalid context ref {context:?}: use a UUID or `@handle/slug` / `+team-slug/slug`"
        ))
    })?;
    if owner == "@me" {
        return Err(TemperError::BadRequest(
            "`@me` is not accepted for share — use your `@handle/slug` (see `context list`) or the context UUID"
                .to_owned(),
        ));
    }
    let contexts = client
        .contexts()
        .list()
        .await
        .map_err(crate::commands::client_err)?;
    contexts
        .into_iter()
        .find(|c| c.owner_ref == owner && c.slug == slug)
        .map(|c| *c.id)
        .ok_or_else(|| {
            TemperError::Api(format!(
                "context '{context}' not found among the contexts you can see"
            ))
        })
}

/// `temper context share <context_ref> <team>` — share a context into a team (admin-only).
pub async fn share_remote(
    client: &temper_client::TemperClient,
    context: &str,
    team: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let context_id = resolve_context_id(client, context).await?;
    let team_id = crate::actions::cogmap::resolve_team_id(client, team).await?;
    let outcome = client
        .contexts()
        .share_team(context_id, &ShareContextRequest { team_id })
        .await
        .map_err(crate::commands::client_err)?;
    let rendered = crate::format::render(&outcome, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// `temper context unshare <context_ref> <team>` — unshare a context from a team (admin-only).
pub async fn unshare_remote(
    client: &temper_client::TemperClient,
    context: &str,
    team: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let context_id = resolve_context_id(client, context).await?;
    let team_id = crate::actions::cogmap::resolve_team_id(client, team).await?;
    let outcome = client
        .contexts()
        .unshare_team(context_id, team_id)
        .await
        .map_err(crate::commands::client_err)?;
    let rendered = crate::format::render(&outcome, fmt)?;
    println!("{rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn render_context_list_json_is_array_of_strings() {
        let contexts = vec!["temper".to_string(), "knowledge".to_string()];
        let out = crate::format::render(&contexts, crate::format::OutputFormat::Json)
            .expect("json render");
        assert!(out.contains("\"temper\""), "json: {out}");
        assert!(out.contains("\"knowledge\""), "json: {out}");
        assert!(out.starts_with('['), "json should be an array: {out}");
    }

    #[test]
    fn render_context_list_toon_contains_context_names() {
        let contexts = vec!["temper".to_string(), "knowledge".to_string()];
        let out = crate::format::render(&contexts, crate::format::OutputFormat::Toon)
            .expect("toon render");
        assert!(out.contains("temper"), "toon: {out}");
        assert!(out.contains("knowledge"), "toon: {out}");
    }
}
