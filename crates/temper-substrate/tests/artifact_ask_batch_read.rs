#![cfg(feature = "test-db")]
//! **Matching artifacts are obtainable whole, in bounded pages** (spec
//! `temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md` §6, §7), against a
//! real database. Migration `20261106000000` moves the counted set's body into
//! `__temper_ungated_artifact_ask_rows`, which also takes a keyset position, a row limit,
//! matched-only and a read budget, and adds the gated page `artifacts_matching_ask` and the gated
//! population `artifact_ask_counts`; this file is the SQL's witness, read through `readback`'s
//! `artifacts_matching_ask` where the production read is what is asserted.
//!
//! **The asks are bound as the batch read binds them** — `ValidatedAsk::bind`, never the caller's
//! own ask.
//!
//! **The deny direction, conjunct by conjunct (§6.3).** Each §6.1 conjunct lives in the one body,
//! and is removed in turn from a `pg_temp` variant of the deployed body (`common::function_variant`);
//! where the gated page can see the difference, the page is varied too, calling the cut body, and
//! its answer is compared with the deployed page's. Within one home a reader of the home sees every
//! resource homed there, so the owner gate is witnessed on the body, with the visible set handed in.
//!
//! **What a page reads.** A plan witness counts the content rows one limited page reads from a home
//! of hundreds of matching artifacts (`pg_stat_xact_user_tables`), and the variant that sorts after
//! matching reads the whole home. Under a read budget the same count is bounded by the budget over
//! the per-candidate charge, for a sparse ask and for a run of oversized matches, and the page
//! says where to go on — the read frontier — however few rows it serves.

mod common;

use std::collections::BTreeSet;

use common::function_variant::variant;
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool};
use temper_core::types::artifact_ask::{
    ArtifactAsk, ArtifactPredicate, LoadedShape, ShapeContext, ShapeLoad,
};
use temper_core::types::data_artifact::ArtifactCursor;
use temper_core::types::ids::ShapeId;
use temper_core::types::query::filter::PropertyOp;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, DataArtifactId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, ArtifactIntent, EnforcementMode, KindOwner};
use temper_substrate::readback::{self, AskPageParams, RetrievedArtifact, RetrievedAskPage};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{
    self, CommitDataArtifactParams, CreateMode, CreateParams, DeclareShapeParams,
};
use uuid::Uuid;

// ── Harness ─────────────────────────────────────────────────────────────────────────────────────

const MEASUREMENT: &str = "measurement";

/// The page budget the production door passes is two million bytes; these tests pass their own,
/// so a page's edges can be built from small contents.
const ROOMY: usize = 1_000_000;

/// What these tests charge a row beside its content and its text fields (the production door
/// passes `ARTIFACT_ROW_FIXED_BYTES`).
const ROW_BYTES: usize = 64;

/// The read budget these tests pass where reading is not what they witness: far past anything a
/// fixture holds, so no page here stops on it unless a test says so.
const ROOMY_READ: usize = 1_000_000_000;

/// What these tests charge each candidate a page reads beside its content (the production door
/// passes `ARTIFACT_READ_CANDIDATE_BYTES`).
const READ_CHARGE: usize = 1_000;

const ROWS: &str = "__temper_ungated_artifact_ask_rows";
const ROWS_NARGS: i16 = 9;
const ROWS_SIGNATURE: &str = "(p_visible_ids uuid[], p_reader uuid, p_ask jsonb, \
     p_after_created timestamptz, p_after_id uuid, p_limit bigint, p_matched_only boolean, \
     p_read_budget bigint, p_read_charge bigint) \
     RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int, \
     shape_state text, matched boolean, created timestamptz, content_bytes bigint, \
     frontier boolean)";

const PAGE: &str = "artifacts_matching_ask";
const PAGE_NARGS: i16 = 9;
const PAGE_SIGNATURE: &str = "(p_principal uuid, p_ask jsonb, p_after_created timestamptz, \
     p_after_id uuid, p_limit bigint, p_page_budget bigint, p_row_bytes bigint, \
     p_read_budget bigint, p_read_charge bigint) \
     RETURNS TABLE (artifact_id uuid, resource_id uuid, kind_owner_table varchar, \
     kind_owner_id uuid, artifact_kind text, intent text, precedence double precision, \
     content_hash text, content_bytes bigint, shape_state text, is_folded boolean, \
     created timestamptz, matched boolean, frontier boolean, examined boolean, served boolean, \
     content jsonb)";

const POPULATION: &str = "__temper_ungated_artifact_ask_population";
const POPULATION_SIGNATURE: &str = "(p_visible_ids uuid[], p_reader uuid, p_ask jsonb) \
     RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int, \
     shape_state text, matched boolean)";

const COUNTS: &str = "__temper_ungated_artifact_ask_counts";
const COUNTS_SIGNATURE: &str = "(p_visible_ids uuid[], p_reader uuid, p_ask jsonb) \
     RETURNS TABLE (in_force_shape_id uuid, in_force_shape_version int, \
     declared_satisfied bigint, declared_not_satisfied bigint, \
     declared_not_yet_checked bigint, matched bigint)";

const GOVERNED_IN: &str = "__temper_ungated_governed_artifacts_in";
const GOVERNED_IN_SIGNATURE: &str = "(p_visible_ids uuid[], p_home_anchor_table text, \
     p_home_anchor_id uuid, p_kind_owner_table text, p_kind_owner_id uuid, p_artifact_kind text) \
     RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int, \
     content_hash text)";

const STATES_IN: &str = "__temper_ungated_governed_artifact_states_in";
const STATES_IN_SIGNATURE: &str = "(p_visible_ids uuid[], p_home_anchor_table text, \
     p_home_anchor_id uuid, p_kind_owner_table text, p_kind_owner_id uuid, p_artifact_kind text) \
     RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int, \
     content_hash text, shape_state text)";

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

/// The asked shape's schema: an object whose `unit` is a required string and whose `count` is an
/// integer. Advisory, so content outside it still commits.
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

/// One context home owned by the system profile P, and one of P's families (`measurement`, unless
/// a test names another); `shape` is the family's shape there, or nil until [`declare`] runs.
struct World {
    owner: ProfileId,
    emitter: EntityId,
    home: AnchorRef,
    family: (KindOwner, String),
    shape: Uuid,
}

async fn undeclared_world(pool: &PgPool) -> World {
    undeclared_world_of(pool, MEASUREMENT).await
}

async fn undeclared_world_of(pool: &PgPool, kind: &str) -> World {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let home = AnchorRef::context(ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), "asked", "asked")
            .await
            .unwrap(),
    ));
    World {
        owner,
        emitter,
        home,
        family: (KindOwner::Profile(owner.uuid()), kind.to_string()),
        shape: Uuid::nil(),
    }
}

async fn world(pool: &PgPool) -> World {
    let mut w = undeclared_world(pool).await;
    w.shape = declare(pool, &w, &shape_schema()).await;
    w
}

/// Declare the world's family's shape with `schema`: its first declaration, or an amendment of the
/// shape in force, which folds that row and mints a new id.
async fn declare(pool: &PgPool, w: &World, schema: &Value) -> Uuid {
    writes::declare_shape(
        pool,
        DeclareShapeParams {
            home: w.home,
            kind: &w.family.1,
            kind_owner: Some(w.family.0),
            schema,
            enforcement: EnforcementMode::Advisory,
            emitter: w.emitter,
        },
    )
    .await
    .unwrap()
    .uuid()
}

fn params<'a>(w: &World, title: &'a str, origin: &'a str) -> CreateParams<'a> {
    CreateParams {
        idempotency_key: None,
        title,
        origin_uri: origin,
        body: "fixture body",
        doc_type: "research",
        home: w.home,
        owner: w.owner,
        originator: w.owner,
        emitter: w.emitter,
        properties: &[],
        chunks: None,
        sources: vec![],
    }
}

/// A resource homed in the world's home, its embedding deferred.
async fn resource(pool: &PgPool, w: &World, title: &str) -> Uuid {
    let origin = format!("test://artifact-ask-batch/{title}");
    writes::create_resource_deferred_with(pool, params(w, title, &origin), EventContext::default())
        .await
        .unwrap()
        .uuid()
}

/// Commit one artifact of the world's family to `on` with `intent`.
async fn commit_as(
    pool: &PgPool,
    w: &World,
    on: Uuid,
    content: &Value,
    intent: ArtifactIntent,
) -> DataArtifactId {
    writes::commit_data_artifact(
        pool,
        CommitDataArtifactParams {
            resource: ResourceId::from(on),
            kind: &w.family.1,
            kind_owner: Some(w.family.0),
            intent,
            precedence: 0.0,
            content,
            supersedes: &[],
            emitter: w.emitter,
        },
    )
    .await
    .unwrap()
}

async fn commit(pool: &PgPool, w: &World, on: Uuid, content: &Value) -> DataArtifactId {
    commit_as(pool, w, on, content, ArtifactIntent::Member).await
}

fn contains(path: &str, value: Value) -> ArtifactPredicate {
    ArtifactPredicate {
        path: vec![path.to_string()],
        op: PropertyOp::Contains {
            values: vec![value],
        },
    }
}

/// The caller's ask of `shape`: these predicates, and nothing else said.
fn ask_of(shape: Uuid, predicates: Vec<ArtifactPredicate>) -> ArtifactAsk {
    ArtifactAsk {
        shape: ShapeId::from(shape),
        predicates,
        intent: vec![],
        include_folded: false,
        shape_states: vec![],
    }
}

fn unit_ms(shape: Uuid) -> ArtifactAsk {
    ask_of(shape, vec![contains("unit", json!("ms"))])
}

