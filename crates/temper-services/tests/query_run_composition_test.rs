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
    validate, ActInvocation, ActName, BoundTerm, Composition, Extent, IdKind, IdSet, Intention,
    OutcomeDeclaration, ResourceFilter, ReturnSpec, StageDisposition, StageInput, StageName,
    StageNode, StageOutput, StageRelation, ValidatedComposition,
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

/// A second principal, so a test can tell an APPLIED owner narrowing from an inert one.
async fn insert_profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id")
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
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
            inputs: vec![],
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

/// A `find-resources-with` selection piped into a `find-exact` as a bound.
///
/// The selection is NOT returned — it orders nothing, so asking for its rows is
/// `stage_not_returnable`. Its work shows up in the find stage's `input_ids` and in its own trace
/// entry, which is exactly the disclosure a caller needs to tell a real narrowing from a costume.
fn selection_then_find(doc_type: &str, query: &str) -> ValidatedComposition {
    let sel = StageName::parse("sel").unwrap();
    let find = StageName::parse("hits").unwrap();
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: find.clone(),
                with: vec![],
            }],
        },
        stages: vec![
            StageNode::Act(ActInvocation {
                name: sel.clone(),
                act: ActName::FindResourcesWith,
                // No intention: a pure selection asks the corpus nothing.
                intention: None,
                inputs: vec![],
                terms: Default::default(),
                resource_filter: Some(ResourceFilter {
                    doc_type: vec![doc_type.to_string()],
                    ..Default::default()
                }),
                edge_filter: None,
                properties: vec![],
            }),
            StageNode::Act(ActInvocation {
                name: find,
                act: ActName::FindExact,
                intention: Some(Intention {
                    query: query.to_string(),
                    embedding: None,
                }),
                inputs: vec![temper_core::types::query::StageInput::Upstream {
                    relation: temper_core::types::query::StageRelation::Bound,
                    stage: sel,
                }],
                terms: Default::default(),
                resource_filter: None,
                edge_filter: None,
                properties: vec![],
            }),
        ],
    };
    validate(&c).expect("plan is valid")
}

/// A selection narrowed by `owner`, piped into a find act so the plan has something returnable.
fn selection_owned_by(owner: &str) -> ValidatedComposition {
    let sel = StageName::parse("sel").unwrap();
    let find = StageName::parse("hits").unwrap();
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: find.clone(),
                with: vec![],
            }],
        },
        stages: vec![
            StageNode::Act(ActInvocation {
                name: sel.clone(),
                act: ActName::FindResourcesWith,
                intention: None,
                inputs: vec![],
                terms: Default::default(),
                resource_filter: Some(ResourceFilter {
                    owner: Some(owner.to_string()),
                    ..Default::default()
                }),
                edge_filter: None,
                properties: vec![],
            }),
            StageNode::Act(ActInvocation {
                name: find,
                act: ActName::FindExact,
                intention: Some(Intention {
                    query: "composable".to_string(),
                    embedding: None,
                }),
                inputs: vec![temper_core::types::query::StageInput::Upstream {
                    relation: temper_core::types::query::StageRelation::Bound,
                    stage: sel,
                }],
                terms: Default::default(),
                resource_filter: None,
                edge_filter: None,
                properties: vec![],
            }),
        ],
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
/// **The pipe: a selection bounds a find act, end to end, and the trace says so.**
///
/// `[rewritten — 2026-08-14]` This was
/// `a_declared_doc_type_narrows_the_result_and_the_echo_reports_a_filter_that_ran`, which put
/// `doc_type` on the find stage as a MODIFIER. That is no longer expressible: narrowing by what a
/// resource IS is the `find-resources-with` act, and a find act carrying a resource filter is
/// refused (`filter_not_applicable`) rather than ignored.
///
/// **Rewritten rather than deleted, because the property it witnessed did not go away — it moved.**
/// The old test proved a declared narrowing actually excluded rows and was echoed honestly. Both
/// halves are asserted here, one stage later: the selection excludes the session, the find act
/// returns only what the selection admitted, and `narrowed_by` reports the narrowing on the stage
/// that applied it. The refusal that replaced the old spelling is witnessed separately, below.
///
/// **`find-exact` downstream, not `find-about-within`.** The acceptance criterion names the latter;
/// the wide arm needs a query embedding, this suite is `test-db` with no embedder, and a stage the
/// compiler refuses for want of one emits a refused body carrying no bound at all — so the pipe
/// would be untestable here for a reason that has nothing to do with the pipe. The property under
/// test is that an upstream selection reaches stage two's `p_bound_ids`, and that is the same
/// property whichever find act consumes it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_selection_bounds_the_find_act_it_is_piped_into(pool: PgPool) {
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

    let r = run_composition(&pool, owner, &selection_then_find("concept", "composable"))
        .await
        .expect("runs");

    let got = hits(&r);
    assert_eq!(
        got.len(),
        1,
        "the session is outside the selection, so the find act must not return it; got: {got:?}"
    );
    assert_eq!(got[0].resource.id.uuid(), concept);

    // **The trace is where a composition is legible**, and it carries every stage — including the
    // selection, whose rows are never returned. `input_ids` is the field that answers "did my pipe
    // actually do anything": a bound stage showing 0 narrowed nothing and its answer is the
    // unbounded one wearing a composition's costume.
    let sel = StageName::parse("sel").unwrap();
    let sel_trace = r
        .trace
        .stages
        .iter()
        .find(|t| t.stage == sel)
        .expect("the selection stage appears in the trace even though it returns nothing");
    let hits_trace = r
        .trace
        .stages
        .iter()
        .find(|t| t.stage == StageName::parse("hits").unwrap())
        .expect("the find stage appears in the trace");
    assert_eq!(
        hits_trace.input_ids, 1,
        "the bound must REACH stage two — one id, the concept the selection admitted"
    );

    // And the echo: the narrowing is reported on the stage that applied it, with counts absent
    // rather than zero, because no fragment computes what it dropped.
    assert_eq!(
        sel_trace.narrowed_by.len(),
        1,
        "got: {:?}",
        sel_trace.narrowed_by
    );
    assert_eq!(sel_trace.narrowed_by[0].key, "doc_type");
    assert_eq!(sel_trace.narrowed_by[0].value, "concept");
    assert_eq!(sel_trace.narrowed_by[0].admitted, None);
    assert!(
        hits_trace.narrowed_by.is_empty(),
        "the find act narrows by nothing now; the echo belongs to the act that applied it"
    );
}

