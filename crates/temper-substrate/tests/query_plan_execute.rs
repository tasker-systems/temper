#![cfg(feature = "artifact-tests")]
//! **The compiled statement is run against a real database.**
//!
//! Everything in `query_plan_compile.rs` is decidable from the emitted text, and that is its
//! strength — but it cannot see whether the text is valid SQL. This file exists because it wasn't:
//! the hoisted visibility CTE read `SELECT id FROM resources_visible_to($1)`, and that function
//! returns a column called `resource_id`. Invalid since PR #663, caught by nothing, because no act
//! body referenced the CTE and no test ever handed the statement to Postgres.
//!
//! `[amended — 2026-08-09]` These tests used to bind and run the statement **by hand**, through a
//! local `run()` that mapped every returned row's `id` unconditionally. That copy went stale the
//! moment the compiler grew per-stage tally arms — whose `id` is NULL by construction — and both
//! tests died on `UnexpectedNullError`. A test-local reimplementation of the thing under test is a
//! second definition free to drift from the first, and this is what drifting looked like.
//!
//! They now run [`readback::query_exec::execute`], the shipped path. Same subjects, and stronger:
//! what executes here is what a door will execute.
//!
//! What this file witnesses:
//!
//! 1. The emitted SQL parses, plans and executes.
//! 2. **The hoisted relation is the only thing gating it.** Since `20260808000030` every act stage
//!    calls an ungated core, so if `__temper_vis` were wrong — mis-spelled column, wrong principal,
//!    an `array_agg` that swallowed the filter — a composition would return rows its principal
//!    cannot see and every text-level assertion would stay green.
//! 3. The disclosure numbers the trace is built from: every stage tallied, matched or not, and
//!    `input_unusable` counted against that same hoisted relation.

mod common;

use temper_core::types::query::{
    validate, ActInvocation, ActName, BoundTerm, Composition, IdKind, IdSet, Intention,
    OutcomeDeclaration, RefusalReason, ReturnSpec, StageInput, StageName, StageNode, StageRelation,
    ValidatedComposition,
};
use temper_substrate::ids::{ContextId, EntityId, ProfileId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::readback::query_exec::execute;
use temper_substrate::readback::query_plan::{compile, CompiledQuery};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes;
use uuid::Uuid;

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
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

async fn mk(
    pool: &sqlx::PgPool,
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

/// A chain of two `find-exact` stages: a root, and a second narrowed by the root's ids. The shape
/// the whole phase exists for, and the one where the visible set and the bound are both in scope.
fn two_stage_find(query: &str) -> ValidatedComposition {
    let stage = |name: &str, input: Option<StageInput>| {
        StageNode::Act(ActInvocation {
            name: StageName::parse(name).unwrap(),
            act: ActName::FindExact,
            intention: Some(Intention {
                query: query.to_string(),
                embedding: None,
            }),
            inputs: input.into_iter().collect(),
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    };
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: StageName::parse("narrowed").unwrap(),
                with: vec![],
            }],
        },
        stages: vec![
            stage("hits", None),
            stage(
                "narrowed",
                Some(StageInput::Upstream {
                    relation: StageRelation::Bound,
                    stage: StageName::parse("hits").unwrap(),
                }),
            ),
        ],
    };
    validate(&c).expect("plan is valid")
}

/// Run a [`CompiledQuery`] through the shipped executor, returning `(id, stage)` per HIT row.
///
/// A thin adapter over [`execute`] rather than a second implementation: the ordering and the
/// row-class split are the executor's to decide, and a test that decided them for itself is exactly
/// what went stale here before.
async fn run(pool: &sqlx::PgPool, c: &CompiledQuery) -> Result<Vec<(Uuid, String)>, anyhow::Error> {
    let rows = execute(pool, c).await?;
    Ok(rows.hits.iter().map(|h| (h.id, h.stage.clone())).collect())
}

/// The compiled statement is valid SQL, executes, and returns the rows the composition asked for.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_compiled_composition_executes_and_returns_its_declared_arm(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner.uuid(), "exec", "exec")
            .await
            .unwrap(),
    );

    let kestrel = mk(
        &pool,
        home,
        owner,
        emitter,
        "Kestrel",
        "The kestrel hovers over the verge.",
    )
    .await;
    let merlin = mk(
        &pool,
        home,
        owner,
        emitter,
        "Merlin",
        "The merlin hunts the kestrel's ground.",
    )
    .await;
    // Carries neither term: present so "returns everything" and "returns the matches" differ.
    mk(
        &pool,
        home,
        owner,
        emitter,
        "Heron",
        "The heron stands in the shallows.",
    )
    .await;

    let c = compile(&two_stage_find("kestrel"), owner).expect("compiles");
    let rows = run(&pool, &c)
        .await
        .expect("the compiled statement must run");

    let mut got: Vec<Uuid> = rows.iter().map(|(id, _)| *id).collect();
    got.sort();
    let mut want = vec![kestrel, merlin];
    want.sort();
    assert_eq!(
        got, want,
        "the narrowed stage must return the term matches and nothing else"
    );
    assert!(
        rows.iter().all(|(_, stage)| stage == "narrowed"),
        "every row is labelled by the arm that produced it; got {rows:?}"
    );
}

