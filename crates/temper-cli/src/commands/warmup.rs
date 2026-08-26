use serde::Serialize;
use temper_core::context_ref::{parse_context_ref, ContextOwnerRef, ContextRef};
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
    /// The team whose pending invitation would grant the context this run asked for — `None`
    /// when nothing establishes that relation.
    ///
    /// **Not part of the primer's answer, so not serialized.** Every other field here is a fact
    /// about the caller; this one is a fact about the caller *and this invocation's `--context`*,
    /// and it exists for exactly one reader: `context_failure_hint`, deciding whether it has
    /// grounds to say that an invitation explains the failure. Putting it in the JSON would
    /// publish a context-relative answer inside a principal-scoped block.
    ///
    /// `Some` only when the server was asked about a specific team **and** answered that the
    /// caller holds an invitation to it. A personal context, a UUID ref, an unrelated team, or a
    /// team the caller holds nothing for all leave it `None` — the states differ in why, and none
    /// of them establishes a cause.
    #[serde(skip)]
    pub granting_team: Option<String>,
    /// Join requests awaiting an admin — `None` when the queue was **not read**.
    ///
    /// `None` and `Some(0)` are different facts and are never collapsed: `Some(0)` means an admin
    /// read an empty queue, `None` means nothing was read at all. Collapsing them would make the
    /// field useless to the only person it is for.
    ///
    /// Two things produce `None`, and they are not distinguished *in the field*: the caller is not
    /// an instance admin (every `403` arm), or the read failed. The second always prints its reason
    /// to stderr, so the stream — not the JSON — is what tells them apart. That is a deliberate
    /// trade: the alternative was losing the whole block, invitation count included, whenever one
    /// secondary queue was unavailable.
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
/// **All three reads are count-shaped.** They were not: each fetched full rows (handles, emails,
/// messages, and for invitations the redemption `token` — a bearer capability) so that `.len()`
/// could be taken, on a command wired into the session-start hook. Reporting how many things await
/// someone does not require transferring them, and it certainly does not require moving their
/// credentials to do it.
///
/// Still three round trips, and two of them are still a refusal for every non-admin. That half is
/// by design and stays: a caller's standing is the server's to state, and you cannot learn that
/// someone is not an admin without asking. What is gone is the payload — three integers now, and
/// no `token` on the wire at all.
///
/// `requested_team` exists for one reason: when this run's `--context` names a team, the invitation
/// count is asked a second question in the same round trip — *how many of these are to that team?*
/// That answer is the only thing that entitles [`context_failure_hint`] to say an invitation
/// explains a context failure.
async fn fetch_pending(
    client: &temper_client::TemperClient,
    requested_team: Option<&str>,
) -> Result<PendingSummary> {
    let counts = client
        .teams()
        .count_my_invitations(requested_team)
        .await
        .map_err(runtime::client_err_to_temper)?;

    // The operator queues degrade to "not read" independently. Only the invitation read above is
    // allowed to take the whole block down with it, because that count is what this block is FOR —
    // if it is unknown there is nothing worth reporting, and the hint has nothing to say.
    Ok(PendingSummary {
        invitations: counts.count.max(0) as usize,
        // `matching` is `Some` only when a team was named, and `> 0` only when the server found
        // one. Both conditions are the server's answer, not a client-side guess about it.
        granting_team: requested_team
            .filter(|_| counts.matching.is_some_and(|m| m > 0))
            .map(str::to_owned),
        join_requests: operator_count(client.admin().count_requests().await, "join request"),
        review_requests: operator_count(client.admin().count_reviews().await, "reconsideration"),
    })
}

/// The team a context ref is owned by, when it names one.
///
/// Parsing is [`parse_context_ref`]'s job, not this module's: `@me/x`, `@handle/x`, `+team/x` and
/// a bare UUID are one grammar with one parser, and a second copy here would be free to disagree
/// with the one the server resolves against. Every non-team form yields `None`, which is the
/// honest answer — **no team invitation can grant a personal context**, and a UUID names a context
/// whose owner this command could not read, which is why it is failing.
fn requested_team(context_ref: &str) -> Option<String> {
    match parse_context_ref(context_ref) {
        Ok(ContextRef::OwnerSlug {
            owner: ContextOwnerRef::Team(slug),
            ..
        }) => Some(slug),
        _ => None,
    }
}

