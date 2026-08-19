//! Subscription types — a team/context/cogmap subscribes to an aspect of a connection.
//! See `internal/superpowers/specs/2026-07-13-external-systems-as-subscribed-emitters-design.md`
//! and the S2 chunk A design spec (research `01a017ad-b91d-7431-82cd-6402b9615e95`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A declared subscription: a subscriber (team/context/cogmap) wants to be told when a
/// connection emits events matching a selector.
///
/// The subscriber is polymorphic over `kb_contexts`/`kb_cogmaps`/`kb_teams` — the three kinds
/// the goal names, and the three `AnchorTable` variants chunk B writes as the `touches` rel at
/// intake. `subscriber_id` carries no FK: same discipline as `kb_access_grants.subject_id`,
/// validated in the service layer (the polymorphic `subscriber_table` makes a single FK target
/// impossible).
///
/// Revocation, not deletion: a subscription that existed at intake must stay resolvable at
/// disposition time (the delivery row's research-corpus property). Mirrors `kb_connections`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Subscription {
    pub id: Uuid,
    /// `kb_contexts` | `kb_cogmaps` | `kb_teams`. Maps 1:1 to `AnchorTable::{Contexts, Cogmaps,
    /// Teams}` — the `EventRef.target.kind` variants chunk B writes.
    pub subscriber_table: String,
    pub subscriber_id: Uuid,
    /// The team whose manage-capable role authorizes this subscription. NOT derived from the
    /// subscriber: `kb_cogmaps` has no owner team (only `kb_team_cogmaps` links), and
    /// `kb_contexts.owner_table` can be `kb_profiles` (no team). The caller names the team; the
    /// two-leg gate checks against it. For `kb_teams` subscribers, `authoring_team_id =
    /// subscriber_id`.
    pub authoring_team_id: Uuid,
    /// The connection whose events this subscription wants. A revoked connection may still have
    /// live subscriptions against it — the row stays honest about what was declared.
    pub connection_id: Uuid,
    /// The per-provider typed selector, stored as JSONB. The column is the storage; the wire type
    /// ([`SubscriptionSelector`]) is the shape. The variant IS the capability declaration.
    pub selector: serde_json::Value,
    pub created_by_profile_id: Uuid,
    pub created: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by_profile_id: Option<Uuid>,
}

/// The per-provider, per-grain selector a subscription carries. Deliberately thin: the goal's
/// design says *"per-provider and per-grain, and it must declare its own capability."*
///
/// **The variant IS the capability declaration.** A [`SubscriptionSelector::GitHubCodeownersPaths`]
/// selector declares "I need enrichment to resolve" by being that variant — no separate
/// `needs_enrichment: bool` that could drift out of sync with the variant. A
/// [`SubscriptionSelector::LinearProject`] selector declares "payload-only, no enrichment
/// needed" the same way.
///
/// Adding a provider = adding a variant = a compile error at every match site, which is the
/// desired forcing function (the enum is append-only for shipped variants; a deprecated variant
/// stays with a doc-comment and no new writes).
///
/// Tagged `#[serde(tag = "kind", rename_all = "snake_case")]` so the wire shape is
/// `{"kind": "git_hub_repository", "repo": "acme/temper", "event_types": [...]}` — the `kind`
/// field is the discriminator the service layer dispatches on, and the one a reader needs to
/// know which variant to deserialize without a separate schema registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubscriptionSelector {
    /// GitHub: subscribe to a repository's PR events. Coarse (payload-only) — the webhook
    /// payload carries the repo and the event type, so this selector matches at intake without
    /// any fetch. The CODEOWNERS-path filter is a separate variant
    /// ([`Self::GitHubCodeownersPaths`]) that declares its need for enrichment.
    GitHubRepository {
        /// `owner/repo` — e.g. `acme/temper`.
        repo: String,
        /// The event types to match, e.g. `["pull_request.merged"]`. Empty = match all
        /// registered event types on the connection.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        event_types: Vec<String>,
    },
    /// GitHub: subscribe to CODEOWNERS paths in a repo. Carries the path filter the selector
    /// will be evaluated against post-enrichment. **The presence of this variant declares
    /// "this subscription needs enrichment to resolve"** — GitHub's `pull_request` webhook
    /// payload carries no changed-file list, so the coarse intake match is on `repo` only, and
    /// the path filter is applied by S4's enrichment against the fetched file list. A
    /// subscription with this selector that never gets enriched stays `undetermined` —
    /// visible, never silently `out_of_scope` (invariant 6).
    GitHubCodeownersPaths {
        /// `owner/repo` — e.g. `acme/temper`.
        repo: String,
        /// The path globs to match against the changed-file list, e.g.
        /// `["src/api/**", "internal/superpowers/**"]`. Empty = match any file in the repo
        /// (degenerate but legal — the selector still declares enrichment is needed to get
        /// the file list at all).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        paths: Vec<String>,
    },
    /// Linear: subscribe to a project. Payload-only — Linear's issue webhook carries the
    /// issue inline, so no enrichment is needed for this grain. The `project_id` is Linear's
    /// identifier; the selector matches when the webhook's issue belongs to that project.
    LinearProject {
        /// Linear's UUID for the project.
        project_id: String,
    },
}

/// Create a subscription. The two-leg authz gate (authoring-team manage-capable + reach grant
/// held on the connection) runs in the service layer before the INSERT; this request is what the
/// caller supplies, not what the gate checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubscriptionRequest {
    /// `kb_contexts` | `kb_cogmaps` | `kb_teams`. The service layer validates the subscriber row
    /// exists and that `authoring_team_id` is linked to it (for contexts/cogmaps) or equals it
    /// (for teams).
    pub subscriber_table: String,
    pub subscriber_id: Uuid,
    /// The team whose manage-capable role authorizes this subscription. The caller must be an
    /// owner/maintainer of this team, and this team must hold a reach grant on the connection.
    /// For `kb_teams` subscribers, this equals `subscriber_id`.
    pub authoring_team_id: Uuid,
    pub connection_id: Uuid,
    /// The typed selector. The service layer deserializes this into a [`SubscriptionSelector`]
    /// before storing, so an unknown `kind` or a malformed payload is a 400, not a silent
    /// untyped JSON write.
    pub selector: SubscriptionSelector,
}
