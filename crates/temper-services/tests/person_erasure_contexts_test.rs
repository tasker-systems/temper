#![cfg(feature = "test-db")]
//! Witnesses for the person act's evented custody closure (20261022100000; rulings R7 and R8 of the
//! 2026-10-10 erasure inventory, and the event-shape and ledger-copy rulings recorded on task
//! 01a125a1-d1a4-7509-867f-34900bd5a51c).
//!
//! Each world is built through the real write paths and erased through
//! `erasure_service::execute_erasure`. Before this migration the act retired its estate contexts
//! with an un-evented UPDATE, left their names and slugs in the projection and on the ledger, let
//! a personal-team co-admin restore one, and named only text hashes for what the subject wrote
//! outside the estate. Every witness below fails against that act.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_services::error::ApiError;
use temper_services::services::context_service;
use temper_services::services::erasure_service::execute_erasure;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{BlockId, ContextId, EntityId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity, Incorporation, ProvenanceSource};
use temper_substrate::replay;
use temper_substrate::writes::{self, CitationAuditParams, CreateParams};
use temper_workflow::operations::Surface;

/// A name that must end nowhere once its context is erased.
const NAME: &str = "Jane Roe divorce";
const SLUG: &str = "jane-roe-divorce";
const JOURNAL: &str = "Jane Roe journal";
const JOURNAL_SLUG: &str = "jane-roe-journal";

struct Person {
    profile: Uuid,
    emitter: Uuid,
    /// An `@me` context.
    me: ContextId,
    /// A context of the person's personal team (`kb_teams.personal_of`).
    personal_team: ContextId,
}

/// A profile (the trigger makes its personal team), its `@web` emitter, an `@me` context and a
/// context owned by its personal team. The handle is the full id, so same-millisecond UUIDv7
/// handles cannot collide.
async fn person(pool: &PgPool) -> Person {
    let profile = Uuid::now_v7();
    let handle = format!("user-{profile}");
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
                 VALUES ($1, $2, $2, $3, '{}'::jsonb)",
    )
    .bind(profile)
    .bind(&handle)
    .bind(format!("{handle}@x.test"))
    .execute(pool)
    .await
    .expect("seed profile");
    let emitter = entity(pool, profile, &format!("{handle}@web")).await;
    let me: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
              VALUES ('kb_profiles', $1, 'notes', 'notes') RETURNING id",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .expect("seed @me context");
    let personal_team: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
              SELECT 'kb_teams', t.id, 'journal', 'journal' FROM kb_teams t \
               WHERE t.personal_of = $1 RETURNING id",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .expect("seed a personal-team context (the trigger made the team)");
    Person {
        profile,
        emitter,
        me: ContextId::from(me),
        personal_team: ContextId::from(personal_team),
    }
}

async fn entity(pool: &PgPool, profile: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id")
        .bind(profile)
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("seed emitter")
}

/// A context of a team that is NOT the person's personal team: outside the estate.
async fn other_team_context(pool: &PgPool) -> ContextId {
    let id: Uuid = sqlx::query_scalar(
        "WITH t AS (INSERT INTO kb_teams (slug, name) VALUES ('team-' || gen_random_uuid(), 'T') \
                    RETURNING id) \
         INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
              SELECT 'kb_teams', t.id, 'shared', 'shared' FROM t RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("seed another team's context");
    ContextId::from(id)
}

async fn operator(pool: &PgPool) -> (Person, temper_services::auth::SystemAdmin) {
    let op = person(pool).await;
    temper_services::test_support::approved_admin(pool, op.profile).await;
    let admin = temper_services::test_support::system_admin_proof_for(pool, op.profile).await;
    (op, admin)
}

async fn resource(
    pool: &PgPool,
    p: &Person,
    home: ContextId,
    title: &str,
    source: Option<ProvenanceSource>,
) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &format!("test://{}", Uuid::now_v7()),
            body: "a body",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner: ProfileId::from(p.profile),
            originator: ProfileId::from(p.profile),
            emitter: EntityId::from(p.emitter),
            properties: &[],
            chunks: None,
            sources: source
                .map(|source| vec![Incorporation { source, seq: 1 }])
                .unwrap_or_default(),
        },
        EventContext::default(),
    )
    .await
    .expect("create through the real path")
}

