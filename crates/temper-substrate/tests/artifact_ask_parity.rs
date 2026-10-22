#![cfg(feature = "test-db")]
//! **The differentials for asking data artifacts by what they are** (spec
//! `temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md` §2.3, §3.6, §5).
//! Migration `20261103000000` restates two plpgsql incumbents set-based and adds an ask matcher;
//! each restatement is held here to the definition it restates.
//!
//! - **Governance.** `__temper_ungated_governed_artifacts(visible, shape)` holds a live artifact
//!   exactly when `_data_artifact_shape_in_force` resolves that artifact to a version of the
//!   shape's lineage (its home and family), and reports that version.
//! - **Shape state.** `__temper_ungated_governed_artifact_states` equals
//!   `_data_artifact_shape_state` row for row, over verdict-ed, stale-verdict and unverdict-ed
//!   artifacts.
//! - **Predicates.** `_temper_artifact_ask_match` equals `temper_core`'s `reference::matches` on
//!   every pair of generated content and generated ask, content that does not satisfy any shape
//!   included.
//! - **The plan.** Over a home that is small beside the rest of the table, the relations reach
//!   `kb_data_artifacts` through an index and never by a sequential scan.
//!
//! ## The governance world, and which witness each conjunct has
//!
//! Two context homes, A and B, owned by the system profile P; two families, `measurement` and
//! `run`, owned by P; a shape for each family in each home, and A's `measurement` shape amended
//! once. Beside them, in A, three more families share the bare kind `measurement`, each with a
//! shape and an artifact of its own. Each isolates one owner conjunct — on the artifact
//! (`a.kind_owner_*`) and on the lineage's live row (`live.kind_owner_*`):
//!
//! - `(kb_teams, T, measurement)` — another owner in both columns;
//! - `(kb_teams, P, measurement)` — P's own uuid under the other owner table. Dropping
//!   `a.kind_owner_table` admits its artifact to P's family; dropping `live.kind_owner_table` pairs
//!   P's artifacts with its shape as well as P's;
//! - `(kb_profiles, Q, measurement)` — the same owner table, another owner. Dropping
//!   `a.kind_owner_id` or `live.kind_owner_id` does the same.
//!
//! B's shapes are the two-home witness: dropping the relation's home conjunct admits B's artifacts
//! to A's lineage. A cogmap home C is minted under **A's own uuid**, with a `measurement` shape
//! and an artifact, so the home's table is the only thing telling C from A: dropping
//! `h.anchor_table` admits C's artifact to A's lineage, and dropping `live.home_anchor_table` pairs
//! A's artifacts with C's shape. One resource is rehomed from A to B after its artifact was
//! committed and verdict-ed under A's shape, and one artifact is folded by a successor that names
//! it.
//!
//! ## The predicate differential's two traps (decision P9)
//!
//! **Numbers.** jsonb numbers are `numeric`, and `serde_json` here is `f64`-backed. The generator
//! therefore writes only integers within ±2^53 and finite decimals with at most six fractional
//! digits, as JSON numbers, and the reference compares them as exact decimals — so `5` and `5.0`
//! are equal on both sides, as in jsonb.
//!
//! **Strings.** jsonb orders strings by the database collation. The database is the oracle: every
//! string the generator can produce, object keys included, is ranked by Postgres as a jsonb string
//! (`dense_rank() OVER (ORDER BY to_jsonb(s))`, the comparison jsonb's own ordering makes), and the
//! comparator handed to the reference orders each pair by those ranks. The test asserts that some
//! ranked pair orders differently bytewise, so the trap is exercised rather than assumed: on a
//! database whose collation is bytewise, it fails and says so. A hand-picked ask, `unit > "B"` over
//! `{"unit": "a"}`, is true bytewise and false under a linguistic collation.
//!
//! **Determinism.** The generator is a splitmix64 stream from [`SEED`]: [`GENERATED_CONTENTS`]
//! contents and [`GENERATED_ASKS`] asks, after the hand-picked ones, so a failure reproduces from
//! the seed alone.

mod common;

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};
use sqlx::PgPool;
use temper_core::types::artifact_ask::reference;
use temper_core::types::query::filter::{OrdOp, PropertyOp};
use temper_core::types::ArtifactPredicate;
use temper_substrate::content::{PreparedBlock, PreparedChunk};
use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::{
    BlockId, ChunkId, CogmapId, ContextId, DataArtifactId, EntityId, ProfileId, ResourceId,
};
use temper_substrate::payloads::{AnchorRef, ArtifactIntent, EnforcementMode, KindOwner};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CommitDataArtifactParams, CreateParams, DeclareShapeParams};
use uuid::Uuid;

// ── Harness ─────────────────────────────────────────────────────────────────────────────────────

const MEASUREMENT: &str = "measurement";
const RUN: &str = "run";

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

async fn context(pool: &PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

/// A resource homed in `home`, created with its embedding deferred so no model is needed.
async fn resource(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: AnchorRef,
    title: &str,
) -> ResourceId {
    let origin = format!("test://artifact-ask/{title}");
    writes::create_resource_deferred_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &origin,
            body: "fixture body",
            doc_type: "research",
            home,
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap()
}

/// The schema every fixture shape declares: an object whose `unit` is a required string.
fn shape_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "unit": { "type": "string" },
            "count": { "type": "integer" }
        },
        "required": ["unit"]
    })
}

fn conforming() -> Value {
    json!({ "unit": "ms", "count": 1 })
}

fn non_conforming() -> Value {
    json!({ "count": "many" })
}

