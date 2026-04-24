use temper_core::vault::Vault;

use crate::config::Config;
use crate::error::{Result, TemperError};

// Re-export data types and functions from the actions layer
pub use crate::actions::task::{create, done, find_task, load_tasks, move_task, next_seq};
pub use crate::actions::types::TaskInfo;

/// Show a single task's content.
///
/// Local mode: for JSON, emits the `TaskInfo` struct (fast, no API call needed
/// for the task metadata). For plain text, uses the three-tier freshness
/// ladder to decide whether to serve from cache or fetch from the API.
///
/// Cloud mode: resolves the task id via `GET /api/resources/by-uri` then
/// fetches content via `GET /api/resources/{id}/content`. No disk writes.
pub fn show(
    config: &Config,
    slug_or_suffix: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::{runtime, show_cache};
    use std::time::Duration;
    use temper_core::types::VaultState;

    let vault_state = VaultState::from_env();

    match vault_state {
        VaultState::Local => {
            let task = find_task(config, slug_or_suffix, context)?
                .ok_or_else(|| TemperError::Vault(format!("task not found: {slug_or_suffix}")))?;

            if format == "json" {
                let json = serde_json::to_string_pretty(&task)
                    .map_err(|e| TemperError::Vault(format!("json serialization failed: {e}")))?;
                println!("{json}");
                return Ok(());
            }

            let vault_layout = Vault::new(&config.vault_root);
            let owner = config.owner_for_context(&task.context);
            let path = vault_layout.doc_file(&owner, &task.context, "task", &task.slug);
            let task_ctx = task.context.clone();
            let task_slug = task.slug.clone();
            let config_clone = config.clone();

            // Tier 0: serve from disk if fresh — no runtime or API needed.
            if let Some(body) = show_cache::read_if_fresh(
                &path,
                std::time::Duration::from_secs(show_cache::DEFAULT_DEBOUNCE_SECONDS),
            )? {
                print!("{body}");
                return Ok(());
            }

            let body = runtime::with_client(|client| {
                Box::pin(async move {
                    let id = super::resource::resolve_resource_id(
                        &config_clone,
                        client,
                        "task",
                        &task_slug,
                        Some(&task_ctx),
                        VaultState::Local,
                    )
                    .await?;
                    let result = show_cache::fetch(show_cache::ShowCacheParams {
                        client,
                        resource_id: id,
                        local_path: &path,
                        debounce: Duration::from_secs(show_cache::DEFAULT_DEBOUNCE_SECONDS),
                    })
                    .await?;
                    Ok(result.content)
                })
            })?;

            print!("{body}");
            Ok(())
        }
        VaultState::Cloud => {
            let context_s = context.map(str::to_string);
            let slug_s = slug_or_suffix.to_string();
            let config_clone = config.clone();

            let body = runtime::with_client(|client| {
                Box::pin(async move {
                    let id = super::resource::resolve_resource_id(
                        &config_clone,
                        client,
                        "task",
                        &slug_s,
                        context_s.as_deref(),
                        VaultState::Cloud,
                    )
                    .await?;
                    let resp = client
                        .resources()
                        .content(*id.as_uuid())
                        .await
                        .map_err(crate::actions::runtime::client_err_to_temper)?;
                    Ok(resp.markdown)
                })
            })?;

            print!("{body}");
            Ok(())
        }
    }
}
