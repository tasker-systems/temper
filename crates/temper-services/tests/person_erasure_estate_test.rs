#![cfg(feature = "test-db")]
//! Witnesses for the person act running resource erasure over its estate (20261021100000; design
//! temper-artifacts `specs/2026-10-10-person-erasure-runs-resource-erasure-design.md`, rulings R1
//! and R5 of the 2026-10-10 erasure inventory).
//!
//! Each world is built through the real write paths and erased through
//! `erasure_service::execute_erasure`. Every witness below fails against the act before this
//! migration: that act reached only `@me` contexts, emptied prose by hash, and left titles,
//! properties, edge labels and the ledger. Witness W3 of the design (a charter in the estate) has
//! no witness here: a charter's telos resource is homed in its own cogmap
//! (`_project_cogmap_seeded`) and `kb_resource_homes` allows one home per resource, so no write
//! path puts a charter in an estate context. The act's charter disposition is a guard for a state
//! no write path produces.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::erasure::{EstateDisposition, EstateErasureKind};
use temper_core::types::ids::ProfileId;
use temper_services::services::erasure_service::{execute_erasure, survey_erasure};
use temper_services::services::resource_erasure_service::{
    execute_resource_erasure, ResourceErasureOutcome, ResourceErasureRequest,
};
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, EdgeId, EntityId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity, Incorporation, ProvenanceSource};
use temper_substrate::replay;
use temper_substrate::writes::{self, CreateParams};
use temper_workflow::operations::Surface;

/// Text that must end nowhere: in no projection row the act reaches and in no ledger payload.
const TITLE: &str = "Jane Roe medical history";
const LABEL: &str = "jane roe told us";
const PROPERTY_VALUE: &str = "jane roe diagnosis";
const URL: &str = "https://example.test/jane-roe-records";

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
    let emitter: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id",
    )
    .bind(profile)
    .bind(format!("{handle}@web"))
    .fetch_one(pool)
    .await
    .expect("seed emitter");
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

async fn operator(pool: &PgPool) -> temper_services::auth::SystemAdmin {
    let op = person(pool).await;
    temper_services::test_support::grant_governance(pool, op.profile).await;
    temper_services::test_support::system_admin_proof_for(pool, op.profile).await
}

/// A resource through the real create path, optionally citing a remote source.
async fn resource(
    pool: &PgPool,
    p: &Person,
    home: ContextId,
    title: &str,
    body: &str,
    url: Option<&str>,
) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &format!("test://{}", Uuid::now_v7()),
            body,
            doc_type: "research",
            home: AnchorRef::context(home),
            owner: ProfileId::from(p.profile),
            originator: ProfileId::from(p.profile),
            emitter: EntityId::from(p.emitter),
            properties: &[],
            chunks: None,
            sources: url
                .map(|u| {
                    vec![Incorporation {
                        source: ProvenanceSource::Remote(u.to_owned()),
                        seq: 1,
                    }]
                })
                .unwrap_or_default(),
        },
        EventContext::default(),
    )
    .await
    .expect("create through the real path")
}

async fn edge(
    pool: &PgPool,
    p: &Person,
    src: ResourceId,
    tgt: ResourceId,
    home: ContextId,
    label: &str,
) -> EdgeId {
    let mut conn = pool.acquire().await.expect("acquire");
    fire(
        &mut conn,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(src),
            tgt: AnchorRef::resource(tgt),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some(label),
            weight: 1.0,
            home: EdgeHome::Context(home),
            emitter: EntityId::from(p.emitter),
        },
    )
    .await
    .expect("assert the edge")
    .relationship()
    .expect("an edge id")
}

async fn erase(
    pool: &PgPool,
    subject: Uuid,
) -> temper_services::services::erasure_service::ErasureCompletion {
    let admin = operator(pool).await;
    execute_erasure(
        pool,
        &admin,
        ProfileId::from(subject),
        Uuid::now_v7(),
        Surface::ApiHttp,
    )
    .await
    .expect("the person act completes")
}

/// Ledger rows (payload or metadata) whose text contains `needle`.
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

