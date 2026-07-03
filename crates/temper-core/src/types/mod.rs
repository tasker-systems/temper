//! Domain types for temper-cloud — profiles, teams, access control, auth,
//! sync protocol, manifest, config, vault, upload, search, reassign,
//! conflict resolution, device tracking, and events.
//!
//! All struct types derive `Debug, Clone, sqlx::FromRow` (database-backed)
//! or `Debug, Clone, serde::{Serialize, Deserialize}` (API types).
//! All enum types derive `Debug, Clone, Copy, PartialEq, Eq, sqlx::Type`
//! (Postgres enums) or add `Serialize, Deserialize` (API enums).

pub mod access;
pub mod access_gate;
pub mod admin;
pub mod api;
pub mod audit;
pub mod auth;
pub mod authorship;
pub mod cognitive_maps;
pub mod config;
pub mod conflict;
pub mod context;
pub mod device;
pub mod element_trail;
pub mod event;
pub mod facet_requests;
pub mod graph;
pub mod graph_atlas;
pub mod graph_scope;
pub mod graph_territory;
pub mod home;
pub mod ids;
pub mod ingest;
pub mod invitation;
pub mod invocation;
pub mod invocation_requests;
pub mod materialize;
pub mod merge;
pub mod ownership;
pub mod profile;
pub mod reassign;
pub mod reconcile;
pub mod relationship_events;
pub mod relationship_requests;
pub mod resource_grant;
pub mod search;
pub mod steward;
pub mod team;
pub mod upload;
pub mod vault;
pub mod vault_config;

pub use access::{AccessLevel, AccessScoped, TeamResource};
pub use access_gate::{
    Entitlements, JoinRequest, JoinRequestStatus, JoinRequestWithProfile, PublicSystemSettings,
    SystemSettings,
};
pub use api::{HealthResponse, ProfileUpdateRequest, SearchParams, SearchResultRow};
pub use audit::ResourceAuditRow;
pub use auth::{AuthClaims, AuthProvider, AuthenticatedProfile, PrincipalKind, ReconcileRequest};
pub use authorship::{ActContext, ActInput, AgentAuthorship, ConfidenceBand};
pub use config::{
    expand_tilde, global_config_path, load_config, load_config_from, AuthConfig, CloudConfig,
    CloudSection, CloudVaultConfig, MergePolicy, SkillConfig, SyncConfig, SyncSubscription,
    SyncSubscriptions, TemperConfig, UnifiedConfig, UnifiedSyncConfig, TEMPER_AUTH_PATH_ENV,
};
pub use conflict::{ConflictRecord, TemperSystemAnnotation};
pub use context::{ContextCreateRequest, ContextRow, ContextRowWithCounts};
pub use device::DeviceSyncState;
pub use element_trail::{ElementEvent, ElementKind, EventTrail};
pub use event::{EventQuery, EventResponse};
pub use graph::{EdgeKind, Polarity};
pub use graph_atlas::{AtlasEdge, AtlasNode, AtlasSubgraph, NodeHome, SliceRequest};
pub use graph_scope::{TeamRef, TeamScopeView, TeamZone};
pub use graph_territory::{
    Bridge, Component, OrphanNode, RegionMember, Territory, TerritoryKind, TerritoryOverview,
    TerritorySlice,
};
pub use ids::{ContextId, DocTypeId, EventId, ProfileId, ResourceAuditId, ResourceId, RevisionId};
pub use ingest::{pack_chunks, unpack_chunks, IngestPayload, PackError, PackedChunk};
pub use invitation::{
    AcceptInvitationResponse, CreateInvitationRequest, InvitationStatus, TeamInvitation,
};
pub use materialize::{
    MaterializeAck, MaterializeDelta, MaterializeDeltaInput, MaterializeRequest,
    MaterializeTriggerInput, DEFAULT_MATERIALIZE_THRESHOLD,
};
pub use merge::{MergeResult, MergeStrategy, PushKind};
pub use ownership::ResourceOwnership;
pub use profile::{DeactivationCheck, Profile, ProfileAuthLink};
pub use reassign::{BulkReassignAck, BulkReassignRequest, ReassignAck, ReassignResourceRequest};
pub use steward::{
    AdvanceWatermarkAck, AdvanceWatermarkRequest, IngestDelta, StewardAdvanceWatermarkInput,
    StewardDeltaInput, DEFAULT_STEWARD_INGEST_THRESHOLD,
};
pub use team::{
    AddMemberRequest, ChangeRoleRequest, Team, TeamCreateRequest, TeamDetail, TeamMember,
    TeamMemberDetail, TeamMemberRow, TeamMemberSource, TeamRole, TeamRow,
};
pub use upload::{UploadProcessingStatus, UploadResponse};
pub use vault::{IngestionSource, ResourceFrontmatter, VaultAddResult};
pub use vault_config::{DeviceOverrides, Subscription, SubscriptionOverride, VaultConfig};
