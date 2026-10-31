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
//!    row step 1 admitted, and the lookup is gated besides. A batch read's later pages skip this
//!    step for the row their first page named ([`load_one_pinned`]) and carry the successor beside
//!    the load instead.
//! 3. **Compiles?** First measured: a schema larger than the walk reads
//!    ([`within_schema_bound`]) is `SchemaTooLarge` and is never compiled. Within the bound,
//!    through the no-fetch constructor ([`shape_schema::compile`]); a schema it refuses is
//!    `Uncompilable`, and the compiler's text goes nowhere.
//!
//! **One decision for every door that decides whether a schema can be asked** ([`judge`]): the
//! loader, the by-id read's `askable`, and declare (`shape_service::declare_shape`) measure
//! against the same bound and compile through the same constructor, so they cannot disagree about
//! which schemas can be asked, and none of them compiles a schema past the walk's bound. Commit and
//! reconcile are not among them: they validate artifacts against a stored schema through
//! [`shape_schema::compile`] directly, whatever its size.
//!
//! **How often and how widely a schema is compiled is bounded.** Each compile runs on the blocking
//! pool, under one of a few process-wide permits (`compile_permits`), so a burst of reads queues
//! rather than taking every core; and a stored shape's verdict is remembered by id, so a shape
//! read many times is compiled about once per process. A compiling shape's by-id read enumerates
//! its vocabulary on every read, after the verdict, under a permit too. The constructor holds each
//! `pattern` and `patternProperties` regex to `shape_schema::MAX_PATTERN_BYTES`. A shape row is
//! immutable per id (amendment mints a new id), so a remembered verdict never goes stale. It is
//! consulted only with a row a gated read returned — the loader reads its own, and the by-id read
//! is passed only such rows — so it answers nothing about a shape the caller cannot read.
//!
//! **What a path refusal teaches is made per request.** A request whose ask is refused a path on a
//! shape enumerates that shape's vocabulary once to teach it, in `temper-core`'s ask session, on
//! the async executor: at most `MAX_ASKED_SHAPES` shapes a request, each walk linear in a schema
//! of at most the byte bound.
//!
//! The successor lookup reads only the home and family of the row step 1 returned; no caller text
//! reaches SQL here except the shape id, bound as a `uuid`.

use std::collections::{BTreeSet, HashMap};
use std::sync::{LazyLock, Mutex, PoisonError};

use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::Semaphore;

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
/// the walk's byte bound, and — once validation teaches its vocabulary — one enumeration, linear in
/// the schema. The asks then share one budget for membership
/// ([`AskSession`](temper_core::types::artifact_ask::AskSession)); each path they name is resolved
/// once per request.
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
    Ok(context_of(loads))
}

/// The context of these loads, each filed under the id it was loaded for.
pub(crate) fn context_of(loads: Vec<(ShapeId, ShapeLoad)>) -> ShapeContext {
    loads.into_iter().collect()
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
        let successor = successor_of(pool, principal, &row).await?;
        return Ok(ShapeLoad::Superseded { successor });
    }

    // Step 3 — reached only for a live row.
    compiled(row).await
}

/// What a continuation page's load found for the shape row its first page named: the load, and,
/// when that row has been folded since, the version of its lineage now in force.
#[derive(Debug, Clone, PartialEq)]
pub struct PinnedLoad {
    /// Steps 1 and 3 on the named row, folded or not: `NotAskable`, `Uncompilable`,
    /// `SchemaTooLarge`, or `Loaded` — never `Superseded`.
    pub load: ShapeLoad,
    /// The live row at the named row's home and family, when the named row is folded and the load
    /// is `Loaded`; `None` otherwise.
    pub amended_since: Option<ShapeSuccessor>,
}

