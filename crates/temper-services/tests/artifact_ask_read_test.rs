#![cfg(feature = "test-db")]

//! **The batch read, through its service** — the shape loaded for the principal on every page, the
//! ask validated through one session, the page and the first page's population read from the
//! validated bind — against a real database (spec `2026-10-19-asking-artifacts-by-what-they-are`
//! §7.2, §7.3, §3.5).
//!
//! The SQL's own witnesses, the page's edges and the deny direction conjunct by conjunct, are
//! `temper-substrate`'s `artifact_ask_batch_read.rs`; this file holds what only the service shows:
//! the population on the first page alone, a continuation answering on the row its first page named
//! after an amendment, and readability re-decided on every page, unreadable answering as absent.

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::artifact_ask::{
    ArtifactAsk, ArtifactPredicate, AskOrder, AskRequest, AskResponse, ShapeSuccessor,
};
use temper_core::types::data_artifact::ArtifactCursor;
use temper_core::types::ids::ShapeId;
use temper_core::types::query::filter::PropertyOp;
use temper_core::types::query::{ArtifactAskPopulation, PlanRefusal, RefusalReason};
use temper_services::backend::artifact_ask_read::ask_page;
use temper_services::error::{ApiError, ApiResult};
use temper_substrate::ids::{ContextId, DataArtifactId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, ArtifactIntent, EnforcementMode, KindOwner};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CommitDataArtifactParams, CreateParams, DeclareShapeParams};
use uuid::Uuid;

const MEASUREMENT: &str = "measurement";

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

fn schema(with_p99: bool) -> serde_json::Value {
    let mut properties = json!({ "unit": { "type": "string" }, "count": { "type": "integer" } });
    if with_p99 {
        properties["p99"] = json!({ "type": "number" });
    }
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": properties
    })
}

/// One context home owned by the system profile, its `measurement` shape declared, and one
/// resource holding the family's artifacts.
struct World {
    owner: ProfileId,
    emitter: EntityId,
    home: AnchorRef,
    shape: Uuid,
    on: Uuid,
}

async fn declare(pool: &PgPool, w: &World, with_p99: bool) -> Uuid {
    writes::declare_shape(
        pool,
        DeclareShapeParams {
            home: w.home,
            kind: MEASUREMENT,
            kind_owner: Some(KindOwner::Profile(w.owner.uuid())),
            schema: &schema(with_p99),
            enforcement: EnforcementMode::Advisory,
            emitter: w.emitter,
        },
    )
    .await
    .unwrap()
    .uuid()
}

async fn world(pool: &PgPool) -> World {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let context: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, 'asked', 'asked') RETURNING id",
    )
    .bind(owner.uuid())
    .fetch_one(pool)
    .await
    .unwrap();
    let home = AnchorRef::context(ContextId::from(context));
    let origin = "test://artifact-ask-read/holder";
    let on = writes::create_resource_deferred_with(
        pool,
        CreateParams {
            idempotency_key: None,
            sources: vec![],
            title: "holder",
            origin_uri: origin,
            body: "measurements",
            doc_type: "research",
            home,
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
        },
        temper_substrate::events::EventContext::default(),
    )
    .await
    .unwrap()
    .uuid();
    let mut w = World {
        owner,
        emitter,
        home,
        shape: Uuid::nil(),
        on,
    };
    w.shape = declare(pool, &w, false).await;
    w
}

async fn commit(pool: &PgPool, w: &World, content: serde_json::Value) -> DataArtifactId {
    writes::commit_data_artifact(
        pool,
        CommitDataArtifactParams {
            resource: ResourceId::from(w.on),
            kind: MEASUREMENT,
            kind_owner: Some(KindOwner::Profile(w.owner.uuid())),
            intent: ArtifactIntent::Member,
            precedence: 0.0,
            content: &content,
            supersedes: &[],
            emitter: w.emitter,
        },
    )
    .await
    .unwrap()
}

/// `unit` contains `"ms"`, of `shape`, one page of `limit` after `after`.
fn request(shape: Uuid, limit: u32, after: Option<ArtifactCursor>) -> AskRequest {
    AskRequest {
        ask: ArtifactAsk {
            shape: ShapeId::from(shape),
            predicates: vec![ArtifactPredicate {
                path: vec!["unit".to_string()],
                op: PropertyOp::Contains {
                    values: vec![json!("ms")],
                },
            }],
            intent: vec![],
            include_folded: false,
            shape_states: vec![],
        },
        limit: Some(limit),
        after,
    }
}

