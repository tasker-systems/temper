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

async fn mk_typed(
    pool: &PgPool,
    home: ContextId,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    body: &str,
    doc_type: &str,
) -> Uuid {
    writes::create_resource(
        pool,
        writes::CreateParams {
            idempotency_key: None,
            sources: vec![],
            title,
            origin_uri: &format!("test://{title}"),
            body,
            doc_type,
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
        stages: vec![StageNode::Act(ActInvocation {
            name,
            act: ActName::FindExact,
            intention: Some(Intention {
                query: query.to_string(),
                embedding: None,
            }),
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

    let r = run_composition(&pool, owner, &one_find("composable", vec![], vec![]))
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

    let without = run_composition(&pool, owner, &one_find("composable", vec![], vec![]))
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
    )
    .await
    .expect("runs");
    // `is_some()` cannot see this property and used to be the whole assertion: `Some(Value::Null)`
    // passes it while saying something the doc above forbids. The tier must be an empty OBJECT —
    // "you asked, and there is nothing here" — not merely present. Mutation-probed.
    assert_eq!(
        hits(&with)[0].resource.open_meta,
        Some(serde_json::Value::Object(Default::default())),
        "requested and empty is `{{}}`, which is a different answer from both absent and null"
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

/// **A declared doc-type filter actually NARROWS, and the echo is evidence of a filter that ran.**
///
/// `[rewritten — 2026-08-09]` Its predecessor asserted only the echo, and picked a `doc_type` that
/// MATCHED the sole fixture — so it could not distinguish "filtered" from "not filtered", and it
/// passed while the compiler was dropping the filter entirely and the response was reporting it as
/// applied. It codified the defect instead of catching it. Two doc types now, and the assertion is
/// on which rows come back.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_declared_doc_type_narrows_the_result_and_the_echo_reports_a_filter_that_ran(
    pool: PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "filtered").await;
    let concept = mk_typed(
        &pool,
        home,
        owner,
        emitter,
        "A concept",
        "composable fragments",
        "concept",
    )
    .await;
    mk_typed(
        &pool,
        home,
        owner,
        emitter,
        "A session",
        "composable fragments",
        "session",
    )
    .await;

    let unfiltered = run_composition(&pool, owner, &one_find("composable", vec![], vec![]))
        .await
        .expect("runs");
    assert_eq!(
        hits(&unfiltered).len(),
        2,
        "precondition: both match the query, or the contrast below proves nothing"
    );

    let r = run_composition(
        &pool,
        owner,
        &one_find("composable", vec![], vec!["concept".to_string()]),
    )
    .await
    .expect("runs");

    let got = hits(&r);
    assert_eq!(got.len(), 1, "the session must be excluded; got: {got:?}");
    assert_eq!(got[0].resource.id.uuid(), concept);

    let narrowed = &r.returned[&StageName::parse("hits").unwrap()].narrowed_by;
    assert_eq!(narrowed.len(), 1);
    assert_eq!(narrowed[0].key, "doc_type");
    assert_eq!(narrowed[0].value, "concept");
    assert_eq!(
        narrowed[0].admitted, None,
        "counts ride only where an act computes them for free; absent is not zero"
    );
}

/// **A narrowing this door cannot apply is REFUSED, never silently dropped.**
///
/// The compiler has one filter slot. Everything else a `ResourceFilter` can carry — and every
/// property predicate — was accepted by validation and then ignored, so a caller asking for
/// `tags: ["x"]` received everything and was told nothing. Refusing is the contract's own rule: a
/// narrowing is declined, never ignored.
#[test]
fn a_narrowing_this_door_cannot_apply_is_refused_rather_than_dropped() {
    let name = StageName::parse("hits").unwrap();
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: name.clone(),
                with: vec![],
            }],
        },
        stages: vec![StageNode::Act(ActInvocation {
            name,
            act: ActName::FindExact,
            intention: Some(Intention {
                query: "composable".to_string(),
                embedding: None,
            }),
            input: None,
            terms: Default::default(),
            resource_filter: Some(ResourceFilter {
                tags: vec!["x".to_string()],
                ..Default::default()
            }),
            edge_filter: None,
            properties: vec![],
        })],
    };
    let errs = validate(&c).expect_err("an unapplied narrowing must refuse");
    assert!(
        errs.iter()
            .any(|e| e.reason == temper_core::types::query::RefusalReason::FilterNotApplicable),
        "got: {errs:?}"
    );
}