async fn erased_at(pool: &PgPool, r: ResourceId) -> Option<chrono::DateTime<chrono::Utc>> {
    sqlx::query_scalar("SELECT erased_at FROM kb_resources WHERE id = $1")
        .bind(r.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

/// W1. FAILS IF the estate stops at `@me`: a resource homed in the subject's personal team's
/// context must end a husk, its text emptied and its title a sentinel, like an `@me` resource.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_personal_team_resource_is_erased_with_the_estate(pool: PgPool) {
    let p = person(&pool).await;
    let r = resource(
        &pool,
        &p,
        p.personal_team,
        TITLE,
        "a private journal entry",
        None,
    )
    .await;

    let completion = erase(&pool, p.profile).await;

    assert!(
        erased_at(&pool, r).await.is_some(),
        "the resource is a husk"
    );
    let (title, live_text): (String, i64) = sqlx::query_as(
        "SELECT r.title, (SELECT count(*) FROM kb_chunks c \
                            JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
                           WHERE c.resource_id = r.id AND cc.content <> '') \
           FROM kb_resources r WHERE r.id = $1",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        title,
        format!("erased-{}", r.uuid()),
        "the title is the sentinel"
    );
    assert_eq!(live_text, 0, "no chunk keeps its prose");
    assert!(
        completion
            .resource_erasures
            .iter()
            .any(|e| e.resource_id == r.uuid() && e.kind == EstateErasureKind::Erasure),
        "the record names the personal-team resource's erasure"
    );
    let retired: bool = sqlx::query_scalar("SELECT NOT is_active FROM kb_contexts WHERE id = $1")
        .bind(p.personal_team.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        retired,
        "the personal team's context is retired with the estate"
    );
}

/// W2. FAILS IF the estate's surface survives anywhere: an `@me` resource's title, property, edge
/// label (on an edge to another team's resource) and remote-source URL must be gone from the
/// projection AND from every ledger payload, and the edge folded.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_estates_surface_is_gone_from_projection_and_ledger(pool: PgPool) {
    let p = person(&pool).await;
    let team = other_team_context(&pool).await;
    let r = resource(&pool, &p, p.me, TITLE, "notes about a person", Some(URL)).await;
    let outside = resource(&pool, &p, team, "team page", "team text", None).await;
    let e = edge(&pool, &p, r, outside, p.me, LABEL).await;
    writes::set_property(
        &pool,
        r,
        "diagnosis",
        &serde_json::json!(PROPERTY_VALUE),
        EntityId::from(p.emitter),
    )
    .await
    .expect("set a property");
    for needle in [TITLE, LABEL, PROPERTY_VALUE, URL] {
        assert!(
            ledger_quoting(&pool, needle).await > 0,
            "setup: the ledger holds {needle:?}"
        );
    }

    erase(&pool, p.profile).await;

    for needle in [TITLE, LABEL, PROPERTY_VALUE, URL] {
        assert_eq!(
            ledger_quoting(&pool, needle).await,
            0,
            "no ledger payload quotes {needle:?} after the act"
        );
    }
    let (title, origin): (String, String) =
        sqlx::query_as("SELECT title, origin_uri FROM kb_resources WHERE id = $1")
            .bind(r.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(title, format!("erased-{}", r.uuid()));
    assert_eq!(origin, format!("erased:{}", r.uuid()));
    let live_props: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties WHERE owner_table = 'kb_resources' AND owner_id = $1 \
            AND (property_value::text ILIKE '%jane%' OR NOT is_folded)",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(live_props, 0, "every property row is folded and sentineled");
    let (folded, label): (bool, Option<String>) =
        sqlx::query_as("SELECT is_folded, label FROM kb_edges WHERE id = $1")
            .bind(e.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(folded, "the edge into the team's resource is folded (2e)");
    assert_eq!(label, None, "its label is cleared");
    let urls: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_remote_sources WHERE uri = $1")
        .bind(URL)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(urls, 0, "the remote source the estate alone cited is gone");
    // The team's resource itself is outside the estate and stays live.
    assert!(
        erased_at(&pool, outside).await.is_none(),
        "the team's resource is not erased"
    );
}

/// W5. FAILS IF two estate resources sharing an edge collide: the second erasure must name none of
/// the shared events' paths the first already rewrote. One redaction row per (event, path) is the
/// table's key, so a collision raises and the act would never commit.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn two_estate_resources_sharing_an_edge_are_erased_in_one_act(pool: PgPool) {
    let p = person(&pool).await;
    let a = resource(&pool, &p, p.me, "first note", "first body", None).await;
    let b = resource(
        &pool,
        &p,
        p.personal_team,
        "second note",
        "second body",
        None,
    )
    .await;
    let shared = edge(&pool, &p, a, b, p.me, LABEL).await;
    let asserted: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'relationship_asserted' AND (e.payload->>'edge_id')::uuid = $1",
    )
    .bind(shared.uuid())
    .fetch_one(&pool)
    .await
    .expect("the edge's asserting event");

    let completion = erase(&pool, p.profile).await;

    assert_eq!(
        completion.resource_erasures.len(),
        2,
        "both resources are erased"
    );
    assert_eq!(
        ledger_quoting(&pool, LABEL).await,
        0,
        "the shared label is gone from the ledger"
    );
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_event_field_redactions WHERE event_id = $1 AND path = 'label'",
    )
    .bind(asserted)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 1, "one redaction row for the shared label");
    let naming: Vec<Uuid> = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_erased' \
            AND e.payload->'redacted_fields' @> jsonb_build_array(jsonb_build_object('event', $1::text))",
    )
    .bind(asserted.to_string())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        naming.len(),
        1,
        "exactly one erasure names the shared event"
    );
}