/// The ask as the batch read binds it: validated against the shape's schema through a session,
/// then `ValidatedAsk::bind`. Home and family are not in the bind — the SQL reads them by id.
fn bound(ask: &ArtifactAsk) -> Value {
    let loaded = LoadedShape {
        shape_id: ask.shape,
        shape_version: 1,
        home_anchor_table: "kb_contexts".to_string(),
        home_anchor_id: Uuid::nil(),
        kind_owner_table: "kb_profiles".to_string(),
        kind_owner_id: Uuid::nil(),
        artifact_kind: MEASUREMENT.to_string(),
        schema: shape_schema(),
    };
    let shapes: ShapeContext = [(loaded.shape_id, ShapeLoad::Loaded(loaded))]
        .into_iter()
        .collect();
    let mut session = shapes.session();
    session
        .validate(ask)
        .expect("the fixture's ask lies inside its shape")
        .bind()
}

/// What one page may spend: its row limit, its byte budget, and its read budget.
#[derive(Debug, Clone, Copy)]
struct Budgets {
    limit: u32,
    page: usize,
    read: usize,
}

/// One page through the production read, `readback::artifacts_matching_ask`, under a read budget
/// no fixture reaches ([`ROOMY_READ`]).
async fn page(
    pool: &PgPool,
    principal: ProfileId,
    ask: &ArtifactAsk,
    after: Option<ArtifactCursor>,
    limit: u32,
    page_budget: usize,
) -> RetrievedAskPage {
    let budgets = Budgets {
        limit,
        page: page_budget,
        read: ROOMY_READ,
    };
    page_within(pool, principal, ask, after, budgets).await
}

/// One page through the production read under `budgets`, each candidate charged
/// [`READ_CHARGE`] beside its content.
async fn page_within(
    pool: &PgPool,
    principal: ProfileId,
    ask: &ArtifactAsk,
    after: Option<ArtifactCursor>,
    budgets: Budgets,
) -> RetrievedAskPage {
    readback::artifacts_matching_ask(
        pool,
        AskPageParams {
            principal,
            ask: &bound(ask),
            after,
            limit: budgets.limit,
            page_budget: budgets.page,
            row_bytes: ROW_BYTES,
            read_budget: budgets.read,
            read_charge: READ_CHARGE,
        },
    )
    .await
    .expect("the page reads")
}

/// Every page of one ask from the start under `budgets`, until `next` is `None`; at most
/// `max_pages` pages, so a walk that never ends fails here.
async fn walk(
    pool: &PgPool,
    principal: ProfileId,
    ask: &ArtifactAsk,
    budgets: Budgets,
    max_pages: usize,
) -> Vec<RetrievedAskPage> {
    let mut pages = Vec::new();
    let mut after = None;
    loop {
        let p = page_within(pool, principal, ask, after, budgets).await;
        after = p.next;
        pages.push(p);
        if after.is_none() {
            return pages;
        }
        assert!(
            pages.len() < max_pages,
            "the walk ends within {max_pages} pages"
        );
    }
}

fn served(page: &RetrievedAskPage) -> Vec<Uuid> {
    page.artifacts
        .iter()
        .map(|a| a.artifact_id.uuid())
        .collect()
}

fn oversized(page: &RetrievedAskPage) -> Vec<Uuid> {
    page.oversized
        .iter()
        .map(|a| a.artifact_id.uuid())
        .collect()
}

fn ids(list: &[DataArtifactId]) -> Vec<Uuid> {
    list.iter().map(|id| id.uuid()).collect()
}

fn set(list: &[DataArtifactId]) -> BTreeSet<Uuid> {
    list.iter().map(|id| id.uuid()).collect()
}

/// [`ROW_BYTES`] as the page function binds it.
fn row_bytes() -> i64 {
    i64::try_from(ROW_BYTES).expect("fits")
}

/// `bytes` as the page function binds a budget.
fn bigint(bytes: usize) -> i64 {
    i64::try_from(bytes).expect("fits")
}

/// Where `id` sits in the keyset order: its `created` and its id.
async fn position_of(pool: &PgPool, id: DataArtifactId) -> ArtifactCursor {
    let created = sqlx::query_scalar("SELECT created FROM kb_data_artifacts WHERE id = $1")
        .bind(id.uuid())
        .fetch_one(pool)
        .await
        .unwrap();
    ArtifactCursor {
        created,
        artifact_id: id,
    }
}

fn position(a: &RetrievedArtifact) -> ArtifactCursor {
    ArtifactCursor {
        created: a.created,
        artifact_id: a.artifact_id,
    }
}

/// The artifacts a page `function` — the deployed one or a variant — serves, with their content,
/// for one ask from the start.
async fn served_through(
    conn: &mut PgConnection,
    function: &str,
    principal: ProfileId,
    ask: &Value,
    limit: i64,
    page_budget: i64,
) -> Vec<(Uuid, Option<Value>)> {
    // `function` is one of this file's literals; every value is bound.
    let sql = format!(
        "SELECT artifact_id, content FROM {function}($1::uuid, $2::jsonb, NULL::timestamptz, \
         NULL::uuid, $3::bigint, $4::bigint, $5::bigint, $6::bigint, $7::bigint) WHERE served \
         ORDER BY created, artifact_id"
    );
    sqlx::query_as::<_, (Uuid, Option<Value>)>(&sql)
        .bind(principal.uuid())
        .bind(ask)
        .bind(limit)
        .bind(page_budget)
        .bind(row_bytes())
        .bind(bigint(ROOMY_READ))
        .bind(bigint(READ_CHARGE))
        .fetch_all(&mut *conn)
        .await
        .unwrap()
}

async fn served_ids_through(
    conn: &mut PgConnection,
    function: &str,
    principal: ProfileId,
    ask: &Value,
) -> BTreeSet<Uuid> {
    served_through(conn, function, principal, ask, 100, 1_000_000)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// The matched rows the one body `function` — deployed or a variant — returns for one ask, from
/// the start, as the page calls it (matched only), with the visible set and reader handed in.
async fn matched_through(
    conn: &mut PgConnection,
    function: &str,
    visible: Option<Vec<Uuid>>,
    reader: ProfileId,
    ask: &Value,
) -> BTreeSet<Uuid> {
    // `function` is one of this file's literals; every value is bound.
    let sql = format!(
        "SELECT artifact_id FROM {function}($1::uuid[], $2::uuid, $3::jsonb, NULL::timestamptz, \
         NULL::uuid, 100::bigint, true, NULL::bigint, NULL::bigint)"
    );
    sqlx::query_scalar::<_, Uuid>(&sql)
        .bind(visible)
        .bind(reader.uuid())
        .bind(ask)
        .fetch_all(&mut *conn)
        .await
        .unwrap()
        .into_iter()
        .collect()
}

/// A page variant named `name` that reads its rows through `pg_temp.{rows}` — a variant of the one
/// body — in place of the deployed body.
async fn page_over(conn: &mut PgConnection, name: &str, rows: &str) {
    variant(
        conn,
        PAGE,
        PAGE_NARGS,
        PAGE_SIGNATURE,
        name,
        &format!("{ROWS}("),
        &format!("pg_temp.{rows}("),
    )
    .await;
}

// ── Whole content ───────────────────────────────────────────────────────────────────────────────

/// **Each matching artifact is served whole: its content equals what was committed, across two
/// owning resources in one home.** The contents nest objects and arrays and carry escapes and
/// non-ASCII text, so a re-encoding that drops or rewrites anything differs; `serde_json::Value`
/// equality is canonical-JSON equality (key order is not significant), and each row's
/// `content_bytes` is that content's compact length. Both are attributed to their own resource.
///
/// The bite, in the test: a page variant that projects one key of the stored content
/// (`(SELECT c.content FROM` → `(SELECT jsonb_build_object('unit', c.content->'unit') FROM`) serves
/// contents that are not what was committed.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_matching_artifact_is_served_whole_across_two_owning_resources(pool: PgPool) {
    let w = world(&pool).await;
    let r1 = resource(&pool, &w, "r1").await;
    let r2 = resource(&pool, &w, "r2").await;
    let c1 = json!({
        "unit": "ms",
        "count": 3,
        "samples": [1.5, 2, { "k": "v", "nested": [true, null] }],
        "note": "é \"quoted\" \\ back"
    });
    let c2 = json!({ "unit": "ms", "count": 7, "tags": { "a": [false], "b": {} } });
    let x1 = commit(&pool, &w, r1, &c1).await;
    let x2 = commit(&pool, &w, r2, &c2).await;
    commit(&pool, &w, r2, &json!({ "unit": "s" })).await;

    let p = page(&pool, w.owner, &unit_ms(w.shape), None, 10, ROOMY).await;
    assert_eq!(served(&p), ids(&[x1, x2]), "both matches, in commit order");
    assert!(p.oversized.is_empty());
    assert_eq!(p.next, None, "one page holds them");
    for (a, (committed, on)) in p.artifacts.iter().zip([(&c1, r1), (&c2, r2)]) {
        assert_eq!(a.content.as_ref(), Some(committed), "whole, as committed");
        assert_eq!(a.resource_id.uuid(), on, "attributed to its own resource");
        assert_eq!(
            a.content_bytes,
            i64::try_from(serde_json::to_string(committed).unwrap().len()).unwrap()
        );
    }

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        PAGE,
        PAGE_NARGS,
        PAGE_SIGNATURE,
        "page_one_key",
        "(SELECT c.content FROM",
        "(SELECT jsonb_build_object('unit', c.content->'unit') FROM",
    )
    .await;
    let projected = served_through(
        &mut conn,
        "pg_temp.page_one_key",
        w.owner,
        &bound(&unit_ms(w.shape)),
        10,
        1_000_000,
    )
    .await;
    assert_eq!(projected.len(), 2);
    assert!(
        projected
            .iter()
            .zip([&c1, &c2])
            .all(|((_, content), committed)| content.as_ref() != Some(committed)),
        "the bite: a projection is not the content"
    );
}

