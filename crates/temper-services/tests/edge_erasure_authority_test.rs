#![cfg(feature = "test-db")]
//! Witnesses for the `edge` erasure authority (20261202000020): the `edge_erased` record, its
//! redaction projector `_project_edge_erased_redactions`, and the ledger exception's arms for it
//! (`kb_events_append_only`, `kb_events_redaction_in_trail`, `_erasure_sentinel_admits`) and
//! `sensitivity.place_closure`'s.
//!
//! An `edge` row admits a rewrite when its record's subject edge is folded and homed in a context
//! of the estate the record names (`_erasure_estate_contexts(estate_of)`), the rewritten event is
//! on that edge's own trail (`_edge_erasure_trail_scope`), and the new value is the sentinel
//! `erasure` would write for the path's class. Each case forges what the act will do in one
//! transaction: fold the edge (`relationship_folded` appended and projected), append the
//! `edge_erased` record, project its rows, UPDATE `kb_events`, then tombstone the estate's owner, as
//! the person act does after its edge step. Each refusal is paired with an admitted twin that
//! differs only in the condition under test, and is asserted by the trigger's own message, so a
//! refusal for any other reason fails the pair.
//!
//! The owner is checked at commit (`edge_erasure_requires_an_erased_owner`, 20261202000040): a
//! forge naming a live `estate_of` lands every statement and is refused by COMMIT. The cases that
//! roll back never reach it.

mod edge_erasure;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_substrate::events::EventContext;
use temper_substrate::ids::EdgeId;

use edge_erasure::{asserting_event, edge, fold, resource, tombstone, world, World};

const LABEL: &str = "jane roe told us";
const SENTINEL: &str = "erased-label-1";

/// Two edges in one world's `@me` context, each with a free-text label, and their assertions.
struct Edges {
    w: World,
    e: EdgeId,
    asserted: Uuid,
    other_asserted: Uuid,
}

async fn edges(pool: &PgPool) -> Edges {
    let w = world(pool).await;
    let a = resource(pool, &w, "alpha").await;
    let b = resource(pool, &w, "beta").await;
    let c = resource(pool, &w, "gamma").await;
    let e = edge(pool, &w, a, b, LABEL, EventContext::default()).await;
    let other = edge(
        pool,
        &w,
        b,
        c,
        "another free label",
        EventContext::default(),
    )
    .await;
    let asserted = asserting_event(pool, e).await;
    let other_asserted = asserting_event(pool, other).await;
    Edges {
        w,
        e,
        asserted,
        other_asserted,
    }
}

/// One forged act: the record it appends, and what its transaction does around it.
#[derive(Debug, Clone)]
struct Forged {
    /// The record's subject edge.
    subject: EdgeId,
    /// The record's `estate_of`.
    estate_of: Uuid,
    /// Fold the subject first, as the act does for a live edge.
    fold: bool,
    /// The event the record names (path `label`), and the one the UPDATE rewrites.
    target: Uuid,
    /// The value the UPDATE writes at `label`.
    value: String,
    /// Project the record's rows with `temper.replaying` on, so the projector's trail check (the
    /// earlier of two) is skipped and the verifier's is the one that answers.
    project_as_replay: bool,
    /// Tombstone `estate_of` after the UPDATE, as the person act does after its edge step.
    tombstone: bool,
}

/// Run `act` on one connection. The first error stops it.
async fn forge(conn: &mut sqlx::PgConnection, w: &World, act: &Forged) -> Result<(), sqlx::Error> {
    if act.fold {
        let fold = json!({ "edge_id": act.subject.uuid(), "reason": "erased" });
        let ev: Uuid = sqlx::query_scalar(
            "SELECT _event_append('relationship_folded', $1, 'kb_contexts', $2, $3)",
        )
        .bind(w.emitter)
        .bind(w.home.uuid())
        .bind(&fold)
        .fetch_one(&mut *conn)
        .await?;
        sqlx::query("SELECT _project_relationship_folded($1, $2)")
            .bind(ev)
            .bind(&fold)
            .execute(&mut *conn)
            .await?;
    }
    let payload = json!({
        "subject_table": "kb_edges",
        "subject_id": act.subject.uuid(),
        "estate_of": act.estate_of,
        "folded": act.fold,
        "redacted_fields": [{ "event": act.target, "paths": ["label"] }],
    });
    let record: Uuid =
        sqlx::query_scalar("SELECT _event_append('edge_erased', $1, NULL, NULL, $2)")
            .bind(w.emitter)
            .bind(&payload)
            .fetch_one(&mut *conn)
            .await?;
    if act.project_as_replay {
        sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
            .execute(&mut *conn)
            .await?;
        temper_substrate::test_schema::mark_replay_target(conn).await;
    }
    sqlx::query("SELECT _project_edge_erased_redactions($1, $2)")
        .bind(record)
        .bind(&payload)
        .execute(&mut *conn)
        .await?;
    if act.project_as_replay {
        sqlx::query("SELECT set_config('temper.replaying', 'off', true)")
            .execute(&mut *conn)
            .await?;
    }
    sqlx::query(
        "UPDATE kb_events SET payload = jsonb_set(payload, '{label}', to_jsonb($2::text)) \
          WHERE id = $1",
    )
    .bind(act.target)
    .bind(&act.value)
    .execute(&mut *conn)
    .await?;
    if act.tombstone {
        tombstone(&mut *conn, act.estate_of).await?;
    }
    Ok(())
}