async fn erase(pool: &PgPool, subject: Uuid) {
    let (_, admin) = operator(pool).await;
    execute_erasure(
        pool,
        &admin,
        ProfileId::from(subject),
        Uuid::now_v7(),
        Surface::ApiHttp,
    )
    .await
    .expect("the person act completes");
}

async fn rename(pool: &PgPool, c: ContextId, from: (&str, &str), to: (&str, &str), emitter: Uuid) {
    writes::rename_context_with(
        pool,
        c,
        from,
        to,
        EntityId::from(emitter),
        EventContext::default(),
    )
    .await
    .expect("rename through the real path");
}

async fn context_state(pool: &PgPool, c: ContextId) -> (String, String, bool) {
    sqlx::query_as("SELECT name, slug, is_active FROM kb_contexts WHERE id = $1")
        .bind(c.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Ledger rows (payload or metadata) whose text contains `needle`, case-insensitively.
async fn ledger_quoting(pool: &PgPool, needle: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events \
          WHERE payload::text ILIKE '%' || $1 || '%' OR metadata::text ILIKE '%' || $1 || '%'",
    )
    .bind(needle)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn completion_payload(pool: &PgPool, subject: Uuid) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'principal_erased' AND e.payload->>'subject_id' = $1::text",
    )
    .bind(subject)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// FAILS IF the custody closure is un-evented or leaves the context's identity: the ledger must
/// hold one `context_erased` per estate context, carrying only the sentinels, and each context must
/// end retired under the sentinel name and slug.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn each_estate_context_is_erased_by_its_own_event(pool: PgPool) {
    let p = person(&pool).await;

    erase(&pool, p.profile).await;

    for c in [p.me, p.personal_team] {
        let payloads: Vec<serde_json::Value> = sqlx::query_scalar(
            "SELECT e.payload FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
              WHERE t.name = 'context_erased' AND e.producing_anchor_table = 'kb_contexts' \
                AND e.producing_anchor_id = $1",
        )
        .bind(c.uuid())
        .fetch_all(&pool)
        .await
        .unwrap();
        let slug = format!("erased-{}", c.uuid());
        assert_eq!(
            payloads,
            vec![serde_json::json!({
                "context_id": c.uuid().to_string(),
                "to_name": "erased",
                "to_slug": slug,
            })],
            "one context_erased for {c:?}, carrying only the sentinels"
        );
        assert_eq!(
            context_state(&pool, c).await,
            ("erased".to_string(), slug, false),
            "the context is retired under the sentinels"
        );
    }
}

/// FAILS IF a context's earlier name or slug survives on the ledger: renames, a retirement and a
/// restoration of estate contexts all wrote the real text into event payloads, and the act must
/// rewrite every copy to the sentinels, under its own `principal` authority. A context of another
/// team, renamed to the same name, is outside the estate and keeps its events.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_estate_contexts_names_and_slugs_are_gone_from_the_ledger(pool: PgPool) {
    let p = person(&pool).await;
    rename(&pool, p.me, ("notes", "notes"), (NAME, SLUG), p.emitter).await;
    writes::retire_context_with(
        &pool,
        p.me,
        SLUG,
        &format!("{SLUG}-retired"),
        EntityId::from(p.emitter),
        EventContext::default(),
    )
    .await
    .unwrap();
    writes::restore_context_with(
        &pool,
        p.me,
        &format!("{SLUG}-retired"),
        SLUG,
        EntityId::from(p.emitter),
        EventContext::default(),
    )
    .await
    .unwrap();
    rename(
        &pool,
        p.personal_team,
        ("journal", "journal"),
        (JOURNAL, JOURNAL_SLUG),
        p.emitter,
    )
    .await;
    let outside = other_team_context(&pool).await;
    let stranger = person(&pool).await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
              SELECT owner_id, $2, 'owner' FROM kb_contexts WHERE id = $1",
    )
    .bind(outside.uuid())
    .bind(stranger.profile)
    .execute(&pool)
    .await
    .unwrap();
    rename(
        &pool,
        outside,
        ("shared", "shared"),
        ("Jane Roe club", "jane-roe-club"),
        stranger.emitter,
    )
    .await;
    for needle in [NAME, SLUG, JOURNAL, JOURNAL_SLUG] {
        assert!(
            ledger_quoting(&pool, needle).await > 0,
            "setup: the ledger holds {needle:?}"
        );
    }

    erase(&pool, p.profile).await;

    for needle in [NAME, SLUG, JOURNAL, JOURNAL_SLUG] {
        assert_eq!(
            ledger_quoting(&pool, needle).await,
            0,
            "no ledger payload quotes {needle:?} after the act"
        );
    }
    assert!(
        ledger_quoting(&pool, "jane-roe-club").await > 0,
        "a context outside the estate keeps its events"
    );
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_event_field_redactions WHERE authority = 'principal'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    // me: rename (4 paths), retire (2), restore (2); personal team: rename (4).
    assert_eq!(rows, 12, "one principal redaction row per rewritten path");
    let payload = completion_payload(&pool, p.profile).await;
    assert_eq!(
        payload["redacted_fields"].as_array().map(Vec::len),
        Some(4),
        "the record names the four rewritten events, by id and path"
    );
}