/// An owner-qualified family: `(kind_owner, artifact_kind)`.
type Family = (KindOwner, &'static str);

/// Declare (or amend) the advisory shape for a family in a home.
async fn declare(pool: &PgPool, emitter: EntityId, home: AnchorRef, family: Family) -> Uuid {
    let (owner, kind) = family;
    writes::declare_shape(
        pool,
        DeclareShapeParams {
            home,
            kind,
            kind_owner: Some(owner),
            schema: &shape_schema(),
            enforcement: EnforcementMode::Advisory,
            emitter,
        },
    )
    .await
    .unwrap()
    .uuid()
}

/// Commit one artifact of a family through the real write path, which records a verdict when a
/// shape is in force.
async fn commit(
    pool: &PgPool,
    emitter: EntityId,
    on: ResourceId,
    family: Family,
    content: &Value,
) -> DataArtifactId {
    let (owner, kind) = family;
    writes::commit_data_artifact(
        pool,
        CommitDataArtifactParams {
            resource: on,
            kind,
            kind_owner: Some(owner),
            intent: ArtifactIntent::Member,
            precedence: 0.0,
            content,
            supersedes: &[],
            emitter,
        },
    )
    .await
    .unwrap()
}

/// Every resource id: the visible set the differentials hand to the relations, so that what they
/// compare is governance and nothing narrower.
async fn every_resource(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar!("SELECT id FROM kb_resources")
        .fetch_all(pool)
        .await
        .unwrap()
}

// ── The governance world ────────────────────────────────────────────────────────────────────────

/// The world of the module header, and the artifacts its witnesses are checked against.
struct World {
    visible: Vec<Uuid>,
    owner: ProfileId,
    shape_a_v1: Uuid,
    shape_a_v2: Uuid,
    shape_b: Uuid,
    /// The cogmap home C's `measurement` shape; C has A's uuid.
    shape_c: Uuid,
    /// The shapes of `(kb_teams, P, measurement)` and `(kb_profiles, Q, measurement)` in A.
    shape_other_table: Uuid,
    shape_other_owner: Uuid,
    /// Committed before any shape was declared, so it has no verdict.
    unverdicted: DataArtifactId,
    /// Verdict-ed under A's first version, which the amendment folded.
    version_stale: DataArtifactId,
    /// Verdict-ed satisfied under the version in force, then given a verdict whose content hash is
    /// not the artifact's — stale by the hash alone.
    hash_stale: DataArtifactId,
    satisfied: DataArtifactId,
    not_satisfied: DataArtifactId,
    /// Committed in A under A's shape, its resource then rehomed to B.
    rehomed: DataArtifactId,
    folded: DataArtifactId,
    /// `(kb_teams, P, measurement)`: P's uuid under the other owner table.
    other_owner_table: DataArtifactId,
    /// `(kb_profiles, Q, measurement)`: the same owner table, another owner.
    other_owner_id: DataArtifactId,
    /// P's `measurement` artifact on a resource homed in the cogmap C.
    cogmap_homed: DataArtifactId,
}

/// Mint a cogmap under a chosen id. The world gives it a context's uuid, so that only the anchor
/// table tells the two homes apart.
async fn cogmap_with_id(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    id: Uuid,
    name: &str,
) -> CogmapId {
    let mut embedding = vec![0.0_f32; 768];
    embedding[0] = 1.0;
    let charter = vec![PreparedBlock {
        incorporated: vec![],
        raw_text: None,
        block_id: BlockId::from(Uuid::now_v7()),
        seq: 0,
        role: None,
        chunks: vec![PreparedChunk {
            chunk_id: ChunkId::from(Uuid::now_v7()),
            chunk_index: 0,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            content: "charter statement".to_string(),
            embedding: Some(embedding),
            embedded_with: None,
            header_path: None,
            heading_depth: None,
        }],
    }];
    let mut tx = pool.begin().await.unwrap();
    let fired = fire(
        &mut tx,
        SeedAction::CogmapGenesis {
            name,
            telos_title: "Telos",
            charter: &charter,
            cogmap_id: Some(CogmapId::from(id)),
            telos_resource_id: None,
            owner,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    fired.cogmap_genesis().unwrap().0
}

async fn world(pool: &PgPool) -> World {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let team = common::create_team(pool, "artifact-ask-team").await;
    let stranger = common::create_profile(pool, "artifact-ask-stranger@example.test").await;

    // P's two families, and three more of the bare kind `measurement` under other owners.
    let measured: Family = (KindOwner::Profile(owner.uuid()), MEASUREMENT);
    let runs: Family = (KindOwner::Profile(owner.uuid()), RUN);
    let team_measured: Family = (KindOwner::Team(team), MEASUREMENT);
    let other_table: Family = (KindOwner::Team(owner.uuid()), MEASUREMENT);
    let other_owner: Family = (KindOwner::Profile(stranger), MEASUREMENT);

    let a_id = context(pool, owner, "artifact-ask-a").await;
    let b_id = context(pool, owner, "artifact-ask-b").await;
    let c_id = cogmap_with_id(pool, owner, emitter, a_id.uuid(), "artifact-ask-c").await;
    assert_eq!(c_id.uuid(), a_id.uuid(), "C is minted under A's uuid");
    let (a, b, c) = (
        AnchorRef::context(a_id),
        AnchorRef::context(b_id),
        AnchorRef::cogmap(c_id),
    );
    let r_a1 = resource(pool, owner, emitter, a, "a-one").await;
    let r_a2 = resource(pool, owner, emitter, a, "a-two").await;
    let r_b1 = resource(pool, owner, emitter, b, "b-one").await;
    let r_c1 = resource(pool, owner, emitter, c, "c-one").await;
    let r_move = resource(pool, owner, emitter, a, "moves-to-b").await;

    let unverdicted = commit(pool, emitter, r_a2, measured, &conforming()).await;

    let shape_a_v1 = declare(pool, emitter, a, measured).await;
    let shape_b = declare(pool, emitter, b, measured).await;
    let shape_c = declare(pool, emitter, c, measured).await;
    declare(pool, emitter, a, runs).await;
    declare(pool, emitter, b, runs).await;
    declare(pool, emitter, a, team_measured).await;
    let shape_other_table = declare(pool, emitter, a, other_table).await;
    let shape_other_owner = declare(pool, emitter, a, other_owner).await;

    let version_stale = commit(pool, emitter, r_a1, measured, &conforming()).await;
    let rehomed = commit(pool, emitter, r_move, measured, &conforming()).await;
    commit(pool, emitter, r_a1, runs, &conforming()).await;
    commit(pool, emitter, r_a1, team_measured, &conforming()).await;
    let other_owner_table = commit(pool, emitter, r_a1, other_table, &conforming()).await;
    let other_owner_id = commit(pool, emitter, r_a1, other_owner, &conforming()).await;
    let cogmap_homed = commit(pool, emitter, r_c1, measured, &conforming()).await;
    commit(pool, emitter, r_b1, measured, &conforming()).await;
    commit(pool, emitter, r_b1, runs, &non_conforming()).await;
    let folded = commit(pool, emitter, r_a1, measured, &conforming()).await;
    writes::commit_data_artifact(
        pool,
        CommitDataArtifactParams {
            resource: r_a1,
            kind: MEASUREMENT,
            kind_owner: Some(measured.0),
            intent: ArtifactIntent::Member,
            precedence: 0.0,
            content: &conforming(),
            supersedes: &[folded],
            emitter,
        },
    )
    .await
    .unwrap();

    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceRehome {
            resource: r_move,
            home: b,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let shape_a_v2 = declare(pool, emitter, a, measured).await;

    let satisfied = commit(pool, emitter, r_a1, measured, &conforming()).await;
    let not_satisfied = commit(pool, emitter, r_a1, measured, &non_conforming()).await;
    let hash_stale = commit(pool, emitter, r_a2, measured, &conforming()).await;
    sqlx::query!(
        "SELECT data_artifact_verdict_upsert($1,$2,$3,$4,$5,$6)",
        hash_stale.uuid(),
        shape_a_v2,
        2,
        "0000000000000000000000000000000000000000000000000000000000000000",
        true,
        None::<serde_json::Value>,
    )
    .execute(pool)
    .await
    .unwrap();

    World {
        visible: every_resource(pool).await,
        owner,
        shape_a_v1,
        shape_a_v2,
        shape_b,
        shape_c,
        shape_other_table,
        shape_other_owner,
        unverdicted,
        version_stale,
        hash_stale,
        satisfied,
        not_satisfied,
        rehomed,
        folded,
        other_owner_table,
        other_owner_id,
        cogmap_homed,
    }
}

/// A shape row's lineage: its home and its owner-qualified family.
type Lineage = (String, Uuid, String, Uuid, String);

async fn lineages(pool: &PgPool) -> BTreeMap<Uuid, Lineage> {
    sqlx::query!(
        "SELECT id, home_anchor_table, home_anchor_id, kind_owner_table, kind_owner_id, \
                artifact_kind \
           FROM kb_data_artifact_shapes"
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|s| {
        (
            s.id,
            (
                s.home_anchor_table,
                s.home_anchor_id,
                s.kind_owner_table,
                s.kind_owner_id,
                s.artifact_kind,
            ),
        )
    })
    .collect()
}

/// The incumbent's answer for every live artifact: the shape in force for it, if any.
async fn in_force(pool: &PgPool) -> Vec<(Uuid, Option<(Uuid, i32)>)> {
    sqlx::query!(
        r#"SELECT a.id AS "artifact_id!", f.shape_id, f.shape_version
             FROM kb_data_artifacts a
             LEFT JOIN LATERAL _data_artifact_shape_in_force(
                    a.resource_id, a.kind_owner_table, a.kind_owner_id, a.artifact_kind) AS f
                    ON true
            WHERE NOT a.is_folded"#
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| (r.artifact_id, r.shape_id.zip(r.shape_version)))
    .collect()
}

/// The relation's answer: `(artifact, shape in force, its version)` for every governed row.
async fn governed(pool: &PgPool, visible: &[Uuid], shape: Uuid) -> BTreeSet<(Uuid, Uuid, i32)> {
    sqlx::query!(
        r#"SELECT artifact_id AS "artifact_id!", shape_id AS "shape_id!",
                  shape_version AS "shape_version!"
             FROM __temper_ungated_governed_artifacts($1, $2)"#,
        visible,
        shape,
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| (r.artifact_id, r.shape_id, r.shape_version))
    .collect()
}

async fn governed_ids(pool: &PgPool, visible: &[Uuid], shape: Uuid) -> BTreeSet<Uuid> {
    governed(pool, visible, shape)
        .await
        .into_iter()
        .map(|(artifact, _, _)| artifact)
        .collect()
}

// ── Governance ──────────────────────────────────────────────────────────────────────────────────

/// For every shape row, folded ones included, and every live artifact: the artifact is in the
/// relation for that shape exactly when `_data_artifact_shape_in_force` resolves it to a version of
/// the shape's lineage, and the relation reports the version the incumbent resolves.
///
/// Bites, each one line of `__temper_ungated_governed_artifacts` in `20261103000000`, each turning
/// this test red:
///
/// - drop `a.kind_owner_table = s.kind_owner_table`: `(kb_teams, P, measurement)` joins A's
///   `measurement` population;
/// - drop `h.anchor_id = s.home_anchor_id`: B's artifacts join A's;
/// - drop `live.kind_owner_table = s.kind_owner_table`: A's artifacts are paired with the
///   `(kb_teams, P, measurement)` shape as well as P's;
/// - drop `live.kind_owner_id = s.kind_owner_id`: the same with `(kb_profiles, Q, measurement)`;
/// - drop `h.anchor_table = s.home_anchor_table`: the cogmap C, minted under A's uuid, has its
///   artifact join A's population;
/// - drop `live.home_anchor_table = s.home_anchor_table`: A's artifacts are paired with C's shape.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_governance_relation_is_shape_in_force_set_based(pool: PgPool) {
    let w = world(&pool).await;
    let lineages = lineages(&pool).await;
    let in_force = in_force(&pool).await;

    for (shape, lineage) in &lineages {
        let expected: BTreeSet<(Uuid, Uuid, i32)> = in_force
            .iter()
            .filter_map(|(artifact, resolved)| {
                let (id, version) = (*resolved)?;
                (lineages.get(&id) == Some(lineage)).then_some((*artifact, id, version))
            })
            .collect();
        let actual = governed(&pool, &w.visible, *shape).await;
        assert_eq!(
            actual, expected,
            "shape {shape} {lineage:?}: the set-based relation and _data_artifact_shape_in_force \
             disagree about which artifacts it governs"
        );
    }

    // The fixture is what it claims to be, so the agreement above is about something.
    let a_v1 = governed_ids(&pool, &w.visible, w.shape_a_v1).await;
    let a_v2 = governed_ids(&pool, &w.visible, w.shape_a_v2).await;
    assert_eq!(
        a_v1, a_v2,
        "an amendment moves neither home nor family, so both versions name one population"
    );
    for (artifact, what) in [
        (w.unverdicted, "the artifact committed before any shape"),
        (
            w.version_stale,
            "the artifact verdict-ed under the folded version",
        ),
        (
            w.satisfied,
            "an artifact verdict-ed under the version in force",
        ),
        (
            w.hash_stale,
            "the artifact whose verdict names another hash",
        ),
    ] {
        assert!(
            a_v2.contains(&artifact.uuid()),
            "{what} is governed by A's shape"
        );
    }
    for (artifact, what) in [
        (w.rehomed, "the rehomed artifact"),
        (w.folded, "the folded artifact"),
        (w.other_owner_table, "(kb_teams, P, measurement)"),
        (w.other_owner_id, "(kb_profiles, Q, measurement)"),
    ] {
        assert!(
            !a_v2.contains(&artifact.uuid()),
            "{what} is not governed by A's shape"
        );
    }
    assert!(
        governed_ids(&pool, &w.visible, w.shape_b)
            .await
            .contains(&w.rehomed.uuid()),
        "the rehomed artifact is governed by its new home's shape"
    );
    for (shape, artifact, what) in [
        (w.shape_c, w.cogmap_homed, "the cogmap C, under A's uuid"),
        (
            w.shape_other_table,
            w.other_owner_table,
            "(kb_teams, P, measurement)",
        ),
        (
            w.shape_other_owner,
            w.other_owner_id,
            "(kb_profiles, Q, measurement)",
        ),
    ] {
        assert_eq!(
            governed_ids(&pool, &w.visible, shape).await,
            BTreeSet::from([artifact.uuid()]),
            "{what}: its own shape governs its own artifact and nothing of P's in A"
        );
        assert!(
            !a_v2.contains(&artifact.uuid()),
            "{what}: its artifact is not governed by A's shape"
        );
    }

    // The namespace witness shares P's uuid and differs only in the owner table, and is live in A.
    let witness = sqlx::query!(
        "SELECT kind_owner_table, kind_owner_id, artifact_kind, is_folded \
           FROM kb_data_artifacts WHERE id = $1",
        w.other_owner_table.uuid(),
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (
            witness.kind_owner_table.as_str(),
            witness.kind_owner_id,
            witness.artifact_kind.as_str(),
            witness.is_folded,
        ),
        ("kb_teams", w.owner.uuid(), MEASUREMENT, false),
        "the owner-table witness must differ from P's family in the owner table alone"
    );
}

/// The relations take the caller's verdict and nothing else: a NULL set and an empty set admit
/// nothing, and a set of one resource admits only that resource's artifacts.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_governance_relations_admit_only_the_visible_set(pool: PgPool) {
    let w = world(&pool).await;

    let under_null = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM __temper_ungated_governed_artifacts(NULL::uuid[], $1)"#,
        w.shape_a_v2,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(under_null, 0, "a NULL visible set admits nothing");
    let states_under_null = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!"
             FROM __temper_ungated_governed_artifact_states(NULL::uuid[], $1)"#,
        w.shape_a_v2,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(states_under_null, 0, "a NULL visible set admits nothing");
    assert!(
        governed(&pool, &[], w.shape_a_v2).await.is_empty(),
        "an empty visible set admits nothing"
    );

    let owning = sqlx::query_scalar!(
        "SELECT resource_id FROM kb_data_artifacts WHERE id = $1",
        w.unverdicted.uuid(),
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let narrowed = governed_ids(&pool, &[owning], w.shape_a_v2).await;
    // What the one-resource set should admit, decided without it: the artifacts the table says that
    // resource owns, among those the whole set admits.
    let on_owning: BTreeSet<Uuid> = sqlx::query_scalar!(
        "SELECT id FROM kb_data_artifacts WHERE resource_id = $1",
        owning,
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .collect();
    let owned: BTreeSet<Uuid> = governed_ids(&pool, &w.visible, w.shape_a_v2)
        .await
        .into_iter()
        .filter(|artifact| on_owning.contains(artifact))
        .collect();
    assert!(
        narrowed.contains(&w.unverdicted.uuid()),
        "the visible resource's artifact is admitted"
    );
    assert!(
        !narrowed.contains(&w.satisfied.uuid()),
        "an artifact on a resource outside the set is not"
    );
    assert_eq!(
        narrowed, owned,
        "a one-resource set admits exactly that resource's governed artifacts"
    );
}

// ── Shape state ─────────────────────────────────────────────────────────────────────────────────

/// For every shape row, the set-based state of every governed artifact equals
/// `_data_artifact_shape_state`'s, and the state relation's rows are the governance relation's.
///
/// Bite: drop `AND v.content_hash  = g.content_hash` from
/// `__temper_ungated_governed_artifact_states` in `20261103000000`, and the hash-stale artifact
/// reads `declared_satisfied` against the incumbent's `declared_not_yet_checked`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_shape_state_relation_is_shape_state_set_based(pool: PgPool) {
    let w = world(&pool).await;
    let lineages = lineages(&pool).await;

    let mut seen: BTreeMap<Uuid, String> = BTreeMap::new();
    for shape in lineages.keys() {
        let rows = sqlx::query!(
            r#"SELECT st.artifact_id AS "artifact_id!", st.shape_state AS "shape_state!",
                      _data_artifact_shape_state(st.artifact_id) AS "incumbent!"
                 FROM __temper_ungated_governed_artifact_states($1, $2) AS st"#,
            w.visible.as_slice(),
            *shape,
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        for row in &rows {
            assert_eq!(
                row.shape_state, row.incumbent,
                "shape {shape}, artifact {}: the set-based state and _data_artifact_shape_state \
                 disagree",
                row.artifact_id
            );
            seen.insert(row.artifact_id, row.shape_state.clone());
        }
        let ids: BTreeSet<Uuid> = rows.iter().map(|r| r.artifact_id).collect();
        assert_eq!(
            ids,
            governed_ids(&pool, &w.visible, *shape).await,
            "shape {shape}: the state relation's rows are the governance relation's"
        );
    }

    // Every state is reached, each by the artifact built to reach it.
    for (artifact, state) in [
        (w.satisfied, "declared_satisfied"),
        (w.not_satisfied, "declared_not_satisfied"),
        (w.unverdicted, "declared_not_yet_checked"),
        (w.version_stale, "declared_not_yet_checked"),
        (w.hash_stale, "declared_not_yet_checked"),
        (w.rehomed, "declared_not_yet_checked"),
    ] {
        assert_eq!(
            seen.get(&artifact.uuid()).map(String::as_str),
            Some(state),
            "artifact {artifact:?}"
        );
    }

    // The hash-stale verdict matches the shape in force in id and version and says satisfied, so
    // the content hash is the only leg of the triple that keeps it from being trusted.
    let verdict = sqlx::query!(
        "SELECT v.shape_id, v.shape_version, v.satisfied, \
                v.content_hash = a.content_hash AS \"hash_matches!\" \
           FROM kb_data_artifact_verdicts v \
           JOIN kb_data_artifacts a ON a.id = v.artifact_id \
          WHERE v.artifact_id = $1",
        w.hash_stale.uuid(),
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (
            verdict.shape_id,
            verdict.shape_version,
            verdict.satisfied,
            verdict.hash_matches
        ),
        (w.shape_a_v2, 2, true, false)
    );
    let unverdicted = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM kb_data_artifact_verdicts WHERE artifact_id = $1"#,
        w.unverdicted.uuid(),
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        unverdicted, 0,
        "the unverdict-ed artifact has no verdict row"
    );
}

// ── The plan ────────────────────────────────────────────────────────────────────────────────────

const BACKGROUND_HOMES: i32 = 150;
const BACKGROUND_RESOURCES: i32 = 3_000;
const ARTIFACTS_PER_BACKGROUND_RESOURCE: i32 = 6;

/// Every node of an `EXPLAIN (FORMAT JSON)` plan, the root first.
fn plan_nodes<'p>(node: &'p Value, out: &mut Vec<&'p Map<String, Value>>) {
    if let Some(fields) = node.as_object() {
        out.push(fields);
        for child in fields
            .get("Plans")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            plan_nodes(child, out);
        }
    }
}

/// One small home among 150 larger ones, 18,000 background artifacts of which half are the asked
/// family, and the visible set every resource, bound as a parameter. Asserted, for both relations,
/// on `EXPLAIN (FORMAT JSON)` node types and index names, never timings:
///
/// 1. `kb_data_artifacts` appears in the plan by name, so the relation was inlined — a plan that
///    called it opaquely would show no scan of anything, and pass the next check for the wrong
///    reason;
/// 2. no node is a `Seq Scan` of `kb_data_artifacts`;
/// 3. some node uses `idx_kb_data_artifacts_family` or `idx_kb_resource_homes_anchor`.
///
/// It does not assert which of the two the planner takes, or that the choice is the cheaper one.
/// The home and family come from the shape row inside the statement, so the planner estimates the
/// family join from `n_distinct(kind_owner_id)`. Here the other half of the background has a fresh
/// owner per row, the estimate is about one row, and the planner takes the family index for a
/// family that holds about 9,000. The plan of the composed statement, under its own estimates, is
/// the composing arm's to test.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_relations_start_from_the_home_or_the_family_never_a_scan_of_artifacts(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let measured: Family = (KindOwner::Profile(owner.uuid()), MEASUREMENT);
    let home = AnchorRef::context(context(&pool, owner, "artifact-ask-plan").await);
    let on = resource(&pool, owner, emitter, home, "plan-subject").await;
    let shape = declare(&pool, emitter, home, measured).await;
    for _ in 0..3 {
        commit(&pool, emitter, on, measured, &conforming()).await;
    }

    // Background rows are fixture inserts: plain rows the relations read, at a volume the write
    // path would take minutes to build.
    sqlx::query(
        "WITH homes AS (
             SELECT n, gen_random_uuid() AS anchor_id FROM generate_series(0, $2 - 1) AS n
         ), made AS (
             INSERT INTO kb_resources (title, origin_uri)
             SELECT 'background ' || g, 'test://artifact-ask/background/' || g
               FROM generate_series(1, $3) AS g
             RETURNING id
         )
         INSERT INTO kb_resource_homes
                (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
         SELECT m.id, 'kb_contexts', h.anchor_id, $1, $1
           FROM (SELECT id, row_number() OVER (ORDER BY id) AS rn FROM made) AS m
           JOIN homes AS h ON h.n = m.rn % $2",
    )
    .bind(owner.uuid())
    .bind(BACKGROUND_HOMES)
    .bind(BACKGROUND_RESOURCES)
    .execute(&pool)
    .await
    .unwrap();
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO kb_data_artifacts
                (id, resource_id, kind_owner_table, kind_owner_id, artifact_kind, intent,
                 content_hash, content_bytes, asserted_by_event_id, last_event_id)
         SELECT gen_random_uuid(), r.id,
                CASE WHEN k % 2 = 0 THEN 'kb_profiles' ELSE 'kb_teams' END,
                CASE WHEN k % 2 = 0 THEN $1 ELSE gen_random_uuid() END,
                'measurement', 'member', 'background', 0, $2, $2
           FROM kb_resources r
          CROSS JOIN generate_series(1, $3) AS k
          WHERE r.origin_uri LIKE 'test://artifact-ask/background/%'",
    )
    .bind(owner.uuid())
    .bind(event)
    .bind(ARTIFACTS_PER_BACKGROUND_RESOURCE)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "ANALYZE kb_resources, kb_resource_homes, kb_data_artifacts, kb_data_artifact_shapes, \
                 kb_data_artifact_verdicts",
    )
    .execute(&pool)
    .await
    .unwrap();

    let visible = every_resource(&pool).await;
    for relation in [
        "__temper_ungated_governed_artifacts",
        "__temper_ungated_governed_artifact_states",
    ] {
        // The relation name is one of the two literals above; the arguments are bound.
        let explain = format!("EXPLAIN (FORMAT JSON) SELECT * FROM {relation}($1, $2)");
        let plan: Value = sqlx::query_scalar(&explain)
            .bind(&visible)
            .bind(shape)
            .fetch_one(&pool)
            .await
            .unwrap();
        let mut nodes = Vec::new();
        plan_nodes(&plan[0]["Plan"], &mut nodes);
        let on_artifacts: Vec<&&Map<String, Value>> = nodes
            .iter()
            .filter(|n| n.get("Relation Name") == Some(&json!("kb_data_artifacts")))
            .collect();
        assert!(
            !on_artifacts.is_empty(),
            "{relation} was not inlined: kb_data_artifacts is not in its plan\n{plan:#}"
        );
        assert!(
            on_artifacts.iter().all(|n| n["Node Type"] != "Seq Scan"),
            "{relation} scans kb_data_artifacts\n{plan:#}"
        );
        assert!(
            nodes.iter().any(|n| matches!(
                n.get("Index Name").and_then(Value::as_str),
                Some("idx_kb_data_artifacts_family" | "idx_kb_resource_homes_anchor")
            )),
            "{relation} starts from neither the home's index nor the family's\n{plan:#}"
        );
    }
}

