use serde::Serialize;
use temper_core::types::config::CliSection;
use temper_workflow::operations::decorated_ref;
use temper_workflow::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

use crate::actions::runtime;
use crate::actions::types::TaskInfo;
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

/// What is waiting on the caller — counts only, never detail.
///
/// **Counts, because the primer's job is to say that something is waiting, not to answer it.** The
/// detail lives one command away: `temper invitations`, `temper admin requests list`,
/// `temper admin reviews list`.
///
/// **Only items with a terminal state belong here** — each of these disappears when the caller acts
/// on it. That is the line `kb_subscription_deliveries` already drew for external deliveries ("a
/// record of awareness, NOT an unfinished queue"), and it is why "a context was shared with your
/// team" is deliberately absent: an FYI with no terminal state, counted next to real obligations,
/// teaches a reader to ignore the count.
///
/// **This block is principal-scoped, not context-scoped** — unlike every other field on
/// [`WarmupResult`]. Orthogonal in meaning, but not in delivery: they ride one command, and
/// `--context` is fatal, so an unreadable context takes the whole primer down with it. See
/// `context_failure_hint` for the one case where that is actively perverse.
#[derive(Debug, Serialize)]
pub struct PendingSummary {
    /// Team invitations addressed to the caller's verified email.
    pub invitations: usize,
    /// Join requests awaiting an admin — `None` when the caller is not an instance admin.
    ///
    /// `None` and `Some(0)` are different facts and are never collapsed: `None` means nothing was
    /// read, `Some(0)` means an admin read an empty queue. Collapsing them would make the field
    /// useless to the only person it is for.
    pub join_requests: Option<usize>,
    /// Reconsideration requests awaiting an admin — same `None` semantics as `join_requests`.
    pub review_requests: Option<usize>,
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
    /// What is waiting on the caller — `None` when the read failed.
    ///
    /// Two nulls at different levels mean different things, and both are deliberate:
    /// `pending: null` is "could not read"; `pending.join_requests: null` is "not yours to see".
    /// `invitations: 0` would assert a reading that never happened; absence asserts nothing.
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

    let cloud = collect_standing_state(&context_ref, limits)?;
    let in_progress_tasks = collect_in_progress_tasks(config, &context_ref);

    Ok(WarmupResult {
        context: context_ref,
        active_goals: cloud.active_goals,
        active_goal_total: cloud.active_goal_total,
        in_progress_tasks,
        recent_sessions: cloud.recent_sessions,
        pending: cloud.pending,
    })
}

/// Everything one `with_client` closure gathers — the principal-scoped block and the
/// context-scoped reads together.
///
/// A struct rather than a tuple because these four are not interchangeable and one of them
/// (`active_goal_total`) is a bare `usize` that a positional return would happily let a caller
/// swap with a length.
struct CloudState {
    pending: Option<PendingSummary>,
    active_goals: Vec<WarmupGoal>,
    active_goal_total: usize,
    recent_sessions: Vec<WarmupSession>,
}

/// Count what is waiting on the caller, over a client the caller already has open.
///
/// The two operator counts are `Option` because their endpoint answers a non-admin with `403`, and
/// **the server's gate is the only copy of that rule.** Asking it is deliberate: a client-side
/// `Entitlements.is_admin` test would be a second copy, free to drift from the gate it predicts —
/// the same shape this repo retired when the client-side goal-status filter became a query
/// predicate (see [`goal_from_row`]).
async fn fetch_pending(client: &temper_client::TemperClient) -> Result<PendingSummary> {
    let invitations = client
        .teams()
        .list_my_invitations()
        .await
        .map_err(runtime::client_err_to_temper)?
        .len();

    let join_requests = admin_count(client.admin().list_requests().await.map(|r| r.len()))?;
    let review_requests = admin_count(client.admin().list_reviews().await.map(|r| r.len()))?;

    Ok(PendingSummary {
        invitations,
        join_requests,
        review_requests,
    })
}

