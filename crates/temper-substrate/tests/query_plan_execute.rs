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
    validate, ActInvocation, ActName, Composition, IdKind, IdSet, Intention, OutcomeDeclaration,
    ReturnSpec, StageInput, StageName, StageNode, StageRelation, ValidatedComposition,
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
            input,
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
        intention: Some(Intention {
            query: query.to_string(),
            embedded: false,
        }),
        meta_detail: Default::default(),
        bounds: Default::default(),
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

    let c = compile(&two_stage_find("kestrel"), owner, None).expect("compiles");
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

    let mine = run(&pool, &compile(&plan, owner, None).expect("compiles"))
        .await
        .expect("runs");
    assert_eq!(
        mine.len(),
        1,
        "precondition: the owner sees their own resource, or the contrast below proves nothing"
    );

    let stranger = ProfileId::from(common::insert_profile(&pool, "exec-stranger").await);
    let theirs = run(&pool, &compile(&plan, stranger, None).expect("compiles"))
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
        intention: Some(Intention {
            query: query.to_string(),
            embedded: false,
        }),
        meta_detail: Default::default(),
        bounds: Default::default(),
        stages: vec![StageNode::Act(ActInvocation {
            name,
            act: ActName::FindExact,
            input: bound.map(|ids| StageInput::Caller {
                relation: StageRelation::Bound,
                ids: IdSet {
                    kind: IdKind::Resource,
                    provenance: None,
                    ids,
                },
            }),
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })],
    };
    validate(&c).expect("plan is valid")
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
        &compile(&one_find_exact("composable", None), owner, None).unwrap(),
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
        &compile(&one_find_exact("nothing matches this", None), owner, None).unwrap(),
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
    let rows = execute(&pool, &compile(&plan, owner, None).unwrap())
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
        &compile(
            &one_find_exact("composable", Some(vec![a, b])),
            outsider,
            None,
        )
        .unwrap(),
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
                input,
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
            intention: Some(Intention {
                query: "kestrel".to_string(),
                embedded: false,
            }),
            meta_detail: Default::default(),
            bounds: Default::default(),
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
        let compiled = compile(&v, owner, None).expect("compiles");
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
