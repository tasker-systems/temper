use serde::Serialize;
use temper_core::types::config::CliSection;
use temper_workflow::operations::decorated_ref;
use temper_workflow::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

use temper_client::error::ClientError;

use crate::actions::runtime;
use crate::actions::types::TaskInfo;
use crate::config::Config;
use crate::error::{Result, TemperError};
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

/// What is waiting on **you** — a queue, not a feed.
///
/// **Principal-scoped, unlike the rest of this primer.** Every other field here is scoped to
/// the `context` argument; these counts are scoped to the caller and span every team and
/// context they touch. Which context you are priming has no bearing on what you owe other
/// people.
///
/// **They are orthogonal in MEANING but not in DELIVERY, and the gap is worth knowing.** This
/// block rides a command whose context argument is fatal: ask for a context you cannot read and
/// `warmup` errors, so the block never prints — including in the ironic case where the
/// invitation you have not seen is the very thing that would grant you that context. Warmup
/// erroring on an unreadable context is its contract and this block does not get to change it,
/// so the surface that closes that gap is `temper invitations`, which needs no context at all. Keeping them in one payload is what
/// makes the queue arrive without anyone remembering to ask for it, which is the whole point —
/// a surface nobody runs is the out-of-band-notification problem wearing a new hat.
///
/// **Counts only, deliberately.** The detail lives behind `temper invitations` and
/// `temper admin requests list`. A primer that inlined the rows would grow without bound with
/// the thing it is least qualified to prioritize.
///
/// **Only items with a terminal state belong here.** Each of these disappears when the caller
/// acts on it. That is the line this repo already drew for external deliveries — an undisposed
/// row is "a record of awareness, NOT an unfinished queue"
/// (`migrations/20260819000030_kb_subscription_deliveries.sql`) — and it is why "a context was
/// shared with your team" is absent: it is an FYI with no terminal state, and mixing FYIs into
/// a count trains a reader to ignore the count.
#[derive(Debug, Serialize)]
pub struct PendingSummary {
    /// Team invitations addressed to you and still redeemable.
    pub invitations: usize,
    /// Join requests awaiting your review, or `None` when this is not yours to see.
    ///
    /// `None` and `Some(0)` are different facts and are never collapsed: `None` means the
    /// caller is not an instance admin and read nothing, `Some(0)` means an admin read an
    /// empty queue. The `None` is produced by the server's own admin gate refusing the
    /// operator surface — not by a client-side guess at who counts as an admin.
    pub join_requests: Option<usize>,
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
    /// What is waiting on the caller, or `None` when that could not be read.
    ///
    /// Principal-scoped — see [`PendingSummary`]. Absent rather than zeroed on failure: a zero
    /// asserts a reading that never happened, and the reason is written to stderr instead.
    pub pending: Option<PendingSummary>,
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

/// Collect everything the warmup primer reports: active goals, in-progress tasks, recent
/// session pointers, and what is waiting on the caller.
///
/// Cloud-only: goals and sessions are listed from the API and tasks come from the
/// cloud-backed [`crate::commands::task::load_tasks`]. The local vault is a read-only
/// projection cache that is empty/absent on a fresh device, so a `fs::read_dir` scan
/// would silently return nothing there. Reading live is what settles drift between
/// machines — an on-disk copy is an offline cache, not a source.
///
/// Goals, sessions and the pending block are gathered in a single `with_client` closure
/// (one runtime); tasks come from `load_tasks`, which manages its own runtime, in a
/// sequential (not nested) call. Keeping the pending read inside the existing closure is
/// deliberate — a third `with_client` is a third token-store resolve, and that re-emits the
/// near-expiry warning a third time.
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