// ── Paging ──────────────────────────────────────────────────────────────────────────────────────

/// **Five matches at two a page are three pages — 2, 2, 1 — in `(created, id)` order, with no gap
/// and no repeat, and `next` is `None` on the last page only.** A non-matching artifact sits between
/// the matches, so a page that counted it, or resumed on it, would show. The population counts all
/// six and five matched (the service carries it on the first page alone; this is the SQL's count).
///
/// Bite (perl, Rust): in `readback::artifacts_matching_ask`, `i64::from(params.limit),` →
/// `i64::from(params.limit) + 1,` — pages of 3 and 2, and the first assertion goes red.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn five_matches_at_two_a_page_are_three_exact_pages(pool: PgPool) {
    let w = world(&pool).await;
    let on = resource(&pool, &w, "paged").await;
    let mut matches = Vec::new();
    for i in 0..5 {
        matches.push(commit(&pool, &w, on, &json!({ "unit": "ms", "count": i })).await);
        if i == 2 {
            commit(&pool, &w, on, &json!({ "unit": "s" })).await;
        }
    }
    let ask = unit_ms(w.shape);

    let mut pages = Vec::new();
    let mut after = None;
    loop {
        let p = page(&pool, w.owner, &ask, after, 2, ROOMY).await;
        after = p.next;
        let last = after.is_none();
        pages.push(p);
        if last || pages.len() > 5 {
            break;
        }
    }
    let sizes: Vec<usize> = pages.iter().map(|p| p.artifacts.len()).collect();
    assert_eq!(sizes, [2, 2, 1], "three exact pages");
    for p in &pages[..2] {
        assert_eq!(
            p.next,
            p.artifacts.last().map(position),
            "next is the last row examined"
        );
    }
    let read: Vec<Uuid> = pages.iter().flat_map(served).collect();
    assert_eq!(read, ids(&matches), "every match once, in order");

    let population = readback::artifact_ask_population(&pool, w.owner, &bound(&ask))
        .await
        .unwrap();
    assert_eq!(
        (population.declared_satisfied, population.matched),
        (6, 5),
        "all six are governed; five matched"
    );
}

// ── The byte budget ─────────────────────────────────────────────────────────────────────────────

/// A content of `bytes` compact bytes, matching `unit` contains `"ms"`.
fn sized(bytes: usize) -> Value {
    // `{"pad":"","unit":"ms"}` is 22 bytes; the pad makes up the rest.
    let pad = "x".repeat(bytes - 22);
    let content = json!({ "unit": "ms", "pad": pad });
    assert_eq!(serde_json::to_string(&content).unwrap().len(), bytes);
    content
}

/// What a page charges a served row beside its content, as `20261106000000`'s page measures it:
/// [`ROW_BYTES`], and each text field the row writes at the length of its JSON string. The test's
/// own reckoning of the charge, from the row as served.
fn metadata_charge(a: &RetrievedArtifact) -> usize {
    let written = |text: &str| serde_json::to_string(text).unwrap().len();
    ROW_BYTES
        + written(a.kind_owner.owner_table())
        + written(&a.artifact_kind)
        + serde_json::to_string(&a.intent).unwrap().len()
        + written(&a.content_hash)
        + serde_json::to_string(&a.shape_state).unwrap().len()
}

/// What a served row costs the page: its content and its metadata.
fn charge(a: &RetrievedArtifact) -> usize {
    usize::try_from(a.content_bytes).unwrap() + metadata_charge(a)
}

/// The one metadata charge every row of `p` carries: the fixtures commit one family with one intent
/// and a hash of one length, and the state is asserted alike, so every row's is the same.
fn one_metadata_charge(p: &RetrievedAskPage) -> usize {
    let charges: BTreeSet<usize> = p.artifacts.iter().map(metadata_charge).collect();
    assert_eq!(
        charges.len(),
        1,
        "every row of the fixture carries one metadata charge"
    );
    charges.into_iter().next().unwrap()
}

/// **A row that does not fit the budget alone is named in `oversized`, in order, and the page goes
/// on; a row that fits alone but not the remainder ends the page and opens the next.** Every row is
/// charged its metadata `M` beside its content (measured from a roomy page). A (30), B (10,000,
/// alone over the budget), C (30), D (30), E (30), under a budget of `4 × (30 + M) + ROW_BYTES - 1`:
/// the first page serves A, C, D, names B between them (charged `ROW_BYTES`), and stops before E,
/// which would be one byte past the budget; `next` is D. The second page serves E and is the last.
///
/// Bites, in the test:
/// - **oversized ends the page** — an oversized row charged its content (`ELSE p_row_bytes END AS
///   charge` → `ELSE k.content_bytes END AS charge`): B ends the first page, which then holds A
///   alone.
/// - **the page serves past its budget** — a row examined whatever the running charge (`b.n <=
///   p_limit AND b.spent <= p_page_budget AS examined` → `b.n <= p_limit AS examined`): E is served
///   on the first page, past the budget, and the page no longer ends.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_oversized_row_is_named_in_order_and_a_row_past_the_remainder_ends_the_page(
    pool: PgPool,
) {
    let w = world(&pool).await;
    let on = resource(&pool, &w, "budgeted").await;
    let mut made = Vec::new();
    for bytes in [30, 10_000, 30, 30, 30] {
        made.push(commit(&pool, &w, on, &sized(bytes)).await);
    }
    let &[a, b, c, d, e] = made.as_slice() else {
        unreachable!("five commits")
    };
    let ask = unit_ms(w.shape);
    let roomy = page(&pool, w.owner, &ask, None, 10, ROOMY).await;
    assert_eq!(
        served(&roomy),
        ids(&made),
        "the fixture: all five fit a roomy page"
    );
    let m = one_metadata_charge(&roomy);
    let budget = 4 * (30 + m) + ROW_BYTES - 1;
    assert!(
        10_000 + m > budget,
        "the fixture: B does not fit the budget alone"
    );

    let first = page(&pool, w.owner, &ask, None, 10, budget).await;
    assert_eq!(served(&first), ids(&[a, c, d]));
    assert_eq!(oversized(&first), ids(&[b]));
    assert_eq!(first.oversized[0].content_bytes, 10_000);
    assert_eq!(first.oversized[0].resource_id.uuid(), on);
    assert_eq!(
        first.next,
        first.artifacts.last().map(position),
        "the page ends after D, the last row it examined"
    );
    let second = page(&pool, w.owner, &ask, first.next, 10, budget).await;
    assert_eq!(served(&second), ids(&[e]), "E opens the next page");
    assert!(second.oversized.is_empty());
    assert_eq!(second.next, None);

    let mut conn = pool.acquire().await.unwrap();
    let bind = bound(&ask);
    let budget = i64::try_from(budget).unwrap();
    variant(
        &mut conn,
        PAGE,
        PAGE_NARGS,
        PAGE_SIGNATURE,
        "page_oversized_counts",
        "ELSE p_row_bytes END AS charge",
        "ELSE k.content_bytes END AS charge",
    )
    .await;
    let ended: Vec<Uuid> = served_through(
        &mut conn,
        "pg_temp.page_oversized_counts",
        w.owner,
        &bind,
        10,
        budget,
    )
    .await
    .into_iter()
    .map(|(id, _)| id)
    .collect();
    assert_eq!(ended, ids(&[a]), "the bite: an oversized row ends the page");

    variant(
        &mut conn,
        PAGE,
        PAGE_NARGS,
        PAGE_SIGNATURE,
        "page_past_budget",
        "b.n <= p_limit AND b.spent <= p_page_budget AS examined",
        "b.n <= p_limit AS examined",
    )
    .await;
    let past: Vec<Uuid> = served_through(
        &mut conn,
        "pg_temp.page_past_budget",
        w.owner,
        &bind,
        10,
        budget,
    )
    .await
    .into_iter()
    .map(|(id, _)| id)
    .collect();
    assert_eq!(
        past,
        ids(&[a, c, d, e]),
        "the bite: the row past the remainder is served and the page goes on"
    );
}

