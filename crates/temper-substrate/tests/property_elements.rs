#![cfg(feature = "artifact-tests")]
//! The owner-agnostic element relation, and the write-time `tags` normalization beside it.
//!
//! Task `01a00502-a774-7001-b5b2-0ce462158f1c`. Design
//! `internal/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md`
//! §6.2 (*"a shape convention lives in a view, and the view is owner-agnostic"*) and §7, ruled on
//! 2026-08-15.
//!
//! ## The two objects here do DIFFERENT jobs, and neither substitutes for the other
//!
//! `kb_property_elements` decides what a **read** does with a stored value of any shape: explode an
//! array, pass a scalar. It is universal — one rule for all 70 live keys — which is what §7 said the
//! `tags` split was blocking.
//!
//! The projector normalization decides what is **stored**. It is not made redundant by the view: a
//! value's stored shape is what `readback::meta` surfaces in `open_meta`, what the FTS projection
//! reads, and what any future reader that has never heard of this view will see. A convention
//! honoured only at read time is a convention every new reader has to re-implement.
//!
//! ## Where the normalization lives, and why the neighbouring rule does not forbid it
//!
//! `[decided — 2026-08-15, Pete]` In `_project_property_asserted` and `_project_property_set` —
//! the two projectors — rather than at the door or in Rust. Nothing can bypass a projector: not a
//! direct SQL caller, not the scenario loader, not replay.
//!
//! `20260730000010:228-231` states the rule a reader will reach for here: *"The guard lives HERE and
//! not in the projector, and the split matters: replay calls the projector directly, so a projector
//! that refused a shape would make the history that already contains it unreplayable. The door
//! refuses new bad shapes; the projector forgives old ones."*
//!
//! `[corrected — 2026-08-15, found in review]` This file previously answered that rule with *"a
//! projector that normalizes forgives every historical shape and converges it — it refuses none."*
//! **That is false, in exactly one case**, and
//! `normalizing_can_collide_with_an_existing_live_row_and_that_raises` below is the witness for it:
//! the assert arm APPENDS, `uq_kb_properties_active` is unique on (owner, key, value) for live
//! rows, so normalizing a bare string onto an array that is already there is a duplicate and
//! raises. The rule holds for every shape except a duplicate this normalization itself creates.
//!
//! Recorded rather than papered over. The prod measurement that bounds it is about the EVENT LOG,
//! not the live rows — 467 `tags` events, every one an array — because it is replay, not the
//! current projection, that the refusal would break.

mod common;

use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
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

/// A resource carrying an arbitrary property set, written through the real create path so every
/// property rides `property_asserted` → `_project_property_asserted`.
async fn mk(
    pool: &sqlx::PgPool,
    home: AnchorRef,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
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
            doc_type: "note",
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

fn prop(k: &str, v: serde_json::Value) -> (String, serde_json::Value) {
    (k.to_string(), v)
}

/// The live stored value of one key on one resource.
async fn stored(pool: &sqlx::PgPool, owner: Uuid, key: &str) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT property_value FROM kb_properties \
          WHERE owner_table='kb_resources' AND owner_id=$1 AND property_key=$2 AND NOT is_folded",
    )
    .bind(owner)
    .bind(key)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Every element the view exposes for one key on one resource, as text, sorted.
async fn elements(pool: &sqlx::PgPool, owner: Uuid, key: &str) -> Vec<String> {
    let mut v: Vec<String> = sqlx::query_scalar(
        "SELECT element #>> '{}' FROM kb_property_elements \
          WHERE owner_table='kb_resources' AND owner_id=$1 AND property_key=$2",
    )
    .bind(owner)
    .bind(key)
    .fetch_all(pool)
    .await
    .unwrap();
    v.sort();
    v
}

// ── The view ────────────────────────────────────────────────────────────────────────────────────

