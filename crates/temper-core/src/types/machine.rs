//! Machine-principal registration types. See
//! `docs/superpowers/specs/2026-07-10-machine-principal-registration-design.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A registered machine (`client_credentials`) principal.
///
/// No secret is stored, in this phase or ever (D1). `team_id` is the machine's
/// OWNER, never its reach (D6).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MachineClient {
    pub id: Uuid,
    pub client_id: String,
    pub issuer: String,
    pub label: String,
    pub profile_id: Uuid,
    pub team_id: Option<Uuid>,
    pub registered_by_profile_id: Uuid,
    pub created: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by_profile_id: Option<Uuid>,
}

/// One team the machine should be enrolled in, with its role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamSpec {
    pub team_id: Uuid,
    /// `owner` | `maintainer` | `member` | `watcher`. Defaults to `member` at the CLI.
    pub role: String,
}

/// One cogmap grant the machine should hold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantSpec {
    pub cogmap_id: Uuid,
    pub can_write: bool,
}

/// Register a new machine principal. Reach is plural and always explicit (D10).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionMachineRequest {
    pub client_id: String,
    pub label: String,
    /// Recorded as `team_id`. Owner, not reach.
    pub owner_team_id: Option<Uuid>,
    pub teams: Vec<TeamSpec>,
    pub grants: Vec<GrantSpec>,
}

/// Point a fresh `client_id` at an existing agent profile (D8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebindMachineRequest {
    /// The new IdP client id.
    pub client_id: String,
    /// The existing `kb_machine_clients.id` whose profile is inherited.
    pub from_machine_client_id: Uuid,
    pub label: String,
    /// When false (the default), the old row is revoked in the same transaction.
    pub keep_old_active: bool,
}