/// **The budget bounds the whole page, a family name of several KiB included: a page of small
/// contents under a long family name ends at the budget, and serves no byte past it.** The family
/// name is 4,096 bytes, repeated on every row; each content is 13 bytes. The budget holds three rows
/// and half a fourth, counting each row's metadata: the first page serves three, what they cost is
/// within the budget, and `next` is the third. The rest follow on later pages, every match once.
///
/// The bite, in the test: a page variant that charges content only (the metadata multiplied out,
/// `AS metadata_bytes` → `* 0 AS metadata_bytes`) serves all five on the first page — 65 content
/// bytes, far under the budget, but more than the budget once the family name is written five times.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_long_family_name_is_charged_on_every_row_and_the_page_ends_at_the_budget(pool: PgPool) {
    let family = "f".repeat(4096);
    let mut w = undeclared_world_of(&pool, &family).await;
    w.shape = declare(&pool, &w, &shape_schema()).await;
    let on = resource(&pool, &w, "long-family").await;
    let mut made = Vec::new();
    for _ in 0..5 {
        made.push(commit(&pool, &w, on, &json!({ "unit": "ms" })).await);
    }
    let ask = unit_ms(w.shape);
    let roomy = page(&pool, w.owner, &ask, None, 10, ROOMY).await;
    assert_eq!(
        served(&roomy),
        ids(&made),
        "the fixture: all five fit a roomy page"
    );
    let m = one_metadata_charge(&roomy);
    assert!(m > family.len(), "the family name is charged: {m}");
    let row = 13 + m;
    let budget = 3 * row + row / 2;

    let first = page(&pool, w.owner, &ask, None, 10, budget).await;
    assert_eq!(
        served(&first),
        ids(&made[..3]),
        "three rows fit, the fourth does not"
    );
    assert!(
        first.oversized.is_empty(),
        "no row is over the budget alone"
    );
    let spent: usize = first.artifacts.iter().map(charge).sum();
    assert!(
        spent <= budget,
        "the page served {spent} bytes of rows under a {budget} budget"
    );
    assert_eq!(first.next, first.artifacts.last().map(position));

    let mut read = served(&first);
    let mut after = first.next;
    let mut pages = 1;
    while let Some(cursor) = after {
        let p = page(&pool, w.owner, &ask, Some(cursor), 10, budget).await;
        read.extend(served(&p));
        after = p.next;
        pages += 1;
        assert!(pages <= made.len(), "the walk ends");
    }
    assert_eq!(read, ids(&made), "every match once, in order");

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        PAGE,
        PAGE_NARGS,
        PAGE_SIGNATURE,
        "page_content_only",
        "AS metadata_bytes",
        "* 0 AS metadata_bytes",
    )
    .await;
    let content_only = served_through(
        &mut conn,
        "pg_temp.page_content_only",
        w.owner,
        &bound(&ask),
        10,
        i64::try_from(budget).unwrap(),
    )
    .await;
    assert_eq!(
        content_only.len(),
        5,
        "the bite: charged its content alone, the page serves past its budget"
    );
}

// ── No predicates, no content ───────────────────────────────────────────────────────────────────

/// **An artifact committed with no content is returned by an ask with no predicates, by no ask
/// with one, and counted in the population either way.** Its content row is absent (asserted, so
/// the fixture is what it claims), so it is served with `content: None`; `unit` contains `"ms"`
/// cannot hold of nothing.
///
/// Bite (perl, Rust): in `readback::artifacts_matching_ask`'s caller here, bind the caller's raw
/// ask (`serde_json::to_value(ask)`) in place of `bound(ask)`: the raw ask with no predicates
/// carries no `predicates` key, matches nothing, and the first assertion goes red.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_contentless_artifact_matches_only_an_ask_with_no_predicates(pool: PgPool) {
    let w = world(&pool).await;
    let on = resource(&pool, &w, "contentless").await;
    let empty = commit(&pool, &w, on, &Value::Null).await;
    let full = commit(&pool, &w, on, &json!({ "unit": "ms" })).await;
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_data_artifact_content WHERE artifact_id = $1")
            .bind(empty.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 0, "the fixture is an artifact with no content row");

    let everything = ask_of(w.shape, vec![]);
    let all = page(&pool, w.owner, &everything, None, 10, ROOMY).await;
    assert_eq!(served(&all), ids(&[empty, full]));
    assert_eq!(all.artifacts[0].content, None, "no content, served as none");

    let some = page(&pool, w.owner, &unit_ms(w.shape), None, 10, ROOMY).await;
    assert_eq!(
        served(&some),
        ids(&[full]),
        "a predicate cannot hold of nothing"
    );

    for (ask, matched) in [(&everything, 2), (&unit_ms(w.shape), 1)] {
        let p = readback::artifact_ask_population(&pool, w.owner, &bound(ask))
            .await
            .unwrap();
        let counted = p.declared_satisfied + p.declared_not_satisfied + p.declared_not_yet_checked;
        assert_eq!((counted, p.matched), (2, matched), "counted either way");
    }
}

// ── §6.3: the deny direction, conjunct by conjunct ──────────────────────────────────────────────

/// **The one body admits only the visible set it is handed; NULL and empty admit nothing.** R1 and
/// R2 each own a match. Handed `[R2]`, the body returns R2's alone; NULL and `{}` return nothing;
/// the gated page, which builds the owner's own set, serves both.
///
/// The bite is the owner gate removed — the governance relation's visibility join cut, read through
/// its states form and the one body: handed `[R2]` or NULL, R1's artifact comes back.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_batch_body_admits_only_the_visible_set_it_is_handed(pool: PgPool) {
    let w = world(&pool).await;
    let r1 = resource(&pool, &w, "r1").await;
    let r2 = resource(&pool, &w, "r2").await;
    let x1 = commit(&pool, &w, r1, &json!({ "unit": "ms" })).await;
    let x2 = commit(&pool, &w, r2, &json!({ "unit": "ms" })).await;
    let ask = bound(&unit_ms(w.shape));

    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        matched_through(&mut conn, ROWS, Some(vec![r2]), w.owner, &ask).await,
        set(&[x2])
    );
    assert!(matched_through(&mut conn, ROWS, None, w.owner, &ask)
        .await
        .is_empty());
    assert!(
        matched_through(&mut conn, ROWS, Some(vec![]), w.owner, &ask)
            .await
            .is_empty()
    );
    assert_eq!(
        served_ids_through(&mut conn, PAGE, w.owner, &ask).await,
        set(&[x1, x2]),
        "the gated page hands the body the owner's own set"
    );

    variant(
        &mut conn,
        GOVERNED_IN,
        6,
        GOVERNED_IN_SIGNATURE,
        "governed_ungated",
        "JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = h.resource_id",
        "",
    )
    .await;
    variant(
        &mut conn,
        STATES_IN,
        6,
        STATES_IN_SIGNATURE,
        "states_ungated",
        "__temper_ungated_governed_artifacts_in(",
        "pg_temp.governed_ungated(",
    )
    .await;
    variant(
        &mut conn,
        ROWS,
        ROWS_NARGS,
        ROWS_SIGNATURE,
        "rows_ungated",
        "__temper_ungated_governed_artifact_states_in(",
        "pg_temp.states_ungated(",
    )
    .await;
    for visible in [Some(vec![r2]), None] {
        let leaked = matched_through(
            &mut conn,
            "pg_temp.rows_ungated",
            visible.clone(),
            w.owner,
            &ask,
        )
        .await;
        assert!(
            leaked.contains(&x1.uuid()),
            "the bite: without the owner gate R1's artifact is returned for {visible:?}"
        );
    }
}

/// **The shape's home is re-checked readable for the reader.** An outsider handed R1 in its visible
/// set gets nothing from the body, and the gated page serves it nothing; the owner, the control,
/// gets R1's artifact. Through a body whose readability check always answers yes, the outsider gets
/// it: the bite.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_batch_body_rechecks_the_shapes_home_for_the_reader(pool: PgPool) {
    let w = world(&pool).await;
    let outsider = ProfileId::from(common::insert_profile(&pool, "batch-outsider").await);
    let r1 = resource(&pool, &w, "r1").await;
    let x = commit(&pool, &w, r1, &json!({ "unit": "ms" })).await;
    let ask = bound(&unit_ms(w.shape));

    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        matched_through(&mut conn, ROWS, Some(vec![r1]), w.owner, &ask).await,
        set(&[x]),
        "the control"
    );
    assert!(
        matched_through(&mut conn, ROWS, Some(vec![r1]), outsider, &ask)
            .await
            .is_empty()
    );
    assert!(served_ids_through(&mut conn, PAGE, outsider, &ask)
        .await
        .is_empty());

    variant(
        &mut conn,
        ROWS,
        ROWS_NARGS,
        ROWS_SIGNATURE,
        "rows_without_home_check",
        "anchor_readable_by_profile(p_reader,",
        "true OR anchor_readable_by_profile(p_reader,",
    )
    .await;
    assert_eq!(
        matched_through(
            &mut conn,
            "pg_temp.rows_without_home_check",
            Some(vec![r1]),
            outsider,
            &ask
        )
        .await,
        set(&[x]),
        "the bite: without the re-check the outsider reads under a shape it cannot read"
    );
}

/// **The enumeration floor holds: a deactivated resource and one still ingesting are outside the
/// batch.** R5 is deactivated, R7 is a segmented begin still `in_progress` (asserted); each owns a
/// match. The body handed both returns neither, and the gated page serves neither.
///
/// The bite is the `kb_resources_live` join cut from the body: handed both, it returns both; and the
/// gated page reading through it serves R7's — visible to its owner, since visibility is not
/// completeness — where the deployed page does not.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_deactivated_or_ingesting_resource_is_outside_the_batch(pool: PgPool) {
    let w = world(&pool).await;
    let r5 = resource(&pool, &w, "r5-deactivated").await;
    let x5 = commit(&pool, &w, r5, &json!({ "unit": "ms" })).await;
    let origin = "test://artifact-ask-batch/r7-ingesting";
    let r7 = writes::create_resource_with_mode(
        &pool,
        params(&w, "r7-ingesting", origin),
        EventContext::default(),
        CreateMode {
            defer: true,
            segmented: true,
        },
    )
    .await
    .unwrap()
    .uuid();
    let state: String = sqlx::query_scalar("SELECT ingest_state FROM kb_resources WHERE id = $1")
        .bind(r7)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "in_progress", "the fixture is an interrupted ingest");
    let x7 = commit(&pool, &w, r7, &json!({ "unit": "ms" })).await;
    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(r5)
        .execute(&pool)
        .await
        .unwrap();
    let ask = bound(&unit_ms(w.shape));

    let mut conn = pool.acquire().await.unwrap();
    assert!(
        matched_through(&mut conn, ROWS, Some(vec![r5, r7]), w.owner, &ask)
            .await
            .is_empty()
    );
    assert!(served_ids_through(&mut conn, PAGE, w.owner, &ask)
        .await
        .is_empty());

    variant(
        &mut conn,
        ROWS,
        ROWS_NARGS,
        ROWS_SIGNATURE,
        "rows_without_floor",
        "JOIN kb_resources_live lv ON lv.id = g.resource_id",
        "",
    )
    .await;
    assert_eq!(
        matched_through(
            &mut conn,
            "pg_temp.rows_without_floor",
            Some(vec![r5, r7]),
            w.owner,
            &ask
        )
        .await,
        set(&[x5, x7]),
        "the bite: without the floor both come back"
    );
    page_over(&mut conn, "page_without_floor", "rows_without_floor").await;
    assert_eq!(
        served_ids_through(&mut conn, "pg_temp.page_without_floor", w.owner, &ask).await,
        set(&[x7]),
        "and the page serves the ingesting resource's artifact"
    );
}