/// One rule for every key: an array becomes its elements, anything else becomes itself.
///
/// **There is deliberately no `tags` branch**, and that absence is the assertion. §7's split existed
/// for `tags` alone, and every key exercised here — an array-shaped one, a string-shaped one, a
/// date, and an object-shaped facet mark — is served by the same expression.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_element_view_explodes_arrays_and_passes_scalars_for_every_key(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "elements").await);

    let r = mk(
        &pool,
        home,
        owner,
        emitter,
        "every shape",
        &[
            prop("tags", serde_json::json!(["ci", "auth"])),
            prop("descriptor", serde_json::json!("one whole string")),
            prop("date", serde_json::json!("2026-08-15")),
            prop("relates_to", serde_json::json!([])),
        ],
    )
    .await;

    assert_eq!(
        elements(&pool, r, "tags").await,
        vec!["auth".to_string(), "ci".to_string()],
        "an array explodes to one row per element"
    );
    assert_eq!(
        elements(&pool, r, "descriptor").await,
        vec!["one whole string".to_string()],
        "a scalar passes through WHOLE — it is one element, never split on anything"
    );
    assert_eq!(
        elements(&pool, r, "date").await,
        vec!["2026-08-15".to_string()],
        "the same rule serves a shape-convention key with no FTS role"
    );
    assert!(
        elements(&pool, r, "relates_to").await.is_empty(),
        "an empty array has no elements, so it contributes no rows — a key whose presence the \
         element relation cannot witness. Named here because it is the case a `has_key` predicate \
         built over this view would get wrong"
    );
}

/// A folded row is not live, and the view carries `NOT is_folded` so no predicate has to remember to.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_element_view_omits_folded_rows(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "folded").await);

    let r = mk(
        &pool,
        home,
        owner,
        emitter,
        "supersede me",
        &[prop("tags", serde_json::json!(["before"]))],
    )
    .await;
    assert_eq!(elements(&pool, r, "tags").await, vec!["before".to_string()]);

    // `property_set` means "this key holds one current value": it folds the whole live set first.
    writes::set_property(
        &pool,
        ResourceId::from(r),
        "tags",
        &serde_json::json!(["after"]),
        emitter,
    )
    .await
    .unwrap();

    assert_eq!(
        elements(&pool, r, "tags").await,
        vec!["after".to_string()],
        "the superseded element must not survive in the view — the folded row is not live"
    );
}

// ── The write-time normalization ────────────────────────────────────────────────────────────────

/// A bare-string `tags` value is stored as a ONE-element array, and the element is the whole string.
///
/// `[decided — 2026-08-15, Pete]` §7. `tags: "concept design"` is one tag named `concept design`.
/// The split it replaces inferred a list from whitespace, and the FTS agreement that justified the
/// split does not exist: FTS delegates to a tokenizer that splits differently.
///
/// Asserted at BOTH projectors. `property_asserted` is the create path (414 production rows arrived
/// that way) and `property_set` is the `resource update --open-meta` path (29 rows); a normalization
/// present in one of them is not a normalization — the same *"an invariant honoured by two of three
/// write paths is no more an invariant than one of two"* rule `context_service::canonical_name`
/// records.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_bare_string_tags_value_is_stored_as_one_element_by_both_projectors(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "normalize").await);

    // ── property_asserted, via create ──
    let asserted = mk(
        &pool,
        home,
        owner,
        emitter,
        "bare at create",
        &[prop("tags", serde_json::json!("concept design"))],
    )
    .await;
    assert_eq!(
        stored(&pool, asserted, "tags").await,
        serde_json::json!(["concept design"]),
        "_project_property_asserted must store a bare string as one element, whole"
    );
    assert_eq!(
        elements(&pool, asserted, "tags").await,
        vec!["concept design".to_string()],
        "and the view must therefore see exactly one tag"
    );

    // ── property_set, via update ──
    let set = mk(&pool, home, owner, emitter, "bare at update", &[]).await;
    writes::set_property(
        &pool,
        ResourceId::from(set),
        "tags",
        &serde_json::json!("ci auth"),
        emitter,
    )
    .await
    .unwrap();
    assert_eq!(
        stored(&pool, set, "tags").await,
        serde_json::json!(["ci auth"]),
        "_project_property_set must normalize identically — the open_meta round-trip writes here"
    );
}

/// Normalization is scoped to `tags` and touches no other key.
///
/// The control for the test above. Without it, a projector that wrapped EVERY scalar in an array
/// would satisfy both assertions there while silently changing `date`, `descriptor` and the 61
/// unrecognized keys — a far larger behaviour change than the one that was ruled.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn normalization_is_scoped_to_tags_and_leaves_every_other_key_verbatim(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "scoped").await);

    let r = mk(
        &pool,
        home,
        owner,
        emitter,
        "untouched",
        &[
            prop("descriptor", serde_json::json!("a bare string")),
            prop("date", serde_json::json!("2026-08-15")),
            prop("keywords", serde_json::json!("also bare")),
        ],
    )
    .await;

    for key in ["descriptor", "date", "keywords"] {
        let value = stored(&pool, r, key).await;
        assert!(
            value.is_string(),
            "`{key}` is not `tags` and must be stored exactly as written; got {value}"
        );
    }

    // And an ARRAY-shaped `tags` is left alone rather than double-wrapped.
    let arrayed = mk(
        &pool,
        home,
        owner,
        emitter,
        "already an array",
        &[prop("tags", serde_json::json!(["ci", "auth"]))],
    )
    .await;
    assert_eq!(
        stored(&pool, arrayed, "tags").await,
        serde_json::json!(["ci", "auth"]),
        "normalization applies to the string shape only — an array must not be nested inside another"
    );
}