/// **The hoisted relation is what gates a composition — asserted by taking it away from nobody.**
///
/// Every act stage calls a core that applies no visibility gate of its own, so `__temper_vis` is the
/// single point at which a compiled composition can withhold anything. The same plan and the same
/// corpus, run for a principal with no home, no team and no grant, must return nothing.
///
/// Fails against: an `array_agg` over the wrong column, a principal bound in the wrong position, a
/// core that ignored `p_visible_ids`, or a `__temper_vis` that a stage stopped reading. Every one of
/// those leaves the compile-time text assertions green.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_composition_returns_nothing_to_a_principal_who_can_see_nothing(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner.uuid(), "exec-gate", "exec-gate")
            .await
            .unwrap(),
    );

    mk(
        &pool,
        home,
        owner,
        emitter,
        "Kestrel",
        "The kestrel hovers over the verge.",
    )
    .await;

    let plan = two_stage_find("kestrel");

    let mine = run(&pool, &compile(&plan, owner).expect("compiles"))
        .await
        .expect("runs");
    assert_eq!(
        mine.len(),
        1,
        "precondition: the owner sees their own resource, or the contrast below proves nothing"
    );

    let stranger = ProfileId::from(common::insert_profile(&pool, "exec-stranger").await);
    let theirs = run(&pool, &compile(&plan, stranger).expect("compiles"))
        .await
        .expect("runs");
    assert!(
        theirs.is_empty(),
        "a principal with no path to the resource must receive nothing — the ungated cores gate \
         nothing themselves, so this is entirely the hoisted relation's job; got {theirs:?}"
    );
}

// ─── The disclosure numbers, at the seam where they are computed ────────────────────────────────
//
// Everything above is about WHICH rows come back. These are about what the trace is built from —
// the per-stage tallies, which are the only evidence a stage that returned nothing ran at all.

async fn ctx(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

/// One `find-exact` stage, optionally narrowed to a caller-supplied resource set.
///
/// `find-exact` rather than a `find-about-*` act because it needs no embedding: the property under
/// test is the tally, not which arm produced the rows.
fn one_find_exact(query: &str, bound: Option<Vec<Uuid>>) -> ValidatedComposition {
    let name = StageName::parse("a").unwrap();
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
                query: query.to_string(),
                embedding: None,
            }),
            inputs: bound
                .map(|ids| StageInput::Caller {
                    relation: StageRelation::Bound,
                    ids: IdSet {
                        kind: IdKind::Resource,
                        provenance: None,
                        ids,
                    },
                })
                .into_iter()
                .collect(),
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })],
    };
    validate(&c).expect("plan is valid")
}

/// One `find-exact` stage carrying declared bound terms, returned.
fn find_exact_paged(query: &str, terms: Vec<(BoundTerm, i64)>) -> ValidatedComposition {
    let name = StageName::parse("a").unwrap();
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
                query: query.to_string(),
                embedding: None,
            }),
            inputs: vec![],
            terms: terms.into_iter().collect(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })],
    };
    validate(&c).expect("plan is valid")
}

/// **`limit` returns that many rows and `offset` returns different ones.**
///
/// `limit` and `offset` are POSITIONAL — two `int` slots in the fragment's signature — and until
/// this test nothing anywhere asserted which value reached which slot. The compile-level test
/// asserts `ints.contains(&10) && ints.contains(&20)`, which is set membership over a positional
/// property: swapping the two binds leaves it green. Mutation-probed by swapping the slots outright;
/// the whole suite stayed green, and this test fires.
///
/// The three resources carry the term at descending frequency so `ts_rank` orders them stably.
/// Without that, "offset moved the window" would be a claim about a tie-break Postgres never
/// promised, and the test would be measuring luck.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_declared_limit_returns_that_many_rows_and_an_offset_returns_a_different_one(
    pool: sqlx::PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "paging").await;
    for (title, body) in [
        ("Thrice", "kestrel kestrel kestrel"),
        ("Twice", "kestrel kestrel"),
        ("Once", "kestrel"),
    ] {
        mk(&pool, home, owner, emitter, title, body).await;
    }

    async fn page(
        pool: &sqlx::PgPool,
        owner: ProfileId,
        terms: Vec<(BoundTerm, i64)>,
    ) -> Vec<Uuid> {
        let v = find_exact_paged("kestrel", terms);
        let rows = execute(pool, &compile(&v, owner).expect("compiles"))
            .await
            .expect("runs");
        rows.hits_for("a").into_iter().map(|h| h.id).collect()
    }

    // The denominator: without a limit the stage sees all three, so a short page below is the
    // limit biting rather than the corpus being small.
    assert_eq!(
        page(&pool, owner, vec![]).await.len(),
        3,
        "all three match unbounded"
    );

    let one = page(&pool, owner, vec![(BoundTerm::Limit, 1)]).await;
    assert_eq!(
        one.len(),
        1,
        "the caller asked for one row and got: {one:?}"
    );

    let two = page(&pool, owner, vec![(BoundTerm::Limit, 2)]).await;
    assert_eq!(two.len(), 2, "and for two: {two:?}");

    let skipped = page(
        &pool,
        owner,
        vec![(BoundTerm::Limit, 1), (BoundTerm::Offset, 1)],
    )
    .await;
    assert_eq!(skipped.len(), 1, "one row, from further in: {skipped:?}");
    assert_ne!(
        one[0], skipped[0],
        "offset must move the window, not merely be accepted"
    );
    assert_eq!(
        two[1], skipped[0],
        "and it must move it by exactly one — the second row of a two-row page"
    );
}

