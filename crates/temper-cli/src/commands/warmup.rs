use serde::Serialize;
use temper_core::types::config::CliSection;
use temper_workflow::operations::decorated_ref;
use temper_workflow::types::managed_meta::ManagedMeta;
use temper_workflow::types::resource::{
    ResourceDetail, ResourceListParams, ResourceRow, ResourceSortField, SortOrder,
};

use crate::actions::runtime;
use crate::config::Config;
use crate::error::Result;
use crate::format::{render, OutputFormat};

/// How many recent sessions to surface as pointers when unconfigured.
const DEFAULT_RECENT_SESSIONS: usize = 5;

/// How many active goals to list when unconfigured.
///
/// A cap is never silent: [`WarmupResult::active_goal_total`] always reports how many
/// active goals exist, so a reader can tell a short list from a truncated one.
const DEFAULT_ACTIVE_GOALS: usize = 20;

/// `temper-status` value that marks a goal as standing.
const GOAL_STATUS_ACTIVE: &str = "active";

/// A pointer to a recent session — date and title, never a body.
///
/// A title is a pointer; a body is a claim. Bodies are deliberately absent from this
/// primer: see [`WarmupResult`].
#[derive(Debug, Serialize)]
pub struct WarmupSession {
    pub date: String,
    pub title: String,
}