/// Map an operator-queue read to a count the primer can report.
///
/// A `403` — in **either** of its arms — is the server saying "not yours to see", which is `None`.
/// Every other error propagates: a transport failure must never reach the caller wearing the same
/// `None` a refusal wears, because that `None` is a claim about the caller's *role*, not about the
/// network. Propagation out of *this* function is what stops `Forbidden` widening into "any failure
/// means not-an-admin"; what [`build_warmup_result`] then does with the error is a separate
/// decision, taken in [`degrade_pending`].
fn admin_count(
    result: std::result::Result<usize, temper_client::error::ClientError>,
) -> Result<Option<usize>> {
    use temper_client::error::ClientError;
    match result {
        Ok(n) => Ok(Some(n)),
        Err(ClientError::Forbidden | ClientError::ForbiddenDetail { .. }) => Ok(None),
        Err(e) => Err(runtime::client_err_to_temper(e)),
    }
}

/// Absorb a failed pending read: the block goes absent, the reason goes to stderr, and the rest of
/// the primer still prints.
///
/// Every other component of warmup already degrades — tasks fall back to an empty list on any
/// error. Letting the newest and least important component be the fatal one would mean a single
/// `500` on a secondary endpoint, or a CLI newer than the API it points at, turning the
/// session-start hook into no output at all.
fn degrade_pending(result: Result<PendingSummary>) -> Option<PendingSummary> {
    match result {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("warning: could not read what is pending for you: {e}");
            None
        }
    }
}

/// The one line printed to stderr when warmup fails on its `--context` while an invitation is
/// waiting.
///
/// **The trap this closes.** `warmup` requires a context and fails if the caller cannot read it.
/// Someone invited to a team so they can work in that team's context hits exactly that failure —
/// *because the invitation they have not accepted is what would grant them the context.* The one
/// fact that resolves the situation sits behind the situation.
///
/// The command still fails; that contract is not this feature's to rewrite, and a partial primer
/// would be worse than a refusal because every other field is context-scoped — `active_goals: []`
/// would read as "none" when it means "could not read". So the fix is a sentence on the way out,
/// pointing at `temper invitations`, which needs no context and is therefore reachable from inside
/// the failure it is printed during.
///
/// **It stays quiet when there is nothing to say** — zero invitations, or a pending block that
/// could not be read. A hint on every failed warmup is noise, and noise is how a real hint gets
/// ignored. The `None` case is the sharper one: not knowing is not evidence of an invitation.
fn context_failure_hint(pending: Option<&PendingSummary>) -> Option<String> {
    let n = pending.map(|p| p.invitations).filter(|n| *n > 0)?;
    let plural = if n == 1 { "invitation" } else { "invitations" };
    Some(format!(
        "! {n} team {plural} waiting on you — run `temper invitations`.\n  \
         Accepting one may be what grants you the context this command could not read."
    ))
}

