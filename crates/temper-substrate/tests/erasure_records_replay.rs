#![cfg(feature = "test-db")]
//! The erasure record's projectors and their replay (erasure estate-membership spec D1, D1.1, D6
//! *Replay*, D7.5; migration 20261201000030_erasure_record_projectors.sql).
//!
//! Each test appends erasure record events through `_event_append` under the canonical system
//! actor and then calls the kind's projector in the same transaction, as a live writer does. The
//! payloads are built from the `payloads::Erasure*` structs, so the keys the projectors read are
//! the keys the Rust contract writes.
//!
//! Witnesses:
//!   * one per projector: `erasure_declared` (the erasure, the estate snapshot, the map with a
//!     born-erased row, the former holders, the flags of pending members only, and a legacy
//!     declaration), `erasure_member_arrived`, `erasure_scope_sealed`, `erasure_reopened`,
//!     `erasure_completed` and `erasure_batch_applied`;
//!   * the projectors refuse a writer's defect rather than absorb it: a legacy declaration that
//!     carries a manifest or a request, an estate whose counts omit a table its map holds, a map
//!     entry with half a prior, and a refusal that names no erasure or a row that is not open;
//!   * D7.5: replay rebuilds the four record tables byte for byte over a sequence holding a
//!     refusal and a born-erased row, and the checksum recomputed from the rebuilt map equals the
//!     `erasure_declared` payload's for every non-legacy erasure;
//!   * the golden checksum: one fixed map's D1.1 checksum, pinned to a literal;
//!   * the walk order: with ids forced to invert the order the events were written (in one
//!     transaction, and across two), replay still rebuilds what the live projectors wrote: an
//!     arrival after its declaration, a reopening after its completion, a second refusal after the
//!     first; and with a transaction that took its `xact_id` before another committed and then
//!     appended after that commit, replay walks its later append after the other's;
//!   * a declaration's counts, checksum and holders are verified against its map, a field its
//!     kind does not carry raises, and so does a non-legacy declaration without its checksum;
//!   * the edge fold: an edge member folds with its edge-owned properties, an edge folded before
//!     keeps its own fold, and `relationship_folded` still folds and raises through the shared
//!     helper.

mod common;

use common::erasure_map::d1_1_checksum;

use sqlx::{PgPool, Row};
use std::collections::BTreeMap;
use temper_core::types::ids::EntityId;
use temper_core::types::property_owner::PropertyOwner;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, EdgeId, ProfileId, ResourceId};
use temper_substrate::payloads::{
    AnchorRef, EdgePolarity, ErasureArrivalSource, ErasureBatchApplied, ErasureBatchEntry,
    ErasureBatchOutcome, ErasureCompleted, ErasureDeclared, ErasureKind, ErasureMapEntry,
    ErasureMemberArrived, ErasureMemberRef, ErasureMemberRefusalReason, ErasureMemberTable,
    ErasureReopened, ErasureScopeSealed,
};
use temper_substrate::replay;
use temper_substrate::writes::{self, AssertParams, CreateParams};
use uuid::Uuid;

/// A manifest digest of the right shape. The projectors never interpret it.
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const MANIFEST_VERSION: &str = "1.0.0";

// ── the world ────────────────────────────────────────────────────────────────────────────────

async fn fresh(pool: &PgPool) {
    common::reset_schema(pool).await;
    temper_substrate::scenario::bootseed::seed_system(pool)
        .await
        .unwrap();
}

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: EntityId =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), entity)
}