/// [`forge`] in its own transaction: committed when `commit` and it succeeded, otherwise rolled
/// back. Returns the error text of a refusal, a statement's or the commit's, `None` when every
/// step landed.
async fn attempt(pool: &PgPool, w: &World, act: &Forged, commit: bool) -> Option<String> {
    let mut tx = pool.begin().await.unwrap();
    match forge(&mut tx, w, act).await {
        Err(e) => {
            tx.rollback().await.unwrap();
            Some(e.to_string())
        }
        Ok(()) if commit => tx.commit().await.err().map(|e| e.to_string()),
        Ok(()) => {
            tx.rollback().await.unwrap();
            None
        }
    }
}

fn append_only(what: &str, err: Option<String>) {
    let err = err.unwrap_or_else(|| panic!("{what} was admitted"));
    assert!(
        err.contains("event ledger is append-only"),
        "{what}: refused with the trigger's own message; got {err}"
    );
}

fn admitted(what: &str, err: Option<String>) {
    assert!(err.is_none(), "{what} was refused: {err:?}");
}

/// The admitted act: the folded edge's own assertion, its label at the sentinel, under the estate
/// that homes it, whose owner is tombstoned before commit.
fn lawful(x: &Edges) -> Forged {
    Forged {
        subject: x.e,
        estate_of: x.w.profile,
        fold: true,
        target: x.asserted,
        value: SENTINEL.into(),
        project_as_replay: false,
        tombstone: true,
    }
}

/// The error text names the estate's owner as not erased: the commit-time check's own message.
fn owner_not_erased(what: &str, owner: Uuid, err: Option<String>) {
    let err = err.unwrap_or_else(|| panic!("{what} was admitted"));
    assert!(
        err.contains(&format!(
            "edge erasure: the estate's owner {owner} is not erased"
        )),
        "{what}: refused at commit for its owner; got {err}"
    );
}

async fn label_of(pool: &PgPool, event: Uuid) -> Value {
    sqlx::query_scalar("SELECT payload -> 'label' FROM kb_events WHERE id = $1")
        .bind(event)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The act commits with its estate's owner tombstoned; the same forge with the owner live fails at
/// commit (`a_forged_edge_rewrite_naming_a_live_owner_fails_at_commit_not_at_insert`).
///
/// FAILS IF `edge_erased` authorises nothing: no event type, no projector, no `edge` authority in
/// the CHECK, no `edge` arm in the verifier's kind map or subject check, or a sentinel check that
/// refuses `edge`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_record_admits_its_trails_rewrite(pool: PgPool) {
    let x = edges(&pool).await;
    admitted(
        "the folded edge's own label at its sentinel",
        attempt(&pool, &x.w, &lawful(&x), true).await,
    );
    assert_eq!(label_of(&pool, x.asserted).await, json!(SENTINEL));
    let authority: String = sqlx::query_scalar(
        "SELECT authority FROM kb_event_field_redactions WHERE event_id = $1 AND path = 'label'",
    )
    .bind(x.asserted)
    .fetch_one(&pool)
    .await
    .expect("the record projected its row");
    assert_eq!(authority, "edge");
}

