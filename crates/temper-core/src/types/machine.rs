//! Machine-principal registration types. See
//! `temper-artifacts:specs/2026-07-10-machine-principal-registration-design.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A registered machine (`client_credentials`) principal.
///
/// No secret is stored, in this phase or ever (D1). `team_id` is the machine's
/// OWNER, never its reach (D6).
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
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
    /// Who minted the current secret by rotation, if it was rotated since this was recorded. With
    /// `registered_by_profile_id`, it names who holds or can hold the key: a person leaving a team
    /// takes off it every machine whose key they hold. Absent from an older server's response.
    #[serde(default)]
    pub secret_rotated_by_profile_id: Option<Uuid>,
}

/// One team the machine should be enrolled in, with its role.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamSpec {
    pub team_id: Uuid,
    /// The team role: `member` or `watcher`. A role above `member` is refused for every caller,
    /// a system admin included.
    // The CLI defaults to `member`; `MAX_MACHINE_TEAM_ROLE` is the ceiling (D4b).
    pub role: String,
}

/// One cogmap grant the machine should hold.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantSpec {
    pub cogmap_id: Uuid,
    pub can_write: bool,
}

/// Register a machine principal for an externally issued IdP `client_id`, with its team
/// memberships and cogmap grants listed explicitly.
// Reach is plural and always explicit (D10).
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionMachineRequest {
    pub client_id: String,
    pub label: String,
    /// The team that owns the machine. Ownership confers no reach; `teams` and `grants` do.
    pub owner_team_id: Option<Uuid>,
    pub teams: Vec<TeamSpec>,
    pub grants: Vec<GrantSpec>,
}

/// Point a fresh `client_id` at an existing agent profile (D8).
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebindMachineRequest {
    /// The new IdP client id.
    pub client_id: String,
    /// The existing `kb_machine_clients.id` whose profile is inherited. On
    /// `POST /api/machine-clients/{id}/rebind` the path's `{id}` is authoritative and overwrites it,
    /// so the HTTP body may omit it (it defaults to the nil UUID and is replaced before dispatch).
    #[serde(default)]
    pub from_machine_client_id: Uuid,
    pub label: String,
    /// When false (the default when omitted), the old row is revoked in the same transaction.
    #[serde(default)]
    pub keep_old_active: bool,
}

/// Issue a machine credential: temper mints both the `client_id` and the secret, so there is no
/// external client id. Team memberships and cogmap grants are listed explicitly.
// Phase B1 (`issuer='temper'`). Reach is plural and always explicit (D10).
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueMachineRequest {
    pub label: String,
    /// The team that owns the machine. Ownership confers no reach; `teams` and `grants` do.
    pub owner_team_id: Option<Uuid>,
    pub teams: Vec<TeamSpec>,
    pub grants: Vec<GrantSpec>,
}

/// A machine client with its plaintext `client_secret`, returned by issue and by secret
/// rotation. The secret is shown once and never stored, so it cannot be retrieved again.
// Only its SHA-256 hex persists (D1).
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssuedMachineCredential {
    pub client: MachineClient,
    pub client_secret: String,
    /// `true` when this instance cannot yet authenticate the credential: an external identity
    /// provider mints its tokens, so no token for a temper-issued client can be obtained here. The
    /// credential starts working once the instance runs temper's own authorization server.
    /// Absent from an older server's response, which reads as `false`.
    #[serde(default)]
    pub dormant: bool,
}

/// Rotate a temper-issued secret, leaving the previous secret valid for a grace window.
// D6: two live secrets, briefly.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotateSecretRequest {
    /// Seconds the previous secret stays valid after rotation, from 0 to 604800 (7 days).
    // The CLI supplies a default.
    pub grace_seconds: i64,
}
