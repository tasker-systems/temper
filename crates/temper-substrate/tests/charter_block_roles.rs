#![cfg(feature = "artifact-tests")]
//! Deliverable-3 acceptance: charter blocks carry a `block_role` property, and the generic
//! `resource_blocks` read filters by role — so framing never leaks into the questions projection
//! (code-review finding #1 from D2). ONNX-dependent. Isolated ephemeral DB via `temper_substrate::MIGRATOR`.
mod common;

use temper_substrate::content;
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{EntityId, ProfileId};
use temper_substrate::scenario::bootseed;
use temper_substrate::scenario::model::{QuestionDef, TelosDef};
use uuid::Uuid;

async fn seed_actor(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name, system_access) \
         VALUES ('owner', 'Owner', 'approved'::system_access) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, 'agent#1', '{}'::jsonb) RETURNING id",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .unwrap();
    (profile, entity)
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn framing_never_projects_as_a_question(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = seed_actor(&pool).await;

    // statement + 2 questions + 1 framing block. The framing prose carries a marker string we assert
    // never appears in the questions projection.
    let telos = TelosDef {
        title: "Onboarding charter".into(),
        statement: "Help a new EPD engineer reach first-merge confidence in week one.".into(),
        questions: vec![
            QuestionDef {
                question: "What transfers?".into(),
                context: String::new(),
            },
            QuestionDef {
                question: "Smallest real change?".into(),
                context: String::new(),
            },
        ],
        framing: vec!["This map coordinates with the schema-migration initiative.".into()],
    };
    let specs = telos.block_specs();
    let refs: Vec<(Option<&str>, &str)> = specs
        .iter()
        .map(|(role, prose)| (Some(*role), prose.as_str()))
        .collect();
    let blocks = content::prepare_blocks(&refs).unwrap();

    // genesis through the single firing surface (payload-first: fire builds the CogmapSeeded
    // payload + content sidecar and pre-generates the ids).
    let mut conn = pool.acquire().await.unwrap();
    let (cogmap, telos_resource) = fire(
        &mut conn,
        SeedAction::CogmapGenesis {
            name: "onboarding-cogmap",
            telos_title: "Onboarding charter",
            charter: &blocks,
            cogmap_id: None,
            telos_resource_id: None,
            owner: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
        },
    )
    .await
    .unwrap()
    .cogmap_genesis()
    .unwrap();
    let (cogmap, telos_resource): (Uuid, Uuid) = (cogmap.uuid(), telos_resource.uuid());
    drop(conn);

    // every block carries a block_role property, in seq order: statement, question, question, framing
    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT p.property_value #>> '{}' FROM kb_properties p \
         JOIN kb_content_blocks b ON b.id = p.owner_id \
         WHERE p.owner_table='kb_content_blocks' AND p.property_key='block_role' \
           AND b.resource_id = $1 ORDER BY b.seq",
    )
    .bind(telos_resource)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(roles, vec!["statement", "question", "question", "framing"]);

    // the questions projection returns exactly the two questions and NEVER the framing block
    // (principal = the cogmap itself; map-home-confers makes the telos readable, no team wiring).
    let q_rows: Vec<String> = sqlx::query_scalar(
        "SELECT body_text FROM resource_blocks($1, 'cogmap', $2, 'question') ORDER BY seq",
    )
    .bind(telos_resource)
    .bind(cogmap)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(q_rows.len(), 2, "exactly the two questions");
    assert!(
        q_rows
            .iter()
            .all(|t| !t.contains("schema-migration initiative")),
        "framing prose must not leak into the questions projection, got {q_rows:?}"
    );

    // the framing projection returns exactly the framing block
    let f_rows: Vec<String> = sqlx::query_scalar(
        "SELECT body_text FROM resource_blocks($1, 'cogmap', $2, 'framing') ORDER BY seq",
    )
    .bind(telos_resource)
    .bind(cogmap)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(f_rows.len(), 1);
    assert!(f_rows[0].contains("schema-migration initiative"));

    // unfiltered returns all four blocks
    let all_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM resource_blocks($1, 'cogmap', $2, NULL)")
            .bind(telos_resource)
            .bind(cogmap)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(all_rows, 4);
}