async fn make_context(pool: &PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

async fn make_resource(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    slug: &str,
) -> ResourceId {
    let origin = format!("test://erasure-records/{slug}");
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: slug,
            origin_uri: &origin,
            body: "body",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap()
}

/// An edge src → tgt homed in `home`, carrying one edge-owned facet.
async fn make_edge(
    pool: &PgPool,
    emitter: EntityId,
    home: ContextId,
    src: ResourceId,
    tgt: ResourceId,
    label: &str,
) -> EdgeId {
    let edge = writes::assert_relationship(
        pool,
        AssertParams {
            src,
            tgt,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some(label),
            weight: 1.0,
            home,
            emitter,
        },
    )
    .await
    .unwrap();
    writes::set_facet(
        pool,
        PropertyOwner::edge(edge),
        &serde_json::json!({ "clause": label }),
        1.0,
        emitter,
    )
    .await
    .unwrap();
    edge
}

async fn fold_edge(pool: &PgPool, emitter: EntityId, edge: EdgeId) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::RelationshipFold {
            edge,
            reason: Some("folded before the erasure"),
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    sqlx::query_scalar("SELECT last_event_id FROM kb_edges WHERE id = $1")
        .bind(edge.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

// ── the record's events ──────────────────────────────────────────────────────────────────────

/// Append one erasure record event under the system actor and project it in the same
/// transaction, as the live path does. Returns the event id, or the error either step raised.
///
/// Each record is its own transaction, after the one before it, and replay walks events in append
/// order (`replay::replay`), so the records replay in the order they were written, whatever order
/// `uuid_generate_v7()` gave their ids. The ordering witnesses below force the ids to disagree
/// with that order.
async fn try_record(
    pool: &PgPool,
    kind: &str,
    payload: &serde_json::Value,
) -> Result<Uuid, sqlx::Error> {
    let (_, emitter) = system_actor(pool).await;
    let mut tx = pool.begin().await?;
    let event: Uuid =
        sqlx::query_scalar("SELECT _event_append($1, $2, NULL, NULL, $3, p_correlation => $4)")
            .bind(kind)
            .bind(emitter)
            .bind(payload)
            .bind(Uuid::now_v7())
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query(&format!("SELECT _project_{kind}($1, $2)"))
        .bind(event)
        .bind(payload)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(event)
}

async fn record(pool: &PgPool, kind: &str, payload: &serde_json::Value) -> Uuid {
    try_record(pool, kind, payload)
        .await
        .unwrap_or_else(|e| panic!("{kind} must append and project: {e}"))
}

fn member(table: ErasureMemberTable, id: Uuid) -> ErasureMemberRef {
    ErasureMemberRef { table, id }
}

fn pending(
    table: ErasureMemberTable,
    id: Uuid,
    parent: Option<ErasureMemberRef>,
) -> ErasureMapEntry {
    ErasureMapEntry {
        table,
        id,
        parent,
        prior_erasure: None,
        prior_event: None,
    }
}

fn born_erased(
    table: ErasureMemberTable,
    id: Uuid,
    parent: Option<ErasureMemberRef>,
    prior_erasure: Uuid,
    prior_event: Uuid,
) -> ErasureMapEntry {
    ErasureMapEntry {
        table,
        id,
        parent,
        prior_erasure: Some(prior_erasure),
        prior_event: Some(prior_event),
    }
}

/// A non-legacy declaration: a fresh request, the fixed manifest, the per-table counts (for an
/// estate) and the D1.1 checksum of `map`.
fn declaration(
    erasure: Uuid,
    kind: ErasureKind,
    subject: Option<(&str, Uuid)>,
    map: Vec<ErasureMapEntry>,
    holders: Vec<Uuid>,
) -> ErasureDeclared {
    let counts = (kind == ErasureKind::Estate).then(|| {
        let mut counts: BTreeMap<ErasureMemberTable, u64> = BTreeMap::new();
        for e in &map {
            *counts.entry(e.table).or_default() += 1;
        }
        counts
    });
    ErasureDeclared {
        erasure_id: erasure,
        kind,
        subject_table: subject.map(|(t, _)| t.to_owned()),
        subject_id: subject.map(|(_, id)| id),
        request_ref: Some(Uuid::now_v7()),
        manifest_version: Some(MANIFEST_VERSION.to_owned()),
        manifest_digest: Some(DIGEST.to_owned()),
        checksum: Some(d1_1_checksum(erasure, DIGEST, &map)),
        map,
        counts,
        holders,
        legacy: false,
    }
}

/// The legacy shape a door that names no erasure records: kind `resource`, the subject, a map, and
/// nothing else.
fn legacy_declaration(erasure: Uuid, resource: Uuid, map: Vec<ErasureMapEntry>) -> ErasureDeclared {
    ErasureDeclared {
        erasure_id: erasure,
        kind: ErasureKind::Resource,
        subject_table: Some("kb_resources".to_owned()),
        subject_id: Some(resource),
        request_ref: None,
        manifest_version: None,
        manifest_digest: None,
        map,
        counts: None,
        checksum: None,
        holders: vec![],
        legacy: true,
    }
}

async fn declare(pool: &PgPool, d: &ErasureDeclared) -> Uuid {
    record(pool, "erasure_declared", &serde_json::to_value(d).unwrap()).await
}

fn refused(
    member: Uuid,
    erasures: Vec<Uuid>,
    reason: ErasureMemberRefusalReason,
) -> ErasureBatchEntry {
    ErasureBatchEntry {
        member,
        erasures,
        outcome: ErasureBatchOutcome::Refused,
        authority_event: None,
        reason: Some(reason),
        sqlstate: Some("XX000".to_owned()),
        routine: Some("resource_erasure_execute".to_owned()),
    }
}

async fn batch(pool: &PgPool, table: ErasureMemberTable, entries: Vec<ErasureBatchEntry>) -> Uuid {
    let payload = serde_json::to_value(ErasureBatchApplied {
        member_table: table,
        entries,
    })
    .unwrap();
    record(pool, "erasure_batch_applied", &payload).await
}

// ── reading the tables back ──────────────────────────────────────────────────────────────────

/// One member row: (status, parent_table, parent_id, prior_erasure_id, late, erased_by_event_id,
/// refusal_reason). `None` when the erasure maps no such row.
type MemberRow = (
    String,
    Option<String>,
    Option<Uuid>,
    Option<Uuid>,
    bool,
    Option<Uuid>,
    Option<String>,
);

async fn member_row(pool: &PgPool, erasure: Uuid, table: &str, id: Uuid) -> Option<MemberRow> {
    sqlx::query(
        "SELECT status, parent_table, parent_id, prior_erasure_id, late, erased_by_event_id, \
                refusal_reason \
           FROM kb_erasure_members WHERE erasure_id = $1 AND member_table = $2 AND member_id = $3",
    )
    .bind(erasure)
    .bind(table)
    .bind(id)
    .fetch_optional(pool)
    .await
    .unwrap()
    .map(|r| {
        (
            r.get("status"),
            r.get("parent_table"),
            r.get("parent_id"),
            r.get("prior_erasure_id"),
            r.get("late"),
            r.get("erased_by_event_id"),
            r.get("refusal_reason"),
        )
    })
}

async fn resource_is_active(pool: &PgPool, id: ResourceId) -> bool {
    sqlx::query_scalar("SELECT is_active FROM kb_resources WHERE id = $1")
        .bind(id.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn context_is_active(pool: &PgPool, id: ContextId) -> bool {
    sqlx::query_scalar("SELECT is_active FROM kb_contexts WHERE id = $1")
        .bind(id.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

/// (is_folded, last_event_id) of an edge.
async fn edge_state(pool: &PgPool, edge: EdgeId) -> (bool, Uuid) {
    let r = sqlx::query("SELECT is_folded, last_event_id FROM kb_edges WHERE id = $1")
        .bind(edge.uuid())
        .fetch_one(pool)
        .await
        .unwrap();
    (r.get("is_folded"), r.get("last_event_id"))
}

/// (is_folded, last_event_id) of every property the edge owns.
async fn edge_property_states(pool: &PgPool, edge: EdgeId) -> Vec<(bool, Uuid)> {
    sqlx::query(
        "SELECT is_folded, last_event_id FROM kb_properties \
          WHERE owner_table = 'kb_edges' AND owner_id = $1 ORDER BY id",
    )
    .bind(edge.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| (r.get("is_folded"), r.get("last_event_id")))
    .collect()
}

async fn map_checksum(pool: &PgPool, erasure: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT erasure_map_checksum($1)")
        .bind(erasure)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// (scope_sealed_event_id, completed_event_id) of an erasure.
async fn erasure_events(pool: &PgPool, erasure: Uuid) -> (Option<Uuid>, Option<Uuid>) {
    let r = sqlx::query(
        "SELECT scope_sealed_event_id, completed_event_id FROM kb_erasures WHERE id = $1",
    )
    .bind(erasure)
    .fetch_one(pool)
    .await
    .unwrap();
    (r.get("scope_sealed_event_id"), r.get("completed_event_id"))
}

/// The erasure record's tables: every replay witness here must find each among the dumped
/// projection tables, or the diff would skip it.
const RECORD_TABLES: [&str; 4] = [
    "kb_erasures",
    "kb_erasure_members",
    "kb_erasure_estate_snapshots",
    "kb_erasure_former_holders",
];

/// Snapshot, reset, replay, and diff every projection table. Every record table must be among the
/// dumps, and each of `written` (the record tables the sequence under test writes) must be
/// non-empty before replay, so the diff compared rows rather than two empty arrays.
async fn assert_replay_byte_identical(pool: &PgPool, after_what: &str, written: &[&str]) {
    let before = replay::dump_projections(pool).await.unwrap();
    for table in RECORD_TABLES {
        let dumped = before
            .iter()
            .find(|(name, _)| name.as_str() == table)
            .unwrap_or_else(|| panic!("setup: {table} is among the replay's projection dumps"));
        if written.contains(&table) {
            assert!(
                dumped.1.as_array().is_some_and(|rows| !rows.is_empty()),
                "setup: {table} holds rows before replay {after_what}, or the diff compares nothing"
            );
        }
    }
    assert!(
        written.iter().all(|table| RECORD_TABLES.contains(table)),
        "setup: {written:?} are record tables"
    );
    let snap = replay::snapshot(pool).await.unwrap();
    common::reset_schema(pool).await;
    replay::replay(pool, &snap).await.unwrap();
    let after = replay::dump_projections(pool).await.unwrap();
    for ((ta, a), (tb, b)) in before.iter().zip(after.iter()) {
        assert_eq!(ta, tb);
        assert_eq!(
            a, b,
            "projection table {ta} diverged under replay {after_what}"
        );
    }
}

// ── erasure_declared ─────────────────────────────────────────────────────────────────────────

/// An estate declaration projects its erasure, its snapshot, its map (a born-erased row among
/// them), its former holders, and the flags of its pending members only.
///
/// FAILS IF: any column of the erasure or the snapshot is not the payload's; the born-erased row
/// is not `erased` with its prior erasure and event, or is flagged; a pending resource or context
/// stays active; the holder is not recorded.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn erasure_declared_projects_the_erasure_its_map_its_holders_and_its_flags(pool: PgPool) {
    fresh(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_context(&pool, owner, "declared-home").await;
    let container = make_context(&pool, owner, "declared-container").await;
    let r1 = make_resource(&pool, owner, emitter, home, "declared-r1").await;
    let r3 = make_resource(&pool, owner, emitter, home, "declared-r3").await;
    let subject = common::insert_profile(&pool, "declared-subject").await;

    // The earlier erasure that finished r3: a selection over nothing, so it flags nothing.
    let prior = Uuid::now_v7();
    let prior_event = declare(
        &pool,
        &declaration(prior, ErasureKind::Selection, None, vec![], vec![]),
    )
    .await;

    let erasure = Uuid::now_v7();
    let parent = member(ErasureMemberTable::KbContexts, container.uuid());
    let d = declaration(
        erasure,
        ErasureKind::Estate,
        Some(("kb_profiles", subject)),
        vec![
            pending(ErasureMemberTable::KbContexts, container.uuid(), None),
            pending(ErasureMemberTable::KbResources, r1.uuid(), Some(parent)),
            born_erased(
                ErasureMemberTable::KbResources,
                r3.uuid(),
                Some(parent),
                prior,
                prior_event,
            ),
        ],
        vec![owner.uuid()],
    );
    let event = declare(&pool, &d).await;

    let row = sqlx::query(
        "SELECT kind, subject_table, subject_id, request_ref, manifest_version, manifest_digest, \
                legacy, declared_event_id, scope_sealed_event_id, completed_event_id \
           FROM kb_erasures WHERE id = $1",
    )
    .bind(erasure)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("kind"), "estate");
    assert_eq!(
        row.get::<Option<String>, _>("subject_table").as_deref(),
        Some("kb_profiles")
    );
    assert_eq!(row.get::<Option<Uuid>, _>("subject_id"), Some(subject));
    assert_eq!(row.get::<Option<Uuid>, _>("request_ref"), d.request_ref);
    assert_eq!(
        row.get::<Option<String>, _>("manifest_version").as_deref(),
        Some(MANIFEST_VERSION)
    );
    assert_eq!(
        row.get::<Option<String>, _>("manifest_digest").as_deref(),
        Some(DIGEST)
    );
    assert!(!row.get::<bool, _>("legacy"));
    assert_eq!(row.get::<Uuid, _>("declared_event_id"), event);
    assert_eq!(row.get::<Option<Uuid>, _>("scope_sealed_event_id"), None);
    assert_eq!(row.get::<Option<Uuid>, _>("completed_event_id"), None);

    let snap = sqlx::query(
        "SELECT resource_count, context_count, cogmap_count, blob_count, edge_count, checksum \
           FROM kb_erasure_estate_snapshots WHERE erasure_id = $1",
    )
    .bind(erasure)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(snap.get::<i32, _>("resource_count"), 2);
    assert_eq!(snap.get::<i32, _>("context_count"), 1);
    assert_eq!(snap.get::<i32, _>("cogmap_count"), 0);
    assert_eq!(snap.get::<i32, _>("blob_count"), 0);
    assert_eq!(snap.get::<i32, _>("edge_count"), 0);
    let checksum: String = snap.get("checksum");
    assert_eq!(Some(&checksum), d.checksum.as_ref());
    assert_eq!(
        map_checksum(&pool, erasure).await.as_deref(),
        Some(checksum.as_str()),
        "the snapshot's checksum is the map's, recomputed in SQL"
    );

    assert_eq!(
        member_row(&pool, erasure, "kb_contexts", container.uuid()).await,
        Some(("pending".to_owned(), None, None, None, false, None, None))
    );
    assert_eq!(
        member_row(&pool, erasure, "kb_resources", r1.uuid()).await,
        Some((
            "pending".to_owned(),
            Some("kb_contexts".to_owned()),
            Some(container.uuid()),
            None,
            false,
            None,
            None
        ))
    );
    assert_eq!(
        member_row(&pool, erasure, "kb_resources", r3.uuid()).await,
        Some((
            "erased".to_owned(),
            Some("kb_contexts".to_owned()),
            Some(container.uuid()),
            Some(prior),
            false,
            Some(prior_event),
            None
        )),
        "an entry naming prior_erasure and prior_event is born erased"
    );
    let drain: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT next_attempt_at::text, lease_expires_at::text FROM kb_erasure_members \
          WHERE erasure_id = $1 AND member_id = $2",
    )
    .bind(erasure)
    .bind(r3.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(drain, (None, None), "a row born erased is off the drain");

    let holders: Vec<Uuid> = sqlx::query_scalar(
        "SELECT profile_id FROM kb_erasure_former_holders WHERE erasure_id = $1",
    )
    .bind(erasure)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(holders, vec![owner.uuid()]);

    assert!(
        !resource_is_active(&pool, r1).await,
        "a pending resource is flagged"
    );
    assert!(
        !context_is_active(&pool, container).await,
        "a pending context is flagged"
    );
    assert!(
        resource_is_active(&pool, r3).await,
        "a row born erased is not flagged by this declaration"
    );
    assert!(
        context_is_active(&pool, home).await,
        "an unmapped context is untouched"
    );
}

/// A legacy declaration records `legacy = true` with no manifest and no request, and its erasure
/// has no checksum. A legacy payload that carries a manifest or a request is refused, one key at a
/// time, and nothing of it lands.
///
/// FAILS IF: the legacy row gains a manifest, a request, a snapshot or a checksum; or a legacy
/// payload carrying `request_ref`, `manifest_version` or `manifest_digest` projects (its stray key
/// dropped) instead of raising.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_legacy_erasure_declared_has_no_manifest_request_or_checksum(pool: PgPool) {
    fresh(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_context(&pool, owner, "legacy-home").await;
    let r = make_resource(&pool, owner, emitter, home, "legacy-r").await;

    for (key, value) in [
        ("request_ref", serde_json::json!(Uuid::now_v7())),
        ("manifest_version", serde_json::json!(MANIFEST_VERSION)),
        ("manifest_digest", serde_json::json!(DIGEST)),
    ] {
        let stray = Uuid::now_v7();
        let mut payload = serde_json::to_value(legacy_declaration(
            stray,
            r.uuid(),
            vec![pending(ErasureMemberTable::KbResources, r.uuid(), None)],
        ))
        .unwrap();
        payload[key] = value;
        let err = try_record(&pool, "erasure_declared", &payload)
            .await
            .expect_err("a legacy payload carrying a manifest or a request must be refused");
        assert!(
            err.to_string()
                .contains("a legacy declaration carries a manifest or a request"),
            "{key}: unexpected error: {err}"
        );
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_erasures WHERE id = $1")
            .bind(stray)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0, "{key}: nothing of the refused declaration lands");
    }
    assert!(
        resource_is_active(&pool, r).await,
        "setup: no refused declaration flagged the resource"
    );

    let erasure = Uuid::now_v7();
    let event = declare(
        &pool,
        &legacy_declaration(
            erasure,
            r.uuid(),
            vec![pending(ErasureMemberTable::KbResources, r.uuid(), None)],
        ),
    )
    .await;

    let row = sqlx::query(
        "SELECT kind, legacy, request_ref, manifest_version, manifest_digest, declared_event_id \
           FROM kb_erasures WHERE id = $1",
    )
    .bind(erasure)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("kind"), "resource");
    assert!(row.get::<bool, _>("legacy"));
    assert_eq!(row.get::<Option<Uuid>, _>("request_ref"), None);
    assert_eq!(row.get::<Option<String>, _>("manifest_version"), None);
    assert_eq!(row.get::<Option<String>, _>("manifest_digest"), None);
    assert_eq!(row.get::<Uuid, _>("declared_event_id"), event);
    assert_eq!(
        member_row(&pool, erasure, "kb_resources", r.uuid()).await,
        Some(("pending".to_owned(), None, None, None, false, None, None))
    );
    let snapshots: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_erasure_estate_snapshots WHERE erasure_id = $1",
    )
    .bind(erasure)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(snapshots, 0);
    assert_eq!(map_checksum(&pool, erasure).await, None);
    assert!(!resource_is_active(&pool, r).await);
}

/// An estate declaration whose counts omit a table its map holds is refused, and nothing of it
/// lands.
///
/// FAILS IF: the projector fills the missing count with 0 and projects.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_estate_whose_counts_omit_a_mapped_table_is_refused(pool: PgPool) {
    fresh(&pool).await;
    let subject = common::insert_profile(&pool, "counts-subject").await;
    let erasure = Uuid::now_v7();
    let mut d = declaration(
        erasure,
        ErasureKind::Estate,
        Some(("kb_profiles", subject)),
        vec![
            pending(ErasureMemberTable::KbBlobs, Uuid::now_v7(), None),
            pending(ErasureMemberTable::KbBlobs, Uuid::now_v7(), None),
        ],
        vec![],
    );
    let counts = d.counts.as_mut().expect("an estate declaration has counts");
    assert_eq!(
        counts.remove(&ErasureMemberTable::KbBlobs),
        Some(2),
        "setup: the counts held the mapped table"
    );
    counts.insert(ErasureMemberTable::KbResources, 0);
    let err = try_record(
        &pool,
        "erasure_declared",
        &serde_json::to_value(&d).unwrap(),
    )
    .await
    .expect_err("counts that omit a mapped table must be refused");
    assert!(
        err.to_string()
            .contains("the estate counts omit a table its map holds"),
        "unexpected error: {err}"
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_erasures WHERE id = $1")
        .bind(erasure)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

/// A map entry that names only one of `prior_erasure` and `prior_event` is refused, and nothing
/// of the declaration lands.
///
/// FAILS IF: the declaration projects, or the error does not name the half-prior entry.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_map_entry_with_half_a_prior_is_refused(pool: PgPool) {
    fresh(&pool).await;
    let erasure = Uuid::now_v7();
    let mut entry = pending(ErasureMemberTable::KbResources, Uuid::now_v7(), None);
    entry.prior_erasure = Some(Uuid::now_v7());
    let payload = serde_json::to_value(declaration(
        erasure,
        ErasureKind::Selection,
        None,
        vec![entry],
        vec![],
    ))
    .unwrap();
    let err = try_record(&pool, "erasure_declared", &payload)
        .await
        .expect_err("half a prior must be refused");
    assert!(
        err.to_string().contains("without the other"),
        "unexpected error: {err}"
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_erasures WHERE id = $1")
        .bind(erasure)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

// ── erasure_member_arrived ───────────────────────────────────────────────────────────────────

/// An arrival projects one late, pending row under its parent, flags the item, and leaves the
/// declared checksum as it was.
///
/// FAILS IF: the row is not late and pending under its parent, the item stays active, or the late
/// row moves the checksum.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn erasure_member_arrived_projects_a_late_row_and_its_flag(pool: PgPool) {
    fresh(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_context(&pool, owner, "arrived-home").await;
    let container = make_context(&pool, owner, "arrived-container").await;
    let late = make_resource(&pool, owner, emitter, home, "arrived-late").await;

    let erasure = Uuid::now_v7();
    let d = declaration(
        erasure,
        ErasureKind::Selection,
        None,
        vec![pending(
            ErasureMemberTable::KbContexts,
            container.uuid(),
            None,
        )],
        vec![],
    );
    declare(&pool, &d).await;
    let parent = member(ErasureMemberTable::KbContexts, container.uuid());
    record(
        &pool,
        "erasure_member_arrived",
        &serde_json::to_value(ErasureMemberArrived {
            erasure_id: erasure,
            member: member(ErasureMemberTable::KbResources, late.uuid()),
            parent: Some(parent),
            source: ErasureArrivalSource::Trigger,
        })
        .unwrap(),
    )
    .await;

    assert_eq!(
        member_row(&pool, erasure, "kb_resources", late.uuid()).await,
        Some((
            "pending".to_owned(),
            Some("kb_contexts".to_owned()),
            Some(container.uuid()),
            None,
            true,
            None,
            None
        ))
    );
    assert!(
        !resource_is_active(&pool, late).await,
        "the arrival flags its item"
    );
    assert_eq!(
        map_checksum(&pool, erasure).await,
        d.checksum,
        "a late row is outside the declared checksum"
    );
}

// ── erasure_scope_sealed, erasure_reopened, erasure_completed ────────────────────────────────

/// FAILS IF: the seal does not set `scope_sealed_event_id` to its own event, or a seal of an
/// erasure that does not exist projects.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn erasure_scope_sealed_sets_the_seal(pool: PgPool) {
    fresh(&pool).await;
    let erasure = Uuid::now_v7();
    declare(
        &pool,
        &declaration(erasure, ErasureKind::Selection, None, vec![], vec![]),
    )
    .await;
    let sealed = |id: Uuid| {
        serde_json::to_value(ErasureScopeSealed {
            erasure_id: id,
            late_count: 0,
            late_digest: DIGEST.to_owned(),
        })
        .unwrap()
    };
    let event = record(&pool, "erasure_scope_sealed", &sealed(erasure)).await;
    assert_eq!(erasure_events(&pool, erasure).await, (Some(event), None));

    let err = try_record(&pool, "erasure_scope_sealed", &sealed(Uuid::now_v7()))
        .await
        .expect_err("a seal of no erasure must be refused");
    assert!(
        err.to_string().contains("not found"),
        "unexpected error: {err}"
    );
}

/// FAILS IF: a completion does not set `completed_event_id` to its own event, or a second
/// completion does not replace the first.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn erasure_completed_sets_the_latest_completion(pool: PgPool) {
    fresh(&pool).await;
    let erasure = Uuid::now_v7();
    declare(
        &pool,
        &declaration(erasure, ErasureKind::Selection, None, vec![], vec![]),
    )
    .await;
    let completed = serde_json::to_value(ErasureCompleted {
        erasure_id: erasure,
        redacted_hashes: vec![],
        late_digest: None,
    })
    .unwrap();
    let first = record(&pool, "erasure_completed", &completed).await;
    assert_eq!(erasure_events(&pool, erasure).await, (None, Some(first)));
    let second = record(&pool, "erasure_completed", &completed).await;
    assert_eq!(erasure_events(&pool, erasure).await, (None, Some(second)));
}

/// FAILS IF: a reopening leaves `completed_event_id` set, or touches the seal.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn erasure_reopened_clears_the_completion(pool: PgPool) {
    fresh(&pool).await;
    let erasure = Uuid::now_v7();
    declare(
        &pool,
        &declaration(erasure, ErasureKind::Selection, None, vec![], vec![]),
    )
    .await;
    let sealed = record(
        &pool,
        "erasure_scope_sealed",
        &serde_json::to_value(ErasureScopeSealed {
            erasure_id: erasure,
            late_count: 0,
            late_digest: DIGEST.to_owned(),
        })
        .unwrap(),
    )
    .await;
    let completed = record(
        &pool,
        "erasure_completed",
        &serde_json::to_value(ErasureCompleted {
            erasure_id: erasure,
            redacted_hashes: vec![],
            late_digest: Some(DIGEST.to_owned()),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(
        erasure_events(&pool, erasure).await,
        (Some(sealed), Some(completed))
    );
    record(
        &pool,
        "erasure_reopened",
        &serde_json::to_value(ErasureReopened {
            erasure_id: erasure,
            arrival_event: completed,
        })
        .unwrap(),
    )
    .await;
    assert_eq!(erasure_events(&pool, erasure).await, (Some(sealed), None));
}

// ── erasure_batch_applied ────────────────────────────────────────────────────────────────────

/// A refusal is written to the item's open row in every erasure it names; a second refusal
/// changes the reason and keeps the first `first_refused_at`; an erased entry is not projected by
/// the batch, which is not an authority.
///
/// FAILS IF: a named row is not refused with the entry's reason, SQLSTATE and routine; the second
/// refusal moves `first_refused_at`; an unnamed erasure's row of the item moves; an erased entry
/// marks its row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn erasure_batch_applied_projects_refusals_and_keeps_the_first_refusal_time(pool: PgPool) {
    fresh(&pool).await;
    let item = Uuid::now_v7();
    let other = Uuid::now_v7();
    let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    for erasure in [a, b, c] {
        declare(
            &pool,
            &declaration(
                erasure,
                ErasureKind::Selection,
                None,
                vec![
                    pending(ErasureMemberTable::KbBlobs, item, None),
                    pending(ErasureMemberTable::KbBlobs, other, None),
                ],
                vec![],
            ),
        )
        .await;
    }

    let first = batch(
        &pool,
        ErasureMemberTable::KbBlobs,
        vec![
            refused(item, vec![a, b], ErasureMemberRefusalReason::Defect),
            ErasureBatchEntry {
                member: other,
                erasures: vec![a],
                outcome: ErasureBatchOutcome::Erased,
                authority_event: Some(Uuid::now_v7()),
                reason: None,
                sqlstate: None,
                routine: None,
            },
        ],
    )
    .await;
    for erasure in [a, b] {
        let row = sqlx::query(
            "SELECT status, refusal_reason, refusal_sqlstate, refusal_routine, \
                    first_refused_at = (SELECT occurred_at FROM kb_events WHERE id = $3) AS at_first \
               FROM kb_erasure_members \
              WHERE erasure_id = $1 AND member_table = 'kb_blobs' AND member_id = $2",
        )
        .bind(erasure)
        .bind(item)
        .bind(first)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("status"), "refused");
        assert_eq!(
            row.get::<Option<String>, _>("refusal_reason").as_deref(),
            Some("defect")
        );
        assert_eq!(
            row.get::<Option<String>, _>("refusal_sqlstate").as_deref(),
            Some("XX000")
        );
        assert_eq!(
            row.get::<Option<String>, _>("refusal_routine").as_deref(),
            Some("resource_erasure_execute")
        );
        assert!(
            row.get::<bool, _>("at_first"),
            "first_refused_at is the event's occurred_at"
        );
    }
    assert_eq!(
        member_row(&pool, c, "kb_blobs", item).await.map(|r| r.0),
        Some("pending".to_owned()),
        "an erasure the entry does not name keeps its row"
    );
    assert_eq!(
        member_row(&pool, a, "kb_blobs", other).await.map(|r| r.0),
        Some("pending".to_owned()),
        "the batch is not an authority: an erased entry marks nothing"
    );

    batch(
        &pool,
        ErasureMemberTable::KbBlobs,
        vec![refused(
            item,
            vec![a],
            ErasureMemberRefusalReason::Unclassified,
        )],
    )
    .await;
    let (reason, at_first): (Option<String>, bool) = sqlx::query_as(
        "SELECT refusal_reason, first_refused_at = (SELECT occurred_at FROM kb_events WHERE id = $3) \
           FROM kb_erasure_members \
          WHERE erasure_id = $1 AND member_table = 'kb_blobs' AND member_id = $2",
    )
    .bind(a)
    .bind(item)
    .bind(first)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        reason.as_deref(),
        Some("unclassified"),
        "refused -> refused takes the new reason"
    );
    assert!(at_first, "a retry keeps the first refusal's time");
}

/// A refusal naming an erasure whose row of the item is not open is refused whole.
///
/// FAILS IF: the batch projects although one named row is born erased, or the other named row
/// moves.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_refusal_naming_a_row_that_is_not_open_is_refused(pool: PgPool) {
    fresh(&pool).await;
    let item = Uuid::now_v7();
    let prior = Uuid::now_v7();
    let prior_event = declare(
        &pool,
        &declaration(prior, ErasureKind::Selection, None, vec![], vec![]),
    )
    .await;
    let (open, closed) = (Uuid::now_v7(), Uuid::now_v7());
    declare(
        &pool,
        &declaration(
            open,
            ErasureKind::Selection,
            None,
            vec![pending(ErasureMemberTable::KbBlobs, item, None)],
            vec![],
        ),
    )
    .await;
    declare(
        &pool,
        &declaration(
            closed,
            ErasureKind::Selection,
            None,
            vec![born_erased(
                ErasureMemberTable::KbBlobs,
                item,
                None,
                prior,
                prior_event,
            )],
            vec![],
        ),
    )
    .await;

    let payload = serde_json::to_value(ErasureBatchApplied {
        member_table: ErasureMemberTable::KbBlobs,
        entries: vec![refused(
            item,
            vec![open, closed],
            ErasureMemberRefusalReason::Defect,
        )],
    })
    .unwrap();
    let err = try_record(&pool, "erasure_batch_applied", &payload)
        .await
        .expect_err("a refusal of a row that is not open must be refused");
    assert!(
        err.to_string().contains("is open in 1 of the 2 erasures"),
        "unexpected error: {err}"
    );
    assert_eq!(
        member_row(&pool, open, "kb_blobs", item).await.map(|r| r.0),
        Some("pending".to_owned())
    );
}

/// A refused entry that names no erasure is refused, and the batch projects nothing.
///
/// FAILS IF: an empty `erasures` array passes as a no-op refusal, so the item's row stays
/// pending under a batch that claims to have refused it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_refusal_naming_no_erasure_is_refused(pool: PgPool) {
    fresh(&pool).await;
    let item = Uuid::now_v7();
    let erasure = Uuid::now_v7();
    declare(
        &pool,
        &declaration(
            erasure,
            ErasureKind::Selection,
            None,
            vec![pending(ErasureMemberTable::KbBlobs, item, None)],
            vec![],
        ),
    )
    .await;
    let payload = serde_json::to_value(ErasureBatchApplied {
        member_table: ErasureMemberTable::KbBlobs,
        entries: vec![refused(item, vec![], ErasureMemberRefusalReason::Defect)],
    })
    .unwrap();
    let err = try_record(&pool, "erasure_batch_applied", &payload)
        .await
        .expect_err("a refusal naming no erasure must be refused");
    assert!(
        err.to_string().contains("names no erasure"),
        "unexpected error: {err}"
    );
    assert_eq!(
        member_row(&pool, erasure, "kb_blobs", item)
            .await
            .map(|r| r.0),
        Some("pending".to_owned())
    );
}

/// No batch entry carries text a CHECK never sees: the record is never redacted. A routine that is
/// not a routine name raises on a refused entry and on an erased one, and an erased entry carrying
/// a reason, a SQLSTATE or a routine raises. Each refused payload is an honest one with one change,
/// and the item's row stays as it was.
///
/// FAILS IF: an erased entry's routine, reason or SQLSTATE lands on the ledger unchecked (the
/// projector skips non-refused entries before looking at them), a refused entry's message-shaped
/// routine reaches the column, or (the control) an honest erased entry with none of them is
/// refused.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn no_batch_entry_carries_free_text_or_a_refusals_detail_it_does_not_own(pool: PgPool) {
    fresh(&pool).await;
    let item = Uuid::now_v7();
    let erasure = Uuid::now_v7();
    declare(
        &pool,
        &declaration(
            erasure,
            ErasureKind::Selection,
            None,
            vec![pending(ErasureMemberTable::KbBlobs, item, None)],
            vec![],
        ),
    )
    .await;
    let erased = || ErasureBatchEntry {
        member: item,
        erasures: vec![erasure],
        outcome: ErasureBatchOutcome::Erased,
        authority_event: Some(Uuid::now_v7()),
        reason: None,
        sqlstate: None,
        routine: None,
    };
    let message = "ERROR: could not erase the blob".to_owned();
    let mut message_on_refusal = refused(item, vec![erasure], ErasureMemberRefusalReason::Defect);
    message_on_refusal.routine = Some(message.clone());
    let mut message_on_erased = erased();
    message_on_erased.routine = Some(message);
    let mut routine_on_erased = erased();
    routine_on_erased.routine = Some("resource_erasure_execute".to_owned());
    let mut reason_on_erased = erased();
    reason_on_erased.reason = Some(ErasureMemberRefusalReason::Defect);
    let mut sqlstate_on_erased = erased();
    sqlstate_on_erased.sqlstate = Some("XX000".to_owned());

    for (what, entry, says) in [
        (
            "a refused entry whose routine is a message",
            message_on_refusal,
            "is not a routine name",
        ),
        (
            "an erased entry whose routine is a message",
            message_on_erased,
            "is not a routine name",
        ),
        (
            "an erased entry carrying a routine",
            routine_on_erased,
            "carries a refusal's detail",
        ),
        (
            "an erased entry carrying a reason",
            reason_on_erased,
            "carries a refusal's detail",
        ),
        (
            "an erased entry carrying a SQLSTATE",
            sqlstate_on_erased,
            "carries a refusal's detail",
        ),
    ] {
        let payload = serde_json::to_value(ErasureBatchApplied {
            member_table: ErasureMemberTable::KbBlobs,
            entries: vec![entry],
        })
        .unwrap();
        let err = try_record(&pool, "erasure_batch_applied", &payload)
            .await
            .expect_err(what);
        assert!(
            err.to_string().contains(says),
            "{what}: unexpected error: {err}"
        );
        assert_eq!(
            member_row(&pool, erasure, "kb_blobs", item)
                .await
                .map(|r| r.0),
            Some("pending".to_owned()),
            "{what}: the row is untouched"
        );
    }

    let honest = serde_json::to_value(ErasureBatchApplied {
        member_table: ErasureMemberTable::KbBlobs,
        entries: vec![erased()],
    })
    .unwrap();
    try_record(&pool, "erasure_batch_applied", &honest)
        .await
        .expect("control: an erased entry with no refusal detail projects");
}