/// An active goal — what is in force, addressable but not expanded.
#[derive(Debug, Serialize)]
pub struct WarmupGoal {
    pub title: String,
    /// Decorated `sluggify(title)-<uuid>` ref — paste-able into `resource show`.
    pub r#ref: String,
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
///
/// **Standing state first, pointers last.** Field order is the primer's argument: what
/// is in force (goals), what is being worked (tasks), and only then what recently
/// happened (session titles). The primer carries **no prose**.
///
/// It previously led with `last_session_content` — the whole previous session note,
/// capped at 500 lines. That was dropped rather than shrunk. Several sessions run
/// concurrently on different machines, so "the last session" is frequently a sibling's:
/// the primer asserted a relevance it could not establish, with the authority of being
/// the only prose in the payload, and its confidence scaled with how well the sibling
/// wrote. Titles let a reader recognise which arc is theirs and go read it deliberately.
#[derive(Debug, Serialize)]
pub struct WarmupResult {
    /// The context ref this primer covers (`@owner/slug` or a UUID).
    pub context: String,
    /// Active goals, capped at the configured display limit.
    pub active_goals: Vec<WarmupGoal>,
    /// How many active goals exist in total, so a capped list is never mistaken for a
    /// complete one.
    pub active_goal_total: usize,
    pub in_progress_tasks: Vec<WarmupTask>,
    /// Recent session pointers — titles and dates only.
    pub recent_sessions: Vec<WarmupSession>,
}

/// Display limits for the primer, resolved once by [`resolve_limits`].
#[derive(Debug, Clone, Copy)]
pub struct WarmupLimits {
    pub sessions: usize,
    pub goals: usize,
}

/// Resolve display limits with the CLI's standard precedence: flag → `[cli]` config →
/// default. (No env layer: these are presentation counts, not deployment-varying
/// settings like `format`/`color`.)
///
/// Pure over its inputs so the precedence is testable without touching a config file;
/// [`run`] does the loading.
pub fn resolve_limits(
    cli_section: &CliSection,
    sessions: Option<usize>,
    goals: Option<usize>,
) -> WarmupLimits {
    WarmupLimits {
        sessions: sessions
            .or(cli_section.warmup_sessions)
            .unwrap_or(DEFAULT_RECENT_SESSIONS),
        goals: goals
            .or(cli_section.warmup_goals)
            .unwrap_or(DEFAULT_ACTIVE_GOALS),
    }
}

/// Run the warmup command — output a context primer for a new session.
///
/// Thin wrapper: all data collection lives in [`build_warmup_result`] so it is
/// testable from an external crate. This function only builds, renders, prints.
pub fn run(
    config: &Config,
    context: &str,
    sessions: Option<usize>,
    goals: Option<usize>,
    format: OutputFormat,
) -> Result<()> {
    let global_cfg = temper_core::types::config::load_config().unwrap_or_default();
    let limits = resolve_limits(&global_cfg.cli, sessions, goals);
    let result = build_warmup_result(config, context, limits)?;
    let rendered = render(&result, format)?;
    println!("{rendered}");
    Ok(())
}

/// Collect everything the warmup primer reports: active goals, in-progress tasks, and
/// recent session pointers.
///
/// Cloud-only: goals and sessions are listed from the API and tasks come from the
/// cloud-backed [`crate::commands::task::load_tasks`]. The local vault is a read-only
/// projection cache that is empty/absent on a fresh device, so a `fs::read_dir` scan
/// would silently return nothing there. Reading live is what settles drift between
/// machines — an on-disk copy is an offline cache, not a source.
///
/// Goals and sessions are gathered in a single `with_client` closure (one runtime);
/// tasks come from `load_tasks`, which manages its own runtime, in a sequential (not
/// nested) call.
///
/// `context` is required rather than defaulted. There is no defensible default: the
/// previous hardcoded `"general"` is a bare name the API rejects outright ("bare names
/// are not addressable"), and no context name is guaranteed to exist for a given
/// principal — a missing one is a hard 404. A clap-level "required" beats a runtime 404
/// naming a context the caller never chose.
pub fn build_warmup_result(
    config: &Config,
    context: &str,
    limits: WarmupLimits,
) -> Result<WarmupResult> {
    let context_ref = context.to_string();

    let (active_goals, active_goal_total, recent_sessions) =
        collect_standing_state(&context_ref, limits)?;
    let in_progress_tasks = collect_in_progress_tasks(config, &context_ref);

    Ok(WarmupResult {
        context: context_ref,
        active_goals,
        active_goal_total,
        in_progress_tasks,
        recent_sessions,
    })
}

/// Fetch active goals and recent session pointers over one client runtime.
///
/// Returns `(displayed_goals, total_active_goals, sessions)`.
fn collect_standing_state(
    context_ref: &str,
    limits: WarmupLimits,
) -> Result<(Vec<WarmupGoal>, usize, Vec<WarmupSession>)> {
    // Goals: `limit: None` means "no cap" (the server returns every matching row — the
    // same thing `resource list --all` asks for). Unbounded is deliberate: the status
    // filter runs client-side, so a page cap here would truncate *before* filtering and
    // silently under-report what is in force.
    let goal_params = ResourceListParams {
        doc_type_name: Some("goal".to_string()),
        context_ref: Some(context_ref.to_string()),
        sort: Some(ResourceSortField::Updated),
        order: Some(SortOrder::Desc),
        limit: None,
        meta_only: Some(true),
        ..Default::default()
    };

    let session_params = ResourceListParams {
        doc_type_name: Some("session".to_string()),
        context_ref: Some(context_ref.to_string()),
        sort: Some(ResourceSortField::Created),
        order: Some(SortOrder::Desc),
        limit: Some(limits.sessions as i64),
        ..Default::default()
    };

    runtime::with_client(move |client| {
        let goal_params = goal_params.clone();
        let session_params = session_params.clone();
        Box::pin(async move {
            let goal_response = client
                .resources()
                .list_meta(&goal_params)
                .await
                .map_err(runtime::client_err_to_temper)?;

            let active: Vec<WarmupGoal> = goal_response
                .rows
                .iter()
                .filter(|detail| is_active_goal(detail))
                .map(goal_from_detail)
                .collect();
            let active_goal_total = active.len();
            let displayed = active.into_iter().take(limits.goals).collect();

            let session_response = client
                .resources()
                .list(&session_params)
                .await
                .map_err(runtime::client_err_to_temper)?;
            let sessions: Vec<WarmupSession> =
                session_response.rows.iter().map(session_from_row).collect();

            Ok((displayed, active_goal_total, sessions))
        })
    })
}

/// Whether a goal row is standing.
///
/// Status truth is `managed_meta["temper-status"]`, read here rather than asked of the
/// list endpoint on purpose: `ResourceListParams` carries **no** `status` field, so
/// `resource list --status active` returns every row and accepts values outside the
/// enum without error. Under that flag, *presence* is the lie — a row it returns is no
/// evidence at all of being active. (The flag's own fix is task
/// `019fa607-25f3-7bd0-88b4-ab8b7844225f`; this read does not depend on it landing.)
///
/// A goal with no `managed_meta` is **not** treated as active: an unknown status must
/// not enter a primer claiming to say what is in force.
fn is_active_goal(detail: &ResourceDetail) -> bool {
    status_is_active(&detail.managed_meta)
}

/// The status predicate itself, over just the meta tier.
///
/// Split from [`is_active_goal`] so the rule is testable without standing up a whole
/// `ResourceDetail` — the judgment worth pinning is "which statuses count", not the
/// field access.
fn status_is_active(managed_meta: &Option<ManagedMeta>) -> bool {
    managed_meta
        .as_ref()
        .and_then(|meta| meta.status.as_deref())
        .is_some_and(|status| status == GOAL_STATUS_ACTIVE)
}

/// Derive a [`WarmupGoal`] from a meta row, computing the decorated ref the same way
/// `resource list` renders it.
fn goal_from_detail(detail: &ResourceDetail) -> WarmupGoal {
    WarmupGoal {
        title: detail.row.title.clone(),
        r#ref: decorated_ref(&detail.row.title, detail.row.id),
    }
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

/// Collect in-progress tasks for a context from the cloud-backed task list.
fn collect_in_progress_tasks(config: &Config, context_ref: &str) -> Vec<WarmupTask> {
    let tasks = match crate::commands::task::load_tasks(config, Some(context_ref)) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use temper_workflow::types::managed_meta::ManagedMeta;

    fn cli_section(sessions: Option<usize>, goals: Option<usize>) -> CliSection {
        CliSection {
            warmup_sessions: sessions,
            warmup_goals: goals,
            ..Default::default()
        }
    }

    #[test]
    fn limits_fall_back_to_defaults_when_nothing_is_set() {
        let limits = resolve_limits(&cli_section(None, None), None, None);
        assert_eq!(limits.sessions, DEFAULT_RECENT_SESSIONS);
        assert_eq!(limits.goals, DEFAULT_ACTIVE_GOALS);
    }

    #[test]
    fn config_overrides_defaults() {
        let limits = resolve_limits(&cli_section(Some(2), Some(3)), None, None);
        assert_eq!(limits.sessions, 2);
        assert_eq!(limits.goals, 3);
    }

    #[test]
    fn flag_outranks_config() {
        let limits = resolve_limits(&cli_section(Some(2), Some(3)), Some(9), Some(8));
        assert_eq!(
            limits.sessions, 9,
            "--sessions must outrank cli.warmup_sessions"
        );
        assert_eq!(limits.goals, 8, "--goals must outrank cli.warmup_goals");
    }

    /// Status truth is `managed_meta["temper-status"]`. A goal carrying no managed_meta
    /// is NOT active: an unknown status must not enter a primer that claims to report
    /// what is in force.
    #[test]
    fn only_status_active_counts_as_standing() {
        let with_status = |status: Option<&str>| ManagedMeta {
            status: status.map(str::to_string),
            ..Default::default()
        };

        assert!(status_is_active(&Some(with_status(Some("active")))));
        assert!(!status_is_active(&Some(with_status(Some("completed")))));
        assert!(!status_is_active(&Some(with_status(Some("cancelled")))));
        assert!(!status_is_active(&Some(with_status(Some("paused")))));
        assert!(
            !status_is_active(&Some(with_status(None))),
            "a goal with no status must not read as active"
        );
        assert!(
            !status_is_active(&None),
            "a goal with no managed_meta at all must not read as active"
        );
    }
}