/// FAILS IF the subject check admits an edge that is not folded.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_record_refuses_a_live_edge(pool: PgPool) {
    let x = edges(&pool).await;
    admitted(
        "the folded twin",
        attempt(&pool, &x.w, &lawful(&x), false).await,
    );
    let live = Forged {
        fold: false,
        ..lawful(&x)
    };
    append_only(
        "a record naming a live edge",
        attempt(&pool, &x.w, &live, false).await,
    );
    assert_eq!(label_of(&pool, x.asserted).await, json!(LABEL));
}

/// FAILS IF the subject check admits an edge whose home is not in the estate the record names.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_record_refuses_an_edge_homed_outside_the_estate(pool: PgPool) {
    let x = edges(&pool).await;
    let stranger = world(&pool).await;
    admitted(
        "the twin naming the estate that homes the edge",
        attempt(&pool, &x.w, &lawful(&x), false).await,
    );
    let elsewhere = Forged {
        estate_of: stranger.profile,
        ..lawful(&x)
    };
    append_only(
        "a record naming an estate that does not home the edge",
        attempt(&pool, &x.w, &elsewhere, false).await,
    );
    assert_eq!(label_of(&pool, x.asserted).await, json!(LABEL));
}

/// The edge is re-homed by SQL to `kb_cogmaps` inside the probe's transaction, its
/// `home_anchor_id` kept at the estate's context, so only the home-table conjunct differs from
/// the admitted twin.
///
/// FAILS IF the subject check admits an edge whose home is not a context, because its anchor id
/// matches one of the estate's contexts.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_record_refuses_an_edge_not_homed_in_a_context(pool: PgPool) {
    let x = edges(&pool).await;
    admitted(
        "the twin whose edge is homed in the estate's context",
        attempt(&pool, &x.w, &lawful(&x), false).await,
    );
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE kb_edges SET home_anchor_table = 'kb_cogmaps' WHERE id = $1")
        .bind(x.e.uuid())
        .execute(&mut *tx)
        .await
        .expect("re-home the edge's table in the probe's transaction");
    let out = forge(&mut tx, &x.w, &lawful(&x)).await;
    tx.rollback().await.unwrap();
    append_only(
        "a record naming an edge homed on a cogmap with the context's id",
        out.err().map(|e| e.to_string()),
    );
    assert_eq!(label_of(&pool, x.asserted).await, json!(LABEL));
}

/// The record names another edge's assertion. The projector refuses it outside replay; projected
/// as replay projects it, the verifier refuses the rewrite.
///
/// FAILS IF either trail check (the projector's, or `kb_events_redaction_in_trail`'s `edge` loop)
/// admits an event off the subject edge's own trail.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_record_refuses_an_event_outside_the_edge_trail(pool: PgPool) {
    let x = edges(&pool).await;
    admitted(
        "the twin naming the subject's own assertion, projected as replay",
        attempt(
            &pool,
            &x.w,
            &Forged {
                project_as_replay: true,
                ..lawful(&x)
            },
            false,
        )
        .await,
    );
    let off_trail = Forged {
        target: x.other_asserted,
        ..lawful(&x)
    };
    let err = attempt(&pool, &x.w, &off_trail, false)
        .await
        .expect("the projector refuses an event off the trail");
    assert!(
        err.contains("which is not in its subject's trail"),
        "the projector's own trail message; got {err}"
    );
    append_only(
        "a rewrite of an event off the subject's trail",
        attempt(
            &pool,
            &x.w,
            &Forged {
                project_as_replay: true,
                ..off_trail
            },
            false,
        )
        .await,
    );
    assert_eq!(
        label_of(&pool, x.other_asserted).await,
        json!("another free label")
    );
}

/// FAILS IF the sentinel check admits a value that is not the label class's sentinel.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_record_refuses_a_non_sentinel_value(pool: PgPool) {
    let x = edges(&pool).await;
    admitted(
        "the twin writing the sentinel",
        attempt(&pool, &x.w, &lawful(&x), false).await,
    );
    let other_value = Forged {
        value: "something".into(),
        ..lawful(&x)
    };
    append_only(
        "a rewrite to a value that is no sentinel",
        attempt(&pool, &x.w, &other_value, false).await,
    );
    assert_eq!(label_of(&pool, x.asserted).await, json!(LABEL));
}