    let cloud = match collect_cloud_state(&context_ref, limits) {
        Ok(cloud) => cloud,
        Err(failure) => {
            // The context read failed. What is waiting on the CALLER did not, and is very often
            // the way out of that failure — so it is said before the error, and the error itself
            // is surfaced unchanged.
            if let Some(hint) = failure.hint {
                crate::output::warning(hint);
            }
            return Err(failure.error);
        }
    };
    let in_progress_tasks = collect_in_progress_tasks(config, &context_ref);

    // stderr, so stdout stays a parseable primer. `degrade_pending` decided; this only writes.
    if let Some(message) = cloud.pending_warning {
        crate::output::warning(message);
    }

    Ok(WarmupResult {
        context: context_ref,
        active_goals: cloud.active_goals,
        active_goal_total: cloud.active_goal_total,
        in_progress_tasks,
        recent_sessions: cloud.recent_sessions,
        pending: cloud.pending,
    })
}

/// A context-scoped failure, carrying the principal-scoped line that outlived it.
///
/// The two scopes fail independently, so the error and the hint travel together rather than the
/// error erasing the hint. Printing happens at the caller — see [`pending_hint`].
struct CloudFailure {
    error: TemperError,
    hint: Option<String>,
}

/// Everything one open client is asked for, so the primer opens exactly one.
struct CloudState {
    active_goals: Vec<WarmupGoal>,
    active_goal_total: usize,
    recent_sessions: Vec<WarmupSession>,
    /// `None` when the pending read failed — see [`degrade_pending`].
    pending: Option<PendingSummary>,
    /// The stderr line the caller owes the reader when `pending` is `None`.
    pending_warning: Option<String>,
}