/// **The ask's intent narrows the batch.** A `pinned` artifact matches on content, but an ask that
/// names no intent admits `current` and `member` only, so the page does not serve it; an ask naming
/// `pinned` does (the control). Through a body with the intent conjunct made vacuous, read by the
/// page, the default ask serves it: the bite.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_asks_intent_narrows_the_batch(pool: PgPool) {
    let w = world(&pool).await;
    let on = resource(&pool, &w, "pinned").await;
    let member = commit(&pool, &w, on, &json!({ "unit": "ms" })).await;
    let pinned = commit_as(
        &pool,
        &w,
        on,
        &json!({ "unit": "ms" }),
        ArtifactIntent::Pinned,
    )
    .await;
    let default_ask = bound(&unit_ms(w.shape));
    let pinned_ask = bound(&ArtifactAsk {
        intent: vec!["pinned".to_string()],
        ..unit_ms(w.shape)
    });

    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        served_ids_through(&mut conn, PAGE, w.owner, &default_ask).await,
        set(&[member])
    );
    assert_eq!(
        served_ids_through(&mut conn, PAGE, w.owner, &pinned_ask).await,
        set(&[pinned]),
        "the control: the pinned artifact matches when its intent is asked"
    );

    variant(
        &mut conn,
        ROWS,
        ROWS_NARGS,
        ROWS_SIGNATURE,
        "rows_any_intent",
        "$7->'intent' @> jsonb_build_array(a.intent)",
        "true",
    )
    .await;
    page_over(&mut conn, "page_any_intent", "rows_any_intent").await;
    assert_eq!(
        served_ids_through(&mut conn, "pg_temp.page_any_intent", w.owner, &default_ask).await,
        set(&[member, pinned]),
        "the bite: without the intent conjunct the pinned artifact is served"
    );
}

/// **The ask's verdict states narrow the batch.** The early artifact was committed before the shape
/// (unchecked), the late one after it, conforming (satisfied); both match on content. Asked for
/// `declared_satisfied`, the page serves the late one alone. Through a body whose `matched` ignores
/// the states, read by the page, it serves both: the bite.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_asks_verdict_states_narrow_the_batch(pool: PgPool) {
    let mut w = undeclared_world(&pool).await;
    let early_on = resource(&pool, &w, "early").await;
    let early = commit(&pool, &w, early_on, &json!({ "unit": "ms" })).await;
    w.shape = declare(&pool, &w, &shape_schema()).await;
    let late_on = resource(&pool, &w, "late").await;
    let late = commit(&pool, &w, late_on, &json!({ "unit": "ms" })).await;
    let satisfied = bound(&ArtifactAsk {
        shape_states: vec!["declared_satisfied".to_string()],
        ..unit_ms(w.shape)
    });

    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        served_ids_through(&mut conn, PAGE, w.owner, &bound(&unit_ms(w.shape))).await,
        set(&[early, late]),
        "the fixture: both match with every state admitted"
    );
    assert_eq!(
        served_ids_through(&mut conn, PAGE, w.owner, &satisfied).await,
        set(&[late])
    );

    variant(
        &mut conn,
        ROWS,
        ROWS_NARGS,
        ROWS_SIGNATURE,
        "rows_any_state",
        "$7->'shape_states' @> jsonb_build_array(o.shape_state)",
        "true",
    )
    .await;
    page_over(&mut conn, "page_any_state", "rows_any_state").await;
    assert_eq!(
        served_ids_through(&mut conn, "pg_temp.page_any_state", w.owner, &satisfied).await,
        set(&[early, late]),
        "the bite: without the state narrowing the unchecked artifact is served"
    );
}

/// One row of the counts: the three by-state counts and `matched`.
type Counts = (i64, i64, i64, i64);

async fn counts_through(
    conn: &mut PgConnection,
    function: &str,
    visible: Option<Vec<Uuid>>,
    reader: ProfileId,
    ask: &Value,
) -> Counts {
    // `function` is one of this file's literals; every value is bound.
    let sql = format!(
        "SELECT declared_satisfied, declared_not_satisfied, declared_not_yet_checked, matched \
           FROM {function}($1::uuid[], $2::uuid, $3::jsonb)"
    );
    sqlx::query_as::<_, Counts>(&sql)
        .bind(visible)
        .bind(reader.uuid())
        .bind(ask)
        .fetch_one(&mut *conn)
        .await
        .unwrap()
}

/// **The population counts range over the verdict, through the one body.** The gated population
/// counts the owner's both; an outsider's counts nothing. Handed `[R1]`, the counts see R1's alone.
///
/// The bite is the owner gate removed — the visibility join cut from the governance relation, read
/// through the one body, the counted set and the counts: handed `[R1]`, R2's artifact is counted,
/// so the counts differ from the gated ones (spec §6.3).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_population_counts_range_over_the_verdict_through_the_one_body(pool: PgPool) {
    let w = world(&pool).await;
    let outsider = ProfileId::from(common::insert_profile(&pool, "batch-counts-outsider").await);
    let r1 = resource(&pool, &w, "r1").await;
    let r2 = resource(&pool, &w, "r2").await;
    commit(&pool, &w, r1, &json!({ "unit": "ms" })).await;
    commit(&pool, &w, r2, &json!({ "unit": "ms" })).await;
    let ask = bound(&unit_ms(w.shape));

    let owned = readback::artifact_ask_population(&pool, w.owner, &ask)
        .await
        .unwrap();
    assert_eq!(
        (owned.declared_satisfied, owned.matched),
        (2, 2),
        "the owner's own set"
    );
    let outside = readback::artifact_ask_population(&pool, outsider, &ask)
        .await
        .unwrap();
    assert_eq!(
        (
            outside.declared_satisfied,
            outside.declared_not_satisfied,
            outside.declared_not_yet_checked,
            outside.matched
        ),
        (0, 0, 0, 0),
        "an outsider's set and home count nothing"
    );

    let mut conn = pool.acquire().await.unwrap();
    let gated = counts_through(&mut conn, COUNTS, Some(vec![r1]), w.owner, &ask).await;
    assert_eq!(gated, (1, 0, 0, 1), "R1's alone");

    variant(
        &mut conn,
        GOVERNED_IN,
        6,
        GOVERNED_IN_SIGNATURE,
        "governed_ungated",
        "JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = h.resource_id",
        "",
    )
    .await;
    variant(
        &mut conn,
        STATES_IN,
        6,
        STATES_IN_SIGNATURE,
        "states_ungated",
        "__temper_ungated_governed_artifacts_in(",
        "pg_temp.governed_ungated(",
    )
    .await;
    variant(
        &mut conn,
        ROWS,
        ROWS_NARGS,
        ROWS_SIGNATURE,
        "rows_ungated",
        "__temper_ungated_governed_artifact_states_in(",
        "pg_temp.states_ungated(",
    )
    .await;
    variant(
        &mut conn,
        POPULATION,
        3,
        POPULATION_SIGNATURE,
        "population_ungated",
        &format!("{ROWS}("),
        "pg_temp.rows_ungated(",
    )
    .await;
    variant(
        &mut conn,
        COUNTS,
        3,
        COUNTS_SIGNATURE,
        "counts_ungated",
        &format!("{POPULATION}("),
        "pg_temp.population_ungated(",
    )
    .await;
    let leaked = counts_through(
        &mut conn,
        "pg_temp.counts_ungated",
        Some(vec![r1]),
        w.owner,
        &ask,
    )
    .await;
    assert_eq!(leaked, (2, 0, 0, 2), "the bite: R2's artifact is counted");
    assert_ne!(leaked, gated);
}

// ── The one body holds: a limited page reads content for the rows it examines ───────────────────

const LIMIT: u32 = 2;

/// The matching artifacts the large home holds beyond the fixture's own.
const LARGE_HOME: i64 = 400;

/// The most content rows one limited page may read. **The arithmetic:** the body examines at most
/// `LIMIT + 1` matches in order and its matcher reads each one's content once; the page re-reads
/// the content of at most `LIMIT` served rows. That is `2 × LIMIT + 1` = 5 reads; the bound is twice
/// that, so a plan that reads each examined row's content twice still passes, while a body that
/// matches before it sorts reads every one of the home's [`LARGE_HOME`] + 1.
const CONTENT_READ_BOUND: i64 = 2 * (2 * LIMIT as i64 + 1);