/// A matched row carries the quantity its act ordered by, and the tally counts it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_matched_row_carries_its_quantity_and_is_counted_by_the_tally(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "tally").await;
    let id = mk(
        &pool,
        home,
        owner,
        emitter,
        "Pipe",
        "composable search fragments",
    )
    .await;

    let rows = execute(
        &pool,
        &compile(&one_find_exact("composable", None), owner).unwrap(),
    )
    .await
    .expect("runs");

    let hits = rows.hits_for("a");
    assert_eq!(hits.len(), 1, "got: {hits:?}");
    assert_eq!(hits[0].id, id);
    assert_eq!(hits[0].kind, "resource", "the currency the stage produced");
    assert!(
        hits[0].quantity.unwrap_or(0.0) > 0.0,
        "a matched row carries the act's own quantity: {hits:?}"
    );
    assert_eq!(rows.tally("a").expect("tallied").produced, 1);
}

/// **An honest zero is a different answer from an absent one**, and the tally is where they part.
///
/// With no hit rows, the tally is the only evidence the stage ran — which is what lets an assembler
/// say `empty` ("asked, no match") instead of having to guess between that and a stage that never
/// executed.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_stage_that_matched_nothing_is_tallied_zero_rather_than_going_unreported(
    pool: sqlx::PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, _) = system_actor(&pool).await;

    let rows = execute(
        &pool,
        &compile(&one_find_exact("nothing matches this", None), owner).unwrap(),
    )
    .await
    .expect("runs");

    let tally = rows
        .tally("a")
        .expect("every stage is tallied, matched or not");
    assert_eq!(tally.produced, 0);
    assert_eq!(tally.unusable, 0, "no ids were supplied to be unusable");
    assert!(rows.hits_for("a").is_empty());
}

/// `input_unusable` — invisible, nonexistent and malformed as ONE number, and nothing about WHICH
/// or WHY. Naming the invisible case alone would make the trace a single-probe existence oracle.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_id_the_principal_cannot_see_is_counted_unusable_without_saying_why(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let mine = ctx(&pool, owner, "mine").await;
    let visible = mk(&pool, mine, owner, emitter, "Mine", "composable fragments").await;

    let stranger = ProfileId::from(common::insert_profile(&pool, "unusable-stranger").await);
    let theirs = ctx(&pool, stranger, "theirs").await;
    let hidden = mk(
        &pool,
        theirs,
        stranger,
        emitter,
        "Theirs",
        "composable fragments",
    )
    .await;

    let plan = one_find_exact("composable", Some(vec![visible, hidden]));
    let rows = execute(&pool, &compile(&plan, owner).unwrap())
        .await
        .expect("runs");

    let tally = rows.tally("a").expect("tallied");
    assert_eq!(
        tally.unusable, 1,
        "one of the two supplied ids is not this principal's to use"
    );
    assert_eq!(tally.produced, 1, "and only the visible one was returned");
    let hits = rows.hits_for("a");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, visible, "the hidden id must not leak as a row");
}

/// **The one direction this counter must never fail in**, and it is not hypothetical.
///
/// The visible set arrives as `array_agg(...)`, which yields NULL over zero rows. Compared against a
/// bare NULL, `NOT (id = ANY(NULL))` is NULL for every id and `count(*)` over a NULL predicate counts
/// none — so the principal who can use NOTHING would be told nothing was unusable, the most
/// confidently wrong answer available. `COALESCE(..., '{}')` is what makes the comparison false
/// rather than unknown.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_principal_who_can_see_nothing_finds_every_supplied_id_unusable(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "coalesce").await;
    let a = mk(&pool, home, owner, emitter, "One", "composable fragments").await;
    let b = mk(&pool, home, owner, emitter, "Two", "composable fragments").await;

    // No context, no team, no grant: `resources_visible_to` returns zero rows for this principal.
    let outsider = ProfileId::from(common::insert_profile(&pool, "coalesce-outsider").await);
    let rows = execute(
        &pool,
        &compile(&one_find_exact("composable", Some(vec![a, b])), outsider).unwrap(),
    )
    .await
    .expect("runs");

    let tally = rows.tally("a").expect("tallied");
    assert_eq!(
        tally.unusable, 2,
        "an empty visible set makes every supplied id unusable, never zero of them"
    );
    assert_eq!(tally.produced, 0);
    assert!(rows.hits_for("a").is_empty(), "and nothing comes back");
}