/// Map an operator-queue read to a count the primer can report.
///
/// A `403` — in **any** of its three arms — is the server saying "not yours to see", which is
/// `None`. Three, not two: `SystemAccessRequired` is checked before `ForbiddenDetail` and
/// `Forbidden` and does not carry "Forbidden" in its name, which is how it was missed once. It is
/// the arm the not-yet-admitted principal meets, and therefore the only one the invitee ever sees.
/// Every other error propagates: a transport failure must never reach the caller wearing the same
/// `None` a refusal wears, because that `None` is a claim about the caller's *role*, not about the
/// network. Propagation out of *this* function is what stops `Forbidden` widening into "any failure
/// means not-an-admin"; what [`build_warmup_result`] then does with the error is a separate
/// decision, taken in [`degrade_pending`].
fn admin_count(
    result: std::result::Result<i32, temper_client::error::ClientError>,
) -> Result<Option<usize>> {
    use temper_client::error::ClientError;
    match result {
        Ok(n) => Ok(Some(n.max(0) as usize)),
        Err(
            ClientError::Forbidden
            | ClientError::ForbiddenDetail { .. }
            | ClientError::SystemAccessRequired(_),
        ) => Ok(None),
        Err(e) => Err(runtime::client_err_to_temper(e)),
    }
}

