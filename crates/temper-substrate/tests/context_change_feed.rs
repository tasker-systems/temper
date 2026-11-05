#![cfg(feature = "test-db")]
//! Witnesses for the substrate half of the context change feed: migration 20261116000010's order
//! columns and `context_changes_after`.
//!
//! The goal is temper 01a1a1a2-e253-759c-8c82-3411a8b4e010 and the ordering ruling is R5 (temper
//! decision 01a1a1ad-b0ae-7562-88ea-2035b238d517). Each witness names the clause it holds and what
//! makes it fail. Every one fails against the ledger before the feed, because the function and the
//! columns do not exist there. The ordering witness also has a bite probe that leaves both in place:
//! drop the horizon filter, and transaction A is skipped.
//!
//! The world is built through the real write paths: `create_resource_with`, `fire`, and the erasure
//! act. The read is called as SQL, with `("0", 0)` as "the beginning". The SQL accepts any position.
//! Refusing a position this deployment did not issue is the service's job (PR 2).

mod common;

use sqlx::PgPool;
use temper_core::types::facet_requests::ANCHORED_AT_PROPERTY_KEY;
use temper_core::types::ids::EntityId;
use temper_core::types::property_owner::PropertyOwner;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, EdgeId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

/// The entry keys R4 allows. An entry is a pointer: anything else in it is a disclosure.
const ENTRY_KEYS: &[&str] = &["position", "kind", "subject", "actor", "occurred_at", "act"];

const BEGINNING: (&str, i64) = ("0", 0);

fn chunk(prose: &str) -> IncomingChunk {
    use sha2::Digest;
    IncomingChunk {
        chunk_index: 0,
        content_hash: hex::encode(sha2::Sha256::digest(prose.trim())),
        content: prose.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: "first".to_string(),
        heading_depth: 1,
    }
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

/// An operator distinct from every author, as an admin act's operator is: its own profile and
/// entity, a member of nothing.
async fn operator_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile = common::create_profile(pool, "operator@example.test").await;
    let entity: EntityId = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'operator@admin') RETURNING id",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .unwrap();
    (ProfileId::from(profile), entity)
}

async fn setup(pool: &PgPool) -> (ProfileId, EntityId, ContextId) {
    common::reset_schema(pool).await;
    temper_substrate::scenario::bootseed::seed_system(pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let home = ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), "feed", "feed")
            .await
            .unwrap(),
    );
    (owner, emitter, home)
}

async fn create(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    title: &str,
    body: &str,
    properties: &[(String, serde_json::Value)],
) -> ResourceId {
    let origin = format!("test://{title}");
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &origin,
            body,
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties,
            chunks: Some(vec![chunk(body)]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create through the create path")
}

fn set_action<'a>(
    resource: ResourceId,
    key: &'a str,
    value: &'a serde_json::Value,
    emitter: EntityId,
) -> SeedAction<'a> {
    SeedAction::PropertySet {
        resource,
        key,
        value,
        weight: 1.0,
        emitter,
    }
}

async fn set(pool: &PgPool, emitter: EntityId, resource: ResourceId, key: &str, value: &str) {
    let value = serde_json::json!(value);
    let mut tx = pool.begin().await.unwrap();
    fire(&mut tx, set_action(resource, key, &value, emitter))
        .await
        .expect("the write fires");
    tx.commit().await.unwrap();
}

/// One read. `None` is the refusal: the profile cannot read the context now.
async fn read(
    pool: &PgPool,
    profile: ProfileId,
    context: ContextId,
    after: Option<(&str, i64)>,
    limit: i32,
) -> Option<serde_json::Value> {
    sqlx::query_scalar("SELECT context_changes_after($1, $2, $3::text::xid8, $4, $5)")
        .bind(profile.uuid())
        .bind(context.uuid())
        .bind(after.map(|a| a.0.to_owned()))
        .bind(after.map(|a| a.1))
        .bind(limit)
        .fetch_one(pool)
        .await
        .expect("the read runs")
}

fn entries(answer: &serde_json::Value) -> &Vec<serde_json::Value> {
    answer["entries"].as_array().expect("entries is an array")
}

fn through(answer: &serde_json::Value) -> (String, i64) {
    (
        answer["through"]["xact"].as_str().unwrap().to_owned(),
        answer["through"]["seq"].as_i64().unwrap(),
    )
}