/// **A stage may be named after a SQL reserved word, and the emitted statement must still parse.**
///
/// `StageName::parse` admits `[a-z][a-z0-9_]{0,62}`, which includes `both`, `all`, `order`, `end`
/// and every other reserved word in lower case. Emitted unquoted, each is a syntax error the caller
/// sees as a 500 — a well-formed composition, refused by nothing, failing at the database for a
/// reason nothing in the contract predicts. Found in review, as the incidental cause of an unrelated
/// probe failing.
///
/// Quoting closes the whole class rather than one word, and it is safe precisely because the
/// parse-only constructor already guarantees the shape: no quote, no dot, no case to fold.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_stage_named_after_a_reserved_word_still_compiles_to_valid_sql(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "reserved").await;
    mk(&pool, home, owner, emitter, "Kestrel", "the kestrel hovers").await;

    // TWO stages, with the reserved word UPSTREAM. A single-stage plan quotes the CTE definition
    // and nothing else, so it cannot see an unquoted REFERENCE — which is exactly what this test
    // missed on its first pass: `narrowing_for` still emitted a bare `ARRAY(SELECT id FROM both)`
    // and the single-stage version passed anyway.
    for reserved in ["both", "all", "order", "end", "table"] {
        let up = StageName::parse(reserved).unwrap();
        let down = StageName::parse("downstream").unwrap();
        let stage = |n: &StageName, input: Option<StageInput>| {
            StageNode::Act(ActInvocation {
                name: n.clone(),
                act: ActName::FindExact,
                intention: Some(Intention {
                    query: "kestrel".to_string(),
                    embedding: None,
                }),
                inputs: input.into_iter().collect(),
                terms: Default::default(),
                resource_filter: None,
                edge_filter: None,
                properties: vec![],
            })
        };
        let c = Composition {
            outcome: OutcomeDeclaration {
                returns: vec![ReturnSpec {
                    stage: down.clone(),
                    with: vec![],
                }],
            },
            stages: vec![
                stage(&up, None),
                stage(
                    &down,
                    Some(StageInput::Upstream {
                        relation: StageRelation::Bound,
                        stage: up.clone(),
                    }),
                ),
            ],
        };
        let v = validate(&c).expect("a reserved word is a legal stage name");
        let compiled = compile(&v, owner).expect("compiles");
        let rows = execute(&pool, &compiled)
            .await
            .unwrap_or_else(|e| panic!("stage named `{reserved}` failed to run: {e}"));
        assert_eq!(
            rows.hits_for("downstream").len(),
            1,
            "and it must return the match through the reserved-word stage, not merely parse"
        );
    }
}

// ─── Set combinators, which had no witness at any layer below temper-core ───────────────────────

/// **`intersect` across two acts must be the true intersection.**
///
/// It was always EMPTY. `emit_combine_body` selected `id, kind, quantity`, and `INTERSECT` compares
/// whole rows — so a resource found by two acts carried two different scores and was two different
/// rows. `union` double-counted it for the same reason.
///
/// Nothing caught it because no test below `temper-core` had ever constructed a `StageNode::Combine`
/// — a mutation to `emit_combine_body` up to and including returning the empty string would have
/// survived vacuously. Found in review.
///
/// **This reads the combinator's TALLY, not its rows**, because a combinator may no longer be a
/// returned stage (its rows have no single act to score them). The tally is the only observation of
/// a combinator this surface offers, which makes it the whole witness.
///
/// **THIS TEST DOES NOT CATCH THE WHOLE-ROW BUG, and saying so is the point.** It was written
/// believing that bounding one stage would change its `fts_norm` — it does not: `ts_rank` is a
/// property of the document and the query, not of the candidate set, so both stages score every
/// resource identically and the whole-row comparison still matches. Probed, and it did not fire.
///
/// What it IS: the first execution of a `StageNode::Combine` anywhere below `temper-core`, which
/// had zero coverage at any layer. It witnesses that a combinator compiles to valid SQL, runs, and
/// tallies its true set size. The whole-row defect itself is witnessed at the emission level by
/// `a_combinator_projects_membership_only_and_never_a_quantity`, which fires.
///
/// An end-to-end witness needs two stages whose scores genuinely differ for one resource — i.e. a
/// `find-exact` beside a `find-about-*` — and that needs embedded chunk fixtures. Named as a
/// remainder rather than faked with a test that cannot fail.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn intersect_across_stages_returns_the_true_intersection_not_the_empty_set(
    pool: sqlx::PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "combine").await;
    let kestrel = mk(&pool, home, owner, emitter, "Kestrel", "the kestrel hovers").await;
    let merlin = mk(
        &pool,
        home,
        owner,
        emitter,
        "Merlin",
        "the kestrel and the merlin",
    )
    .await;

    let all = StageName::parse("all_hits").unwrap();
    let just_one = StageName::parse("just_one").unwrap();
    let merged = StageName::parse("merged").unwrap();

    let find = |n: &StageName, input: Option<StageInput>| {
        StageNode::Act(ActInvocation {
            name: n.clone(),
            act: ActName::FindExact,
            intention: Some(Intention {
                query: "kestrel".to_string(),
                embedding: None,
            }),
            inputs: input.into_iter().collect(),
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    };
    let compose = |op: temper_core::types::query::CombineOp| Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: all.clone(),
                with: vec![],
            }],
        },
        stages: vec![
            find(&all, None),
            // Bounded to Kestrel alone, so this stage produces a strict subset of `all_hits`.
            find(
                &just_one,
                Some(StageInput::Caller {
                    relation: StageRelation::Bound,
                    ids: IdSet {
                        kind: IdKind::Resource,
                        provenance: None,
                        ids: vec![kestrel],
                    },
                }),
            ),
            StageNode::Combine(temper_core::types::query::CombineNode {
                name: merged.clone(),
                op,
                inputs: vec![all.clone(), just_one.clone()],
            }),
        ],
    };

    // Precondition: the two stages genuinely differ, or neither assertion below means anything.
    let v = validate(&compose(temper_core::types::query::CombineOp::Intersect)).expect("valid");
    let rows = execute(&pool, &compile(&v, owner).expect("compiles"))
        .await
        .expect("runs");
    assert_eq!(rows.tally("all_hits").unwrap().produced, 2, "both match");
    assert_eq!(rows.tally("just_one").unwrap().produced, 1);
    assert_eq!(
        rows.tally("merged").unwrap().produced,
        1,
        "the intersection is Kestrel — an empty result here is the whole-row comparison bug"
    );

    let v = validate(&compose(temper_core::types::query::CombineOp::Union)).expect("valid");
    let rows = execute(&pool, &compile(&v, owner).expect("compiles"))
        .await
        .expect("runs");
    assert_eq!(
        rows.tally("merged").unwrap().produced,
        2,
        "the union is both resources — 3 would mean one resource counted twice under two scores"
    );
    let _ = merlin;
}