/// FAILS IF replay leans on the verbatim restore of `kb_contexts` for the closure: the walk
/// re-applies the estate context's own retirement and restoration, so without a `context_erased`
/// arm it would end the context ACTIVE. Replay must reproduce the live projections byte for byte,
/// and the context's erased state.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_reproduces_the_erased_contexts_from_the_walk(pool: PgPool) {
    let p = person(&pool).await;
    rename(&pool, p.me, ("notes", "notes"), (NAME, SLUG), p.emitter).await;
    writes::retire_context_with(
        &pool,
        p.me,
        SLUG,
        &format!("{SLUG}-retired"),
        EntityId::from(p.emitter),
        EventContext::default(),
    )
    .await
    .unwrap();
    writes::restore_context_with(
        &pool,
        p.me,
        &format!("{SLUG}-retired"),
        SLUG,
        EntityId::from(p.emitter),
        EventContext::default(),
    )
    .await
    .unwrap();
    resource(&pool, &p, p.me, "a page", None).await;
    resource(&pool, &p, p.personal_team, "a journal page", None).await;

    erase(&pool, p.profile).await;

    let live = (
        context_state(&pool, p.me).await,
        context_state(&pool, p.personal_team).await,
    );
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    reset_namespace(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();
    for ((table_a, x), (table_b, y)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(x, y, "projection table {table_a} diverged under replay");
    }
    assert_eq!(
        (
            context_state(&pool, p.me).await,
            context_state(&pool, p.personal_team).await,
        ),
        live,
        "the walk reproduces the erased contexts"
    );
    assert!(!live.0 .2, "the @me context ends retired");
}

/// FAILS IF an erased context can come back: a co-admin of the erased person's personal team, who
/// administers its contexts, and an instance admin are both refused, through the service (410)
/// and through `context_restore` itself (SQLSTATE TE001).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_erased_context_cannot_be_restored_by_anyone(pool: PgPool) {
    let p = person(&pool).await;
    let co_admin = person(&pool).await;
    temper_services::test_support::approve(&pool, co_admin.profile).await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
              SELECT t.id, $2, 'maintainer' FROM kb_teams t WHERE t.personal_of = $1",
    )
    .bind(p.profile)
    .bind(co_admin.profile)
    .execute(&pool)
    .await
    .unwrap();

    erase(&pool, p.profile).await;

    let co_admin_proof =
        temper_services::test_support::authenticated_profile_for(&pool, co_admin.profile).await;
    match context_service::restore(&pool, &co_admin_proof, p.personal_team.uuid()).await {
        Err(ApiError::Gone(_)) => {}
        other => panic!("the co-admin's restore is refused with 410, got {other:?}"),
    }
    let (op, _) = operator(&pool).await;
    let op_proof =
        temper_services::test_support::authenticated_profile_for(&pool, op.profile).await;
    for c in [p.me, p.personal_team] {
        match context_service::restore(&pool, &op_proof, c.uuid()).await {
            Err(ApiError::Gone(_)) => {}
            other => {
                panic!("the instance admin's restore of {c:?} is refused with 410, got {other:?}")
            }
        }
    }
    let err = writes::restore_context_with(
        &pool,
        p.personal_team,
        &format!("erased-{}", p.personal_team.uuid()),
        "journal",
        EntityId::from(co_admin.emitter),
        EventContext::default(),
    )
    .await
    .expect_err("context_restore refuses in its own transaction");
    let code = err
        .downcast_ref::<sqlx::Error>()
        .and_then(|e| e.as_database_error())
        .and_then(|d| d.code().map(|c| c.into_owned()));
    assert_eq!(
        code.as_deref(),
        Some("TE001"),
        "refused with TE001: {err:#}"
    );
    assert!(
        !context_state(&pool, p.personal_team).await.2,
        "the context stays retired"
    );
}