/// The person act's classes are its own: `_erasure_sentinel_admits` refuses each of them under
/// `edge`, though each new value is the exact sentinel `principal` admits. An edge's trail holds no
/// event of these classes and the projector's allowlist names none, so no rewrite can reach this
/// conjunct alone; the probe reads the function itself, with `edge` admitting the edge classes
/// beside it.
///
/// FAILS IF `edge` reaches a context name or slug, a principal reason or a Slack id, or if `edge`
/// is not an authority the function knows (the edge-class controls).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_record_refuses_a_context_class(pool: PgPool) {
    let ev = Uuid::now_v7();
    let payload = json!({ "context_id": Uuid::now_v7(), "to_name": "jane's notes" });
    let admits = |class: &'static str, old: Value, new: Value, authority: &'static str| {
        let pool = pool.clone();
        let payload = payload.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT _erasure_sentinel_admits($1, $2, $3, '{to_name}'::text[], $4, $5, $6)",
            )
            .bind(class)
            .bind(ev)
            .bind(&payload)
            .bind(&old)
            .bind(&new)
            .bind(authority)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    for class in [
        "context-name",
        "context-slug",
        "principal-reason",
        "slack-principal",
    ] {
        let exact: Value = sqlx::query_scalar("SELECT _erasure_sentinel_exact($1, $2, $3)")
            .bind(class)
            .bind(ev)
            .bind(&payload)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(
            admits(class, json!("jane's notes"), exact.clone(), "principal").await,
            "{class}: the person act admits its exact sentinel"
        );
        assert!(
            !admits(class, json!("jane's notes"), exact, "edge").await,
            "{class}: the edge authority refuses the person act's class"
        );
    }
    assert!(
        admits("edge-label", json!(LABEL), json!(SENTINEL), "edge").await,
        "the edge authority admits an edge label's sentinel, as erasure does"
    );
    assert!(
        admits(
            "facet-value",
            json!({ "node_label": "jane roe" }),
            json!({ "erased-facet-1": "erased" }),
            "edge"
        )
        .await,
        "the edge authority admits erasure's facet marks"
    );
}

/// FAILS IF `sensitivity.place_closure` has no `edge` arm: a ledger finding on the rewritten label
/// stays open after the act.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn place_closure_closes_an_edge_redaction(pool: PgPool) {
    let x = edges(&pool).await;
    let closure = |pool: PgPool, event: Uuid| async move {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT sensitivity.place_closure('kb_events.payload', $1, repeat('0', 64), '/label')",
        )
        .bind(event)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert_eq!(
        closure(pool.clone(), x.asserted).await,
        None,
        "open before the act"
    );
    admitted("the act", attempt(&pool, &x.w, &lawful(&x), true).await);
    assert_eq!(
        closure(pool.clone(), x.asserted).await.as_deref(),
        Some("sentinel")
    );
    assert_eq!(
        closure(pool.clone(), x.other_asserted).await,
        None,
        "the other edge's label, which no record names, stays open"
    );
}

/// The lawful forge with the estate's owner left live. Every statement lands, the fold, the record,
/// its rows and the rewrite, and COMMIT is refused: the owner is checked at commit, not at insert.
///
/// FAILS IF an `edge` rewrite commits for an owner who is not tombstoned (no
/// `edge_erasure_requires_an_erased_owner` trigger on either table), or if the check runs at a
/// statement rather than at commit.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_forged_edge_rewrite_naming_a_live_owner_fails_at_commit_not_at_insert(pool: PgPool) {
    let x = edges(&pool).await;
    let live_owner = Forged {
        tombstone: false,
        ..lawful(&x)
    };
    admitted(
        "every statement of the forge, rolled back before commit",
        attempt(&pool, &x.w, &live_owner, false).await,
    );
    owner_not_erased(
        "the forge's commit",
        x.w.profile,
        attempt(&pool, &x.w, &live_owner, true).await,
    );
    assert_eq!(label_of(&pool, x.asserted).await, json!(LABEL));
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_event_field_redactions WHERE event_id = $1")
            .bind(x.asserted)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 0, "no redaction row persists");
}

/// The twin of the live-owner forge: the same rewrite, the owner tombstoned before commit.
///
/// FAILS IF the commit-time check refuses a tombstoned owner.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_same_rewrite_commits_once_the_owner_is_tombstoned(pool: PgPool) {
    let x = edges(&pool).await;
    admitted(
        "the forge with its owner tombstoned, committed",
        attempt(&pool, &x.w, &lawful(&x), true).await,
    );
    assert_eq!(label_of(&pool, x.asserted).await, json!(SENTINEL));
}

