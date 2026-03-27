//! Domain types for temper-cloud — profiles, teams, access control, auth.
//!
//! All struct types derive `Debug, Clone, sqlx::FromRow`.
//! All enum types derive `Debug, Clone, Copy, PartialEq, Eq, sqlx::Type`.
//! Postgres enums map directly via `sqlx::Type` with `type_name` attributes.

pub mod access;
pub mod auth;
pub mod invitation;
pub mod ownership;
pub mod profile;
pub mod team;

pub use access::{AccessLevel, AccessScoped, TeamResource};
pub use auth::{AuthClaims, AuthProvider, AuthenticatedProfile};
pub use invitation::{InvitationStatus, TeamInvitation};
pub use ownership::ResourceOwnership;
pub use profile::{DeactivationCheck, Profile, ProfileAuthLink};
pub use team::{Team, TeamMember, TeamRole};
