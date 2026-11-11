use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Team role — strict hierarchy: Owner > Maintainer > Member > Watcher.
///
/// Maps directly to the `team_role` Postgres enum. Four roles is small enough
/// that explicit matching in SQL functions and Rust logic is clearer than a
/// join-table permission model.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "team_role", rename_all = "snake_case")]
pub enum TeamRole {
    Owner,
    Maintainer,
    Member,
    Watcher,
}

impl TeamRole {
    /// Numeric rank for the strict hierarchy Owner > Maintainer > Member > Watcher.
    ///
    /// `TeamRole` deliberately does not derive `Ord` — the derive would rank by declaration
    /// order, which puts `Owner` *lowest* and silently inverts every comparison. Callers that
    /// need to compare authority use this instead.
    ///
    /// A role added to this enum must be given a rank here, and the compiler's exhaustiveness
    /// check is what makes that non-optional. That is the point: policy ceilings expressed as
    /// `a.rank() <= b.rank()` (see `machine_authz`) then bar the new role by construction
    /// rather than by someone remembering to extend a hand-listed set.
    #[must_use]
    pub fn rank(self) -> u8 {
        match self {
            TeamRole::Owner => 3,
            TeamRole::Maintainer => 2,
            TeamRole::Member => 1,
            TeamRole::Watcher => 0,
        }
    }

    /// The SQL (`team_role`) and wire spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            TeamRole::Owner => "owner",
            TeamRole::Maintainer => "maintainer",
            TeamRole::Member => "member",
            TeamRole::Watcher => "watcher",
        }
    }
}

/// Response row for team endpoints — matches the real `kb_teams` columns.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRow {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    /// Optional human description; NULL until set via `PATCH /api/teams/{id}`.
    pub description: Option<String>,
    pub created: DateTime<Utc>,
    /// NULL = not an auto-join ("everyone") team; otherwise the role new
    /// members are enrolled at. Admin-gated to set.
    pub auto_join_role: Option<TeamRole>,
    /// What creating the team did to `public_within` rows (an auto-join team created under an
    /// anchor makes that anchor's rows stop conferring). Present only on `POST /api/teams` asked
    /// with `?answer=effects`; absent from every other answer. A row is named only to a caller who
    /// manages its team or its anchor, or is a system admin, and counted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(optional))]
    #[cfg_attr(feature = "web-api", schema(required = false))]
    pub public_within: Option<PublicWithinEffects>,
    /// The effective `public_within` rows the new team's roster reads through its parents, filtered
    /// as `public_within` is. Present only on `POST /api/teams` asked with `?answer=effects`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(optional))]
    #[cfg_attr(feature = "web-api", schema(required = false))]
    pub public_within_in_reach: Option<PublicWithinReach>,
}

/// What a team act's answer carries beyond its default, as `?answer=` on `POST /api/teams`,
/// `DELETE /api/teams/{id}` and `DELETE /api/teams/{id}/children/{child_id}`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamActAnswer {
    /// Also answer what the act did to `public_within` rows, filtered for the caller.
    Effects,
}

/// The query string of the three team acts whose default answer predates their `public_within`
/// effects. With no `answer`, each answers exactly as it always has: `201 TeamRow`, or `204`.
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
#[cfg_attr(feature = "web-api", into_params(parameter_in = Query))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamActAnswerQuery {
    /// `effects`: also answer what the act did to `public_within` rows (`POST` fills
    /// `TeamRow.public_within` and `public_within_in_reach`; the `DELETE`s answer `200` with
    /// `PublicWithinEffects` instead of `204`). A row is named only to a caller who manages its team
    /// or its anchor, or is a system admin, and counted otherwise.
    #[serde(default)]
    #[cfg_attr(feature = "web-api", param(inline))]
    pub answer: Option<TeamActAnswer>,
}

impl TeamActAnswerQuery {
    /// The query that asks for the effects, as a client sends it.
    #[must_use]
    pub fn effects() -> Self {
        Self {
            answer: Some(TeamActAnswer::Effects),
        }
    }

    #[must_use]
    pub fn wants_effects(self) -> bool {
        self.answer == Some(TeamActAnswer::Effects)
    }
}

