#![cfg(feature = "artifact-tests")]
//! The selection mechanic: `query_find_resources_with` narrows by what a resource IS, ranks
//! nothing, and truncates nothing.
//!
//! Task `01a0003c-7468-7b31-b6b3-81913b78d150`, beat 2. Migration
//! `20260814000010_find_resources_with.sql`. Isolated ephemeral DB via `MIGRATOR`.
//!
//! ## What these tests are FOR
//!
//! Six of the seven narrowings have an incumbent — `filtered_visible_page` has run them against
//! real storage since the list endpoint shipped — so for those the question under test is not *does
//! this predicate work* but **does this function mean the same thing the rest of the system means
//! by that word**. A `doc_type` that matched a different set here than `temper resource list`
//! returns would be a silent question substitution across two doors, which is the class this whole
//! surface exists against.
//!
//! `facets` is the seventh and has **no incumbent read anywhere** — `ResourceListParams` carries no
//! facet filter and `FacetPredicate` appears nowhere outside the query contract's own types. Its
//! test is therefore doing different work from its six siblings: it is the first executable
//! statement of what a facet predicate MEANS, rather than a comparison against something that
//! already meant it. Named here because a reader skimming a uniform-looking file would not
//! otherwise know that one of these seven is load-bearing in a way the others are not.
//!
//! ## What is deliberately NOT witnessed here
//!
//! **Composition.** That a selection's ids reach a find act's `p_bound_ids` is witnessed
//! elsewhere, not here: `query_run_composition_test.rs::a_selection_bounds_the_find_act_it_is_piped_into`
//! drives the real compiled statement end to end and reads the trace.
//!
//! `[corrected — 2026-08-14]` This said composition "is beat 3's, because nothing compiles this act
//! yet — `validate` refuses it as `not_implemented`". Beat 3 landed **on this same branch**: the act
//! is `Served`, it is in `CALLABLE_FRAGMENTS`, and the pipe is exercised. The paragraph outlived its
//! subject by two commits, which is how a file comes to understate its own coverage.
//!
//! What stays true is the reason not to assert composition *here*: doing so would need a
//! hand-written statement that is not the one the compiler emits, which witnesses the test's own SQL
//! rather than the system's.

mod common;

use temper_substrate::ids::{ContextId, EntityId, ProfileId};
use temper_substrate::payloads::AnchorRef;
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

