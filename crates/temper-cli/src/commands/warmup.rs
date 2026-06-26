use serde::Serialize;
use temper_workflow::types::resource::{
    ResourceListParams, ResourceRow, ResourceSortField, SortOrder,
};

use crate::actions::runtime;
use crate::config::Config;
use crate::error::Result;
use crate::format::{render, OutputFormat};

/// Cap on the most-recent session body injected into the warmup primer. Bounds
/// how much of a long session note lands in a new agent's context window —
/// enough to carry the narrative, short of dominating the primer.
const MAX_SESSION_LINES: usize = 500;

/// How many recent sessions to surface in the warmup primer.
const RECENT_SESSION_LIMIT: usize = 5;

/// Structured session entry for JSON/Toon rendering.
#[derive(Debug, Serialize)]
pub struct WarmupSession {
    pub date: String,
    pub title: String,
}

/// In-progress task entry for JSON/Toon rendering.
#[derive(Debug, Serialize)]
pub struct WarmupTask {
    pub title: String,
    pub slug: String,
    pub mode: Option<String>,
    pub effort: Option<String>,
}

/// Full warmup result — serialized by `render()` for JSON and Toon outputs.
#[derive(Debug, Serialize)]
pub struct WarmupResult {
    pub project: String,
    pub recent_sessions: Vec<WarmupSession>,
    pub last_session_content: Option<String>,
    pub in_progress_tasks: Vec<WarmupTask>,
}

/// Run the warmup command — output a context primer for a new session.
///
/// Thin wrapper: all data collection lives in [`build_warmup_result`] so it is
/// testable from an external crate. This function only builds, renders, prints.
pub fn run(config: &Config, project: Option<&str>, format: OutputFormat) -> Result<()> {
    let result = build_warmup_result(config, project)?;
    let rendered = render(&result, format)?;
    println!("{rendered}");
    Ok(())
}

/// Collect everything the warmup primer reports: recent sessions, the most
/// recent session's body, and in-progress tasks.
///
/// Cloud-only: sessions are listed from the API (`client.resources().list`),
/// the last session's body is fetched via `client.resources().content`, and
/// tasks come from the cloud-backed [`crate::commands::task::load_tasks`]. The
/// local vault is a read-only projection cache that is empty/absent on a fresh
/// device, so a `fs::read_dir` scan would silently return nothing there.
///
/// Sessions + last-session content are gathered in a single `with_client`
/// closure (one runtime); tasks come from `load_tasks`, which manages its own
/// runtime, in a sequential (not nested) call.
pub fn build_warmup_result(config: &Config, project: Option<&str>) -> Result<WarmupResult> {
    let project_name = project.unwrap_or("general").to_string();

    let (recent_sessions, last_session_content) =
        collect_sessions_with_content(&project_name, RECENT_SESSION_LIMIT)?;
    let in_progress_tasks = collect_in_progress_tasks(config, &project_name);

    Ok(WarmupResult {
        project: project_name,
        recent_sessions,
        last_session_content,
        in_progress_tasks,
    })
}

/// List recent sessions for a context (most-recent-first, capped at `limit`)
/// and fetch the most-recent session's body, truncated to [`MAX_SESSION_LINES`].
///
/// Both reads share one tokio runtime via a single `with_client` closure.
fn collect_sessions_with_content(
    context: &str,
    limit: usize,
) -> Result<(Vec<WarmupSession>, Option<String>)> {
    let api_params = ResourceListParams {
        doc_type_name: Some("session".to_string()),
        context_name: Some(context.to_string()),
        sort: Some(ResourceSortField::Created),
        order: Some(SortOrder::Desc),
        limit: Some(limit as i64),
        ..Default::default()
    };

    runtime::with_client(move |client| {
        let api_params = api_params.clone();
        Box::pin(async move {
            let response = client
                .resources()
                .list(&api_params)
                .await
                .map_err(runtime::client_err_to_temper)?;

            let sessions: Vec<WarmupSession> = response.rows.iter().map(session_from_row).collect();

            // Most-recent session's body — fetched only when there is one.
            let last_session_content = match response.rows.first() {
                Some(row) => {
                    let resp = client
                        .resources()
                        .content(*row.id.as_uuid())
                        .await
                        .map_err(runtime::client_err_to_temper)?;
                    Some(truncate_lines(resp.markdown, MAX_SESSION_LINES))
                }
                None => None,
            };

            Ok((sessions, last_session_content))
        })
    })
}

/// Derive a [`WarmupSession`] from a resource row: the date is the row's
/// creation timestamp (`%Y-%m-%d`) and the title is the row's `title` column
/// (kept in sync with `temper-title` on every write).
fn session_from_row(row: &ResourceRow) -> WarmupSession {
    WarmupSession {
        date: row.created.format("%Y-%m-%d").to_string(),
        title: row.title.clone(),
    }
}

/// Truncate `content` to at most `max_lines` lines, joining with `\n`. Inputs
/// at or under the limit are returned unchanged.
fn truncate_lines(content: String, max_lines: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() > max_lines {
        lines[..max_lines].join("\n")
    } else {
        content
    }
}

/// Collect in-progress tasks for a project from the cloud-backed task list.
fn collect_in_progress_tasks(config: &Config, project: &str) -> Vec<WarmupTask> {
    let tasks = match crate::commands::task::load_tasks(config, Some(project), None) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    tasks
        .into_iter()
        .filter(|t| t.stage == "in-progress")
        .map(|t| WarmupTask {
            title: t.title,
            slug: t.slug,
            mode: t.mode,
            effort: t.effort,
        })
        .collect()
}