/// **`difference` subtracts, and it subtracts in the order the caller declared.**
///
/// The asymmetry is the whole assertion, and it is why this needs a database rather than a text
/// check: `A EXCEPT B` and `B EXCEPT A` are the same emitted shape with the arms swapped, so
/// nothing about the SQL string distinguishes a correct implementation from one that folds the
/// inputs as a set. Only running both orders against the same corpus does.
///
/// Reads the TALLY, for the same reason its `intersect` sibling above does: a combinator may not be
/// a returned stage, so its tally is the only observation of it this surface offers.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn difference_subtracts_and_swapping_the_arms_changes_the_answer(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "subtract").await;
    let kestrel = mk(&pool, home, owner, emitter, "Kestrel", "the kestrel hovers").await;
    let _merlin = mk(
        &pool,
        home,
        owner,
        emitter,
        "Merlin",
        "the kestrel and the merlin",
    )
    .await;

    let all = StageName::parse("all_hits").unwrap();
    let just_one = StageName::parse("just_one").unwrap();
    let gap = StageName::parse("gap").unwrap();

    let find = |n: &StageName, input: Option<StageInput>| {
        StageNode::Act(ActInvocation {
            name: n.clone(),
            act: ActName::FindExact,
            intention: Some(Intention {
                query: "kestrel".to_string(),
                embedding: None,
            }),
            inputs: input.into_iter().collect(),
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    };
    // `just_one` is bounded to Kestrel, so it is a strict SUBSET of `all_hits` — which is what
    // makes the two orders give different answers rather than merely different rows.
    let compose = |minuend: &StageName, subtrahend: &StageName| Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: all.clone(),
                with: vec![],
            }],
        },
        stages: vec![
            find(&all, None),
            find(
                &just_one,
                Some(StageInput::Caller {
                    relation: StageRelation::Bound,
                    ids: IdSet {
                        kind: IdKind::Resource,
                        provenance: None,
                        ids: vec![kestrel],
                    },
                }),
            ),
            StageNode::Combine(temper_core::types::query::CombineNode {
                name: gap.clone(),
                op: temper_core::types::query::CombineOp::Difference,
                inputs: vec![minuend.clone(), subtrahend.clone()],
            }),
        ],
    };

    let run = |c: Composition| {
        let pool = pool.clone();
        async move {
            let v = validate(&c).expect("valid");
            execute(&pool, &compile(&v, owner).expect("compiles"))
                .await
                .expect("runs")
        }
    };

    let rows = run(compose(&all, &just_one)).await;
    // The denominator: without these, "1" below could be produced by a subtraction that did
    // nothing at all against a one-row corpus.
    assert_eq!(rows.tally("all_hits").unwrap().produced, 2, "both match");
    assert_eq!(rows.tally("just_one").unwrap().produced, 1);
    assert_eq!(
        rows.tally("gap").unwrap().produced,
        1,
        "all_hits - just_one is Merlin; 2 would mean nothing was subtracted"
    );

    let rows = run(compose(&just_one, &all)).await;
    assert_eq!(
        rows.tally("gap").unwrap().produced,
        0,
        "just_one - all_hits is empty — a difference that folded its inputs as a SET would answer \
         1 here, identically to the order above"
    );
}

// ─── The two properties that were asserted only as SQL TEXT ─────────────────────────────────────
//
// Both live in `query_plan_compile.rs` as claims about the emitted string. Neither had ever been
// handed to Postgres, which is the same gap this whole file was opened for: a text assertion
// cannot see what the database does with the text.

/// `wide` = `find-about-anywhere`; `narrowed` = `find-exact` bound to it.
///
/// With `bind_to_wide` false the `wide` stage is omitted entirely and `narrowed` runs as a ROOT —
/// **the same stage, same act, same threaded question, unbounded.** That is the denominator, and
/// it is built from this one function rather than a second hand-written plan so the two cannot
/// drift into being different questions.
fn wide_then_narrowed(query: &str, bind_to_wide: bool) -> ValidatedComposition {
    let wide = StageName::parse("wide").unwrap();
    let narrowed = StageName::parse("narrowed").unwrap();

    let mut stages = Vec::new();
    if bind_to_wide {
        stages.push(StageNode::Act(ActInvocation {
            name: wide.clone(),
            act: ActName::FindAboutAnywhere,
            // No vector, deliberately — the caller of this helper asserts the wide stage refuses
            // `EmbeddingUnavailable` at compile and emits `refused_body`.
            intention: Some(Intention {
                query: query.to_string(),
                embedding: None,
            }),
            inputs: vec![],
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        }));
    }
    stages.push(StageNode::Act(ActInvocation {
        name: narrowed.clone(),
        act: ActName::FindExact,
        intention: Some(Intention {
            query: query.to_string(),
            embedding: None,
        }),
        inputs: bind_to_wide
            .then(|| StageInput::Upstream {
                relation: StageRelation::Bound,
                stage: wide.clone(),
            })
            .into_iter()
            .collect(),
        terms: Default::default(),
        resource_filter: None,
        edge_filter: None,
        properties: vec![],
    }));

    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: narrowed,
                with: vec![],
            }],
        },
        stages,
    };
    validate(&c).expect("plan is valid")
}