/// Wait until every transaction id assigned so far is below the horizon. The horizon is cluster-wide
/// (R5), and nextest runs these tests in parallel on one cluster, each migrating its own database
/// inside a transaction. So a read made straight after a commit can lag behind it. That lag is R5's
/// accepted cost, and a real reader meets it too. A witness that expects to see a committed write
/// waits for it to settle first. It never waits on a transaction the test itself holds open.
async fn settle(pool: &PgPool) {
    let target: String = sqlx::query_scalar("SELECT pg_snapshot_xmax(pg_current_snapshot())::text")
        .fetch_one(pool)
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let settled: bool =
            sqlx::query_scalar("SELECT pg_snapshot_xmin(pg_current_snapshot()) >= $1::text::xid8")
                .bind(&target)
                .fetch_one(pool)
                .await
                .unwrap();
        if settled {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the horizon did not pass xid {target} within 120 s"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Wait for every commit so far to settle, then read every entry from `from`, page by page with
/// bound `limit`. Returns the entries and the last `through`.
async fn drain(
    pool: &PgPool,
    profile: ProfileId,
    context: ContextId,
    from: (String, i64),
    limit: i32,
) -> (Vec<serde_json::Value>, (String, i64)) {
    settle(pool).await;
    let mut pos = from;
    let mut all = Vec::new();
    loop {
        let a = read(pool, profile, context, Some((&pos.0, pos.1)), limit)
            .await
            .expect("readable");
        all.extend(entries(&a).iter().cloned());
        pos = through(&a);
        if !a["has_more"].as_bool().unwrap() {
            return (all, pos);
        }
    }
}

fn subjects_of_kind(entries: &[serde_json::Value], kind: &str) -> Vec<String> {
    entries
        .iter()
        .filter(|e| e["kind"] == kind)
        .map(|e| e["subject"]["id"].as_str().unwrap().to_owned())
        .collect()
}

/// `no-change-lands-behind-a-passed-position`. Transaction A writes first and commits last. A reader
/// that reads while A is open, then resumes after A commits, receives A's change and B's change, in
/// `(xact_id, seq)` order. While A is open, B's change is held back: the cost is lag, never a skip.
///
/// FAILS IF: the read is not bounded by the settled horizon (bite probe: drop `e.xact_id < g.h`, and
/// the first read serves B, its `through` passes A, and A is never served), or rows are ordered by
/// anything that does not follow the order transactions took their ids.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_change_committed_after_a_read_never_lands_behind_its_position(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let ra = create(&pool, owner, emitter, home, "res-a", "a", &[]).await;
    let rb = create(&pool, owner, emitter, home, "res-b", "b", &[]).await;
    let (_, start) = drain(&pool, owner, home, (BEGINNING.0.into(), BEGINNING.1), 1000).await;

    let va = serde_json::json!("from-a");
    let vb = serde_json::json!("from-b");
    let mut tx_a = pool.begin().await.unwrap();
    fire(&mut tx_a, set_action(ra, "status", &va, emitter))
        .await
        .unwrap();
    let mut tx_b = pool.begin().await.unwrap();
    fire(&mut tx_b, set_action(rb, "status", &vb, emitter))
        .await
        .unwrap();
    tx_b.commit().await.unwrap();

    // A is open: neither A nor B is past the horizon yet.
    let held = read(&pool, owner, home, Some((&start.0, start.1)), 1000)
        .await
        .unwrap();
    assert!(
        entries(&held).is_empty(),
        "B committed but sits above an open A, so it is held back: {held}"
    );
    let mid = through(&held);

    tx_a.commit().await.unwrap();
    let (after, _) = drain(&pool, owner, home, mid, 1000).await;
    let set_subjects = subjects_of_kind(&after, "property_set");
    assert_eq!(
        set_subjects,
        vec![ra.uuid().to_string(), rb.uuid().to_string()],
        "both changes arrive, A (the earlier transaction id) first: {after:?}"
    );
}

/// `meta-only-changes-are-changes` and `a-change-says-what-changed`. A write that changes only a
/// resource's meta arrives as a `property_set` entry pointing at the resource and naming the actor.
///
/// FAILS IF: a meta-only write produces no entry, or the entry omits its subject or actor.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_meta_only_write_is_an_entry_naming_its_resource(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "meta", "body", &[]).await;
    let (_, pos) = drain(&pool, owner, home, (BEGINNING.0.into(), BEGINNING.1), 1000).await;

    set(&pool, emitter, r, "temper-stage", "in-progress").await;

    let (got, _) = drain(&pool, owner, home, pos, 1000).await;
    assert_eq!(got.len(), 1, "exactly the meta write: {got:?}");
    let e = &got[0];
    assert_eq!(e["kind"], "property_set");
    assert_eq!(e["subject"]["table"], "kb_resources");
    assert_eq!(e["subject"]["id"], r.uuid().to_string());
    assert_eq!(e["actor"], emitter.uuid().to_string());
    assert!(e["occurred_at"].is_string() && e["act"].is_string());
}

/// `an-answer-says-how-far-it-looked`. Every answer carries `through` and `has_more`. An empty answer
/// still says how far it looked. A bounded page says more exists, and paging with bound 1 serves
/// exactly what one unbounded read serves, in the same order.
///
/// FAILS IF: an answer lacks `through` or `has_more`, `has_more` is false on a cut page, or paging
/// skips or repeats an entry.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn every_answer_says_how_far_it_looked(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let props = vec![
        ("k1".to_string(), serde_json::json!("v1")),
        ("k2".to_string(), serde_json::json!("v2")),
    ];
    create(&pool, owner, emitter, home, "paged", "body", &props).await;
    settle(&pool).await;

    let whole = read(&pool, owner, home, Some(BEGINNING), 1000)
        .await
        .unwrap();
    assert!(
        entries(&whole).len() >= 3,
        "a create with two properties is several events"
    );
    assert_eq!(whole["has_more"], false);

    let first = read(&pool, owner, home, Some(BEGINNING), 1).await.unwrap();
    assert_eq!(entries(&first).len(), 1);
    assert_eq!(first["has_more"], true, "a cut page says more exists");
    assert_eq!(
        first["through"],
        entries(&first)[0]["position"],
        "a cut page reads through its last entry"
    );

    let (paged, end) = drain(&pool, owner, home, (BEGINNING.0.into(), BEGINNING.1), 1).await;
    assert_eq!(
        &paged,
        entries(&whole),
        "paging serves the same entries in the same order"
    );

    let empty = read(&pool, owner, home, Some((&end.0, end.1)), 100)
        .await
        .unwrap();
    assert!(entries(&empty).is_empty());
    assert_eq!(empty["has_more"], false);
    assert!(
        empty["through"]["xact"].is_string(),
        "an empty answer says how far it looked"
    );

    // F5: a read with no position serves nothing and hands back the current horizon.
    let none = read(&pool, owner, home, None, 100).await.unwrap();
    assert!(entries(&none).is_empty());
    assert_eq!(none["through"]["seq"], 0);

    // F6: a bound outside 1..1000 is refused, never clamped.
    for bad in [0, 1001] {
        let err = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT context_changes_after($1, $2, '0'::xid8, 0, $3)",
        )
        .bind(owner.uuid())
        .bind(home.uuid())
        .bind(bad)
        .fetch_one(&pool)
        .await
        .expect_err("an out-of-range bound is refused");
        assert!(err.to_string().contains("outside 1..1000"), "{err}");
    }
}