/// Load the shape row a batch read's first page named, for a later page (spec §7.3, P17): refusal
/// steps 1 and 3 on that row **even when it is folded**, and step 2 skipped, so an amendment
/// between pages never strands an enumeration on `shape_superseded`. The row stays readable by id
/// after folding (`shape_by_id`, `20260822000030`), and step 1 re-decides that readability on every
/// page. A first page (no `after`) goes through [`load_one`], so it is still refused
/// `shape_superseded` naming a folded id, and taught its successor. Any `after` makes a request a
/// continuation, so that refusal is advice to a first page, never a boundary: the position carries
/// no authority either way.
///
/// # Errors
/// A database fault, never a refusal.
pub async fn load_one_pinned(
    pool: &PgPool,
    principal: ProfileId,
    id: ShapeId,
) -> ApiResult<PinnedLoad> {
    // Step 1 — as `load_one`'s: absent and unreadable are both `None`.
    let Some(row) = readback::shape_by_id(pool, principal, id)
        .await
        .map_err(read_fault)?
    else {
        return Ok(PinnedLoad {
            load: ShapeLoad::NotAskable,
            amended_since: None,
        });
    };

    let successor = if row.is_folded {
        successor_of(pool, principal, &row).await?
    } else {
        None
    };
    // Step 3 — on the named row, folded or not.
    let load = compiled(row).await?;
    let amended_since = successor.filter(|_| matches!(load, ShapeLoad::Loaded(_)));
    Ok(PinnedLoad {
        load,
        amended_since,
    })
}

/// The live row at a folded row's home and family — its successor — read through the gated
/// [`readback::live_shape_at`]. Reached only for a row step 1 admitted; a successor is in the same
/// home, so it is as readable as that row.
async fn successor_of(
    pool: &PgPool,
    principal: ProfileId,
    row: &RetrievedShape,
) -> ApiResult<Option<ShapeSuccessor>> {
    Ok(readback::live_shape_at(
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
    }))
}

/// Step 3 for a row a gated read returned: `Loaded`, or the refusal its schema's verdict decides.
async fn compiled(row: RetrievedShape) -> ApiResult<ShapeLoad> {
    let verdict = verdict_of(row.shape_id, &row.schema).await?;
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

/// Step 3's decision, the one every door that decides whether a schema can be asked shares:
/// measure the schema against the walk's byte bound ([`within_schema_bound`], which reads no
/// further than the bound), then against the parsed-size budget
/// ([`shape_schema::within_node_budget`], which reads no further than its caps), and only within
/// both compile through the no-fetch constructor — on the blocking pool, under a compile permit.
/// The schema is copied for the blocking task only once it is known to be within both and a
/// permit is held, so no request waiting for a permit holds a copy. The budget is counted here,
/// before the permit, because the permit is shared with declare and ask loads: a schema past the
/// budget is `Uncompilable` without taking one, without the copy and without a compile.
///
/// # Errors
/// Only if the permits are closed or the blocking task cannot be joined.
pub async fn judge(schema: &Value) -> ApiResult<SchemaVerdict> {
    if !within_schema_bound(schema) {
        return Ok(SchemaVerdict::TooLarge);
    }
    if !shape_schema::within_node_budget(schema) {
        return Ok(SchemaVerdict::Uncompilable);
    }
    with_schema_off_the_executor(schema, |schema| {
        if shape_schema::compile(schema).is_ok() {
            SchemaVerdict::Compiles
        } else {
            SchemaVerdict::Uncompilable
        }
    })
    .await
}

/// The most verdicts the process remembers. An entry is a shape id and a byte, about 150 KiB of
/// map at the cap; past it an arbitrary entry is forgotten, and that shape is judged again when it
/// is next read.
const MAX_REMEMBERED_VERDICTS: usize = 4096;

/// Each stored shape's verdict, by id, for the life of the process.
///
/// # What a hit saves
///
/// A compile, under a permit: `T_compile` 0.85 ms for a schema with both caps full, against
/// `T_parse` 0.26 ms for the same schema, and up to 3.3 ms for one filling the 12,288-value budget
/// with boolean fields (`shape_schema_cost_bench.rs`, `parse_worst_case_at_the_caps` and
/// `many_tiny_subschemas_at_the_size_cap`; arm64, release). A compile costs more than three parses
/// of the same schema, and a read that finds its verdict here spares both it and the permit.
static VERDICTS: LazyLock<Mutex<HashMap<ShapeId, SchemaVerdict>>> = LazyLock::new(Mutex::default);

/// A stored shape's verdict — remembered, or judged now and remembered. Called only with a row the
/// gated read returned. Two first reads that race both judge, under the permits; after that, none.
async fn verdict_of(id: ShapeId, schema: &Value) -> ApiResult<SchemaVerdict> {
    if let Some(verdict) = remembered(id) {
        return Ok(verdict);
    }
    let verdict = judge(schema).await?;
    remember(id, verdict);
    Ok(verdict)
}

fn remembered(id: ShapeId) -> Option<SchemaVerdict> {
    VERDICTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&id)
        .copied()
}