/// **A refused stage's downstream receives an EMPTY set, not an absent one — run, not read.**
///
/// `a_stage_downstream_of_a_refusal_is_bounded_to_nothing_rather_than_unbounded` asserts this over
/// the emitted text: `ARRAY(SELECT id FROM "wide")` is present and `NULL::uuid[]` is not. What no
/// test anywhere did was hand that statement to Postgres and look at the rows.
///
/// **The text test is BLIND to the defect, and this was measured, not supposed.** Wrapping the
/// upstream bound as `NULLIF(ARRAY(SELECT id FROM "wide"), '{}')` — the empty-into-absent collapse
/// in one plausible line — keeps both of its substring assertions true: the `ARRAY(...)` it looks
/// for is still there and the `NULL::uuid[]` it forbids never appears. Probed: the whole compile
/// suite stayed green on that property and this test fired, returning the ENTIRE corpus through a
/// stage whose upstream had refused. That is the shape the failure takes — a different question,
/// answered confidently, with a full page of plausible results and nothing marking it wrong.
///
/// **The denominator is the whole test.** Zero rows out of an empty corpus proves nothing, so the
/// same `narrowed` stage is run a second time as a root — unbounded, same question, same corpus —
/// and must return rows. Without that half, deleting the `find-exact` fragment entirely would leave
/// this green.
///
/// The refusal itself is asserted too, so the zero cannot be explained by the wide stage merely
/// having matched nothing: a refused stage and an honest-empty one are byte-identical in the row
/// set (`produced = 0, unusable = 0`), and only `refusals` tells them apart.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_stage_bound_to_a_refused_stage_returns_nothing_when_actually_executed(
    pool: sqlx::PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "refused-bound").await;
    for (title, body) in [
        ("Kestrel", "the kestrel hovers over the verge"),
        ("Merlin", "the merlin hunts the kestrel's ground"),
        ("Hobby", "the hobby takes a kestrel's airspace"),
    ] {
        mk(&pool, home, owner, emitter, title, body).await;
    }

    // No embedding, so the wide stage refuses at compile and emits `refused_body`.
    let bound = compile(&wide_then_narrowed("kestrel", true), owner)
        .expect("a runtime refusal does not abort the plan");
    let rows = execute(&pool, &bound)
        .await
        .expect("a refused stage still produces a statement that runs");

    let refusal = rows
        .refusal("wide")
        .expect("the wide stage refused, and the trace must be able to say so");
    assert_eq!(
        refusal.reason,
        RefusalReason::EmbeddingUnavailable,
        "the one runtime refusal; got {refusal:?}"
    );
    assert_eq!(
        rows.tally("wide").expect("tallied even so").produced,
        0,
        "a refused stage produces no rows"
    );

    assert!(
        rows.hits_for("narrowed").is_empty(),
        "bounded to an empty set is bounded to NOTHING; anything here is the refusal having been \
         read as unbounded, which is a global search wearing the answer's clothes: {:?}",
        rows.hits_for("narrowed")
    );
    assert_eq!(
        rows.tally("narrowed").expect("tallied").produced,
        0,
        "and the tally agrees with the rows"
    );

    // ── The denominator ──
    // The same stage, unbounded, over the same corpus. If this returned nothing the assertions
    // above would be vacuous — they would hold against a corpus with nothing to find.
    let unbound = compile(&wide_then_narrowed("kestrel", false), owner).expect("compiles");
    let rows = execute(&pool, &unbound).await.expect("runs");
    assert!(
        rows.refusals.is_empty(),
        "with no wide stage there is nothing to refuse; got {:?}",
        rows.refusals
    );
    assert_eq!(
        rows.hits_for("narrowed").len(),
        3,
        "the corpus DOES hold rows this exact query matches — which is what makes the empty result \
         above a consequence of the bound rather than of there being nothing to find"
    );
}

/// One `find-about-anywhere` stage, returned. The wide arm's 10-slot call, and the only
/// composition in this file that reaches it.
fn one_find_about_anywhere(
    query: &str,
    // `[2026-08-12]` The vector used to be `compile`'s third argument; spec ⟨7⟩ put it on the
    // stage's intention, so the plan builder is where it enters.
    embedding: Option<Vec<f32>>,
    terms: Vec<(BoundTerm, i64)>,
) -> ValidatedComposition {
    let name = StageName::parse("wide").unwrap();
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: name.clone(),
                with: vec![],
            }],
        },
        stages: vec![StageNode::Act(ActInvocation {
            name,
            act: ActName::FindAboutAnywhere,
            intention: Some(Intention {
                query: query.to_string(),
                embedding,
            }),
            inputs: vec![],
            terms: terms.into_iter().collect(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })],
    };
    validate(&c).expect("plan is valid")
}