/// `the-feed-reads-under-standing-now`. A member of the owning team reads the team's context. Once
/// the membership is gone, the same read from the same position is refused (NULL), whatever standing
/// the reader held when they took the position.
///
/// FAILS IF: the read is not gated by `context_readable_by_profile` at the moment of the read.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_reader_who_loses_standing_is_refused(pool: PgPool) {
    let (owner, emitter, _) = setup(&pool).await;
    let team = common::create_team(&pool, "feed-team").await;
    let reader = ProfileId::from(common::create_profile(&pool, "reader@example.test").await);
    common::add_team_member(&pool, team, reader.uuid()).await;
    let ctx = ContextId::from(
        common::insert_context(&pool, "kb_teams", team, "team-feed", "team-feed")
            .await
            .unwrap(),
    );
    create(&pool, owner, emitter, ctx, "team-res", "body", &[]).await;
    settle(&pool).await;

    let held = read(&pool, reader, ctx, Some(BEGINNING), 100).await;
    let pos = through(held.as_ref().expect("a member can read the team context"));
    assert!(!entries(held.as_ref().unwrap()).is_empty());

    sqlx::query("DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
        .bind(team)
        .bind(reader.uuid())
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        read(&pool, reader, ctx, Some((&pos.0, pos.1)), 100).await,
        None,
        "standing is checked at read, not at the time the position was taken"
    );
    let stranger = ProfileId::from(common::create_profile(&pool, "stranger@example.test").await);
    assert_eq!(read(&pool, stranger, ctx, Some(BEGINNING), 100).await, None);
}

/// `an-entry-never-carries-content`. Tokens planted in a title, a meta key and value, and a body
/// appear nowhere in any answer, and every entry's keys lie within R4's allowlist. A key name is
/// content too: it is what erasure rewrites, so an entry that never carries one has nothing for
/// erasure to remove.
///
/// FAILS IF: any payload value (title, key, value, origin, blocks) is copied into an entry, or an
/// entry grows a key outside the allowlist.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn no_entry_carries_content(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let props = vec![("KEYTOKENzq".to_string(), serde_json::json!("VALUETOKENzq"))];
    let r = create(
        &pool,
        owner,
        emitter,
        home,
        "TITLETOKENzq",
        "BODYTOKENzq",
        &props,
    )
    .await;
    set(&pool, emitter, r, "SETKEYTOKENzq", "VALUETOKENzq").await;

    let (got, _) = drain(&pool, owner, home, (BEGINNING.0.into(), BEGINNING.1), 1000).await;
    assert!(!got.is_empty());
    let text = serde_json::to_string(&got).unwrap();
    for token in [
        "TITLETOKENzq",
        "KEYTOKENzq",
        "SETKEYTOKENzq",
        "VALUETOKENzq",
        "BODYTOKENzq",
    ] {
        assert!(
            !text.contains(token),
            "{token} leaked into the feed: {text}"
        );
    }
    for e in &got {
        for k in e.as_object().unwrap().keys() {
            assert!(
                ENTRY_KEYS.contains(&k.as_str()),
                "entry key {k} is outside R4: {e}"
            );
        }
    }
}

/// R5 for rows written before cutover, and for rows that would lie about their order. A row with no
/// recorded order is refused outside replay, and admitted under `temper.replaying`, where replay
/// restores legacy rows. The feed never serves such a row. A row naming a transaction id other than
/// its own is refused.
///
/// The planted row names no subject, so it is about the context itself. Only its missing order
/// withholds it.
///
/// FAILS IF: the insert trigger is missing or admits a NULL or forged order, or the feed serves an
/// unordered row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn rows_without_a_truthful_order_are_refused_or_never_served(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let insert = |xact: Option<&'static str>, seq: Option<i64>| {
        sqlx::query(
            "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, \
             producing_anchor_id, payload, category, xact_id, seq) \
             SELECT id, $1, 'kb_contexts', $2, '{}'::jsonb, \
                    category, $3::text::xid8, $4 \
               FROM kb_event_types WHERE name = 'resource_deleted'",
        )
        .bind(emitter)
        .bind(home.uuid())
        .bind(xact)
        .bind(seq)
    };

    let err = insert(None, None)
        .execute(&pool)
        .await
        .expect_err("NULL order refused");
    assert!(
        err.to_string().contains("must record xact_id and seq"),
        "{err}"
    );
    let err = insert(Some("1"), Some(1))
        .execute(&pool)
        .await
        .expect_err("forged xact refused");
    assert!(
        err.to_string().contains("not the inserting transaction"),
        "{err}"
    );

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    insert(None, None)
        .execute(&mut *tx)
        .await
        .expect("replay restores a legacy row");
    tx.commit().await.unwrap();

    let legacy: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events WHERE producing_anchor_id = $1 AND xact_id IS NULL",
    )
    .bind(home.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(legacy, 1);
    let (got, _) = drain(&pool, owner, home, (BEGINNING.0.into(), BEGINNING.1), 1000).await;
    assert!(
        subjects_of_kind(&got, "resource_deleted").is_empty(),
        "an unordered row is never served: {got:?}"
    );
}

/// R5's audit property. Within one transaction, `seq` order is the order the events were appended,
/// whatever order their ids sort in.
///
/// FAILS IF: `seq` is not taken from a per-backend monotonic source at insert.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn seq_follows_append_order_within_a_transaction(pool: PgPool) {
    let (_, emitter, home) = setup(&pool).await;
    let marks: Vec<Uuid> = (0..50).map(|_| Uuid::now_v7()).collect();
    let mut tx = pool.begin().await.unwrap();
    for m in &marks {
        sqlx::query(
            "SELECT _event_append('resource_deleted', $1, 'kb_contexts', $2, \
             jsonb_build_object('resource_id', $3::uuid))",
        )
        .bind(emitter)
        .bind(home.uuid())
        .bind(m)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();

    let by_seq: Vec<Uuid> = sqlx::query_scalar(
        "SELECT (payload->>'resource_id')::uuid FROM kb_events \
          WHERE producing_anchor_id = $1 AND payload ? 'resource_id' ORDER BY seq",
    )
    .bind(home.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(by_seq, marks, "seq order is append order");
    let xacts: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT xact_id) FROM kb_events WHERE producing_anchor_id = $1",
    )
    .bind(home.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(xacts, 1, "one transaction, one xact_id");
}