/// Read active goals, recent session pointers, and the pending block over ONE client.
///
/// Both list filters are the query's, not this function's: goals are narrowed by
/// `status = active` and sessions by doc type, each capped server-side. Nothing is re-tested
/// after it arrives.
///
/// **One client, three reads, two failure regimes.** Goals and sessions are the primer's
/// subject and stay fatal. The pending block is a passenger and degrades to absent — see
/// [`degrade_pending`] for why a passenger must not be able to abort the journey.
fn collect_cloud_state(
    context_ref: &str,
    limits: WarmupLimits,
) -> std::result::Result<CloudState, CloudFailure> {
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

    // `with_client`'s own failure (no runtime, no token) happens before any read, so there is no
    // hint to have gathered — hence the nested Result: the inner one carries a hint, the outer
    // one cannot.
    let outcome = runtime::with_client(move |client| {
        let goal_params = goal_params.clone();
        let session_params = session_params.clone();
        Box::pin(async move {
            // FIRST, deliberately. The pending read does not depend on the context, and reading it
            // before the context-scoped reads is what lets `pending_hint` survive their failure —
            // the case where the unaccepted invitation IS why the context cannot be read. Ordering
            // it here costs nothing: it is the same single request either way.
            let (pending, pending_warning) = degrade_pending(fetch_pending(client).await);
            let hint = pending_hint(pending.as_ref());

            // Goals and sessions are the primer's SUBJECT and stay fatal; the pending block is a
            // passenger and degrades. That asymmetry is the point — see `degrade_pending`.
            let read = async {
                let goal_response = client
                    .resources()
                    .list(&goal_params)
                    .await
                    .map_err(runtime::client_err_to_temper)?;

                let displayed: Vec<WarmupGoal> =
                    goal_response.rows.iter().map(goal_from_row).collect();
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

                Ok::<_, TemperError>(CloudState {
                    active_goals: displayed,
                    active_goal_total,
                    recent_sessions: sessions,
                    pending,
                    pending_warning,
                })
            }
            .await;

            Ok(read.map_err(|error| CloudFailure { error, hint }))
        })
    });

    match outcome {
        Ok(inner) => inner,
        Err(error) => Err(CloudFailure { error, hint: None }),
    }
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
///
/// The fetch and the filter are separate so the filter has a seam a test can reach: this
/// half needs a server, [`in_progress_tasks`] does not.
fn collect_in_progress_tasks(config: &Config, context_ref: &str) -> Vec<WarmupTask> {
    let tasks = match crate::commands::task::load_tasks(config, Some(context_ref)) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    in_progress_tasks(tasks)
}

/// Keep the tasks whose stage is `in-progress`, as [`WarmupTask`]s.
///
/// `TaskInfo::stage` is not a column read. It is
/// `ResourceView::managed_meta.stage` — `temper-stage` in the managed tier — routed through
/// `task_info_from_row`. `stage`/`mode`/`effort`/`seq` used to be hoisted flat onto the
/// retired `ResourceRow`, and this filter read them from there; dropping the hoist is
/// lossless only because `managed_meta` is non-`Option`, so a task with no stage still
/// reaches this predicate and is simply not `in-progress`.
fn in_progress_tasks(tasks: Vec<TaskInfo>) -> Vec<WarmupTask> {
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

/// The line worth saying even when the primer itself could not be built.
///
/// **The trap this exists for.** `warmup` requires a context and fails if the caller cannot read
/// it. Someone invited to a team so they can work in that team's context hits exactly that
/// failure — because the invitation they have not accepted is what would grant them the context.
/// The one fact that resolves the situation is otherwise hidden behind the situation.
///
/// So the count survives the failure as a stderr line. It is `None` when there is nothing to say:
/// zero invitations, or a pending block that could not be read. A hint that fires on every failed
/// warmup would be noise, and noise is how a real hint gets ignored.
///
/// Returns the message rather than printing it, like [`degrade_pending`] — the caller owns the
/// write.
fn pending_hint(pending: Option<&PendingSummary>) -> Option<String> {
    let count = pending.map(|p| p.invitations).filter(|n| *n > 0)?;
    let noun = if count == 1 {
        "invitation is"
    } else {
        "invitations are"
    };
    Some(format!(
        "{count} team {noun} waiting on you — run `temper invitations`. \
         Accepting one may be what grants you the context this command could not read."
    ))
}

/// Degrade a failed pending read to an absent block plus a warning, rather than to a failed
/// warmup.
///
/// **The primer's subject is the context; the pending block is a passenger.** Letting a passenger
/// abort the journey is the regression this exists to prevent: before this block existed, a 500
/// on one secondary endpoint could not stop `warmup` from printing goals, tasks and sessions, and
/// it must not start now. Version skew makes that concrete — a CLI newer than the API it is
/// pointed at would otherwise turn every session-start hook into no output at all.
///
/// Returning `None` is not the lie that returning `PendingSummary { invitations: 0, .. }` would
/// be. Zero asserts a reading that never happened; absence asserts nothing, and the warning says
/// why on stderr while stdout stays machine-parseable.
///
/// Shaped as `(value, Option<warning>)` rather than printing here, mirroring
/// `runtime::token_expiry_warning`: the decision is testable and the caller owns the write.
fn degrade_pending(result: Result<PendingSummary>) -> (Option<PendingSummary>, Option<String>) {
    match result {
        Ok(summary) => (Some(summary), None),
        Err(e) => (
            None,
            Some(format!(
                "could not read what is waiting on you: {e}. Run `temper invitations` directly."
            )),
        ),
    }
}

/// Fetch what is waiting on the caller: their pending invitations, and — only if they are an
/// instance admin — the join requests awaiting review.
///
/// **Takes the caller's client rather than building its own.** This used to hold a third
/// `runtime::with_client`, which is a third tokio runtime AND a third
/// `build_config_store_and_client` — and that last one is not free of observable effect:
/// `resolve_token_store` re-emits the near-expiry warning on every resolve, so a session inside
/// the one-hour window printed that warning three times per warmup instead of twice. Riding the
/// closure that is already open costs nothing and says it once fewer.
///
/// Only the admin read has a non-error absence, and it is a real answer from the server rather
/// than a swallowed failure (see [`join_request_count`]). A failure of either read is a failure
/// of this whole fetch, which [`degrade_pending`] then turns into an absent block rather than an
/// absent warmup.
async fn fetch_pending(client: &temper_client::TemperClient) -> Result<PendingSummary> {
    let invitations = client
        .teams()
        .list_my_invitations()
        .await
        .map_err(runtime::client_err_to_temper)?
        .len();

    let join_requests = join_request_count(client.admin().list_requests().await)?;

    Ok(PendingSummary {
        invitations,
        join_requests,
    })
}

/// Turn an admin-only pending-queue read into a count, distinguishing **"not yours to
/// see"** from **"yours, and empty"**.
///
/// `GET /api/access/admin/requests` is an operator-only surface: the route is mounted
/// unconditionally and the gate lives in the handler, which answers a non-admin with
/// `ApiError::Forbidden` (`temper_services::auth::require_system_admin`). So the server's
/// own gate is the authority on who may read this queue, and asking it is the ONLY copy of
/// that rule. A client-side `Entitlements.is_admin` test would be a second copy — able to
/// drift from the gate it predicts — which is the shape this repo has already retired once
/// (see `goal_from_row` on the client-side status filter that became a query predicate).
///
/// `Forbidden` therefore maps to `None`, and `None` is not `Some(0)`: an admin whose queue
/// is empty has *read* an empty queue, while a non-admin has read nothing at all. Collapsing
/// the two would make the field useless to the only person it is for.
///
/// **Every other error still propagates out of THIS function** — which is what keeps
/// `Forbidden` from widening into "any failure means not-an-admin". A network failure or an
/// expired token must never arrive at the caller wearing the same `None` a refusal wears,
/// because that `None` is a claim about the caller's ROLE.
///
/// It no longer follows that such an error fails warmup, and this comment used to say it did.
/// [`degrade_pending`] catches it one level up and turns it into an absent BLOCK plus a stderr
/// line, so the primer still prints. The distinction that survives is the one that matters
/// here: a transport failure never becomes `Some(0)`, and never becomes a field-level `None`
/// that would read as "you are not an admin".
///
/// Generic over the row type because the count is all this needs — pinning it to
/// `JoinRequestWithProfile` would buy nothing and cost every test a fifteen-field fixture.
fn join_request_count<T>(
    result: std::result::Result<Vec<T>, ClientError>,
) -> Result<Option<usize>> {
    match result {
        Ok(rows) => Ok(Some(rows.len())),
        // Only the bare `Forbidden` — the arm the handler's `require_system_admin` actually
        // emits. `ForbiddenDetail` is the capability-naming 403 a caller who already reads the
        // subject gets, which this operator-only route never issues; leaving it out of this arm
        // keeps the mapping to the refusal that is really on the wire.
        Err(ClientError::Forbidden) => Ok(None),
        Err(e) => Err(runtime::client_err_to_temper(e)),
    }
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

    /// A `ResourceView` carrying its workflow values where they actually live.
    ///
    /// Built field-by-field rather than from a `Default`, deliberately: `ResourceView` has
    /// no `Default`, and the reason is the point of this test — there is no top-level
    /// `stage`/`mode`/`effort`/`seq` to accidentally set, so the only way to give this view
    /// a stage is through `managed_meta`.
    fn view_with_stage(
        title: &str,
        stage: &str,
    ) -> temper_core::types::resource_view::ResourceView {
        use temper_core::types::ids::{ProfileId, ResourceId};
        use temper_core::types::managed_meta::ManagedMeta;

        temper_core::types::resource_view::ResourceView {
            id: ResourceId::from(uuid::Uuid::now_v7()),
            r#ref: String::new(),
            title: title.to_string(),
            origin_uri: String::new(),
            kb_context_id: None,
            context_name: None,
            context_slug: None,
            context_owner_ref: None,
            context_ref: None,
            cogmap_id: None,
            cogmap_name: None,
            doc_type_name: "task".to_string(),
            owner_handle: "me".to_string(),
            owner_profile_id: ProfileId::from(uuid::Uuid::nil()),
            originator_profile_id: ProfileId::from(uuid::Uuid::nil()),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            body_hash: None,
            ingest_state: None,
            body_storage: None,
            managed_meta: ManagedMeta {
                stage: Some(stage.to_string()),
                mode: Some("build".to_string()),
                effort: Some("small".to_string()),
                ..Default::default()
            },
            open_meta: None,
            content: None,
        }
    }

    /// **The warmup in-progress filter reads `managed_meta`, not a hoisted column.**
    ///
    /// `stage`/`mode`/`effort`/`seq` were flat fields on the retired `ResourceRow`, and this
    /// filter reached them there. They are not hoisted onto `ResourceView`; they live in
    /// `managed_meta` under their canonical `temper-*` names, and dropping the hoist is
    /// lossless only because `managed_meta` is non-`Option`. Nothing else in the repo read
    /// those four (verified: every other `.stage`/`.mode`/`.effort` in `crates/` is either
    /// `ManagedMeta` assembly in `readback`, a block ordinal, or `TaskInfo`'s own field), so
    /// this path is the whole surface of that change and it gets the whole witness.
    ///
    /// Both directions are asserted. The positive half alone would pass against a filter
    /// that admitted everything — which is exactly what a `stage` defaulting to `""` on a
    /// mis-wired read would NOT do, but a `stage` defaulting to `"in-progress"` would.
    #[test]
    fn in_progress_filter_reads_managed_meta_stage() {
        let rows = vec![
            view_with_stage("Live Task", "in-progress"),
            view_with_stage("Parked Task", "backlog"),
        ];
        let tasks: Vec<TaskInfo> = rows
            .into_iter()
            .map(|row| crate::actions::task::task_info_from_row(row, "@me/ctx"))
            .collect();

        // The value survived the trip from the managed tier onto `TaskInfo`.
        assert_eq!(tasks[0].stage, "in-progress");
        assert_eq!(tasks[1].stage, "backlog");

        let warm = in_progress_tasks(tasks);

        assert_eq!(
            warm.len(),
            1,
            "only the in-progress task warms up: {warm:?}"
        );
        assert_eq!(warm[0].title, "Live Task");
        // `mode` and `effort` came the same way, and they are what warmup actually prints.
        assert_eq!(warm[0].mode.as_deref(), Some("build"));
        assert_eq!(warm[0].effort.as_deref(), Some("small"));
    }

    /// A task with **no** stage in the managed tier is not in progress — and does not
    /// panic. `managed_meta` is always present; the values inside it are not.
    #[test]
    fn a_task_with_no_managed_stage_is_not_in_progress() {
        let mut row = view_with_stage("Stageless", "in-progress");
        row.managed_meta.stage = None;

        let task = crate::actions::task::task_info_from_row(row, "@me/ctx");
        assert_eq!(
            task.stage, "",
            "an absent stage reads as empty, not as a stage"
        );

        assert!(
            in_progress_tasks(vec![task]).is_empty(),
            "no stage is not `in-progress`"
        );
    }

    /// An admin reads the queue, so the count is a real reading of it.
    #[test]
    fn an_admin_gets_a_counted_review_queue() {
        let result = join_request_count(Ok(vec![(), ()])).expect("an admin read must succeed");
        assert_eq!(result, Some(2), "two pending requests must count as two");
    }

    /// **The distinction the field exists for.** A non-admin is refused the operator
    /// surface, and that refusal reads as an ABSENT section — never as an empty one.
    #[test]
    fn a_forbidden_read_is_an_absent_section_not_an_empty_one() {
        let result = join_request_count::<()>(Err(ClientError::Forbidden))
            .expect("a refusal is not a warmup failure");
        assert_eq!(
            result, None,
            "a non-admin has read nothing — that is None, not Some(0)"
        );
    }

    /// The other half of the same distinction, which the test above cannot prove alone: an
    /// admin whose queue happens to be empty has still *read* it. A collapsed implementation
    /// that returned `None` for both would pass the previous test and fail this one.
    #[test]
    fn an_admin_with_an_empty_queue_is_not_a_non_admin() {
        let result = join_request_count::<()>(Ok(vec![])).expect("an admin read must succeed");
        assert_eq!(
            result,
            Some(0),
            "an empty queue an admin CAN see is Some(0), distinct from None"
        );
    }

    /// **This is what keeps the arm above from being a swallowed-error path.** Only
    /// `Forbidden` means "not yours"; anything else is a real failure and must surface AS one.
    /// It does not end warmup — `degrade_pending` absorbs it — but it must not be laundered
    /// into a refusal on the way, because a refusal is a claim about who the caller is.
    #[test]
    fn a_transport_failure_is_not_mistaken_for_an_absent_section() {
        let result = join_request_count::<()>(Err(ClientError::TokenExpired));
        assert!(
            result.is_err(),
            "an expired token must surface as an error here — never as the `None` that means \
             `not an admin`. What warmup then DOES with that error is `degrade_pending`'s call"
        );
    }

    /// A successful read is carried through untouched, and says nothing on stderr.
    #[test]
    fn a_successful_pending_read_is_carried_through_without_a_warning() {
        let (value, warning) = degrade_pending(Ok(PendingSummary {
            invitations: 2,
            join_requests: Some(1),
        }));

        assert_eq!(value.expect("the block must survive").invitations, 2);
        assert!(warning.is_none(), "a success has nothing to warn about");
    }

    /// **A failed read must not fail the primer.** The block goes absent — never zero, which
    /// would assert a reading that never happened — and the reason goes to stderr.
    #[test]
    fn a_failed_pending_read_degrades_to_an_absent_block_and_warns() {
        let (value, warning) =
            degrade_pending(Err(crate::error::TemperError::Api("boom".to_string())));

        assert!(
            value.is_none(),
            "a failed read is an absent block, not a zero count"
        );
        assert!(
            warning.is_some_and(|w| w.contains("boom")),
            "the warning must carry the reason the read failed"
        );
    }

    /// The trap case: a waiting invitation is named even though the primer failed.
    #[test]
    fn a_waiting_invitation_is_named_when_the_primer_could_not_be_built() {
        let hint = pending_hint(Some(&PendingSummary {
            invitations: 1,
            join_requests: None,
        }))
        .expect("one waiting invitation is worth saying");

        assert!(hint.contains('1'), "the count is the point: {hint}");
        assert!(
            hint.contains("temper invitations"),
            "the hint must name the command that needs no context: {hint}"
        );
    }

    /// Plural reads as plural. A hint that says "1 invitations" undercuts itself.
    #[test]
    fn more_than_one_waiting_invitation_reads_as_plural() {
        let hint = pending_hint(Some(&PendingSummary {
            invitations: 3,
            join_requests: None,
        }))
        .expect("three waiting invitations are worth saying");

        assert!(hint.contains('3'), "got: {hint}");
        assert!(hint.contains("invitations"), "got: {hint}");
    }

    /// **Silence when there is nothing to say.** A hint on every failed warmup is noise, and
    /// noise is how a real hint gets ignored.
    #[test]
    fn nothing_waiting_produces_no_hint() {
        assert!(
            pending_hint(Some(&PendingSummary {
                invitations: 0,
                join_requests: Some(4),
            }))
            .is_none(),
            "zero invitations is nothing to say — join requests are not the caller's own trap"
        );
    }

    /// An unreadable pending block has nothing to assert either, and must not invent a count.
    #[test]
    fn an_unreadable_pending_block_produces_no_hint() {
        assert!(
            pending_hint(None).is_none(),
            "a block that could not be read cannot claim an invitation is waiting"
        );
    }

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
