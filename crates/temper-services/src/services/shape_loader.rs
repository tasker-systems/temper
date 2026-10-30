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
//! read many times is compiled about once per process. The vocabulary a compiling shape's by-id
//! read advertises is remembered by id as well, bounded by an estimate of the memory it holds
//! rather than by how many there are, and the read looks it up before the verdict, so the same
//! shape is enumerated, and judged, about once per process too — while its vocabulary stays
//! remembered; one heavier than the whole cap is enumerated on each read. What a path refusal on a
//! shape teaches of its vocabulary ([`PathTeaching`]) is remembered by id in the same way, so a
//! request refusing a path on a shape an earlier request already taught enumerates nothing. The
//! constructor holds each `pattern` and `patternProperties` regex to
//! `shape_schema::MAX_PATTERN_BYTES`. A shape row is immutable per id (amendment mints a new id), so
//! a remembered verdict, vocabulary or teaching never goes stale. Each is consulted only with a row
//! a gated read returned — the loader reads its own, and the by-id read is passed only such rows —
//! and a teaching is remembered only for a shape the request's load returned readable and
//! compiling (`ShapeLoad::Loaded`), so none answers anything about a shape the caller cannot read.
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
    enumerate, within_schema_bound, AskOperator, AskSession, AskablePath, BoundKind, JsonType,
    LoadedShape, PathFacts, PathTeaching, RefusalTeaching, ShapeContext, ShapeLoad, ShapeSuccessor,
    Vocabulary, WalkBounds,
};
use temper_core::types::ids::{ProfileId, ShapeId};
use temper_core::types::query::filter::{property_value_bytes, property_value_nodes};
use temper_core::types::query::PlanRefusal;
use temper_substrate::readback::{self, RetrievedShape};
use temper_substrate::shape_schema;

/// Load every shape one request's asks name, for `principal`, into the context its asks are
/// validated against ([`ShapeContext::session`]). One [`load_one`] per id, in id order.
///
/// **What it costs is per distinct shape**, and the caller bounds how many it passes: each id is
/// one gated read (the stored schema whole), one bounded measurement, at most one compile within
/// the walk's byte bound, and — once validation teaches its vocabulary, if no teaching of it is
/// remembered — one enumeration, linear in the schema. The asks then share one budget for
/// membership ([`AskSession`]); each path they name is resolved once per request.
///
/// **Each shape loaded live and compiling carries its remembered [`PathTeaching`]**, if the
/// process remembers one (`remember_path_teachings`), so a path refusal on it teaches without
/// enumerating. Only such a shape's is looked up: a shape the gated read did not return, or one it
/// returned folded or not compiling, is never walked, and nothing remembered of it is consulted.
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

