#![cfg(feature = "artifact-tests")]
//! The owner-agnostic element relation, and the write-time `tags` normalization beside it.
//!
//! Task `01a00502-a774-7001-b5b2-0ce462158f1c`. Design
//! `docs/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md`
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
//! refuses new bad shapes; the projector forgives old ones."* That rule is about **refusal**, and
//! this is not one. A projector that normalizes forgives every historical shape and converges it —
//! which is the behaviour that rule exists to protect, arrived at from the other side.

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
