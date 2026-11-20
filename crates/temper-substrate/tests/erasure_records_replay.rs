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
//!   * D7.5: replay rebuilds the four record tables byte for byte over a sequence holding a
//!     refusal and a born-erased row, and the checksum recomputed from the rebuilt map equals the
//!     `erasure_declared` payload's for every non-legacy erasure;
//!   * the golden checksum: one fixed map's D1.1 checksum, pinned to a literal;
//!   * the edge fold: an edge member folds with its edge-owned properties, an edge folded before
//!     keeps its own fold, and `relationship_folded` still folds and raises through the shared
//!     helper.

mod common;

use sha2::Digest;
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
/// It first waits two milliseconds. Replay walks the ledger in id order, and `uuid_generate_v7()`
/// has no order within a millisecond (20261116000010_kb_events_record_order.sql), so two appends
/// in one millisecond could replay in either order: a declaration after the arrival that needs it,
/// or the second refusal before the first. The wait puts every record in a later millisecond than
/// whatever was written before it.
async fn try_record(
    pool: &PgPool,
    kind: &str,
    payload: &serde_json::Value,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query("SELECT pg_sleep(0.002)").execute(pool).await?;
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

fn table_name(table: ErasureMemberTable) -> String {
    serde_json::to_value(table)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
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

/// D1.1, re-implemented here from the spec's text and independently of `erasure_map_checksum`, so
/// a test can compare the two. The header names the erasure and its manifest digest; each line is
/// one declared member, tab-separated, an absent parent or prior erasure written as the empty
/// string; lines sort by member table as bytes (`COLLATE "C"`), then by member id (a uuid compares
/// as its bytes in both Postgres and `uuid::Uuid`). Every separator is a single tab or newline
/// byte.
fn d1_1_checksum(erasure: Uuid, digest: &str, map: &[ErasureMapEntry]) -> String {
    let mut lines: Vec<(String, Uuid, String)> = map
        .iter()
        .map(|e| {
            let table = table_name(e.table);
            let (parent_table, parent_id) = match e.parent {
                Some(p) => (table_name(p.table), p.id.to_string()),
                None => (String::new(), String::new()),
            };
            let prior = e.prior_erasure.map(|p| p.to_string()).unwrap_or_default();
            let line = format!("{table}\t{}\t{parent_table}\t{parent_id}\t{prior}\n", e.id);
            (table, e.id, line)
        })
        .collect();
    lines.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()).then(a.1.cmp(&b.1)));
    let mut text = format!("temper-erasure-map/v1\t{erasure}\t{digest}\n");
    for (_, _, line) in lines {
        text.push_str(&line);
    }
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
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

/// Snapshot, reset, replay, and diff every projection table.
async fn assert_replay_byte_identical(pool: &PgPool, after_what: &str) {
    let before = replay::dump_projections(pool).await.unwrap();
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

/// A legacy declaration records `legacy = true` with no manifest and no request, even when its
/// payload carries them, and its erasure has no checksum.
///
/// FAILS IF: a legacy row keeps a stray manifest or request from its payload (the CHECKs would
/// refuse the insert), gains a snapshot, or has a checksum.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_legacy_erasure_declared_has_no_manifest_request_or_checksum(pool: PgPool) {
    fresh(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_context(&pool, owner, "legacy-home").await;
    let r = make_resource(&pool, owner, emitter, home, "legacy-r").await;

    let erasure = Uuid::now_v7();
    let mut payload = serde_json::to_value(legacy_declaration(
        erasure,
        r.uuid(),
        vec![pending(ErasureMemberTable::KbResources, r.uuid(), None)],
    ))
    .unwrap();
    // Stray keys a legacy payload should never carry: the projector does not take them.
    payload["request_ref"] = serde_json::json!(Uuid::now_v7());
    payload["manifest_version"] = serde_json::json!(MANIFEST_VERSION);
    payload["manifest_digest"] = serde_json::json!(DIGEST);
    let event = record(&pool, "erasure_declared", &payload).await;

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

// ── the edge fold (P3) ───────────────────────────────────────────────────────────────────────

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

    assert_replay_byte_identical(&pool, "of the erasure record").await;

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