// ── the edge fold ────────────────────────────────────────────────────────────────────────────

/// An edge member folds under the declaration with its edge-owned properties; an edge folded
/// before keeps its own fold; a mapped edge that does not exist is refused; and
/// `relationship_folded` still folds and raises through the shared helper.
///
/// FAILS IF: the edge or its facet stays live, or carries another event than the declaration; the
/// earlier fold's event is overwritten; a missing edge member projects; the wrapper's raise is
/// lost.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_member_folds_with_its_edge_owned_properties(pool: PgPool) {
    fresh(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_context(&pool, owner, "fold-home").await;
    let r1 = make_resource(&pool, owner, emitter, home, "fold-r1").await;
    let r2 = make_resource(&pool, owner, emitter, home, "fold-r2").await;
    let live = make_edge(&pool, emitter, home, r1, r2, "live edge").await;
    let folded = make_edge(&pool, emitter, home, r2, r1, "folded edge").await;
    let earlier_fold = fold_edge(&pool, emitter, folded).await;
    assert_eq!(edge_property_states(&pool, live).await.len(), 1);
    assert_eq!(
        edge_property_states(&pool, folded).await,
        vec![(true, earlier_fold)]
    );

    let erasure = Uuid::now_v7();
    let event = declare(
        &pool,
        &declaration(
            erasure,
            ErasureKind::Selection,
            None,
            vec![
                pending(ErasureMemberTable::KbEdges, live.uuid(), None),
                pending(ErasureMemberTable::KbEdges, folded.uuid(), None),
            ],
            vec![],
        ),
    )
    .await;

    assert_eq!(edge_state(&pool, live).await, (true, event));
    assert_eq!(
        edge_property_states(&pool, live).await,
        vec![(true, event)],
        "an edge's facets fold with it, under the declaration"
    );
    assert_eq!(
        edge_state(&pool, folded).await,
        (true, earlier_fold),
        "an edge folded before keeps its own fold"
    );
    assert_eq!(
        edge_property_states(&pool, folded).await,
        vec![(true, earlier_fold)]
    );

    let payload = serde_json::to_value(declaration(
        Uuid::now_v7(),
        ErasureKind::Selection,
        None,
        vec![pending(ErasureMemberTable::KbEdges, Uuid::now_v7(), None)],
        vec![],
    ))
    .unwrap();
    let err = try_record(&pool, "erasure_declared", &payload)
        .await
        .expect_err("a mapped edge that does not exist must be refused");
    assert!(
        err.to_string().contains("mapped edge"),
        "unexpected error: {err}"
    );

    // The wrapper: a fold through the real path still cascades (fold_edge above), and a missing
    // edge still raises in the wrapper's words.
    let err = sqlx::query(
        "SELECT _project_relationship_folded($1, jsonb_build_object('edge_id', $2::uuid))",
    )
    .bind(Uuid::now_v7())
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect_err("a fold of no edge must raise");
    assert!(
        err.to_string().contains("relationship_fold: edge"),
        "unexpected error: {err}"
    );
}

