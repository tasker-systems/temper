//! Types for the system access gate: join requests, system settings, and entitlements.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a join request in its lifecycle.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "join_request_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum JoinRequestStatus {
    Pending,
    Approved,
    Rejected,
    Withdrawn,
}

/// A user-initiated request to join a team (typically the gating team).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct JoinRequest {
    pub id: Uuid,
    pub team_id: Uuid,
    pub requesting_profile_id: Uuid,
    pub status: JoinRequestStatus,
    pub message: Option<String>,
    pub source: String,
    pub accepted_terms_version: Option<String>,
    pub accepted_terms_at: Option<DateTime<Utc>>,
    pub reviewed_by_profile_id: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// A join request with the requesting profile's display info (for admin queue).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct JoinRequestWithProfile {
    pub id: Uuid,
    pub team_id: Uuid,
    pub requesting_profile_id: Uuid,
    pub status: JoinRequestStatus,
    pub message: Option<String>,
    pub source: String,
    pub accepted_terms_version: Option<String>,
    pub accepted_terms_at: Option<DateTime<Utc>>,
    pub reviewed_by_profile_id: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    // Joined from kb_profiles
    pub display_name: String,
    pub email: Option<String>,
}

/// An **open** reconsideration request, with the asking principal's identity (spec D15 admin
/// inbox).
///
/// There is deliberately no `decided_at` on this shape. The queue is what is *outstanding*, so a
/// row reaching a reader is open by construction — carrying a column that is always `NULL` would
/// invite a caller to filter on it and believe they had narrowed something.
///
/// The identity join is the same one [`JoinRequestWithProfile`] does, for the same reason: the row
/// on its own is a bare `profile_id`, and an admin weighing a reconsideration needs to know who is
/// asking. It carries **no** decision field beyond the note — closing a review records that it was
/// handled and moves no standing (D15); the admin's actual answer is a separate `Approve`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ReviewRequestWithProfile {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub message: Option<String>,
    pub created: DateTime<Utc>,
    // Joined from kb_profiles
    pub handle: String,
    pub display_name: String,
    pub email: Option<String>,
}

// The `AccessMode` enum was retired with the `access_mode` control (spec §14 / D18): standing now
// answers per-principal what a global mode switch answered instance-wide, so no code branches on the
// mode any more. Phase 2 finishes the retirement — the `access_mode` wire field is gone from both
// settings structs below, and the `kb_system_settings.access_mode` column drops in Phase 2's
// operator-run migration. Re-introducing a typed mode here would be the first step of re-coupling
// admission to a global switch — which is exactly what standing replaced.

/// Instance-wide system settings (singleton row).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SystemSettings {
    pub id: i32,
    pub gating_team_slug: Option<String>,
    pub terms_version: Option<String>,
    pub terms_resource_uri: Option<String>,
    pub instance_name: Option<String>,
    pub updated: DateTime<Utc>,
}

/// Public-facing system settings (no gating_team_slug — prevents info leakage).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicSystemSettings {
    pub terms_version: Option<String>,
    pub terms_resource_uri: Option<String>,
    pub instance_name: Option<String>,
}

impl From<SystemSettings> for PublicSystemSettings {
    fn from(s: SystemSettings) -> Self {
        Self {
            terms_version: s.terms_version,
            terms_resource_uri: s.terms_resource_uri,
            instance_name: s.instance_name,
        }
    }
}

