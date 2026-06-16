//! Backend-selection gate (WS6 chunk 4a, §D).
//!
//! A process reads the `kb_backend_selection` flag once at startup into
//! [`BackendSelection`] and stores it on `AppState`. Surfaces construct their
//! backend through [`select_backend`] / [`require_legacy_backend`] rather than
//! calling `DbBackend::new` directly, so the flip (chunk 5) is one config row
//! + one redeploy.

use sqlx::PgPool;
use temper_core::error::TemperError;
use temper_core::operations::{Backend, Surface};
use temper_core::types::ids::ProfileId;

use crate::backend::DbBackend;

/// Which substrate the surfaces dispatch reads/writes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendSelection {
    /// Today's `public.*` schema via `DbBackend`.
    Legacy,
    /// The `temper_next.*` substrate via `NextBackend` (lands in 4b).
    Next,
}

impl BackendSelection {
    /// Parse the stored flag value. Encapsulated so the stringly form never
    /// leaks past this boundary.
    pub(crate) fn from_db(value: &str) -> Result<Self, TemperError> {
        match value {
            "legacy" => Ok(Self::Legacy),
            "next" => Ok(Self::Next),
            other => Err(TemperError::Config(format!(
                "unknown backend selection flag value: {other:?}"
            ))),
        }
    }
}

/// Construct the active backend for a trait-method call site (the six
/// `Backend` commands). Returns a boxed trait object so the `next` arm can
/// later supply `NextBackend` behind the same interface.
pub fn select_backend(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: ProfileId,
    device_id: String,
    surface: Surface,
) -> Result<Box<dyn Backend>, TemperError> {
    match selection {
        BackendSelection::Legacy => Ok(Box::new(DbBackend::new(
            pool.clone(),
            profile_id,
            device_id,
            surface,
        ))),
        BackendSelection::Next => {
            #[cfg(feature = "next-backend")]
            {
                let _ = (device_id, surface);
                Ok(Box::new(crate::backend::NextBackend::new(
                    pool.clone(),
                    profile_id,
                )))
            }
            #[cfg(not(feature = "next-backend"))]
            {
                let _ = (pool, profile_id, device_id, surface);
                Err(TemperError::NotImplemented(
                    "next backend requires the `next-backend` build feature".into(),
                ))
            }
        }
    }
}

/// Construct a concrete `DbBackend` for call sites whose methods are not yet
/// on the `Backend` trait (relationship/edge writes). These stay on legacy in
/// 4a but refuse `next`, so a process never half-switches: resource ops would
/// route to a substrate these ops can't reach. The trait growth that brings
/// them under `select_backend` lands in 4c.
pub fn require_legacy_backend(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: ProfileId,
    device_id: String,
    surface: Surface,
) -> Result<DbBackend, TemperError> {
    match selection {
        BackendSelection::Legacy => {
            Ok(DbBackend::new(pool.clone(), profile_id, device_id, surface))
        }
        BackendSelection::Next => Err(TemperError::NotImplemented(
            "relationship/edge writes not yet ported to the next backend (WS6 4c)".into(),
        )),
    }
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn pid() -> ProfileId {
        ProfileId::from(Uuid::nil())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn select_backend_legacy_returns_a_backend(pool: PgPool) {
        let b = select_backend(
            BackendSelection::Legacy,
            &pool,
            pid(),
            "api".to_string(),
            Surface::ApiHttp,
        );
        assert!(b.is_ok(), "legacy arm must construct a backend");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn select_backend_next_constructs_or_gates(pool: PgPool) {
        let b = select_backend(
            BackendSelection::Next,
            &pool,
            pid(),
            "api".to_string(),
            Surface::ApiHttp,
        );
        #[cfg(feature = "next-backend")]
        assert!(
            b.is_ok(),
            "with the next-backend feature, the next arm constructs NextBackend"
        );
        #[cfg(not(feature = "next-backend"))]
        assert!(
            matches!(b, Err(TemperError::NotImplemented(_))),
            "without the feature, the next arm gates"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn require_legacy_refuses_next(pool: PgPool) {
        let ok = require_legacy_backend(
            BackendSelection::Legacy,
            &pool,
            pid(),
            "mcp".to_string(),
            Surface::Mcp,
        );
        assert!(ok.is_ok(), "legacy arm yields a concrete DbBackend");

        let err = require_legacy_backend(
            BackendSelection::Next,
            &pool,
            pid(),
            "mcp".to_string(),
            Surface::Mcp,
        );
        assert!(
            matches!(err, Err(TemperError::NotImplemented(_))),
            "relationship/edge sites must refuse next until 4c"
        );
    }
}
