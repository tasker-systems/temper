//! Wire types for the admin / system-settings surface (Chunk 6).
//!
//! `UpdateSettingsRequest` is a partial-update payload: every `Some` field
//! overwrites that `kb_system_settings` column, every `None` leaves it
//! unchanged (COALESCE on the server). `access_mode` is a raw string validated
//! server-side against `{open, invite_only}` — mirrors how `SystemSettings`
//! keeps `access_mode` as `String` rather than a sqlx-decoded enum.
//!
//! `PromoteAdminRequest` grants the target profile `owner` on a team. A `None`
//! `team_id` means "the configured gating team" (resolved server-side so the
//! gating slug never leaves the server). System-admin ≡ owner of the gating
//! team, so the default case mints a second system admin.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Partial-update body for `PATCH /api/access/admin/settings`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateSettingsRequest {
    /// `"open"` or `"invite_only"`. Validated server-side.
    pub access_mode: Option<String>,
    /// Slug of the team that gates the instance in `invite_only` mode.
    pub gating_team_slug: Option<String>,
    /// Human-facing instance name.
    pub instance_name: Option<String>,
    /// Terms-of-service version label.
    pub terms_version: Option<String>,
    /// URI of the terms-of-service resource.
    pub terms_resource_uri: Option<String>,
}

impl UpdateSettingsRequest {
    /// True when no field is set — the caller wants a read, not a write.
    pub fn is_empty(&self) -> bool {
        self.access_mode.is_none()
            && self.gating_team_slug.is_none()
            && self.instance_name.is_none()
            && self.terms_version.is_none()
            && self.terms_resource_uri.is_none()
    }
}

/// Body for `POST /api/access/admin/promote`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteAdminRequest {
    /// Profile to promote (grant `owner` on the target team).
    pub profile_id: Uuid,
    /// Target team; `None` ⇒ the configured gating team (mints a system admin).
    pub team_id: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_settings_is_empty_detects_no_fields() {
        assert!(UpdateSettingsRequest::default().is_empty());
        let one = UpdateSettingsRequest {
            instance_name: Some("Acme".to_owned()),
            ..Default::default()
        };
        assert!(!one.is_empty());
    }

    #[test]
    fn promote_request_roundtrips_through_json() {
        let req = PromoteAdminRequest {
            profile_id: Uuid::nil(),
            team_id: None,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let back: PromoteAdminRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.profile_id, req.profile_id);
        assert!(back.team_id.is_none());
    }
}