/// Entitlements included in the profile response — tells the client
/// what this profile is allowed to do at the system level.
///
/// `Deserialize` is not decoration: `temper-client` reads this back off `GET /api/profile`, and
/// before it was derived the client deserialized the response into a bare `Profile`, which has no
/// `entitlements` field and no `deny_unknown_fields` — so serde dropped the whole object silently
/// on every call. The CLI was fetching the authoritative access answer and discarding it.
///
/// **There is deliberately no narrowed `standing` field here.** One was built and removed: the
/// premise was that a principal must not learn they were *revoked* rather than merely *denied*.
/// That is not this system's posture. Spec D15 grants a revoked principal the right to request
/// reconsideration — `Act::RequestReview` is legal from `Revoked` and from nothing else
/// (`temper-principal/src/transition.rs`) — and a right nobody may be told they hold is not a
/// right. Accordingly the refusal type names the state on purpose
/// (`temper-principal/src/refusal.rs`: *"access was revoked; you may request a review"*), and the
/// CLI routes it to a different remedy than `denied` (`temper-cli/src/access_gate.rs`). Narrowing
/// here would have contradicted all of that while five other surfaces still disclosed it — and it
/// also hid legitimate *rejections*, since rejection returns standing to `denied`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entitlements {
    pub system_access: bool,
    pub is_admin: bool,
    /// The caller's own standing, **as stored** — all five states, not a narrowed subset.
    ///
    /// Reporting `revoked` and `deactivated` is deliberate and matches the rest of the system.
    /// Spec D15 grants a revoked principal the right to request reconsideration
    /// (`Act::RequestReview` is legal from `Revoked` and nothing else), so the refusal path names
    /// the state on purpose and routes it to a different remedy than `denied`. A narrowed
    /// three-variant version of this field was built and reverted: it contradicted that design
    /// while five other surfaces still disclosed the state, and it hid legitimate *rejections*,
    /// since rejection returns standing to `denied`.
    ///
    /// **`None` means the server predates this field — never "no standing".** Absence of a
    /// standing row denies, and the producer reports that as `Denied`, so `None` carries exactly
    /// one meaning: this instance is older than the client asking. That matters because a CLI
    /// upgrades independently of the instance it talks to; a required field here would cost such a
    /// client the whole object, `system_access` included, which older servers answer correctly.
    ///
    /// The `Option` alone buys that — serde's derive already reads a missing `Option` field as
    /// `None`, so no `#[serde(default)]` is needed and one here would be inert. Verified by probe:
    /// removing it changed nothing. `crates/temper-client/tests/profile_entitlements_test.rs`
    /// covers the absent-field case against a real body rather than a constructed value.
    ///
    /// `Deactivated` is unreachable through `GET /api/profile`: a deactivated principal fails
    /// authentication outright (`temper-services/src/auth/mod.rs` Level-1 kill-switch) and never
    /// reaches a handler. It is in the type because the type is the stored state, not this
    /// endpoint's reachable subset.
    pub standing: Option<temper_principal::Standing>,
    pub join_request_status: Option<JoinRequestStatus>,
}

/// The command a rejected caller runs to request system access.
///
/// Lives here, beside the `SystemAccessDetails` it rides in, rather than as a
/// literal at the surface that builds the payload: temper-api cannot see the
/// clap tree, so a string authored there is gated by nothing. Here, temper-cli
/// depends on temper-core and pins it against the real parser.
pub const REQUEST_ACCESS_COMMAND: &str = "temper auth request-access --message \"...\"";

/// Details included in the SystemAccessRequired error response.
///
/// SECURITY NOTE: The `email` and `display_name` fields are safe to include
/// because the caller already proved ownership of this identity through OAuth.
/// We are reflecting the caller's own profile back — not disclosing another
/// user's information. Do not add fields that reveal other users' data.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemAccessDetails {
    pub email: Option<String>,
    pub display_name: Option<String>,
    /// Why this principal was refused, typed (spec §7). The sole refusal signal on the 403 since
    /// Phase 2 retired the legacy `join_request_status` field new clients never branched on. The
    /// typed refusal distinguishes "never granted" (`denied`) from "granted and revoked" (`revoked`)
    /// — a distinction that matters to the user and in an audit.
    ///
    /// Carried as `temper_principal::Refusal` so every surface branches on it exhaustively — the
    /// Rust ones through the enum, the generated temper-ts / temper-rb clients through the
    /// discriminated `kind` union that crate's feature-gated derives now emit.
    pub refusal: temper_principal::Refusal,
    pub request_url: Option<String>,
    pub cli_command: Option<String>,
}

/// How many principals are waiting in an operator queue — the count-shaped answer to
/// `GET /api/access/admin/requests/count` and `GET /api/access/admin/reviews/count`.
///
/// **A count, not a queue.** `temper warmup` reports that a queue has something in it; the
/// queue itself is one command away (`temper admin requests list`, `temper admin reviews
/// list`). Fetching every row with its handle, display name, email and message so that
/// `.len()` could be taken made a session-start primer carry other people's identities
/// through the client on every session.
///
/// **A refusal is a `403`, never a zero.** These routes sit behind `require_system_admin`,
/// exactly as their list siblings do, so a caller who may not see the queue is told so — and
/// "not yours to see" stays distinguishable from "yours to see, and empty". A count endpoint
/// that answered a non-admin with `0` would erase that difference silently, which is the one
/// thing the primer's `Option` fields exist to prevent.
///
/// No `utoipa` derive, matching [`ReviewRequestWithProfile`] and every other type on this
/// operator-only surface: those routes are mounted with a plain `.route(...)` and stay off the
/// documented contract on purpose.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueCount {
    /// `i32` for the same reason as [`crate::types::invitation::PendingInvitationCounts::count`]: a 64-bit count reaches
    /// TypeScript as `bigint` and does not survive `JSON.stringify`.
    pub count: i32,
}