/// **`@me` resolves to the calling principal, and does so at EXECUTION.**
///
/// `[regression — 2026-08-14, found in adversarial review]` `@me` fell through to the handle slot
/// and matched nothing: handles carry no sigil (`system`, not `@system`), so the most likely owner
/// value anyone writes selected an empty set while `narrowed_by` echoed it back as an applied
/// narrowing — the silent question substitution this act exists to remove, in the one act whose
/// entire output is a narrowing.
///
/// Two assertions, because either alone would pass over a different bug: that `@me` selects the
/// caller's own resources (it resolves at all), and that it is not compiled to a literal id (it
/// resolves to whoever RUNS the plan, so the same compiled plan cannot answer for another profile).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn owner_at_me_resolves_to_the_calling_principal(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "atme").await;
    let mine = mk_typed(
        &pool,
        home,
        owner,
        emitter,
        "A concept",
        "composable fragments",
        "concept",
    )
    .await;

    // **A SECOND resource owned by someone else, or this test cannot tell "resolved" from
    // "inert".** The first version of this test had one fixture, so an owner slot bound to NULL —
    // no narrowing at all — produced the same single hit as a correctly resolved `@me`. It passed
    // against the bug it was written for. Bite-probed after this change: reverting `@me` to the
    // handle slot now reds it.
    // Homed in the CALLER's context so it is visible, but OWNED by someone else — which is the
    // only shape that makes the owner slot observable. A resource in the other principal's own
    // context would be correctly invisible, and the contrast would then be visibility rather than
    // ownership.
    let theirs = ProfileId::from(insert_profile(&pool, "someone-else").await);
    let not_mine = mk_typed(
        &pool,
        home,
        theirs,
        emitter,
        "Their concept",
        "composable fragments",
        "concept",
    )
    .await;

    // Precondition: unfiltered, the caller sees BOTH — so the contrast below is the owner slot.
    let unfiltered = run_composition(&pool, owner, &one_find("composable", vec![], vec![]))
        .await
        .expect("runs");
    assert_eq!(
        hits(&unfiltered).len(),
        2,
        "precondition: both resources are visible and match, or the filter proves nothing"
    );

    let r = run_composition(&pool, owner, &selection_owned_by("@me"))
        .await
        .expect("runs");
    let got = hits(&r);
    assert_eq!(
        got.len(),
        1,
        "`@me` must resolve to the caller — not match a literal handle (0 hits), and not go \
         un-applied (2 hits); got: {got:?}"
    );
    assert_eq!(got[0].resource.id.uuid(), mine);
    assert!(
        !got.iter().any(|h| h.resource.id.uuid() == not_mine),
        "another principal's resource must not survive an `@me` narrowing"
    );

    // A handle that exists but is not the caller's spelling of themselves still works, so the
    // resolution above is `@me` specifically and not a filter that stopped narrowing.
    let by_handle = run_composition(&pool, owner, &selection_owned_by("system"))
        .await
        .expect("runs");
    assert_eq!(hits(&by_handle).len(), 1);

    // And a handle nobody has is an honest empty — the `@me` fix did not make the owner slot inert.
    let nobody = run_composition(&pool, owner, &selection_owned_by("nobody"))
        .await
        .expect("runs");
    assert!(
        hits(&nobody).is_empty(),
        "an owner nobody has selects nothing"
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
            inputs: vec![],
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

/// One `leads_to` edge, weight 1.0 — everything the walk below needs beyond the resources
/// themselves. The imports sit inside the body the way `query_plan_execute.rs`'s `edge` keeps them,
/// so the write-path types do not enter this file's header for one fixture.
async fn edge(pool: &PgPool, src: Uuid, tgt: Uuid, home: ContextId, emitter: EntityId) {
    use temper_substrate::affinity::EdgeKind;
    use temper_substrate::events::{fire, EdgeHome, SeedAction};
    use temper_substrate::ids::ResourceId;
    use temper_substrate::payloads::EdgePolarity;
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src: ResourceId::from(src),
            tgt: ResourceId::from(tgt),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("rel"),
            weight: 1.0,
            home: EdgeHome::Context(home),
            emitter,
        },
    )
    .await
    .unwrap()
    .relationship()
    .unwrap();
    tx.commit().await.unwrap();
}