fn served(page: &AskResponse) -> Vec<Uuid> {
    page.artifacts
        .iter()
        .map(|a| a.artifact_id.uuid())
        .collect()
}

fn refusals(answer: ApiResult<AskResponse>) -> Vec<PlanRefusal> {
    match answer {
        Err(ApiError::PlanRefused { refusals }) => refusals,
        Err(other) => panic!("expected refusals, got {other:?}"),
        Ok(page) => panic!("expected refusals, got a page of {:?}", served(&page)),
    }
}

/// **Five matches at two a page: three pages, the population on the first alone, each artifact
/// whole.** The first page carries what the ask counted (five matched of six governed) and the later
/// ones `None`; the rows come in order, once each, with their content as committed; every page names
/// the asked shape and the order; `next` is `None` on the last page only.
///
/// Bite (perl, Rust): in `artifact_ask_read::read`, `None => Some(` → `_ => Some(`: the later pages
/// carry the population too, and the second assertion goes red.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_first_page_carries_the_population_and_later_pages_do_not(pool: PgPool) {
    let w = world(&pool).await;
    let mut matches = Vec::new();
    for i in 0..5 {
        let content = json!({ "unit": "ms", "count": i });
        matches.push((commit(&pool, &w, content.clone()).await.uuid(), content));
        if i == 1 {
            commit(&pool, &w, json!({ "unit": "s" })).await;
        }
    }

    let mut pages = Vec::new();
    let mut after = None;
    loop {
        let page = ask_page(&pool, w.owner, &request(w.shape, 2, after))
            .await
            .expect("the owner asks its own shape");
        after = page.next;
        pages.push(page);
        if after.is_none() || pages.len() > 5 {
            break;
        }
    }
    assert_eq!(
        pages.iter().map(|p| p.artifacts.len()).collect::<Vec<_>>(),
        [2, 2, 1]
    );
    assert_eq!(
        pages
            .iter()
            .map(|p| p.population.is_some())
            .collect::<Vec<_>>(),
        [true, false, false],
        "the population on the first page alone"
    );
    assert_eq!(
        pages[0].population,
        Some(ArtifactAskPopulation {
            declared_satisfied: 6,
            declared_not_satisfied: 0,
            declared_not_yet_checked: 0,
            matched: 5,
        })
    );
    let read: Vec<(Uuid, serde_json::Value)> = pages
        .iter()
        .flat_map(|p| &p.artifacts)
        .map(|a| (a.artifact_id.uuid(), a.content.clone().expect("content")))
        .collect();
    assert_eq!(read, matches, "every match once, in order, whole");
    for page in &pages {
        assert_eq!(page.shape.shape_id, ShapeId::from(w.shape));
        assert_eq!(page.ordered_by, AskOrder::Created);
        assert_eq!(page.amended_since, None);
        assert!(page.oversized.is_empty());
        assert!(page
            .artifacts
            .iter()
            .all(|a| a.shape_state == "declared_satisfied"));
    }
}