/// An `edge` row inserted by hand, in a transaction that appends no record, under an `edge_erased`
/// record a replaying transaction committed for a live owner. Only the kb_event_field_redactions
/// trigger sees it, since no event is inserted. The record's edge is folded first, so the twin,
/// which tombstones the owner in the same transaction, meets every other conjunct of the
/// commit-time check and commits.
///
/// FAILS IF `trg_edge_redaction_requires_an_erased_owner` is missing or does not read the row's
/// record: the row commits under a record whose owner is live.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_row_under_a_record_naming_a_live_owner_fails_at_commit(pool: PgPool) {
    let x = edges(&pool).await;
    fold(&pool, &x.w, x.e).await;
    let payload = json!({
        "subject_table": "kb_edges",
        "subject_id": x.e.uuid(),
        "estate_of": x.w.profile,
        "folded": false,
        "redacted_fields": [{ "event": x.asserted, "paths": ["label"] }],
    });
    // The setting alone, in a database without the replay-target marker, keeps the commit-time
    // check (edge_erasure_requires_an_erased_owner) up.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT _event_append('edge_erased', $1, NULL, NULL, $2)")
        .bind(x.w.emitter)
        .bind(&payload)
        .execute(&mut *tx)
        .await
        .unwrap();
    owner_not_erased(
        "a record under the setting alone",
        x.w.profile,
        tx.commit().await.err().map(|e| e.to_string()),
    );

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    temper_substrate::test_schema::mark_replay_target(&mut tx).await;
    let record: Uuid =
        sqlx::query_scalar("SELECT _event_append('edge_erased', $1, NULL, NULL, $2)")
            .bind(x.w.emitter)
            .bind(&payload)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    tx.commit()
        .await
        .expect("a replaying transaction commits its record for a live owner");

    let row = |tombstoned: bool| {
        let pool = pool.clone();
        let owner = x.w.profile;
        let event = x.asserted;
        async move {
            let mut tx = pool.begin().await.unwrap();
            sqlx::query(
                "INSERT INTO kb_event_field_redactions (event_id, path, redacted_by, authority) \
                 VALUES ($1, 'label', $2, 'edge')",
            )
            .bind(event)
            .bind(record)
            .execute(&mut *tx)
            .await
            .expect("the row's insert lands");
            if tombstoned {
                tombstone(&mut tx, owner).await.unwrap();
            }
            tx.commit().await.err().map(|e| e.to_string())
        }
    };
    owner_not_erased("the hand-inserted row", x.w.profile, row(false).await);
    admitted("the twin with the owner tombstoned", row(true).await);
}

/// The commit-time check's message for a record whose edge is not folded, or not homed in the estate
/// it names.
fn not_an_estate_edge(what: &str, e: EdgeId, owner: Uuid, err: Option<String>) {
    let err = err.unwrap_or_else(|| panic!("{what} was admitted"));
    assert!(
        err.contains(&format!(
            "edge erasure: edge {} is not a folded edge homed in the estate of {owner}",
            e.uuid()
        )),
        "{what}: refused at commit for its edge; got {err}"
    );
}