async fn edge(
    pool: &PgPool,
    emitter: EntityId,
    home: ContextId,
    source: ResourceId,
    target: ResourceId,
) -> EdgeId {
    writes::assert_anchored_edge_with(
        pool,
        writes::AssertAnchoredEdgeParams {
            source: AnchorRef::resource(source),
            target: AnchorRef::resource(target),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("rel"),
            weight: 1.0,
            home: EdgeHome::Context(home),
            emitter,
        },
        EventContext::default(),
    )
    .await
    .expect("assert the edge")
}

/// `a-change-says-what-changed`, for the property kinds whose payload names its owner as an
/// `AnchorRef`. An unset points at the resource it unset a key on. A retraction points at the edge
/// that owned the retracted row.
///
/// FAILS IF: the owner is read as anything but `{table, id}` (then an unset names the context).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn unset_and_retracted_entries_name_their_owner(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "unset-me", "a", &[]).await;
    let r2 = create(&pool, owner, emitter, home, "other-end", "b", &[]).await;
    set(&pool, emitter, r, "dropkey", "v").await;
    let e = edge(&pool, emitter, home, r, r2).await;
    let address = serde_json::json!({
        "endpoint": "source",
        "address": format!("{r}#{}", Uuid::now_v7()),
    });
    let property = writes::assert_keyed_property_with(
        &pool,
        PropertyOwner::Edge { id: e },
        ANCHORED_AT_PROPERTY_KEY,
        &address,
        1.0,
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();
    let (_, pos) = drain(&pool, owner, home, (BEGINNING.0.into(), BEGINNING.1), 1000).await;

    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::PropertyUnset {
            resource: r,
            key: "dropkey",
            emitter,
        },
    )
    .await
    .expect("the unset fires");
    tx.commit().await.unwrap();
    writes::retract_property_with(&pool, e, property, emitter, EventContext::default())
        .await
        .unwrap();

    let (got, _) = drain(&pool, owner, home, pos, 1000).await;
    let unset: Vec<_> = got
        .iter()
        .filter(|x| x["kind"] == "property_unset")
        .collect();
    assert_eq!(unset.len(), 1, "one unset: {got:?}");
    assert_eq!(unset[0]["subject"]["table"], "kb_resources");
    assert_eq!(unset[0]["subject"]["id"], r.uuid().to_string());
    let retracted: Vec<_> = got
        .iter()
        .filter(|x| x["kind"] == "property_retracted")
        .collect();
    assert_eq!(retracted.len(), 1, "one retraction: {got:?}");
    assert_eq!(retracted[0]["subject"]["table"], "kb_edges");
    assert_eq!(retracted[0]["subject"]["id"], e.uuid().to_string());
}

