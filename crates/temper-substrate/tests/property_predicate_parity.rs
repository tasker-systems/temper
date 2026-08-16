#![cfg(feature = "artifact-tests")]
//! **The differential witness for the one predicate that exists twice.**
//!
//! Task `01a00675-b111-79b2-8aac-e872f30acdd5`. Isolated ephemeral DB via `MIGRATOR`.
//!
//! ## What this file is, and what it is NOT
//!
//! `EdgeFilter::properties` and `ResourceFilter::properties` carry the same Rust type
//! (`Vec<PropertyPredicate>`), mean the same thing (`property_value @> v`, the value whole), and are
//! implemented **twice in SQL**: inside `__temper_ungated_follow_from`'s `adj` (`20260815000010`)
//! and inside `__temper_ungated_find_resources_with`'s `WHERE` (`20260815000040`).
//!
//! **The two bodies cannot be made one, and that is measured rather than stylistic.**
//! `[measured — 20260808000020:308]` a `LANGUAGE sql STABLE` predicate whose body contains a sublink
//! does not inline: the `EXISTS` loses its Index Only Scan on `uq_kb_properties_active` and becomes
//! a per-row call. `[measured — 2026-08-15]` that index is the access path the live plan actually
//! uses, so the obvious DRY extraction is the one move that must not be made. The *view* layer under
//! them was unified instead (`20260815000050`), because a view is a rewrite rule and flattens —
//! also measured.
//!
//! So this file is the substitute for the extraction that cannot happen. It drives **both**
//! fragments over **one** predicate corpus and asserts identical admit/deny.
//!
//! **Two bodies with a proof of agreement is a weaker thing than one body.** It is worth saying
//! plainly, because "unified" is what this will be remembered as. What a differential witness gets
//! you is that a divergence is caught *at the next test run*; what one body would have got you is
//! that a divergence is *unrepresentable*. This is the weaker guarantee, chosen because the stronger
//! one costs an index scan.
//!
//! ## Why the fixture asserts on its own data before asserting on the predicate
//!
//! The two halves are populated by **different write paths** — `create_resource`'s `PropertyAssert`
//! slice for the resource, `property_set` with a polymorphic owner for the edge. If those two ever
//! store different bytes for the same input, every disagreement below would be attributed to the
//! predicate bodies when its actual cause was upstream of them, and the "proof of agreement" would
//! be measuring the wrong thing. So [`stored_rows`] reads both back and the fixture asserts they are
//! identical **before** any predicate runs.
//!
//! ## Why the corpus is checked for containing both outcomes
//!
//! `assert_eq!(resource_admits, edge_admits)` passes trivially if **everything denies** — which is
//! exactly what a fixture that silently failed to write any property would produce. A green
//! differential over a corpus that never admits proves the two bodies agree about nothing. The
//! denominator is asserted: the corpus must produce at least one admit and at least one deny, or the
//! agreement it reports is vacuous.

mod common;