// ── The path helper and the matcher ─────────────────────────────────────────────────────────────

async fn sql_get(pool: &PgPool, content: &Value, path: &[&str]) -> Option<Value> {
    let path: Vec<String> = path.iter().map(|s| s.to_string()).collect();
    sqlx::query_scalar!(
        "SELECT _temper_artifact_get($1, $2)",
        content.clone(),
        path.as_slice(),
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn sql_match(
    pool: &PgPool,
    content: Option<Value>,
    predicates: Option<Value>,
) -> Option<bool> {
    sqlx::query_scalar!(
        "SELECT _temper_artifact_ask_match($1, $2)",
        content,
        predicates,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

fn has_key(path: &[&str]) -> ArtifactPredicate {
    ArtifactPredicate {
        path: path.iter().map(|s| s.to_string()).collect(),
        op: PropertyOp::HasKey,
    }
}

fn bytewise(a: &str, b: &str) -> Ordering {
    a.cmp(b)
}

/// The path helper steps object keys and nothing else. Bite: replace its loop in `20261103000000`
/// with `RETURN p_content #> p_path;` and `["rows","0","x"]` over an array of rows reads `1`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_path_helper_steps_object_keys_only(pool: PgPool) {
    let cases: [(Value, &[&str], Option<Value>); 9] = [
        (json!({ "a": { "b": 1 } }), &["a", "b"], Some(json!(1))),
        (json!({ "rows": [{ "x": 1 }] }), &["rows", "0", "x"], None),
        (
            json!({ "rows": { "0": { "x": 1 } } }),
            &["rows", "0", "x"],
            Some(json!(1)),
        ),
        (json!([{ "x": 1 }]), &["0", "x"], None),
        (json!({ "a": "text" }), &["a", "b"], None),
        (json!({ "a": null }), &["a"], Some(Value::Null)),
        (json!({ "a": null }), &["a", "b"], None),
        (json!({ "a": 1 }), &["b"], None),
        (json!({ "a": 1 }), &[], Some(json!({ "a": 1 }))),
    ];
    for (content, path, expected) in cases {
        assert_eq!(
            sql_get(&pool, &content, path).await,
            expected,
            "{path:?} over {content}"
        );
    }
}

/// A numeric segment over an array: `["rows","0","x"]` over a stored `rows` array is absent on
/// both sides, for every operator; over an object keyed `"0"` it is present on both. Bite: in
/// `_temper_artifact_ask_match` (`20261103000000`), replace
/// `_temper_artifact_get(p_content, p.segments)` with `p_content #> p.segments`, and the array case
/// matches in SQL.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_numeric_segment_over_an_array_is_absent_on_both_sides(pool: PgPool) {
    let path = vec!["rows".to_string(), "0".to_string(), "x".to_string()];
    let operators = [
        PropertyOp::HasKey,
        PropertyOp::Contains {
            values: vec![json!(1)],
        },
        PropertyOp::Compare {
            direction: OrdOp::Gte,
            value: json!(1),
        },
    ];
    for (content, present) in [
        (json!({ "rows": [{ "x": 1 }] }), false),
        (json!({ "rows": { "0": { "x": 1 } } }), true),
    ] {
        for op in &operators {
            let ask = vec![ArtifactPredicate {
                path: path.clone(),
                op: op.clone(),
            }];
            assert_eq!(
                reference::matches(&ask, &content, &bytewise),
                present,
                "reference: {op:?} over {content}"
            );
            assert_eq!(
                sql_match(
                    &pool,
                    Some(content.clone()),
                    Some(serde_json::to_value(&ask).unwrap())
                )
                .await,
                Some(present),
                "SQL: {op:?} over {content}"
            );
        }
    }
}

/// A malformed predicate list narrows to nothing, never to everything, and each malformed case
/// stands beside its well-formed twin, which matches the same content — so the `false` is the
/// malformation's and not the content's.
///
/// The empty path is one of them: validation refuses it (`empty_artifact_path`), so the matcher
/// fails closed on it. The reference evaluator, which takes asks already validated, reads an empty
/// path as the whole document; the predicate differential therefore never generates one.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_malformed_predicate_matches_nothing_beside_a_well_formed_twin(pool: PgPool) {
    let content = json!({ "unit": "ms", "n": 5, "0": 1 });
    let has_unit = json!([{ "path": ["unit"], "op": { "op": "has_key" } }]);
    let cases: [(&str, Option<Value>, Value); 12] = [
        ("a NULL list", None, json!([])),
        ("an object for a list", Some(json!({})), json!([])),
        ("a scalar for a list", Some(json!("unit")), json!([])),
        (
            "a predicate that is not an object",
            Some(json!(["unit"])),
            has_unit.clone(),
        ),
        (
            "a path that is a string",
            Some(json!([{ "path": "unit", "op": { "op": "has_key" } }])),
            has_unit.clone(),
        ),
        (
            "an empty path",
            Some(json!([{ "path": [], "op": { "op": "has_key" } }])),
            has_unit.clone(),
        ),
        (
            "a path segment that is a number",
            Some(json!([{ "path": [0], "op": { "op": "has_key" } }])),
            json!([{ "path": ["0"], "op": { "op": "has_key" } }]),
        ),
        (
            "an operator outside the closed set",
            Some(json!([{ "path": ["unit"], "op": { "op": "eq", "value": "ms" } }])),
            json!([{ "path": ["unit"], "op": { "op": "contains", "values": ["ms"] } }]),
        ),
        (
            "no operator",
            Some(json!([{ "path": ["unit"] }])),
            has_unit.clone(),
        ),
        (
            "contains values that are not a list",
            Some(json!([{ "path": ["unit"], "op": { "op": "contains", "values": "ms" } }])),
            json!([{ "path": ["unit"], "op": { "op": "contains", "values": ["ms"] } }]),
        ),
        (
            "compare with no direction",
            Some(json!([{ "path": ["n"], "op": { "op": "compare", "value": 3 } }])),
            json!([{ "path": ["n"], "op": { "op": "compare", "direction": "gt", "value": 3 } }]),
        ),
        (
            "compare with no bound",
            Some(json!([{ "path": ["n"], "op": { "op": "compare", "direction": "gt" } }])),
            json!([{ "path": ["n"], "op": { "op": "compare", "direction": "gt", "value": 3 } }]),
        ),
    ];
    for (what, malformed, twin) in cases {
        assert_eq!(
            sql_match(&pool, Some(content.clone()), malformed).await,
            Some(false),
            "{what} matches nothing"
        );
        assert_eq!(
            sql_match(&pool, Some(content.clone()), Some(twin)).await,
            Some(true),
            "{what}: its well-formed twin matches"
        );
    }

    // An artifact with no content: the empty list is the whole population; every path is absent.
    assert_eq!(sql_match(&pool, None, Some(json!([]))).await, Some(true));
    assert_eq!(sql_match(&pool, None, Some(has_unit)).await, Some(false));
}

// ── The predicate differential ──────────────────────────────────────────────────────────────────

/// The stream's seed. A failure reproduces from it.
const SEED: u64 = 0x5EED_A5C0_2026_1019;
/// Contents generated after the hand-picked ones.
const GENERATED_CONTENTS: usize = 160;
/// Asks generated after the hand-picked ones.
const GENERATED_ASKS: usize = 240;

/// Object keys: two numeric strings and a non-ASCII key among them.
const KEYS: [&str; 6] = ["rows", "0", "x", "unit", "é", "Z"];
/// String values, chosen where byte order and a collation can disagree.
const STRINGS: [&str; 18] = [
    "", "a", "A", "b", "B", "ms", "s", "z", "Z", "é", "e", "ä", "ß", "ss", "10", "9", "0", "unit",
];
const DIRECTIONS: [OrdOp; 4] = [OrdOp::Gt, OrdOp::Gte, OrdOp::Lt, OrdOp::Lte];

/// splitmix64.
struct Stream(u64);

impl Stream {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    fn pick<T: Copy>(&mut self, from: &[T]) -> T {
        from[self.below(from.len())]
    }
}

/// An integer within ±2^53, or a finite decimal with at most six fractional digits, as a JSON
/// number (P9).
fn number(s: &mut Stream) -> Value {
    match s.below(4) {
        0 => json!(s.below(41) as i64 - 20),
        1 => json!(s.pick(&[
            0_i64,
            1,
            -1,
            10,
            -10,
            9_007_199_254_740_991,
            9_007_199_254_740_992,
            -9_007_199_254_740_992,
        ])),
        2 => {
            let places = 1 + s.below(6);
            let whole = s.below(1_000);
            let fraction = s.below(10_usize.pow(places as u32));
            let sign = if s.one_in(2) { "-" } else { "" };
            serde_json::from_str(&format!("{sign}{whole}.{fraction:0places$}")).unwrap()
        }
        // An integral value written with a fraction, which jsonb holds equal to the integer.
        _ => serde_json::from_str(&format!("{}.0", s.below(21) as i64 - 10)).unwrap(),
    }
}

/// Any JSON value; a container only while `depth` is under 3, so nesting is bounded.
fn value(s: &mut Stream, depth: usize) -> Value {
    let kinds = if depth >= 3 { 6 } else { 8 };
    match s.below(kinds) {
        0 => Value::Null,
        1 => Value::Bool(s.one_in(2)),
        2 | 3 => number(s),
        4 | 5 => Value::String(s.pick(&STRINGS).to_string()),
        6 => {
            let len = s.below(4);
            Value::Array((0..len).map(|_| value(s, depth + 1)).collect())
        }
        _ => object(s, depth + 1, 0),
    }
}

fn object(s: &mut Stream, depth: usize, at_least: usize) -> Value {
    let mut fields = Map::new();
    let len = at_least + s.below(4);
    for _ in 0..len {
        let key = s.pick(&KEYS).to_string();
        let v = value(s, depth);
        fields.insert(key, v);
    }
    Value::Object(fields)
}

/// Stored content, conformant to nothing in particular: mostly objects, and also arrays, scalars,
/// nulls, and `rows` as an array where a shape would declare an object, or as an object keyed by
/// numeric strings.
fn content(s: &mut Stream) -> Value {
    match s.below(10) {
        0 => {
            let len = s.below(4);
            Value::Array((0..len).map(|_| value(s, 1)).collect())
        }
        1 => value(s, 3),
        2 => {
            let len = 1 + s.below(3);
            let rows: Vec<Value> = (0..len).map(|_| object(s, 1, 1)).collect();
            let rows = if s.one_in(2) {
                Value::Array(rows)
            } else {
                Value::Object(
                    rows.into_iter()
                        .enumerate()
                        .map(|(i, row)| (i.to_string(), row))
                        .collect(),
                )
            };
            json!({ "rows": rows })
        }
        _ => object(s, 0, 1),
    }
}

fn path(s: &mut Stream) -> Vec<String> {
    let depth = match s.below(4) {
        0 | 1 => 1,
        2 => 2,
        _ => 3,
    };
    (0..depth).map(|_| s.pick(&KEYS).to_string()).collect()
}

/// A `contains` value: mostly a scalar, sometimes an element wrapped as `validate_ask` binds one at
/// an array path, sometimes a container.
fn probe(s: &mut Stream) -> Value {
    match s.below(5) {
        0 => Value::Array(vec![value(s, 3)]),
        1 => value(s, 2),
        _ => value(s, 3),
    }
}

/// A `compare` bound: mostly a number or a string; sometimes null, a boolean or a container, which
/// a path declaring no type admits.
fn bound(s: &mut Stream) -> Value {
    match s.below(10) {
        0..=3 => number(s),
        4..=7 => Value::String(s.pick(&STRINGS).to_string()),
        8 => value(s, 3),
        _ => value(s, 2),
    }
}

fn predicate(s: &mut Stream) -> ArtifactPredicate {
    let path = path(s);
    let op = match s.below(3) {
        0 => PropertyOp::HasKey,
        1 => {
            let len = 1 + s.below(3);
            PropertyOp::Contains {
                values: (0..len).map(|_| probe(s)).collect(),
            }
        }
        _ => PropertyOp::Compare {
            direction: s.pick(&DIRECTIONS),
            value: bound(s),
        },
    };
    ArtifactPredicate { path, op }
}

fn ask(s: &mut Stream) -> Vec<ArtifactPredicate> {
    let len = match s.below(4) {
        0 | 1 => 1,
        2 => 2,
        _ => 3,
    };
    (0..len).map(|_| predicate(s)).collect()
}

fn at(path: &[&str], op: PropertyOp) -> ArtifactPredicate {
    ArtifactPredicate {
        path: path.iter().map(|s| s.to_string()).collect(),
        op,
    }
}

fn hand_picked_contents() -> Vec<Value> {
    vec![
        json!({ "rows": [{ "x": 1 }] }),
        json!({ "rows": { "0": { "x": 1 } } }),
        json!({ "n": 5 }),
        json!({ "n": 5.0 }),
        json!({ "s": "é", "unit": "a" }),
        json!({ "unit": null }),
        json!({ "unit": ["ms", "s"] }),
        json!({ "rows": [[1, 3], 2] }),
        Value::Null,
        json!([]),
        json!("unit"),
    ]
}

fn hand_picked_asks() -> Vec<Vec<ArtifactPredicate>> {
    vec![
        vec![],
        vec![has_key(&["rows", "0", "x"])],
        vec![has_key(&["unit"])],
        vec![at(
            &["n"],
            PropertyOp::Contains {
                values: vec![json!(5.0)],
            },
        )],
        vec![at(
            &["n"],
            PropertyOp::Compare {
                direction: OrdOp::Gte,
                value: json!(5),
            },
        )],
        vec![at(
            &["unit"],
            PropertyOp::Compare {
                direction: OrdOp::Gt,
                value: json!("B"),
            },
        )],
        vec![at(
            &["unit"],
            PropertyOp::Contains {
                values: vec![json!("ms")],
            },
        )],
        vec![at(
            &["rows"],
            PropertyOp::Contains {
                values: vec![json!([[1, 3]])],
            },
        )],
        vec![at(
            &["unit"],
            PropertyOp::Compare {
                direction: OrdOp::Gte,
                value: Value::Null,
            },
        )],
    ]
}

/// Every string a comparison can meet: string values and object keys, at any depth.
fn strings_in(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::String(s) => {
            out.insert(s.clone());
        }
        Value::Array(elements) => elements.iter().for_each(|e| strings_in(e, out)),
        Value::Object(fields) => {
            for (key, v) in fields {
                out.insert(key.clone());
                strings_in(v, out);
            }
        }
        _ => {}
    }
}

