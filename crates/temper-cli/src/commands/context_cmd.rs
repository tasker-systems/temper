use uuid::Uuid;

use crate::commands::resource::inject_context_ref;
use crate::config;
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

/// Subscribe to a context locally: add it to sync.subscriptions.contexts in the
/// global config so `temper pull` materializes it. This is a local-only
/// subscription toggle — it does not create or touch the context server-side.
pub fn subscribe(name: &str) -> Result<()> {
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

    output::success(format!("Subscribed to context '{name}'"));
    Ok(())
}

/// Unsubscribe from a context locally: remove it from
/// sync.subscriptions.contexts in the global config. Local-only — no server effect.
pub fn unsubscribe(name: &str) -> Result<()> {
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

    output::success(format!("Unsubscribed from context '{name}'"));
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

/// List the contexts visible to the caller on the server, each rendered with an
/// injected `ref` field (`{owner_ref}/{slug}`) for copy-paste addressing. This is
/// API-only — it reflects server state (owner + resource counts), not the local
/// `context subscribe` set.
pub async fn list(
    client: &temper_client::TemperClient,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let contexts = client
        .contexts()
        .list()
        .await
        .map_err(crate::commands::client_err)?;

    let mut rows = serde_json::to_value(&contexts)
        .map_err(|e| crate::error::TemperError::Api(format!("context serialize: {e}")))?;
    if let Some(arr) = rows.as_array_mut() {
        for row in arr.iter_mut() {
            inject_context_ref(row);
        }
    }

    let rendered = crate::format::render(&rows, fmt)?;
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

/// Map a `context share`/`unshare` client error to a CLI error, enriching the bare
/// `Forbidden` (the server returns a message-less 403) with the actual authorization
/// requirement and the escalation path — instead of the opaque "forbidden" that reads as a
/// permissions bug (issue #367). The word "instance administrator" is spelled out so it is
/// never confused with the per-team `admin`/`owner` roles.
fn map_share_err(action: &str, e: temper_client::error::ClientError) -> TemperError {
    match e {
        temper_client::error::ClientError::Forbidden => TemperError::Api(format!(
            "not authorized: `context {action}` requires that you administer the context \
             (own it, or manage its owning team) AND manage the target team \
             (owner/maintainer) — or that you are an instance administrator. Ask an instance \
             administrator, or use `context create --owner +<team>` to create a new \
             team-owned context instead."
        )),
        other => crate::commands::client_err(other),
    }
}

/// `temper context share <context_ref> <team>` — share a context into a team's read-reach.
/// Authorized by the server's `can_share` gate (context-admin + team-manager, or instance-admin).
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
        .map_err(|e| map_share_err("share", e))?;
    let rendered = crate::format::render(&outcome, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// `temper context unshare <context_ref> <team>` — unshare a context from a team
/// (same authority as `share`).
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
        .map_err(|e| map_share_err("unshare", e))?;
    let rendered = crate::format::render(&outcome, fmt)?;
    println!("{rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_owner_accepts_me_and_team() {
        assert!(matches!(parse_owner("@me"), Ok(ContextOwnerRef::Me)));
        match parse_owner("+platform") {
            Ok(ContextOwnerRef::Team(slug)) => assert_eq!(slug, "platform"),
            other => panic!("expected team owner, got {other:?}"),
        }
    }

    #[test]
    fn parse_owner_rejects_handle_and_empty_team() {
        assert!(parse_owner("@someone").is_err());
        assert!(parse_owner("+").is_err());
    }

    #[test]
    fn list_render_injects_ref_from_owner_and_slug() {
        // Mirror the API-only `list` render path: a context row carrying
        // `owner_ref` + `slug` gets a decorated `ref` injected for addressing.
        let mut row = serde_json::json!({
            "owner_ref": "@alice",
            "slug": "temper",
            "name": "temper",
            "resource_count": 3,
        });
        inject_context_ref(&mut row);
        let out =
            crate::format::render(&row, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.contains("\"ref\""), "expected injected ref: {out}");
        assert!(
            out.contains("@alice/temper"),
            "expected decorated ref: {out}"
        );
    }
}