/// A husk as cut 1 left it (the substrate witness `simulate_cut1_husk`'s shape): a
/// `resource_erased` naming no `redacted_fields`, and the projection-side body, with the ledger's
/// text untouched.
async fn simulate_cut1_husk(pool: &PgPool, p: &Person, resource: Uuid) {
    let request_ref = Uuid::now_v7();
    let mut tx = pool.begin().await.unwrap();
    let plan: serde_json::Value = sqlx::query_scalar("SELECT resource_erasure_survey_plan($1)")
        .bind(resource)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(
        plan["edges"].as_array().unwrap().is_empty(),
        "setup: no edge to fold"
    );
    let erased: Uuid = sqlx::query_scalar(
        "SELECT _event_append('resource_erased', $2, NULL, NULL, jsonb_build_object( \
             'subject_table', 'kb_resources', 'subject_id', $1, 'actor', $3, \
             'folded_edges', $4->'edges', 'targets', $4->'targets', 'remainder', $4->'remainder', \
             'ledger_remainder', ($4->'redacted_fields') || ($4->'ledger_remainder')), \
             p_correlation => $5)",
    )
    .bind(resource)
    .bind(p.emitter)
    .bind(p.profile)
    .bind(&plan)
    .bind(request_ref)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query("SELECT _resource_erasure_apply_redaction($1, $2)")
        .bind(resource)
        .bind(erased)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

/// W6. FAILS IF an already-erased resource raises or is erased twice: a husk the resource act left
/// complete is skipped and counted, and a husk erased before cut 2 gets its completion pass.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn already_erased_resources_are_skipped_or_completed(pool: PgPool) {
    let p = person(&pool).await;
    let complete = resource(&pool, &p, p.me, "complete husk", "body one", None).await;
    let cut1 = resource(&pool, &p, p.me, TITLE, "body two", None).await;
    let admin = operator(&pool).await;
    let outcome = execute_resource_erasure(
        &pool,
        None,
        &admin,
        ResourceErasureRequest {
            resource: complete,
            also_strike_blobs: &[],
            surface: Surface::ApiHttp,
        },
    )
    .await
    .expect("the resource act runs");
    assert!(matches!(outcome, ResourceErasureOutcome::Completed(_)));
    simulate_cut1_husk(&pool, &p, cut1.uuid()).await;
    assert!(
        ledger_quoting(&pool, TITLE).await > 0,
        "setup: the cut-1 husk's ledger keeps its title"
    );

    let survey = survey_erasure(&pool, &admin, ProfileId::from(p.profile))
        .await
        .expect("survey");
    let disposition = |r: ResourceId| {
        survey
            .resources
            .iter()
            .find(|x| x.resource_id == r.uuid())
            .map(|x| x.disposition)
    };
    assert_eq!(disposition(complete), Some(EstateDisposition::Skip));
    assert_eq!(disposition(cut1), Some(EstateDisposition::Complete));

    let completion = erase(&pool, p.profile).await;

    assert_eq!(
        completion
            .resource_erasures
            .iter()
            .map(|e| (e.resource_id, e.kind))
            .collect::<Vec<_>>(),
        vec![(cut1.uuid(), EstateErasureKind::Completion)],
        "only the cut-1 husk is reached, by a completion pass"
    );
    assert!(
        completion.targets.iter().any(|t| t.target == "kb_resources"
            && t.outcome == "1 estate resource(s) already erased and complete; skipped"),
        "the complete husk is counted as skipped: {:?}",
        completion.targets
    );
    assert_eq!(
        ledger_quoting(&pool, TITLE).await,
        0,
        "the completion pass rewrote the title"
    );
}