/// The pointer ruling (2026-11-03, task 01a1a1a5-773a-7599-a157-6128705ef71e). The context gate is
/// the whole gate for resources, blocks and blobs. A subject that has left the reader's view is still
/// pointed at, and its own read is what refuses it. That refusal is how a relay learns it went. Edges
/// are the exception (owner ruling 2026-11-05, migration 20261117000020): an edge whose endpoint the
/// reader cannot read is never pointed at, as `edges_visible_to` never admits it. Every one of these
/// is anchored on the shared context:
///   - an edge homed there whose target lives in a context the reader cannot read (withheld);
///   - a resource created there and since moved to that context;
///   - a resource deleted there;
///   - a resource erased there.
/// A deletion and a move are each an entry in the context they left (ruled 2026-11-03, migration
/// 20261116000020): `resource_deleted`, and one `resource_departed` that names no destination. An
/// erasure is an admin act, and the feed never serves an admin act's events (ruled the same day):
/// it writes no departure, its fold of the erased resource's edge is withheld, and neither its
/// operator nor its request reference reaches the reader. The erased resource's past entries stay,
/// as pointers. None of the titles or key names reaches the reader.
///
/// FAILS IF: the read filters resources by subject (then a departed subject's entries vanish and a
/// relay is never told), the hidden edge is pointed at, a move leaves the old home silent, a departure names where the resource went, an
/// erasure's act reaches the feed, or a subject's own read admits what the reader can no longer
/// read.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_subject_that_left_view_is_still_pointed_at_and_its_own_read_refuses(pool: PgPool) {
    let (owner, emitter, _) = setup(&pool).await;
    let team = common::create_team(&pool, "feed-team").await;
    let reader = ProfileId::from(common::create_profile(&pool, "reader@example.test").await);
    common::add_team_member(&pool, team, reader.uuid()).await;
    let shared = ContextId::from(
        common::insert_context(&pool, "kb_teams", team, "shared", "shared")
            .await
            .unwrap(),
    );
    let private = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner.uuid(), "private", "private")
            .await
            .unwrap(),
    );

    let a1 = create(&pool, owner, emitter, shared, "a1", "a1", &[]).await;
    let p1 = create(&pool, owner, emitter, private, "p1", "p1", &[]).await;
    let hidden_edge = edge(&pool, emitter, shared, a1, p1).await;

    let moved = create(&pool, owner, emitter, shared, "MOVEDTITLEzq", "m", &[]).await;
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceRehome {
            resource: moved,
            home: AnchorRef::context(private),
            emitter,
        },
    )
    .await
    .expect("the rehome fires");
    tx.commit().await.unwrap();

    let deleted = create(&pool, owner, emitter, shared, "DELETEDTITLEzq", "d", &[]).await;
    writes::delete_resource(&pool, deleted, emitter)
        .await
        .unwrap();

    let erased = create(
        &pool,
        owner,
        emitter,
        shared,
        "ERASEDTITLEzq",
        "e",
        &[("ERASEDKEYzq".to_string(), serde_json::json!("v"))],
    )
    .await;
    let erased_edge = edge(&pool, emitter, shared, a1, erased).await;
    let (_, operator_entity) = operator_actor(&pool).await;
    let request = Uuid::now_v7();
    sqlx::query_scalar::<_, serde_json::Value>("SELECT resource_erasure_execute($1,$2,$3,$4)")
        .bind(erased.uuid())
        .bind(operator_entity)
        .bind(operator_entity)
        .bind(request)
        .fetch_one(&pool)
        .await
        .expect("the erasure completes");

    let (whole, _) = drain(
        &pool,
        reader,
        shared,
        (BEGINNING.0.into(), BEGINNING.1),
        1000,
    )
    .await;
    let pointed = |id: Uuid| -> Vec<&str> {
        whole
            .iter()
            .filter(|e| e["subject"]["id"] == id.to_string())
            .map(|e| e["kind"].as_str().unwrap())
            .collect()
    };

    assert!(
        pointed(hidden_edge.uuid()).is_empty(),
        "the hidden edge is pointed at: {whole:?}"
    );
    // Past entries stay: no subject filter withholds what was written while the resource lived here.
    assert!(
        pointed(moved.uuid()).contains(&"resource_created"),
        "the moved resource's past is still pointed at: {whole:?}"
    );
    assert!(
        pointed(erased.uuid()).contains(&"resource_created"),
        "the erased resource's past is still pointed at: {whole:?}"
    );
    assert!(
        pointed(deleted.uuid()).contains(&"resource_deleted"),
        "the deletion itself is an entry: {whole:?}"
    );
    let departures = |id: Uuid| {
        pointed(id)
            .iter()
            .filter(|k| **k == "resource_departed")
            .count()
    };
    assert_eq!(
        departures(moved.uuid()),
        1,
        "the move is one departure here: {whole:?}"
    );
    assert_eq!(
        departures(erased.uuid()),
        0,
        "an erasure departs: {whole:?}"
    );
    // The move's departure is its act's own event: it carries the move's correlation id. The move
    // was fired without one, so this is the id the move's own record took.
    let move_act: Uuid = sqlx::query_scalar(
        "SELECT e.correlation_id FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'resource_rehomed' AND e.payload->>'resource_id' = $1::text",
    )
    .bind(moved.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let move_departure = whole
        .iter()
        .find(|e| {
            e["kind"] == "resource_departed" && e["subject"]["id"] == moved.uuid().to_string()
        })
        .unwrap();
    assert_eq!(
        move_departure["act"],
        move_act.to_string(),
        "the move's departure pairs with its record"
    );
    // The erasure's act is silent: its edge fold is withheld, and neither its operator nor its
    // request reference is served. Anti-vacuity: the fold is anchored on the shared context.
    let fold_anchored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'relationship_folded' AND e.payload->>'edge_id' = $1::text \
            AND e.producing_anchor_id = $2 AND e.correlation_id = $3",
    )
    .bind(erased_edge.uuid())
    .bind(shared.uuid())
    .bind(request)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        fold_anchored, 1,
        "the erasure folded the edge on the shared context"
    );
    assert!(
        !pointed(erased_edge.uuid()).contains(&"relationship_folded"),
        "the erasure's fold was served: {whole:?}"
    );
    let text = serde_json::to_string(&whole).unwrap();
    assert!(
        !text.contains(&operator_entity.to_string()),
        "the erasure's operator was served: {text}"
    );
    assert!(
        !text.contains(&request.to_string()),
        "the erasure's request reference was served: {text}"
    );
    // The departure's record carries the resource id and nothing else: no destination.
    let payloads: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'resource_departed' ORDER BY e.xact_id, e.seq",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        payloads,
        vec![serde_json::json!({ "resource_id": moved.uuid() })],
        "a departure carries only the resource id"
    );
    assert!(
        pointed(p1.uuid()).is_empty(),
        "the private resource has no event here"
    );

    let text = serde_json::to_string(&whole).unwrap();
    for token in [
        "MOVEDTITLEzq",
        "DELETEDTITLEzq",
        "ERASEDTITLEzq",
        "ERASEDKEYzq",
    ] {
        assert!(!text.contains(token), "{token} reached the reader: {text}");
    }

    let visible = |sql: &'static str, id: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(sql)
                .bind(reader.uuid())
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    const RESOURCE: &str =
        "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) WHERE resource_id = $2)";
    const EDGE: &str = "SELECT EXISTS (SELECT 1 FROM edges_visible_to($1) WHERE edge_id = $2)";
    // Anti-vacuity: the subject reads admit what the reader can read.
    assert!(
        visible(RESOURCE, a1.uuid()).await,
        "a readable resource reads"
    );
    for (what, id) in [
        ("moved resource", moved.uuid()),
        ("deleted resource", deleted.uuid()),
        ("erased resource", erased.uuid()),
    ] {
        assert!(!visible(RESOURCE, id).await, "the {what} still reads");
    }
    assert!(
        !visible(EDGE, hidden_edge.uuid()).await,
        "the hidden edge still reads"
    );
}

/// The departure's rule (migration 20261116000020): a resource departs once, and only from a home
/// it is leaving. A deleted resource has its deletion as its one way out, so moving it afterwards
/// writes no departure. A move to the home the resource is already in is no departure.
///
/// FAILS IF: the departure skips the live check (then the deleted resource's move departs), or the
/// same-home check (then the same-home move departs).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_resource_departs_once_and_only_from_a_home_it_leaves(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let gone = create(&pool, owner, emitter, home, "gone", "g", &[]).await;
    writes::delete_resource(&pool, gone, emitter).await.unwrap();
    let elsewhere = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner.uuid(), "elsewhere", "elsewhere")
            .await
            .unwrap(),
    );
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceRehome {
            resource: gone,
            home: AnchorRef::context(elsewhere),
            emitter,
        },
    )
    .await
    .expect("a deleted resource can still be moved");
    tx.commit().await.unwrap();

    let stays = create(&pool, owner, emitter, home, "stays", "s", &[]).await;
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceRehome {
            resource: stays,
            home: AnchorRef::context(home),
            emitter,
        },
    )
    .await
    .expect("the move to its own home fires");
    tx.commit().await.unwrap();

    let (got, _) = drain(&pool, owner, home, (BEGINNING.0.into(), BEGINNING.1), 1000).await;
    let kinds = |id: Uuid| -> Vec<&str> {
        got.iter()
            .filter(|e| e["subject"]["id"] == id.to_string())
            .map(|e| e["kind"].as_str().unwrap())
            .collect()
    };
    assert!(kinds(gone.uuid()).contains(&"resource_deleted"), "{got:?}");
    assert!(
        !kinds(gone.uuid()).contains(&"resource_departed"),
        "a deleted resource departs again when moved: {got:?}"
    );
    assert!(
        kinds(stays.uuid()).contains(&"resource_rehomed"),
        "anti-vacuity: the same-home move fired: {got:?}"
    );
    assert!(
        !kinds(stays.uuid()).contains(&"resource_departed"),
        "a move to the same home departs: {got:?}"
    );
}