// ── D7.5 and the golden checksum ─────────────────────────────────────────────────────────────

/// D7.5: replay rebuilds the four record tables byte for byte, and the checksum recomputed from the
/// rebuilt map equals the `erasure_declared` payload's for every non-legacy erasure.
///
/// The sequence holds every kind: a selection, a legacy declaration, an estate whose map holds a
/// container, a leaf under it, an edge with a facet and a row born erased, an arrival, two refusals
/// of one item (so the vocabulary's restore is exercised and `first_refused_at` must survive), an
/// erased batch entry, a seal, a completion, a reopening and a second completion.
///
/// FAILS IF: any projection table diverges under replay; a non-legacy erasure's recomputed checksum
/// is NULL or differs from its payload's; a legacy erasure has a checksum.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_rebuilds_the_erasure_record_and_the_checksum_re_derives(pool: PgPool) {
    fresh(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_context(&pool, owner, "replay-home").await;
    let container = make_context(&pool, owner, "replay-container").await;
    let r1 = make_resource(&pool, owner, emitter, home, "replay-r1").await;
    let r2 = make_resource(&pool, owner, emitter, home, "replay-r2").await;
    let r3 = make_resource(&pool, owner, emitter, home, "replay-r3").await;
    let r4 = make_resource(&pool, owner, emitter, home, "replay-r4").await;
    let edge = make_edge(&pool, emitter, home, r1, r2, "replay edge").await;
    let subject = common::insert_profile(&pool, "replay-subject").await;

    // P finished r3 before the estate was declared.
    let p = Uuid::now_v7();
    let p_event = declare(
        &pool,
        &declaration(
            p,
            ErasureKind::Resource,
            Some(("kb_resources", r3.uuid())),
            vec![pending(ErasureMemberTable::KbResources, r3.uuid(), None)],
            vec![],
        ),
    )
    .await;
    // L: a legacy erasure of r2.
    let l = Uuid::now_v7();
    declare(
        &pool,
        &legacy_declaration(
            l,
            r2.uuid(),
            vec![pending(ErasureMemberTable::KbResources, r2.uuid(), None)],
        ),
    )
    .await;
    // B: the estate.
    let b = Uuid::now_v7();
    let parent = member(ErasureMemberTable::KbContexts, container.uuid());
    declare(
        &pool,
        &declaration(
            b,
            ErasureKind::Estate,
            Some(("kb_profiles", subject)),
            vec![
                pending(ErasureMemberTable::KbContexts, container.uuid(), None),
                pending(ErasureMemberTable::KbResources, r1.uuid(), Some(parent)),
                pending(ErasureMemberTable::KbEdges, edge.uuid(), None),
                born_erased(
                    ErasureMemberTable::KbResources,
                    r3.uuid(),
                    Some(parent),
                    p,
                    p_event,
                ),
            ],
            vec![owner.uuid(), subject],
        ),
    )
    .await;
    record(
        &pool,
        "erasure_member_arrived",
        &serde_json::to_value(ErasureMemberArrived {
            erasure_id: b,
            member: member(ErasureMemberTable::KbResources, r4.uuid()),
            parent: Some(parent),
            source: ErasureArrivalSource::ClosingSurvey,
        })
        .unwrap(),
    )
    .await;
    let first_refusal = batch(
        &pool,
        ErasureMemberTable::KbResources,
        vec![
            refused(r1.uuid(), vec![b], ErasureMemberRefusalReason::Defect),
            ErasureBatchEntry {
                member: r3.uuid(),
                erasures: vec![p],
                outcome: ErasureBatchOutcome::Erased,
                authority_event: Some(Uuid::now_v7()),
                reason: None,
                sqlstate: None,
                routine: None,
            },
        ],
    )
    .await;
    batch(
        &pool,
        ErasureMemberTable::KbResources,
        vec![refused(
            r1.uuid(),
            vec![b],
            ErasureMemberRefusalReason::Unclassified,
        )],
    )
    .await;
    record(
        &pool,
        "erasure_scope_sealed",
        &serde_json::to_value(ErasureScopeSealed {
            erasure_id: b,
            late_count: 1,
            late_digest: DIGEST.to_owned(),
        })
        .unwrap(),
    )
    .await;
    let p_completed = record(
        &pool,
        "erasure_completed",
        &serde_json::to_value(ErasureCompleted {
            erasure_id: p,
            redacted_hashes: vec![],
            late_digest: None,
        })
        .unwrap(),
    )
    .await;
    record(
        &pool,
        "erasure_reopened",
        &serde_json::to_value(ErasureReopened {
            erasure_id: p,
            arrival_event: p_completed,
        })
        .unwrap(),
    )
    .await;
    record(
        &pool,
        "erasure_completed",
        &serde_json::to_value(ErasureCompleted {
            erasure_id: b,
            redacted_hashes: vec![],
            late_digest: Some(DIGEST.to_owned()),
        })
        .unwrap(),
    )
    .await;

    // The sequence's own claims, before replay: the refusal kept its first time.
    let at_first: bool = sqlx::query_scalar(
        "SELECT first_refused_at = (SELECT occurred_at FROM kb_events WHERE id = $3) \
           FROM kb_erasure_members \
          WHERE erasure_id = $1 AND member_table = 'kb_resources' AND member_id = $2 \
            AND status = 'refused' AND refusal_reason = 'unclassified'",
    )
    .bind(b)
    .bind(r1.uuid())
    .bind(first_refusal)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(at_first);

    assert_replay_byte_identical(&pool, "of the erasure record", &RECORD_TABLES).await;

    let declarations: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'erasure_declared' ORDER BY e.id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(declarations.len(), 3);
    let mut non_legacy = 0;
    for payload in declarations {
        let d: ErasureDeclared = serde_json::from_value(payload).unwrap();
        let recomputed = map_checksum(&pool, d.erasure_id).await;
        if d.legacy {
            assert_eq!(recomputed, None, "a legacy erasure has no checksum");
            continue;
        }
        non_legacy += 1;
        let recomputed = recomputed
            .unwrap_or_else(|| panic!("erasure {} must have a recomputed checksum", d.erasure_id));
        assert_eq!(
            Some(&recomputed),
            d.checksum.as_ref(),
            "erasure {}: the rebuilt map's checksum is the declared one",
            d.erasure_id
        );
    }
    assert_eq!(non_legacy, 2, "the loop above saw both non-legacy erasures");
    let snapshot: String = sqlx::query_scalar(
        "SELECT checksum FROM kb_erasure_estate_snapshots WHERE erasure_id = $1",
    )
    .bind(b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(map_checksum(&pool, b).await, Some(snapshot));
}

/// The golden checksum: one fixed map's D1.1 checksum, pinned to a literal, so a change to
/// `erasure_map_checksum` that every live erasure and its replay would reproduce alike still
/// fails here. The literal was computed from the spec's D1.1 text by an implementation outside
/// both this crate and the database; `d1_1_checksum` is a second one, held equal too.
///
/// The map exercises each part of a line: a container with no parent, two leaves under it whose
/// ids sort against their insertion order, a blob that sorts before both tables by `COLLATE "C"`
/// and is born erased (so its prior erasure is hashed), and a late arrival, which is not hashed.
///
/// FAILS IF: `erasure_map_checksum` of the fixed map is not the literal, or the literal is not
/// D1.1 as `d1_1_checksum` reads it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_map_checksum_of_a_fixed_map_is_pinned(pool: PgPool) {
    const GOLDEN: &str = "18b5dc1090d4ac5dee44dad37bef9a1ff4857e68a7a3f305ea9e97004228656d";
    fresh(&pool).await;
    let fixed =
        |tail: &str| Uuid::parse_str(&format!("01900000-0000-7000-8000-0000000000{tail}")).unwrap();
    let (prior, erasure) = (fixed("e0"), fixed("e1"));
    let container = fixed("c1");
    let parent = member(ErasureMemberTable::KbContexts, container);

    let mut p = declaration(prior, ErasureKind::Selection, None, vec![], vec![]);
    p.request_ref = Some(fixed("f0"));
    let prior_event = declare(&pool, &p).await;

    let map = vec![
        pending(ErasureMemberTable::KbContexts, container, None),
        pending(ErasureMemberTable::KbResources, fixed("a2"), Some(parent)),
        pending(ErasureMemberTable::KbResources, fixed("a1"), Some(parent)),
        born_erased(
            ErasureMemberTable::KbBlobs,
            fixed("b1"),
            Some(parent),
            prior,
            prior_event,
        ),
    ];
    assert_eq!(
        d1_1_checksum(erasure, DIGEST, &map),
        GOLDEN,
        "the literal is D1.1 of the fixed map"
    );
    let mut d = declaration(erasure, ErasureKind::Selection, None, map, vec![]);
    d.request_ref = Some(fixed("f1"));
    declare(&pool, &d).await;
    record(
        &pool,
        "erasure_member_arrived",
        &serde_json::to_value(ErasureMemberArrived {
            erasure_id: erasure,
            member: member(ErasureMemberTable::KbResources, fixed("a0")),
            parent: Some(parent),
            source: ErasureArrivalSource::Straggler,
        })
        .unwrap(),
    )
    .await;

    assert_eq!(map_checksum(&pool, erasure).await.as_deref(), Some(GOLDEN));
}