/// **The wide core's 10-slot call, executed — the only test anywhere that runs it.**
///
/// `emit_ungated_core_call` emits ten arguments for this arm (`{VISIBLE_IDS}`, `$e::vector`,
/// `$k::int`, bound, anchor_table, anchor_id, `{PRINCIPAL_BIND}`, doc_type, limit, offset) against
/// `__temper_ungated_find_wide`'s ten parameters. **Every other `#[sqlx::test]` on this path uses
/// `find-exact`**, whose arm takes a text intent argument instead of the vector/k pair — so the
/// wide call's arity, its casts and its slot order were decided by nothing but the text assertions
/// in `query_plan_compile.rs`, which cannot see a signature.
///
/// **What is uniquely this test's to catch is what the WIDE ARM alone contributes**, and the
/// distinction was measured rather than assumed. `emit_ungated_core_call` is shared with
/// `find-exact`, so the slots it fixes — including a `p_visible_ids`/`p_bound_ids` transposition,
/// the two `uuid[]` parameters with opposite NULL semantics — are already witnessed: probed by
/// swapping them outright, and seven of this file's `find-exact` tests fire alongside this one.
/// Believing otherwise (that the exact arm's `text` argument between its two `uuid[]` slots makes
/// the same swap a type error) is wrong — `(uuid[], text, uuid[])` transposed is still type-valid,
/// and it returns wrong rows rather than an error.
///
/// What NOTHING else executes:
///
/// * **The `$e::vector, $k::int` pair and the two binds behind it.** Transposing the two indices —
///   `${ki}::vector, ${ei}::int`, a one-character slip — fails with `cannot cast type bigint to
///   vector`, and this test is the ONLY one in the file that fires. Probed.
/// * **The resulting 10-argument arity, and the `vec_norm` projection over it.** The exact arm
///   emits nine and reads `fts_norm`; a wide call that named the wrong column or the wrong count
///   would be a `function does not exist` at the door and nowhere before it. This is the same
///   class as the `resources_visible_to`-column defect that opened this file.
/// * **Ordering being real.** `vec_norm` is `shrunk_best_of_n` over cosine DISTANCE, rescaled so
///   higher is closer. Three resources at cosine 1.0 / 0.5 / 0.0 to the query vector must come
///   back in exactly that order — the vector genuinely reached the similarity, rather than the
///   arm returning whatever the top-k draw happened to hold.
///
/// **The `limit` half needs the unlimited half as its denominator, and that is not a formality.**
/// The unscoped wide branch applies no distance threshold: every resource in the top-k draw is
/// returned, `Far` included. So "the far one does not come back" is only ever true of a *page*,
/// and asserting it without first showing that `Far` DOES come back unlimited would be asserting
/// that a small corpus is small.
///
/// A caller-supplied vector needs no ONNX and no `test-embed`: the floats are handed to `compile`
/// and bound as pgvector text by the executor. Nothing here computes an embedding.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_wide_arms_ten_slot_call_executes_and_the_query_vector_reaches_its_slot(
    pool: sqlx::PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "wide-slots").await);

    // Cosine 1.0, 0.5 and 0.0 to `unit(0)` — the query vector below. Orthogonal is as far as a
    // unit vector gets, so "near" and "far" need no embedding model to be unambiguous.
    let near = common::mk_embedded(&pool, home, owner, emitter, "Near", common::unit(0)).await;
    let mid = common::mk_embedded(&pool, home, owner, emitter, "Mid", common::at_cos(0.5)).await;
    let far = common::mk_embedded(&pool, home, owner, emitter, "Far", common::unit(1)).await;

    // The words are deliberately unrelated to any body: the wide arm searches the VECTOR, and an
    // intention that also matched textually would let a mis-wired call look right for the wrong
    // reason. `find-about-anywhere` never reads the query text.
    let query_vec = common::unit(0);
    let plan = one_find_about_anywhere("something I cannot spell", Some(query_vec.clone()), vec![]);
    let compiled = compile(&plan, owner).expect("compiles");
    assert!(
        compiled.refusals.is_empty(),
        "an embedding was supplied, so nothing may refuse; got {:?}",
        compiled.refusals
    );

    let rows = execute(&pool, &compiled)
        .await
        .expect("the wide arm's 10-argument call must run");

    let hits = rows.hits_for("wide");
    let got: Vec<Uuid> = hits.iter().map(|h| h.id).collect();
    assert_eq!(
        got,
        vec![near, mid, far],
        "all three are drawn, ordered by closeness to the query vector — near, mid, far. A \
         different order means the vector did not reach the similarity; nothing at all means the \
         visible set did not reach `p_visible_ids`. Got: {hits:?}"
    );
    assert_eq!(
        rows.tally("wide").expect("tallied").produced,
        3,
        "and the tally counts what the arm produced"
    );

    // Quantities strictly decreasing, so the order above is the arm's score rather than an
    // incidental row order the executor's sort left alone.
    let q: Vec<f64> = hits.iter().map(|h| h.quantity.expect("scored")).collect();
    assert!(
        q[0] > q[1] && q[1] > q[2],
        "vec_norm is better-when-higher and these sit at cosine 1.0 / 0.5 / 0.0: {q:?}"
    );

    // ── The page ──
    // `Far` came back above, so its ABSENCE here is the limit biting on a real ordering rather
    // than a claim about a corpus that never held it.
    let paged = one_find_about_anywhere(
        "something I cannot spell",
        Some(common::unit(0)),
        vec![(BoundTerm::Limit, 1)],
    );
    let rows = execute(&pool, &compile(&paged, owner).expect("compiles"))
        .await
        .expect("runs");
    let hits = rows.hits_for("wide");
    assert_eq!(hits.len(), 1, "one row was asked for; got {hits:?}");
    assert_eq!(
        hits[0].id, near,
        "and it is the NEAREST — the limit truncates beneath the arm's own ORDER BY, so the row \
         that survives is the best one rather than whichever the draw returned first"
    );
}