/// The departure's rule under concurrency (code review of PR 1, I1). Two moves of one resource out
/// of the same home, in overlapping transactions, serialize on the resource: the second reads the
/// home the first left it in. Each home records exactly the one departure that left it.
///
/// FAILS IF: the departure reads the home without locking the resource first (then both moves
/// depart from the first home, and the second home never sees the resource leave).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn concurrent_moves_each_depart_from_the_home_they_left(pool: PgPool) {
    let (owner, emitter, a) = setup(&pool).await;
    let b = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner.uuid(), "b", "b")
            .await
            .unwrap(),
    );
    let c = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner.uuid(), "c", "c")
            .await
            .unwrap(),
    );
    let r = create(&pool, owner, emitter, a, "racer", "r", &[]).await;

    let mut first = pool.begin().await.unwrap();
    fire(
        &mut first,
        SeedAction::ResourceRehome {
            resource: r,
            home: AnchorRef::context(b),
            emitter,
        },
    )
    .await
    .expect("the first move fires");

    let second_pool = pool.clone();
    let second = tokio::spawn(async move {
        let mut tx = second_pool.begin().await.unwrap();
        fire(
            &mut tx,
            SeedAction::ResourceRehome {
                resource: r,
                home: AnchorRef::context(c),
                emitter,
            },
        )
        .await
        .expect("the second move fires");
        tx.commit().await.unwrap();
    });

    // The second move must be waiting on the first before the first commits, or this is no race.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                         WHERE datname = current_database() AND wait_event_type = 'Lock')",
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the second move never waited on the first"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    first.commit().await.unwrap();
    second.await.unwrap();

    let departures: Vec<Uuid> = sqlx::query_scalar(
        "SELECT e.producing_anchor_id FROM kb_events e \
           JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'resource_departed' ORDER BY e.xact_id, e.seq",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        departures,
        vec![a.uuid(), b.uuid()],
        "one departure from each home the resource left"
    );
}

/// The gate answers before the bound (`context_changes_after`'s comment on `_feed_refuse_limit`).
/// A caller who cannot read a context is answered NULL whatever bound it sends, so a bad bound
/// never tells it the context exists.
///
/// FAILS IF: the bound is checked before the gate (then an unreadable context answers a bound
/// error instead of NULL).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_unreadable_context_is_refused_whatever_bound_is_sent(pool: PgPool) {
    let (owner, _, home) = setup(&pool).await;
    let stranger = ProfileId::from(common::create_profile(&pool, "stranger@example.test").await);
    for bound in [0, 1001, -1] {
        let answer: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT context_changes_after($1, $2, '0'::xid8, 0, $3)")
                .bind(stranger.uuid())
                .bind(home.uuid())
                .bind(bound)
                .fetch_one(&pool)
                .await
                .expect("an unreadable context is answered, not refused for its bound");
        assert!(answer.is_none(), "bound {bound} answered: {answer:?}");
    }
    // Anti-vacuity: the owner, who can read it, is refused for the same bound.
    let refused = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT context_changes_after($1, $2, '0'::xid8, 0, 0)",
    )
    .bind(owner.uuid())
    .bind(home.uuid())
    .fetch_one(&pool)
    .await;
    assert!(refused.is_err(), "a readable context refuses bound 0");
}

/// The admin-act ruling (2026-11-03): the feed never serves an admin act's events, of any kind.
/// A field scrub in clear mode writes a `property_unset` on a live resource, carrying the operator
/// and the request reference;
/// a `blob_erased` is a domain kind anchored on the context. Neither is served. The reader's own
/// change to the same resource still is.
///
/// FAILS IF: the read serves an event sharing its correlation id and emitter with an admin record,
/// or serves `blob_erased` / `context_erased` by kind.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn admin_acts_are_never_served(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let live = create(&pool, owner, emitter, home, "live", "l", &[]).await;
    set(&pool, emitter, live, "scrubkeyzq", "v").await;
    let set_event: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'property_set' AND e.payload->>'property_key' = 'scrubkeyzq'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let (operator, operator_entity) = operator_actor(&pool).await;
    let request = Uuid::now_v7();
    sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT resource_field_scrub_execute($1, 'property', $2, true, $3, $4, $5)",
    )
    .bind(live.uuid())
    .bind(set_event)
    .bind(operator.uuid())
    .bind(operator_entity)
    .bind(request)
    .fetch_one(&pool)
    .await
    .expect("the scrub completes");
    let strike: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, \
                                producing_anchor_id, payload) \
         SELECT id, $1, 'kb_contexts', $2, '{}'::jsonb FROM kb_event_types \
          WHERE name = 'blob_erased' RETURNING id",
    )
    .bind(operator_entity)
    .bind(home.uuid())
    .fetch_one(&pool)
    .await
    .expect("a blob_erased anchored on the context");

    // Anti-vacuity: the scrub wrote domain events on this context under the act's reference.
    let scrub_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE e.producing_anchor_id = $1 AND e.correlation_id = $2 AND et.category = 'domain'",
    )
    .bind(home.uuid())
    .bind(request)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(scrub_events > 0, "the scrub wrote on the context");

    let (got, _) = drain(&pool, owner, home, (BEGINNING.0.into(), BEGINNING.1), 1000).await;
    let text = serde_json::to_string(&got).unwrap();
    assert!(
        !text.contains(&request.to_string()),
        "the scrub's act was served: {text}"
    );
    assert!(
        !text.contains(&operator_entity.to_string()),
        "an admin act's operator was served: {text}"
    );
    assert!(
        !got.iter().any(|e| e["kind"] == "blob_erased"),
        "a blob_erased was served: {text}"
    );
    let _ = strike;
    assert!(
        got.iter()
            .any(|e| e["kind"] == "property_set" && e["actor"] == emitter.uuid().to_string()),
        "the reader's own set is still served: {text}"
    );
}