/// [`admin_count`], with a failed read degraded to "not read" and the reason put on stderr.
///
/// The split is deliberate: `admin_count` keeps the strict mapping so a transport failure can never
/// silently wear a refusal's `None`, and this wrapper decides — separately, and out loud — that a
/// primer should not lose the rest of its answer over one secondary queue.
///
/// The concrete case is version skew, which this change re-creates for a fresh window exactly as
/// the one that introduced these queues did: a CLI that knows `/api/access/admin/requests/count`
/// pointed at an API serving only the uncounted list gets a `404`, and without this the caller's
/// own invitation count — read successfully, moments earlier — would be thrown away on every
/// session start.
fn operator_count(
    result: std::result::Result<i32, temper_client::error::ClientError>,
    queue: &str,
) -> Option<usize> {
    match admin_count(result) {
        Ok(count) => count,
        Err(e) => {
            eprintln!("warning: could not read the {queue} queue: {e}");
            None
        }
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
///
/// # Two lines, and the second one has to be earned
///
/// - the **waiting line** reports what is waiting. It is a fact about the caller, true regardless
///   of why this command failed, and it is what puts `temper invitations` in front of a reader who
///   cannot reach anything else.
/// - the **grant line** says an invitation explains *this* failure. That is a claim about cause,
///   and it is printed only when [`PendingSummary::granting_team`] carries the server's answer
///   that the caller holds an invitation to the very team that owns the requested context.
///
/// This shipped saying the second thing whenever the first was true, hedged to "may be what
/// grants you" — so `temper warmup --context @me/typo` told a reader that a team invitation might
/// explain a *personal* context, which no team invitation can ever grant, and an invitation to
/// `+acme-eng` was offered as the explanation for `+other-team/roadmap`. The hedge was doing the
/// work of a check. It is read at the moment a reader has least context to doubt it, which is what
/// makes a wrong hint worse than no hint.
///
/// **The un-established case keeps the waiting line rather than falling silent.** A newcomer whose
/// one invitation is to a different team than the context they asked for still has an invitation
/// waiting, and on a context failure this stderr line is the *only* place it appears — the primer
/// never renders, because the command fails. Dropping to silence would protect them from a wrong
/// cause by also withholding the true fact.
fn context_failure_hint(pending: Option<&PendingSummary>) -> Option<String> {
    let pending = pending?;
    let n = pending.invitations;
    if n == 0 {
        return None;
    }

    let plural = if n == 1 { "invitation" } else { "invitations" };
    let waiting_line = format!("! {n} team {plural} waiting on you — run `temper invitations`.");

    let Some(team) = pending.granting_team.as_deref() else {
        return Some(waiting_line);
    };
    Some(format!(
        "{waiting_line}\n  \
         Accepting your invitation to +{team} grants you the context this command could not read."
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
/// depend on the context being *readable*, so reading it first is what lets
/// [`context_failure_hint`] survive the context reads failing. It costs nothing — the same single
/// request either way.
///
/// It does now consult the context *ref*, which is not the same thing: [`requested_team`] is pure
/// string parsing over the argument the caller typed, needing no server round trip and no
/// permission to read anything. The invitation count carries that team along and comes back saying
/// whether one of the caller's invitations is to it.
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

    // Parsed once, here, where the ref the caller typed is still in hand.
    let requested_team = requested_team(context_ref);

    runtime::with_client(move |client| {
        let goal_params = goal_params.clone();
        let session_params = session_params.clone();
        let requested_team = requested_team.clone();
        Box::pin(async move {
            // First, and deliberately: this read does not need the context to be readable, so it
            // is the only thing still true if the context reads below fail.
            let pending = degrade_pending(fetch_pending(client, requested_team.as_deref()).await);

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
/// **Named remainder — what these tests do not reach.** `fetch_pending`'s behaviour under
/// CLI/API version skew (invitations read, an operator queue `404`s, block survives with that
/// field `None`) is asserted only at the `operator_count` seam. Driving the whole function through
/// that state needs a server that serves one route and not another, which no fixture here builds.
/// The seam is pinned; the composition is not.
#[cfg(test)]
mod tests {
    use super::*;

    use temper_client::error::ClientError;

    /// Invitations waiting, and **nothing establishing that any of them grants the context** —
    /// the ordinary state, and the one the shipped hint got wrong.
    fn pending(invitations: usize) -> PendingSummary {
        PendingSummary {
            invitations,
            granting_team: None,
            join_requests: None,
            review_requests: None,
        }
    }

    /// Invitations waiting, one of them to the team that owns the requested context — the server
    /// having answered that the relation holds.
    fn pending_granting(invitations: usize, team: &str) -> PendingSummary {
        PendingSummary {
            invitations,
            granting_team: Some(team.to_string()),
            join_requests: None,
            review_requests: None,
        }
    }

    fn system_access_required() -> ClientError {
        ClientError::SystemAccessRequired(Box::new(temper_core::error::CliAccessDetails {
            email: None,
            display_name: None,
            refusal: None,
            request_url: None,
            cli_command: None,
        }))
    }

    /// **The third `403` arm — and the one that matters most, because it is the newcomer's.**
    ///
    /// `handlers::invitations::list_mine` is mounted in `auth_only_routes()`, while the two operator
    /// queues are in `gated_routes()` behind `require_system_access`. So a principal who has signed
    /// in but holds no approved standing reads their invitations fine and gets
    /// `403 SYSTEM_ACCESS_REQUIRED` from both queues — a *third* arm, checked before the other two
    /// in `http.rs`.
    ///
    /// Propagating it collapsed the whole block to `None`, which then silenced
    /// [`context_failure_hint`] — so the one population this feature exists for, the invited
    /// newcomer who cannot yet read the team's context, got no invitation count and no hint.
    /// Exactly the trap the hint was written to close, closed against them.
    #[test]
    fn no_system_access_reads_as_not_an_admin_too() {
        assert_eq!(admin_count(Err(system_access_required())).unwrap(), None);
    }

    /// **Every `403` arm means the same fact, and all of them must map to `None`.**
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

    /// **A failed operator read degrades to "not read" instead of taking the block with it.**
    ///
    /// The case this PR itself creates: `/api/access/admin/reviews` is a new route, so during any
    /// rollout window a CLI that knows it can be pointed at an API that does not serve it yet, and
    /// answers `404`. Before this, that discarded the caller's own invitation count — read
    /// successfully moments earlier — on every single session start.
    ///
    /// Note what is NOT claimed here: this pins `operator_count`'s return, not the end-to-end
    /// behaviour of `fetch_pending` under skew, which has no witness (see the test module's note).
    #[test]
    fn a_failed_operator_read_does_not_propagate() {
        assert_eq!(
            operator_count(
                Err(ClientError::NotFound {
                    message: "no such route".to_string(),
                }),
                "reconsideration",
            ),
            None,
        );
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

    /// **The defect this fixes, stated as a test.** An invitation waiting is not, on its own,
    /// an explanation for a context that could not be read — and the shipped hint said it was,
    /// hedged to "may be what grants you", on every failure where any invitation existed.
    ///
    /// `pending()` is the un-established state: invitations exist, nothing relates them to what
    /// was asked for. What must be absent is the CAUSAL sentence, not the notice — see
    /// `the_unexplained_case_still_says_what_is_waiting` for the half that must survive.
    #[test]
    fn no_cause_is_offered_for_a_context_nothing_was_shown_to_grant() {
        let hint = context_failure_hint(Some(&pending(1))).unwrap();
        assert!(
            !hint.contains("grants you the context"),
            "a cause must be established before it is named: {hint}"
        );
        assert!(
            !hint.contains("may be"),
            "and a hedge is not a substitute for the check: {hint}"
        );
    }

    /// **The other half, and the reason silence was the wrong answer.** A newcomer whose one
    /// invitation is to a different team than the context they asked for still has an invitation
    /// waiting — and when `warmup` fails on its context, this stderr line is the only place it
    /// appears, because the primer never renders. Refusing to name a cause must not turn into
    /// withholding the fact.
    #[test]
    fn the_unexplained_case_still_says_what_is_waiting() {
        let hint =
            context_failure_hint(Some(&pending(1))).expect("the notice survives the missing cause");
        assert!(hint.contains("1 team invitation waiting on you"), "{hint}");
        assert!(hint.contains("temper invitations"), "{hint}");
    }

    /// **When the relation IS established, the hint says so — definitely, and names it.**
    ///
    /// `granting_team` carries the server's answer that one of the caller's invitations is to the
    /// team owning the requested context. That is a checked cause, so the sentence is flat: no
    /// "may be", and the team the reader must accept an invitation to is named rather than left
    /// as "one" of an unnamed set.
    #[test]
    fn an_established_cause_is_named_without_a_hedge() {
        let hint = context_failure_hint(Some(&pending_granting(1, "acme-eng"))).unwrap();
        assert!(hint.contains("1 team invitation waiting on you"), "{hint}");
        assert!(
            hint.contains("Accepting your invitation to +acme-eng grants you the context"),
            "the established cause is stated definitely, and names which invitation: {hint}"
        );
        assert!(!hint.contains("may be"), "no hedge is needed now: {hint}");
    }

    #[test]
    fn the_hint_is_plural_when_it_should_be() {
        let hint = context_failure_hint(Some(&pending(3))).unwrap();
        assert!(hint.contains("3 team invitations"), "{hint}");

        // The waiting line counts everything waiting; the grant line names the ONE team that
        // explains this failure. Three invitations of which one grants the context is the case
        // where conflating the two numbers would misreport both.
        let granting = context_failure_hint(Some(&pending_granting(3, "acme-eng"))).unwrap();
        assert!(
            granting.contains("3 team invitations waiting"),
            "{granting}"
        );
        assert!(
            granting.contains("your invitation to +acme-eng"),
            "{granting}"
        );
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

    /// **Which refs can a team invitation possibly explain?** Only one shape: a context owned by
    /// a team. This is the test that decides whether the count read even asks the question, so
    /// the negative cases matter more than the positive one.
    ///
    /// `@me/...` and `@handle/...` are personal contexts — **no team invitation can ever grant
    /// one**, which is precisely the case the shipped hint got wrong. A bare UUID names a context
    /// whose owner this command could not read; that is why it is failing, so nothing here can
    /// establish a relation to it. An unparseable ref answers nothing rather than erroring: the
    /// command is already failing and the hint is not the place to re-report why.
    #[test]
    fn only_a_team_owned_context_can_be_granted_by_an_invitation() {
        assert_eq!(
            requested_team("+acme-eng/work"),
            Some("acme-eng".to_string())
        );

        assert_eq!(requested_team("@me/typo"), None, "a personal context");
        assert_eq!(requested_team("@someone/notes"), None, "someone else's");
        assert_eq!(
            requested_team("019fbb77-72a3-72e1-bbbd-13eb6aa64982"),
            None,
            "a UUID names a context whose owner could not be read"
        );
        assert_eq!(
            requested_team("not-a-ref"),
            None,
            "and garbage asks nothing"
        );
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