fn remember(id: ShapeId, verdict: SchemaVerdict) {
    let mut verdicts = VERDICTS.lock().unwrap_or_else(PoisonError::into_inner);
    if verdicts.len() >= MAX_REMEMBERED_VERDICTS && !verdicts.contains_key(&id) {
        if let Some(evicted) = verdicts.keys().next().copied() {
            verdicts.remove(&evicted);
        }
    }
    verdicts.insert(id, verdict);
}

/// What a caller may ask of a shape, for its by-id read (spec §2.5): the vocabulary the walk
/// enumerates, and the refusal step 3 gives this version's schema when it cannot be walked — past
/// the byte bound (`askable` then says `truncated: [schema_bytes]`), or not compiling (`askable`
/// empty). The verdict is the loader's own ([`judge`], remembered by id), so the read and the
/// loader judge a schema alike.
///
/// Every read asks the verdict first, and for a compiling shape then enumerates the vocabulary
/// under a compile permit, off the executor (`with_schema_off_the_executor`): a walk of at most
/// the byte bound, linear in it. A schema past the bound or not compiling is not walked.
///
/// Takes the row the gated read returned, whole, so the id the verdict is remembered by and the
/// schema it was derived from are one value's. `askable` is passed only rows `readback`'s gated
/// reads returned: its one caller, `substrate_read::get_shape`, passes the row
/// `readback::shape_by_id` gave it.
///
/// # Errors
/// Only if the permits are closed or a blocking task cannot be joined.
pub async fn askable(row: &RetrievedShape) -> ApiResult<(Vocabulary, Option<PlanRefusal>)> {
    let verdict = verdict_of(row.shape_id, &row.schema).await?;
    let vocabulary = match verdict {
        // Within the bound: a walk of at most 256 KiB, linear in it, off the executor.
        SchemaVerdict::Compiles => {
            #[cfg(feature = "test-db")]
            test_support::record(&row.schema);
            let shape = row.shape_id;
            with_schema_off_the_executor(&row.schema, move |schema| {
                let _shape = tracing::error_span!("askable", %shape).entered();
                enumerate(schema, &WalkBounds::BUILD)
            })
            .await?
        }
        // Past it the walk reads nothing — the measurement stops at the bound — and says why:
        // `truncated: [schema_bytes]`.
        SchemaVerdict::TooLarge => enumerate(&row.schema, &WalkBounds::BUILD),
        SchemaVerdict::Uncompilable => Vocabulary::default(),
    };
    let refusal = verdict.refusing_load().and_then(|load| load.loaded().err());
    Ok((vocabulary, refusal))
}

/// How many shape compiles may run at once in this process: half the cores, and at least one.
/// A compile is CPU-bound, so more at once than there are cores buys no throughput, only memory —
/// each holds its validator until it is judged — and the blocking pool it runs on is shared with
/// query embedding, which the other half leaves room for. A burst past it waits for a permit.
fn compile_permits() -> usize {
    std::thread::available_parallelism().map_or(1, |cores| (cores.get() / 2).max(1))
}

static COMPILE_PERMITS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(compile_permits()));

/// Run CPU-bound work on a schema — a compile, an enumeration — on the blocking pool, holding a
/// compile permit for as long as it runs. The schema is copied for the blocking task only once the
/// permit is held, so a request queued for a permit holds no copy. The permit moves into the
/// blocking task, so a request dropped while the work runs does not release a permit the work
/// still holds.
async fn with_schema_off_the_executor<T: Send + 'static>(
    schema: &Value,
    work: impl FnOnce(&Value) -> T + Send + 'static,
) -> ApiResult<T> {
    let permit = COMPILE_PERMITS
        .acquire()
        .await
        .map_err(|e| ApiError::Internal(format!("shape compile permits closed: {e}")))?;
    let schema = schema.clone();
    tokio::task::spawn_blocking(move || {
        let _held = permit;
        work(&schema)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("shape schema task failed: {e}")))
}