/// Every entry the ledger anchored on `context` that names `edge`, as the feed derives an edge
/// subject: `edge_id` in the payload, or an edge-owned property.
async fn ledger_edge_entries(pool: &PgPool, context: ContextId, edge: EdgeId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e \
          WHERE e.producing_anchor_table = 'kb_contexts' AND e.producing_anchor_id = $1 \
            AND e.category = 'domain' AND e.xact_id IS NOT NULL \
            AND (e.payload->>'edge_id' = $2::text \
                 OR (e.payload->'owner'->>'table' = 'kb_edges' AND e.payload->'owner'->>'id' = $2::text))",
    )
    .bind(context.uuid())
    .bind(edge.uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn touch_edge(pool: &PgPool, emitter: EntityId, e: EdgeId, end: ResourceId) {
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::RelationshipReweight {
            edge: e,
            weight: 0.5,
            emitter,
        },
    )
    .await
    .expect("the reweight fires");
    tx.commit().await.unwrap();
    let address = serde_json::json!({
        "endpoint": "source",
        "address": format!("{end}#{}", Uuid::now_v7()),
    });
    writes::assert_keyed_property_with(
        pool,
        PropertyOwner::Edge { id: e },
        ANCHORED_AT_PROPERTY_KEY,
        &address,
        1.0,
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();
}

async fn fold_edge(pool: &PgPool, emitter: EntityId, e: EdgeId) {
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::RelationshipFold {
            edge: e,
            reason: None,
            emitter,
        },
    )
    .await
    .expect("the fold fires");
    tx.commit().await.unwrap();
}

/// Owner ruling 2026-11-05 (cross-review tasker-systems/temper-artifacts#203, migration
/// 20261117000020). An edge-subject entry is served only while the reader can read both endpoints
/// NOW, as `edges_visible_to` answers, whatever the entry's kind: the assertion, a reweight, an
/// edge-owned property, a fold. Three edges homed in the shared context, each touched the same way:
///   - between two resources the reader reads: every entry served, its fold included;
///   - to a resource in a context the reader cannot read: nothing served;
///   - to a resource the reader read when the edge was made, which has since moved out: nothing
///     served, its earlier entries included. The endpoint's own departure is served, which is how a
///     relay learns to drop the edge.
/// The owner reads both contexts and is served all three: the filter is the reader's standing, not
/// a drop of every edge.
///
/// FAILS IF: edge subjects are not filtered (the hidden edges are served), only one endpoint is
/// tested, the filter tests the fold (the visible edge's fold is withheld), or edge-owned property
/// entries escape the filter.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_is_served_only_while_the_reader_can_read_both_endpoints(pool: PgPool) {
    let (owner, emitter, _) = setup(&pool).await;
    let team = common::create_team(&pool, "edge-team").await;
    let reader = ProfileId::from(common::create_profile(&pool, "edge-reader@example.test").await);
    common::add_team_member(&pool, team, reader.uuid()).await;
    common::add_team_member(&pool, team, owner.uuid()).await;
    let shared = ContextId::from(
        common::insert_context(&pool, "kb_teams", team, "shared", "shared")
            .await
            .unwrap(),
    );
    let private = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner.uuid(), "private", "private")
            .await
            .unwrap(),
    );
    let a1 = create(&pool, owner, emitter, shared, "a1", "a1", &[]).await;
    let a2 = create(&pool, owner, emitter, shared, "a2", "a2", &[]).await;
    let p1 = create(&pool, owner, emitter, private, "p1", "p1", &[]).await;
    let leaver = create(&pool, owner, emitter, shared, "leaver", "l", &[]).await;

    let visible = edge(&pool, emitter, shared, a1, a2).await;
    let hidden = edge(&pool, emitter, shared, a1, p1).await;
    // Hidden by its SOURCE: only a test of both endpoints withholds it.
    let hidden_source = edge(&pool, emitter, shared, p1, a2).await;
    let later = edge(&pool, emitter, shared, a1, leaver).await;
    for e in [visible, hidden, hidden_source, later] {
        touch_edge(&pool, emitter, e, a1).await;
    }
    fold_edge(&pool, emitter, visible).await;
    fold_edge(&pool, emitter, hidden).await;

    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceRehome {
            resource: leaver,
            home: AnchorRef::context(private),
            emitter,
        },
    )
    .await
    .expect("the rehome fires");
    tx.commit().await.unwrap();

    let from = (BEGINNING.0.to_owned(), BEGINNING.1);
    let (seen, _) = drain(&pool, reader, shared, from.clone(), 1000).await;
    let (owner_seen, _) = drain(&pool, owner, shared, from, 1000).await;
    let served = |got: &[serde_json::Value], e: EdgeId| -> Vec<String> {
        got.iter()
            .filter(|x| {
                x["subject"]["table"] == "kb_edges" && x["subject"]["id"] == e.uuid().to_string()
            })
            .map(|x| x["kind"].as_str().unwrap().to_owned())
            .collect()
    };

    for (what, e) in [
        ("visible", visible),
        ("hidden", hidden),
        ("hidden-source", hidden_source),
        ("later", later),
    ] {
        let ledger = ledger_edge_entries(&pool, shared, e).await;
        // Anti-vacuity: each edge's assertion, reweight and property are in the shared ledger, and
        // the owner, who reads both endpoints, is served every one of them.
        assert!(ledger >= 3, "{what}: the ledger holds {ledger} entries");
        assert_eq!(
            served(&owner_seen, e).len() as i64,
            ledger,
            "{what}: the owner is served the whole edge: {owner_seen:?}"
        );
        let expect = if e == visible { ledger } else { 0 };
        assert_eq!(
            served(&seen, e).len() as i64,
            expect,
            "{what}: the reader is served {:?}",
            served(&seen, e)
        );
    }
    assert!(
        served(&seen, visible).contains(&"relationship_folded".to_owned()),
        "the visible edge's fold is a change the reader could see: {seen:?}"
    );
    assert!(
        served(&seen, visible).contains(&"property_asserted".to_owned()),
        "the visible edge's property is served: {seen:?}"
    );
    assert_eq!(
        subjects_of_kind(&seen, "resource_departed"),
        vec![leaver.uuid().to_string()],
        "the endpoint's departure is served"
    );
    let edge_visible = |e: EdgeId| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM edges_visible_to($1) WHERE edge_id = $2)",
            )
            .bind(reader.uuid())
            .bind(e.uuid())
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    // The feed and the edge's own read agree on the live hidden edges.
    for e in [hidden_source, later] {
        assert!(!edge_visible(e).await, "a hidden edge reads");
    }
}