// ── the walk order: forced id inversions ─────────────────────────────────────────────────────

/// Two version-7 ids in the current millisecond, `(high, low)` with `low < high`. On PG17
/// `uuid_generate_v7()` has no order within a millisecond (20261116000010_kb_events_record_order.sql),
/// so of two appends in one millisecond either may get the lower id. The witnesses below give the
/// LATER append `low`: an inversion PG17 can mint, built on every run rather than by chance.
///
/// Bytes 0..6 hold the millisecond; the version is byte 6's high nibble and the variant byte 8's
/// top two bits. Every other bit is random, so all ones and all zeros there are the highest and the
/// lowest id of the millisecond.
fn same_millisecond() -> (Uuid, Uuid) {
    let mut high = *Uuid::now_v7().as_bytes();
    let mut low = high;
    high[6] = 0x7f;
    high[7] = 0xff;
    high[8] = 0xbf;
    low[6] = 0x70;
    low[7] = 0x00;
    low[8] = 0x80;
    for byte in &mut high[9..] {
        *byte = 0xff;
    }
    for byte in &mut low[9..] {
        *byte = 0x00;
    }
    (Uuid::from_bytes(high), Uuid::from_bytes(low))
}

/// Append one erasure record event whose id is `id`, and project it, in `tx`: what
/// `_event_append` and the live writer do, with the id chosen rather than minted. The row's
/// recorded order is still the live one: the column defaults give it its own transaction's
/// `xact_id` and a fresh `seq`, and `kb_events_order_recorded` refuses anything else.
async fn append_as(
    tx: &mut sqlx::PgConnection,
    emitter: EntityId,
    id: Uuid,
    kind: &str,
    payload: &serde_json::Value,
) {
    let appended = sqlx::query(
        "INSERT INTO kb_events (id, event_type_id, emitter_entity_id, payload, correlation_id, \
                                category) \
         SELECT $1, t.id, $2, $3, $1, t.category FROM kb_event_types t WHERE t.name = $4",
    )
    .bind(id)
    .bind(emitter)
    .bind(payload)
    .bind(kind)
    .execute(&mut *tx)
    .await
    .unwrap_or_else(|e| panic!("{kind} {id} must append: {e}"))
    .rows_affected();
    assert_eq!(appended, 1, "setup: {kind} is a registered kind");
    sqlx::query(&format!("SELECT _project_{kind}($1, $2)"))
        .bind(id)
        .bind(payload)
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("{kind} {id} must project: {e}"));
}

