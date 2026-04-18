//! Domain types for temper-cloud — profiles, teams, access control, auth,
//! sync protocol, manifest, config, vault, upload, search, transfer,
//! conflict resolution, device tracking, and events.
//!
//! All struct types derive `Debug, Clone, sqlx::FromRow` (database-backed)
//! or `Debug, Clone, serde::{Serialize, Deserialize}` (API types).
//! All enum types derive `Debug, Clone, Copy, PartialEq, Eq, sqlx::Type`
//! (Postgres enums) or add `Serialize, Deserialize` (API enums).

pub mod access;
pub mod access_gate;
pub mod api;
pub mod audit;
pub mod auth;
pub mod config;
pub mod conflict;
pub mod context;
pub mod device;
pub mod event;
pub mod graph;
pub mod ids;
pub mod ingest;
pub mod invitation;
pub mod managed_meta;
pub mod manifest;
pub mod merge;
pub mod ownership;
pub mod profile;
pub mod resource;
pub mod search;
pub mod sync;
pub mod team;
pub mod transfer;
pub mod upload;
pub mod vault;
pub mod vault_config;

pub use access::{AccessLevel, AccessScoped, TeamResource};
pub use access_gate::{
    Entitlements, JoinRequest, JoinRequestStatus, JoinRequestWithProfile, PublicSystemSettings,
    SystemSettings,
};
pub use api::{
    EventListParams, EventRow, HealthResponse, ProfileUpdateRequest, SearchParams, SearchResultRow,
};
pub use audit::ResourceAuditRow;
pub use auth::{AuthClaims, AuthProvider, AuthenticatedProfile};
pub use config::{
    expand_tilde, global_config_path, load_config, load_config_from, AuthConfig, CloudConfig,
    CloudSection, CloudVaultConfig, MergePolicy, SkillConfig, SyncConfig, SyncSubscription,
    SyncSubscriptions, TemperConfig, UnifiedConfig, UnifiedSyncConfig, VaultState,
    TEMPER_VAULT_STATE_ENV,
};
pub use conflict::{ConflictRecord, TemperSystemAnnotation};
pub use context::{ContextCreateRequest, ContextRow, ContextRowWithCounts};
pub use device::DeviceSyncState;
pub use event::{EventQuery, EventResponse};
pub use graph::{
    EdgeReconciliation, EdgeType, GraphEdgeRow, GraphNeighborRow, GraphTraversalRow, ResolvedEdge,
    ResourceRelationships, TargetRef,
};
pub use ids::{ContextId, DocTypeId, EventId, ProfileId, ResourceAuditId, ResourceId};
pub use ingest::{pack_chunks, unpack_chunks, IngestPayload, PackError, PackedChunk};
pub use invitation::{InvitationStatus, TeamInvitation};
pub use managed_meta::{ManagedMeta, MetaUpdatePayload, ResourceManifestRow};
pub use manifest::{Manifest, ManifestEntry, ManifestEntryState};
pub use merge::{MergeResult, MergeStrategy, PushKind};
pub use ownership::ResourceOwnership;
pub use profile::{DeactivationCheck, Profile, ProfileAuthLink};
pub use resource::{
    ContentChunk, ContentResponse, DeleteResponse, ResourceCreateRequest, ResourceFacets,
    ResourceListParams, ResourceListResponse, ResourceRow, ResourceSortField,
    ResourceUpdateRequest, SortOrder,
};
pub use sync::{
    MergedResource, ResolutionType, SyncCompleteRequest, SyncCompleteResponse, SyncConflictItem,
    SyncContextEntries, SyncManifestEntry, SyncManifestItem, SyncManifestResponse, SyncPullItem,
    SyncPushItem, SyncRemovedItem, SyncResolveRequest, SyncStatusRequest, SyncStatusResponse,
};
pub use team::{Team, TeamMember, TeamRole};
pub use transfer::{BulkReassignRequest, ResourceTransfer, TransferRequest, TransferStatus};
pub use upload::{UploadProcessingStatus, UploadResponse};
pub use vault::{IngestionSource, ResourceFrontmatter, VaultAddResult};
pub use vault_config::{DeviceOverrides, Subscription, SubscriptionOverride, VaultConfig};