/// W7. FAILS IF the record does not name the act's resource erasures: in resource-id order, each
/// referenced with rel `erasure`, all sharing the completion's correlation id and `occurred_at`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_record_names_the_resource_erasures_in_order(pool: PgPool) {
    let p = person(&pool).await;
    let mut ids = Vec::new();
    for i in 0..3 {
        let home = if i % 2 == 0 { p.me } else { p.personal_team };
        ids.push(
            resource(&pool, &p, home, &format!("note {i}"), "body", None)
                .await
                .uuid(),
        );
    }
    ids.sort();

    let completion = erase(&pool, p.profile).await;

    assert_eq!(
        completion
            .resource_erasures
            .iter()
            .map(|e| e.resource_id)
            .collect::<Vec<_>>(),
        ids,
        "in resource-id order"
    );
    let (payload, refs, correlation, occurred): (
        serde_json::Value,
        serde_json::Value,
        Uuid,
        chrono::DateTime<chrono::Utc>,
    ) = sqlx::query_as(
        "SELECT payload, \"references\", correlation_id, occurred_at FROM kb_events WHERE id = $1",
    )
    .bind(completion.event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut estate: Vec<Uuid> = vec![p.me.uuid(), p.personal_team.uuid()];
    estate.sort();
    assert_eq!(
        payload["estate_contexts"],
        serde_json::json!(estate),
        "the record names the estate it reached"
    );
    for e in &completion.resource_erasures {
        assert!(
            refs.as_array()
                .unwrap()
                .iter()
                .any(|r| r["rel"] == "erasure"
                    && r["target"]["id"] == serde_json::json!(e.event_id.to_string())),
            "the completion references {}",
            e.event_id
        );
        let (c, o): (Uuid, chrono::DateTime<chrono::Utc>) =
            sqlx::query_as("SELECT correlation_id, occurred_at FROM kb_events WHERE id = $1")
                .bind(e.event_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            (c, o),
            (correlation, occurred),
            "one span: one correlation, one transaction"
        );
    }
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

/// W4. FAILS IF replay of the combined act diverges from live. The world holds what W1, W2, W5 and
/// W6 hold together, so one span carries several resource erasures, a completion pass, their edge
/// folds and the completion, whose ids sort in no fixed order within the millisecond: replay must
/// apply every body at the span's end, in the act's order (design D7).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_of_the_estate_act_is_byte_identical(pool: PgPool) {
    let p = person(&pool).await;
    let team = other_team_context(&pool).await;
    let a = resource(&pool, &p, p.me, TITLE, "first body", Some(URL)).await;
    let b = resource(
        &pool,
        &p,
        p.personal_team,
        "second",
        "second body",
        Some(URL),
    )
    .await;
    let outside = resource(&pool, &p, team, "team page", "team text", None).await;
    edge(&pool, &p, a, b, p.me, LABEL).await;
    edge(&pool, &p, b, outside, p.personal_team, "another label").await;
    writes::set_property(
        &pool,
        a,
        "diagnosis",
        &serde_json::json!(PROPERTY_VALUE),
        EntityId::from(p.emitter),
    )
    .await
    .unwrap();
    let cut1 = resource(&pool, &p, p.me, "older husk", "older body", None).await;
    simulate_cut1_husk(&pool, &p, cut1.uuid()).await;

    let completion = erase(&pool, p.profile).await;
    assert_eq!(
        completion.resource_erasures.len(),
        3,
        "a, b and the cut-1 husk's completion"
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
}