/// A forged `edge_erased` record alone, with no redaction row and no rewrite, its `estate_of` an
/// owner already tombstoned. Committed, it would refuse every later write to the edge it names
/// (`_edge_is_erased`), over text no act redacted. At commit its edge must be folded and homed in
/// that estate: the twin naming a folded edge of the estate commits; a record naming the estate's
/// live edge, and one naming a folded edge homed in another person's estate, are each refused, and
/// neither edge is then erased.
///
/// FAILS IF the commit-time check reads only the owner's tombstone: either forge commits. Each
/// refusal isolates one conjunct: the live edge is homed in the estate, and the other estate's edge
/// is folded.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_forged_record_for_a_tombstoned_owner_needs_a_folded_edge_of_its_estate(pool: PgPool) {
    let w = world(&pool).await;
    let stranger = world(&pool).await;
    let a = resource(&pool, &w, "alpha").await;
    let b = resource(&pool, &w, "beta").await;
    let c = resource(&pool, &w, "gamma").await;
    let folded = edge(&pool, &w, a, b, LABEL, EventContext::default()).await;
    let live = edge(
        &pool,
        &w,
        b,
        c,
        "another free label",
        EventContext::default(),
    )
    .await;
    let d = resource(&pool, &stranger, "delta").await;
    let f = resource(&pool, &stranger, "epsilon").await;
    let elsewhere = edge(
        &pool,
        &stranger,
        d,
        f,
        "a stranger's label",
        EventContext::default(),
    )
    .await;
    fold(&pool, &w, folded).await;
    fold(&pool, &stranger, elsewhere).await;
    let mut conn = pool.acquire().await.unwrap();
    tombstone(&mut conn, w.profile)
        .await
        .expect("the owner is tombstoned before the forge");
    drop(conn);

    let record = |subject: EdgeId| {
        let pool = pool.clone();
        let emitter = w.emitter;
        let owner = w.profile;
        async move {
            let mut tx = pool.begin().await.unwrap();
            sqlx::query("SELECT _event_append('edge_erased', $1, NULL, NULL, $2)")
                .bind(emitter)
                .bind(json!({
                    "subject_table": "kb_edges",
                    "subject_id": subject.uuid(),
                    "estate_of": owner,
                    "folded": false,
                    "redacted_fields": [],
                }))
                .execute(&mut *tx)
                .await
                .expect("the record's append lands");
            tx.commit().await.err().map(|e| e.to_string())
        }
    };
    not_an_estate_edge(
        "a record naming the estate's live edge",
        live,
        w.profile,
        record(live).await,
    );
    not_an_estate_edge(
        "a record naming a folded edge of another estate",
        elsewhere,
        w.profile,
        record(elsewhere).await,
    );
    admitted(
        "the twin naming a folded edge of the estate",
        record(folded).await,
    );
    for (e, what) in [
        (live, "the live edge"),
        (elsewhere, "the other estate's edge"),
    ] {
        let erased: bool = sqlx::query_scalar("SELECT _edge_is_erased($1)")
            .bind(e.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!erased, "{what} is not erased");
    }
}

/// A set tombstone never changes (`kb_profiles_tombstone_is_irreversible`): an UPDATE to NULL and
/// one to another time are each refused, and the person act's own form, `COALESCE(tombstoned_at,
/// …)`, re-run on the tombstoned profile, passes and keeps the value. Without the trigger a
/// transaction could tombstone the owner, run the owner check early (`SET CONSTRAINTS ALL
/// IMMEDIATE`), clear the tombstone and commit an edge erasure of a live owner.
///
/// FAILS IF a set tombstoned_at can be cleared or moved.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_profiles_tombstone_is_irreversible(pool: PgPool) {
    let w = world(&pool).await;
    let mut conn = pool.acquire().await.unwrap();
    tombstone(&mut conn, w.profile).await.expect("tombstone");
    drop(conn);
    let at = |pool: PgPool| async move {
        sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT tombstoned_at FROM kb_profiles WHERE id = $1",
        )
        .bind(w.profile)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let set = at(pool.clone()).await;
    assert!(set.is_some(), "setup: the profile is tombstoned");

    for (what, sql) in [
        (
            "clearing the tombstone",
            "UPDATE kb_profiles SET tombstoned_at = NULL WHERE id = $1",
        ),
        (
            "moving the tombstone",
            "UPDATE kb_profiles SET tombstoned_at = tombstoned_at + interval '1 second' WHERE id = $1",
        ),
    ] {
        let err = sqlx::query(sql)
            .bind(w.profile)
            .execute(&pool)
            .await
            .err()
            .unwrap_or_else(|| panic!("{what} was admitted"))
            .to_string();
        assert!(
            err.contains(&format!(
                "profile {} is tombstoned; its tombstone is irreversible",
                w.profile
            )),
            "{what}: refused by the trigger's own message; got {err}"
        );
    }
    sqlx::query(
        "UPDATE kb_profiles SET tombstoned_at = COALESCE(tombstoned_at, now()) WHERE id = $1",
    )
    .bind(w.profile)
    .execute(&pool)
    .await
    .expect("the person act's own form re-runs on a tombstoned profile");
    assert_eq!(at(pool.clone()).await, set, "the tombstone is unchanged");
}

/// Call `_edge_erasure_apply_redaction(e, event)` on `conn`: the refusal's text, `None` when it ran.
async fn apply_redaction(
    conn: &mut sqlx::PgConnection,
    e: EdgeId,
    event: Option<Uuid>,
) -> Option<String> {
    sqlx::query("SELECT _edge_erasure_apply_redaction($1, $2)")
        .bind(e.uuid())
        .bind(event)
        .execute(conn)
        .await
        .err()
        .map(|err| err.to_string())
}