/// The context of these loads, each shape loaded under its own id carrying the [`PathTeaching`]
/// the process remembers for it, if any (`remember_path_teachings`). Only a loaded shape's is
/// looked up: one whose load refused is never walked.
pub(crate) fn context_of(loads: Vec<(ShapeId, ShapeLoad)>) -> ShapeContext {
    let taught: Vec<PathTeaching> = loads
        .iter()
        .filter(|(_, load)| matches!(load, ShapeLoad::Loaded(_)))
        .filter_map(|(id, _)| remembered_path_teaching(*id))
        .collect();
    let context: ShapeContext = loads.into_iter().collect();
    taught
        .into_iter()
        .fold(context, ShapeContext::with_path_teaching)
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

/// The most bytes of vocabulary the process remembers, each entry weighed as an estimate of the
/// memory it holds (`vocabulary_weight`).
///
/// # 32 MiB
///
/// A count cap would not bound this: what one vocabulary copies is bounded by its schema's size
/// (`MAX_SCHEMA_BYTES`), not by a count, so in this weight one ranges from a few KiB for an
/// ordinary shape to several MiB, and past the cap for values built of many small objects. Weighed
/// instead, 32 MiB holds thousands of ordinary ones, or one of the heaviest that fit. Past the
/// cap an arbitrary entry is forgotten, and that shape is enumerated again when it is next read; a
/// vocabulary heavier than the whole cap is never remembered, and is enumerated on each read.
///
/// # What the weight bounds
///
/// An entry is weighed by what it holds, not by what it serializes to: its place in the map
/// (`ENTRY_BYTES`); each path its own size, its segments' `String`s and their bytes, and its
/// operators; each declared value at least `NODE_BYTES` and never less than a `Value`, plus its
/// text; each node nested in a value at `NODE_BYTES`; and each object in a value the map nodes its
/// members sit in (`MAP_NODE_BYTES`). A short code that serializes to two bytes is charged as the
/// `Value` it occupies, and a container as the allocations it holds, so what the map holds in
/// memory is its weight within a small factor — allocator rounding, the table's spare slots — and
/// 32 MiB of weight is about that much memory.
const MAX_REMEMBERED_VOCABULARY_BYTES: usize = 32 << 20;

/// What one entry holds beside what it remembers: its key and value in the map's table, a control
/// byte, and the `Arc`'s two counts.
const fn entry_bytes<V>() -> usize {
    size_of::<ShapeId>() + size_of::<(V, usize)>() + 1 + 2 * size_of::<usize>()
}

/// What one vocabulary entry holds beside its vocabulary (`entry_bytes`).
const ENTRY_BYTES: usize = entry_bytes::<Arc<Vocabulary>>();

/// What one `Value` nested in a declared value is weighed at, beside its text: the `Value` itself
/// (32 bytes) in a container's slot, and its share of the allocation that holds it.
///
/// # 64
///
/// An estimate, and low for an object: a copied non-empty map allocates about 630 bytes, which
/// [`MAP_NODE_BYTES`] adds for each one. The const/enum bound at declare decides whether a value
/// can nest deeply enough for the difference to matter (task
/// `01a187e3-f00b-706f-b217-c49022239529`).
const NODE_BYTES: usize = 64;

/// One leaf node of a JSON object's map: eleven key and value slots and the node's own header. A
/// non-empty object copied into a vocabulary holds at least one, however few members it has, and
/// a node other than the root holds at least five members, so an object of `n` members holds at
/// most `n.div_ceil(5)` nodes. An internal node holds its children's pointers besides, a few dozen
/// bytes more, which each member's `NODE_BYTES` more than covers. Under serde_json's
/// `preserve_order` the map is an `IndexMap`, which this estimates about as closely.
const MAP_NODE_BYTES: usize = 11 * (size_of::<String>() + size_of::<Value>()) + 16;

/// What the process remembers by shape id — a vocabulary, or a path teaching — with each entry's
/// weight, and what they weigh together. The running total lives beside the map, under one lock,
/// so it cannot drift from what the map holds. `V` is a shared handle, so reading an entry clones
/// a pointer.
#[derive(Debug)]
struct RememberedById<V> {
    /// Each entry and its weight.
    entries: HashMap<ShapeId, (V, usize)>,
    bytes: usize,
}

impl<V> Default for RememberedById<V> {
    fn default() -> Self {
        RememberedById {
            entries: HashMap::new(),
            bytes: 0,
        }
    }
}

/// The vocabularies the by-id read remembers.
type RememberedVocabularies = RememberedById<Arc<Vocabulary>>;

impl<V: Clone> RememberedById<V> {
    fn get(&self, id: ShapeId) -> Option<V> {
        self.entries
            .get(&id)
            .map(|(remembered, _)| remembered.clone())
    }

    /// Remember `value`, weighing `weight`, forgetting arbitrary entries in one pass over the
    /// map, stopping as soon as it fits `budget`. One heavier than the whole budget is not
    /// remembered.
    ///
    /// Returns every value it let go of — the ones forgotten, the one `id` held before, or
    /// `value` itself when it is not remembered — so the caller frees them once the lock is
    /// released, not while other reads wait on it.
    #[must_use]
    fn insert(&mut self, id: ShapeId, value: V, weight: usize, budget: usize) -> Vec<V> {
        if weight > budget {
            return vec![value];
        }
        let mut let_go = Vec::new();
        if let Some((replaced, replaced_weight)) = self.entries.remove(&id) {
            self.bytes -= replaced_weight;
            let_go.push(replaced);
        }
        if self.bytes + weight > budget {
            // Dropping the pass part-way keeps every entry it has not reached.
            for (_, (evicted, held)) in self.entries.extract_if(|_, _| true) {
                self.bytes -= held;
                let_go.push(evicted);
                if self.bytes + weight <= budget {
                    break;
                }
            }
        }
        self.entries.insert(id, (value, weight));
        self.bytes += weight;
        let_go
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

/// What a vocabulary weighs in the map: an estimate of the memory it holds, in the walk's model
/// (see `MAX_REMEMBERED_VOCABULARY_BYTES`). Its place in the map, its own size, and each path's.
/// `truncated` holds at most one of each bound, negligible beside the paths, and is left at the
/// vocabulary's own size. Paths are weighed by their count, so a vocabulary is remembered with
/// `paths` shrunk to it.
fn vocabulary_weight(vocabulary: &Vocabulary) -> usize {
    ENTRY_BYTES + size_of::<Vocabulary>() + vocabulary.paths.iter().map(path_weight).sum::<usize>()
}

/// One path: its own size, each segment's `String` and bytes, its operators, and its facts.
fn path_weight(path: &AskablePath) -> usize {
    size_of::<AskablePath>()
        + path
            .path
            .iter()
            .map(|segment| size_of::<String>() + segment.len())
            .sum::<usize>()
        + path.operators.len() * size_of::<AskOperator>()
        + facts_weight(&path.facts)
}

/// One path's facts: their own size, the declared types, each declared value, and an element's
/// facts, as deep as they are reported.
fn facts_weight(facts: &PathFacts) -> usize {
    size_of::<PathFacts>()
        + facts
            .types
            .as_ref()
            .map_or(0, |types| types.len() * size_of::<JsonType>())
        + facts
            .values
            .iter()
            .flatten()
            .map(value_weight)
            .sum::<usize>()
        + facts.items.as_deref().map_or(0, facts_weight)
}

/// One declared value, as a vocabulary copies it: at least `NODE_BYTES`, and never less
/// than the `Value` it occupies; each node nested in it at `NODE_BYTES`; the map nodes of each
/// object in it; and its compact text, counted through a sink so that weighing a large value
/// allocates nothing.
fn value_weight(value: &Value) -> usize {
    size_of::<Value>().max(NODE_BYTES)
        + NODE_BYTES * property_value_nodes(value)
        + map_node_weight(value)
        + property_value_bytes(value)
}

/// The map nodes every non-empty object in `value` holds, at any depth (`MAP_NODE_BYTES`).
fn map_node_weight(value: &Value) -> usize {
    match value {
        Value::Array(xs) => xs.iter().map(map_node_weight).sum(),
        Value::Object(m) if !m.is_empty() => {
            MAP_NODE_BYTES * m.len().div_ceil(5) + m.values().map(map_node_weight).sum::<usize>()
        }
        _ => 0,
    }
}

/// Remember `vocabulary` for `id`, weighing `weight`. What the map lets go of is freed here, after
/// its lock is released.
fn remember_vocabulary(id: ShapeId, vocabulary: Arc<Vocabulary>, weight: usize) {
    if weight > MAX_REMEMBERED_VOCABULARY_BYTES {
        return;
    }
    let let_go = VOCABULARIES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id, vocabulary, weight, MAX_REMEMBERED_VOCABULARY_BYTES);
    drop(let_go);
}

/// The most bytes of path teaching the process remembers, each entry weighed as an estimate of the
/// memory it holds, in the vocabulary map's model (`teaching_weight`).
///
/// # 16 MiB, by weight rather than by count
///
/// A teaching is held to `MAX_TEACHING_BYTES` (16 KiB) on the wire, but what it holds in memory
/// depends on what its values are built of, so a count cap would bound it only at the worst
/// case. Weighed as a vocabulary is, a declared value costs at least `NODE_BYTES` plus its text,
/// and a value with an object in it a map node more (`MAP_NODE_BYTES`): two wire bytes of a short
/// code weigh about 65, and the eight of a one-member object about 770. So a teaching ranges from
/// a few KiB of weight for an ordinary shape — dozens of typed paths, a few short enums — to about
/// 1.8 MiB for the heaviest, 16 KiB of one-member objects. 16 MiB holds about a thousand ordinary
/// teachings, or eight of the heaviest; a count cap of a thousand would have had to admit 1.8 GiB.
/// Past the cap an arbitrary entry is forgotten, and that shape is enumerated again the next time a
/// request refuses a path on it; a teaching heavier than the whole cap is never remembered.
const MAX_REMEMBERED_TEACHING_BYTES: usize = 16 << 20;

/// Each loaded shape's path teaching, by id, for the life of the process.
static PATH_TEACHINGS: LazyLock<Mutex<RememberedById<PathTeaching>>> =
    LazyLock::new(Mutex::default);

fn remembered_path_teaching(id: ShapeId) -> Option<PathTeaching> {
    PATH_TEACHINGS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(id)
}

/// What a path teaching weighs in the map: an estimate of the memory it holds, in the vocabulary
/// map's model (`MAX_REMEMBERED_VOCABULARY_BYTES`). Its place in the map, the teaching's own size,
/// and each path it sends. A non-empty `truncated` set holds one B-tree leaf node, at most one entry
/// per bound, weighed as one; its elision is inline, left at the teaching's own size; its paths are
/// held with no spare capacity, so they are weighed by their count.
fn teaching_weight(teaching: &PathTeaching) -> usize {
    entry_bytes::<PathTeaching>()
        + size_of::<RefusalTeaching>()
        + teaching
            .teaching()
            .vocabulary
            .iter()
            .flat_map(|vocabulary| &vocabulary.paths)
            .map(path_weight)
            .sum::<usize>()
        + teaching
            .teaching()
            .vocabulary
            .as_ref()
            .filter(|vocabulary| !vocabulary.truncated.is_empty())
            .map_or(0, |_| TRUNCATED_NODE_BYTES)
}

/// A B-tree leaf node of a `truncated` set: eleven keys, a parent pointer, and its index and length.
const TRUNCATED_NODE_BYTES: usize = 11 * size_of::<BoundKind>() + size_of::<usize>() + 4;

/// Remember each path teaching one request's ask session made ([`AskSession::taught`]), by the
/// shape it teaches, for later requests' contexts ([`load`]). Called once the request's asks are
/// validated, whether they were admitted or refused — a refusal is what makes a teaching.
///
/// **Only a teaching of a shape `shapes` holds as loaded, under that id, is remembered** — one the
/// request's gated read returned and that compiles. A session makes a teaching only for such a
/// shape, since it walks no other; the check keeps that true of what this map holds whatever a
/// caller passes. What the map lets go of is freed here, after its lock is released.
///
/// [`AskSession::taught`]: temper_core::types::artifact_ask::AskSession::taught
pub(crate) fn remember_path_teachings(
    shapes: &ShapeContext,
    taught: impl IntoIterator<Item = PathTeaching>,
) {
    for teaching in taught {
        let id = teaching.shape();
        let loaded_here = matches!(
            shapes.get(&id),
            Some(ShapeLoad::Loaded(shape)) if shape.shape_id == id
        );
        if !loaded_here {
            continue;
        }
        let weight = teaching_weight(&teaching);
        if weight > MAX_REMEMBERED_TEACHING_BYTES {
            continue;
        }
        let let_go = PATH_TEACHINGS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, teaching, weight, MAX_REMEMBERED_TEACHING_BYTES);
        drop(let_go);
    }
}

/// Once one request's asks are validated, remember each path teaching its session made, by the
/// shape it teaches ([`remember_path_teachings`]), whether the asks were admitted or refused — a
/// refusal is what makes a teaching. Every reader that validates asks calls this with the session
/// it validated them through: `/api/query`'s `prepare`, and the batch read.
pub(crate) fn remember_session(shapes: &ShapeContext, session: &AskSession<'_>) {
    #[cfg(feature = "test-db")]
    test_support::record_path_teachings(shapes, session.taught().cloned());
    remember_path_teachings(shapes, session.taught().cloned());
}

/// What a caller may ask of a shape, for its by-id read (spec §2.5): the vocabulary the walk
/// enumerates, and the refusal step 3 gives this version's schema when it cannot be walked — past
/// the byte bound (`askable` then says `truncated: [schema_bytes]`), or not compiling (`askable`
/// empty). The verdict is the loader's own ([`judge`], remembered by id), so the read and the
/// loader judge a schema alike.
///
/// A compiling shape's vocabulary is remembered by id too, bounded by bytes
/// (`MAX_REMEMBERED_VOCABULARY_BYTES`), and it is looked up first: only a compiling shape's is
/// remembered, so a read that finds one answers it with no refusal, and neither asks the verdict
/// nor takes a permit. A miss asks the verdict, and for a compiling shape enumerates and weighs
/// the vocabulary under a permit, off the executor, then remembers it. Two first reads that race
/// both enumerate; after that, none while it stays remembered. One heavier than the whole cap is
/// never remembered, and is enumerated on each read. A schema past the bound or not compiling is
/// not walked, so there is nothing to remember.
///
/// Takes the row the gated read returned, whole, so the id the verdict and the vocabulary are
/// remembered by and the schema they were derived from are one value's. `askable` is passed only
/// rows `readback`'s gated reads returned: its one caller, `substrate_read::get_shape`, passes the
/// row `readback::shape_by_id` gave it.
///
/// # Errors
/// Only if the permits are closed or a blocking task cannot be joined.
pub async fn askable(row: &RetrievedShape) -> ApiResult<(Vocabulary, Option<PlanRefusal>)> {
    if let Some(vocabulary) = remembered_vocabulary(row.shape_id) {
        return Ok((Vocabulary::clone(&vocabulary), None));
    }
    let verdict = verdict_of(row.shape_id, &row.schema).await?;
    let vocabulary = match verdict {
        // Within the bound: a walk of at most 256 KiB, linear in it, off the executor.
        SchemaVerdict::Compiles => {
            #[cfg(feature = "test-db")]
            test_support::record(&row.schema);
            let shape = row.shape_id;
            let (vocabulary, weight) = with_schema_off_the_executor(&row.schema, move |schema| {
                let _shape = tracing::error_span!("askable", %shape).entered();
                let mut vocabulary = enumerate(schema, &WalkBounds::BUILD);
                vocabulary.paths.shrink_to_fit();
                let weight = vocabulary_weight(&vocabulary);
                (vocabulary, weight)
            })
            .await?;
            let vocabulary = Arc::new(vocabulary);
            remember_vocabulary(row.shape_id, Arc::clone(&vocabulary), weight);
            Arc::unwrap_or_clone(vocabulary)
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
/// whichever read asked for it — and how often a request's ask session made a shape's path
/// teaching, each one an enumeration of its vocabulary.
#[cfg(feature = "test-db")]
pub mod test_support {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock, PoisonError};

    use serde_json::Value;
    use temper_core::types::artifact_ask::{PathTeaching, ShapeContext, ShapeLoad};

    type Tally = OnceLock<Mutex<HashMap<String, usize>>>;

    static ENUMERATIONS: Tally = OnceLock::new();
    static PATH_TEACHINGS_MADE: Tally = OnceLock::new();

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

    /// Count each path teaching a request's ask session made, by the tag of the loaded shape's
    /// schema it teaches. Called by `remember_session` beside remembering them, and apart from it,
    /// so a request that remembers nothing is still counted.
    pub(crate) fn record_path_teachings(
        shapes: &ShapeContext,
        made: impl IntoIterator<Item = PathTeaching>,
    ) {
        for teaching in made {
            if let Some(ShapeLoad::Loaded(shape)) = shapes.get(&teaching.shape()) {
                count(&PATH_TEACHINGS_MADE, &shape.schema);
            }
        }
    }

    /// How many path teachings — each one enumeration of the shape's vocabulary — requests' ask
    /// sessions have made, in this process, of a schema whose root `$comment` is `tag`.
    pub fn path_teachings_made_tagged(tag: &str) -> usize {
        read(&PATH_TEACHINGS_MADE, tag)
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
    use temper_core::types::artifact_ask::{ArtifactAsk, ArtifactPredicate};
    use temper_core::types::query::PropertyOp;

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
    /// the entry offered last reads back, the running total is the sum of the weights held, every
    /// entry forgotten is handed back to be freed, and an entry heavier than the whole budget is
    /// not remembered. That a remembered vocabulary spares an enumeration is witnessed by the
    /// integration test `a_shape_read_many_times_is_enumerated_once`. Bite: delete the
    /// `extract_if` pass that forgets entries from `RememberedById::insert`; the total passes the
    /// budget on the fifth entry.
    #[test]
    fn the_vocabulary_map_is_capped_by_bytes() {
        let each = vocabulary_of(1_000);
        let weight = vocabulary_weight(&each);
        let budget = 4 * weight + weight / 2;
        let mut map = RememberedVocabularies::default();

        let mut last = ShapeId::new();
        let mut let_go = 0;
        for _ in 0..20 {
            last = ShapeId::new();
            let_go += map.insert(last, Arc::clone(&each), weight, budget).len();
            assert!(map.bytes <= budget, "{} > {budget}", map.bytes);
        }
        assert_eq!(map.entries.len(), 4);
        assert_eq!(let_go, 16, "each entry forgotten is handed back");
        assert_eq!(
            map.bytes,
            map.entries.values().map(|(_, held)| held).sum::<usize>()
        );
        assert_eq!(map.get(last).as_deref(), Some(&*each));

        let too_heavy = ShapeId::new();
        let refused = map.insert(too_heavy, Arc::clone(&each), budget + 1, budget);
        assert_eq!(refused.len(), 1, "handed back, not held");
        assert!(map.get(too_heavy).is_none());
        assert!(map.bytes <= budget);
        assert!(map.get(last).is_some(), "nothing was forgotten for it");
    }

    /// An id offered again — two first reads that race both remember — replaces its entry rather
    /// than counting it twice: the entry count is unchanged, the running total is the sum of the
    /// weights held, and the replaced vocabulary is handed back. The budget leaves room, so nothing
    /// is forgotten for it either way. Bite: delete the block in `RememberedById::insert` that
    /// removes `id`'s entry first; the total counts the weight twice and the sum check goes red.
    #[test]
    fn a_vocabulary_remembered_again_is_weighed_once() {
        let each = vocabulary_of(1_000);
        let weight = vocabulary_weight(&each);
        let budget = 10 * weight;
        let mut map = RememberedVocabularies::default();

        let mut last = ShapeId::new();
        for _ in 0..3 {
            last = ShapeId::new();
            assert!(map
                .insert(last, Arc::clone(&each), weight, budget)
                .is_empty());
        }
        let replaced = map.insert(last, Arc::clone(&each), weight, budget);
        assert_eq!(map.entries.len(), 3);
        assert_eq!(
            map.bytes,
            map.entries.values().map(|(_, held)| held).sum::<usize>()
        );
        assert_eq!(replaced.len(), 1, "the replaced vocabulary is handed back");
    }

    /// A vocabulary is weighed by what it holds, not by what it serializes to: one path carrying a
    /// thousand copies of `0`, two bytes each on the wire, weighs at least a `Value` and at least
    /// `NODE_BYTES` a copy. Bite: return the compact serialized length from `vocabulary_weight`;
    /// that is about 2 KiB, and both assertions go red.
    #[test]
    fn a_vocabulary_weighs_what_it_holds() {
        let vocabulary = Vocabulary {
            paths: vec![AskablePath {
                path: vec!["code".to_owned()],
                facts: PathFacts {
                    values: Some(vec![Value::from(0); 1_000]),
                    ..PathFacts::default()
                },
                operators: vec![],
            }],
            truncated: BTreeSet::new(),
        };
        let weight = vocabulary_weight(&vocabulary);
        assert!(weight >= 1_000 * size_of::<Value>(), "{weight}");
        assert!(weight >= 1_000 * NODE_BYTES, "{weight}");
    }

    /// An object in a value is weighed with the map node its members sit in, which a copy holds
    /// however few members it has: one path carrying a thousand copies of `{"a": 0}`, seven bytes
    /// each on the wire, weighs at least a `Value` and a map node a copy. Bite: drop
    /// `map_node_weight` from `value_weight`; a copy then weighs about 135 bytes, and the
    /// assertion goes red.
    #[test]
    fn an_object_in_a_value_weighs_its_map_node() {
        let vocabulary = Vocabulary {
            paths: vec![AskablePath {
                path: vec!["point".to_owned()],
                facts: PathFacts {
                    values: Some(vec![serde_json::json!({ "a": 0 }); 1_000]),
                    ..PathFacts::default()
                },
                operators: vec![],
            }],
            truncated: BTreeSet::new(),
        };
        let weight = vocabulary_weight(&vocabulary);
        assert!(
            weight >= 1_000 * (size_of::<Value>() + MAP_NODE_BYTES),
            "{weight}"
        );
    }

    /// A wide object is weighed with every map node its members fill, not just one: sixty members
    /// sit in at least twelve nodes' worth of the estimate. Bite: charge one node per object
    /// (replace `m.len().div_ceil(5)` with `1` in `map_node_weight`); the object then weighs one
    /// node, and the assertion goes red.
    #[test]
    fn a_wide_object_weighs_every_node() {
        let wide: serde_json::Map<String, Value> =
            (0..60).map(|i| (format!("k{i}"), Value::from(0))).collect();
        let weight = map_node_weight(&Value::Object(wide));
        assert!(weight >= 12 * MAP_NODE_BYTES, "{weight}");
    }

    // ── Path teachings, remembered by shape id ────────────────────────────────────────────

    /// A shape loaded under a fresh id, declaring `schema`.
    fn loaded_shape(schema: Value) -> LoadedShape {
        LoadedShape {
            shape_id: ShapeId::new(),
            shape_version: 1,
            home_anchor_table: "kb_contexts".to_owned(),
            home_anchor_id: uuid::Uuid::nil(),
            kind_owner_table: "kb_teams".to_owned(),
            kind_owner_id: uuid::Uuid::nil(),
            artifact_kind: "measurement".to_owned(),
            schema,
        }
    }

    /// What a path refusal on `shape` teaches, made by an ask session as a request's is.
    fn teaching_of(shape: &LoadedShape) -> PathTeaching {
        let context: ShapeContext = [(shape.shape_id, ShapeLoad::Loaded(shape.clone()))]
            .into_iter()
            .collect();
        let mut session = context.session();
        let ask = ArtifactAsk {
            shape: shape.shape_id,
            predicates: vec![ArtifactPredicate {
                path: vec!["undeclared".to_owned()],
                op: PropertyOp::HasKey,
            }],
            intent: vec![],
            include_folded: false,
            shape_states: vec![],
        };
        session.validate(&ask).expect_err("the path is refused");
        let teaching = session
            .taught()
            .next()
            .cloned()
            .expect("the refusal made the shape's teaching");
        teaching
    }

    /// The teaching of a shape declaring one path whose single segment is `segment_bytes` long.
    fn teaching_of_one_path(segment_bytes: usize) -> PathTeaching {
        let mut properties = serde_json::Map::new();
        properties.insert("x".repeat(segment_bytes), serde_json::json!({}));
        let teaching = teaching_of(&loaded_shape(
            serde_json::json!({ "properties": properties }),
        ));
        assert_eq!(
            teaching
                .teaching()
                .vocabulary
                .as_ref()
                .map(|v| v.paths.len()),
            Some(1),
            "the one path is taught"
        );
        teaching
    }

    /// The teaching map holds no more than its byte budget, however many entries are offered, the
    /// entry offered last reads back, the running total is the sum of the weights held, every
    /// entry forgotten is handed back to be freed, and an entry heavier than the whole budget is
    /// not remembered. That a remembered teaching spares an enumeration is witnessed by the
    /// integration test `a_shape_refusing_paths_across_requests_is_enumerated_once`. Bite: delete
    /// the `extract_if` pass that forgets entries from `RememberedById::insert`; the total passes
    /// the budget on the fifth entry.
    #[test]
    fn the_teaching_map_is_capped_by_bytes() {
        let each = teaching_of_one_path(200);
        let weight = teaching_weight(&each);
        let budget = 4 * weight + weight / 2;
        let mut map = RememberedById::<PathTeaching>::default();

        let mut last = ShapeId::new();
        let mut let_go = 0;
        for _ in 0..20 {
            last = ShapeId::new();
            let_go += map.insert(last, each.clone(), weight, budget).len();
            assert!(map.bytes <= budget, "{} > {budget}", map.bytes);
        }
        assert_eq!(map.entries.len(), 4);
        assert_eq!(let_go, 16, "each entry forgotten is handed back");
        assert_eq!(
            map.bytes,
            map.entries.values().map(|(_, held)| held).sum::<usize>()
        );
        assert_eq!(map.get(last).as_ref(), Some(&each));

        let too_heavy = ShapeId::new();
        let refused = map.insert(too_heavy, each.clone(), budget + 1, budget);
        assert_eq!(refused.len(), 1, "handed back, not held");
        assert!(map.get(too_heavy).is_none());
        assert!(map.bytes <= budget);
        assert!(map.get(last).is_some(), "nothing was forgotten for it");
    }

    /// An id offered again — two requests that race both remember — replaces its entry rather than
    /// counting it twice: the entry count is unchanged, the running total is the sum of the weights
    /// held, and the replaced teaching is handed back. Bite: delete the block in
    /// `RememberedById::insert` that removes `id`'s entry first; the total counts the weight twice
    /// and the sum check goes red.
    #[test]
    fn a_teaching_remembered_again_is_weighed_once() {
        let each = teaching_of_one_path(200);
        let weight = teaching_weight(&each);
        let budget = 10 * weight;
        let mut map = RememberedById::<PathTeaching>::default();

        let mut last = ShapeId::new();
        for _ in 0..3 {
            last = ShapeId::new();
            assert!(map.insert(last, each.clone(), weight, budget).is_empty());
        }
        let replaced = map.insert(last, each.clone(), weight, budget);
        assert_eq!(map.entries.len(), 3);
        assert_eq!(
            map.bytes,
            map.entries.values().map(|(_, held)| held).sum::<usize>()
        );
        assert_eq!(replaced.len(), 1, "the replaced teaching is handed back");
    }

    /// A teaching is weighed by what it holds, not by what it sends: one path over an enum of a
    /// thousand short codes, about four KiB on the wire, weighs at least a `Value` and at least
    /// `NODE_BYTES` a code. Bite: return the teaching's compact serialized length from
    /// `teaching_weight`; that is about 4 KiB, and both assertions go red.
    #[test]
    fn a_teaching_weighs_what_it_holds() {
        let codes: Vec<Value> = (0..1_000).map(Value::from).collect();
        let teaching = teaching_of(&loaded_shape(serde_json::json!({
            "properties": { "code": { "enum": codes } }
        })));
        let sent: usize = teaching
            .teaching()
            .vocabulary
            .iter()
            .flat_map(|vocabulary| &vocabulary.paths)
            .map(|path| path.facts.values.as_ref().map_or(0, Vec::len))
            .sum();
        assert_eq!(sent, 1_000, "the teaching sends every code");
        let weight = teaching_weight(&teaching);
        assert!(weight >= 1_000 * size_of::<Value>(), "{weight}");
        assert!(weight >= 1_000 * NODE_BYTES, "{weight}");
    }

    /// **A teaching is remembered only for a shape the request loaded live and compiling, under
    /// its id.** Offered beside the same id loaded as refused — absent or unreadable, folded, not
    /// compiling, past the byte bound — or loaded under another id, or not loaded at all, it is
    /// not remembered; beside the shape loaded, it is. Bite: delete the `loaded_here` check in
    /// `remember_path_teachings`; the first refused load remembers it.
    #[test]
    fn only_a_teaching_of_a_shape_loaded_under_its_id_is_remembered() {
        let shape = loaded_shape(serde_json::json!({ "properties": { "unit": {} } }));
        let id = shape.shape_id;
        let teaching = teaching_of(&shape);
        let mut elsewhere = shape.clone();
        elsewhere.shape_id = ShapeId::new();
        for load in [
            ShapeLoad::NotAskable,
            ShapeLoad::Superseded { successor: None },
            ShapeLoad::Uncompilable,
            ShapeLoad::SchemaTooLarge,
            ShapeLoad::Loaded(elsewhere),
        ] {
            let refusing: ShapeContext = [(id, load)].into_iter().collect();
            remember_path_teachings(&refusing, [teaching.clone()]);
            assert_eq!(remembered_path_teaching(id), None, "{refusing:?}");
        }
        remember_path_teachings(&ShapeContext::empty(), [teaching.clone()]);
        assert_eq!(
            remembered_path_teaching(id),
            None,
            "a shape the request did not load"
        );

        let loaded: ShapeContext = [(id, ShapeLoad::Loaded(shape))].into_iter().collect();
        remember_path_teachings(&loaded, [teaching.clone()]);
        assert_eq!(remembered_path_teaching(id), Some(teaching));
    }
}
