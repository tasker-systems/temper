use uuid::Uuid;

/// Resource ownership — present on every resource.
///
/// Two distinct concepts:
/// - `originator_profile_id`: Immutable provenance — who created this resource.
///   Never changes, even on ownership transfer. Part of the permanent audit trail.
/// - `owner_profile_id`: Mutable control — who currently manages this resource.
///   Defaults to originator at creation. Can be transferred (e.g., when someone
///   leaves a team and hands off their work).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceOwnership {
    /// Immutable provenance — who created this resource
    pub originator_profile_id: Uuid,
    /// Mutable control — who currently manages this resource
    pub owner_profile_id: Uuid,
}