/// Which operator a predicate uses, for the coverage tally.
fn operator(op: &PropertyOp) -> &'static str {
    match op {
        PropertyOp::HasKey => "has_key",
        PropertyOp::Contains { .. } => "contains",
        PropertyOp::Compare { .. } => "compare",
    }
}

/// `_temper_artifact_ask_match` and `reference::matches` agree on every pair of content and ask:
/// the hand-picked ones, then [`GENERATED_CONTENTS`] contents and [`GENERATED_ASKS`] asks from
/// [`SEED`]. Strings are ordered by Postgres (P9).
///
/// The corpus is checked for containing both outcomes under every operator and for using every
/// direction, so an agreement that held only because everything was absent cannot pass, and for
/// containing a pair of strings that bytewise order and the database collation order differently.
///
/// Bite: pass `&bytewise` for `&by_postgres` to `reference::matches`, and the hand-picked
/// `unit > "B"` over `{"unit": "a"}` disagrees (bytewise `"a" > "B"`, the collation the reverse).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_matcher_agrees_with_the_reference_evaluator(pool: PgPool) {
    let mut s = Stream(SEED);
    let mut contents = hand_picked_contents();
    contents.extend((0..GENERATED_CONTENTS).map(|_| content(&mut s)));
    let mut asks = hand_picked_asks();
    asks.extend((0..GENERATED_ASKS).map(|_| ask(&mut s)));

    let mut strings = BTreeSet::new();
    for c in &contents {
        strings_in(c, &mut strings);
    }
    for predicate in asks.iter().flatten() {
        match &predicate.op {
            PropertyOp::HasKey => {}
            PropertyOp::Contains { values } => {
                values.iter().for_each(|v| strings_in(v, &mut strings))
            }
            PropertyOp::Compare { value, .. } => strings_in(value, &mut strings),
        }
    }
    let strings: Vec<String> = strings.into_iter().collect();
    let rank: BTreeMap<String, i64> = sqlx::query!(
        r#"SELECT t.s AS "s!", dense_rank() OVER (ORDER BY to_jsonb(t.s)) AS "rank!"
             FROM unnest($1::text[]) AS t(s)"#,
        strings.as_slice(),
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| (r.s, r.rank))
    .collect();
    let ranked = |x: &str| -> i64 {
        *rank
            .get(x)
            .unwrap_or_else(|| panic!("{x:?} was never ranked by Postgres"))
    };
    let by_postgres = |a: &str, b: &str| ranked(a).cmp(&ranked(b));
    assert!(
        strings.iter().any(|x| strings
            .iter()
            .any(|y| bytewise(x, y) != by_postgres(x.as_str(), y.as_str()))),
        "every ranked pair orders the same bytewise and under the database collation, so the \
         string trap (P9) is not exercised here"
    );

    let asks_json: Vec<Value> = asks
        .iter()
        .map(|a| serde_json::to_value(a).unwrap())
        .collect();
    let rows = sqlx::query!(
        r#"SELECT c.n AS "content_n!", q.n AS "ask_n!",
                  _temper_artifact_ask_match(c.content, q.predicates) AS matched
             FROM unnest($1::jsonb[]) WITH ORDINALITY AS c(content, n)
            CROSS JOIN unnest($2::jsonb[]) WITH ORDINALITY AS q(predicates, n)"#,
        contents.as_slice(),
        asks_json.as_slice(),
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), contents.len() * asks.len());

    let mut disagreements = Vec::new();
    let mut outcomes: BTreeSet<(&str, bool)> = BTreeSet::new();
    for row in &rows {
        let content = &contents[usize::try_from(row.content_n - 1).unwrap()];
        let ask = &asks[usize::try_from(row.ask_n - 1).unwrap()];
        let expected = reference::matches(ask, content, &by_postgres);
        if row.matched != Some(expected) {
            disagreements.push(format!(
                "content {content} ask {} — SQL {:?}, reference {expected}",
                serde_json::to_value(ask).unwrap(),
                row.matched
            ));
        }
        if let [only] = ask.as_slice() {
            outcomes.insert((operator(&only.op), expected));
        }
    }
    assert!(
        disagreements.is_empty(),
        "{} of {} pairs disagree (seed {SEED:#x}); the first:\n{}",
        disagreements.len(),
        rows.len(),
        disagreements
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );

    for op in ["has_key", "contains", "compare"] {
        for outcome in [true, false] {
            assert!(
                outcomes.contains(&(op, outcome)),
                "no single-predicate {op} ask ever evaluated {outcome}: the corpus is degenerate"
            );
        }
    }
    let directions: BTreeSet<OrdOp> = asks
        .iter()
        .flatten()
        .filter_map(|p| match &p.op {
            PropertyOp::Compare { direction, .. } => Some(*direction),
            _ => None,
        })
        .collect();
    assert_eq!(
        directions.len(),
        DIRECTIONS.len(),
        "every direction is asked"
    );
}
