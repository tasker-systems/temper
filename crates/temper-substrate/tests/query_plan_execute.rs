#![cfg(feature = "artifact-tests")]
//! **The compiled statement is run against a real database.**
//!
//! Everything in `query_plan_compile.rs` is decidable from the emitted text, and that is its
//! strength — but it cannot see whether the text is valid SQL. This file exists because it wasn't:
//! the hoisted visibility CTE read `SELECT id FROM resources_visible_to($1)`, and that function
//! returns a column called `resource_id`. Invalid since PR #663, caught by nothing, because no act
//! body referenced the CTE and no test ever handed the statement to Postgres.
//!
//! There is still **no executor** — nothing maps a result set into a response shape, and these tests
//! bind and run the statement by hand. What they witness is narrower and load-bearing:
//!
//! 1. The emitted SQL parses, plans and executes.
//! 2. **The hoisted relation is the only thing gating it.** Since `20260808000030` every act stage
//!    calls an ungated core, so if `__temper_vis` were wrong — mis-spelled column, wrong principal,
//!    an `array_agg` that swallowed the filter — a composition would return rows its principal
//!    cannot see and every text-level assertion would stay green.

mod common;

use sqlx::Row;
use temper_core::types::query::{
    validate, ActInvocation, ActName, Composition, Intention, OutcomeDeclaration, ReturnSpec,
    StageInput, StageName, StageNode, StageRelation, ValidatedComposition,
};
use temper_substrate::ids::{ContextId, EntityId, ProfileId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::readback::query_plan::{compile, CompiledQuery, QueryBind};
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

/// Bind a [`CompiledQuery`] positionally and run it, returning `(id, stage)` per row.
///
/// Binds are applied in `binds` order, which is the order the compiler emitted `$1, $2, …` in — the
/// contract `CompiledQuery` publishes. Getting that wrong is not silent: Postgres rejects the type.
async fn run(pool: &sqlx::PgPool, c: &CompiledQuery) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    let mut q = sqlx::query(&c.sql);
    for b in &c.binds {
        q = match b {
            QueryBind::Profile(p) => q.bind(p.uuid()),
            QueryBind::Uuids(v) => q.bind(v.clone()),
            QueryBind::Text(t) => q.bind(t.clone()),
            QueryBind::Int(i) => q.bind(*i),
            QueryBind::Embedding(v) => q.bind(format!(
                "[{}]",
                v.iter().map(f32::to_string).collect::<Vec<_>>().join(",")
            )),
        };
    }
    Ok(q.fetch_all(pool)
        .await?
        .iter()
        .map(|r| (r.get::<Uuid, _>("id"), r.get::<String, _>("stage")))
        .collect())
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