/// `ids` in replay's walk order (`replay::replay`).
async fn walk_order(pool: &PgPool, ids: &[Uuid]) -> Vec<Uuid> {
    sqlx::query_scalar(&format!(
        "SELECT e.id FROM kb_events e WHERE e.id = ANY($1) ORDER BY {}",
        replay::WALK_ORDER
    ))
    .bind(ids)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// One transaction declares an erasure and then maps an arrival into it, and the arrival's id
/// sorts below the declaration's. Replay walks the transaction in append order, so the erasure row
/// exists before the arrival's member row needs it.
///
/// FAILS IF: replay walks by id. The arrival then projects first, and its member row violates
/// `kb_erasure_members`' foreign key to an erasure not yet projected, so replay raises.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_walks_one_transaction_in_append_order_whatever_its_ids(pool: PgPool) {
    fresh(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_context(&pool, owner, "walk-tx-home").await;
    let r = make_resource(&pool, owner, emitter, home, "walk-tx-r").await;
    let erasure = Uuid::now_v7();
    let declared = serde_json::to_value(declaration(
        erasure,
        ErasureKind::Selection,
        None,
        vec![],
        vec![],
    ))
    .unwrap();
    let arrived = serde_json::to_value(ErasureMemberArrived {
        erasure_id: erasure,
        member: member(ErasureMemberTable::KbResources, r.uuid()),
        parent: None,
        source: ErasureArrivalSource::ClosingSurvey,
    })
    .unwrap();

    let (high, low) = same_millisecond();
    let mut tx = pool.begin().await.unwrap();
    append_as(&mut tx, emitter, high, "erasure_declared", &declared).await;
    append_as(&mut tx, emitter, low, "erasure_member_arrived", &arrived).await;
    tx.commit().await.unwrap();

    assert!(low < high, "setup: the later append has the lower id");
    assert_eq!(
        walk_order(&pool, &[high, low]).await,
        vec![high, low],
        "setup: the walk takes the declaration first, as it was appended"
    );
    assert_replay_byte_identical(
        &pool,
        "of one transaction whose ids invert",
        &["kb_erasures", "kb_erasure_members"],
    )
    .await;
}

/// An erasure completes, and a later transaction reopens it with an id that sorts below the
/// completion's, as a straggler's `erasure_reopened` in the completion's millisecond can on PG17.
/// Live, the erasure is not complete.
///
/// FAILS IF: replay walks by id. It then projects the reopening first and the completion last, and
/// the rebuilt erasure is complete (`completed_event_id` set) where the live one is not.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_reopening_whose_id_sorts_before_the_completion_replays_as_it_committed(pool: PgPool) {
    fresh(&pool).await;
    let (_, emitter) = system_actor(&pool).await;
    let erasure = Uuid::now_v7();
    declare(
        &pool,
        &declaration(erasure, ErasureKind::Selection, None, vec![], vec![]),
    )
    .await;

    let (high, low) = same_millisecond();
    let completed = serde_json::to_value(ErasureCompleted {
        erasure_id: erasure,
        redacted_hashes: vec![],
        late_digest: None,
    })
    .unwrap();
    let reopened = serde_json::to_value(ErasureReopened {
        erasure_id: erasure,
        arrival_event: high,
    })
    .unwrap();
    for (id, kind, payload) in [
        (high, "erasure_completed", &completed),
        (low, "erasure_reopened", &reopened),
    ] {
        let mut tx = pool.begin().await.unwrap();
        append_as(&mut tx, emitter, id, kind, payload).await;
        tx.commit().await.unwrap();
    }

    assert!(low < high, "setup: the reopening has the lower id");
    assert_eq!(
        walk_order(&pool, &[high, low]).await,
        vec![high, low],
        "setup: the walk takes the completion first, in append order"
    );
    assert_eq!(
        erasure_events(&pool, erasure).await,
        (None, None),
        "setup: live, the reopening cleared the completion"
    );
    assert_replay_byte_identical(
        &pool,
        "of a reopening whose id sorts before its completion",
        &["kb_erasures"],
    )
    .await;
}