/// Fetch what is pending, then active goals and recent session pointers — over **one** client
/// runtime.
///
/// Both context filters are the query's, not this function's: goals are narrowed by
/// `status = active` and sessions by doc type, each capped server-side. Nothing is re-tested after
/// it arrives.
///
/// **One `with_client`, not two.** Each call builds a tokio runtime *and* re-resolves the token
/// store, and `resolve_token_store` re-emits its near-expiry warning on every resolve — so a
/// separate closure for the pending read would print that warning three times per warmup instead of
/// twice. The pending read rides the closure that is already open.
///
/// **Pending is read FIRST**, and that ordering is load-bearing rather than incidental: it does not
/// depend on the context, so reading it first is what lets [`context_failure_hint`] survive the
/// context reads failing. It costs nothing — the same single request either way.
///
fn collect_standing_state(context_ref: &str, limits: WarmupLimits) -> Result<CloudState> {
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
            // First, and deliberately: this read is context-independent, so it is the only thing
            // still true if the context reads below fail.
            let pending = degrade_pending(fetch_pending(client).await);

            let goal_response = match client.resources().list(&goal_params).await {
                Ok(r) => r,
                Err(e) => {
                    // The error is surfaced unchanged rather than wrapped, so `NotFound` stays
                    // `NotFound` and the JSON error payload on stdout is untouched. The hint is an
                    // extra line on stderr, not a different failure.
                    if let Some(hint) = context_failure_hint(pending.as_ref()) {
                        eprintln!("{hint}");
                    }
                    return Err(runtime::client_err_to_temper(e));
                }
            };

            let displayed: Vec<WarmupGoal> = goal_response.rows.iter().map(goal_from_row).collect();
            // The true count of what is in force comes from the query's `total`, which
            // counts the filtered set and is unaffected by the page cap above. Deriving it
            // from `displayed.len()` instead would make every capped list look complete —
            // the exact confusion `active_goal_total` exists to prevent.
            let active_goal_total = goal_response.total as usize;

            let session_response = match client.resources().list(&session_params).await {
                Ok(r) => r,
                Err(e) => {
                    if let Some(hint) = context_failure_hint(pending.as_ref()) {
                        eprintln!("{hint}");
                    }
                    return Err(runtime::client_err_to_temper(e));
                }
            };
            let sessions: Vec<WarmupSession> =
                session_response.rows.iter().map(session_from_row).collect();

            Ok(CloudState {
                pending,
                active_goals: displayed,
                active_goal_total,
                recent_sessions: sessions,
            })
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

    use temper_client::error::ClientError;

    fn pending(invitations: usize) -> PendingSummary {
        PendingSummary {
            invitations,
            join_requests: None,
            review_requests: None,
        }
    }

    /// **Both `403` arms mean the same fact, and both must map to `None`.**
    ///
    /// `None` on an operator count is a claim about the caller's ROLE: "the server refused to tell
    /// me". `require_system_admin` returns a bare `ApiError::Forbidden` today, so bare
    /// `ClientError::Forbidden` is the arm that arrives — but this repo's own idiom for a refusal
    /// that names the capability it withheld is `ForbiddenDetail`, and this door has exactly one
    /// gate behind it. Matching only the bare arm would mean that the day that gate gains a message,
    /// every non-admin silently stops getting a pending block at all, and nothing fails.
    #[test]
    fn either_forbidden_arm_reads_as_not_an_admin() {
        assert_eq!(admin_count(Err(ClientError::Forbidden)).unwrap(), None);
        assert_eq!(
            admin_count(Err(ClientError::ForbiddenDetail {
                message: "system administration required".to_string(),
            }))
            .unwrap(),
            None,
        );
    }

    /// An admin reading an empty queue is `Some(0)`, which is a different fact from `None` and must
    /// never collapse into it. `None` says "not yours to see"; `Some(0)` says "yours, and empty".
    #[test]
    fn an_admin_reading_an_empty_queue_is_some_zero() {
        assert_eq!(admin_count(Ok(0)).unwrap(), Some(0));
        assert_eq!(admin_count(Ok(3)).unwrap(), Some(3));
    }

    /// **Every non-403 error still propagates out of this function.** This is what stops
    /// `Forbidden` widening into "any failure means not-an-admin" — a transport failure must never
    /// reach the caller wearing the `None` a refusal wears, because that `None` is a claim about
    /// the caller's role rather than about the network.
    ///
    /// What `build_warmup_result` then DOES with the propagated error is a separate decision (it
    /// degrades the whole block to absent). The two must not be conflated: absorbing here would
    /// make a 500 indistinguishable from a refusal.
    #[test]
    fn a_transport_failure_is_not_a_statement_about_the_caller() {
        assert!(admin_count(Err(ClientError::NotFound {
            message: "no such route".to_string(),
        }))
        .is_err());
        assert!(admin_count(Err(ClientError::TokenExpired)).is_err());
    }

    /// The hint names the count, and reaches for the one command that needs no context.
    #[test]
    fn the_hint_names_a_waiting_invitation() {
        let hint = context_failure_hint(Some(&pending(1))).expect("one invitation is worth saying");
        assert!(hint.contains("1 team invitation"), "{hint}");
        assert!(
            hint.contains("temper invitations"),
            "the way out must be reachable from inside the failure: {hint}"
        );
    }

    #[test]
    fn the_hint_is_plural_when_it_should_be() {
        let hint = context_failure_hint(Some(&pending(3))).unwrap();
        assert!(hint.contains("3 team invitations"), "{hint}");
    }

    /// **Both silences.** A hint that fires on every failed warmup is noise, and noise is how a
    /// real hint gets ignored — so zero invitations says nothing, and a pending block that could
    /// not be read says nothing either. The second is the sharper case: absence of knowledge is not
    /// evidence of an invitation, and guessing here would print the hint to everyone whose network
    /// blipped.
    #[test]
    fn the_hint_stays_quiet_when_there_is_nothing_to_say() {
        assert!(context_failure_hint(Some(&pending(0))).is_none());
        assert!(context_failure_hint(None).is_none());
    }

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