/// **`open-meta` is per ARM.** An arm that did not ask for it must not receive it because a sibling
/// did — absent means NOT REQUESTED, and `{}` means requested-and-empty.
///
/// The single-arm test above cannot see this: with one arm, "filled for the response" and "filled
/// for the arm that asked" are the same thing. Found in review, and this is the two-arm shape that
/// distinguishes them.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn open_meta_reaches_only_the_arm_that_asked_for_it(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "arms").await;
    mk(
        &pool,
        home,
        owner,
        emitter,
        "Shared",
        "composable fragments",
    )
    .await;

    let asked = StageName::parse("asked").unwrap();
    let silent = StageName::parse("silent").unwrap();
    let stage = |n: &StageName| {
        StageNode::Act(ActInvocation {
            name: n.clone(),
            act: ActName::FindExact,
            intention: Some(Intention {
                query: "composable".to_string(),
                embedding: None,
            }),
            input: None,
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    };
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![
                ReturnSpec {
                    stage: asked.clone(),
                    with: vec![ResourceSection::OpenMeta],
                },
                ReturnSpec {
                    stage: silent.clone(),
                    with: vec![],
                },
            ],
        },
        stages: vec![stage(&asked), stage(&silent)],
    };
    let v = validate(&c).expect("plan is valid");

    let r = run_composition(&pool, owner, &v).await.expect("runs");

    let arm = |n: &StageName| match &r.returned[n].produced {
        StageOutput::Resources { hits } => hits[0].resource.open_meta.clone(),
        other => panic!("expected resources, got {other:?}"),
    };
    assert_eq!(
        arm(&asked),
        Some(serde_json::Value::Object(Default::default())),
        "the arm that asked receives the tier, and an empty tier is `{{}}` rather than null"
    );
    assert_eq!(
        arm(&silent),
        None,
        "the arm that did not ask must not be told this resource has no open metadata"
    );
}

/// **The server embeds on the caller's behalf, and this is the only place that is witnessed.**
///
/// `test-embed` gated: these run real ONNX twice over — once to give the corpus true vectors
/// (`create_resource` with `chunks: None` chunks and embeds server-side), once for the query. A run
/// scoped `--features test-db` alone compiles this module to nothing and reads green.
///
/// `query_read.rs`'s own `wants_a_vector` doc names the absence these close: a hardcoded
/// `"search_wide"` survived the `served_by` repoint of 2026-08-12 with no test going red, because
/// *"there is no find-about case in `query_run_composition_test.rs`, and `/api/query` has no route
/// yet, so the door would have opened already broken."* The unit test over `search_family()` is a
/// family assertion; nothing drove the embed and the compiler together until here.
///
/// **They go through [`prepare`], not `validate`** — which is the point. `validate` seals a plan,
/// and a sealed plan cannot be given a vector; a test that reached for it would be asserting over a
/// composition the server never had a chance to embed for.
#[cfg(feature = "test-embed")]
mod server_side_embedding {
    use super::*;
    use temper_services::backend::query_read::prepare;

    fn find_about(stage: &str, query: &str) -> StageNode {
        StageNode::Act(ActInvocation {
            name: StageName::parse(stage).unwrap(),
            act: ActName::FindAboutAnywhere,
            // **No embedding.** This is the ruby gem / TypeScript package / MCP caller — the whole
            // class of client that structurally cannot compute one.
            intention: Some(Intention {
                query: query.to_string(),
                embedding: None,
            }),
            input: None,
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    }

    fn about(stages: Vec<StageNode>) -> Composition {
        Composition {
            outcome: OutcomeDeclaration {
                returns: stages
                    .iter()
                    .map(|n| ReturnSpec {
                        stage: n.name().clone(),
                        with: vec![],
                    })
                    .collect(),
            },
            stages,
        }
    }

    fn stage_hits<'a>(
        r: &'a temper_core::types::query::QueryResponse,
        stage: &str,
    ) -> &'a Vec<temper_core::types::query::ResourceHit> {
        let s = &r.returned[&StageName::parse(stage).unwrap()];
        assert_eq!(
            s.disposition,
            StageDisposition::Answered,
            "stage `{stage}` did not answer; refusal: {:?}",
            s.refusal
        );
        match &s.produced {
            StageOutput::Resources { hits } => hits,
            other => panic!("expected resources, got {other:?}"),
        }
    }