// ─── The walk, executed ─────────────────────────────────────────────────────────────────────────

/// **The layer the compile suite cannot reach, and the one the wire gap hid in.**
///
/// `[added — 2026-08-14]` `follow-from`'s bound was shipped in `20260814000030` with a witness at
/// the SQL level and none through a composition — and it was exactly at the composition layer that
/// `ActInvocation.input` turned out to carry one set. So this is not a duplicate of
/// `search_graph_expand.rs`'s bound test: that one calls the fragment, this one asks whether a
/// PLAN can say it.
///
/// `a — b — c`, bound to `{a, c}`. `b` is not walkable, so `c` is unreachable and does not return —
/// the interior reading. Under the refused output-only reading `c` would come back, having been
/// walked straight through a node the caller excluded.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_bounded_walk_through_a_composition_constrains_intermediate_nodes(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "walkexec").await;
    let a = mk(&pool, home, owner, emitter, "wa", "alpha").await;
    let b = mk(&pool, home, owner, emitter, "wb", "beta").await;
    let c = mk(&pool, home, owner, emitter, "wc", "gamma").await;
    edge(&pool, a, b, home, emitter).await;
    edge(&pool, b, c, home, emitter).await;

    let unbounded = walk_ids(&pool, owner, vec![a], None).await;
    assert!(
        unbounded.contains(&c),
        "control: an unbounded walk reaches c at hop 2 through b; got {unbounded:?}"
    );

    let bounded = walk_ids(&pool, owner, vec![a], Some(vec![a, c])).await;
    assert!(
        !bounded.contains(&c),
        "a bound constrains INTERMEDIATE nodes: b is excluded, so the a-b-c path cannot be walked. \
         Returning c here is the output-only reading, which is CombineOp::Intersect and was \
         refused. got {bounded:?}"
    );
}

/// The `via` column survives the whole path — fragment, stage contract, executor row.
///
/// The compile suite asserts the column is EMITTED; only this can say it comes back with content.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_walk_returns_its_provenance_through_the_executor(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "viaexec").await;
    let a = mk(&pool, home, owner, emitter, "va", "alpha").await;
    let b = mk(&pool, home, owner, emitter, "vb", "beta").await;
    edge(&pool, a, b, home, emitter).await;

    let compiled = compile(&walk_plan(vec![a], None), owner).unwrap();
    let rows = execute(&pool, &compiled).await.unwrap();
    let hit = rows
        .hits
        .iter()
        .find(|h| h.id == b)
        .expect("b is a's neighbour");

    let via = hit.via.as_ref().expect("a walk row carries provenance");
    let entries = via.as_array().expect("via is a jsonb array");
    assert_eq!(entries.len(), 1, "one edge reached b; got {via}");
    assert_eq!(
        entries[0]["seed_id"]
            .as_str()
            .unwrap()
            .parse::<Uuid>()
            .unwrap(),
        a,
        "the entry names the seed this node descends from; got {via}"
    );

    // And a NON-walk stage's rows carry no provenance — the column is NULL for every other act,
    // which is what keeps the shared column list honest rather than merely wide.
    let find = compile(&one_find_exact("alpha", None), owner).unwrap();
    let find_rows = execute(&pool, &find).await.unwrap();
    assert!(
        find_rows.hits.iter().all(|h| h.via.is_none()),
        "find-exact does not walk, so it discloses no origin"
    );
}

/// One `leads_to` edge, weight 1.0.
async fn edge(pool: &sqlx::PgPool, src: Uuid, tgt: Uuid, home: ContextId, emitter: EntityId) {
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

/// A one-stage `follow-from` plan: seeds always, a bound when one is given.
fn walk_plan(seeds: Vec<Uuid>, bound: Option<Vec<Uuid>>) -> ValidatedComposition {
    let set = |kind_ids: Vec<Uuid>, relation: StageRelation| StageInput::Caller {
        relation,
        ids: IdSet {
            kind: IdKind::Resource,
            provenance: None,
            ids: kind_ids,
        },
    };
    let mut inputs = vec![set(seeds, StageRelation::Seed)];
    if let Some(b) = bound {
        inputs.push(set(b, StageRelation::Bound));
    }
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: StageName::parse("near").unwrap(),
                with: vec![],
            }],
        },
        stages: vec![StageNode::Act(ActInvocation {
            name: StageName::parse("near").unwrap(),
            act: ActName::FollowFrom,
            intention: None,
            inputs,
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })],
    };
    validate(&c).expect("a seeded walk is well-formed")
}

async fn walk_ids(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    seeds: Vec<Uuid>,
    bound: Option<Vec<Uuid>>,
) -> Vec<Uuid> {
    let compiled = compile(&walk_plan(seeds, bound), owner).unwrap();
    execute(pool, &compiled)
        .await
        .unwrap()
        .hits
        .iter()
        .map(|h| h.id)
        .collect()
}
