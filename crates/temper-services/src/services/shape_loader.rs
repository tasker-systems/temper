//! The shape loader — the I/O half of asking data artifacts by what they are (spec
//! `temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md` §2.4, §3.5 steps
//! 1–3).
//!
//! It turns each shape an ask names, and the principal asking, into a
//! [`ShapeLoad`] — `Loaded`, `NotAskable`,
//! `Superseded`, `Uncompilable` or `SchemaTooLarge` — so that everything after it is the pure half
//! in `temper-core`, which holds no pool and no JSON Schema compiler.
//!
//! **The order is fixed, and each later check is reachable only through the earlier ones:**
//!
//! 1. **Readable?** Through the gated [`readback::shape_by_id`] (`20260822000030`) and nothing
//!    else. No row is `NotAskable`, whether the shape does not exist or the principal cannot read
//!    its home — the read folds both into one answer, and nothing here asks again, so the two are
//!    one value by construction.
//! 2. **Folded?** `Superseded`, carrying the live row at the same home and family when there is one
//!    ([`readback::live_shape_at`]). A successor is in the same home, so it is as readable as the
//!    row step 1 admitted, and the lookup is gated besides.
//! 3. **Compiles?** First measured: a schema larger than the walk reads
//!    ([`within_schema_bound`]) is `SchemaTooLarge` and is never compiled. Within the bound,
//!    through the no-fetch constructor ([`shape_schema::compile`]), off the async executor; a
//!    schema it refuses is `Uncompilable`, and the compiler's text goes nowhere.
//!
//! **One decision for every door that judges a stored or declared schema** ([`schema_verdict`]):
//! the loader, the by-id read's `askable`, and declare (`shape_service::declare_shape`) measure
//! against the same bound and compile through the same constructor, so they cannot disagree about
//! which schemas can be asked. A reader therefore never makes the server compile more than the
//! walk's bound, however large a schema an author stored before declare refused one.
//!
//! The successor lookup reads only the home and family of the row step 1 returned; no caller text
//! reaches SQL here except the shape id, bound as a `uuid`.

use std::collections::BTreeSet;

use serde_json::Value;
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use temper_core::error::TemperError;
use temper_core::types::artifact_ask::{
    enumerate, within_schema_bound, LoadedShape, ShapeContext, ShapeLoad, ShapeSuccessor,
    Vocabulary, WalkBounds,
};
use temper_core::types::ids::{ProfileId, ShapeId};
use temper_core::types::query::PlanRefusal;
use temper_substrate::readback::{self, RetrievedShape};
use temper_substrate::shape_schema;

/// Load every shape one request's asks name, for `principal`, into the context its asks are
/// validated against ([`ShapeContext::session`]). One [`load_one`] per id, in id order.
///
/// **What it costs is per distinct shape**, and the caller bounds how many it passes: each id is
/// one gated read (the stored schema whole), one bounded measurement, at most one compile within
/// the walk's byte bound, and — once validation teaches its vocabulary — one enumeration under a
/// budget of its own. The asks then share one budget for path resolution and membership
/// ([`AskSession`](temper_core::types::artifact_ask::AskSession)).
///
/// # Errors
/// A database fault, never a refusal: every refusal is a [`ShapeLoad`] in the context.
pub async fn load(
    pool: &PgPool,
    principal: ProfileId,
    ids: &BTreeSet<ShapeId>,
) -> ApiResult<ShapeContext> {
    let mut loads = Vec::with_capacity(ids.len());
    for &id in ids {
        loads.push((id, load_one(pool, principal, id).await?));
    }
    Ok(loads.into_iter().collect())
}

/// Load one shape for `principal`: refusal steps 1–3 of §3.5, in order (see the module doc).
///
/// # Errors
/// A database fault, never a refusal.
pub async fn load_one(pool: &PgPool, principal: ProfileId, id: ShapeId) -> ApiResult<ShapeLoad> {
    // Step 1 — the gated read, and nothing else: absent and unreadable are both `None` here.
    let Some(row) = readback::shape_by_id(pool, principal, id)
        .await
        .map_err(read_fault)?
    else {
        return Ok(ShapeLoad::NotAskable);
    };

    // Step 2 — reached only for a row step 1 admitted.
    if row.is_folded {
        let successor = readback::live_shape_at(
            pool,
            principal,
            row.home_anchor,
            row.kind_owner,
            &row.artifact_kind,
        )
        .await
        .map_err(read_fault)?
        .map(|live| ShapeSuccessor {
            shape_id: live.shape_id,
            shape_version: live.shape_version,
        });
        return Ok(ShapeLoad::Superseded { successor });
    }

    // Step 3 — reached only for a live row.
    let (row, verdict) = off_the_executor(move || {
        let verdict = schema_verdict(&row.schema);
        (row, verdict)
    })
    .await?;
    Ok(match verdict.refusing_load() {
        Some(refused) => refused,
        None => ShapeLoad::Loaded(loaded(row)),
    })
}

