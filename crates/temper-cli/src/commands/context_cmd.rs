use uuid::Uuid;

use crate::commands::resource::inject_context_ref;
use crate::config;
use crate::error::{Result, TemperError};
use crate::output;
use temper_core::context_ref::ContextOwnerRef;
use temper_core::types::context::{
    ReassignContextRequest, RenameContextRequest, ShareContextRequest,
};

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
        .map_err(crate::actions::runtime::client_err_to_temper)?;

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
///
/// `retired: true` switches the read from the visibility axis to the ADMIN axis: it lists
/// retired contexts the caller administers, not contexts they can read. A retired context is
/// invisible to the read path by construction, so this is a different question, not a filter
/// over the same rows.
pub async fn list(
    client: &temper_client::TemperClient,
    retired: bool,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let contexts = if retired {
        client
            .contexts()
            .list_retired()
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?
    } else {
        client
            .contexts()
            .list()
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?
    };

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
        .map_err(crate::actions::runtime::client_err_to_temper)?;
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
        other => crate::actions::runtime::client_err_to_temper(other),
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

/// `temper context transfer <context_ref> <team>` — transfer a context's ownership to a team.
///
/// Binding a context to a team is the single path to shared authorship (read-sharing stays
/// `share`; writing into a context requires team ownership). Uses the `@me`-accepting read
/// resolver, because the headline flow is `@me/my-project → team` — the `share`/`unshare`
/// resolver deliberately refuses `@me`.
pub async fn transfer_remote(
    client: &temper_client::TemperClient,
    context: &str,
    team: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let context_id = resolve_context_id_for_read(client, context).await?;
    let to_team_id = crate::actions::cogmap::resolve_team_id(client, team).await?;
    let outcome = client
        .contexts()
        .reassign(context_id, &ReassignContextRequest { to_team_id })
        .await
        .map_err(|e| map_share_err("transfer", e))?;
    let rendered = crate::format::render(&outcome, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Map a `context rename`/`context delete` client error to a CLI error. Deliberately **not**
/// [`map_share_err`]: that message names a target team ("...AND manage the target team"), and
/// neither rename nor delete has one — reusing it would state a requirement that does not exist.
/// Both require only that the caller administer the context itself, so one mapper serves both,
/// parametrized on the action name the way [`map_share_err`] already is on `action`.
fn map_admin_required_err(action: &str, e: temper_client::error::ClientError) -> TemperError {
    match e {
        temper_client::error::ClientError::Forbidden => TemperError::Api(format!(
            "not authorized: `context {action}` requires that you administer the context \
             (own it, or manage its owning team as owner/maintainer) — or that you are an \
             instance administrator."
        )),
        other => crate::actions::runtime::client_err_to_temper(other),
    }
}

/// `temper context rename <context_ref> --name <name>` — change a context's name; the server
/// re-derives the slug from it.
///
/// The rename **re-addresses** the context: the printed outcome carries the new `context_ref`,
/// which is the address to use from now on. Uses the `@me`-accepting read resolver, like
/// `transfer` — the headline flow is `@me/my-project`.
pub async fn rename_remote(
    client: &temper_client::TemperClient,
    context: &str,
    name: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let context_id = resolve_context_id_for_read(client, context).await?;
    let outcome = client
        .contexts()
        .rename(
            context_id,
            &RenameContextRequest {
                name: name.to_owned(),
            },
        )
        .await
        .map_err(|e| map_admin_required_err("rename", e))?;
    let rendered = crate::format::render(&outcome, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// `temper context delete <context_ref>` — retire a context.
///
/// **Reversible.** The context stops being visible on the read path and stops being writeable,
/// but every row it homes is preserved untouched, and the slug is freed for immediate reuse.
/// The rendered outcome carries the mangled `context_ref` — the address `temper context
/// restore` accepts, since the original ref no longer resolves once the row is hidden and the
/// slug has moved. Uses the `@me`-accepting read resolver, like `rename` and `transfer` — the
/// headline flow is retiring your own `@me/my-project`.
pub async fn delete_remote(
    client: &temper_client::TemperClient,
    context: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let context_id = resolve_context_id_for_read(client, context).await?;
    let outcome = client
        .contexts()
        .delete(context_id)
        .await
        .map_err(|e| map_admin_required_err("delete", e))?;
    let rendered = crate::format::render(&outcome, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Resolve a context ref for `restore`. Cannot use [`resolve_context_id_for_read`]: that
/// resolver lists through the read predicate, and a retired context is invisible there by
/// construction (that is what retirement means) — the row `restore` needs to find will never be
/// in that listing. This resolves on the ADMIN axis instead, via
/// [`temper_client::contexts::ContextClient::list_retired`], which lists exactly the retired
/// contexts the caller administers — the same set `temper context list --retired` prints, and
/// the same authority `restore` itself requires server-side.
///
/// `@me` is not accepted, unlike the read resolver: the ref this has to match is the one
/// `delete` printed (`RetireContextOutcome::context_ref`), which the server always composes
/// from the concrete `owner_ref` — `@me` can never actually appear in that data, so accepting it
/// here would add a spelling with nothing behind it.
pub async fn resolve_context_id_for_restore(
    client: &temper_client::TemperClient,
    context: &str,
) -> Result<Uuid> {
    if let Ok(id) = Uuid::parse_str(context) {
        return Ok(id);
    }
    let (owner, slug) = context.split_once('/').ok_or_else(|| {
        TemperError::BadRequest(format!(
            "invalid context ref {context:?}: use a UUID or the mangled ref `context delete` \
             printed (`@me/slug` / `@handle/slug` / `+team-slug/slug`) — NOT the original ref"
        ))
    })?;

    let contexts = client
        .contexts()
        .list_retired()
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;

    // `@me` is resolved exactly as `resolve_context_id_for_read` resolves it — profile lookup,
    // then match on the owner id rather than the decorated ref. Carried rather than dropped: the
    // ref `delete` prints is the `@handle` form, but `context list --retired` shows the same row
    // and an operator will reach for `@me` reflexively. Without this the failure would read
    // "not found among the retired contexts you administer" for a context that IS there and IS
    // administered — a misleading refusal, which is worse than an unsupported one.
    let found = if owner == "@me" {
        let me = client
            .profile()
            .get()
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        contexts
            .into_iter()
            .find(|c| c.kb_owner_table == "kb_profiles" && c.kb_owner_id == me.id && c.slug == slug)
    } else {
        contexts
            .into_iter()
            .find(|c| c.owner_ref == owner && c.slug == slug)
    };

    found.map(|c| *c.id).ok_or_else(|| {
        TemperError::Api(format!(
            "retired context '{context}' not found among the retired contexts you administer \
             — use the mangled ref `context delete` printed, or check `context list --retired`"
        ))
    })
}

/// `temper context restore <context_ref>` — reverse a retirement.
///
/// `context` must be a UUID or the mangled ref `delete` printed; resolved on the admin axis via
/// [`resolve_context_id_for_restore`], not the read-accepting resolver every other context
/// command uses.
pub async fn restore_remote(
    client: &temper_client::TemperClient,
    context: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let context_id = resolve_context_id_for_restore(client, context).await?;
    let outcome = client
        .contexts()
        .restore(context_id)
        .await
        .map_err(|e| map_admin_required_err("restore", e))?;
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

// ── Context orientation reads (spec §3.7, T8) ────────────────────────────────

/// Resolve a context ref for a READ. Unlike [`resolve_context_id`] (which serves `share`/`unshare`
/// and deliberately refuses `@me`, because an operator granting reach should name the concrete owner),
/// the orientation reads accept `@me/<slug>` — it is the form every agent-facing surface already uses
/// (`resource list --context @me/temper`), and refusing it here would be a gratuitous inconsistency.
///
/// `@me` is matched on the caller's own profile id, not on a reconstructed `@handle` string.
pub async fn resolve_context_id_for_read(
    client: &temper_client::TemperClient,
    context: &str,
) -> Result<Uuid> {
    if let Ok(id) = Uuid::parse_str(context) {
        return Ok(id);
    }
    let (owner, slug) = context.split_once('/').ok_or_else(|| {
        TemperError::BadRequest(format!(
            "invalid context ref {context:?}: use a UUID or `@me/slug` / `@handle/slug` / `+team-slug/slug`"
        ))
    })?;

    let contexts = client
        .contexts()
        .list()
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;

    let found = if owner == "@me" {
        let me = client
            .profile()
            .get()
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        contexts
            .into_iter()
            .find(|c| c.kb_owner_table == "kb_profiles" && c.kb_owner_id == me.id && c.slug == slug)
    } else {
        contexts
            .into_iter()
            .find(|c| c.owner_ref == owner && c.slug == slug)
    };

    found.map(|c| *c.id).ok_or_else(|| {
        TemperError::Api(format!(
            "context '{context}' not found among the contexts you can see"
        ))
    })
}

/// Optional lens ref → UUID (trailing-UUID-only, like every other ref on the CLI).
fn lens_id_of(lens: Option<&str>) -> Result<Option<Uuid>> {
    lens.map(|l| temper_workflow::operations::parse_ref(l).map(|p| p.0))
        .transpose()
}

/// `temper context shape <context_ref> [--lens <ref>]` — the context's materialized regions.
///
/// What is rendered is the `AnchorShape` envelope, not a bare list: an empty `regions` carries an
/// `emptiness` naming why it is empty, so the CLI never has to guess a cause on the caller's behalf.
pub async fn shape_remote(
    client: &temper_client::TemperClient,
    context: &str,
    lens: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let context_id = resolve_context_id_for_read(client, context).await?;
    let lens_id = lens_id_of(lens)?;
    let shape = client
        .contexts()
        .shape(context_id, lens_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    let rendered = crate::format::render(&shape, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper context region-metrics <context_ref> [--lens <ref>]` — the per-region analytics tier.
pub async fn region_metrics_remote(
    client: &temper_client::TemperClient,
    context: &str,
    lens: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let context_id = resolve_context_id_for_read(client, context).await?;
    let lens_id = lens_id_of(lens)?;
    let rows = client
        .contexts()
        .region_metrics(context_id, lens_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    let rendered = crate::format::render(&rows, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper context analytics <context_ref>` — the context-level staleness readout.
///
/// The peer of `temper cogmap analytics`, and the last asymmetric row of the anchor read surface.
/// Three fields, not five: a context has no charter resource and no regulation set, so
/// `telos_resource_id` and `regulation` would be null peer fields reporting "nothing found" about
/// two things that cannot exist.
///
/// Deny is an error (404), collapsed with "does not exist" — the `materialize-delta` posture, not
/// the 200-with-`emptiness` posture of `shape`.
pub async fn analytics_remote(
    client: &temper_client::TemperClient,
    context: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let context_id = resolve_context_id_for_read(client, context).await?;
    let staleness = client
        .contexts()
        .analytics(context_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    let rendered = crate::format::render(&staleness, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper context materialize <context_ref> [--threshold N]` — re-form the context's regions.
pub async fn materialize_remote(
    client: &temper_client::TemperClient,
    context: &str,
    threshold: Option<i64>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let context_id = resolve_context_id_for_read(client, context).await?;
    let ack = client
        .contexts()
        .materialize(context_id, threshold)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    let rendered = crate::format::render(&ack, fmt)?;
    crate::output::plain(rendered);
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
