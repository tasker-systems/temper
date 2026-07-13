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

// ── re-embed trigger (operator-only) ──────────────────────────────────────────
//
// Deliberately NOT in the OpenAPI contract: the handler is mounted with a plain `.route()`, like the
// rest of `/api/*/admin/*`. It is an operator action, not part of the product surface.

/// Body for `POST /api/embed/admin/reembed`.
///
/// Exactly one scope: a single resource, a whole context, or everything. Three granularities because a
/// re-embed is a thing you try on **one**, then a **few**, then **all** — in that order. A trigger that
/// only offers "all" is one nobody dares pull.
///
/// Nothing is *marked* dirty. Staleness is derived — a chunk is stale when it has no vector, or when
/// its `embedded_with` is not the model the server embeds with — so this only ever enqueues work for
/// chunks that genuinely need it, and it is safe to re-run at any time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReembedRequest {
    /// Re-embed just this resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<Uuid>,
    /// Re-embed every stale resource homed in this context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<Uuid>,
    /// Re-embed everything stale. Must be set explicitly — an empty body is a no-op, not "all".
    #[serde(default)]
    pub all: bool,
    /// Max resources to enqueue this call. Bounds blast radius: run it repeatedly to walk the index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    /// Report what is stale without enqueuing anything. The safe first move.
    #[serde(default)]
    pub dry_run: bool,
}

/// Result of a re-embed trigger — and, on `dry_run`, just the survey.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReembedSummary {
    /// Resources in scope still holding stale chunks.
    pub stale_resources: u64,
    /// Stale chunks in scope. Divide by the drain's per-tick throughput to estimate the drain time.
    pub stale_chunks: u64,
    /// Resources actually enqueued by this call. Empty on `dry_run`, and empty for any resource that
    /// already had a live job (re-running never double-queues).
    pub enqueued: Vec<Uuid>,
}