/// FAILS IF the record names text the subject wrote outside the estate other than by count: a
/// title, a property value, an edge label and a citation-audit reason the subject wrote in another
/// team's context stay (disposition iii), and the record carries one count per class and none of
/// the text.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_record_counts_text_left_outside_the_estate(pool: PgPool) {
    let p = person(&pool).await;
    let team = other_team_context(&pool).await;
    let b = resource(&pool, &p, team, "another team page", None).await;
    let a = resource(
        &pool,
        &p,
        team,
        "Jane Roe team page",
        Some(ProvenanceSource::Resource(b.uuid())),
    )
    .await;
    writes::set_property(
        &pool,
        a,
        "note",
        &serde_json::json!("jane roe property"),
        EntityId::from(p.emitter),
    )
    .await
    .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    fire(
        &mut conn,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(a),
            tgt: AnchorRef::resource(b),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("jane roe label"),
            weight: 1.0,
            home: EdgeHome::Context(team),
            emitter: EntityId::from(p.emitter),
        },
    )
    .await
    .expect("assert the edge");
    drop(conn);
    let block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 ORDER BY id LIMIT 1",
    )
    .bind(a.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    writes::record_citation_audit(
        &pool,
        CitationAuditParams {
            block: BlockId::from(block),
            source: ProvenanceSource::Resource(b.uuid()),
            value: 0.5,
            reason: Some("jane roe reason"),
            emitter: EntityId::from(p.emitter),
        },
    )
    .await
    .expect("record the audit");
    // A rename the subject made of the team's context, as one of its maintainers.
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
              SELECT owner_id, $2, 'maintainer' FROM kb_contexts WHERE id = $1",
    )
    .bind(team.uuid())
    .bind(p.profile)
    .execute(&pool)
    .await
    .unwrap();
    rename(
        &pool,
        team,
        ("shared", "shared"),
        ("Jane Roe club", "jane-roe-club"),
        p.emitter,
    )
    .await;
    // The estate holds text of the same classes; resource erasure rewrites it, so it is not counted.
    resource(&pool, &p, p.me, "Jane Roe private page", None).await;

    erase(&pool, p.profile).await;

    let payload = completion_payload(&pool, p.profile).await;
    let targets = payload["targets"].as_array().unwrap();
    for (class, n) in [
        ("titles", 2),
        ("property values", 1),
        ("edge labels", 1),
        ("citation-audit reasons", 1),
        ("context names", 1),
    ] {
        let target = format!("kb_events.payload ({class})");
        let row = targets
            .iter()
            .find(|t| t["target"] == serde_json::json!(target))
            .unwrap_or_else(|| panic!("the record counts {class}: {targets:#?}"));
        assert_eq!(
            row["outcome"],
            serde_json::json!(format!(
                "independent_obligation: authored by the subject outside the estate; {n} event(s) kept"
            )),
            "{class}"
        );
    }
    assert!(
        !payload.to_string().to_lowercase().contains("jane roe"),
        "the record carries no authored text"
    );
}