/// Two batches refuse one item, the second in a later transaction with an id that sorts below the
/// first's. Live, the item carries the second refusal's reason and the first refusal's time.
///
/// FAILS IF: replay walks by id. It then projects the second refusal first, and the rebuilt row
/// carries the first refusal's reason and the second refusal's time.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn two_refusals_whose_ids_invert_replay_as_they_committed(pool: PgPool) {
    fresh(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_context(&pool, owner, "walk-refusals-home").await;
    let r = make_resource(&pool, owner, emitter, home, "walk-refusals-r").await;
    let erasure = Uuid::now_v7();
    declare(
        &pool,
        &declaration(
            erasure,
            ErasureKind::Resource,
            Some(("kb_resources", r.uuid())),
            vec![pending(ErasureMemberTable::KbResources, r.uuid(), None)],
            vec![],
        ),
    )
    .await;

    let (high, low) = same_millisecond();
    let refusal = |reason| {
        serde_json::to_value(ErasureBatchApplied {
            member_table: ErasureMemberTable::KbResources,
            entries: vec![refused(r.uuid(), vec![erasure], reason)],
        })
        .unwrap()
    };
    for (id, reason) in [
        (high, ErasureMemberRefusalReason::Defect),
        (low, ErasureMemberRefusalReason::Unclassified),
    ] {
        let mut tx = pool.begin().await.unwrap();
        append_as(
            &mut tx,
            emitter,
            id,
            "erasure_batch_applied",
            &refusal(reason),
        )
        .await;
        tx.commit().await.unwrap();
    }

    assert!(low < high, "setup: the second refusal has the lower id");
    assert_eq!(
        walk_order(&pool, &[high, low]).await,
        vec![high, low],
        "setup: the walk takes the first refusal first, in append order"
    );
    let (reason, at_first): (Option<String>, bool) = sqlx::query_as(
        "SELECT refusal_reason, first_refused_at = (SELECT occurred_at FROM kb_events WHERE id = $3) \
           FROM kb_erasure_members \
          WHERE erasure_id = $1 AND member_table = 'kb_resources' AND member_id = $2",
    )
    .bind(erasure)
    .bind(r.uuid())
    .bind(high)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (reason.as_deref(), at_first),
        (Some("unclassified"), true),
        "setup: live, the row carries the second reason and the first time"
    );
    assert_replay_byte_identical(
        &pool,
        "of two refusals whose ids invert",
        &["kb_erasures", "kb_erasure_members"],
    )
    .await;
}

/// A transaction takes its `xact_id` at its first write, and an act can then wait while another
/// transaction appends and commits. Here transaction A appends its first record, which assigns its
/// id; transaction B then declares an erasure and commits; and A, after B's commit, maps an
/// arrival into B's erasure and commits. A has the lower `xact_id` and the later arrival. Live,
/// the arrival's member row references an erasure B had already committed.
///
/// FAILS IF: replay walks by `(xact_id, seq)`. It then walks A's whole span, the arrival included,
/// before B's declaration, and the arrival's member row violates `kb_erasure_members`' foreign key
/// to an erasure not yet projected, so replay raises.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_append_after_another_transactions_commit_replays_after_it_whatever_its_xact_id(
    pool: PgPool,
) {
    fresh(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_context(&pool, owner, "walk-xact-home").await;
    let r = make_resource(&pool, owner, emitter, home, "walk-xact-r").await;
    let own = serde_json::to_value(declaration(
        Uuid::now_v7(),
        ErasureKind::Selection,
        None,
        vec![],
        vec![],
    ))
    .unwrap();
    let theirs = Uuid::now_v7();
    let declared = serde_json::to_value(declaration(
        theirs,
        ErasureKind::Selection,
        None,
        vec![],
        vec![],
    ))
    .unwrap();
    let arrived = serde_json::to_value(ErasureMemberArrived {
        erasure_id: theirs,
        member: member(ErasureMemberTable::KbResources, r.uuid()),
        parent: None,
        source: ErasureArrivalSource::ClosingSurvey,
    })
    .unwrap();
    let (a_first, b_declared, a_arrived) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

    // A: its first write assigns its transaction id.
    let mut a = pool.begin().await.unwrap();
    append_as(&mut a, emitter, a_first, "erasure_declared", &own).await;
    // B: begins after A took its id, appends, and commits while A is still open.
    let mut b = pool.begin().await.unwrap();
    append_as(&mut b, emitter, b_declared, "erasure_declared", &declared).await;
    b.commit().await.unwrap();
    // A: its next statement sees B's commit, and its arrival references B's erasure.
    append_as(
        &mut a,
        emitter,
        a_arrived,
        "erasure_member_arrived",
        &arrived,
    )
    .await;
    a.commit().await.unwrap();

    let (a_before_b, b_before_arrival): (bool, bool) = sqlx::query_as(
        "SELECT a.xact_id < b.xact_id, b.seq < arr.seq \
           FROM kb_events a, kb_events b, kb_events arr \
          WHERE a.id = $1 AND b.id = $2 AND arr.id = $3",
    )
    .bind(a_first)
    .bind(b_declared)
    .bind(a_arrived)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        a_before_b && b_before_arrival,
        "setup: A took the lower xact_id, and its arrival was appended after B's declaration"
    );
    assert_eq!(
        walk_order(&pool, &[a_first, b_declared, a_arrived]).await,
        vec![a_first, b_declared, a_arrived],
        "setup: the walk takes B's declaration inside A's span, in append order"
    );
    assert_replay_byte_identical(
        &pool,
        "of an append after another transaction's commit",
        &["kb_erasures", "kb_erasure_members"],
    )
    .await;
}

