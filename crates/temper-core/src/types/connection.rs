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
    /// Who affirmed that binding this connection's coarse remote reach to a team is intentional.
    /// `None` = never affirmed (declares no reach, or no grant requiring affirmation yet). A
    /// single-valued, last-writer audit stamp — not a per-grant ledger — and NOT a computed
    /// `exceeds_temper_reach` bool: it records a declared intent, it does not resolve the
    /// (incommensurable) remote-vs-temper scope asymmetry.
    pub reach_affirmed_by: Option<Uuid>,
    /// When the affirmation was made. `None` = never affirmed. Paired with `reach_affirmed_by`
    /// and `reach_affirmation` as one last-writer stamp.
    pub reach_affirmed_at: Option<DateTime<Utc>>,
    /// The stated rationale — why the coarse reach binding is intentional. `None` = never affirmed.
    pub reach_affirmation: Option<String>,
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

    /// The credential, typed. `None` is `needs_credential`; `Some(Err(..))` means the stored JSON
    /// does not parse as a [`ConnectionCredential`] — which a reader must not paper over, because
    /// the broker seam dispatches on `broker` and a credential it cannot read is not a credential.
    pub fn credential_typed(&self) -> Option<Result<ConnectionCredential, serde_json::Error>> {
        self.credential
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()))
    }

    /// Events land, facts accrue. Useful on its own.
    pub fn is_ledger_capable(&self) -> bool {
        !self.webhook_events.is_empty()
    }

    /// A connection "declares reach" when its declared remote-reach fidelity is populated (set at
    /// provision, in the provider's terms). The honest, non-computing signal that its remote reach
    /// is coarse enough to require an intentional affirmation before it is bound to a team — remote
    /// and temper scope are incommensurable, so there is no `exceeds_temper_reach` bool to compute;
    /// a populated declaration is all the signal an honest broker can give. Beat 2 enforces this.
    pub fn declares_reach(&self) -> bool {
        self.reach_granularity.is_some() || self.reach_covers.is_some()
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

/// The abstract credential reference stored in `kb_connections.credential`, and the body of the
/// attach-credential request — one type, so the wire shape and the stored shape cannot drift.
///
/// **This holds no secret.** `broker` names an implementation and `connector` identifies a
/// connector *the broker* holds the secret for; the secret itself never reaches temper. That is
/// why this is safe to return on a read path unredacted, unlike `kb_machine_clients.secret_hash`.
///
/// **`broker` is never a bare Vercel connector id.** It names the implementation so a platform
/// swap costs one adapter — the seam is two operations (`mint`, `verifyInbound`) and nothing above
/// it knows which broker is behind it. Keeping the connector id on the *row* rather than in code is
/// also what lets a self-hosted operator provision their own connectors in their own Vercel team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionCredential {
    /// Names the implementation behind the broker seam — e.g. `vercel-connect`. Nothing dispatches
    /// on this yet; the adapter that does is a later chunk.
    pub broker: String,
    /// The broker's identifier for this connector. Per-instance, per-row, never hardcoded.
    pub connector: String,
    /// The specific installation, where the provider has that concept (a GitHub App installation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation: Option<String>,
}

/// Register the remote event types a connection receives. Non-empty ⇒ **ledger-capable**.
///
/// Replaces the set wholesale rather than appending: the registered set is a mirror of what the
/// remote system is actually configured to send, and a merge would let a stale entry outlive the
/// remote webhook it names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetWebhookEventsRequest {
    pub events: Vec<String>,
}

/// What minting once at attach time observed about a credential — the
/// verification result surfaced back to the operator.
///
/// This is B4's half of the reach story: `observed_reach` is what the credential
/// can *actually* see (the provider's mint metadata), placed next to the
/// connection's *declared* reach (`reach_granularity`/`reach_covers`) so a human
/// can see the gap. There is deliberately **no computed `exceeds` bool** — remote
/// and temper scope are incommensurable and a stored bool would go stale. B3 adds
/// the acknowledgment; B4 only makes the gap visible.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CredentialVerification {
    /// The connector minted successfully (proved live).
    pub verified: bool,
    /// The reach the provider reported at mint (its `metadata`), when verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_reach: Option<serde_json::Value>,
    /// Why verification did not fully succeed, when it did not — consent pending,
    /// no broker configured, or a transient failure. A capability that is absent
    /// is stated, never silent (invariant 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The result of attaching a credential: the updated connection plus what minting
/// once at attach time observed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachCredentialResponse {
    pub connection: Connection,
    pub verification: CredentialVerification,
}

/// Grant (or revoke) a TEAM's read-reach on a connection. Owning a connection is not reaching it:
/// this writes a `kb_access_grants` row (`subject_table = 'kb_connections'`) that lets the named
/// team READ what the connection receives. Reach is read-only — a grant confers no write. One
/// request type carries `team` for both the grant and the revoke, so the two sides cannot drift.
///
/// The CLI resolves the team ref to a UUID before sending, so this is a `Uuid`, not a ref string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantConnectionReachRequest {
    /// The team receiving read-reach. Its members inherit read on what the connection receives.
    pub team: Uuid,
}

/// Declare the read-only remote tools a connection exposes. Non-empty ⇒ **reach-capable**.
///
/// Not decorative: the manifest is the evidence the provider is admissible at all. A provider that
/// cannot be reached through an API, an MCP server, or a CLI we can hold credentials for is
/// rejected — proxying is out of scope by rule, so an empty manifest means judgment is impossible,
/// not merely unconfigured.
///
/// Tool *names* only. Anything richer is a per-provider schema, and no provider needs one yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetToolManifestRequest {
    pub tools: Vec<String>,
}
