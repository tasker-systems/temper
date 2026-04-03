//! Domain types for temper-cloud — profiles, teams, access control, auth,
//! sync protocol, manifest, config, vault, upload, search, transfer,
//! conflict resolution, device tracking, and events.
//!
//! All struct types derive `Debug, Clone, sqlx::FromRow` (database-backed)
//! or `Debug, Clone, serde::{Serialize, Deserialize}` (API types).
//! All enum types derive `Debug, Clone, Copy, PartialEq, Eq, sqlx::Type`
//! (Postgres enums) or add `Serialize, Deserialize` (API enums).

pub mod access;
pub mod api;
pub mod auth;
pub mod config;
pub mod conflict;
pub mod context;
pub mod device;
pub mod event;
pub mod ingest;
pub mod invitation;
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
pub use api::{
    EventListParams, EventRow, HealthResponse, ProfileUpdateRequest, SearchParams, SearchResultRow,
};
pub use auth::{AuthClaims, AuthProvider, AuthenticatedProfile};
pub use config::{
    expand_tilde, global_config_path, load_config, load_config_from, AuthConfig,
    AuthProviderConfig, CliConfig, CloudConfig, CloudSection, CloudVaultConfig, MergePolicy,
    SkillConfig, SyncAutoConfig, SyncConfig, SyncSubscription, SyncSubscriptionsConfig,
    TemperConfig, UnifiedConfig, UnifiedSyncConfig,
};
pub use conflict::{ConflictRecord, TemperSystemAnnotation};
pub use context::{ContextCreateRequest, ContextRow};
pub use device::DeviceSyncState;
pub use event::{EventQuery, EventResponse};
pub use ingest::{pack_chunks, unpack_chunks, IngestPayload, PackError, PackedChunk};
pub use invitation::{InvitationStatus, TeamInvitation};
pub use manifest::{Manifest, ManifestEntry, ManifestEntryState};
pub use merge::{MergeResult, MergeStrategy, PushKind};
pub use ownership::ResourceOwnership;
pub use profile::{DeactivationCheck, Profile, ProfileAuthLink};
pub use resource::{
    ContentChunk, ContentResponse, DeleteResponse, ResourceCreateRequest, ResourceListParams,
    ResourceRow, ResourceUpdateRequest,
};
pub use sync::{
    MergedResource, ResolutionType, SyncCompleteRequest, SyncCompleteResponse, SyncConflictItem,
    SyncContextEntries, SyncManifestEntry, SyncPullItem, SyncPushItem, SyncRemovedItem,
    SyncResolveRequest, SyncStatusRequest, SyncStatusResponse,
};
pub use team::{Team, TeamMember, TeamRole};
pub use transfer::{BulkReassignRequest, ResourceTransfer, TransferRequest, TransferStatus};
pub use upload::{UploadProcessingStatus, UploadResponse};
pub use vault::{IngestionSource, ResourceFrontmatter, VaultAddResult};
pub use vault_config::{DeviceOverrides, Subscription, SubscriptionOverride, VaultConfig};