/// Append an `edge_erased` record naming `e` on `conn`, its estate the world's.
async fn edge_record(conn: &mut sqlx::PgConnection, w: &World, e: EdgeId) -> Uuid {
    sqlx::query_scalar("SELECT _event_append('edge_erased', $1, NULL, NULL, $2)")
        .bind(w.emitter)
        .bind(json!({
            "subject_table": "kb_edges",
            "subject_id": e.uuid(),
            "estate_of": w.profile,
            "folded": false,
            "redacted_fields": [],
        }))
        .fetch_one(conn)
        .await
        .expect("append the record")
}

/// The edge erasure body runs outside replay only under an `edge_erased` record of its own edge
/// appended in the same transaction. It is refused with no record (NULL), with another edge's record
/// of this transaction, and with its own edge's record from an earlier transaction (committed under
/// `temper.replaying`, as the owner check stands down there); the twin, its own edge's record of
/// this transaction, runs and nulls the label. Each probe rolls back.
///
/// FAILS IF the body erases an edge's projection without that record: each refusal isolates one
/// conjunct (a record, naming this edge, of this transaction).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_edge_erasure_body_runs_only_under_its_records_transaction(pool: PgPool) {
    let x = edges(&pool).await;
    let w = &x.w;
    let other = {
        let b = resource(&pool, w, "delta").await;
        let c = resource(&pool, w, "epsilon").await;
        edge(&pool, w, b, c, "a third label", EventContext::default()).await
    };
    // Under the setting alone, in a database without the replay-target marker, the body still
    // refuses an edge with no record of this transaction; once the database is marked, it runs.
    // This unmarked probe must stay first: the transaction that builds `earlier` below commits the
    // marker, so every later probe runs in a marked database.
    {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
            .execute(&mut *tx)
            .await
            .unwrap();
        let unmarked = {
            let mut sp = sqlx::Connection::begin(&mut *tx).await.unwrap();
            let err = apply_redaction(&mut sp, x.e, None).await;
            sp.rollback().await.unwrap();
            err
        };
        assert!(
            unmarked.as_deref().is_some_and(|e| e.contains(&format!(
                "_edge_erasure_apply_redaction: edge {} has no edge_erased record of this transaction",
                x.e.uuid()
            ))),
            "unmarked, the setting alone keeps the record check: {unmarked:?}"
        );
        temper_substrate::test_schema::mark_replay_target(&mut tx).await;
        assert_eq!(
            apply_redaction(&mut tx, x.e, None).await,
            None,
            "control: in a marked target the body runs as the replay walk's does"
        );
        tx.rollback().await.unwrap();
    }
    let earlier = {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
            .execute(&mut *tx)
            .await
            .unwrap();
        temper_substrate::test_schema::mark_replay_target(&mut tx).await;
        let id = edge_record(&mut tx, w, x.e).await;
        tx.commit()
            .await
            .expect("a replaying transaction commits the record");
        id
    };
    let refused = |what: &str, err: Option<String>| {
        let err = err.unwrap_or_else(|| panic!("{what} ran"));
        assert!(
            err.contains(&format!(
                "_edge_erasure_apply_redaction: edge {} has no edge_erased record of this transaction",
                x.e.uuid()
            )),
            "{what}: refused by the body's own message; got {err}"
        );
    };

    let mut tx = pool.begin().await.unwrap();
    refused("no record", apply_redaction(&mut tx, x.e, None).await);
    tx.rollback().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    let theirs = edge_record(&mut tx, w, other).await;
    refused(
        "another edge's record of this transaction",
        apply_redaction(&mut tx, x.e, Some(theirs)).await,
    );
    tx.rollback().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    refused(
        "its own edge's record from an earlier transaction",
        apply_redaction(&mut tx, x.e, Some(earlier)).await,
    );
    tx.rollback().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    let own = edge_record(&mut tx, w, x.e).await;
    assert_eq!(
        apply_redaction(&mut tx, x.e, Some(own)).await,
        None,
        "the twin: its own edge's record of this transaction"
    );
    let label: Option<String> = sqlx::query_scalar("SELECT label FROM kb_edges WHERE id = $1")
        .bind(x.e.uuid())
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(label, None, "the body ran: the label is NULL");
    tx.rollback().await.unwrap();
    let label: Option<String> = sqlx::query_scalar("SELECT label FROM kb_edges WHERE id = $1")
        .bind(x.e.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(label.as_deref(), Some(LABEL), "every probe rolled back");
}