/// Request body for `POST /api/teams`.
///
/// `parent` is an optional team ref (`+slug`, bare `slug`, or a team UUID); when present the new
/// team is created as a child in `kb_teams_parents` and the caller must be `owner`/`maintainer` on
/// the parent. `parents` is the several-parent form (team DAG doors D4): each is checked exactly as
/// `parent` is. Send one or the other, not both. `auto_join_role` is admin-gated.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamCreateRequest {
    pub slug: String,
    pub name: Option<String>,
    /// One parent team ref: the one-element form of `parents`.
    pub parent: Option<String>,
    /// Several parent team refs (at most 16; duplicates collapse). Mutually exclusive with
    /// `parent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub parents: Option<Vec<String>>,
    pub auto_join_role: Option<TeamRole>,
}

/// The most parents `POST /api/teams` takes in one request (team DAG doors D4), so one request
/// cannot fan out unboundedly.
pub const MAX_CREATE_PARENTS: usize = 16;

/// Response row for team membership — matches the real `kb_team_members` columns.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TeamMemberRow {
    pub team_id: Uuid,
    pub profile_id: Uuid,
    pub role: TeamRole,
    pub created: DateTime<Utc>,
}

/// Request body for `POST /api/teams/{id}/members`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddMemberRequest {
    pub profile_id: Uuid,
    pub role: TeamRole,
}

/// Provenance of a team membership row. Maps to the `team_member_source`
/// Postgres enum (added by `20260702000001_saml_group_provisioning.sql`).
/// `Idp` rows are owned by SAML reconcile and are not user-mutable.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "team_member_source", rename_all = "snake_case")]
pub enum TeamMemberSource {
    Native,
    Idp,
}

/// The path a recorded team-membership change came through — the `via` of a
/// `team_membership_changed` admin event (team-role rank spec, "The event").
///
/// A closed set with a fixed SQL spelling: `_admin_team_membership_write` refuses any value not in
/// it, and [`MembershipVia::as_str`] is the one Rust spelling of that set. The derived writers
/// (personal-team genesis, auto-join materialization, the machine-cap trigger) have no `via`
/// because they are not recorded (spec C8, R13).
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipVia {
    /// An add, change or remove by a manager of the team or a system admin, including a system admin
    /// adding themselves or raising their own role.
    MemberAct,
    /// A member acting on themselves: leaving, or lowering their own role.
    #[serde(rename = "self")]
    SelfAct,
    /// An accepted invitation (the invitee is the actor; the payload names the inviter).
    Invitation,
    /// `create_team`'s creator row.
    TeamGenesis,
    /// A system admin approving an access request (`access_service::review_request`).
    AccessApproval,
    /// A machine registration's team reach (`machine_registration_service::apply_reach`).
    MachineReach,
    /// SAML reconcile of identity-provider rows. The system is the actor.
    IdpReconcile,
}

impl MembershipVia {
    /// The wire and SQL spelling — what `_admin_team_membership_write`'s `p_via` accepts.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MembershipVia::MemberAct => "member_act",
            MembershipVia::SelfAct => "self",
            MembershipVia::Invitation => "invitation",
            MembershipVia::TeamGenesis => "team_genesis",
            MembershipVia::AccessApproval => "access_approval",
            MembershipVia::MachineReach => "machine_reach",
            MembershipVia::IdpReconcile => "idp_reconcile",
        }
    }
}

/// A team member enriched with the profile handle and provenance — the row
/// shape returned inside `TeamDetail`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TeamMemberDetail {
    pub profile_id: Uuid,
    pub handle: String,
    pub role: TeamRole,
    pub source: TeamMemberSource,
}