/// FAILS IF a context verb racing the act can undo it: a rename of an estate context that reaches
/// the row while the act holds it must wait, then find the context erased and be refused, so
/// neither the projection nor the ledger ends with the new name.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_rename_racing_the_act_waits_and_is_refused(pool: PgPool) {
    let p = person(&pool).await;
    let (op, _) = operator(&pool).await;
    // The act, uncommitted: it holds every estate context's row lock.
    let mut held = pool.begin().await.unwrap();
    let _: serde_json::Value =
        sqlx::query_scalar("SELECT principal_erasure_execute($1, $2, $3, $4)")
            .bind(p.profile)
            .bind(op.profile)
            .bind(op.emitter)
            .bind(Uuid::now_v7())
            .fetch_one(&mut *held)
            .await
            .expect("the act runs");
    let pool2 = pool.clone();
    let (me, emitter) = (p.me, p.emitter);
    let racer = tokio::spawn(async move {
        writes::rename_context_with(
            &pool2,
            me,
            ("notes", "notes"),
            (NAME, SLUG),
            EntityId::from(emitter),
            EventContext::default(),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert!(!racer.is_finished(), "the rename waits on the act's lock");
    held.commit().await.unwrap();
    let err = racer
        .await
        .unwrap()
        .expect_err("the rename finds the context erased");
    let code = err
        .downcast_ref::<sqlx::Error>()
        .and_then(|e| e.as_database_error())
        .and_then(|d| d.code().map(|c| c.into_owned()));
    assert_eq!(
        code.as_deref(),
        Some("P0002"),
        "refused as retired: {err:#}"
    );
    assert_eq!(
        context_state(&pool, p.me).await,
        (
            "erased".to_string(),
            format!("erased-{}", p.me.uuid()),
            false
        )
    );
    assert_eq!(ledger_quoting(&pool, NAME).await, 0);
}

/// Reset the schema in the current database to a clean, un-seeded baseline (the erasure replay
/// suite's helper; the substrate's own lives in its test tree, which this crate cannot import).
async fn reset_namespace(pool: &PgPool) {
    use sqlx::Executor;
    pool.execute("DROP SCHEMA IF EXISTS sensitivity CASCADE; DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .await
        .expect("drop sensitivity, recreate public schema");
    temper_substrate::MIGRATOR
        .run(pool)
        .await
        .expect("re-apply the migration chain");
    pool.execute(
        "DO $$ DECLARE r record; BEGIN \
           FOR r IN SELECT tablename FROM pg_tables \
                     WHERE schemaname = 'public' AND tablename LIKE 'kb\\_%' \
           LOOP EXECUTE 'TRUNCATE TABLE ' || quote_ident(r.tablename) || ' RESTART IDENTITY CASCADE'; \
           END LOOP; END $$;",
    )
    .await
    .expect("truncate kb_ data tables to a seed-free baseline");
}

/// One forged person-act record and rewrite, on one connection: optionally the subject tombstoned,
/// the record appended and its rows projected (or `rows` inserted by hand), then `rewrite`.
struct Forged<'a> {
    event_type: &'a str,
    projector: &'a str,
    subject_table: &'a str,
    subject: Uuid,
    estate: Vec<Uuid>,
    redacted_fields: serde_json::Value,
    tombstone: bool,
    /// Redaction rows inserted directly instead of projected: (event, path).
    rows: Option<(Uuid, &'a str)>,
    rewrite: String,
}

async fn forge_on(
    conn: &mut sqlx::PgConnection,
    emitter: Uuid,
    f: &Forged<'_>,
) -> Result<(), sqlx::Error> {
    if f.tombstone {
        sqlx::query("UPDATE kb_profiles SET tombstoned_at = now() WHERE id = $1")
            .bind(f.subject)
            .execute(&mut *conn)
            .await?;
    }
    let payload = serde_json::json!({
        "subject_table": f.subject_table,
        "subject_id": f.subject,
        "estate_contexts": f.estate,
        "resource_erasures": [],
        "charters_held": [],
        "redacted_fields": f.redacted_fields,
    });
    let event: Uuid = sqlx::query_scalar("SELECT _event_append($1, $2, NULL, NULL, $3)")
        .bind(f.event_type)
        .bind(emitter)
        .bind(&payload)
        .fetch_one(&mut *conn)
        .await?;
    match f.rows {
        Some((target, path)) => {
            sqlx::query(
                "INSERT INTO kb_event_field_redactions (event_id, path, redacted_by, authority) \
                      VALUES ($1, $2, $3, 'principal')",
            )
            .bind(target)
            .bind(path)
            .bind(event)
            .execute(&mut *conn)
            .await?;
        }
        None => {
            sqlx::query(f.projector)
                .bind(event)
                .bind(&payload)
                .execute(&mut *conn)
                .await?;
        }
    }
    sqlx::raw_sql(&f.rewrite).execute(&mut *conn).await?;
    Ok(())
}

/// [`forge_on`] in its own transaction, always rolled back. `None` when every step landed.
async fn forge(pool: &PgPool, emitter: Uuid, f: Forged<'_>) -> Option<String> {
    let mut tx = pool.begin().await.unwrap();
    let out = forge_on(&mut tx, emitter, &f).await;
    tx.rollback().await.unwrap();
    out.err().map(|e| e.to_string())
}

/// The id of the latest event of `event_type` anchored to context `c`.
async fn context_event(pool: &PgPool, event_type: &str, c: ContextId) -> Uuid {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = $1 AND e.producing_anchor_table = 'kb_contexts' \
            AND e.producing_anchor_id = $2 ORDER BY e.id DESC LIMIT 1",
    )
    .bind(event_type)
    .bind(c.uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

fn set_to_name(event: Uuid, value: &str) -> String {
    format!(
        "UPDATE kb_events SET payload = jsonb_set(payload, '{{to_name}}', to_jsonb('{value}'::text)) \
          WHERE id = '{event}'"
    )
}

/// FAILS IF the ledger's third authority admits more than the person act's own rewrite. A forged
/// `principal_erased` record is admitted only when its subject is tombstoned, only for a context
/// event of that subject's own estate, only at a context name or slug path, and only to the
/// sentinel; a resource act's record never reaches a context path; and a redaction row the record
/// does not list authorises nothing.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_principal_authority_admits_only_the_acts_own_rewrite(pool: PgPool) {
    let p = person(&pool).await;
    rename(&pool, p.me, ("notes", "notes"), (NAME, SLUG), p.emitter).await;
    let renamed = context_event(&pool, "context_renamed", p.me).await;
    let page = resource(&pool, &p, p.me, "Jane Roe page", None).await;
    let created: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_created' AND e.payload->>'resource_id' = $1::text",
    )
    .bind(page.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let stranger = person(&pool).await;
    rename(
        &pool,
        stranger.me,
        ("notes", "notes"),
        ("Stranger notes", "stranger-notes"),
        stranger.emitter,
    )
    .await;
    let strangers = context_event(&pool, "context_renamed", stranger.me).await;
    let (op, _) = operator(&pool).await;
    let principal = |subject: Uuid,
                     estate: Vec<Uuid>,
                     redacted: serde_json::Value,
                     tombstone: bool,
                     rewrite: String| Forged {
        event_type: "principal_erased",
        projector: "SELECT _project_principal_erased_redactions($1, $2)",
        subject_table: "kb_profiles",
        subject,
        estate,
        redacted_fields: redacted,
        tombstone,
        rows: None,
        rewrite,
    };
    let names = |event: Uuid, path: &str| serde_json::json!([{ "event": event, "paths": [path] }]);

    assert_eq!(
        forge(
            &pool,
            op.emitter,
            principal(
                p.profile,
                vec![p.me.uuid()],
                names(renamed, "to_name"),
                true,
                set_to_name(renamed, "erased")
            ),
        )
        .await,
        None,
        "the act's own rewrite is admitted"
    );
    for (what, forged) in [
        (
            "a value other than the sentinel",
            principal(
                p.profile,
                vec![p.me.uuid()],
                names(renamed, "to_name"),
                true,
                set_to_name(renamed, "something else"),
            ),
        ),
        (
            "a subject that is not tombstoned",
            principal(
                p.profile,
                vec![p.me.uuid()],
                names(renamed, "to_name"),
                false,
                set_to_name(renamed, "erased"),
            ),
        ),
        (
            "another principal's context, named as the estate",
            principal(
                p.profile,
                vec![stranger.me.uuid()],
                names(strangers, "to_name"),
                true,
                set_to_name(strangers, "erased"),
            ),
        ),
        (
            "a resource's title under the person's authority",
            principal(
                p.profile,
                vec![p.me.uuid()],
                names(created, "title"),
                true,
                format!(
                    "UPDATE kb_events SET payload = jsonb_set(payload, '{{title}}', \
                               to_jsonb('erased-{}'::text)) WHERE id = '{created}'",
                    page.uuid()
                ),
            ),
        ),
        (
            "a context path under a resource erasure's authority",
            Forged {
                event_type: "resource_erased",
                projector: "SELECT _project_resource_erased_redactions($1, $2)",
                subject_table: "kb_resources",
                subject: page.uuid(),
                estate: vec![],
                redacted_fields: names(renamed, "to_name"),
                tombstone: false,
                rows: None,
                rewrite: set_to_name(renamed, "erased"),
            },
        ),
        (
            "another principal's context event, its row inserted past the projector",
            Forged {
                rows: Some((strangers, "to_name")),
                ..principal(
                    p.profile,
                    vec![stranger.me.uuid()],
                    names(strangers, "to_name"),
                    true,
                    set_to_name(strangers, "erased"),
                )
            },
        ),
        (
            "a redaction row the record does not list",
            Forged {
                rows: Some((renamed, "to_name")),
                ..principal(
                    p.profile,
                    vec![p.me.uuid()],
                    serde_json::json!([]),
                    true,
                    set_to_name(renamed, "erased"),
                )
            },
        ),
    ] {
        let err = forge(&pool, op.emitter, forged)
            .await
            .unwrap_or_else(|| panic!("{what} was admitted"));
        assert!(
            err.contains("append-only") || err.contains("trail") || err.contains("allowlist"),
            "{what}: refused by the ledger's guard or the record's projector; got {err}"
        );
    }
    assert!(
        ledger_quoting(&pool, NAME).await > 0,
        "every forged rewrite rolled back"
    );
}

/// Poll until a backend in this test's database waits on a lock; panics after 10 seconds.
async fn a_backend_waits_on_a_lock(pool: &PgPool) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock')",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if waiting {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no backend waited on a lock within 10 seconds"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// FAILS IF the act reads what to rewrite before it holds the estate contexts: a rename that
/// holds an estate context when the act starts commits while the act waits on it, and the act must
/// then see its event and rewrite it, so the real name ends on no ledger row. Without the act's own
/// lock it reads the ledger before the rename commits, and the rename's event keeps the name.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_rename_committing_while_the_act_waits_is_rewritten(pool: PgPool) {
    let p = person(&pool).await;
    resource(&pool, &p, p.me, "a page", None).await;
    let (_, admin) = operator(&pool).await;
    // The rename, uncommitted: it holds the @me context's row.
    let mut held = pool.begin().await.unwrap();
    let _: Uuid = sqlx::query_scalar("SELECT context_rename($1, $2)")
        .bind(serde_json::json!({
            "context_id": p.me.uuid(),
            "from_name": "notes",
            "from_slug": "notes",
            "to_name": NAME,
            "to_slug": SLUG,
        }))
        .bind(p.emitter)
        .fetch_one(&mut *held)
        .await
        .expect("the rename runs");
    let pool2 = pool.clone();
    let subject = p.profile;
    let act = tokio::spawn(async move {
        execute_erasure(
            &pool2,
            &admin,
            ProfileId::from(subject),
            Uuid::now_v7(),
            Surface::ApiHttp,
        )
        .await
    });
    a_backend_waits_on_a_lock(&pool).await;
    held.commit().await.unwrap();
    act.await.unwrap().expect("the person act completes");

    assert_eq!(
        ledger_quoting(&pool, NAME).await,
        0,
        "the rename's event is rewritten"
    );
    assert_eq!(
        context_state(&pool, p.me).await,
        (
            "erased".to_string(),
            format!("erased-{}", p.me.uuid()),
            false
        )
    );
}

/// FAILS IF an erased context can be moved out of its erased owner's estate, where a later run of
/// the act could no longer reach its ledger copies: a co-admin of the personal team who also
/// administers another team is refused (SQLSTATE TE001).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_erased_context_cannot_be_reassigned(pool: PgPool) {
    let p = person(&pool).await;
    let co_admin = person(&pool).await;
    let personal_team: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(p.profile)
        .fetch_one(&pool)
        .await
        .unwrap();
    let elsewhere = other_team_context(&pool).await;
    let elsewhere_team: Uuid = sqlx::query_scalar("SELECT owner_id FROM kb_contexts WHERE id = $1")
        .bind(elsewhere.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    for team in [personal_team, elsewhere_team] {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'maintainer')",
        )
        .bind(team)
        .bind(co_admin.profile)
        .execute(&pool)
        .await
        .unwrap();
    }

    erase(&pool, p.profile).await;

    let err = writes::reassign_context_with(
        &pool,
        p.personal_team,
        ("kb_teams", personal_team),
        ("kb_teams", elsewhere_team),
        EntityId::from(co_admin.emitter),
        EventContext::default(),
    )
    .await
    .expect_err("the reassign is refused");
    let code = err
        .downcast_ref::<sqlx::Error>()
        .and_then(|e| e.as_database_error())
        .and_then(|d| d.code().map(|c| c.into_owned()));
    assert_eq!(
        code.as_deref(),
        Some("TE001"),
        "refused with TE001: {err:#}"
    );
    let owner: Uuid = sqlx::query_scalar("SELECT owner_id FROM kb_contexts WHERE id = $1")
        .bind(p.personal_team.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner, personal_team, "the context stays in the estate");
}