use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, EdgeHome, SeedAction};
use temper_substrate::ids::{ContextId, EdgeId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes;
use uuid::Uuid;

// ── Harness ─────────────────────────────────────────────────────────────────────────────────────

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

async fn ctx(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

/// A resource carrying an arbitrary property set, through the shipped write path.
///
/// `properties` is the slice `create_resource` fires one `PropertyAssert` per member of, so a value
/// written here takes the same route a real assertion takes — including
/// `_property_value_normalized`. A fixture inserting into `kb_properties` directly would witness the
/// predicate against a grain nothing produces.
///
/// **No embedding, deliberately.** `__temper_ungated_follow_from` is a pure edge walk over
/// `kb_edges` and the visible set — it reads no vector — so an embedded fixture here would suggest
/// the walk depends on something it does not.
async fn resource_with(
    pool: &sqlx::PgPool,
    home: ContextId,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    properties: &[(String, serde_json::Value)],
) -> ResourceId {
    writes::create_resource(
        pool,
        writes::CreateParams {
            idempotency_key: None,
            sources: vec![],
            title,
            origin_uri: &format!("test://parity/{title}"),
            body: "A body, because every resource has one.",
            doc_type: "concept",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties,
            chunks: None,
        },
    )
    .await
    .unwrap()
}

/// One edge src→tgt, and the properties it owns — through `property_set`'s polymorphic owner
/// (`20260727000030`), which is the only write path an edge-owned property has.
async fn edge_with(
    pool: &sqlx::PgPool,
    src: ResourceId,
    tgt: ResourceId,
    home: ContextId,
    emitter: EntityId,
    properties: &[(String, serde_json::Value)],
) -> EdgeId {
    let mut tx = pool.begin().await.unwrap();
    let id = fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src,
            tgt,
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

    for (key, value) in properties {
        sqlx::query("SELECT property_set($1::jsonb, $2)")
            .bind(serde_json::json!({
                "property_id": Uuid::now_v7(),
                "owner": { "table": "kb_edges", "id": id.uuid() },
                "property_key": key,
                "value": value,
                "weight": 1.0,
            }))
            .bind(emitter.uuid())
            .execute(pool)
            .await
            .expect("the edge-owned property write path is shipped and takes a polymorphic owner");
    }
    id
}

/// The live `(key, value)` rows one owner carries, in key order — read from `kb_owner_properties`,
/// the relation `20260815000050` put under both scoped views.
async fn stored_rows(
    pool: &sqlx::PgPool,
    owner_table: &str,
    owner_id: Uuid,
) -> Vec<(String, serde_json::Value)> {
    use sqlx::Row;
    sqlx::query(
        "SELECT property_key, property_value FROM kb_owner_properties \
         WHERE owner_table = $1 AND owner_id = $2 ORDER BY property_key, property_value::text",
    )
    .bind(owner_table)
    .bind(owner_id)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| {
        (
            r.get::<String, _>("property_key"),
            r.get::<serde_json::Value, _>("property_value"),
        )
    })
    .collect()
}

/// Does the RESOURCE fragment admit `subject` under this predicate list?
async fn resource_admits(
    pool: &sqlx::PgPool,
    principal: ProfileId,
    subject: ResourceId,
    predicates: Option<serde_json::Value>,
) -> bool {
    use sqlx::Row;
    sqlx::query(
        "SELECT resource_id FROM query_find_resources_with(\
         $1, NULL::text[], NULL::text[], NULL::jsonb, NULL::text, NULL::text, \
         NULL::uuid, NULL::text, NULL::text, NULL::varchar, NULL::uuid, $2::jsonb)",
    )
    .bind(principal.uuid())
    .bind(predicates)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .any(|r| r.get::<Uuid, _>("resource_id") == subject.uuid())
}

/// Does the EDGE fragment admit the hop — i.e. is `target` still reachable under this predicate
/// list?
///
/// The edge predicate gates TRAVERSAL, so "admitted" is the neighbour arriving rather than a row
/// being returned. That difference in what admission *looks like* is precisely what makes this a
/// differential test rather than a comparison of two result sets.
async fn edge_admits(
    pool: &sqlx::PgPool,
    principal: ProfileId,
    seed: ResourceId,
    target: ResourceId,
    predicates: Option<serde_json::Value>,
) -> bool {
    use sqlx::Row;
    sqlx::query(
        "SELECT resource_id FROM query_follow_from(\
         $1, $2::uuid[], 2, 0.5, NULL::text[], NULL::text[], NULL::uuid[], NULL::int, $3::jsonb)",
    )
    .bind(principal.uuid())
    .bind(vec![seed.uuid()])
    .bind(predicates)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .any(|r| r.get::<Uuid, _>("resource_id") == target.uuid())
}

fn p(k: &str, v: serde_json::Value) -> (String, serde_json::Value) {
    (k.to_string(), v)
}

fn contains(key: &str, values: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "key": key, "op": { "op": "contains", "values": values } })
}