/// Full team detail — the team row plus its member roster. Response body for
/// `GET /api/teams/{id}`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamDetail {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub created: DateTime<Utc>,
    pub auto_join_role: Option<TeamRole>,
    pub members: Vec<TeamMemberDetail>,
    /// The team's direct child teams. Present (non-empty) only for a caller who manages this team
    /// or is a system admin: a child's members reach everything attached to this team, so its
    /// managers see which teams hang beneath it (team-authority fixes D3). Empty for everyone else.
    #[serde(default)]
    pub children: Vec<TeamChildRef>,
    /// The team's live parent teams, for every member of this team (any role) and for system
    /// admins (team DAG doors D9, ruling R2 as amended). A member already reads everything attached
    /// to these teams, so their names disclose nothing new; a child's manager needs them to detach
    /// or move it. Deliberately not symmetrical with `children`, which stays managers-only.
    #[serde(default)]
    pub parents: Vec<TeamRef>,
    /// The ancestors this team's owned contexts are opened to (`public_within` rows with this team
    /// as T), for every member of this team (any role) and for system admins: the same audience as
    /// `parents`, and not an ancestor's manager, since an anchor on another branch is not theirs to
    /// see. Empty for everyone else.
    #[serde(default)]
    pub public_within: Vec<PublicWithinRef>,
    /// The teams opened to this team's subtree (`public_within` rows with this team as the anchor),
    /// for a caller who manages this team or is a system admin (on the root or the gating team, a
    /// system admin only). Empty for everyone else.
    #[serde(default)]
    pub public_within_in: Vec<PublicWithinInRef>,
}

/// How many teams and people a `public_within` row reads to: the live teams in the anchor's
/// subtree, and the distinct people on them. Machines and connection agents never read through a
/// row, so they are not counted. A count, never a list.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicWithinAudience {
    #[cfg_attr(feature = "typescript", ts(type = "number"))]
    pub teams: i64,
    #[cfg_attr(feature = "typescript", ts(type = "number"))]
    pub principals: i64,
}

/// One anchor this team is opened to, on the team's own [`TeamDetail`].
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicWithinRef {
    pub anchor: TeamRef,
    /// Whether the row confers right now. Only a boolean: when it does not, nothing says which
    /// forbid, dead team or pool team stops it.
    pub effective: bool,
    /// Opened by a system admin over an anchor whose subtree reaches the whole instance.
    pub instance_wide: bool,
    pub opened_at: DateTime<Utc>,
    pub audience: PublicWithinAudience,
}

/// One team opened to this team's subtree, on the anchor's [`TeamDetail`].
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicWithinInRef {
    pub team: TeamRef,
    /// Whether the row confers right now. Only a boolean, as on [`PublicWithinRef`].
    pub effective: bool,
}

/// A `public_within` row named by both of its teams: `team` (whose owned contexts are opened) and
/// `anchor` (whose subtree reads them).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicWithinRowRef {
    pub team: TeamRef,
    pub anchor: TeamRef,
}

/// What `PUT /api/teams/{id}/public-within/{anchor_id}` did.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicWithinOpenKind {
    /// The row was written and recorded (`team_public_within_changed`).
    Opened,
    /// The row was already there; nothing was written or recorded.
    Exists,
}

/// Response body for `PUT /api/teams/{id}/public-within/{anchor_id}`. The team's owned contexts are
/// readable by the people under the anchor while the row is effective. `audience` is who that is
/// now; it grows with the anchor's subtree, which every manager of a team under the anchor can
/// change, as can an identity provider's group sync and an admin.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicWithinOutcome {
    pub outcome: PublicWithinOpenKind,
    pub audience: PublicWithinAudience,
    /// The row survives a pool team (the root, the gating team or an auto-join team) under the
    /// anchor: only a system admin opens such a row.
    pub instance_wide: bool,
}

/// Request body for `PUT /api/teams/{id}/public-within-policy`: `true` forbids every
/// `public_within` row over this team and every team beneath it; `false` clears this team's forbid
/// (a forbid set higher up still holds). On the root (`temper-system`) it is the instance-wide off,
/// and only a system admin sets it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicWithinPolicyRequest {
    pub forbidden: bool,
}

/// Response body for `PUT /api/teams/{id}/public-within-policy`: the rows the change made inert
/// (`dormant`) or made confer again (`revived`). A row is named only to a caller who manages its team
/// or its anchor, or is a system admin; the others are counted in `dormant_unnamed` and
/// `revived_unnamed`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicWithinPolicyOutcome {
    pub team_id: Uuid,
    /// The forbid now.
    pub forbidden: bool,
    /// False when the forbid already held the value: nothing was written or recorded.
    pub changed: bool,
    pub dormant: Vec<PublicWithinRowRef>,
    pub revived: Vec<PublicWithinRowRef>,
    pub dormant_unnamed: u32,
    pub revived_unnamed: u32,
}

