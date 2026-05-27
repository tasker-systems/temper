use crate::config::Config;
use crate::error::{Result, TemperError};

// Re-export data types and functions from the actions layer
pub use crate::actions::task::{find_task, load_tasks, next_seq};
pub use crate::actions::types::TaskInfo;

/// Show a single task's content.
///
/// Cloud-only: resolves the task id via `resolve_by_uri`, fetches content,
/// renders it, and writes the canonical projection file (per-resource
/// refresh — best-effort).
pub fn show(
    config: &Config,
    slug_or_suffix: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::runtime;

    let context_s = context.map(str::to_string);
    let slug_s = slug_or_suffix.to_string();
    let config_clone = config.clone();
    let format_s = format.to_string();

    let (row, body) = runtime::with_client(|client| {
        Box::pin(async move {
            let ctx = context_s.as_deref().ok_or_else(|| {
                TemperError::Project("no context specified — use --context <name>".into())
            })?;
            let owner = config_clone.owner_for_context(ctx);
            let row = client
                .resources()
                .resolve_by_uri(&owner, ctx, "task", &slug_s)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let resp = client
                .resources()
                .content(*row.id.as_uuid())
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;

            // Per-resource projection refresh — best-effort.
            if let Err(e) = crate::projection::write_resource_file_from_parts(
                &config_clone.vault_root,
                &row,
                &resp,
            ) {
                crate::output::warning(format!(
                    "could not refresh projection file for '{slug_s}': {e}"
                ));
            }

            Ok((row, resp.markdown))
        })
    })?;

    let fmt = crate::format::OutputFormat::resolve(Some(&format_s));
    let metadata = serde_json::to_value(&row)
        .map_err(|e| TemperError::Api(format!("metadata serialize: {e}")))?;
    let rendered = crate::format::render_resource_show(&metadata, &body, fmt)?;
    println!("{rendered}");
    Ok(())
}
