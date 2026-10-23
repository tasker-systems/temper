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
//! read many times is compiled about once per process. The vocabulary a compiling shape's by-id
//! read advertises is remembered by id as well, bounded by the bytes it weighs rather than by how
//! many there are, so the same shape is enumerated about once per process too. The constructor
//! holds each `pattern` and `patternProperties` regex to `shape_schema::MAX_PATTERN_BYTES`. A
//! shape row is immutable per id (amendment mints a new id), so a remembered verdict or vocabulary
//! never goes stale, and each is consulted only for a row the gated read already returned, so
//! neither answers anything about a shape the caller cannot read.
//!
//! The successor lookup reads only the home and family of the row step 1 returned; no caller text
//! reaches SQL here except the shape id, bound as a `uuid`.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

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
/// further than the bound), and only within it compile through the no-fetch constructor — on the
/// blocking pool, under a compile permit. The schema is copied for the blocking task only once it
/// is known to be within the bound and a permit is held, so the copy is at most 256 KiB and no
/// request waiting for a permit holds one.
///
/// # Errors
/// Only if the permits are closed or the blocking task cannot be joined.
pub async fn judge(schema: &Value) -> ApiResult<SchemaVerdict> {
    if !within_schema_bound(schema) {
        return Ok(SchemaVerdict::TooLarge);
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

/// The most bytes of vocabulary the process remembers, each weighed as its compact serialized
/// length.
///
/// # 32 MiB
///
/// A count cap would not bound this: one vocabulary is at most about 4 MiB of path text and
/// declared values — the walk charges both against `MAX_WALK_VISITS` at `BYTES_PER_VISIT` — plus
/// the structure around up to `MAX_WALK_PATHS` paths, so the largest shapes alone would fill a
/// count cap many times over. Weighed instead, 32 MiB holds about eight of the largest and, since
/// an ordinary shape's vocabulary is a few KiB, thousands of ordinary ones. What the map holds in
/// memory is a small multiple of its weight. Past the cap an arbitrary entry is forgotten, and
/// that shape is enumerated again when it is next read.
const MAX_REMEMBERED_VOCABULARY_BYTES: usize = 32 << 20;

/// The vocabularies remembered, by shape id, and what they weigh together. The running total
/// lives beside the map, under one lock, so it cannot drift from what the map holds.
#[derive(Debug, Default)]
struct RememberedVocabularies {
    /// Each entry's vocabulary and its weight.
    entries: HashMap<ShapeId, (Arc<Vocabulary>, usize)>,
    bytes: usize,
}

impl RememberedVocabularies {
    fn get(&self, id: ShapeId) -> Option<Arc<Vocabulary>> {
        self.entries
            .get(&id)
            .map(|(vocabulary, _)| Arc::clone(vocabulary))
    }

    /// Remember `vocabulary`, weighing `weight`, forgetting arbitrary entries until it fits
    /// `budget`. One heavier than the whole budget is not remembered.
    fn insert(&mut self, id: ShapeId, vocabulary: Arc<Vocabulary>, weight: usize, budget: usize) {
        if weight > budget {
            return;
        }
        if let Some((_, replaced)) = self.entries.remove(&id) {
            self.bytes -= replaced;
        }
        while self.bytes + weight > budget {
            let Some(evicted) = self.entries.keys().next().copied() else {
                break;
            };
            if let Some((_, forgotten)) = self.entries.remove(&evicted) {
                self.bytes -= forgotten;
            }
        }
        self.entries.insert(id, (vocabulary, weight));
        self.bytes += weight;
    }
}

/// Each compiling shape's vocabulary, by id, for the life of the process.
static VOCABULARIES: LazyLock<Mutex<RememberedVocabularies>> = LazyLock::new(Mutex::default);

fn remembered_vocabulary(id: ShapeId) -> Option<Arc<Vocabulary>> {
    VOCABULARIES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(id)
}

/// What a vocabulary weighs in the map: its compact serialized length, or `None` if it somehow
/// does not serialize.
fn vocabulary_weight(vocabulary: &Vocabulary) -> Option<usize> {
    serde_json::to_vec(vocabulary).ok().map(|bytes| bytes.len())
}

/// Remember `vocabulary` for `id`, weighed once here. One that does not serialize is not
/// remembered: it is returned to its caller all the same, and enumerated again next time.
fn remember_vocabulary(id: ShapeId, vocabulary: &Vocabulary) {
    let Some(weight) = vocabulary_weight(vocabulary) else {
        return;
    };
    if weight > MAX_REMEMBERED_VOCABULARY_BYTES {
        return;
    }
    let vocabulary = Arc::new(vocabulary.clone());
    VOCABULARIES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id, vocabulary, weight, MAX_REMEMBERED_VOCABULARY_BYTES);
}

/// What a caller may ask of a shape, for its by-id read (spec §2.5): the vocabulary the walk
/// enumerates, and the refusal step 3 gives this version's schema when it cannot be walked — past
/// the byte bound (`askable` then says `truncated: [schema_bytes]`), or not compiling (`askable`
/// empty). The verdict is the loader's own ([`judge`], remembered by id), so the read and the
/// loader judge a schema alike.
///
/// A compiling shape's vocabulary is remembered by id too, bounded by bytes
/// ([`MAX_REMEMBERED_VOCABULARY_BYTES`]): it is looked up before a permit is taken, and only a miss
/// enumerates, under the permit. Two first reads that race both enumerate; after that, none. A
/// schema past the bound or not compiling is not walked, so there is nothing to remember.
///
/// Takes the row the gated read returned, whole, so the id the verdict and the vocabulary are
/// remembered by and the schema they were derived from are one value's. `RetrievedShape` is built
/// only by `readback`'s gated reads, so its caller holds a row the gate admitted.
///
/// # Errors
/// Only if the permits are closed or a blocking task cannot be joined.
pub async fn askable(row: &RetrievedShape) -> ApiResult<(Vocabulary, Option<PlanRefusal>)> {
    let verdict = verdict_of(row.shape_id, &row.schema).await?;
    let vocabulary = match verdict {
        // Within the bound: a walk of at most 256 KiB, bounded in work, off the executor.
        SchemaVerdict::Compiles => match remembered_vocabulary(row.shape_id) {
            Some(vocabulary) => Vocabulary::clone(&vocabulary),
            None => {
                #[cfg(feature = "test-db")]
                test_support::record(&row.schema);
                let vocabulary = with_schema_off_the_executor(&row.schema, |schema| {
                    enumerate(schema, &WalkBounds::BUILD)
                })
                .await?;
                remember_vocabulary(row.shape_id, &vocabulary);
                vocabulary
            }
        },
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

/// What a test reads to see how often [`askable`] enumerated a compiling shape — every enumeration,
/// whichever read asked for it.
#[cfg(feature = "test-db")]
pub mod test_support {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock, PoisonError};

    use serde_json::Value;

    static ENUMERATIONS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

    /// Count one enumeration of a schema whose root carries a string `$comment` — the tag a test
    /// gives its fixture, so that suites reading other shapes in parallel cannot move its count.
    pub(super) fn record(schema: &Value) {
        if let Some(tag) = schema.get("$comment").and_then(Value::as_str) {
            let mut enumerations = ENUMERATIONS
                .get_or_init(Mutex::default)
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *enumerations.entry(tag.to_owned()).or_default() += 1;
        }
    }

    /// How many times [`askable`](super::askable) has enumerated, in this process, a schema whose
    /// root `$comment` is `tag`.
    pub fn enumerations_tagged(tag: &str) -> usize {
        ENUMERATIONS
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(tag)
            .copied()
            .unwrap_or(0)
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
    use temper_core::types::artifact_ask::{AskablePath, PathFacts};

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

    /// A vocabulary of one path whose single segment is `segment_bytes` long.
    fn vocabulary_of(segment_bytes: usize) -> Arc<Vocabulary> {
        Arc::new(Vocabulary {
            paths: vec![AskablePath {
                path: vec!["x".repeat(segment_bytes)],
                facts: PathFacts::default(),
                operators: vec![],
            }],
            truncated: BTreeSet::new(),
        })
    }

    /// The vocabulary map holds no more than its byte budget, however many entries are offered,
    /// the entry offered last reads back, the running total is the sum of the weights held, and an
    /// entry heavier than the whole budget is not remembered. That a remembered vocabulary spares
    /// an enumeration is witnessed by the integration test
    /// `a_shape_read_many_times_is_enumerated_once`. Bite: delete the `while` that forgets entries
    /// from `RememberedVocabularies::insert`; the total passes the budget on the fifth entry.
    #[test]
    fn the_vocabulary_map_is_capped_by_bytes() {
        let each = vocabulary_of(1_000);
        let weight = vocabulary_weight(&each).expect("a vocabulary serializes");
        let budget = 4 * weight + weight / 2;
        let mut map = RememberedVocabularies::default();

        let mut last = ShapeId::new();
        for _ in 0..20 {
            last = ShapeId::new();
            map.insert(last, Arc::clone(&each), weight, budget);
            assert!(map.bytes <= budget, "{} > {budget}", map.bytes);
        }
        assert_eq!(map.entries.len(), 4);
        assert_eq!(
            map.bytes,
            map.entries.values().map(|(_, held)| held).sum::<usize>()
        );
        assert_eq!(map.get(last).as_deref(), Some(&*each));

        let too_heavy = ShapeId::new();
        map.insert(too_heavy, Arc::clone(&each), budget + 1, budget);
        assert!(map.get(too_heavy).is_none());
        assert!(map.bytes <= budget);
        assert!(map.get(last).is_some(), "nothing was forgotten for it");
    }
}