async fn ctx(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

/// A resource with a chosen `doc_type` and an arbitrary property set.
///
/// `properties` is the same slice `create_resource` fires one `PropertyAssert` per member of, so a
/// `facet` written here takes the SAME write path a real `facet_set` takes — including the
/// inner-key split of `20260730000010`. A fixture that inserted into `kb_properties` directly would
/// witness this function against a grain nothing produces.
async fn mk(
    pool: &sqlx::PgPool,
    home: AnchorRef,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    doc_type: &str,
    properties: &[(String, serde_json::Value)],
) -> Uuid {
    writes::create_resource(
        pool,
        writes::CreateParams {
            idempotency_key: None,
            sources: vec![],
            title,
            origin_uri: &format!("test://{title}"),
            body: "A body, because every resource has one.",
            doc_type,
            home,
            owner,
            originator: owner,
            emitter,
            properties,
            chunks: None,
        },
    )
    .await
    .unwrap()
    .uuid()
}

/// Every narrowing the wrapper takes, so each test names only what it varies.
#[derive(Default)]
struct Narrow<'a> {
    doc_types: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    facets: Option<serde_json::Value>,
    stage: Option<&'a str>,
    status: Option<&'a str>,
    owner_profile: Option<Uuid>,
    owner_handle: Option<&'a str>,
    title_contains: Option<&'a str>,
    anchor: Option<(&'a str, Uuid)>,
    /// The open-key slot (`20260815000040`): the serialization of `Vec<PropertyPredicate>`.
    properties: Option<serde_json::Value>,
}

/// Ids from `query_find_resources_with`, sorted so a comparison never depends on an order this act
/// explicitly does not have.
async fn select(pool: &sqlx::PgPool, principal: ProfileId, n: Narrow<'_>) -> Vec<Uuid> {
    use sqlx::Row;
    let (anchor_table, anchor_id) = match n.anchor {
        Some((t, i)) => (Some(t), Some(i)),
        None => (None, None),
    };
    let mut ids: Vec<Uuid> = sqlx::query(
        "SELECT resource_id FROM query_find_resources_with($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    )
    .bind(principal.uuid())
    .bind(n.doc_types)
    .bind(n.tags)
    .bind(n.facets)
    .bind(n.stage)
    .bind(n.status)
    .bind(n.owner_profile)
    .bind(n.owner_handle)
    .bind(n.title_contains)
    .bind(anchor_table)
    .bind(anchor_id)
    .bind(n.properties)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<Uuid, _>("resource_id"))
    .collect();
    ids.sort();
    ids
}

fn sorted(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort();
    ids
}

fn prop(k: &str, v: serde_json::Value) -> (String, serde_json::Value) {
    (k.to_string(), v)
}

// ── The capability that did not exist ───────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_selection_with_no_narrowing_at_all_returns_the_whole_visible_corpus(pool: sqlx::PgPool) {
    // **THE capability this act adds.** Every find act requires an intention, so "all resources I
    // can see" — and by extension "all resources with property P" — was inexpressible: there was no
    // question-about-meaning to put in the envelope. A call with every narrowing NULL is the
    // legitimate degenerate case, not a malformed one, which is also why the wrapper carries no
    // guaranteed-empty CASE guard.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "everything").await;

    let a = mk(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "One",
        "concept",
        &[],
    )
    .await;
    let b = mk(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "Two",
        "task",
        &[],
    )
    .await;

    let got = select(&pool, owner, Narrow::default()).await;
    for id in [a, b] {
        assert!(
            got.contains(&id),
            "an unnarrowed selection returns everything visible; {id} is missing"
        );
    }
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn doc_type_takes_more_than_one_value(pool: sqlx::PgPool) {
    // The modifier could not do this. The find fragments' `p_doc_type` is a single `text`, so
    // `validate` refuses a multi-value doc-type filter as INEXPRESSIBLE rather than unimplemented —
    // narrowing to the first of them would answer a different question and look like a success.
    // Within the field the semantics are OR; the AND is across fields.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "doctypes").await);

    let task = mk(&pool, home, owner, emitter, "T", "task", &[]).await;
    let session = mk(&pool, home, owner, emitter, "S", "session", &[]).await;
    let concept = mk(&pool, home, owner, emitter, "C", "concept", &[]).await;

    let got = select(
        &pool,
        owner,
        Narrow {
            doc_types: Some(vec!["task".into(), "session".into()]),
            ..Default::default()
        },
    )
    .await;

    assert_eq!(got, sorted(vec![task, session]), "OR within the field");
    assert!(!got.contains(&concept));
}

// ── The six with an incumbent: does this mean what `list` means? ─────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn stage_and_status_read_the_workflow_pivot_not_a_bare_property(pool: sqlx::PgPool) {
    // `stage` and `status` are not properties in their own right — they are `temper-stage` and
    // `temper-status` surfaced through `kb_resource_workflow_props`. Reading them through the view
    // is what keeps those key names in one place; a test that asserted on `kb_properties` directly
    // would pass against a function that had restated them and drifted.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "workflow").await);

    let active = mk(
        &pool,
        home,
        owner,
        emitter,
        "Active",
        "task",
        &[prop("temper-stage", serde_json::json!("in-progress"))],
    )
    .await;
    let backlog = mk(
        &pool,
        home,
        owner,
        emitter,
        "Backlog",
        "task",
        &[prop("temper-stage", serde_json::json!("backlog"))],
    )
    .await;
    let open = mk(
        &pool,
        home,
        owner,
        emitter,
        "Open",
        "goal",
        &[prop("temper-status", serde_json::json!("active"))],
    )
    .await;

    let by_stage = select(
        &pool,
        owner,
        Narrow {
            stage: Some("in-progress"),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(by_stage, vec![active]);
    assert!(!by_stage.contains(&backlog));

    let by_status = select(
        &pool,
        owner,
        Narrow {
            status: Some("active"),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(by_status, vec![open]);
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn owner_matches_by_resolved_profile_id_and_by_handle(pool: sqlx::PgPool) {
    // TWO spellings, which is what `filtered_visible_page` binds: a resolved profile id (what `@me`
    // becomes) and a handle. Independent slots rather than one polymorphic parameter, so neither has
    // to be parsed to know which it is. Both must select the same rows or the two doors disagree
    // about who owns what.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "owners").await);

    let mine = mk(&pool, home, owner, emitter, "Mine", "concept", &[]).await;

    let by_id = select(
        &pool,
        owner,
        Narrow {
            owner_profile: Some(owner.uuid()),
            ..Default::default()
        },
    )
    .await;
    let by_handle = select(
        &pool,
        owner,
        Narrow {
            owner_handle: Some("system"),
            ..Default::default()
        },
    )
    .await;

    assert!(by_id.contains(&mine));
    assert_eq!(by_id, by_handle, "the two spellings name the same set");

    let stranger = select(
        &pool,
        owner,
        Narrow {
            owner_handle: Some("nobody"),
            ..Default::default()
        },
    )
    .await;
    assert!(
        stranger.is_empty(),
        "an owner nobody has is an honest empty, not everything"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn title_contains_is_a_case_insensitive_substring(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "titles").await);

    let hit = mk(
        &pool,
        home,
        owner,
        emitter,
        "Peregrine Falcon",
        "concept",
        &[],
    )
    .await;
    let miss = mk(&pool, home, owner, emitter, "Cartography", "concept", &[]).await;

    let got = select(
        &pool,
        owner,
        Narrow {
            title_contains: Some("peregrine"),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(got, vec![hit], "ILIKE, so the fold is free");
    assert!(!got.contains(&miss));
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn tags_are_and_containment_and_fold_case_on_both_sides(pool: sqlx::PgPool) {
    // Case is a LIVE distinction, not a hypothetical: production carries six tag pairs differing
    // only by case (authn/AuthN, ci/CI, cli/CLI, authz/AuthZ, vercel/Vercel, pr-482/PR-482). The
    // incumbent folds the bind in Rust and the row with `lower(t)`; this function folds both in SQL,
    // so a second caller cannot get half of it wrong. Asserting BOTH directions is the point — a
    // one-sided fold passes a one-sided test.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "tags").await);

    let both = mk(
        &pool,
        home,
        owner,
        emitter,
        "Both",
        "concept",
        &[prop("tags", serde_json::json!(["CI", "auth"]))],
    )
    .await;
    let one = mk(
        &pool,
        home,
        owner,
        emitter,
        "One",
        "concept",
        &[prop("tags", serde_json::json!(["ci"]))],
    )
    .await;
    let untagged = mk(&pool, home, owner, emitter, "None", "concept", &[]).await;

    // Row side folded: the resource stored `CI`, the caller asked for `ci`.
    let lower_query = select(
        &pool,
        owner,
        Narrow {
            tags: Some(vec!["ci".into()]),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(lower_query, sorted(vec![both, one]));

    // Bind side folded: the caller asked for `CI`, one resource stored `ci`.
    let upper_query = select(
        &pool,
        owner,
        Narrow {
            tags: Some(vec!["CI".into()]),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        upper_query, lower_query,
        "the fold is symmetric; a one-sided fold makes these differ"
    );

    // AND, so each added tag narrows.
    let both_tags = select(
        &pool,
        owner,
        Narrow {
            tags: Some(vec!["ci".into(), "auth".into()]),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(both_tags, vec![both], "containment IS the AND semantics");
    assert!(
        !both_tags.contains(&untagged),
        "no `tags` property aggregates to NULL, and NULL @> $ is NULL — correctly excluded"
    );
}

// ── Malformed facet arguments (`20260815000020`) ────────────────────────────────────────────────

/// A malformed `p_facets` narrows to NOTHING, in every shape a caller can send.
///
/// **The control is half the test.** `20260814000010` normalized a non-array argument to `'[]'`,
/// and `NOT EXISTS` over zero elements is TRUE — so every candidate row passed and a caller who
/// asked to narrow received an UNNARROWED page. Without the unfiltered arm below, a body that
/// returned nothing for any reason at all would pass these assertions.
///
/// `[measured on prod — 2026-08-15]` before the fix, over five faceted resources: 5 unfiltered,
/// 5 malformed, 0 for a well-formed non-match.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_malformed_facet_argument_narrows_to_nothing_rather_than_everything(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "malfacet").await);
    let hit = mk(
        &pool,
        home,
        owner,
        emitter,
        "Hit",
        "concept",
        &[prop("facet", serde_json::json!({"domain": "search"}))],
    )
    .await;

    let control = select(&pool, owner, Narrow::default()).await;
    assert!(
        control.contains(&hit),
        "control: with no facet filter the resource is returned"
    );

    // Every non-array shape a caller can put in a jsonb slot.
    for malformed in [
        serde_json::json!({"key": "domain", "value": "search"}), // the object, not wrapped
        serde_json::json!("domain"),
        serde_json::json!(42),
        serde_json::Value::Null,
    ] {
        let got = select(
            &pool,
            owner,
            Narrow {
                facets: Some(malformed.clone()),
                ..Default::default()
            },
        )
        .await;
        assert!(
            got.is_empty(),
            "a malformed facet filter must narrow to nothing, not to everything; \
                 {malformed} returned {got:?}"
        );
    }

    // And the well-formed arms still mean what they meant: the fix must not have closed the door.
    let matched = select(
        &pool,
        owner,
        Narrow {
            facets: Some(serde_json::json!([{"key": "domain", "value": "search"}])),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        matched,
        vec![hit],
        "a well-formed matching predicate still matches"
    );

    // An EMPTY list is not malformed. AND over zero predicates is true, so it narrows nothing —
    // the same reading `p_edge_properties` has, and the one that would be easiest to break here.
    let empty = select(
        &pool,
        owner,
        Narrow {
            facets: Some(serde_json::json!([])),
            ..Default::default()
        },
    )
    .await;
    assert!(
        empty.contains(&hit),
        "an empty predicate list narrows nothing"
    );
}

/// A facet predicate whose element carries no `key` narrows to nothing, and does NOT raise.
///
/// **This is the opposite defect from the one above, in the same slot** `[measured on prod —
/// 2026-08-15]`: `jsonb_build_object` refuses a null key, so a well-formed ARRAY holding a
/// key-less element raised `argument 1: key must not be null` — a caller's malformed filter
/// arriving as a server fault.
///
/// It was **data-dependent**, which is why it needs a faceted resource to witness at all: the
/// inner `EXISTS` short-circuits when a resource carries no live `facet` row, so the same argument
/// was a 500 for a principal who could see one and a silent empty for a principal who could not.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_facet_element_without_a_key_narrows_to_nothing_rather_than_raising(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "keyless").await);
    let hit = mk(
        &pool,
        home,
        owner,
        emitter,
        "Hit",
        "concept",
        &[prop("facet", serde_json::json!({"domain": "search"}))],
    )
    .await;

    // The control that makes the raise reachable: without a live `facet` row the inner EXISTS
    // short-circuits and `jsonb_build_object` is never evaluated, so this test would pass against
    // the unfixed body.
    let control = select(&pool, owner, Narrow::default()).await;
    assert!(
        control.contains(&hit),
        "control: the resource carries a live facet row"
    );

    for keyless in [
        serde_json::json!([{"nope": 1}]),
        serde_json::json!([{"value": "search"}]),
        serde_json::json!([{"key": null, "value": "search"}]),
    ] {
        let got = select(
            &pool,
            owner,
            Narrow {
                facets: Some(keyless.clone()),
                ..Default::default()
            },
        )
        .await;
        assert!(
            got.is_empty(),
            "{keyless} must narrow to nothing; returned {got:?}"
        );
    }
}

// ── The seventh, which has no incumbent ─────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_facet_predicate_matches_at_the_inner_key_grain(pool: sqlx::PgPool) {
    // **The first executable statement of what a facet predicate means.** No read path anywhere
    // filters by facet, so unlike its six siblings this test has no incumbent to agree with — it
    // establishes the meaning rather than checking one.
    //
    // `20260730000010` keys a facet at the INNER key: one `kb_properties` row per inner key holding
    // a single-key object. So a two-key facet written in one assert becomes two rows, and a
    // predicate naming one of them must match — which is the property that would fail if this had
    // been written against the pre-grain whole-object shape.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "facets").await);

    let hit = mk(
        &pool,
        home,
        owner,
        emitter,
        "Hit",
        "concept",
        &[prop(
            "facet",
            serde_json::json!({"domain": "search", "priority": "high"}),
        )],
    )
    .await;
    let other = mk(
        &pool,
        home,
        owner,
        emitter,
        "Other",
        "concept",
        &[prop("facet", serde_json::json!({"domain": "auth"}))],
    )
    .await;

    let got = select(
        &pool,
        owner,
        Narrow {
            facets: Some(serde_json::json!([{"key": "domain", "value": "search"}])),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(got, vec![hit], "one inner key of a multi-key facet matches");
    assert!(!got.contains(&other));

    // AND across the list, spelled as "no listed predicate fails to match".
    let both = select(
        &pool,
        owner,
        Narrow {
            facets: Some(
                serde_json::json!([{"key":"domain","value":"search"},{"key":"priority","value":"high"}]),
            ),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(both, vec![hit]);

    let contradictory = select(
        &pool,
        owner,
        Narrow {
            facets: Some(
                serde_json::json!([{"key":"domain","value":"search"},{"key":"domain","value":"auth"}]),
            ),
            ..Default::default()
        },
    )
    .await;
    assert!(
        contradictory.is_empty(),
        "AND over a list naming one key twice with different values is unsatisfiable, not OR"
    );
}

// ── Composition of narrowings, and the authorization boundary ───────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn narrowings_compose_with_and_across_fields(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "compose").await);

    let both = mk(
        &pool,
        home,
        owner,
        emitter,
        "Both",
        "task",
        &[
            prop("temper-stage", serde_json::json!("in-progress")),
            prop("tags", serde_json::json!(["ci"])),
        ],
    )
    .await;
    let wrong_stage = mk(
        &pool,
        home,
        owner,
        emitter,
        "WrongStage",
        "task",
        &[
            prop("temper-stage", serde_json::json!("backlog")),
            prop("tags", serde_json::json!(["ci"])),
        ],
    )
    .await;
    let wrong_type = mk(
        &pool,
        home,
        owner,
        emitter,
        "WrongType",
        "concept",
        &[
            prop("temper-stage", serde_json::json!("in-progress")),
            prop("tags", serde_json::json!(["ci"])),
        ],
    )
    .await;

    let got = select(
        &pool,
        owner,
        Narrow {
            doc_types: Some(vec!["task".into()]),
            stage: Some("in-progress"),
            tags: Some(vec!["ci".into()]),
            ..Default::default()
        },
    )
    .await;

    assert_eq!(got, vec![both], "each added field narrows");
    for id in [wrong_stage, wrong_type] {
        assert!(!got.contains(&id));
    }
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_anchor_scopes_the_selection_to_one_home(pool: sqlx::PgPool) {
    // The anchor is why this act takes bounds at all. Nothing produces "the resources homed in this
    // context" as a set, so without it `every task in @me/temper` is inexpressible — and that
    // question is most of why the act exists.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let inside = ctx(&pool, owner, "inside").await;
    let outside = ctx(&pool, owner, "outside").await;

    let here = mk(
        &pool,
        AnchorRef::context(inside),
        owner,
        emitter,
        "Here",
        "task",
        &[],
    )
    .await;
    let there = mk(
        &pool,
        AnchorRef::context(outside),
        owner,
        emitter,
        "There",
        "task",
        &[],
    )
    .await;

    let got = select(
        &pool,
        owner,
        Narrow {
            doc_types: Some(vec!["task".into()]),
            anchor: Some(("kb_contexts", inside.uuid())),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(got, vec![here]);
    assert!(!got.contains(&there));
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_unreadable_cogmap_anchor_scopes_nothing_rather_than_erroring(pool: sqlx::PgPool) {
    // Dropping the anchor guard would leak no resource — every row still comes from the caller's
    // visible set — but it would leak MEMBERSHIP of a map the principal cannot read. A genesis'd
    // cogmap is joined to no team, so nobody reads it; the refusal renders as an empty selection
    // rather than an error, which is the existence-oracle rule.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let (map, _telos) = common::genesis_cogmap(&pool, "unreadable", "Why this map exists").await;

    let _homed = mk(
        &pool,
        AnchorRef::cogmap(temper_substrate::ids::CogmapId::from(map)),
        owner,
        emitter,
        "Inside",
        "concept",
        &[],
    )
    .await;

    let got = select(
        &pool,
        owner,
        Narrow {
            anchor: Some(("kb_cogmaps", map)),
            ..Default::default()
        },
    )
    .await;
    assert!(
        got.is_empty(),
        "an unreadable anchor scopes nothing, and says so by returning no rows"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_core_admits_nothing_when_handed_a_null_visible_set(pool: sqlx::PgPool) {
    // `p_visible_ids` is the OPPOSITE polarity from every narrowing beside it: a NULL narrowing
    // narrows nothing, a NULL verdict admits nothing. The two are genuinely confusable — both are
    // arrays, both are optional-looking — which is why the compiler supplies the visible set from a
    // fixed constant rather than an argument. This pins the fail-closed half.
    use sqlx::Row;
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "failclosed").await);
    let _r = mk(&pool, home, owner, emitter, "Visible", "concept", &[]).await;

    let rows =
        sqlx::query("SELECT resource_id FROM __temper_ungated_find_resources_with(NULL::uuid[])")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        rows.is_empty(),
        "a NULL visibility verdict admits nothing, even with every narrowing absent"
    );

    // And the same call with a real verdict returns the row, so the assertion above is about the
    // verdict rather than about an unrelated empty corpus.
    let admitted = sqlx::query(
        "SELECT resource_id FROM __temper_ungated_find_resources_with(\
           ARRAY(SELECT resource_id FROM resources_visible_to($1)))",
    )
    .bind(owner.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        admitted
            .iter()
            .any(|r| r.get::<Uuid, _>("resource_id") == _r),
        "the fixture is visible, so the empty above was the verdict and not the corpus"
    );
}

// ── Regressions from the adversarial security review ────────────────────────────────────────────

/// A bare-string `tags` value does not raise, and is ONE tag.
///
/// `[regression — 2026-08-14]` The **raise** half is the original finding and is unchanged: this
/// once failed for EVERY caller in the deployment, not just the owner of the offending row.
/// open_meta convention v2 declares `tags` an array of strings OR a bare string, and
/// `temper_workflow::schema` asserts both pass validation — so a single schema-VALID resource made
/// `jsonb_array_elements_text` fail with `cannot extract elements from a scalar` and the whole tag
/// filter went down for everyone. Availability, not disclosure; the error names nothing.
///
/// `[inverted — 2026-08-15, §7 ruled by Pete]` The **split** half is reversed. This asserted that
/// `tags: "ci auth"` was filterable by `ci`, *"because that is what the same value already means to
/// FTS — so the two doors do not disagree about one value."* That premise was measured and is
/// false: FTS does not split, it delegates to a tokenizer that splits differently
/// (`to_tsvector('english','ci-auth deploy')` yields `ci`, `auth`, `ci-auth`, `deploy`;
/// `regexp_split_to_array` yields `{ci-auth, deploy}`). The two doors disagreed either way, so the
/// split centralized an answer that did not achieve its own goal. A bare string is now one tag,
/// which is the reading a caller can predict from what they wrote.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_bare_string_tags_value_does_not_raise_and_is_one_tag(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "barestring").await);

    let bare = mk(
        &pool,
        home,
        owner,
        emitter,
        "Bare",
        "concept",
        &[prop("tags", serde_json::json!("ci auth"))],
    )
    .await;
    let arrayed = mk(
        &pool,
        home,
        owner,
        emitter,
        "Arrayed",
        "concept",
        &[prop("tags", serde_json::json!(["ci"]))],
    )
    .await;

    // The call completing at all is still half the assertion — before 2026-08-14 it raised here.
    let by_fragment = select(
        &pool,
        owner,
        Narrow {
            tags: Some(vec!["ci".into()]),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        by_fragment,
        vec![arrayed],
        "`ci` is a fragment of the single tag `ci auth`, not a tag anyone wrote — only the \
         array-shaped resource carries it. The call must still COMPLETE, which is the half of this \
         assertion that has not changed"
    );

    let by_whole = select(
        &pool,
        owner,
        Narrow {
            tags: Some(vec!["ci auth".into()]),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        by_whole,
        vec![bare],
        "and the bare string is reachable as itself — a value matching neither its fragments nor \
         itself would be unreachable, which is worse than the split it replaces"
    );
}

/// A bare-string row that PREDATES the write-time normalization still reads as one tag.
///
/// **This is the only arm here that holds the READ path accountable.** Once
/// `_property_value_normalized` (`20260815000030`) is in the projectors, nothing can store a bare
/// string, so the read-side split becomes unreachable and therefore invisible to every behavioural
/// test — the test above passes with or without it. This one writes the shape the projector can no
/// longer produce, which is exactly the row a deployment predating that migration already holds.
///
/// `[measured on prod — 2026-08-15]` zero such rows exist. Stated in that direction on purpose:
/// that is why the change was cheap, not evidence it was unnecessary.
///
/// Its sibling is `temper-services`' `a_legacy_bare_string_row_reads_as_one_tag_rather_than_its_
/// fragments`, which asserts the same thing of `filtered_visible_page`. Two doors, one value — the
/// disagreement §4 recorded is what both exist to prevent, now that they finally read one relation.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_legacy_bare_string_row_reads_as_one_tag_rather_than_its_fragments(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "legacybare").await);

    let legacy = mk(
        &pool,
        home,
        owner,
        emitter,
        "Legacy",
        "concept",
        &[prop("tags", serde_json::json!(["placeholder"]))],
    )
    .await;

    // Rewrite the stored value in place, leaving every key, owner and event reference exactly as
    // the real write path built them.
    let rewritten = sqlx::query(
        "UPDATE kb_properties SET property_value = '\"ci auth\"'::jsonb \
          WHERE owner_table='kb_resources' AND owner_id=$1 \
            AND property_key='tags' AND NOT is_folded",
    )
    .bind(legacy)
    .execute(&pool)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(
        rewritten, 1,
        "precondition: exactly one live `tags` row to rewrite — a zero here means this probe \
         asserted nothing"
    );

    let by_fragment = select(
        &pool,
        owner,
        Narrow {
            tags: Some(vec!["ci".into()]),
            ..Default::default()
        },
    )
    .await;
    assert!(
        !by_fragment.contains(&legacy),
        "a stored bare string is ONE tag at read time too. Matching `ci` means the fragment is \
         still splitting on whitespace. Got {by_fragment:?}"
    );

    let by_whole = select(
        &pool,
        owner,
        Narrow {
            tags: Some(vec!["ci auth".into()]),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        by_whole,
        vec![legacy],
        "and the legacy row must stay reachable as itself — normalization at write must not orphan \
         the rows written before it"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_second_principal_sees_none_of_another_principals_resources(pool: sqlx::PgPool) {
    // `[added — 2026-08-14]` The suite had NO cross-principal test. Every case used one profile, so
    // the closest thing to a visibility assertion was the degenerate NULL-verdict probe — and a
    // regression that stopped the wrapper scoping at all would have passed the whole file.
    //
    // Both agents that reviewed this verified the property by hand and both named its absence.
    // Verified-by-hand is not held; this holds it.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "mine").await);

    let mine = mk(
        &pool,
        home,
        owner,
        emitter,
        "Private Thing",
        "task",
        &[
            prop("tags", serde_json::json!(["secret"])),
            prop("temper-stage", serde_json::json!("in-progress")),
            prop("derived_from", serde_json::json!(["secret-spec"])),
        ],
    )
    .await;

    let stranger = ProfileId::from(common::insert_profile(&pool, "stranger").await);

    // Every spelling, because a leak through any one of them is a leak.
    for narrow in [
        Narrow::default(),
        Narrow {
            doc_types: Some(vec!["task".into()]),
            ..Default::default()
        },
        Narrow {
            tags: Some(vec!["secret".into()]),
            ..Default::default()
        },
        Narrow {
            stage: Some("in-progress"),
            ..Default::default()
        },
        Narrow {
            title_contains: Some("Private"),
            ..Default::default()
        },
        Narrow {
            owner_handle: Some("system"),
            ..Default::default()
        },
        // **The open-key slot** `[2026-08-15]`. It is the spelling most worth asserting, because
        // it is the one whose predicate reads a SECOND relation: the value lives in
        // `kb_resource_properties`, not on `kb_resources`. The correlation is `rp.resource_id =
        // r.id` and `r` is already joined to `unnest(p_visible_ids)`, so the predicate can only
        // read properties of rows the verdict already admitted — but "can only" is the kind of
        // claim `audit-ungated-fragments.sh` exists to make someone witness rather than reason
        // about, since a leak here would be an existence oracle over an arbitrary caller-chosen
        // key.
        Narrow {
            properties: Some(serde_json::json!([contains(
                "derived_from",
                serde_json::json!(["secret-spec"])
            )])),
            ..Default::default()
        },
        Narrow {
            properties: Some(serde_json::json!([has_key("derived_from")])),
            ..Default::default()
        },
    ] {
        let got = select(&pool, stranger, narrow).await;
        assert!(
            !got.contains(&mine),
            "a stranger reached another principal's resource; got {got:?}"
        );
    }

    // Positive control: the owner still sees it, so the empties above are the gate and not an
    // empty corpus.
    let owner_sees = select(
        &pool,
        owner,
        Narrow {
            tags: Some(vec!["secret".into()]),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(owner_sees, vec![mine], "the fixture must be real");

    // And the open-key spelling specifically, so its empty above is the visibility gate rather
    // than a predicate that matches nothing for anyone.
    let owner_sees_by_property = select(
        &pool,
        owner,
        Narrow {
            properties: Some(serde_json::json!([contains(
                "derived_from",
                serde_json::json!(["secret-spec"])
            )])),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(owner_sees_by_property, vec![mine]);
}

// ── The open-key slot (`20260815000040`) ────────────────────────────────────────────────────────
//
// Sixty-seven of the seventy live property keys were unreachable by any narrowing on any act. These
// witness the slot that reaches them, and — more importantly — the GRAIN ruling behind it: the
// predicate reads `kb_resource_properties` (value whole), never `kb_property_elements`. Three of
// these arms fail under the element grain and are the reason the ruling is a ruling.

/// One `PropertyPredicate`, in the wire shape the fragment parses (`PropertyOp` is internally
/// tagged inside a field named `op`).
fn contains(key: &str, values: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "key": key, "op": { "op": "contains", "values": values } })
}

fn has_key(key: &str) -> serde_json::Value {
    serde_json::json!({ "key": key, "op": { "op": "has_key" } })
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_key_that_is_none_of_doc_type_tags_or_facet_is_narrowable(pool: sqlx::PgPool) {
    // Acceptance criterion 1, witnessed against a real one of the sixty-seven rather than a
    // synthetic key: `derived_from` carries 112 array-shaped and 21 string-shaped rows on prod
    // `[measured — 2026-08-15]`, so it exercises type-instability rather than a tidy fixture.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "openkey").await);

    let derived = mk(
        &pool,
        home,
        owner,
        emitter,
        "Derived",
        "concept",
        &[prop("derived_from", serde_json::json!(["spec-a"]))],
    )
    .await;
    let other = mk(
        &pool,
        home,
        owner,
        emitter,
        "Other",
        "concept",
        &[prop("derived_from", serde_json::json!(["spec-b"]))],
    )
    .await;
    let none = mk(&pool, home, owner, emitter, "None", "concept", &[]).await;

    let got = select(
        &pool,
        owner,
        Narrow {
            properties: Some(serde_json::json!([contains(
                "derived_from",
                serde_json::json!(["spec-a"])
            )])),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(got, vec![derived]);
    assert!(!got.contains(&other) && !got.contains(&none));

    // The fixture is real and the slot NARROWS rather than merely erroring into emptiness: with no
    // predicate at all all three come back. A superset check, not an equality — `seed_system` also
    // seeds the L0 telos resource, which is legitimately visible here.
    let unnarrowed = select(&pool, owner, Narrow::default()).await;
    for id in [derived, other, none] {
        assert!(unnarrowed.contains(&id), "the fixture must be real");
    }
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_array_shaped_probe_matches_an_array_shaped_row(pool: sqlx::PgPool) {
    // **This is the grain ruling's witness, and it FAILS under the element grain.**
    // `'["a"]'::jsonb @> '["a"]'` is true; `'"a"'::jsonb @> '["a"]'` is false — so exploding the
    // row to elements first turns this probe from "matches" into "matches nothing", silently, for
    // the 1,228 array-shaped rows on prod `[measured — 2026-08-15]`.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "grain").await);

    let arr = mk(
        &pool,
        home,
        owner,
        emitter,
        "Array",
        "concept",
        &[prop(
            "derived_from",
            serde_json::json!(["spec-a", "spec-b"]),
        )],
    )
    .await;

    let got = select(
        &pool,
        owner,
        Narrow {
            properties: Some(serde_json::json!([contains(
                "derived_from",
                serde_json::json!([["spec-a"]])
            )])),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        got,
        vec![arr],
        "an array-shaped probe must match the whole value; matching nothing is the element grain"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_scalar_probe_spans_both_populations_of_a_type_unstable_key(pool: sqlx::PgPool) {
    // Containment is asymmetric with the row's value on the LEFT, so the SCALAR probe is the one
    // that spans a key stored both ways — which is what makes `derived_from` answerable at all.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "unstable").await);

    let as_array = mk(
        &pool,
        home,
        owner,
        emitter,
        "AsArray",
        "concept",
        &[prop("derived_from", serde_json::json!(["spec-a"]))],
    )
    .await;
    let as_string = mk(
        &pool,
        home,
        owner,
        emitter,
        "AsString",
        "concept",
        &[prop("derived_from", serde_json::json!("spec-a"))],
    )
    .await;

    let scalar = select(
        &pool,
        owner,
        Narrow {
            properties: Some(serde_json::json!([contains(
                "derived_from",
                serde_json::json!(["spec-a"])
            )])),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        scalar,
        sorted(vec![as_array, as_string]),
        "a scalar probe must reach both shapes of a type-unstable key"
    );

    // The other half of the asymmetry, asserted rather than assumed: the ARRAY probe answers for
    // only the array-shaped half. This is a property of the operator, not a defect — a caller who
    // lists only the array shape silently answers for half the population.
    let array_probe = select(
        &pool,
        owner,
        Narrow {
            properties: Some(serde_json::json!([contains(
                "derived_from",
                serde_json::json!([["spec-a"]])
            )])),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(array_probe, vec![as_array]);
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn has_key_witnesses_an_empty_array_that_the_element_relation_cannot_see(pool: sqlx::PgPool) {
    // **The second half of the grain ruling.** An empty array explodes to NO rows, so
    // `kb_property_elements` cannot distinguish `derived_from: []` from no `derived_from` row at
    // all — eleven such rows exist on prod. Reading `kb_resource_properties` keeps the row, so both
    // operators of the closed set read ONE relation.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "haskey").await);

    let empty = mk(
        &pool,
        home,
        owner,
        emitter,
        "EmptyArray",
        "concept",
        &[prop("derived_from", serde_json::json!([]))],
    )
    .await;
    let absent = mk(&pool, home, owner, emitter, "NoSuchKey", "concept", &[]).await;

    let got = select(
        &pool,
        owner,
        Narrow {
            properties: Some(serde_json::json!([has_key("derived_from")])),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        got,
        vec![empty],
        "has_key must see a []-valued key; seeing nothing is the element grain"
    );
    assert!(!got.contains(&absent));
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn predicates_and_across_the_list_and_or_within_one_predicates_values(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "andor").await);

    let both = mk(
        &pool,
        home,
        owner,
        emitter,
        "Both",
        "concept",
        &[
            prop("derived_from", serde_json::json!(["spec-a"])),
            prop("preceded_by", serde_json::json!(["pr-1"])),
        ],
    )
    .await;
    let only_one = mk(
        &pool,
        home,
        owner,
        emitter,
        "OnlyOne",
        "concept",
        &[prop("derived_from", serde_json::json!(["spec-a"]))],
    )
    .await;
    let other_value = mk(
        &pool,
        home,
        owner,
        emitter,
        "OtherValue",
        "concept",
        &[prop("derived_from", serde_json::json!(["spec-z"]))],
    )
    .await;

    // AND across the list: only the resource carrying BOTH keys.
    let anded = select(
        &pool,
        owner,
        Narrow {
            properties: Some(serde_json::json!([
                contains("derived_from", serde_json::json!(["spec-a"])),
                contains("preceded_by", serde_json::json!(["pr-1"])),
            ])),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(anded, vec![both]);

    // OR within one predicate's values: either value admits.
    let ored = select(
        &pool,
        owner,
        Narrow {
            properties: Some(serde_json::json!([contains(
                "derived_from",
                serde_json::json!(["spec-a", "spec-z"])
            )])),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(ored, sorted(vec![both, only_one, other_value]));
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_malformed_or_unknown_open_key_argument_narrows_to_nothing_rather_than_everything(
    pool: sqlx::PgPool,
) {
    // Fail closed, in all three directions the fragment guards separately: a non-array argument, a
    // predicate whose `values` is not an array, and an operator the closed set does not have. Each
    // must narrow to ZERO. The dangerous failure is the other one — a guard that lets a malformed
    // argument through as "no narrowing" returns the whole corpus while the response's own
    // disclosure says it was filtered.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "failclosed").await);

    let r = mk(
        &pool,
        home,
        owner,
        emitter,
        "Present",
        "concept",
        &[prop("derived_from", serde_json::json!(["spec-a"]))],
    )
    .await;

    for (label, arg) in [
        (
            "not an array at all",
            serde_json::json!({"key": "derived_from"}),
        ),
        ("a bare string", serde_json::json!("derived_from")),
        (
            "values is not an array",
            serde_json::json!([{"key": "derived_from", "op": {"op": "contains", "values": "spec-a"}}]),
        ),
        (
            "an operator the closed set lacks",
            serde_json::json!([{"key": "derived_from", "op": {"op": "starts_with", "values": ["spec"]}}]),
        ),
        (
            "no key at all",
            serde_json::json!([{"op": {"op": "has_key"}}]),
        ),
    ] {
        let got = select(
            &pool,
            owner,
            Narrow {
                properties: Some(arg),
                ..Default::default()
            },
        )
        .await;
        assert!(
            got.is_empty(),
            "{label}: a malformed open-key argument must narrow to nothing, got {got:?}"
        );
    }

    // Positive control, so the empties above are the guards and not an empty corpus — and the
    // NULL polarity, which is the opposite one: an absent argument narrows NOTHING.
    let unnarrowed = select(&pool, owner, Narrow::default()).await;
    assert!(unnarrowed.contains(&r), "the fixture must be real");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_open_key_predicate_does_not_reach_another_owner_kinds_property(pool: sqlx::PgPool) {
    // What the VIEW is for. `kb_properties` is polymorphic and property keys are NOT unique across
    // owner kinds, so a predicate that forgets `owner_table` matches a content block's property of
    // the same name. `kb_resource_doc_type`'s comment records a hand-written copy dropping exactly
    // this filter once, which `20260806000020` had to restore.
    //
    // No key is shared across owner tables in production today (`kb_content_blocks` carries only
    // `block_role`, 37 rows `[measured — 2026-08-15]`), so this guards a future collision rather
    // than a present one — which is why it must live where it cannot be forgotten.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "polymorphic").await);

    let r = mk(&pool, home, owner, emitter, "Resource", "concept", &[]).await;

    // A block-owned property with the SAME owner_id and the SAME key. `kb_properties.owner_id`
    // carries no foreign key precisely because it is polymorphic, so this row is well-formed
    // storage — it is the view's filter, not the schema, that keeps it out of the answer.
    let ev: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY id DESC LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO kb_properties (id, owner_table, owner_id, property_key, property_value,
                                    weight, asserted_by_event_id, last_event_id)
         VALUES (uuid_generate_v7(), 'kb_content_blocks', $1, 'derived_from', $2, 1.0, $3, $3)",
    )
    .bind(r)
    .bind(serde_json::json!(["spec-a"]))
    .bind(ev)
    .execute(&pool)
    .await
    .unwrap();

    let got = select(
        &pool,
        owner,
        Narrow {
            properties: Some(serde_json::json!([contains(
                "derived_from",
                serde_json::json!(["spec-a"])
            )])),
            ..Default::default()
        },
    )
    .await;
    assert!(
        got.is_empty(),
        "a block-owned property reached a resource predicate; the owner_table filter is gone"
    );

    // The same row asserted on the RESOURCE does match — so the empty above is the owner filter
    // and not a broken fixture.
    sqlx::query("UPDATE kb_properties SET owner_table = 'kb_resources' WHERE owner_id = $1")
        .bind(r)
        .execute(&pool)
        .await
        .unwrap();
    let now_matches = select(
        &pool,
        owner,
        Narrow {
            properties: Some(serde_json::json!([contains(
                "derived_from",
                serde_json::json!(["spec-a"])
            )])),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(now_matches, vec![r]);
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_folded_property_is_not_narrowable(pool: sqlx::PgPool) {
    // The view's second filter. A folded row is history, and history must not answer a live
    // question — the same rule `kb_resources_live` makes structural for resources.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "folded").await);

    let r = mk(
        &pool,
        home,
        owner,
        emitter,
        "Folded",
        "concept",
        &[prop("derived_from", serde_json::json!(["spec-a"]))],
    )
    .await;

    let probe = || {
        select(
            &pool,
            owner,
            Narrow {
                properties: Some(serde_json::json!([contains(
                    "derived_from",
                    serde_json::json!(["spec-a"])
                )])),
                ..Default::default()
            },
        )
    };
    assert_eq!(probe().await, vec![r], "the fixture must be real first");

    sqlx::query("UPDATE kb_properties SET is_folded = true WHERE owner_id = $1 AND property_key = 'derived_from'")
        .bind(r)
        .execute(&pool)
        .await
        .unwrap();
    assert!(probe().await.is_empty(), "a folded property still answered");
}