// ── the declaration's claims, verified ───────────────────────────────────────────────────────

/// An honest estate declaration of `map`, for a new subject `handle` that is also its one holder.
async fn honest_estate(
    pool: &PgPool,
    handle: &str,
    map: Vec<ErasureMapEntry>,
) -> serde_json::Value {
    let subject = common::insert_profile(pool, handle).await;
    serde_json::to_value(declaration(
        Uuid::now_v7(),
        ErasureKind::Estate,
        Some(("kb_profiles", subject)),
        map,
        vec![subject],
    ))
    .unwrap()
}

/// A declaration's counts, checksum and holders are verified against the map, never stored on
/// trust, and a field its kind does not carry raises. Each refused payload is an honest one with
/// one change, and nothing of it lands; each honest one projects.
///
/// FAILS IF: an estate projects with a checksum or a count that is not its map's, a count for a
/// table that is not a member table, or no checksum; a non-estate projects with counts, with a
/// checksum that is not its map's, or with no checksum; a legacy declaration projects with
/// counts, a checksum or holders; or (the controls) an honest estate, selection or legacy
/// declaration is refused.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_declaration_whose_claims_are_not_its_maps_is_refused(pool: PgPool) {
    fresh(&pool).await;
    let resource = Uuid::now_v7();
    let container = Uuid::now_v7();
    let parent = member(ErasureMemberTable::KbContexts, container);
    let map = || {
        vec![
            pending(ErasureMemberTable::KbContexts, container, None),
            pending(ErasureMemberTable::KbResources, resource, Some(parent)),
        ]
    };
    let flipped = |checksum: &serde_json::Value| {
        let mut text = checksum.as_str().expect("a checksum").to_owned();
        let last = if text.ends_with('0') { "1" } else { "0" };
        text.replace_range(text.len() - 1.., last);
        serde_json::json!(text)
    };
    let selection = || {
        serde_json::to_value(declaration(
            Uuid::now_v7(),
            ErasureKind::Selection,
            None,
            map(),
            vec![],
        ))
        .unwrap()
    };
    let legacy = || {
        serde_json::to_value(legacy_declaration(
            Uuid::now_v7(),
            resource,
            vec![pending(ErasureMemberTable::KbResources, resource, None)],
        ))
        .unwrap()
    };

    let mut cases: Vec<(&str, serde_json::Value, &str)> = Vec::new();
    let mut p = honest_estate(&pool, "claims-1", map()).await;
    p["checksum"] = flipped(&p["checksum"]);
    cases.push((
        "an estate whose checksum is not its map's",
        p,
        "the checksum is not the map's",
    ));
    let mut p = honest_estate(&pool, "claims-2", map()).await;
    p["counts"]["kb_resources"] = serde_json::json!(2);
    cases.push((
        "an estate that overcounts a table",
        p,
        "the estate counts are not the map's",
    ));
    let mut p = honest_estate(&pool, "claims-3", map()).await;
    p["counts"]["kb_blobs"] = serde_json::json!(1);
    cases.push((
        "an estate that counts a table its map does not hold",
        p,
        "the estate counts are not the map's",
    ));
    let mut p = honest_estate(&pool, "claims-4", map()).await;
    p["counts"]["kb_teams"] = serde_json::json!(0);
    cases.push((
        "an estate that counts a table that is not a member table",
        p,
        "the estate counts name a table that is not a member table",
    ));
    let mut p = honest_estate(&pool, "claims-5", map()).await;
    p.as_object_mut().unwrap().remove("checksum");
    cases.push((
        "an estate with no checksum",
        p,
        "a non-legacy declaration carries no checksum",
    ));
    let mut p = selection();
    p["counts"] = serde_json::json!({"kb_resources": 1, "kb_contexts": 1});
    cases.push((
        "a selection with counts",
        p,
        "only an estate declaration carries counts",
    ));
    let mut p = selection();
    p.as_object_mut().unwrap().remove("checksum");
    cases.push((
        "a selection with no checksum",
        p,
        "a non-legacy declaration carries no checksum",
    ));
    let mut p = selection();
    p["checksum"] = flipped(&p["checksum"]);
    cases.push((
        "a selection whose checksum is not its map's",
        p,
        "the checksum is not the map's",
    ));
    for (key, value) in [
        ("holders", serde_json::json!([Uuid::now_v7()])),
        ("checksum", serde_json::json!(DIGEST)),
        ("counts", serde_json::json!({"kb_resources": 1})),
    ] {
        let mut p = legacy();
        p[key] = value;
        cases.push((
            "a legacy declaration with a field it never carries",
            p,
            "a legacy declaration carries counts, a checksum or holders",
        ));
    }

    for (what, payload, says) in cases {
        let erasure = payload["erasure_id"].clone();
        let err = try_record(&pool, "erasure_declared", &payload)
            .await
            .expect_err(what);
        assert!(
            err.to_string().contains(says),
            "{what}: unexpected error: {err}"
        );
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_erasures WHERE id = $1::uuid")
            .bind(erasure.as_str().unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0, "{what}: nothing of the refused declaration lands");
    }

    for (what, payload) in [
        (
            "an honest estate",
            honest_estate(&pool, "claims-6", map()).await,
        ),
        ("an honest selection", selection()),
        ("an honest legacy declaration", legacy()),
    ] {
        try_record(&pool, "erasure_declared", &payload)
            .await
            .unwrap_or_else(|e| panic!("control: {what} projects: {e}"));
    }
}

/// Every non-legacy declaration carries its checksum, so D7.5 can re-derive every non-legacy
/// erasure's map (D1.1 defines the checksum for each one). A resource declaration, neither an
/// estate nor legacy, without one is refused, and nothing of it lands; the same declaration with
/// its checksum projects, and a legacy one projects without.
///
/// FAILS IF: the projector requires the checksum on an estate only, or verifies it only when it
/// is present; or (the controls) it requires one of a legacy declaration, or refuses an honest
/// resource declaration.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_non_estate_declaration_without_its_checksum_is_refused(pool: PgPool) {
    fresh(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_context(&pool, owner, "unchecked-home").await;
    let r = make_resource(&pool, owner, emitter, home, "unchecked-r").await;
    let resource_declaration = || {
        serde_json::to_value(declaration(
            Uuid::now_v7(),
            ErasureKind::Resource,
            Some(("kb_resources", r.uuid())),
            vec![pending(ErasureMemberTable::KbResources, r.uuid(), None)],
            vec![],
        ))
        .unwrap()
    };

    let mut unchecked = resource_declaration();
    assert!(
        unchecked
            .as_object_mut()
            .unwrap()
            .remove("checksum")
            .is_some(),
        "setup: the honest declaration carried a checksum"
    );
    let erasure = unchecked["erasure_id"].clone();
    let err = try_record(&pool, "erasure_declared", &unchecked)
        .await
        .expect_err("a non-legacy declaration without its checksum must be refused");
    assert!(
        err.to_string()
            .contains("a non-legacy declaration carries no checksum"),
        "unexpected error: {err}"
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_erasures WHERE id = $1::uuid")
        .bind(erasure.as_str().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "nothing of the refused declaration lands");
    assert!(
        resource_is_active(&pool, r).await,
        "setup: the refused declaration flagged nothing"
    );

    try_record(&pool, "erasure_declared", &resource_declaration())
        .await
        .expect("control: the same declaration with its checksum projects");
    let other = make_resource(&pool, owner, emitter, home, "unchecked-legacy").await;
    let legacy = serde_json::to_value(legacy_declaration(
        Uuid::now_v7(),
        other.uuid(),
        vec![pending(ErasureMemberTable::KbResources, other.uuid(), None)],
    ))
    .unwrap();
    assert!(
        legacy.get("checksum").is_none(),
        "setup: a legacy payload has no checksum"
    );
    try_record(&pool, "erasure_declared", &legacy)
        .await
        .expect("control: a legacy declaration projects without a checksum");
}