/// What a structural or admin act did to the `public_within` rows it reached: rows deleted because
/// the anchor is no longer above the team by any path (`lapsed`; such a row never revives, and a
/// reopen needs both sides again), rows that stopped conferring (`dormant`) and rows that confer
/// again (`revived`). A row is named only to a caller who manages its team or its anchor, or is a
/// system admin; the others are counted in the `*_unnamed` fields. Each change is also recorded on
/// the admin ledger, readable by the managers of both teams.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicWithinEffects {
    #[serde(default)]
    pub lapsed: Vec<PublicWithinRowRef>,
    #[serde(default)]
    pub dormant: Vec<PublicWithinRowRef>,
    #[serde(default)]
    pub revived: Vec<PublicWithinRowRef>,
    #[serde(default)]
    pub lapsed_unnamed: u32,
    #[serde(default)]
    pub dormant_unnamed: u32,
    #[serde(default)]
    pub revived_unnamed: u32,
}

/// The effective `public_within` rows a team's roster newly reads through a new parent link: each
/// row whose anchor became one of the team's ancestors. A row is named only to a caller who manages
/// its team or its anchor, or is a system admin; the others are counted in `unnamed`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicWithinReach {
    #[serde(default)]
    pub rows: Vec<PublicWithinRowRef>,
    #[serde(default)]
    pub unnamed: u32,
}

/// A team named by id, slug and display name — a parent or child on [`TeamDetail`], an ancestor on
/// an auto-join outcome, a link on a restore outcome.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct TeamRef {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
}

/// A direct child team, as listed on its parent's [`TeamDetail`]. The same shape as [`TeamRef`],
/// which generalises it (team DAG doors D9); it stays its own type, and so its own schema, for one
/// release so the SDK class of that name keeps serving `children`. New fields use [`TeamRef`].
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct TeamChildRef {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
}

impl From<TeamRef> for TeamChildRef {
    fn from(t: TeamRef) -> Self {
        Self {
            id: t.id,
            slug: t.slug,
            name: t.name,
        }
    }
}

/// What `PUT /api/teams/{id}/children/{child_id}` did (team DAG doors D1).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachOutcome {
    /// The link was written and recorded (`team_parent_link_changed`).
    Linked,
    /// The child was already under this parent; nothing was written or recorded.
    Exists,
}

/// Response body for `PUT /api/teams/{id}/children/{child_id}`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachChildOutcome {
    pub outcome: AttachOutcome,
    /// What the link did to `public_within` rows: a pool brought under an anchor, or a forbid
    /// brought above a team, makes rows dormant; a new live path can revive one.
    #[serde(default)]
    pub public_within: PublicWithinEffects,
    /// The effective rows the child's roster now reads through the new parent.
    #[serde(default)]
    pub public_within_in_reach: PublicWithinReach,
    /// The child's live ancestors after the link, by every path, ordered by slug: every member of
    /// the child reads everything attached to each of them. The walk stops at a soft-deleted team,
    /// which confers nothing, so a team reached only through a dead one is absent. Read on the
    /// act's own transaction, so it is what the link committed. Empty when the server does not
    /// report it.
    #[serde(default)]
    pub ancestors: Vec<TeamRef>,
}

/// Request body for `POST /api/teams/{id}/reparent` (team DAG doors D3): move the team from one
/// parent to another in one act. Each is a team ref: `+slug`, a bare slug, a team UUID, or a
/// decorated `slug-<uuid>` ref.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReparentRequest {
    /// The current parent to leave.
    pub from: String,
    /// The new parent to join.
    pub to: String,
}

/// Response body for `POST /api/teams/{id}/reparent`. Both `team_parent_link_changed` records of the
/// move carry `correlation_id`, so the ledger shows one move rather than two unrelated acts.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReparentOutcome {
    pub team: TeamRef,
    pub from: TeamRef,
    pub to: TeamRef,
    pub correlation_id: Uuid,
    /// What the move did to `public_within` rows. A row whose anchor is no longer above the team by
    /// any path is deleted (`lapsed`) and never revives, even if the team moves back.
    #[serde(default)]
    pub public_within: PublicWithinEffects,
    /// The team's live ancestors after the move, by every path, ordered by slug: what its members
    /// now read. A team that was above only the old parent is absent, and the walk stops at a
    /// soft-deleted team. Read on the act's own transaction. Empty when the server does not report
    /// it.
    #[serde(default)]
    pub ancestors: Vec<TeamRef>,
}