fn has_key(key: &str) -> serde_json::Value {
    serde_json::json!({ "key": key, "op": { "op": "has_key" } })
}

/// One `Compare` predicate in the wire shape the fragment parses — `PropertyOp` is internally
/// tagged inside a field named `op`, with `direction` and `value` inside the `compare` arm.
fn compare(key: &str, direction: &str, value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "key": key, "op": { "op": "compare", "direction": direction, "value": value } })
}

/// The property set BOTH halves carry.
///
/// Chosen to exercise the shapes the grain ruling turns on rather than a tidy fixture: an
/// array-shaped value (which the element grain would have matched differently), an EMPTY array
/// (which the element relation cannot see at all — eleven such rows on prod), an object, and a bare
/// scalar. `tags` and `facet` are deliberately ABSENT: both take key-specific branches in the
/// projector (`_property_value_normalized`, and the `facet` inner-key split), so including them
/// would vary the two halves' stored bytes by write path and put a difference into the fixture that
/// the predicate bodies would then be blamed for.
fn subject_properties() -> Vec<(String, serde_json::Value)> {
    vec![
        p("confidence", serde_json::json!("high")),
        p("derived_from", serde_json::json!(["spec-a", "spec-b"])),
        p("empty_list", serde_json::json!([])),
        p("meta", serde_json::json!({"a": 1, "b": 2})),
        // `[added — 2026-08-16]` A numeric value, so the `compare` type guard exercises BOTH
        // directions: a numeric bound matches here and a string bound misses (honest empty), and
        // the reverse against the string-valued `confidence` key. This is the `temper-pr` shape
        // (mixed on one key) the type guard exists to make safe — `seq` stands in for the numeric
        // sub-population; `confidence` for the string one.
        p("seq", serde_json::json!(42)),
    ]
}