/// Rows this transaction has read from `relation` so far: tuples returned by sequential scans plus
/// heap tuples fetched by index and bitmap scans, counted before any filter runs. The counters see
/// nested execution, so they see the one body's `EXECUTE`.
async fn rows_read(conn: &mut PgConnection, relation: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE(seq_tup_read, 0) + COALESCE(idx_tup_fetch, 0) \
           FROM pg_stat_xact_user_tables WHERE relname = $1",
    )
    .bind(relation)
    .fetch_one(&mut *conn)
    .await
    .unwrap()
}

/// One call of a page function — the deployed one or a variant — from the start of the set.
#[derive(Debug, Clone, Copy)]
struct Probe<'a> {
    function: &'a str,
    principal: ProfileId,
    ask: &'a Value,
    budgets: Budgets,
}

/// The production budgets a limited page passes, with a read budget no fixture here reaches.
const LIMITED: Budgets = Budgets {
    limit: LIMIT,
    page: 2_000_000,
    read: ROOMY_READ,
};

/// One page through `probe`, after the `variants` it reads through are created, and the content
/// and artifact rows it read, measured inside one transaction around that call alone (rolled back):
/// `(served, content rows, artifact rows)`. Each variant is `(name, varied, signature, cut, nargs,
/// with)`, as [`variant`] takes them.
async fn page_reads(
    pool: &PgPool,
    probe: Probe<'_>,
    variants: &[(&str, &str, &str, &str, i16, &str)],
) -> (i64, i64, i64) {
    let mut tx = pool.begin().await.unwrap();
    for (name, varied, signature, cut, nargs, with) in variants {
        variant(&mut tx, varied, *nargs, signature, name, cut, with).await;
    }
    let content_before = rows_read(&mut tx, "kb_data_artifact_content").await;
    let artifacts_before = rows_read(&mut tx, "kb_data_artifacts").await;
    // `function` is one of this file's literals; every value is bound.
    let served: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FILTER (WHERE served) FROM {}($1::uuid, $2::jsonb, \
         NULL::timestamptz, NULL::uuid, $3::bigint, $4::bigint, $5::bigint, $6::bigint, \
         $7::bigint)",
        probe.function
    ))
    .bind(probe.principal.uuid())
    .bind(probe.ask)
    .bind(i64::from(probe.budgets.limit))
    .bind(bigint(probe.budgets.page))
    .bind(row_bytes())
    .bind(bigint(probe.budgets.read))
    .bind(bigint(READ_CHARGE))
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let content = rows_read(&mut tx, "kb_data_artifact_content").await - content_before;
    let artifacts = rows_read(&mut tx, "kb_data_artifacts").await - artifacts_before;
    tx.rollback().await.unwrap();
    (served, content, artifacts)
}

/// **A limited page over a large home reads content for the rows it examines, not for the home.**
/// One resource in the home holds [`LARGE_HOME`] + 1 artifacts of the asked family, every one
/// matching `unit` contains `"ms"` (fixture inserts beside one real commit; counted). A page of
/// [`LIMIT`] serves `LIMIT` and reads at most [`CONTENT_READ_BOUND`] rows of
/// `kb_data_artifact_content` (`pg_stat_xact_user_tables`, so the body's `EXECUTE` is seen), and at
/// least one, so the bound cannot pass by measuring nothing. The ask carries a predicate, so the
/// matcher must read content to answer. The read budget is one no fixture reaches, so what stops
/// the page is its limit.
///
/// `kb_data_artifacts` is read for every governed row — the order is taken on cheap columns before
/// any is matched — and the test reports that count beside the bound rather than bounding it.
///
/// The bite, in the test: the body's top-level order taken on `matched` first (`ORDER BY
/// o.created, o.artifact_id` → `ORDER BY m.matched, o.created, o.artifact_id`), a body that sorts
/// after it matches: the sort needs every row's `matched` before the limit can stop it, so the page
/// reads content for the whole home and goes past the bound.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_limited_page_reads_content_only_for_the_rows_it_examines(pool: PgPool) {
    let w = world(&pool).await;
    let on = resource(&pool, &w, "large-home").await;
    commit(&pool, &w, on, &json!({ "unit": "ms" })).await;
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query(
        "WITH made AS (
             INSERT INTO kb_data_artifacts
                    (id, resource_id, kind_owner_table, kind_owner_id, artifact_kind, intent,
                     content_hash, content_bytes, asserted_by_event_id, last_event_id)
             SELECT gen_random_uuid(), $1, 'kb_profiles', $2, $3, 'member', 'background', 13,
                    $4, $4
               FROM generate_series(1, $5)
             RETURNING id
         )
         INSERT INTO kb_data_artifact_content (artifact_id, content, content_hash)
         SELECT m.id, '{\"unit\": \"ms\"}'::jsonb, 'background' FROM made AS m",
    )
    .bind(on)
    .bind(w.owner.uuid())
    .bind(MEASUREMENT)
    .bind(event)
    .bind(LARGE_HOME)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("ANALYZE kb_data_artifacts, kb_data_artifact_content, kb_resource_homes")
        .execute(&pool)
        .await
        .unwrap();
    let ask = bound(&unit_ms(w.shape));
    let population = readback::artifact_ask_population(&pool, w.owner, &ask)
        .await
        .unwrap();
    assert_eq!(
        population.matched,
        LARGE_HOME + 1,
        "the home holds this many matches"
    );

    let probe = Probe {
        function: PAGE,
        principal: w.owner,
        ask: &ask,
        budgets: LIMITED,
    };
    let (served, content, artifacts) = page_reads(&pool, probe, &[]).await;
    assert_eq!(served, i64::from(LIMIT));
    assert!(content > 0, "the count sees the page's content reads");
    assert!(
        content <= CONTENT_READ_BOUND,
        "a page of {LIMIT} read {content} content rows from a home of {} matches ({artifacts} \
         artifact rows read for the order)",
        LARGE_HOME + 1
    );

    let (_, bitten, _) = page_reads(
        &pool,
        Probe {
            function: "pg_temp.page_sorting_after_matching",
            ..probe
        },
        &[
            (
                "rows_sorting_after_matching",
                ROWS,
                ROWS_SIGNATURE,
                "ORDER BY o.created, o.artifact_id",
                ROWS_NARGS,
                "ORDER BY m.matched, o.created, o.artifact_id",
            ),
            (
                "page_sorting_after_matching",
                PAGE,
                PAGE_SIGNATURE,
                "__temper_ungated_artifact_ask_rows(",
                PAGE_NARGS,
                "pg_temp.rows_sorting_after_matching(",
            ),
        ],
    )
    .await;
    assert!(
        bitten > CONTENT_READ_BOUND,
        "the bite: a body that sorts after it matches read only {bitten} content rows"
    );
}

// ── The read budget: what one page may read to find its rows ────────────────────────────────────

/// The body with its read budget ignored — every candidate read, as if the budget were absent —
/// and a page reading through it: the variants a read-budget bite creates.
const IGNORING_THE_READ_BUDGET: [(&str, &str, &str, &str, i16, &str); 2] = [
    (
        "rows_reading_everything",
        ROWS,
        ROWS_SIGNATURE,
        "WHERE ($12 IS NULL OR o.read_before < $12)",
        ROWS_NARGS,
        "WHERE true",
    ),
    (
        "page_reading_everything",
        PAGE,
        PAGE_SIGNATURE,
        "__temper_ungated_artifact_ask_rows(",
        PAGE_NARGS,
        "pg_temp.rows_reading_everything(",
    ),
];

/// The candidates one page may read under `read`: each costs at least [`READ_CHARGE`], and a
/// candidate is read only while the charge before it is under the budget, so `read / READ_CHARGE`
/// candidates, and the first.
fn candidates_within(read: usize) -> i64 {
    bigint(read / READ_CHARGE + 1)
}

/// **A sparse ask over a large home reads a bounded number of candidates per page, says where to
/// go on, and a walk serves every match once, in order.** Forty artifacts in one home, three of
/// them matching (the 8th, 24th and 39th). Under a read budget of `4.5 × READ_CHARGE`, one page
/// reads content for at most [`candidates_within`] = 5 candidates (`kb_data_artifact_content`
/// rows, as the limited-page witness counts them) and at least one. The first five candidates
/// hold no match, so the first page serves nothing and still carries a `next`: the fifth
/// candidate, the last it read. Walking `next` to `None` serves the three, once each, in order.
///
/// The bite, in the test: the body with its read budget ignored ([`IGNORING_THE_READ_BUDGET`]),
/// read by the page — the first page reads every candidate of the home, past the bound.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_sparse_ask_reads_a_bounded_page_and_a_walk_serves_every_match_once(pool: PgPool) {
    let w = world(&pool).await;
    let on = resource(&pool, &w, "sparse").await;
    let mut made = Vec::new();
    let mut matches = Vec::new();
    for i in 0..40 {
        let unit = if [7, 23, 38].contains(&i) { "ms" } else { "s" };
        let id = commit(&pool, &w, on, &json!({ "unit": unit, "count": i })).await;
        if unit == "ms" {
            matches.push(id);
        }
        made.push(id);
    }
    let ask = unit_ms(w.shape);
    let read = 4 * READ_CHARGE + READ_CHARGE / 2;
    let budgets = Budgets {
        limit: 10,
        page: ROOMY,
        read,
    };

    let first = page_within(&pool, w.owner, &ask, None, budgets).await;
    assert!(first.artifacts.is_empty() && first.oversized.is_empty());
    assert_eq!(
        first.next,
        Some(position_of(&pool, made[4]).await),
        "no match among the first five read, and `next` is the last read: the page goes on"
    );

    let bind = bound(&ask);
    let probe = Probe {
        function: PAGE,
        principal: w.owner,
        ask: &bind,
        budgets,
    };
    let (_, content, _) = page_reads(&pool, probe, &[]).await;
    assert!(content > 0, "the count sees the page's content reads");
    assert!(
        content <= candidates_within(read),
        "one page read {content} content rows under a budget of {} candidates",
        candidates_within(read)
    );

    let pages = walk(&pool, w.owner, &ask, budgets, made.len()).await;
    let read_through: Vec<Uuid> = pages.iter().flat_map(served).collect();
    assert_eq!(read_through, ids(&matches), "every match once, in order");
    assert!(pages.iter().all(|p| p.oversized.is_empty()));

    let (_, bitten, _) = page_reads(
        &pool,
        Probe {
            function: "pg_temp.page_reading_everything",
            ..probe
        },
        &IGNORING_THE_READ_BUDGET,
    )
    .await;
    assert!(
        bitten > candidates_within(read),
        "the bite: a body ignoring its read budget read only {bitten} content rows"
    );
}