/// **A shape amended between pages: the next page answers on the row the first named, and says
/// what is now in force; a fresh ask naming the folded row is refused and taught the successor**
/// (spec §7.3, P17). Three matches, a page of two, then the shape is amended (a new id at version
/// 2, the asked row folded — both asserted). Page two continues against the asked row: the third
/// match, `shape` still the asked row, `amended_since` the new one. A first page naming the folded
/// row is refused `shape_superseded` alone, teaching the same successor.
///
/// Bite (perl, Rust): in `artifact_ask_read::load`, the `Some(_) =>` arm loading through
/// `shape_loader::load_one` like a first page — page two is then refused `shape_superseded`, and the
/// continuation assertion goes red.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_page_after_an_amendment_answers_on_the_pinned_row(pool: PgPool) {
    let w = world(&pool).await;
    let mut matches = Vec::new();
    for i in 0..3 {
        matches.push(
            commit(&pool, &w, json!({ "unit": "ms", "count": i }))
                .await
                .uuid(),
        );
    }
    let first = ask_page(&pool, w.owner, &request(w.shape, 2, None))
        .await
        .expect("page one");
    assert_eq!(served(&first), matches[..2]);
    assert_eq!(first.amended_since, None);

    let amended = declare(&pool, &w, true).await;
    assert_ne!(amended, w.shape, "an amendment mints a new id");
    let folded: bool =
        sqlx::query_scalar("SELECT is_folded FROM kb_data_artifact_shapes WHERE id = $1")
            .bind(w.shape)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(folded, "the asked row is folded");
    let successor = ShapeSuccessor {
        shape_id: ShapeId::from(amended),
        shape_version: 2,
    };

    let second = ask_page(&pool, w.owner, &request(w.shape, 2, first.next))
        .await
        .expect("a continuation answers on the row its first page named");
    assert_eq!(served(&second), matches[2..]);
    assert_eq!(second.shape.shape_id, ShapeId::from(w.shape));
    assert_eq!(second.amended_since, Some(successor));
    assert_eq!(second.population, None);
    assert_eq!(second.next, None);

    let fresh = refusals(ask_page(&pool, w.owner, &request(w.shape, 2, None)).await);
    assert_eq!(
        fresh.iter().map(|r| r.reason.clone()).collect::<Vec<_>>(),
        [RefusalReason::ShapeSuperseded]
    );
    assert_eq!(
        fresh[0].teaches.as_ref().and_then(|t| t.successor),
        Some(successor),
        "the refusal teaches the version now in force"
    );
}

/// **Unreadable is refused exactly as absent is, on every page.** An outsider asking the owner's
/// shape and anyone asking an id that names no shape get equal refusal lists (`shape_not_askable`
/// alone, same detail). A reader granted the home reads page one; once the grant is revoked, page
/// two — the same ask, the same continuation — is refused exactly as an absent shape is: the
/// continuation carried position, never authority.
///
/// Bite (perl, Rust): in `shape_loader::load_one_pinned`, the unreadable arm's
/// `load: ShapeLoad::NotAskable,` → `load: ShapeLoad::Uncompilable,`: the revoked page is refused
/// `shape_schema_uncompilable`, which an absent shape is not, and the last assertion goes red.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_unreadable_shape_is_refused_as_an_absent_one_on_every_page(pool: PgPool) {
    let w = world(&pool).await;
    for i in 0..2 {
        commit(&pool, &w, json!({ "unit": "ms", "count": i })).await;
    }
    let absent = Uuid::now_v7();
    let profile = |handle: &'static str| {
        let pool = pool.clone();
        async move {
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
            )
            .bind(handle)
            .fetch_one(&pool)
            .await
            .unwrap();
            ProfileId::from(id)
        }
    };
    let outsider = profile("batch-read-outsider").await;
    let reader = profile("batch-read-reader").await;

    let as_absent = refusals(ask_page(&pool, w.owner, &request(absent, 1, None)).await);
    assert_eq!(
        as_absent
            .iter()
            .map(|r| r.reason.clone())
            .collect::<Vec<_>>(),
        [RefusalReason::ShapeNotAskable]
    );
    assert_eq!(
        refusals(ask_page(&pool, outsider, &request(w.shape, 1, None)).await),
        as_absent,
        "an unreadable shape is refused as an absent one"
    );

    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, \
              granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
    )
    .bind(w.home.id)
    .bind(reader.uuid())
    .bind(w.owner.uuid())
    .execute(&pool)
    .await
    .expect("grant read on the home");
    let first = ask_page(&pool, reader, &request(w.shape, 1, None))
        .await
        .expect("the reader reads the home");
    assert_eq!(first.artifacts.len(), 1);
    assert!(first.next.is_some(), "a second page remains");

    sqlx::query(
        "DELETE FROM kb_access_grants \
          WHERE subject_table = 'kb_contexts' AND subject_id = $1 AND principal_id = $2",
    )
    .bind(w.home.id)
    .bind(reader.uuid())
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        refusals(ask_page(&pool, reader, &request(w.shape, 1, first.next)).await),
        as_absent,
        "after revocation the continuation is refused as an absent shape is"
    );
}
