#![cfg(feature = "test-db")]

//! **The batch read, through its service** — the shape loaded for the principal on every page, the
//! ask validated through one session, the page and the first page's population read from the
//! validated bind — against a real database (spec `2026-10-19-asking-artifacts-by-what-they-are`
//! §7.2, §7.3, §3.5).
//!
//! The SQL's own witnesses, the page's edges and the deny direction conjunct by conjunct, are
//! `temper-substrate`'s `artifact_ask_batch_read.rs`; this file holds what only the service shows:
//! the population on the first page alone, a continuation answering on the row its first page named
//! after an amendment — and reading its states against the version now in force — readability
//! re-decided on every page, unreadable answering as absent, and served content as long as the
//! budget charged it.

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
    request_in(shape, vec![], limit, after)
}

/// [`request`], narrowed to `shape_states`.
fn request_in(
    shape: Uuid,
    shape_states: Vec<String>,
    limit: u32,
    after: Option<ArtifactCursor>,
) -> AskRequest {
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
            shape_states,
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

/// **A continuation narrowed by `shape_states` reads its states against the version now in force,
/// not the one its first page named.** Three matches, all `declared_satisfied` against the asked
/// version; page one, narrowed to `declared_satisfied`, serves two. The shape is then amended, and
/// no verdict exists for the new version yet. Page two is validated against the asked row and says
/// so (`amended_since`), but its one remaining row reads `declared_not_yet_checked` against the
/// version in force, so the narrowed ask matches nothing: no artifacts and `next: None`. The
/// control, the same continuation with every state admitted, serves that row and shows its state.
/// This pins today's behaviour, which the `amended_since` doc teaches: a narrowed ask restarts on
/// `amended_since`. The read budget plays no part here — three small rows are far inside it.
///
/// Bite (perl, Rust): in `artifact_ask_read::read`, `amended_since,` → `amended_since: None,` in
/// the response — page two no longer says the version moved, and its assertion goes red.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_narrowed_continuation_reads_its_states_against_the_version_now_in_force(pool: PgPool) {
    let w = world(&pool).await;
    let mut matches = Vec::new();
    for i in 0..3 {
        matches.push(
            commit(&pool, &w, json!({ "unit": "ms", "count": i }))
                .await
                .uuid(),
        );
    }
    let satisfied = || vec!["declared_satisfied".to_string()];
    let first = ask_page(&pool, w.owner, &request_in(w.shape, satisfied(), 2, None))
        .await
        .expect("page one");
    assert_eq!(served(&first), matches[..2]);
    assert!(first.next.is_some(), "a third match remains");

    let amended = declare(&pool, &w, true).await;
    let successor = ShapeSuccessor {
        shape_id: ShapeId::from(amended),
        shape_version: 2,
    };

    let second = ask_page(
        &pool,
        w.owner,
        &request_in(w.shape, satisfied(), 2, first.next),
    )
    .await
    .expect("a narrowed continuation answers");
    assert_eq!(second.amended_since, Some(successor));
    assert_eq!(second.shape.shape_id, ShapeId::from(w.shape));
    assert!(
        second.artifacts.is_empty() && second.oversized.is_empty(),
        "against the version in force the remaining row is unchecked, and the narrowed ask \
         matches nothing: {:?}",
        served(&second)
    );
    assert_eq!(second.next, None);

    let every_state = ask_page(&pool, w.owner, &request(w.shape, 2, first.next))
        .await
        .expect("the control");
    assert_eq!(served(&every_state), matches[2..]);
    assert_eq!(
        every_state.artifacts[0].shape_state, "declared_not_yet_checked",
        "the row's state is read against the version now in force"
    );
}

/// The JSON-string length of one text field, as a page writes it and the page's SQL charges it.
fn written(text: &str) -> usize {
    serde_json::to_string(text)
        .expect("a string serializes")
        .len()
}

/// **Served content is as long as its `content_bytes`, floats included, and a served row is
/// within what the page charged for it.** Storage prints a number in plain notation (`1e300` as
/// 301 digits), so each float here is committed, stored, read back and written again: large and
/// small exponents, the extremes of `f64`, a subnormal, and long mantissas. Each served row's
/// content, compactly serialized, is exactly its `content_bytes`; the whole row is within its
/// charge (`content_bytes`, each text field's written length and `ARTIFACT_ROW_FIXED_BYTES`); and
/// the page's rows together are within `ARTIFACT_ASK_PAGE_BYTES`.
///
/// The bite cannot be a perl edit: the length holds because `serde_json`'s `float_roundtrip`
/// makes the parse back from storage exact, and that feature reaches every build also through
/// `jsonschema`, which enables it unconditionally, so dropping it from temper-services and
/// temper-client changes nothing. To see it red, build with every edge to `float_roundtrip` cut
/// (a `[patch.crates-io]` `jsonschema` whose `serde_json` dependency omits it): without it the
/// parse of a long plain expansion is best effort and may land a unit in the last place away,
/// which is then written back in up to 17 significant digits, and the length assertion goes red
/// on that value.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn served_floats_are_as_long_as_their_content_bytes(pool: PgPool) {
    use temper_services::transport::{ARTIFACT_ASK_PAGE_BYTES, ARTIFACT_ROW_FIXED_BYTES};

    let w = world(&pool).await;
    let floats = [
        1e300,
        1e-300,
        f64::MAX,
        f64::MIN_POSITIVE,
        5e-324,
        f64::MIN,
        0.1 + 0.2,
        1.0 / 3.0,
        123_456_789.0 / 7.0,
        -9.876_543_210_987_654e-200,
        2.225_073_858_507_201e-308,
        std::f64::consts::PI * 1e250,
        std::f64::consts::E * 1e-250,
    ];
    let mut committed = Vec::new();
    for (i, x) in floats.iter().enumerate() {
        let content = json!({ "unit": "ms", "count": i, "x": x, "xs": [x, -x, x / 3.0] });
        committed.push((commit(&pool, &w, content.clone()).await.uuid(), content));
    }

    let page = ask_page(&pool, w.owner, &request(w.shape, 50, None))
        .await
        .expect("the owner asks its own shape");
    assert_eq!(
        page.artifacts
            .iter()
            .map(|a| (a.artifact_id.uuid(), a.content.clone().expect("content")))
            .collect::<Vec<_>>(),
        committed,
        "every float served as committed"
    );
    let mut rows = 0;
    for a in &page.artifacts {
        let content = a.content.as_ref().expect("content");
        assert_eq!(
            i64::try_from(serde_json::to_string(content).unwrap().len()).unwrap(),
            a.content_bytes,
            "served at its committed length: {content}"
        );
        let charge = usize::try_from(a.content_bytes).unwrap()
            + written(&a.kind_owner_table)
            + written(&a.artifact_kind)
            + written(&a.intent)
            + written(&a.content_hash)
            + written(&a.shape_state)
            + ARTIFACT_ROW_FIXED_BYTES;
        let row = serde_json::to_string(a).unwrap().len();
        assert!(row <= charge, "a row of {row} bytes charged {charge}");
        rows += row;
    }
    assert!(rows <= ARTIFACT_ASK_PAGE_BYTES);
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