/// Normalizing a bare string onto an array that is ALREADY live is a duplicate, and it raises.
///
/// **The one case where the projector refuses rather than forgives** `[found in review —
/// 2026-08-15]`. Three facts meet here, and none is new on its own:
///
/// - `_project_property_asserted`'s non-facet arm **appends** — only `property_set` folds — so one
///   owner may hold several live rows for one key;
/// - `uq_kb_properties_active` is unique on `(owner_table, owner_id, property_key, property_value)`
///   for live rows;
/// - normalization makes `"ci"` and `["ci"]` the **same value**.
///
/// So a pair that used to store as two distinct live rows now collides: the second raises and takes
/// its transaction with it. Replay calls this projector directly, so a history containing that pair
/// would be unreplayable.
///
/// ## How reachable it actually is — narrower than it first looks, and measured both ways
///
/// **No product path asserts `tags` at all** `[verified — 2026-08-15]`. `create_resource` fires
/// `SeedAction::PropertySet` per property (`writes.rs:297`), and `property_set` folds the key's
/// whole live set first, so it can never collide with itself — the first draft of this test
/// asserted through `create_resource` and did NOT raise, which is what exposed the premise. The only
/// `PropertyAssert` emitters are `FacetSet` (key `facet`) and the scenario loader (key `topic`).
///
/// So this is a property of the **projector**, reachable only by a direct assert — which is exactly
/// what replay is. Hence this test fires one, rather than going through a write path that would
/// quietly fold and prove nothing.
///
/// `[measured on prod — 2026-08-15]` the whole event log holds 467 `tags` events and **every one is
/// an array**. The measurement is over the EVENT LOG rather than the live rows on purpose, because
/// replay is what the refusal would break. Two array-shaped asserts of the same array would already
/// collide today, before this migration; normalization can only create a NEW collision where a
/// string-shaped `tags` event meets an array-shaped one, and there are no string-shaped ones.
///
/// Not papered over with `ON CONFLICT DO NOTHING`: that arm returns `ARRAY[v_prop]`, so swallowing
/// the conflict would name a row it did not write. And the state now refused was already
/// incoherent — `_rebuild_resource_search_vector` reads `tags` with `LIMIT 1` and would index one
/// of the two arbitrarily.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn normalizing_can_collide_with_an_existing_live_row_and_that_raises(pool: sqlx::PgPool) {
    use temper_substrate::events::{fire, SeedAction};

    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = AnchorRef::context(ctx(&pool, owner, "collide").await);

    let r = mk(&pool, home, owner, emitter, "collide", &[]).await;

    let assert_tags = |value: serde_json::Value| {
        let pool = pool.clone();
        async move {
            let mut conn = pool.acquire().await.unwrap();
            fire(
                &mut conn,
                SeedAction::PropertyAssert {
                    resource: ResourceId::from(r),
                    key: "tags",
                    value: &value,
                    weight: 1.0,
                    emitter,
                },
            )
            .await
        }
    };

    assert_tags(serde_json::json!(["ci"]))
        .await
        .expect("the first assert lands");

    // The control, and it is what makes the assertion below about DUPLICATION rather than about
    // asserting the key twice: a second assert that stays distinct after normalization succeeds,
    // and appends rather than replacing.
    assert_tags(serde_json::json!("auth"))
        .await
        .expect("a second assert is legal — the assert arm appends");
    let mut live: Vec<String> = elements(&pool, r, "tags").await;
    live.sort();
    assert_eq!(
        live,
        vec!["auth".to_string(), "ci".to_string()],
        "two live `tags` rows coexist, which is the state the collision below is a duplicate WITHIN"
    );

    // And the collision: `"ci"` normalizes onto the `["ci"]` already live.
    let err = assert_tags(serde_json::json!("ci"))
        .await
        .expect_err("normalization makes this the same value as the live row — it must raise");
    let rendered = format!("{err:#}");
    assert!(
        rendered.contains("uq_kb_properties_active"),
        "the refusal must be the unique index, not some other failure that would make this test \
         pass for the wrong reason. Got: {rendered}"
    );
}
