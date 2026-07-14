//! Connection types — temper's authed link to a remote system. See
//! `docs/superpowers/specs/2026-07-13-external-systems-as-subscribed-emitters-design.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A provisioned connection to a remote system (a GitHub App installation, a Linear workspace).
///
/// `owner_team_id` is the connection's OWNER, never its reach — owning a connection does not
/// confer the right to subscribe to it. Reach is plural and explicitly granted.
///
/// The two capability tiers are separately provisioned and both explicit: a connection is
/// **ledger-capable** when `webhook_events` is non-empty (events land) and **reach-capable**
/// when `tool_manifest` is non-empty (agents can read the remote back, so judgment becomes
/// possible). A ledger-only connection is legal and useful, but inert for judgment — and it
/// says so rather than leaving an agent to mysteriously produce nothing.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Connection {
    pub id: Uuid,
    pub provider: String,
    pub slug: String,
    pub name: String,
    /// Owner, not reach. `None` = teamless = admin-only, and fails closed.
    pub owner_team_id: Option<Uuid>,
    pub registered_by_profile_id: Uuid,
    /// The connection's dedicated agent profile. It carries no auth link and no machine-client
    /// row — a connection never authenticates *to* temper.
    pub profile_id: Uuid,
    /// The entity remote payloads are attributed to (`<handle>@webhook`).
    pub emitter_entity_id: Uuid,
    pub home_context_id: Uuid,
    /// The abstract credential reference behind the broker seam —
    /// `{broker, connector, installation?}`, never a bare connector id. `None` is the
    /// `needs_credential` birth state; see [`Connection::needs_credential`].
    pub credential: Option<serde_json::Value>,
    /// Registered remote event types. Non-empty ⇒ ledger-capable.
    pub webhook_events: Vec<String>,
    /// Declared read-only remote tools. Non-empty ⇒ reach-capable.
    pub tool_manifest: serde_json::Value,
    /// `org` | `workspace` | `installation` | `repo-set` | `project` — the grain the credential
    /// is scoped at, in the provider's terms.
    pub reach_granularity: Option<String>,
    /// What the credential can ACTUALLY see, in provider terms (`acme/temper`, `acme/*`).
    pub reach_covers: Option<String>,
    pub created: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by_profile_id: Option<Uuid>,
}

impl Connection {
    /// A connection with no credential. Derived, never stored: a status enum would only drift
    /// out of sync with the column it describes.
    pub fn needs_credential(&self) -> bool {
        self.credential.is_none()
    }

    /// Events land, facts accrue. Useful on its own.
    pub fn is_ledger_capable(&self) -> bool {
        !self.webhook_events.is_empty()
    }

    /// Agents can read the remote back, so judgment becomes possible.
    pub fn is_reach_capable(&self) -> bool {
        match &self.tool_manifest {
            serde_json::Value::Object(map) => !map.is_empty(),
            serde_json::Value::Array(items) => !items.is_empty(),
            _ => false,
        }
    }
}

/// Provision a connection. It is born `needs_credential` — the credential is attached
/// separately, so a connection never silently pretends to be more than it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionConnectionRequest {
    /// `github` | `linear` | …
    pub provider: String,
    /// Display name. The addressable slug is derived from it.
    pub name: String,
    /// Recorded as `owner_team_id`. Owner, not reach. `None` is admin-only.
    pub owner_team_id: Option<Uuid>,
    /// The declared reach fidelity, in the provider's terms. Both halves are honest fields
    /// rather than a computed `exceeds_temper_reach` bool: remote and temper scope are
    /// incommensurable, and a stored bool would go stale.
    pub reach_granularity: Option<String>,
    pub reach_covers: Option<String>,
}