/// A one-stage `follow-from` over caller-supplied seeds, carrying declared bound terms.
///
/// The stage is named `hits` so [`hits`] reads it, like every other plan in this file.
fn walk_paged(seeds: Vec<Uuid>, terms: Vec<(BoundTerm, i64)>) -> ValidatedComposition {
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
            // A walk asks the corpus nothing, so it carries no intention.
            act: ActName::FollowFrom,
            intention: None,
            inputs: vec![StageInput::Caller {
                relation: StageRelation::Seed,
                ids: IdSet {
                    kind: IdKind::Resource,
                    provenance: None,
                    ids: seeds,
                },
            }],
            terms: terms.into_iter().collect(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })],
    };
    validate(&c).expect("a seeded walk with declared terms is well-formed")
}

/// **A caller told its walk may have been cut has a move that settles it, and taking that move
/// yields the rest.**
///
/// `Extent::Partial` is a promise the caller can act on, and until `20260817000020` the walk could
/// not keep it: `follow-from` accepted `Limit` alone, so a stage reporting `partial` offered a
/// reader no way to see what was behind the page. The two halves are asserted together because
/// either alone is satisfiable while the other is broken — a walk that always reports `partial` has
/// no settling move, and a walk that pages correctly while reporting `complete` on a full page
/// never tells the caller to page.
///
/// The `assemble` unit tests witness `Partial` over a hand-built `QueryRows`, which is where the
/// derivation belongs. What they structurally cannot reach is that the offset a caller declares in
/// response to it runs against a real database and returns the rows the first page did not.
///
/// **A declared limit of 2 over three neighbours, rather than 51 resources against the ceiling of
/// 50.** `extent_of` reports `Partial` iff the stage produced at least its APPLIED limit, and 2 is
/// as applied as 50 — the ceiling would only make this test fifty resources slower without changing
/// which line it is that fires.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_partial_walk_is_settled_by_the_page_behind_it(pool: PgPool) {
    use std::collections::BTreeSet;
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "walkpage").await;
    let hub = mk(&pool, home, owner, emitter, "Hub", "composable fragments").await;
    let mut leaves = BTreeSet::new();
    for title in ["One", "Two", "Three"] {
        let leaf = mk(&pool, home, owner, emitter, title, "composable fragments").await;
        edge(&pool, hub, leaf, home, emitter).await;
        leaves.insert(leaf);
    }

    let stage = StageName::parse("hits").unwrap();

    let page1 = run_composition(
        &pool,
        owner,
        &walk_paged(vec![hub], vec![(BoundTerm::Limit, 2)]),
    )
    .await
    .expect("the walk runs");
    let first: Vec<Uuid> = hits(&page1).iter().map(|h| h.resource.id.uuid()).collect();
    assert_eq!(first.len(), 2, "the declared page: {first:?}");
    assert_eq!(
        page1.returned[&stage].terms_applied.get(&BoundTerm::Limit),
        Some(&2),
        "and it is the page the statement ran, not the one the caller asked for"
    );
    assert_eq!(
        page1.returned[&stage].extent,
        Extent::Partial,
        "a page filled to its applied limit may have more behind it — this is what the caller is \
         told, and what the next request answers"
    );

    // The move the disclosure licenses: skip what was already seen, ask again.
    let page2 = run_composition(
        &pool,
        owner,
        &walk_paged(
            vec![hub],
            vec![(BoundTerm::Limit, 2), (BoundTerm::Offset, 2)],
        ),
    )
    .await
    .expect("the second page runs");
    let second: Vec<Uuid> = hits(&page2).iter().map(|h| h.resource.id.uuid()).collect();
    assert_eq!(
        second.len(),
        1,
        "the move yields the REST — one neighbour, not another two, which is what an offset that \
         was accepted and then ignored would return: {second:?}"
    );
    assert_eq!(
        page2.returned[&stage].extent,
        Extent::Complete,
        "and an unfilled page is the end of the walk, so the caller knows to stop paging"
    );
    assert!(
        second.iter().all(|id| !first.contains(id)),
        "the second page repeats nothing from the first: {first:?} then {second:?}"
    );
    assert_eq!(
        first.iter().chain(&second).copied().collect::<BTreeSet<_>>(),
        leaves,
        "and the two pages together are every neighbour of the hub — the truncation the first page \
         disclosed is fully recoverable"
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
            inputs: vec![],
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
            inputs: vec![],
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

/// **The region column survives compile → execute → `HitRow` → assembler.**
///
/// Beat 0 widened the stage contract with a fifth column so `survey` can disclose the regions it
/// matched. Every test above this one is compile-time: they read emitted SQL text, or they assemble
/// a hand-built `QueryRows`. Neither can see a `UNION` arm mismatch, which is a **runtime** error —
/// the column list is shared across hit arms, tally arms and the zero-arm fallback, and an arm
/// missing it fails only when Postgres parses the statement.
///
/// This runs a real composition against a real database, which is the only level at which that is
/// observable. A `find-exact` stage discloses nothing, so the assertion is that the column comes
/// back as an honest **empty** rather than erroring or arriving absent — the round trip proved by
/// the value, not by the statement merely not throwing.
///
/// # What this does NOT prove, stated so the coverage is not over-read
///
/// **That `survey` populates it.** That needs a context with materialized regions and embedded
/// chunks — `survey`'s lateral join drops any resource with no current chunk — which is the
/// embed-gated tier (`context_region_smoke.rs` loads a 22-resource seed and runs ONNX). It is a
/// **named remainder**, not a covered case, and it is CI's job rather than this file's.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_region_column_round_trips_and_a_non_survey_act_discloses_nothing(pool: PgPool) {
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "disclosure").await;
    mk(
        &pool,
        home,
        owner,
        emitter,
        "salience",
        "a body about salience",
    )
    .await;

    let v = one_find("salience", vec![], vec![]);
    let r = run_composition(&pool, owner, &v)
        .await
        .expect("the widened stage contract must still be valid SQL");

    let result = &r.returned[&StageName::parse("hits").unwrap()];
    assert!(
        result.disclosed_regions.is_empty(),
        "find-exact discloses no region — the column is NULL in its arm and must arrive as an \
         empty list, not as an error and not as a missing field"
    );
    let trace = r
        .trace
        .stages
        .iter()
        .find(|s| s.stage == StageName::parse("hits").unwrap())
        .expect("every stage has a trace entry");
    assert_eq!(
        trace.disclosed_regions, result.disclosed_regions,
        "the pair rule holds across a real round trip, not only over a hand-built QueryRows"
    );
}