/// Run CPU-bound shape work that owns its inputs — a reconcile pass's compile, or its check of one
/// artifact's content — on the blocking pool, holding a compile permit for as long as it runs: the
/// same permits as [`with_schema_off_the_executor`], so background reconciliation and request-path
/// compiles share one bound on how much shape work runs at once.
pub(crate) async fn run_off_the_executor<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> ApiResult<T> {
    let permit = COMPILE_PERMITS
        .acquire()
        .await
        .map_err(|e| ApiError::Internal(format!("shape compile permits closed: {e}")))?;
    tokio::task::spawn_blocking(move || {
        let _held = permit;
        work()
    })
    .await
    .map_err(|e| ApiError::Internal(format!("shape schema task failed: {e}")))
}

/// What a test reads to see how often [`askable`] enumerated a compiling shape — every enumeration,
/// whichever read asked for it.
#[cfg(feature = "test-db")]
pub mod test_support {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock, PoisonError};

    use serde_json::Value;

    type Tally = OnceLock<Mutex<HashMap<String, usize>>>;

    static ENUMERATIONS: Tally = OnceLock::new();

    /// Count one in `tally` for a schema whose root carries a string `$comment` — the tag a test
    /// gives its fixture, so that suites reading other shapes in parallel cannot move its count.
    fn count(tally: &Tally, schema: &Value) {
        if let Some(tag) = schema.get("$comment").and_then(Value::as_str) {
            let mut counts = tally
                .get_or_init(Mutex::default)
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *counts.entry(tag.to_owned()).or_default() += 1;
        }
    }

    fn read(tally: &Tally, tag: &str) -> usize {
        tally
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(tag)
            .copied()
            .unwrap_or(0)
    }

    /// Count one enumeration of a schema by its tag.
    pub(super) fn record(schema: &Value) {
        count(&ENUMERATIONS, schema);
    }

    /// How many times [`askable`](super::askable) has enumerated, in this process, a schema whose
    /// root `$comment` is `tag`.
    pub fn enumerations_tagged(tag: &str) -> usize {
        read(&ENUMERATIONS, tag)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Every permit held, no shape work runs: the work waits for a permit rather than fanning out
    /// to the blocking pool. Deterministic in the green direction — the work cannot start without
    /// a permit. Bite: delete the `acquire` (and `let _held = permit;`) from
    /// `with_schema_off_the_executor`; the trivial work then finishes well inside the wait and the
    /// first assertion goes red.
    #[tokio::test]
    async fn shape_work_waits_for_a_compile_permit() {
        let permits = u32::try_from(compile_permits()).expect("a handful of permits");
        assert!(permits >= 1);
        let held = COMPILE_PERMITS
            .acquire_many(permits)
            .await
            .expect("the permits are open");
        let schema = Value::Null;
        let work = with_schema_off_the_executor(&schema, |_| 7);
        let waited = tokio::time::timeout(Duration::from_millis(500), work).await;
        assert!(
            waited.is_err(),
            "no shape work runs while every permit is held"
        );
        drop(held);
        let ran = with_schema_off_the_executor(&schema, |_| 7).await;
        assert_eq!(ran.expect("runs once a permit is free"), 7);
    }

    /// The map never holds more than its cap, and a verdict remembered after eviction began reads
    /// back. That a remembered verdict spares a compile is witnessed by the integration test
    /// `a_shape_read_many_times_is_compiled_once`.
    #[test]
    fn the_verdict_map_is_capped() {
        for _ in 0..MAX_REMEMBERED_VERDICTS + 8 {
            remember(ShapeId::new(), SchemaVerdict::Compiles);
        }
        let id = ShapeId::new();
        remember(id, SchemaVerdict::Uncompilable);
        assert_eq!(remembered(id), Some(SchemaVerdict::Uncompilable));
        let held = VERDICTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        assert!(held <= MAX_REMEMBERED_VERDICTS, "{held}");
    }
}