/// **A run of oversized matches is read one at a time: a page reads the first and stops at it.**
/// Five matching artifacts of 3,000 content bytes each, under a page budget of 1,000 (so each is
/// oversized) and a read budget of 2,000 (under one of them). The first page reads the first,
/// names it in `oversized`, and ends there — it is the read frontier, and `next` — reading content
/// for that one candidate alone. A walk names all five in `oversized`, once each, in order, and
/// serves nothing.
///
/// The bite, in the test: the body with its read budget ignored ([`IGNORING_THE_READ_BUDGET`]),
/// read by the page — the first page reads all five.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn oversized_matches_past_the_read_budget_are_read_one_page_at_a_time(pool: PgPool) {
    let w = world(&pool).await;
    let on = resource(&pool, &w, "oversized-run").await;
    let mut made = Vec::new();
    for _ in 0..5 {
        made.push(commit(&pool, &w, on, &sized(3_000)).await);
    }
    let ask = unit_ms(w.shape);
    let budgets = Budgets {
        limit: 10,
        page: 1_000,
        read: 2_000,
    };

    let first = page_within(&pool, w.owner, &ask, None, budgets).await;
    assert!(first.artifacts.is_empty());
    assert_eq!(
        oversized(&first),
        ids(&made[..1]),
        "the first, and no other"
    );
    assert_eq!(first.next, Some(position_of(&pool, made[0]).await));

    let bind = bound(&ask);
    let probe = Probe {
        function: PAGE,
        principal: w.owner,
        ask: &bind,
        budgets,
    };
    let (_, content, _) = page_reads(&pool, probe, &[]).await;
    assert_eq!(content, 1, "one candidate's content read");

    let pages = walk(&pool, w.owner, &ask, budgets, made.len() + 1).await;
    let named: Vec<Uuid> = pages.iter().flat_map(oversized).collect();
    assert_eq!(named, ids(&made), "every oversized match once, in order");
    assert!(pages.iter().all(|p| p.artifacts.is_empty()));

    let (_, bitten, _) = page_reads(
        &pool,
        Probe {
            function: "pg_temp.page_reading_everything",
            ..probe
        },
        &IGNORING_THE_READ_BUDGET,
    )
    .await;
    assert_eq!(
        bitten, 5,
        "the bite: a body ignoring its read budget reads all five"
    );
}

/// One row of a page function's answer: the artifact, whether it was examined, served, and the
/// read frontier.
type PageRow = (Uuid, bool, bool, bool);

/// The rows a page function — the deployed one or a variant — answers for one ask from the start.
async fn page_rows_through(
    conn: &mut PgConnection,
    function: &str,
    principal: ProfileId,
    ask: &Value,
    budgets: Budgets,
) -> Vec<PageRow> {
    // `function` is one of this file's literals; every value is bound.
    let sql = format!(
        "SELECT artifact_id, examined, served, frontier FROM {function}($1::uuid, $2::jsonb, \
         NULL::timestamptz, NULL::uuid, $3::bigint, $4::bigint, $5::bigint, $6::bigint, \
         $7::bigint) ORDER BY created, artifact_id"
    );
    sqlx::query_as::<_, PageRow>(&sql)
        .bind(principal.uuid())
        .bind(ask)
        .bind(i64::from(budgets.limit))
        .bind(bigint(budgets.page))
        .bind(row_bytes())
        .bind(bigint(budgets.read))
        .bind(bigint(READ_CHARGE))
        .fetch_all(&mut *conn)
        .await
        .unwrap()
}

/// **The read frontier never counts against `limit` and is never served.** M1 matches, N1 and N2
/// do not, M2 matches. With a page of one and a read budget of 1.5 candidates, the first page
/// reads M1 and N1 and stops: it serves M1 alone, and `next` is N1 — the frontier, examined though
/// the page's one matching row is spent, because only matching rows count toward the limit, and
/// served nowhere. The next page serves M2 and is the last.
///
/// Bites, in the test:
/// - **the frontier counted as a match** (`count(*) FILTER (WHERE m.matched) OVER ordered AS n` →
///   `count(*) OVER ordered AS n`): N1 is past the limit, unexamined, so the page would continue
///   from M1 and read N1 again.
/// - **the frontier served** (`e.examined AND e.matched AND e.fits AS served` → `e.examined AND
///   e.fits AS served`): N1 is served, though it matches nothing.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_read_frontier_never_counts_against_the_limit_and_is_never_served(pool: PgPool) {
    let w = world(&pool).await;
    let on = resource(&pool, &w, "frontier").await;
    let m1 = commit(&pool, &w, on, &json!({ "unit": "ms" })).await;
    let n1 = commit(&pool, &w, on, &json!({ "unit": "s" })).await;
    commit(&pool, &w, on, &json!({ "unit": "s" })).await;
    let m2 = commit(&pool, &w, on, &json!({ "unit": "ms" })).await;
    let ask = unit_ms(w.shape);
    let budgets = Budgets {
        limit: 1,
        page: ROOMY,
        read: READ_CHARGE + READ_CHARGE / 2,
    };

    let first = page_within(&pool, w.owner, &ask, None, budgets).await;
    assert_eq!(served(&first), ids(&[m1]));
    assert!(
        first.oversized.is_empty(),
        "the frontier is never oversized"
    );
    assert_eq!(
        first.next,
        Some(position_of(&pool, n1).await),
        "the frontier is examined beside a full page, and is where the next page starts"
    );
    let second = page_within(&pool, w.owner, &ask, first.next, budgets).await;
    assert_eq!(served(&second), ids(&[m2]));
    assert_eq!(second.next, None);

    let bind = bound(&ask);
    let mut conn = pool.acquire().await.unwrap();
    let deployed = page_rows_through(&mut conn, PAGE, w.owner, &bind, budgets).await;
    assert_eq!(
        deployed,
        [
            (m1.uuid(), true, true, false),
            (n1.uuid(), true, false, true)
        ],
        "M1 served; N1 the frontier, examined and not served"
    );

    variant(
        &mut conn,
        PAGE,
        PAGE_NARGS,
        PAGE_SIGNATURE,
        "page_counting_the_frontier",
        "count(*) FILTER (WHERE m.matched) OVER ordered AS n",
        "count(*) OVER ordered AS n",
    )
    .await;
    let counted = page_rows_through(
        &mut conn,
        "pg_temp.page_counting_the_frontier",
        w.owner,
        &bind,
        budgets,
    )
    .await;
    assert_eq!(
        counted[1],
        (n1.uuid(), false, false, true),
        "the bite: counted as a match, the frontier is past the limit"
    );

    variant(
        &mut conn,
        PAGE,
        PAGE_NARGS,
        PAGE_SIGNATURE,
        "page_serving_the_frontier",
        "e.examined AND e.matched AND e.fits AS served",
        "e.examined AND e.fits AS served",
    )
    .await;
    let serving = page_rows_through(
        &mut conn,
        "pg_temp.page_serving_the_frontier",
        w.owner,
        &bind,
        budgets,
    )
    .await;
    assert_eq!(
        serving[1],
        (n1.uuid(), true, true, true),
        "the bite: the frontier is served"
    );
}

// ── The page's edges, measured ──────────────────────────────────────────────────────────────────

/// **A family name full of escapes is charged exactly as the page writes it.** The name carries a
/// quote, a backslash, a C0 control and a character outside the BMP, sixteen times over, so its
/// JSON string is longer than its bytes. Every row's metadata `M` is this file's own serde
/// reckoning ([`one_metadata_charge`]); with each row costing `13 + M`, a budget of exactly three
/// rows serves three, and one byte less serves two. So the SQL's charge (`to_json(..)::text`) and
/// serde's written length are equal, not merely close: one byte over or under on any row moves a
/// page.
///
/// The bite, in the test: the page charging the family name's raw bytes
/// (`octet_length(to_json(d.artifact_kind)::text)` → `octet_length(d.artifact_kind)`) — under-
/// counted, three rows fit one byte less than three rows' budget.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_escaped_family_name_is_charged_exactly_as_the_page_writes_it(pool: PgPool) {
    let family = "a \"quoted\" \\ name\u{1}\u{1F600}".repeat(16);
    let mut w = undeclared_world_of(&pool, &family).await;
    w.shape = declare(&pool, &w, &shape_schema()).await;
    let on = resource(&pool, &w, "escaped-family").await;
    let mut made = Vec::new();
    for _ in 0..5 {
        made.push(commit(&pool, &w, on, &json!({ "unit": "ms" })).await);
    }
    let ask = unit_ms(w.shape);
    let roomy = page(&pool, w.owner, &ask, None, 10, ROOMY).await;
    assert_eq!(served(&roomy), ids(&made), "the fixture: all five fit");
    assert_eq!(
        roomy.artifacts[0].artifact_kind, family,
        "the name as stored"
    );
    let m = one_metadata_charge(&roomy);
    assert!(
        m > ROW_BYTES + family.len(),
        "the written name is longer than its bytes: {m}"
    );
    let three = 3 * (13 + m);

    let exact = page(&pool, w.owner, &ask, None, 10, three).await;
    assert_eq!(
        served(&exact),
        ids(&made[..3]),
        "three rows fit three rows' budget"
    );
    let under = page(&pool, w.owner, &ask, None, 10, three - 1).await;
    assert_eq!(served(&under), ids(&made[..2]), "and not one byte less");

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        PAGE,
        PAGE_NARGS,
        PAGE_SIGNATURE,
        "page_raw_family_name",
        "octet_length(to_json(d.artifact_kind)::text)",
        "octet_length(d.artifact_kind)",
    )
    .await;
    let raw = served_through(
        &mut conn,
        "pg_temp.page_raw_family_name",
        w.owner,
        &bound(&ask),
        10,
        bigint(three - 1),
    )
    .await;
    assert_eq!(
        raw.len(),
        3,
        "the bite: charged its raw bytes, the name is under-counted"
    );
}

