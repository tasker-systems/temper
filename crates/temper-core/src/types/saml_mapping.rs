//! Wire types for the SAML group-mapping doors (`/api/admin/saml/group-mappings`).
//!
//! A mapping turns an IdP's asserted group into a role on a team, at every login's reconcile, for
//! everyone the IdP puts in the group. Creating and removing one are a system admin's acts, each
//! recorded as `saml_group_mapping_changed`. A mapping confers at most `maintainer` unless the admin
//! sets `confirm_elevated`, which an `owner` role and a target of `temper-system` or the gating team
//! both need.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::team::TeamRole;

/// Request body for `POST /api/admin/saml/group-mappings`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "saml_mapping.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSamlGroupMappingRequest {
    /// The IdP whose assertions carry the group (`kb_saml_idp.idp_key`).
    pub idp_key: String,
    /// The group value, exactly as the IdP asserts it.
    pub group: String,
    /// The team the mapping confers a role on. Not a personal team.
    pub team_id: Uuid,
    /// The role conferred. `owner` needs `confirm_elevated`.
    pub role: TeamRole,
    /// The admin's explicit confirmation, required for an `owner` role and for a target of
    /// `temper-system` or the gating team. Without it such a request is refused and nothing is
    /// written. Ignored otherwise. Defaults to `false`.
    #[serde(default)]
    pub confirm_elevated: bool,
}

/// One `kb_saml_group_mappings` row.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "saml_mapping.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamlGroupMapping {
    pub idp_key: String,
    pub group: String,
    pub team_id: Uuid,
    pub role: TeamRole,
}

/// Response body for `POST /api/admin/saml/group-mappings`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "saml_mapping.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSamlGroupMappingOutcome {
    pub mapping: SamlGroupMapping,
    /// `false` when the mapping already existed with this role: nothing was written or recorded.
    pub created: bool,
    /// `true` when the act needed `confirm_elevated` and had it.
    pub confirmed: bool,
}

/// Query parameters for `DELETE /api/admin/saml/group-mappings`: the mapping's key.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "saml_mapping.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams, utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveSamlGroupMappingParams {
    /// The IdP whose assertions carry the group.
    pub idp_key: String,
    /// The group value, exactly as the mapping holds it.
    pub group: String,
    /// The team the mapping targets.
    pub team_id: Uuid,
}

/// Response body for `DELETE /api/admin/saml/group-mappings`.
///
/// Removing a mapping deletes no membership. The team's IdP memberships stay until each member's
/// next login, when the reconcile removes those that no remaining mapping confers.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "saml_mapping.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveSamlGroupMappingOutcome {
    /// The mapping as it was.
    pub removed: SamlGroupMapping,
    /// The team's memberships written by the IdP reconcile (`source = idp`), from this mapping or
    /// any other. The removal left every one of them in place.
    #[cfg_attr(feature = "typescript", ts(type = "number"))]
    pub idp_memberships_left_in_place: i64,
    /// What the removal left in place, and what governs it, in a sentence.
    pub memberships: String,
}

/// The sentence a removal answers with in [`RemoveSamlGroupMappingOutcome::memberships`].
pub const MAPPING_REMOVAL_LEAVES_MEMBERSHIPS: &str =
    "Memberships the mapping already conferred are left in place. The IdP reconcile governs them: \
     at each member's next login it removes those that no remaining mapping confers.";