/// The edge filter runs before the page bound (migration 20261117000020). A page with `has_more`
/// is never short, and reading one entry at a time serves exactly what one large page serves, in
/// the same order, though withheld edge entries and webhook receipts sit between the served ones.
///
/// FAILS IF: the filter runs after the bound (then pages come back short, or `has_more` counts what
/// is withheld), or `through` skips a served entry.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_page_is_never_short_for_what_it_withholds(pool: PgPool) {
    let (owner, emitter, _) = setup(&pool).await;
    let team = common::create_team(&pool, "page-team").await;
    let reader = ProfileId::from(common::create_profile(&pool, "page-reader@example.test").await);
    common::add_team_member(&pool, team, reader.uuid()).await;
    let shared = ContextId::from(
        common::insert_context(&pool, "kb_teams", team, "shared", "shared")
            .await
            .unwrap(),
    );
    let private = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner.uuid(), "private", "private")
            .await
            .unwrap(),
    );
    let p1 = create(&pool, owner, emitter, private, "p1", "p1", &[]).await;
    let mut kept = Vec::new();
    for i in 0..4 {
        let r = create(&pool, owner, emitter, shared, &format!("r{i}"), "r", &[]).await;
        let e = edge(&pool, emitter, shared, r, p1).await;
        touch_edge(&pool, emitter, e, r).await;
        insert_webhook(&pool, emitter, shared).await;
        kept.push(r);
    }

    let (whole, _) = drain(
        &pool,
        reader,
        shared,
        (BEGINNING.0.into(), BEGINNING.1),
        1000,
    )
    .await;
    assert!(whole.len() >= kept.len(), "anti-vacuity: {whole:?}");
    assert!(
        whole.iter().all(|x| x["subject"]["table"] != "kb_edges"),
        "every edge here is hidden: {whole:?}"
    );

    let mut pos = (BEGINNING.0.to_owned(), BEGINNING.1);
    let mut one_by_one = Vec::new();
    loop {
        let a = read(&pool, reader, shared, Some((&pos.0, pos.1)), 1)
            .await
            .expect("readable");
        let page = entries(&a).clone();
        pos = through(&a);
        if a["has_more"].as_bool().unwrap() {
            assert_eq!(page.len(), 1, "a page with more after it is full: {a}");
            one_by_one.extend(page);
        } else {
            assert!(page.len() <= 1, "{a}");
            one_by_one.extend(page);
            break;
        }
    }
    assert_eq!(
        one_by_one, whole,
        "paging one at a time serves the same entries"
    );
}

/// A webhook receipt anchored on the connection's home context (20260819000020's shape: a domain
/// kind, the context as subject). Inserted directly: the intake path lives above this crate. The
/// boot-seed stamps only typed kinds, so `reset_schema` leaves this foreign one unregistered: register
/// it as 20260819000020 does.
async fn insert_webhook(pool: &PgPool, emitter: EntityId, context: ContextId) {
    sqlx::query(
        "INSERT INTO kb_event_types (name, payload_schema, schema_version, category) \
         VALUES ('webhook_received', NULL, 1, 'domain') ON CONFLICT (name) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("the receipt kind registers");
    let landed = sqlx::query(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, \
         producing_anchor_id, payload, category) \
         SELECT id, $1, 'kb_contexts', $2, '{\"v\":2,\"body\":{}}'::jsonb, category \
           FROM kb_event_types WHERE name = 'webhook_received'",
    )
    .bind(emitter)
    .bind(context.uuid())
    .execute(pool)
    .await
    .expect("the receipt lands")
    .rows_affected();
    assert_eq!(landed, 1, "one receipt row");
}

/// Owner ruling 2026-11-05 (migration 20261117000020). An inbound webhook is no change to the
/// context: the feed never serves `webhook_received`, whoever reads, and naming it in `kinds`
/// serves nothing. Outside the feed only a subscription manager reads a receipt.
///
/// FAILS IF: `context_changes_after` serves the kind (with or without `kinds` naming it).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_inbound_webhook_is_no_change_to_the_context(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    insert_webhook(&pool, emitter, home).await;
    let r = create(&pool, owner, emitter, home, "after", "a", &[]).await;

    let anchored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'webhook_received' AND e.producing_anchor_id = $1 \
            AND e.category = 'domain' AND e.xact_id IS NOT NULL",
    )
    .bind(home.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        anchored, 1,
        "anti-vacuity: the receipt is an ordered domain row here"
    );

    let (got, _) = drain(&pool, owner, home, (BEGINNING.0.into(), BEGINNING.1), 1000).await;
    assert!(
        subjects_of_kind(&got, "webhook_received").is_empty(),
        "a receipt was served: {got:?}"
    );
    assert_eq!(
        subjects_of_kind(&got, "resource_created"),
        vec![r.uuid().to_string()],
        "the change after it is served"
    );
    let named: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT context_changes_after($1, $2, '0'::xid8, 0, 1000, ARRAY['webhook_received'])",
    )
    .bind(owner.uuid())
    .bind(home.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let named = named.expect("readable");
    assert!(
        entries(&named).is_empty(),
        "naming the kind serves it: {named}"
    );
}