/// Request body for `PUT /api/teams/{id}/auto-join` (team DAG doors D5, system admin only):
/// `member` or `watcher` sets the role every approved profile is enrolled at; `null` clears it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetAutoJoinRequest {
    pub role: Option<TeamRole>,
}

/// Response body for `PUT /api/teams/{id}/auto-join`. `ancestors` are the team's live ancestors,
/// by every path, ordered by slug, read on the act's own transaction (the walk stops at a
/// soft-deleted team): with auto-join set, every approved profile reads everything attached to each
/// of them, so the admin sees exactly what the change opened.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetAutoJoinOutcome {
    pub team: TeamRow,
    pub ancestors: Vec<TeamRef>,
    /// An auto-join team under an anchor makes that anchor's rows dormant unless an admin opened
    /// them instance-wide; clearing it revives them.
    #[serde(default)]
    pub public_within: PublicWithinEffects,
}

/// Response body for `POST /api/teams/{id}/restore` (team DAG doors D6). Restoring revives every
/// preserved row's reach: the roster (`members`, exactly as it was at deletion), the links to the
/// live `parents`, and the live `children` that reach through the team again.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreTeamOutcome {
    pub team: TeamRow,
    pub parents: Vec<TeamRef>,
    pub children: Vec<TeamRef>,
    pub members: Vec<TeamMemberDetail>,
    /// The `public_within` rows the restore revived (or, for a restored pool team, made dormant).
    #[serde(default)]
    pub public_within: PublicWithinEffects,
    /// The restored team's live ancestors, by every path, ordered by slug: its roster reads
    /// everything attached to each of them again, as does every live team beneath it. The walk
    /// stops at a soft-deleted team. Read on the act's own transaction. Empty when the server does
    /// not report it.
    #[serde(default)]
    pub ancestors: Vec<TeamRef>,
}

/// Request body for `POST /api/teams/{id}/rename-slug` (team DAG doors D7). The new slug is
/// lowercased, then must match `^[a-z0-9][a-z0-9-]{0,62}$` and must not start with `personal-`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameTeamSlugRequest {
    pub slug: String,
}

/// Response body for `POST /api/teams/{id}/rename-slug`. Addresses change: `+from/...` refs stop
/// resolving (UUID refs are unaffected), and refs stored in content are not rewritten.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameTeamSlugOutcome {
    pub team_id: Uuid,
    pub from: String,
    pub to: String,
}

/// Request body for `PATCH /api/teams/{id}` — update team metadata.
///
/// Both fields are optional: a `None` leaves that column unchanged (partial
/// merge via SQL COALESCE). Sending neither is a no-op that returns the row.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamUpdateRequest {
    // Display metadata only, on purpose (team DAG review L3). `update_team` checks only that the
    // caller manages the team; it runs none of `create_team`'s checks. A `slug` here would bypass
    // the reserved-prefix and uniqueness rules and could rename a team onto the gating slug; a
    // `parents` field would be an attach-parent door (review L1: it needs manage on both teams,
    // admin-only for the root and gating ancestors, and a roster review); an `auto_join_role`
    // would open an everyone-pool without the admin gate. Each needs its own door with its own
    // checks, not a field here. (Plain comment, not a doc comment, so it stays out of the API
    // schema.)
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Request body for `PATCH /api/teams/{id}/members/{profile_id}`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeRoleRequest {
    pub role: TeamRole,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `as_str` is the SQL spelling and serde is the payload spelling; the ledger reads one and
    /// `_admin_team_membership_write` writes the other, so they must be the same string.
    #[test]
    fn membership_via_as_str_matches_its_serde_spelling() {
        for via in [
            MembershipVia::MemberAct,
            MembershipVia::SelfAct,
            MembershipVia::Invitation,
            MembershipVia::TeamGenesis,
            MembershipVia::AccessApproval,
            MembershipVia::MachineReach,
            MembershipVia::IdpReconcile,
        ] {
            assert_eq!(
                serde_json::to_value(via).unwrap(),
                serde_json::Value::String(via.as_str().to_string())
            );
        }
    }
}