/// One predicate list, and what it is for. The `None` entry is the "narrows nothing" case, which
/// both fragments must treat as an absent argument rather than as an empty one.
fn corpus() -> Vec<(&'static str, Option<serde_json::Value>)> {
    vec![
        ("no predicate at all narrows nothing", None),
        (
            "contains, scalar, matching",
            Some(serde_json::json!([contains(
                "confidence",
                serde_json::json!(["high"])
            )])),
        ),
        (
            "contains, scalar, missing",
            Some(serde_json::json!([contains(
                "confidence",
                serde_json::json!(["low"])
            )])),
        ),
        (
            "contains, OR within one predicate's values",
            Some(serde_json::json!([contains(
                "confidence",
                serde_json::json!(["low", "high"])
            )])),
        ),
        (
            "has_key, present",
            Some(serde_json::json!([has_key("confidence")])),
        ),
        (
            "has_key, absent",
            Some(serde_json::json!([has_key("nonesuch")])),
        ),
        (
            "has_key on an EMPTY-ARRAY value — the row the element grain cannot see",
            Some(serde_json::json!([has_key("empty_list")])),
        ),
        (
            "contains against an empty-array value matches nothing",
            Some(serde_json::json!([contains(
                "empty_list",
                serde_json::json!(["anything"])
            )])),
        ),
        (
            "contains against an ARRAY-shaped value, matching element",
            Some(serde_json::json!([contains(
                "derived_from",
                serde_json::json!(["spec-a"])
            )])),
        ),
        (
            "contains against an ARRAY-shaped value, missing element",
            Some(serde_json::json!([contains(
                "derived_from",
                serde_json::json!(["spec-z"])
            )])),
        ),
        (
            "contains against an OBJECT value, matching subset",
            Some(serde_json::json!([contains(
                "meta",
                serde_json::json!([{"a": 1}])
            )])),
        ),
        (
            "contains against an OBJECT value, wrong value for the key",
            Some(serde_json::json!([contains(
                "meta",
                serde_json::json!([{"a": 2}])
            )])),
        ),
        (
            "AND across the list, both matching",
            Some(serde_json::json!([
                contains("confidence", serde_json::json!(["high"])),
                has_key("derived_from")
            ])),
        ),
        (
            "AND across the list, one missing",
            Some(serde_json::json!([
                contains("confidence", serde_json::json!(["high"])),
                has_key("nonesuch")
            ])),
        ),
        (
            "an operator the closed set lacks falls to ELSE false",
            Some(serde_json::json!([
                {"key": "confidence", "op": {"op": "matches_regex"}}
            ])),
        ),
        // `Compare` — the ordering operator. Type-guarded jsonb native ordering; cross-type rows
        // are honest empties. `confidence` is the string sub-population, `seq` is the numeric one,
        // so the type-guard cases exercise the `temper-pr`-shaped hazard in both directions.
        (
            "compare, string, gte, matching (high >= high)",
            Some(serde_json::json!([compare(
                "confidence",
                "gte",
                serde_json::json!("high")
            )])),
        ),
        (
            "compare, string, gt, missing (high > high is false)",
            Some(serde_json::json!([compare(
                "confidence",
                "gt",
                serde_json::json!("high")
            )])),
        ),
        (
            "compare, string, lt, matching (high < i)",
            Some(serde_json::json!([compare(
                "confidence",
                "lt",
                serde_json::json!("i")
            )])),
        ),
        (
            "compare, numeric, gte, matching (42 >= 42)",
            Some(serde_json::json!([compare(
                "seq",
                "gte",
                serde_json::json!(42)
            )])),
        ),
        (
            "compare, numeric, gt, missing (42 > 42 is false)",
            Some(serde_json::json!([compare(
                "seq",
                "gt",
                serde_json::json!(42)
            )])),
        ),
        (
            "compare, numeric, lt, matching (42 < 100)",
            Some(serde_json::json!([compare(
                "seq",
                "lt",
                serde_json::json!(100)
            )])),
        ),
        (
            "compare, TYPE GUARD: numeric bound against string key → honest empty",
            Some(serde_json::json!([compare(
                "confidence",
                "gte",
                serde_json::json!(42)
            )])),
        ),
        (
            "compare, TYPE GUARD: string bound against numeric key → honest empty",
            Some(serde_json::json!([compare(
                "seq",
                "gte",
                serde_json::json!("42")
            )])),
        ),
        (
            "compare, malformed: missing value fails closed (bound is NULL → guard is falsy)",
            Some(serde_json::json!([
                {"key": "confidence", "op": {"op": "compare", "direction": "gt"}}
            ])),
        ),
        (
            "compare, malformed: missing direction falls to inner ELSE false",
            Some(serde_json::json!([
                {"key": "confidence", "op": {"op": "compare", "value": "high"}}
            ])),
        ),
        (
            "compare, malformed: unknown direction falls to inner ELSE false",
            Some(serde_json::json!([
                {"key": "confidence", "op": {"op": "compare", "direction": "between", "value": "high"}}
            ])),
        ),
        (
            "malformed: not an array at the top level",
            Some(serde_json::json!({"key": "confidence", "op": {"op": "has_key"}})),
        ),
        (
            "malformed: `values` is not an array",
            Some(serde_json::json!([
                {"key": "confidence", "op": {"op": "contains", "values": "high"}}
            ])),
        ),
        (
            "an empty predicate list narrows nothing",
            Some(serde_json::json!([])),
        ),
    ]
}

// ── The witness ─────────────────────────────────────────────────────────────────────────────────