/// What a stored or declared schema is, for asking: within the walk's byte bound and compiling,
/// within it and not compiling, or past it — and then never compiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaVerdict {
    Compiles,
    Uncompilable,
    TooLarge,
}

impl SchemaVerdict {
    /// The load a schema with this verdict is refused as, or `None` when it compiles.
    pub fn refusing_load(self) -> Option<ShapeLoad> {
        match self {
            SchemaVerdict::Compiles => None,
            SchemaVerdict::Uncompilable => Some(ShapeLoad::Uncompilable),
            SchemaVerdict::TooLarge => Some(ShapeLoad::SchemaTooLarge),
        }
    }
}

/// Step 3's decision, the one every door shares: measure the schema against the walk's byte bound
/// ([`within_schema_bound`], which reads no further than the bound), and only within it compile
/// through the no-fetch constructor. Synchronous and CPU-bound: an async caller runs it off the
/// executor ([`load_one`], [`askable`]).
pub fn schema_verdict(schema: &Value) -> SchemaVerdict {
    if !within_schema_bound(schema) {
        return SchemaVerdict::TooLarge;
    }
    if compiles(schema) {
        SchemaVerdict::Compiles
    } else {
        SchemaVerdict::Uncompilable
    }
}

/// Every compile the loader has run on a schema past the walk's byte bound — none, while the
/// measurement holds. A test reads it to see that an oversized schema was never compiled; it is
/// counted only for oversized schemas so that suites compiling ordinary schemas in parallel cannot
/// move it.
#[cfg(any(test, feature = "test-harness"))]
pub static COMPILES_PAST_THE_BOUND: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// The one compile [`schema_verdict`] runs.
fn compiles(schema: &Value) -> bool {
    #[cfg(any(test, feature = "test-harness"))]
    if !within_schema_bound(schema) {
        COMPILES_PAST_THE_BOUND.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    shape_schema::compile(schema).is_ok()
}

/// What a caller may ask of a shape, for its by-id read (spec §2.5): the vocabulary the walk
/// enumerates, and the refusal step 3 gives this version's schema when it cannot be walked — past
/// the byte bound (`askable` then says `truncated: [schema_bytes]`), or not compiling (`askable`
/// empty). [`schema_verdict`] decides it, off the async executor, so the read and the loader
/// judge a schema alike. The row is handed back with the answer.
///
/// # Errors
/// Only if the blocking task cannot be joined.
pub async fn askable(
    row: RetrievedShape,
) -> ApiResult<(RetrievedShape, Vocabulary, Option<PlanRefusal>)> {
    off_the_executor(move || {
        let verdict = schema_verdict(&row.schema);
        let vocabulary = match verdict {
            // Past the bound, the walk reads nothing and says why: `truncated: [schema_bytes]`.
            SchemaVerdict::Compiles | SchemaVerdict::TooLarge => {
                enumerate(&row.schema, &WalkBounds::BUILD)
            }
            SchemaVerdict::Uncompilable => Vocabulary::default(),
        };
        let refusal = verdict.refusing_load().and_then(|load| load.loaded().err());
        (row, vocabulary, refusal)
    })
    .await
}

/// Run CPU-bound work — a compile, an enumeration — on the blocking pool, so a slow one holds no
/// async worker.
async fn off_the_executor<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| ApiError::Internal(format!("shape schema task failed: {e}")))
}

/// A live, compilable row as the pure half reads it.
fn loaded(row: RetrievedShape) -> LoadedShape {
    LoadedShape {
        shape_id: row.shape_id,
        shape_version: row.shape_version,
        home_anchor_table: row.home_anchor.table().to_owned(),
        home_anchor_id: row.home_anchor.uuid(),
        kind_owner_table: row.kind_owner.owner_table().to_owned(),
        kind_owner_id: row.kind_owner.owner_id(),
        artifact_kind: row.artifact_kind,
        schema: row.schema,
    }
}

/// A read fault, mapped as the neighbouring shape reads map theirs (`substrate_read::get_shape`).
fn read_fault(e: anyhow::Error) -> ApiError {
    ApiError::from(TemperError::Api(e.to_string()))
}