/// **`limit` counts oversized rows: a page whose limit is reached by an oversized row ends at
/// it.** A (30 bytes), B (10,000, oversized under a 1,000 budget), C (30), at two a page: the first
/// page serves A, names B, and stops — `next` is B — and the second serves C.
///
/// The bite, in the test: the page numbering only the rows that fit (`count(*) FILTER (WHERE
/// m.matched) OVER ordered AS n` → `count(*) FILTER (WHERE m.matched AND m.fits) OVER ordered AS
/// n`): C is within the limit too, and the first page serves A and C.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_limit_counts_an_oversized_row(pool: PgPool) {
    let w = world(&pool).await;
    let on = resource(&pool, &w, "limited-by-oversized").await;
    let a = commit(&pool, &w, on, &sized(30)).await;
    let b = commit(&pool, &w, on, &sized(10_000)).await;
    let c = commit(&pool, &w, on, &sized(30)).await;
    let ask = unit_ms(w.shape);

    let first = page(&pool, w.owner, &ask, None, 2, 1_000).await;
    assert_eq!(served(&first), ids(&[a]));
    assert_eq!(oversized(&first), ids(&[b]));
    assert_eq!(
        first.next,
        Some(position_of(&pool, b).await),
        "B spends the limit"
    );
    let second = page(&pool, w.owner, &ask, first.next, 2, 1_000).await;
    assert_eq!(served(&second), ids(&[c]));
    assert_eq!(second.next, None);

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        PAGE,
        PAGE_NARGS,
        PAGE_SIGNATURE,
        "page_counting_fitting_rows",
        "count(*) FILTER (WHERE m.matched) OVER ordered AS n",
        "count(*) FILTER (WHERE m.matched AND m.fits) OVER ordered AS n",
    )
    .await;
    let counted: Vec<Uuid> = served_through(
        &mut conn,
        "pg_temp.page_counting_fitting_rows",
        w.owner,
        &bound(&ask),
        2,
        1_000,
    )
    .await
    .into_iter()
    .map(|(id, _)| id)
    .collect();
    assert_eq!(
        counted,
        ids(&[a, c]),
        "the bite: the oversized row spends nothing of the limit"
    );
}

/// **Four matches at two a page are exactly two pages, and the second is the last** — never an
/// extra empty page. The read budget is one no fixture reaches, so no frontier moves `next`.
///
/// Bite (perl, Rust): in `readback::ask_page_of`, `.any(|r| !r.examined || r.frontier)` →
/// `.any(|r| r.examined)` — `next` from "a row was examined" rather than "a row is left": the
/// second page carries a `next`, and a third, empty page follows.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_exact_multiple_of_the_limit_is_that_many_pages(pool: PgPool) {
    let w = world(&pool).await;
    let on = resource(&pool, &w, "exact-multiple").await;
    let mut made = Vec::new();
    for i in 0..4 {
        made.push(commit(&pool, &w, on, &json!({ "unit": "ms", "count": i })).await);
    }
    let budgets = Budgets {
        limit: 2,
        page: ROOMY,
        read: ROOMY_READ,
    };
    let pages = walk(&pool, w.owner, &unit_ms(w.shape), budgets, 4).await;
    assert_eq!(
        pages.iter().map(|p| p.artifacts.len()).collect::<Vec<_>>(),
        [2, 2],
        "two pages, the second the last"
    );
    let read: Vec<Uuid> = pages.iter().flat_map(served).collect();
    assert_eq!(read, ids(&made));
}

// ── §6.1's remaining batch rows: live artifacts, erased families ────────────────────────────────

/// Commit one artifact of the world's family to `on`, superseding `supersedes`.
async fn commit_superseding(
    pool: &PgPool,
    w: &World,
    on: Uuid,
    content: &Value,
    supersedes: &[DataArtifactId],
) -> DataArtifactId {
    writes::commit_data_artifact(
        pool,
        CommitDataArtifactParams {
            resource: ResourceId::from(on),
            kind: &w.family.1,
            kind_owner: Some(w.family.0),
            intent: ArtifactIntent::Member,
            precedence: 0.0,
            content,
            supersedes,
            emitter: w.emitter,
        },
    )
    .await
    .unwrap()
}

/// The governance relation with `cut` removed, read through its states form and the one body by a
/// page: the variants are named `{name}_governed`, `{name}_states`, `{name}_rows` and `name`.
async fn page_over_governance(conn: &mut PgConnection, name: &str, cut: &str) {
    let governed = format!("{name}_governed");
    let states = format!("{name}_states");
    let rows = format!("{name}_rows");
    variant(
        conn,
        GOVERNED_IN,
        6,
        GOVERNED_IN_SIGNATURE,
        &governed,
        cut,
        "",
    )
    .await;
    variant(
        conn,
        STATES_IN,
        6,
        STATES_IN_SIGNATURE,
        &states,
        "__temper_ungated_governed_artifacts_in(",
        &format!("pg_temp.{governed}("),
    )
    .await;
    variant(
        conn,
        ROWS,
        ROWS_NARGS,
        ROWS_SIGNATURE,
        &rows,
        "__temper_ungated_governed_artifact_states_in(",
        &format!("pg_temp.{states}("),
    )
    .await;
    page_over(conn, name, &rows).await;
}

/// **A superseded artifact is not in the batch: live artifacts only.** X1 matches and is served,
/// until a commit supersedes it with X2, which does not match; then the page serves nothing. The
/// fold is the only thing that changed, so the governance relation's live-only conjunct is what
/// excludes X1.
///
/// The bite, in the test: that conjunct cut (`AND NOT a.is_folded`), read through the states form
/// and the one body by a page — the page serves the folded X1.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_superseded_artifact_is_not_in_the_batch(pool: PgPool) {
    let w = world(&pool).await;
    let on = resource(&pool, &w, "superseded").await;
    let x1 = commit(&pool, &w, on, &json!({ "unit": "ms" })).await;
    let ask = bound(&unit_ms(w.shape));
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        served_ids_through(&mut conn, PAGE, w.owner, &ask).await,
        set(&[x1]),
        "the control: served while live"
    );

    commit_superseding(&pool, &w, on, &json!({ "unit": "s" }), &[x1]).await;
    let folded: bool = sqlx::query_scalar("SELECT is_folded FROM kb_data_artifacts WHERE id = $1")
        .bind(x1.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(folded, "the fixture: X1 is folded");
    assert!(served_ids_through(&mut conn, PAGE, w.owner, &ask)
        .await
        .is_empty());

    page_over_governance(&mut conn, "page_with_folded", "AND NOT a.is_folded").await;
    assert_eq!(
        served_ids_through(&mut conn, "pg_temp.page_with_folded", w.owner, &ask).await,
        set(&[x1]),
        "the bite: without the live-only conjunct the folded artifact is served"
    );
}

/// **An erased resource's artifact, renamed out of its family, is not in the batch.** Erasure
/// renames every artifact's family to `erased:<asserted_by_event_id>` (`20261008100000`, step 9f)
/// beside setting `is_active = false`; this fixture applies the rename alone, so the resource stays
/// live and visible. R's artifact is served before the rename and not after, so the governance
/// relation's family match is what excludes it.
///
/// The bite, in the test: that match cut (`AND a.artifact_kind    = p_artifact_kind`), read through
/// the states form and the one body by a page — the page serves the renamed artifact.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_erased_family_is_not_in_the_batch(pool: PgPool) {
    let w = world(&pool).await;
    let on = resource(&pool, &w, "renamed").await;
    let x = commit(&pool, &w, on, &json!({ "unit": "ms" })).await;
    let ask = bound(&unit_ms(w.shape));
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        served_ids_through(&mut conn, PAGE, w.owner, &ask).await,
        set(&[x]),
        "the control: served before the rename"
    );

    sqlx::query(
        "UPDATE kb_data_artifacts SET artifact_kind = 'erased:' || asserted_by_event_id::text \
          WHERE id = $1",
    )
    .bind(x.uuid())
    .execute(&pool)
    .await
    .unwrap();
    assert!(served_ids_through(&mut conn, PAGE, w.owner, &ask)
        .await
        .is_empty());

    page_over_governance(
        &mut conn,
        "page_any_family",
        "AND a.artifact_kind    = p_artifact_kind",
    )
    .await;
    assert_eq!(
        served_ids_through(&mut conn, "pg_temp.page_any_family", w.owner, &ask).await,
        set(&[x]),
        "the bite: without the family match the renamed artifact is served"
    );
}