    /// **A caller that cannot embed gets a server-computed vector, not a refusal.**
    ///
    /// The regression this closes was quiet by construction: an unembedded find-about stage refuses
    /// `EmbeddingUnavailable`, which from outside is indistinguishable from a genuine ONNX failure.
    /// So the assertion is on the stage ANSWERING with a real row — the one outcome the broken path
    /// cannot produce — rather than on the absence of a refusal, which a broken embedder would also
    /// satisfy by refusing for a different reason.
    #[sqlx::test(migrator = "temper_services::MIGRATOR")]
    async fn a_caller_that_cannot_embed_still_gets_a_vector_search(pool: PgPool) {
        bootseed::seed_system(&pool).await.unwrap();
        let (owner, emitter) = system_actor(&pool).await;
        let home = ctx(&pool, owner, "semantic").await;
        let id = mk(
            &pool,
            home,
            owner,
            emitter,
            "Hovering hunters",
            "The kestrel is a small falcon that hunts by hovering above open grassland, \
             dropping onto voles and large insects it spots below.",
        )
        .await;

        // Deliberately shares NO word with the body: a lexical arm cannot produce this hit, so the
        // row can only have arrived through a vector the SERVER computed.
        let c = about(vec![find_about("birds", "raptor predation behaviour")]);
        let v = prepare(c).await.expect("the plan is valid");
        let r = run_composition(&pool, owner, &v)
            .await
            .expect("the composition runs");

        let hits = stage_hits(&r, "birds");
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        assert_eq!(hits[0].resource.id.uuid(), id);
        assert!(
            hits[0].scoring.score > 0.0,
            "a real similarity, not a NULL bind reading as an answer"
        );
    }

    /// **Two stages, two questions, two vectors — the property the whole PR exists to create.**
    ///
    /// Under the retired envelope placement this composition was inexpressible: every find stage
    /// interrogated one string. Asserting it end-to-end rather than at the type layer is what makes
    /// it a property of the ANSWER — a single shared vector would rank both stages identically, so
    /// the two stages disagreeing about their top hit is exactly the observation that cannot be
    /// faked by anything upstream.
    #[sqlx::test(migrator = "temper_services::MIGRATOR")]
    async fn two_stages_asking_different_questions_receive_different_vectors(pool: PgPool) {
        bootseed::seed_system(&pool).await.unwrap();
        let (owner, emitter) = system_actor(&pool).await;
        let home = ctx(&pool, owner, "twoquestions").await;
        let falcon = mk(
            &pool,
            home,
            owner,
            emitter,
            "Hovering hunters",
            "The kestrel is a small falcon that hunts by hovering above open grassland, \
             dropping onto voles and large insects it spots below.",
        )
        .await;
        let bread = mk(
            &pool,
            home,
            owner,
            emitter,
            "The slow rise",
            "Sourdough dough ferments with a wild yeast starter; the baker folds it every hour \
             and bakes the loaf in a covered pot at high heat.",
        )
        .await;

        let c = about(vec![
            find_about("birds", "raptor predation behaviour"),
            find_about("baking", "fermented loaves and oven craft"),
        ]);
        let v = prepare(c).await.expect("the plan is valid");
        let r = run_composition(&pool, owner, &v)
            .await
            .expect("the composition runs");

        assert_eq!(
            stage_hits(&r, "birds")[0].resource.id.uuid(),
            falcon,
            "the stage asking about raptors ranked the baking note first — both stages were \
             answered by ONE vector"
        );
        assert_eq!(
            stage_hits(&r, "baking")[0].resource.id.uuid(),
            bread,
            "the stage asking about bread ranked the falconry note first — both stages were \
             answered by ONE vector"
        );
    }
}
