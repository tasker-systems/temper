use serde::Serialize;
use temper_core::types::config::CliSection;
use temper_workflow::operations::decorated_ref;
use temper_workflow::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

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
///
/// Sent as the list endpoint's `status` filter rather than compared client-side. That
/// filter is validated against the goal schema's enum on both sides, so a typo here is a
/// 400 naming the legal values, not a primer that silently reports no goals as standing.
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
/// Both filters are the query's, not this function's: goals are narrowed by
/// `status = active` and sessions by doc type, each capped server-side. Nothing is
/// re-tested after it arrives.
///
/// Returns `(displayed_goals, total_active_goals, sessions)`.
fn collect_standing_state(
    context_ref: &str,
    limits: WarmupLimits,
) -> Result<(Vec<WarmupGoal>, usize, Vec<WarmupSession>)> {
    // Goals: the server filters to `status = active` and `total` counts the *filtered*
    // set, so the page cap is safe — it bounds what is displayed without touching what is
    // counted. This asked for every goal unbounded (`limit: None`, `meta_only`) while the
    // status test ran client-side, because a cap would then have truncated *before*
    // filtering and under-reported what is in force. That is no longer the trade: the
    // filter moved into the query, and with it the reason to over-fetch.
    let goal_params = ResourceListParams {
        doc_type_name: Some("goal".to_string()),
        context_ref: Some(context_ref.to_string()),
        status: Some(GOAL_STATUS_ACTIVE.to_string()),
        sort: Some(ResourceSortField::Updated),
        order: Some(SortOrder::Desc),
        limit: Some(limits.goals as i64),
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
                .list(&goal_params)
                .await
                .map_err(runtime::client_err_to_temper)?;

            let displayed: Vec<WarmupGoal> = goal_response.rows.iter().map(goal_from_row).collect();
            // The true count of what is in force comes from the query's `total`, which
            // counts the filtered set and is unaffected by the page cap above. Deriving it
            // from `displayed.len()` instead would make every capped list look complete —
            // the exact confusion `active_goal_total` exists to prevent.
            let active_goal_total = goal_response.total as usize;

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

/// Derive a [`WarmupGoal`] from a goal row, computing the decorated ref the same way
/// `resource list` renders it.
///
/// The row is already known to be standing: the query asked for `status = active`, so
/// there is no client-side status test to apply here.
///
/// There used to be one. This read deliberately compared `managed_meta["temper-status"]`
/// itself because `ResourceListParams` carried no `status` field at all, which made
/// `resource list --status active` return every row and accept values outside the enum —
/// under that flag, *presence* was the lie. Task
/// `019fa607-25f3-7bd0-88b4-ab8b7844225f` (PR #564) closed it: the filter now rides into
/// `filtered_visible_page` as a real predicate over the `kb_resource_workflow_props`
/// pivot, with the value rejected against the goal schema's enum receive-side. Asking the
/// query is now both correct and the only copy of the rule.
fn goal_from_row(row: &temper_core::types::resource_view::ResourceView) -> WarmupGoal {
    WarmupGoal {
        title: row.title.clone(),
        r#ref: decorated_ref(&row.title, row.id),
    }
}

/// Derive a [`WarmupSession`] from a resource row: the date is the row's
/// creation timestamp (`%Y-%m-%d`) and the title is the row's `title` column
/// (kept in sync with `temper-title` on every write).
fn session_from_row(row: &temper_core::types::resource_view::ResourceView) -> WarmupSession {
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

/// **Retired here, as a named remainder rather than a silent gap**:
/// `only_status_active_counts_as_standing`, which pinned the client-side rule that only
/// `temper-status = active` reads as standing (and that a goal with no status does not).
/// The rule did not disappear — it moved into the query as `status = active`, so there is
/// no longer a local predicate to unit-test. Its witness moved down the stack with it:
/// `warmup_reports_only_active_goals` in `tests/e2e/tests/cloud_warmup_e2e_test.rs` seeds
/// goals across every status value and drives a real server, so it now exercises the
/// filter that actually runs instead of a second copy of the rule.
#[cfg(test)]
mod tests {
    use super::*;

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

    /// The value sent as the list endpoint's `status` filter must be one the goal schema
    /// actually declares. A typo would not fail loudly at this layer on its own — it would
    /// ride out as a filter value — so pin it against the same schema the server validates
    /// against, rather than against a second hand-written list of statuses.
    #[test]
    fn the_active_filter_value_is_a_status_the_schema_declares() {
        temper_workflow::schema::validate_goal_status(GOAL_STATUS_ACTIVE)
            .expect("GOAL_STATUS_ACTIVE must be a status the goal schema declares");
    }
}
