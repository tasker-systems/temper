use temper_workflow::operations::sluggify;
use temper_workflow::types::resource::{
    ResourceListParams, ResourceRow, ResourceSortField, SortOrder,
};

use crate::actions::runtime;
use crate::actions::types::TaskInfo;
use crate::config::Config;
use crate::error::{Result, TemperError};

/// Per-context cap on the number of tasks fetched by [`load_tasks`]. Tasks per
/// context are bounded in practice, so this generous cap avoids pagination
/// while protecting against a runaway response. If a context ever returns
/// exactly this many rows the result is assumed truncated and a warning is
/// emitted (no-silent-caps).
const TASK_LIST_LIMIT: i64 = 1000;

/// Load all tasks, optionally scoped to a context.
///
/// Cloud-only: tasks are listed from the API via `client.resources().list` (full
/// rows), not by scanning the local vault. The local vault is a read-only
/// projection cache that is empty/absent on a fresh device, so a disk scan would
/// silently return nothing there.
///
/// Identity (`title`) and the workflow projections (`stage`/`mode`/`effort`/`seq`)
/// come from the resource row's top-level columns; the `slug` is title-derived
/// (`sluggify`) for suffix matching in `find_task`. `context` is stamped from the
/// query scope. (Post-Phase-2 the managed_meta tier is Property-only and no longer
/// carries identity; the full field projection + ref-based `find_task` is task
/// 019f3d55.)
///
/// The function is synchronous: `runtime::with_client` builds its own tokio
/// runtime, and all callers (`warmup::collect_in_progress_tasks`, `find_task`)
/// are synchronous and not already inside a runtime.
///
/// Per-context results are capped (see `TASK_LIST_LIMIT`); hitting the cap
/// emits a `tracing::warn!` rather than silently dropping the overflow.
pub fn load_tasks(config: &Config, context: Option<&str>) -> Result<Vec<TaskInfo>> {
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
        // The list API addresses contexts by ref (Decision 1): bare names are
        // rejected. Decorate an owner-less context name as `@me/<name>`; a ref that
        // already carries an `@`/`+` sigil (or is a UUID) passes through untouched.
        let is_ref =
            ctx.starts_with('@') || ctx.starts_with('+') || uuid::Uuid::parse_str(&ctx).is_ok();
        let context_ref = if is_ref {
            ctx.clone()
        } else {
            format!("@me/{ctx}")
        };
        let api_params = ResourceListParams {
            doc_type_name: Some("task".to_string()),
            context_ref: Some(context_ref),
            sort: Some(ResourceSortField::Seq),
            order: Some(SortOrder::Asc),
            limit: Some(TASK_LIST_LIMIT),
            ..Default::default()
        };

        let ctx_for_query = ctx.clone();
        let response = runtime::with_client(move |client| {
            let api_params = api_params.clone();
            Box::pin(async move {
                client
                    .resources()
                    .list(&api_params)
                    .await
                    .map_err(runtime::client_err_to_temper)
            })
        })?;

        if response.rows.len() as i64 == TASK_LIST_LIMIT {
            tracing::warn!(
                context = %ctx_for_query,
                limit = TASK_LIST_LIMIT,
                "task list hit the per-context cap; tasks beyond the cap were not \
                 loaded (find_task / warmup may be incomplete for this context)"
            );
        }

        for row in response.rows {
            tasks.push(task_info_from_row(row, &ctx_for_query));
        }
    }

    tasks.sort_by_key(|t| t.seq.unwrap_or(u32::MAX));
    Ok(tasks)
}

/// Build a [`TaskInfo`] from a full resource row plus the context the listing was
/// scoped to. Identity `title` and the workflow projections (`stage`/`mode`/
/// `effort`/`seq`) are top-level row columns; `slug` is title-derived (`sluggify`)
/// for `find_task` suffix matching. `seq` is `i64` on the row and `u32` on
/// `TaskInfo`, so negative/out-of-range values clamp to `None` (unsequenced, sorts
/// last). `branch`/`pr` are not in the list projection today — they read as `None`
/// here (restored by task 019f3d55).
fn task_info_from_row(row: ResourceRow, context: &str) -> TaskInfo {
    TaskInfo {
        id: row.id,
        slug: sluggify(&row.title),
        title: row.title,
        context: context.to_string(),
        stage: row.stage.unwrap_or_default(),
        mode: row.mode,
        effort: row.effort,
        seq: row.seq.and_then(|s| u32::try_from(s).ok()),
        branch: None,
        pr: None,
    }
}

/// Find a task by exact slug or unambiguous suffix match.
pub fn find_task(
    config: &Config,
    slug_or_suffix: &str,
    context: Option<&str>,
) -> Result<Option<TaskInfo>> {
    let all = load_tasks(config, context)?;
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