/// **The differential.** One corpus, both fragments, identical admit/deny required.
///
/// This is the acceptance criterion the SQL half of the parity task gets INSTEAD of a shared
/// function, and the reason is measured — see this file's header.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn both_property_predicate_bodies_admit_and_deny_identically(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "parity").await;

    let props = subject_properties();

    // The resource half's subject: a resource CARRYING the property set.
    let subject = resource_with(&pool, home, owner, emitter, "subject", &props).await;
    // The edge half's subject: an edge carrying the SAME set, on the only hop between two
    // property-free resources, so the walk's outcome turns on the edge predicate and nothing else.
    let seed = resource_with(&pool, home, owner, emitter, "seed", &[]).await;
    let target = resource_with(&pool, home, owner, emitter, "target", &[]).await;
    let hop = edge_with(&pool, seed, target, home, emitter, &props).await;

    // **The fixture proves itself before it proves anything else.** Two different write paths
    // populated these; if they ever store different bytes, every disagreement below would be
    // attributed to the predicate bodies when the cause was upstream of them.
    let resource_rows = stored_rows(&pool, "kb_resources", subject.uuid()).await;
    let edge_rows = stored_rows(&pool, "kb_edges", hop.uuid()).await;

    // **`create_resource` writes one property the edge has no analogue for**: `doc_type`, which is a
    // resource's kind and not something an edge has. `[found by this assertion — 2026-08-15]` It is
    // named rather than filtered blind, so a *second* asymmetry appearing later fails here instead
    // of being absorbed. The corpus below therefore must not narrow on `doc_type`: it would
    // legitimately differ between the halves, and a differential test cannot tell "legitimately
    // different data" from "drifted predicate".
    let fixture_keys: Vec<&str> = props.iter().map(|(k, _)| k.as_str()).collect();
    let resource_extra: Vec<&String> = resource_rows
        .iter()
        .map(|(k, _)| k)
        .filter(|k| !fixture_keys.contains(&k.as_str()))
        .collect();
    assert_eq!(
        resource_extra,
        vec!["doc_type"],
        "a resource carries exactly one property the fixture did not write — its doc_type. If this \
         set has grown, the new key must be understood before it is excluded: an asymmetry the two \
         halves cannot both express is a place the differential below stops meaning anything"
    );

    let shared = |rows: &[(String, serde_json::Value)]| -> Vec<(String, serde_json::Value)> {
        rows.iter()
            .filter(|(k, _)| fixture_keys.contains(&k.as_str()))
            .cloned()
            .collect()
    };
    assert_eq!(
        shared(&resource_rows),
        shared(&edge_rows),
        "the two halves must carry byte-identical property rows for every key the fixture wrote, \
         or this file is comparing write paths rather than predicate bodies"
    );
    assert_eq!(
        shared(&resource_rows).len(),
        props.len(),
        "every property in the fixture reached storage — a subject that silently carries nothing \
         would make the whole corpus deny on both sides and the differential vacuously green"
    );

    // The differential itself.
    let mut verdicts = Vec::new();
    for (label, predicates) in corpus() {
        let via_resource = resource_admits(&pool, owner, subject, predicates.clone()).await;
        let via_edge = edge_admits(&pool, owner, seed, target, predicates.clone()).await;
        assert_eq!(
            via_resource,
            via_edge,
            "the two predicate bodies disagree on `{label}`: the resource fragment \
             {} and the edge fragment {}. They are two copies of one predicate and cannot be made \
             one (see this file's header), so agreement is asserted rather than structural — this \
             failure means they have drifted",
            if via_resource { "admits" } else { "denies" },
            if via_edge { "admits" } else { "denies" }
        );
        verdicts.push((label, via_resource));
    }

    // **The denominator.** `assert_eq!` over a corpus that denies everything passes while proving
    // the two bodies agree about nothing — which is exactly what a fixture that failed to write its
    // properties would produce. Both outcomes must occur.
    let admitted = verdicts.iter().filter(|(_, v)| *v).count();
    let denied = verdicts.len() - admitted;
    assert!(
        admitted > 0 && denied > 0,
        "the corpus must exercise BOTH outcomes or the agreement it reports is vacuous; \
         got {admitted} admitted and {denied} denied across {} predicates",
        verdicts.len()
    );
}

