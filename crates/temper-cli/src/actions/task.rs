use temper_core::types::managed_meta::ManagedMeta;
use temper_core::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

use crate::actions::runtime;
use crate::actions::types::TaskInfo;
use crate::config::Config;
use crate::error::{Result, TemperError};

/// Load all tasks, optionally filtered by context and/or goal.
///
/// Cloud-only: tasks are listed from the API via `client.resources().list_meta`,
/// not by scanning the local vault. The local vault is a read-only projection
/// cache that is empty/absent on a fresh device, so a disk scan would silently
/// return nothing there. This keeps `load_tasks` (and its callers `next_seq` /
/// `find_task`) correct regardless of projection state.
///
/// Each `TaskInfo` is built by combining the server's `managed_meta` (which
/// carries `temper-title`, `temper-slug`, `temper-stage`, `temper-mode`,
/// `temper-effort`, `temper-goal`, `temper-seq`, `temper-branch`, `temper-pr`)
/// with the context the query was scoped to — `temper-context` is generally not
/// present in managed_meta, so it is stamped from the query scope rather than
/// read from a row column.
///
/// The function is synchronous: `runtime::with_client` builds its own tokio
/// runtime, and all callers (`warmup::collect_in_progress_tasks`, `next_seq`,
/// `find_task`) are synchronous and not already inside a runtime.
pub fn load_tasks(
    config: &Config,
    context: Option<&str>,
    goal_slug: Option<&str>,
) -> Result<Vec<TaskInfo>> {
    // Scope the query to a single context when given, else fan out across the
    // profile's configured contexts. Each query is scoped server-side to
    // visible resources; the `context_name` we pass in is the canonical
    // context for every row it returns, which is what we stamp into TaskInfo.
    let contexts: Vec<String> = match context {
        Some(p) => vec![p.to_string()],
        None => config.contexts.clone(),
    };

    let mut tasks = Vec::new();
    for ctx in contexts {
        let api_params = ResourceListParams {
            doc_type_name: Some("task".to_string()),
            context_name: Some(ctx.clone()),
            sort: Some(ResourceSortField::Seq),
            order: Some(SortOrder::Asc),
            // Tasks per context are bounded; a generous cap avoids pagination
            // while still protecting against a runaway response.
            limit: Some(1000),
            ..Default::default()
        };

        let ctx_for_query = ctx.clone();
        let response = runtime::with_client(move |client| {
            let api_params = api_params.clone();
            Box::pin(async move {
                client
                    .resources()
                    .list_meta(&api_params)
                    .await
                    .map_err(runtime::client_err_to_temper)
            })
        })?;

        for row in response.rows {
            let Some(meta) = row.managed_meta else {
                // A task without a manifest meta row predates meta population;
                // skip it rather than fabricate a partial TaskInfo.
                continue;
            };
            let info = task_info_from_meta(meta, &ctx_for_query)?;
            if let Some(gs) = goal_slug {
                if info.goal.as_deref() != Some(gs) {
                    continue;
                }
            }
            tasks.push(info);
        }
    }

    tasks.sort_by_key(|t| t.seq.unwrap_or(u32::MAX));
    Ok(tasks)
}

/// Build a [`TaskInfo`] from a resource's typed `managed_meta` plus the context
/// the listing was scoped to. `temper-title` and `temper-slug` are required —
/// a task is meaningless without them — so their absence is an error rather
/// than a silent skip. `temper-seq` is stored as `i64` in `ManagedMeta`;
/// `TaskInfo` uses `u32`, so negative or out-of-range values clamp to `None`
/// (treated as unsequenced, sorting last).
fn task_info_from_meta(meta: ManagedMeta, context: &str) -> Result<TaskInfo> {
    let title = meta
        .title
        .ok_or_else(|| TemperError::Api("task managed_meta missing temper-title".to_string()))?;
    let slug = meta
        .slug
        .ok_or_else(|| TemperError::Api("task managed_meta missing temper-slug".to_string()))?;
    Ok(TaskInfo {
        title,
        slug,
        context: context.to_string(),
        goal: meta.goal,
        stage: meta.stage.unwrap_or_default(),
        mode: meta.mode,
        effort: meta.effort,
        seq: meta.seq.and_then(|s| u32::try_from(s).ok()),
        branch: meta.branch,
        pr: meta.pr,
    })
}

/// Get the next seq value for a new task in a goal.
pub fn next_seq(config: &Config, context: &str, goal_slug: &str) -> Result<u32> {
    let tasks = load_tasks(config, Some(context), Some(goal_slug))?;
    let max_seq = tasks.iter().filter_map(|t| t.seq).max().unwrap_or(0);
    Ok(max_seq + 10)
}

/// Find a task by exact slug or unambiguous suffix match.
pub fn find_task(
    config: &Config,
    slug_or_suffix: &str,
    context: Option<&str>,
) -> Result<Option<TaskInfo>> {
    let all = load_tasks(config, context, None)?;
    // Exact match first
    if let Some(t) = all.iter().find(|t| t.slug == slug_or_suffix) {
        return Ok(Some(t.clone()));
    }
    // Suffix match
    let matches: Vec<_> = all
        .iter()
        .filter(|t| t.slug.ends_with(slug_or_suffix))
        .collect();
    match matches.len() {
        1 => return Ok(Some(matches[0].clone())),
        n if n > 1 => {
            return Err(TemperError::Vault(format!(
                "ambiguous slug suffix '{slug_or_suffix}', matches: {}",
                matches
                    .iter()
                    .map(|t| t.slug.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
        _ => {}
    }
    // Seq number match
    if let Ok(seq) = slug_or_suffix.parse::<u32>() {
        let seq_matches: Vec<_> = all.iter().filter(|t| t.seq == Some(seq)).collect();
        match seq_matches.len() {
            1 => return Ok(Some(seq_matches[0].clone())),
            n if n > 1 => {
                return Err(TemperError::Vault(format!(
                    "ambiguous seq number '{slug_or_suffix}', matches: {}",
                    seq_matches
                        .iter()
                        .map(|t| t.slug.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )))
            }
            _ => {}
        }
    }
    Ok(None)
}
