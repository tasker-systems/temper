#![cfg(feature = "test-db")]

//! `run_composition` — the whole `/api/query` read path against a real database.
//!
//! The pure assembly tests (`backend::query_read`) cover every derivation with a hand-built
//! `QueryRows`, which is where the disclosure numbers belong. What they structurally cannot reach is
//! **hydration**: that a row's id survives compile → execute → `hit_identities` and comes back as a
//! `ResourceView` carrying the resource the caller would recognise. A hand-built map proves nothing
//! about that chain.

use sqlx::PgPool;
use temper_core::types::query::{
    validate, ActInvocation, ActName, Composition, Intention, OutcomeDeclaration, ResourceFilter,
    ReturnSpec, StageDisposition, StageName, StageNode, StageOutput, ValidatedComposition,
};
use temper_core::types::resource_view::ResourceSection;
use temper_services::backend::query_read::run_composition;
use temper_substrate::ids::{ContextId, EntityId, ProfileId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes;
use uuid::Uuid;

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

async fn ctx(pool: &PgPool, owner: ProfileId, slug: &str) -> ContextId {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles',$1,$2,$2) RETURNING id",
    )
    .bind(owner.uuid())
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap();
    ContextId::from(id)
}

async fn mk(
    pool: &PgPool,
    home: ContextId,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    body: &str,
) -> Uuid {
    writes::create_resource(
        pool,
        writes::CreateParams {
            idempotency_key: None,
            sources: vec![],
            title,
            origin_uri: &format!("test://{title}"),
            body,
            doc_type: "concept",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
        },
    )
    .await
    .unwrap()
    .uuid()
}

/// One `find-exact` stage, optionally hydrating `open-meta` and optionally filtered by doc type.
fn one_find(
    query: &str,
    with: Vec<ResourceSection>,
    doc_type: Vec<String>,
) -> ValidatedComposition {
    let name = StageName::parse("hits").unwrap();
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: name.clone(),
                with,
            }],
        },
        intention: Some(Intention {
            query: query.to_string(),
            embedded: false,
        }),
        meta_detail: Default::default(),
        bounds: Default::default(),
        stages: vec![StageNode::Act(ActInvocation {
            name,
            act: ActName::FindExact,
            input: None,
            terms: Default::default(),
            resource_filter: (!doc_type.is_empty()).then(|| ResourceFilter {
                doc_type,
                ..Default::default()
            }),
            edge_filter: None,
            properties: vec![],
        })],
    };
    validate(&c).expect("plan is valid")
}

fn hits(
    response: &temper_core::types::query::QueryResponse,
) -> &Vec<temper_core::types::query::ResourceHit> {
    match &response.returned[&StageName::parse("hits").unwrap()].produced {
        StageOutput::Resources { hits } => hits,
        other => panic!("expected resources, got {other:?}"),
    }
}

/// **The chain nothing else covers**: a match survives compile → execute → hydrate and comes back
/// as the resource the caller would recognise, carrying the score kind its act declared.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_matched_resource_comes_back_hydrated_and_carries_its_acts_score_kind(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "hydrate").await;
    let id = mk(
        &pool,
        home,
        owner,
        emitter,
        "The pipe",
        "composable search fragments",
    )
    .await;

    let r = run_composition(&pool, owner, &one_find("composable", vec![], vec![]), None)
        .await
        .expect("the composition runs");

    let hits = hits(&r);
    assert_eq!(hits.len(), 1, "got: {hits:?}");
    assert_eq!(hits[0].resource.id.uuid(), id);
    assert_eq!(
        hits[0].resource.title, "The pipe",
        "hydrated from the real row, not reconstructed from the id"
    );
    assert_eq!(
        hits[0].scoring.score_kind.as_str(),
        "fts_norm",
        "the kind travels WITH the row so it can be read on its own"
    );
    assert!(hits[0].scoring.score > 0.0);

    let stage = &r.returned[&StageName::parse("hits").unwrap()];
    assert_eq!(stage.disposition, StageDisposition::Answered);
    assert_eq!(r.trace.stages.len(), 1, "every stage is traced");
}

/// `with: [open-meta]` fills the open tier; **absent means NOT REQUESTED, never empty.**
///
/// An empty open tier is `{}` and an unrequested one is absent, and both survive the wire. Conflating
/// them tells a caller "this resource has no open metadata" when the truth is "you did not ask".
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn open_meta_is_absent_until_asked_for_and_empty_is_a_different_answer(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "sections").await;
    mk(&pool, home, owner, emitter, "Plain", "composable fragments").await;

    let without = run_composition(&pool, owner, &one_find("composable", vec![], vec![]), None)
        .await
        .expect("runs");
    assert_eq!(
        hits(&without)[0].resource.open_meta,
        None,
        "not requested is ABSENT"
    );

    let with = run_composition(
        &pool,
        owner,
        &one_find("composable", vec![ResourceSection::OpenMeta], vec![]),
        None,
    )
    .await
    .expect("runs");
    assert!(
        hits(&with)[0].resource.open_meta.is_some(),
        "requested and empty is `{{}}`, which is a different answer from absent"
    );
}

/// A resource the principal cannot see never reaches the response — and the stage says how many of
/// nothing it found rather than pretending it searched everything.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_resource_the_principal_cannot_see_is_not_in_the_answer(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "private").await;
    mk(&pool, home, owner, emitter, "Mine", "composable fragments").await;

    let stranger: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('qr-stranger','qr-stranger') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let r = run_composition(
        &pool,
        ProfileId::from(stranger),
        &one_find("composable", vec![], vec![]),
        None,
    )
    .await
    .expect("runs");

    assert!(hits(&r).is_empty());
    assert_eq!(
        r.returned[&StageName::parse("hits").unwrap()].disposition,
        StageDisposition::Empty,
        "asked and matched nothing — never `withheld`, which would disclose that something exists"
    );
}

/// A declared filter is echoed back so a reader can see what narrowed the stage.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_declared_filter_is_echoed_into_the_stages_disclosure(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "filtered").await;
    mk(&pool, home, owner, emitter, "One", "composable fragments").await;

    let r = run_composition(
        &pool,
        owner,
        &one_find("composable", vec![], vec!["concept".to_string()]),
        None,
    )
    .await
    .expect("runs");

    let narrowed = &r.returned[&StageName::parse("hits").unwrap()].narrowed_by;
    assert_eq!(narrowed.len(), 1);
    assert_eq!(narrowed[0].key, "doc_type");
    assert_eq!(narrowed[0].value, "concept");
    assert_eq!(
        narrowed[0].admitted, None,
        "counts ride only where an act computes them for free; absent is not zero"
    );
}