/// The differential above compares the two bodies to **each other**. This one pins what they agree
/// ON, so a change that breaks both identically cannot pass by staying symmetric.
///
/// Symmetry is the blind spot of any differential test, and it is a real risk here rather than a
/// theoretical one: the two bodies are currently textually identical modulo four substitutions, so
/// the natural way to edit one is to edit both the same way. A corpus asserted only against itself
/// would wave that through.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_shared_predicate_admits_and_denies_the_cases_it_is_supposed_to(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "absolute").await;
    let props = subject_properties();
    let subject = resource_with(&pool, home, owner, emitter, "subject", &props).await;

    // Expected verdicts, stated independently of either implementation. `has_key` on an empty-array
    // value and `contains` against an array-shaped value are the two the GRAIN ruling turns on:
    // both fail under the element grain, which is why they are here rather than in a comment.
    let expected: Vec<(&str, Option<serde_json::Value>, bool)> = vec![
        ("no predicate", None, true),
        ("empty list", Some(serde_json::json!([])), true),
        (
            "contains scalar hit",
            Some(serde_json::json!([contains(
                "confidence",
                serde_json::json!(["high"])
            )])),
            true,
        ),
        (
            "contains scalar miss",
            Some(serde_json::json!([contains(
                "confidence",
                serde_json::json!(["low"])
            )])),
            false,
        ),
        (
            "has_key on an empty array is TRUE — the whole grain sees the row",
            Some(serde_json::json!([has_key("empty_list")])),
            true,
        ),
        (
            "contains inside an array-shaped value",
            Some(serde_json::json!([contains(
                "derived_from",
                serde_json::json!(["spec-a"])
            )])),
            true,
        ),
        (
            "an unknown operator denies rather than raising",
            Some(serde_json::json!([{"key": "confidence", "op": {"op": "matches_regex"}}])),
            false,
        ),
        (
            "a malformed list fails CLOSED",
            Some(serde_json::json!({"key": "confidence", "op": {"op": "has_key"}})),
            false,
        ),
        // `Compare` expected verdicts. The type-guard cases are the load-bearing ones: they pin
        // that a cross-type bound is an honest empty (ELSE false), not a type-confusion match.
        // A symmetric mistake — both SQL bodies wrong identically — is what THIS test catches,
        // and the type-guard cases are where it would show.
        (
            "compare gte hit (high >= high)",
            Some(serde_json::json!([compare(
                "confidence",
                "gte",
                serde_json::json!("high")
            )])),
            true,
        ),
        (
            "compare gt miss (high > high is false)",
            Some(serde_json::json!([compare(
                "confidence",
                "gt",
                serde_json::json!("high")
            )])),
            false,
        ),
        (
            "compare numeric gte hit (42 >= 42)",
            Some(serde_json::json!([compare(
                "seq",
                "gte",
                serde_json::json!(42)
            )])),
            true,
        ),
        (
            "compare type guard: numeric bound vs string key → honest empty",
            Some(serde_json::json!([compare(
                "confidence",
                "gte",
                serde_json::json!(42)
            )])),
            false,
        ),
        (
            "compare type guard: string bound vs numeric key → honest empty",
            Some(serde_json::json!([compare(
                "seq",
                "gte",
                serde_json::json!("42")
            )])),
            false,
        ),
        (
            "compare malformed: missing value fails closed",
            Some(serde_json::json!([
                {"key": "confidence", "op": {"op": "compare", "direction": "gt"}}
            ])),
            false,
        ),
        (
            "compare malformed: unknown direction falls to inner ELSE false",
            Some(serde_json::json!([
                {"key": "confidence", "op": {"op": "compare", "direction": "between", "value": "high"}}
            ])),
            false,
        ),
    ];

    for (label, predicates, want) in expected {
        let got = resource_admits(&pool, owner, subject, predicates).await;
        assert_eq!(
            got, want,
            "`{label}`: expected admit={want}, got admit={got}"
        );
    }
}
