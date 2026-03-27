//! Domain types for temper-cloud — profiles, teams, access control, auth,
//! sync protocol, manifest, config, vault, upload, search, transfer,
//! conflict resolution, device tracking, and events.
//!
//! All struct types derive `Debug, Clone, sqlx::FromRow` (database-backed)
//! or `Debug, Clone, serde::{Serialize, Deserialize}` (API types).
//! All enum types derive `Debug, Clone, Copy, PartialEq, Eq, sqlx::Type`
//! (Postgres enums) or add `Serialize, Deserialize` (API enums).

pub mod access;
pub mod auth;
pub mod config;
pub mod conflict;
pub mod device;
pub mod event;
pub mod invitation;
pub mod manifest;
pub mod ownership;
pub mod profile;
pub mod search;
pub mod sync;
pub mod team;
pub mod transfer;
pub mod upload;
pub mod vault;

pub use access::{AccessLevel, AccessScoped, TeamResource};
pub use auth::{AuthClaims, AuthProvider, AuthenticatedProfile};
pub use config::{
    CliConfig, CloudConfig, CloudVaultConfig, MergePolicy, SyncConfig, SyncSubscription,
};
pub use conflict::{ConflictRecord, TemperSystemAnnotation};
pub use device::{DeviceIdentity, DeviceSyncState};
pub use event::{EventQuery, EventResponse};
pub use invitation::{InvitationStatus, TeamInvitation};
pub use manifest::{Manifest, ManifestEntry, ManifestEntryState};
pub use ownership::ResourceOwnership;
pub use profile::{DeactivationCheck, Profile, ProfileAuthLink};
pub use search::{SearchMode, SearchRequest, SearchResponse, SearchResult};
pub use sync::{
    ResolutionType, SyncCompleteRequest, SyncCompleteResponse, SyncConflictEntry,
    SyncManifestEntry, SyncPullEntry, SyncPullRequest, SyncPullResourceMeta, SyncPushEntry,
    SyncRemovedEntry, SyncResolveRequest, SyncStatusRequest, SyncStatusResponse,
};
pub use team::{Team, TeamMember, TeamRole};
pub use transfer::{BulkReassignRequest, ResourceTransfer, TransferRequest, TransferStatus};
pub use upload::{
    UploadCompleteRequest, UploadInitRequest, UploadInitResponse, UploadProcessingStatus,
    UploadStatusResponse,
};
pub use vault::{IngestionSource, ResourceFrontmatter, VaultAddResult};
