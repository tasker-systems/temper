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
use temper_core::types::ids::EntityId;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

/// The entry keys R4 allows. Anything else in an entry is a disclosure.
const ENTRY_KEYS: &[&str] = &[
    "position",
    "kind",
    "subject",
    "actor",
    "occurred_at",
    "act",
    "key",
    "redacted",
];

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
/// resource's meta arrives as a `property_set` entry naming the resource, the actor and the key.
///
/// FAILS IF: a meta-only write produces no entry, or the entry omits its subject, actor or key.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_meta_only_write_is_an_entry_naming_its_key(pool: PgPool) {
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
    assert_eq!(e["key"], "temper-stage");
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

/// `an-entry-never-carries-content`. Tokens planted in a title, a meta value and a body appear
/// nowhere in any answer, and every entry's keys lie within R4's allowlist.
///
/// FAILS IF: any payload value (title, value, origin, blocks) is copied into an entry, or an entry
/// grows a key outside the allowlist.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn no_entry_carries_content(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let props = vec![("open-key".to_string(), serde_json::json!("VALUETOKENzq"))];
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
    set(&pool, emitter, r, "other", "VALUETOKENzq").await;

    let (got, _) = drain(&pool, owner, home, (BEGINNING.0.into(), BEGINNING.1), 1000).await;
    assert!(!got.is_empty());
    let text = serde_json::to_string(&got).unwrap();
    for token in ["TITLETOKENzq", "VALUETOKENzq", "BODYTOKENzq"] {
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

/// `the-feed-never-discloses-what-erasure-removed`. Before erasure the feed names an open meta key.
/// After erasure the same events are served with `"key": null, "redacted": true`, and the key's text
/// appears nowhere.
///
/// FAILS IF: the key is read from anywhere erasure does not rewrite (`kb_properties`), or a redaction
/// row is ignored.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn erasure_removes_key_names_from_the_feed(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let props = vec![("secretkeyzq".to_string(), serde_json::json!("v"))];
    let r = create(&pool, owner, emitter, home, "to-erase", "body", &props).await;

    let (before, _) = drain(&pool, owner, home, (BEGINNING.0.into(), BEGINNING.1), 1000).await;
    assert!(
        serde_json::to_string(&before)
            .unwrap()
            .contains("secretkeyzq"),
        "before erasure the key is named, so the witness below can see it go"
    );

    let (_, operator) = system_actor(&pool).await;
    sqlx::query_scalar::<_, serde_json::Value>("SELECT resource_erasure_execute($1,$2,$3,$4)")
        .bind(r.uuid())
        .bind(operator)
        .bind(operator)
        .bind(Uuid::now_v7())
        .fetch_one(&pool)
        .await
        .expect("the erasure completes");

    let (after, _) = drain(&pool, owner, home, (BEGINNING.0.into(), BEGINNING.1), 1000).await;
    let text = serde_json::to_string(&after).unwrap();
    assert!(
        !text.contains("secretkeyzq"),
        "an erased key name came back: {text}"
    );
    let redacted: Vec<_> = after
        .iter()
        .filter(|e| e["kind"] == "property_set" && e["subject"]["id"] == r.uuid().to_string())
        .filter(|e| e["redacted"] == true)
        .collect();
    assert!(
        !redacted.is_empty(),
        "the erased key is served as redacted: {after:?}"
    );
    assert!(redacted
        .iter()
        .all(|e| e["key"].is_null() || e.get("key").is_none()));
}

/// R5 for rows written before cutover, and for rows that would lie about their order. A row with no
/// recorded order is refused outside replay, and admitted under `temper.replaying`, where replay
/// restores legacy rows. The feed never serves such a row. A row naming a transaction id other than
/// its own is refused.
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
             SELECT id, $1, 'kb_contexts', $2, jsonb_build_object('resource_id', gen_random_uuid()), \
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
