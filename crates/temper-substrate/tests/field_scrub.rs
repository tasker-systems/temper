#![cfg(feature = "test-db")]
//! Field-grain scrub witnesses (spec 2026-10-09 field-grain-scrub).
//!
//! `_project_property_unset` (migration 20261018100000) is the one body of the `property_unset`
//! projection: the write guard, the fold of `(kb_resources, owner, key)`'s live rows, and the
//! search-vector rebuild for `keywords` / `descriptor` / `tags`. The fire path and replay both
//! reach it through `events::project_property_unset`; a SQL act clearing a property family
//! (S4 step 3) can call it directly. These witnesses call it directly, against a world built
//! through the real write paths.
//!
//! Then the verifier's second authority (migration 20261018100010, spec witness 3), and the act
//! itself (migration 20261018100020, spec witnesses 1, 2 and 7, review focus 1-4): each act runs
//! through `resource_field_scrub_execute` against a world built through the real write paths, and
//! each witness of the act ends with replay byte-identical over `PROJECTION_DUMPS`.

mod common;

use sha2::Digest;
use sqlx::PgPool;
use temper_core::types::ids::EntityId;
use temper_core::types::property_owner::PropertyOwner;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, ProfileId, ResourceId};
use temper_substrate::payloads::{
    self, AnchorRef, EdgePolarity, ErasureAct, RecordedRefusalReason, ResourceErasureRefused,
    ResourceScrubbed, ScrubFieldKind,
};
use temper_substrate::replay;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

/// The indexed term the `tags` witnesses look for in the search vector.
const TAG_TERM: &str = "alpharhythm";

fn chunk_hash(prose: &str) -> String {
    format!("{:x}", sha2::Sha256::digest(prose.trim()))
}

fn chunk(prose: &str) -> IncomingChunk {
    IncomingChunk {
        chunk_index: 0,
        content_hash: chunk_hash(prose),
        content: prose.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: "first".to_string(),
        heading_depth: 1,
    }
}

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: EntityId =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), entity)
}

async fn make_home(pool: &PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

async fn setup(pool: &PgPool) -> (ProfileId, EntityId, ContextId) {
    common::reset_schema(pool).await;
    temper_substrate::scenario::bootseed::seed_system(pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let home = make_home(pool, owner, "field-scrub").await;
    (owner, emitter, home)
}

/// A complete resource with one block and `properties`, through the create path.
async fn create(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    slug: &str,
    properties: &[(String, serde_json::Value)],
) -> ResourceId {
    let origin = format!("test://{slug}");
    let body = "a plain body";
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: slug,
            origin_uri: &origin,
            body,
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties,
            chunks: Some(vec![chunk(body)]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create through the create path")
}

/// The `property_unset` payload the fire path writes for `(resource, key)`.
fn unset_payload(resource: ResourceId, key: &str) -> serde_json::Value {
    serde_json::to_value(payloads::PropertyUnset {
        owner: AnchorRef::resource(resource),
        property_key: key.to_owned(),
    })
    .unwrap()
}

/// Append a `property_unset` event for `(resource, key)` as the fire path does, with no
/// projection, and return its id and payload: the witnesses then project it themselves.
async fn append_unset(
    pool: &PgPool,
    emitter: EntityId,
    resource: ResourceId,
    key: &str,
) -> (Uuid, serde_json::Value) {
    let payload = unset_payload(resource, key);
    let event: Uuid = sqlx::query_scalar(
        "SELECT _event_append('property_unset', $1, a.anchor_table, a.anchor_id, $2) \
           FROM _property_owner_anchor('kb_resources', $3) a",
    )
    .bind(emitter)
    .bind(&payload)
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .expect("the property_unset event appends");
    (event, payload)
}

/// The projector, called directly.
async fn project_unset(
    pool: &PgPool,
    event: Uuid,
    payload: &serde_json::Value,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT _project_property_unset($1, $2)")
        .bind(event)
        .bind(payload)
        .fetch_one(pool)
        .await
}

async fn live_rows(pool: &PgPool, owner_table: &str, owner_id: Uuid, key: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = $1 AND owner_id = $2 AND property_key = $3 AND NOT is_folded",
    )
    .bind(owner_table)
    .bind(owner_id)
    .bind(key)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn search_vector(pool: &PgPool, resource: ResourceId) -> Option<String> {
    sqlx::query_scalar(
        "SELECT search_vector::text FROM kb_resource_search_index WHERE resource_id = $1",
    )
    .bind(resource.uuid())
    .fetch_optional(pool)
    .await
    .unwrap()
}

async fn vector_matches(pool: &PgPool, resource: ResourceId, term: &str) -> bool {
    sqlx::query_scalar(
        "SELECT COALESCE((SELECT search_vector @@ plainto_tsquery('english', $2) \
           FROM kb_resource_search_index WHERE resource_id = $1), false)",
    )
    .bind(resource.uuid())
    .bind(term)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The erasure act, as the system operator under a fresh request reference; returns the
/// `resource_erased` event id.
async fn erase(pool: &PgPool, resource: ResourceId) -> Uuid {
    let (_, operator_entity) = system_actor(pool).await;
    let raw: String =
        sqlx::query_scalar("SELECT (resource_erasure_execute($1,$2,$3,$4)->>'event_id')::text")
            .bind(resource.uuid())
            .bind(operator_entity)
            .bind(operator_entity)
            .bind(Uuid::now_v7())
            .fetch_one(pool)
            .await
            .expect("the erasure completes");
    Uuid::parse_str(&raw).expect("the event id parses")
}

/// Called directly, `_project_property_unset` folds every live row of `(kb_resources, R, key)`
/// and returns their count. Another resource's row of the same key, an edge-owned row of the
/// same key text, and R's other keys stay live.
///
/// FAILS IF: the function does not exist; the count is not the number of R's live rows of the
/// key; a folded row does not carry the unset event as `last_event_id`; or the fold reaches past
/// `(kb_resources, R, key)`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_unset_projector_folds_only_the_owners_live_rows_of_the_key(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let tags = vec![("tags".to_string(), serde_json::json!(["alpha"]))];
    let mut r_props = tags.clone();
    r_props.push(("descriptor".to_string(), serde_json::json!("kept")));
    let r = create(&pool, owner, emitter, home, "scrub-r", &r_props).await;
    let twin = create(&pool, owner, emitter, home, "scrub-twin", &tags).await;

    // A second live `tags` row on R: an assert inserts without folding.
    let beta = serde_json::json!(["beta"]);
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::PropertyAssert {
            resource: r,
            key: "tags",
            value: &beta,
            weight: 1.0,
            emitter,
        },
    )
    .await
    .expect("a second tags row asserts");
    tx.commit().await.unwrap();

    // An edge-owned row with the same key text.
    let edge = writes::assert_anchored_edge_with(
        &pool,
        writes::AssertAnchoredEdgeParams {
            source: AnchorRef::resource(r),
            target: AnchorRef::resource(twin),
            kind: temper_substrate::affinity::EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("derived_from"),
            weight: 1.0,
            home: EdgeHome::Context(home),
            emitter,
        },
        EventContext::default(),
    )
    .await
    .unwrap();
    writes::assert_keyed_property_with(
        &pool,
        PropertyOwner::Edge { id: edge },
        "tags",
        &serde_json::json!(["alpha"]),
        1.0,
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();

    assert_eq!(live_rows(&pool, "kb_resources", r.uuid(), "tags").await, 2);
    let (event, payload) = append_unset(&pool, emitter, r, "tags").await;

    let folded = project_unset(&pool, event, &payload)
        .await
        .expect("the projector runs");

    assert_eq!(folded, 2, "the count is R's live rows of the key");
    assert_eq!(live_rows(&pool, "kb_resources", r.uuid(), "tags").await, 0);
    let stamped: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1 AND property_key = 'tags' \
            AND is_folded AND last_event_id = $2",
    )
    .bind(r.uuid())
    .bind(event)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stamped, 2, "each folded row carries the unset event");
    assert_eq!(
        live_rows(&pool, "kb_resources", r.uuid(), "descriptor").await,
        1,
        "R's other key is untouched"
    );
    assert_eq!(
        live_rows(&pool, "kb_resources", twin.uuid(), "tags").await,
        1,
        "another resource's row of the same key is untouched"
    );
    assert_eq!(
        live_rows(&pool, "kb_edges", edge.uuid(), "tags").await,
        1,
        "an edge-owned row of the same key text is untouched"
    );
}

/// The write guard runs inside the projector: an erased owner refuses the unset.
///
/// FAILS IF: the function does not exist, or it projects onto an erased resource without
/// raising the guard's refusal.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_unset_projector_refuses_an_erased_owner(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(
        &pool,
        owner,
        emitter,
        home,
        "scrub-erased",
        &[("tags".to_string(), serde_json::json!(["alpha"]))],
    )
    .await;
    let erased_event = erase(&pool, r).await;

    // The guard raises before the fold writes anything, so the erasure's own event stands in
    // for the unset event's id.
    let err = project_unset(&pool, erased_event, &unset_payload(r, "tags"))
        .await
        .expect_err("an erased owner refuses the unset");
    let msg = err
        .as_database_error()
        .map(|db| db.message().to_owned())
        .unwrap_or_else(|| err.to_string());
    assert!(
        msg.contains(&format!(
            "resource {} is erased; writes are refused",
            r.uuid()
        )),
        "got {msg}"
    );
}

/// Folding the last `tags` row rebuilds the resource's search vector, so the tag's term no
/// longer matches.
///
/// FAILS IF: the function does not exist, or it folds the row and leaves the stale vector.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_unset_projector_rebuilds_the_search_vector_when_the_last_tags_row_folds(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(
        &pool,
        owner,
        emitter,
        home,
        "scrub-fts",
        &[("tags".to_string(), serde_json::json!([TAG_TERM]))],
    )
    .await;
    assert!(
        vector_matches(&pool, r, TAG_TERM).await,
        "the tag term is indexed through the create path"
    );
    let before = search_vector(&pool, r).await;
    let (event, payload) = append_unset(&pool, emitter, r, "tags").await;

    let folded = project_unset(&pool, event, &payload)
        .await
        .expect("the projector runs");

    assert_eq!(folded, 1);
    assert_ne!(
        search_vector(&pool, r).await,
        before,
        "the vector is rebuilt"
    );
    assert!(
        !vector_matches(&pool, r, TAG_TERM).await,
        "the folded tag's term no longer matches"
    );
}

// ── Spec witness 3: the verifier admits a scrub's own rewrite and nothing else ────────────────
//
// Migration 20261018100010 makes `resource_scrubbed` a second authorising event for the ledger
// exception (spec S6). Each case below forges what the act will do (S4 steps 4-5) in one
// transaction: append a `resource_scrubbed` record, project its redaction rows through
// `_project_resource_scrubbed_redactions`, then UPDATE `kb_events`. Every refusal is asserted by
// the trigger's own message, and each refused case is paired with an admitted one that differs only
// in the condition under test, so a refusal for any other reason would fail the pair.

/// Fire one write through the live write path and commit it.
async fn write(pool: &PgPool, action: SeedAction<'_>) {
    let mut tx = pool.begin().await.unwrap();
    fire(&mut tx, action).await.expect("the write fires");
    tx.commit().await.unwrap();
}

async fn set(pool: &PgPool, emitter: EntityId, resource: ResourceId, key: &str, value: &str) {
    let value = serde_json::json!(value);
    write(
        pool,
        SeedAction::PropertySet {
            resource,
            key,
            value: &value,
            weight: 1.0,
            emitter,
        },
    )
    .await;
}

async fn unset(pool: &PgPool, emitter: EntityId, resource: ResourceId, key: &str) {
    write(
        pool,
        SeedAction::PropertyUnset {
            resource,
            key,
            emitter,
        },
    )
    .await;
}

async fn retitle(pool: &PgPool, emitter: EntityId, resource: ResourceId, title: &str) {
    write(
        pool,
        SeedAction::ResourceUpdate {
            resource,
            title: Some(title),
            origin_uri: None,
            emitter,
        },
    )
    .await;
}

/// R's `event_type` events naming `key` as resource-owned property events, in walk order.
async fn property_events(
    pool: &PgPool,
    event_type: &str,
    resource: ResourceId,
    key: &str,
) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = $1 AND e.payload#>>'{owner,table}' = 'kb_resources' \
            AND e.payload#>>'{owner,id}' = $2 AND e.payload->>'property_key' = $3 \
          ORDER BY e.id",
    )
    .bind(event_type)
    .bind(resource.uuid().to_string())
    .bind(key)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// R's `event_type` events keyed by `resource_id`, in walk order.
async fn resource_events(pool: &PgPool, event_type: &str, resource: ResourceId) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = $1 AND e.payload->>'resource_id' = $2 ORDER BY e.id",
    )
    .bind(event_type)
    .bind(resource.uuid().to_string())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// A `redacted_fields` list: each event with the one path named.
fn redacted(entries: &[(Uuid, &str)]) -> serde_json::Value {
    serde_json::Value::Array(
        entries
            .iter()
            .map(|(event, path)| serde_json::json!({"event": event, "paths": [path]}))
            .collect(),
    )
}

/// The authorising act a forged record stands for.
#[derive(Debug, Clone, Copy)]
enum Authority {
    /// `resource_scrubbed`, projected by `_project_resource_scrubbed_redactions`.
    Scrub,
    /// `resource_erased`, projected by `_project_resource_erased_redactions`.
    Erasure,
}

/// One forged act: the record it appends, and what its transaction does around it.
#[derive(Debug, Clone)]
struct Forged<'a> {
    authority: Authority,
    subject: ResourceId,
    /// The scrub's `field`; `None` for an erasure record, which has none.
    field: Option<serde_json::Value>,
    redacted_fields: serde_json::Value,
    /// Runs before the record is appended.
    prelude: Option<&'a str>,
    /// Runs after its rows are projected.
    rewrite: &'a str,
}

/// Run `act` on one connection: the prelude, then the record appended and its rows projected,
/// then the rewrite. The first error stops it.
async fn forge(
    conn: &mut sqlx::PgConnection,
    emitter: EntityId,
    act: &Forged<'_>,
) -> Result<(), sqlx::Error> {
    if let Some(prelude) = act.prelude {
        sqlx::raw_sql(prelude).execute(&mut *conn).await?;
    }
    let mut payload = serde_json::json!({
        "subject_table": "kb_resources",
        "subject_id": act.subject.uuid(),
        "redacted_fields": act.redacted_fields,
    });
    if let Some(field) = &act.field {
        payload["field"] = field.clone();
    }
    let (event_type, projector) = match act.authority {
        Authority::Scrub => (
            "resource_scrubbed",
            "SELECT _project_resource_scrubbed_redactions($1, $2)",
        ),
        Authority::Erasure => (
            "resource_erased",
            "SELECT _project_resource_erased_redactions($1, $2)",
        ),
    };
    let event: Uuid = sqlx::query_scalar("SELECT _event_append($1, $2, NULL, NULL, $3)")
        .bind(event_type)
        .bind(emitter)
        .bind(&payload)
        .fetch_one(&mut *conn)
        .await?;
    sqlx::query(projector)
        .bind(event)
        .bind(&payload)
        .execute(&mut *conn)
        .await?;
    sqlx::raw_sql(act.rewrite).execute(&mut *conn).await?;
    Ok(())
}

/// [`forge`] in its own transaction: committed when `commit` and it succeeded, otherwise rolled
/// back. Returns the error text of a refusal, `None` when every step landed.
async fn attempt(
    pool: &PgPool,
    emitter: EntityId,
    act: &Forged<'_>,
    commit: bool,
) -> Option<String> {
    let mut tx = pool.begin().await.unwrap();
    let out = forge(&mut tx, emitter, act).await;
    if out.is_ok() && commit {
        tx.commit().await.unwrap();
    } else {
        tx.rollback().await.unwrap();
    }
    out.err().map(|e| e.to_string())
}

/// A scrub of `field`, rolled back.
async fn scrub(
    pool: &PgPool,
    emitter: EntityId,
    subject: ResourceId,
    field: serde_json::Value,
    redacted_fields: serde_json::Value,
    rewrite: &str,
) -> Option<String> {
    let act = Forged {
        authority: Authority::Scrub,
        subject,
        field: Some(field),
        redacted_fields,
        prelude: None,
        rewrite,
    };
    attempt(pool, emitter, &act, false).await
}

fn append_only(what: &str, err: Option<String>) {
    let err = err.unwrap_or_else(|| panic!("{what} was admitted"));
    assert!(
        err.contains("event ledger is append-only"),
        "{what}: refused with the trigger's own message; got {err}"
    );
}

fn admitted(what: &str, err: Option<String>) {
    assert!(err.is_none(), "{what} was refused: {err:?}");
}

fn property_family(handle: Uuid) -> serde_json::Value {
    serde_json::json!({"kind": "property", "family": handle})
}

/// Rewrite each event's `title` to the title class's sentinel.
fn title_rewrite(events: &[Uuid]) -> String {
    format!(
        "UPDATE kb_events SET payload = jsonb_set(payload, '{{title}}', \
                 to_jsonb('erased-' || (payload->>'resource_id'))) \
          WHERE id IN ({})",
        in_list(events)
    )
}

/// Rewrite each event's `value` to the property-value class's sentinel.
fn value_rewrite(events: &[Uuid]) -> String {
    format!(
        "UPDATE kb_events SET payload = jsonb_set(payload, '{{value}}', \
                 to_jsonb('erased:' || id::text)) \
          WHERE id IN ({})",
        in_list(events)
    )
}

/// Rewrite each event's `property_key` to `key`, in one statement.
fn key_rewrite(events: &[Uuid], key: &str) -> String {
    format!(
        "UPDATE kb_events SET payload = jsonb_set(payload, '{{property_key}}', \
                 to_jsonb('{key}'::text)) \
          WHERE id IN ({})",
        in_list(events)
    )
}

fn in_list(events: &[Uuid]) -> String {
    events
        .iter()
        .map(|e| format!("'{e}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The statement that marks `resource` erased inside a forged transaction, so a case can hold an
/// erased subject without running the erasure act, which would rewrite the very paths under test.
fn mark_erased(resource: ResourceId) -> String {
    format!(
        "UPDATE kb_resources SET is_active = false, erased_at = now() WHERE id = '{}'",
        resource.uuid()
    )
}

/// A live resource with a `colour` family that was set, unset, then set twice more (A, U, B, C),
/// and a `size` family set twice (S1, S2). B's row is folded by C; A's by U; S1's by S2.
struct Families {
    r: ResourceId,
    a: Uuid,
    u: Uuid,
    b: Uuid,
    s1: Uuid,
}

async fn families(pool: &PgPool, owner: ProfileId, emitter: EntityId, home: ContextId) -> Families {
    let r = create(pool, owner, emitter, home, "scrub-families", &[]).await;
    set(pool, emitter, r, "colour", "blue").await;
    unset(pool, emitter, r, "colour").await;
    set(pool, emitter, r, "colour", "red").await;
    set(pool, emitter, r, "colour", "green").await;
    set(pool, emitter, r, "size", "one").await;
    set(pool, emitter, r, "size", "two").await;
    let colour_sets = property_events(pool, "property_set", r, "colour").await;
    let colour_unsets = property_events(pool, "property_unset", r, "colour").await;
    let size_sets = property_events(pool, "property_set", r, "size").await;
    assert_eq!(colour_sets.len(), 3, "A, B and C");
    assert_eq!(colour_unsets.len(), 1, "U");
    assert_eq!(size_sets.len(), 2, "S1 and S2");
    Families {
        r,
        a: colour_sets[0],
        u: colour_unsets[0],
        b: colour_sets[1],
        s1: size_sets[0],
    }
}

/// A scrub of the title admits the rewrite of every prior title-carrying event (the create, and
/// a leaking update later reverted) and refuses the last one, which produces today's title (S3,
/// S6.3). The create's live `doc_type` row does not hold its title back: a title is prior by walk
/// order, not by its event's property rows.
///
/// FAILS IF: `resource_scrubbed` authorises nothing (no projector, no `scrub` authority), or a
/// scrub row can redact the event producing today's title.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_scrub_admits_a_prior_title_and_refuses_the_current_one(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-title", &[]).await;
    retitle(&pool, emitter, r, "a leaked title").await;
    retitle(&pool, emitter, r, "scrub-title").await;
    let created = resource_events(&pool, "resource_created", r).await;
    let updates = resource_events(&pool, "resource_updated", r).await;
    assert_eq!(created.len(), 1, "one create");
    assert_eq!(updates.len(), 2, "the leak, then the revert");
    let (leak, revert) = (updates[0], updates[1]);
    let title = serde_json::json!({"kind": "title"});

    admitted(
        "a scrub of the prior titles (the create and the leaking update)",
        scrub(
            &pool,
            emitter,
            r,
            title.clone(),
            redacted(&[(created[0], "title"), (leak, "title")]),
            &title_rewrite(&[created[0], leak]),
        )
        .await,
    );
    append_only(
        "a scrub row naming the last title-carrying event",
        scrub(
            &pool,
            emitter,
            r,
            title,
            redacted(&[(revert, "title")]),
            &title_rewrite(&[revert]),
        )
        .await,
    );
}

/// A scrub of one property family refuses an event of another family, though that event is R's own,
/// prior, and rewritten to its exact sentinel; the same rewrite under its own family's handle is
/// admitted (S6.3: "it names the scrubbed field, or the scrubbed family's owner and key").
///
/// FAILS IF: the statement verifier does not hold a scrub to its named family.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_scrub_refuses_an_event_outside_its_family(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let f = families(&pool, owner, emitter, home).await;
    let rf = redacted(&[(f.s1, "value")]);

    append_only(
        "a size event under the colour family's scrub",
        scrub(
            &pool,
            emitter,
            f.r,
            property_family(f.a),
            rf.clone(),
            &value_rewrite(&[f.s1]),
        )
        .await,
    );
    admitted(
        "the same size event under the size family's scrub",
        scrub(
            &pool,
            emitter,
            f.r,
            property_family(f.s1),
            rf,
            &value_rewrite(&[f.s1]),
        )
        .await,
    );
}

/// Key text is scrubbable at or before the family's latest unset and not after it (S3, "unset
/// first"). B is after the unset and folded by C, so its value is prior, but renaming its key is
/// refused; renaming A and U, the unset included, in one statement is admitted.
///
/// FAILS IF: the verifier admits a renamed key past the family's latest `property_unset`, or
/// refuses one at or before it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_scrub_renames_key_text_only_up_to_the_latest_unset(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let f = families(&pool, owner, emitter, home).await;
    let sentinel = format!("scrubbed-key-{}", f.a);

    append_only(
        "key text renamed past the family's latest unset",
        scrub(
            &pool,
            emitter,
            f.r,
            property_family(f.a),
            redacted(&[(f.b, "property_key")]),
            &key_rewrite(&[f.b], &sentinel),
        )
        .await,
    );
    admitted(
        "key text renamed at and before the family's latest unset",
        scrub(
            &pool,
            emitter,
            f.r,
            property_family(f.a),
            redacted(&[(f.a, "property_key"), (f.u, "property_key")]),
            &key_rewrite(&[f.a, f.u], &sentinel),
        )
        .await,
    );
}

/// A scrub row authorises nothing on an erased subject (S6.2: the scrub's precondition is that the
/// subject is not erased). The subject is marked erased inside the forged transaction, so the
/// rewrite is otherwise the admitted one of `a_scrub_renames_key_text_only_up_to_the_latest_unset`.
///
/// FAILS IF: the row verifier admits a scrub row without checking its subject's `erased_at`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_scrub_authorises_nothing_on_an_erased_subject(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let f = families(&pool, owner, emitter, home).await;

    let prelude = mark_erased(f.r);
    let rewrite = key_rewrite(&[f.a, f.u], &format!("scrubbed-key-{}", f.a));
    let act = Forged {
        authority: Authority::Scrub,
        subject: f.r,
        field: Some(property_family(f.a)),
        redacted_fields: redacted(&[(f.a, "property_key"), (f.u, "property_key")]),
        prelude: Some(&prelude),
        rewrite: &rewrite,
    };

    append_only(
        "a scrub of an erased subject",
        attempt(&pool, emitter, &act, false).await,
    );
}

/// A scrub's authorisation is spent with its transaction: its record and rows, committed, leave a
/// later transaction no leave to make the rewrite they named.
///
/// FAILS IF: the verifier reads a scrub row without requiring its event's `created = now()`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_scrub_authorisation_from_an_earlier_transaction_is_refused(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let f = families(&pool, owner, emitter, home).await;
    let rewrite = key_rewrite(&[f.a, f.u], &format!("scrubbed-key-{}", f.a));

    let record_only = Forged {
        authority: Authority::Scrub,
        subject: f.r,
        field: Some(property_family(f.a)),
        redacted_fields: redacted(&[(f.a, "property_key"), (f.u, "property_key")]),
        prelude: None,
        rewrite: "SELECT 1",
    };
    admitted(
        "the record and its rows, committed with no rewrite",
        attempt(&pool, emitter, &record_only, true).await,
    );
    let mut tx = pool.begin().await.unwrap();
    let later = sqlx::raw_sql(&rewrite)
        .execute(&mut *tx)
        .await
        .err()
        .map(|e| e.to_string());
    tx.rollback().await.unwrap();
    append_only("the rewrite in a later transaction", later);
}

/// Each authority admits only its own renamed-key sentinel (S6.2, S7): `scrubbed-key-<handle>`
/// is refused under an erasure row and `erased-key-<n>` under a scrub row, while each is admitted
/// under its own.
///
/// FAILS IF: `_erasure_sentinel_admits` ignores the row's authority.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn each_authority_admits_only_its_own_key_sentinel(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let f = families(&pool, owner, emitter, home).await;
    let rf = redacted(&[(f.a, "property_key"), (f.u, "property_key")]);
    let scrubbed = key_rewrite(&[f.a, f.u], &format!("scrubbed-key-{}", f.a));
    let erased = key_rewrite(&[f.a, f.u], "erased-key-1");
    let prelude = mark_erased(f.r);
    let erasure_writing_scrubbed = Forged {
        authority: Authority::Erasure,
        subject: f.r,
        field: None,
        redacted_fields: rf.clone(),
        prelude: Some(&prelude),
        rewrite: &scrubbed,
    };
    let erasure_writing_erased = Forged {
        rewrite: &erased,
        ..erasure_writing_scrubbed.clone()
    };

    append_only(
        "the scrub key sentinel under an erasure row",
        attempt(&pool, emitter, &erasure_writing_scrubbed, false).await,
    );
    admitted(
        "the erasure key sentinel under an erasure row",
        attempt(&pool, emitter, &erasure_writing_erased, false).await,
    );
    append_only(
        "the erasure key sentinel under a scrub row",
        scrub(
            &pool,
            emitter,
            f.r,
            property_family(f.a),
            rf.clone(),
            &erased,
        )
        .await,
    );
    admitted(
        "the scrub key sentinel under a scrub row",
        scrub(&pool, emitter, f.r, property_family(f.a), rf, &scrubbed).await,
    );
}

/// An erasure of a path a scrub already rewrote records its own row beside the scrub's and is
/// admitted (S6.1: the key is `(event_id, path, authority)`). Against the key `(event_id, path)`
/// the erasure's row collides with the scrub's.
///
/// FAILS IF: `kb_event_field_redactions`' key omits `authority`, or the row verifier stops at the
/// first row for a path instead of admitting when any row authorises it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_erasure_row_lands_beside_a_scrub_row_for_the_same_path(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let f = families(&pool, owner, emitter, home).await;
    let rf = redacted(&[(f.a, "property_key"), (f.u, "property_key")]);

    let scrubbed = key_rewrite(&[f.a, f.u], &format!("scrubbed-key-{}", f.a));
    let prelude = mark_erased(f.r);
    let erased = key_rewrite(&[f.a, f.u], "erased-key-1");
    let scrub_act = Forged {
        authority: Authority::Scrub,
        subject: f.r,
        field: Some(property_family(f.a)),
        redacted_fields: rf.clone(),
        prelude: None,
        rewrite: &scrubbed,
    };
    let erasure_act = Forged {
        authority: Authority::Erasure,
        subject: f.r,
        field: None,
        redacted_fields: rf,
        prelude: Some(&prelude),
        rewrite: &erased,
    };

    admitted(
        "the scrub, committed",
        attempt(&pool, emitter, &scrub_act, true).await,
    );
    admitted(
        "an erasure of the scrubbed paths, committed",
        attempt(&pool, emitter, &erasure_act, true).await,
    );
    let authorities: Vec<String> = sqlx::query_scalar(
        "SELECT authority FROM kb_event_field_redactions \
          WHERE event_id = $1 AND path = 'property_key' ORDER BY authority",
    )
    .bind(f.a)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        authorities,
        ["erasure", "scrub"],
        "one row per authority for the same (event, path)"
    );
}

// ── The act (migration 20261018100020): spec witnesses 1 and 7, review focus 1-4 ───────────────
//
// Every witness below calls `resource_field_scrub_execute`, `resource_field_scrub_plan`,
// `resource_field_scrub_survey` or `resource_field_scrub_families`, none of which exists before
// migration 20261018100020, so each fails there with an unknown function. The witnesses of the act
// end with replay byte-identical, which before the replay arm (Task 5) fails on the walk's refusal
// of `resource_scrubbed`.

/// The text of a database error: the raise's own message.
fn db_message(err: sqlx::Error) -> String {
    err.as_database_error()
        .map(|db| db.message().to_owned())
        .unwrap_or_else(|| err.to_string())
}

/// `resource_field_scrub_execute` as the system operator, under a fresh request reference, on its
/// own connection: a raise rolls the whole act back.
async fn try_act(
    pool: &PgPool,
    resource: ResourceId,
    field: &str,
    family: Option<Uuid>,
    clear: bool,
) -> Result<serde_json::Value, String> {
    let (operator, emitter) = system_actor(pool).await;
    sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT resource_field_scrub_execute($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(resource.uuid())
    .bind(field)
    .bind(family)
    .bind(clear)
    .bind(operator.uuid())
    .bind(emitter)
    .bind(Uuid::now_v7())
    .fetch_one(pool)
    .await
    .map_err(db_message)
}

async fn act(
    pool: &PgPool,
    resource: ResourceId,
    field: &str,
    family: Option<Uuid>,
    clear: bool,
) -> serde_json::Value {
    try_act(pool, resource, field, family, clear)
        .await
        .unwrap_or_else(|e| panic!("the scrub of {field} completes: {e}"))
}

async fn plan(
    pool: &PgPool,
    resource: ResourceId,
    field: &str,
    family: Option<Uuid>,
    clear: bool,
) -> serde_json::Value {
    sqlx::query_scalar("SELECT resource_field_scrub_plan($1, $2, $3, $4)")
        .bind(resource.uuid())
        .bind(field)
        .bind(family)
        .bind(clear)
        .fetch_one(pool)
        .await
        .expect("the plan computes")
}

async fn survey(
    pool: &PgPool,
    resource: ResourceId,
    field: &str,
    family: Option<Uuid>,
    clear: bool,
) -> serde_json::Value {
    sqlx::query_scalar("SELECT resource_field_scrub_survey($1, $2, $3, $4)")
        .bind(resource.uuid())
        .bind(field)
        .bind(family)
        .bind(clear)
        .fetch_one(pool)
        .await
        .expect("the survey computes")
}

async fn listing(pool: &PgPool, resource: ResourceId) -> Vec<serde_json::Value> {
    sqlx::query_scalar("SELECT to_jsonb(f) FROM resource_field_scrub_families($1) f")
        .bind(resource.uuid())
        .fetch_all(pool)
        .await
        .expect("the listing reads")
}

/// The handle the listing gives the family whose handle event carries `key` as its key text.
async fn handle_of(pool: &PgPool, resource: ResourceId, key: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT f.family FROM resource_field_scrub_families($1) f \
           JOIN kb_events e ON e.id = f.family WHERE e.payload->>'property_key' = $2",
    )
    .bind(resource.uuid())
    .bind(key)
    .fetch_one(pool)
    .await
    .expect("the listing names the family")
}

async fn payload_of(pool: &PgPool, event: Uuid) -> serde_json::Value {
    sqlx::query_scalar("SELECT payload FROM kb_events WHERE id = $1")
        .bind(event)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The `kb_properties` rows `event` asserted: key, value, folded.
async fn rows_of(pool: &PgPool, event: Uuid) -> Vec<(String, serde_json::Value, bool)> {
    sqlx::query_as(
        "SELECT property_key, property_value, is_folded FROM kb_properties \
          WHERE asserted_by_event_id = $1 ORDER BY property_value::text",
    )
    .bind(event)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// R's resource-owned `event_type` events of any key, in walk order.
async fn owned_events(pool: &PgPool, event_type: &str, resource: ResourceId) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = $1 AND e.payload#>>'{owner,table}' = 'kb_resources' \
            AND e.payload#>>'{owner,id}' = $2 ORDER BY e.id",
    )
    .bind(event_type)
    .bind(resource.uuid().to_string())
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn title_and_origin(pool: &PgPool, resource: ResourceId) -> (String, Option<String>) {
    sqlx::query_as("SELECT title, origin_uri FROM kb_resources WHERE id = $1")
        .bind(resource.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

fn event_id(out: &serde_json::Value) -> Uuid {
    out["event_id"]
        .as_str()
        .expect("the act returns its event id")
        .parse()
        .unwrap()
}

/// A `redacted_fields` list as `(event, paths)` pairs.
fn pairs(fields: &serde_json::Value) -> Vec<(Uuid, Vec<String>)> {
    fields
        .as_array()
        .expect("redacted_fields is a list")
        .iter()
        .map(|entry| {
            (
                entry["event"].as_str().unwrap().parse().unwrap(),
                entry["paths"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().unwrap().to_string())
                    .collect(),
            )
        })
        .collect()
}

fn pair(event: Uuid, paths: &[&str]) -> (Uuid, Vec<String>) {
    (event, paths.iter().map(|p| p.to_string()).collect())
}

/// A `kept` or `unreachable` entry of the plan.
fn held(event: Uuid, paths: &[&str], why: &str) -> serde_json::Value {
    serde_json::json!({"event": event, "paths": paths, "why": why})
}

fn value_sentinel(event: Uuid) -> serde_json::Value {
    serde_json::json!(format!("erased:{event}"))
}

fn key_sentinel(handle: Uuid) -> String {
    format!("scrubbed-key-{handle}")
}

async fn reorigin(pool: &PgPool, emitter: EntityId, resource: ResourceId, uri: &str) {
    write(
        pool,
        SeedAction::ResourceUpdate {
            resource,
            title: None,
            origin_uri: Some(uri),
            emitter,
        },
    )
    .await;
}

async fn set_value(
    pool: &PgPool,
    emitter: EntityId,
    resource: ResourceId,
    key: &str,
    value: serde_json::Value,
) {
    write(
        pool,
        SeedAction::PropertySet {
            resource,
            key,
            value: &value,
            weight: 1.0,
            emitter,
        },
    )
    .await;
}

/// A `property_asserted` of `facet` with `values`, the facet write path.
async fn facet(pool: &PgPool, emitter: EntityId, resource: ResourceId, values: serde_json::Value) {
    write(
        pool,
        SeedAction::FacetSet {
            owner: PropertyOwner::Resource { id: resource },
            values: &values,
            weight: 1.0,
            emitter,
        },
    )
    .await;
}

/// Copied from `resource_erasure_act.rs`: dump every projection, replay the ledger into a reset
/// schema, and require every dump equal.
async fn assert_replay_byte_identical(pool: &PgPool, after_what: &str) {
    let before = replay::dump_projections(pool).await.unwrap();
    let snap = replay::snapshot(pool).await.unwrap();
    common::reset_schema(pool).await;
    replay::replay(pool, &snap).await.unwrap();
    let after = replay::dump_projections(pool).await.unwrap();
    for ((ta, a), (tb, b)) in before.iter().zip(after.iter()) {
        assert_eq!(ta, tb);
        assert_eq!(
            a, b,
            "projection table {ta} diverged under replay {after_what}"
        );
    }
}

/// Spec witness 1, keep mode, the title with the revert (S3): created, then a leaking update, then
/// a revert to the original. The revert produces today's title and is kept; the create and the
/// leak are prior and take the title sentinel; the record names the field and nothing else.
///
/// FAILS IF: the act redacts the event producing today's title, misses a prior one, changes
/// `kb_resources.title`, or replay of the redacted ledger lands elsewhere.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn keep_mode_redacts_every_prior_title_and_keeps_the_revert(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-title", &[]).await;
    retitle(&pool, emitter, r, "a leaked title").await;
    retitle(&pool, emitter, r, "scrub-title").await;
    let created = resource_events(&pool, "resource_created", r).await[0];
    let updates = resource_events(&pool, "resource_updated", r).await;
    let (leak, revert) = (updates[0], updates[1]);

    let out = act(&pool, r, "title", None, false).await;

    assert_eq!(
        pairs(&out["redacted_fields"]),
        vec![pair(created, &["title"]), pair(leak, &["title"])]
    );
    let sentinel = serde_json::json!(format!("erased-{}", r.uuid()));
    assert_eq!(payload_of(&pool, created).await["title"], sentinel);
    assert_eq!(payload_of(&pool, leak).await["title"], sentinel);
    assert_eq!(payload_of(&pool, revert).await["title"], "scrub-title");
    assert_eq!(title_and_origin(&pool, r).await.0, "scrub-title");
    assert_eq!(out["cleared"], false);
    let record: ResourceScrubbed = serde_json::from_value(payload_of(&pool, event_id(&out)).await)
        .expect("the record is a ResourceScrubbed");
    assert_eq!(record.field.kind, ScrubFieldKind::Title);
    assert!(record.field.family.is_none());
    assert!(!record.cleared);
    assert_eq!(record.subject_id, r.uuid());
    assert_eq!(record.redacted_fields.len(), 2);

    assert_replay_byte_identical(&pool, "after a keep-mode title scrub").await;
}

/// Spec witness 1, keep mode, the origin URI: every origin URI before the last takes the origin
/// sentinel, and today's is kept.
///
/// FAILS IF: the act misses a prior origin URI, touches today's, or replay diverges.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn keep_mode_redacts_every_prior_origin_uri(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-origin", &[]).await;
    reorigin(&pool, emitter, r, "test://a-leaked-origin").await;
    reorigin(&pool, emitter, r, "test://scrub-origin-today").await;
    let created = resource_events(&pool, "resource_created", r).await[0];
    let updates = resource_events(&pool, "resource_updated", r).await;

    let out = act(&pool, r, "origin_uri", None, false).await;

    assert_eq!(
        pairs(&out["redacted_fields"]),
        vec![
            pair(created, &["origin_uri"]),
            pair(updates[0], &["origin_uri"])
        ]
    );
    let sentinel = serde_json::json!(format!("erased:{}", r.uuid()));
    assert_eq!(payload_of(&pool, created).await["origin_uri"], sentinel);
    assert_eq!(payload_of(&pool, updates[0]).await["origin_uri"], sentinel);
    assert_eq!(
        payload_of(&pool, updates[1]).await["origin_uri"],
        "test://scrub-origin-today"
    );
    assert_eq!(
        title_and_origin(&pool, r).await.1.as_deref(),
        Some("test://scrub-origin-today")
    );

    assert_replay_byte_identical(&pool, "after a keep-mode origin URI scrub").await;
}

/// Spec witness 1, keep mode, a family's values: the two folded values take their event's value
/// sentinel in the payload and in their folded rows; today's value is kept, and the key text of a
/// family never unset is unreachable (S3, S5). The act's record is the plan's `redacted_fields`.
///
/// FAILS IF: the plan and the act disagree, a live value is redacted, a key is renamed with no
/// unset, a folded row keeps its original value, or replay diverges.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn keep_mode_redacts_a_familys_prior_values_and_keeps_todays(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-values", &[]).await;
    set(&pool, emitter, r, "colour", "blue").await;
    set(&pool, emitter, r, "colour", "red").await;
    set(&pool, emitter, r, "colour", "green").await;
    let sets = property_events(&pool, "property_set", r, "colour").await;
    let (blue, red, green) = (sets[0], sets[1], sets[2]);
    let handle = handle_of(&pool, r, "colour").await;
    assert_eq!(handle, blue, "the handle is the family's first event");

    let p = plan(&pool, r, "property", Some(handle), false).await;
    assert!(p["refusal"].is_null(), "{p}");
    assert_eq!(
        pairs(&p["redacted_fields"]),
        vec![pair(blue, &["value"]), pair(red, &["value"])]
    );
    assert_eq!(
        p["kept"],
        serde_json::json!([held(green, &["value"], "current")])
    );
    assert_eq!(
        p["unreachable"],
        serde_json::json!([
            held(blue, &["property_key"], "after_latest_unset"),
            held(red, &["property_key"], "after_latest_unset"),
            held(green, &["property_key"], "after_latest_unset"),
        ])
    );

    let out = act(&pool, r, "property", Some(handle), false).await;

    assert_eq!(out["redacted_fields"], p["redacted_fields"]);
    for e in [blue, red] {
        let payload = payload_of(&pool, e).await;
        assert_eq!(payload["value"], value_sentinel(e));
        assert_eq!(payload["property_key"], "colour");
        assert_eq!(
            rows_of(&pool, e).await,
            vec![("colour".to_string(), value_sentinel(e), true)]
        );
    }
    assert_eq!(payload_of(&pool, green).await["value"], "green");
    assert_eq!(
        rows_of(&pool, green).await,
        vec![("colour".to_string(), serde_json::json!("green"), false)]
    );

    assert_replay_byte_identical(&pool, "after a keep-mode value scrub").await;
}

/// Review focus 2: `_property_value_normalized` wraps a string `tags` value in an array, and only
/// under the literal key. A string value redacted under the kept key `tags` projects as
/// `["erased:<id>"]`; once the family's key text is renamed past its unset, the same folded row
/// projects the plain string, as replay of the renamed event does.
///
/// FAILS IF: the projection rewrite writes the sentinel without the projector's normalisation, or
/// normalises by the original key rather than the key as rewritten.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_tags_sentinel_is_an_array_under_the_kept_key_and_a_string_once_renamed(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-tags", &[]).await;
    set(&pool, emitter, r, "tags", "first").await;
    set(&pool, emitter, r, "tags", "second").await;
    let sets = property_events(&pool, "property_set", r, "tags").await;
    let (first, second) = (sets[0], sets[1]);
    let handle = handle_of(&pool, r, "tags").await;

    act(&pool, r, "property", Some(handle), false).await;

    assert_eq!(
        payload_of(&pool, first).await["value"],
        value_sentinel(first)
    );
    assert_eq!(
        rows_of(&pool, first).await,
        vec![(
            "tags".to_string(),
            serde_json::json!([format!("erased:{first}")]),
            true
        )]
    );

    unset(&pool, emitter, r, "tags").await;
    act(&pool, r, "property", Some(handle), false).await;

    let renamed = key_sentinel(handle);
    for e in [first, second] {
        assert_eq!(payload_of(&pool, e).await["property_key"], renamed.as_str());
        assert_eq!(
            rows_of(&pool, e).await,
            vec![(renamed.clone(), value_sentinel(e), true)],
            "under the renamed key the sentinel stays a string"
        );
    }

    assert_replay_byte_identical(&pool, "after scrubbing tags under a kept and a renamed key")
        .await;
}

/// Spec witness 1, clear mode, the title (S2): the act appends the placeholder title through
/// `resource_update` under the request's correlation, after which every earlier title is prior.
///
/// FAILS IF: the placeholder is not today's title, an earlier title survives, the clearing event
/// is redacted, the record omits `cleared`, or replay diverges.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn clear_mode_sets_the_placeholder_title_and_redacts_every_earlier_one(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-clear-title", &[]).await;
    retitle(&pool, emitter, r, "a leaked title").await;
    let created = resource_events(&pool, "resource_created", r).await[0];
    let leak = resource_events(&pool, "resource_updated", r).await[0];

    let out = act(&pool, r, "title", None, true).await;

    let placeholder = format!("scrubbed-{}", r.uuid());
    assert_eq!(title_and_origin(&pool, r).await.0, placeholder);
    let updates = resource_events(&pool, "resource_updated", r).await;
    assert_eq!(updates.len(), 2, "the leak and the clearing event");
    let clearing = updates[1];
    assert_eq!(
        payload_of(&pool, clearing).await["title"],
        placeholder.as_str()
    );
    let correlations: Vec<Uuid> =
        sqlx::query_scalar("SELECT correlation_id FROM kb_events WHERE id = ANY($1) ORDER BY id")
            .bind(vec![clearing, event_id(&out)])
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        correlations[0], correlations[1],
        "one request, one correlation"
    );
    assert_eq!(
        pairs(&out["redacted_fields"]),
        vec![pair(created, &["title"]), pair(leak, &["title"])]
    );
    assert_eq!(out["cleared"], true);
    let record: ResourceScrubbed =
        serde_json::from_value(payload_of(&pool, event_id(&out)).await).unwrap();
    assert!(record.cleared);

    assert_replay_byte_identical(&pool, "after a clear-mode title scrub").await;
}

/// Spec witness 1, clear mode, a live key's text (S2, S3 "unset first"), and the survey's reading of
/// clear mode (S5): the act appends one `property_unset` of the family, after which every event of
/// the family, the unset included, is prior; the key text is renamed everywhere and no event of R
/// still carries it. The survey, which cannot append, names the same paths, minus the act's own
/// clearing event.
///
/// FAILS IF: a live row survives, any of R's events still carries the key text, a folded row keeps
/// the real key, the survey and the act disagree, or replay diverges.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn clear_mode_unsets_a_live_key_and_renames_its_text_everywhere(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-clear-key", &[]).await;
    set(&pool, emitter, r, "secretkey", "one").await;
    set(&pool, emitter, r, "secretkey", "two").await;
    let sets = property_events(&pool, "property_set", r, "secretkey").await;
    let handle = handle_of(&pool, r, "secretkey").await;
    assert_eq!(handle, sets[0]);

    let surveyed = survey(&pool, r, "property", Some(handle), true).await;
    assert_eq!(
        surveyed["plan"]["clears"],
        serde_json::json!([{"event_type": "property_unset", "path": "property_key"}])
    );
    assert!(surveyed["plan"]["refusal"].is_null(), "{surveyed}");
    assert!(
        !surveyed["families"].as_array().unwrap().is_empty(),
        "the survey renders the listing"
    );

    let out = act(&pool, r, "property", Some(handle), true).await;

    let unsets = owned_events(&pool, "property_unset", r).await;
    assert_eq!(unsets.len(), 1, "the act's clearing unset");
    let clearing = unsets[0];
    let renamed = key_sentinel(handle);
    assert_eq!(
        live_rows(&pool, "kb_resources", r.uuid(), "secretkey").await,
        0
    );
    assert_eq!(
        live_rows(&pool, "kb_resources", r.uuid(), &renamed).await,
        0
    );
    for e in [sets[0], sets[1]] {
        assert_eq!(
            rows_of(&pool, e).await,
            vec![(renamed.clone(), value_sentinel(e), true)]
        );
    }
    assert_eq!(
        payload_of(&pool, clearing).await["property_key"],
        renamed.as_str()
    );
    let carrying: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events \
          WHERE payload#>>'{owner,id}' = $1 AND payload::text LIKE '%secretkey%'",
    )
    .bind(r.uuid().to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(carrying, 0, "no event of R still carries the key text");

    let act_without_clearing: Vec<_> = pairs(&out["redacted_fields"])
        .into_iter()
        .filter(|(e, _)| *e != clearing)
        .collect();
    assert_eq!(
        act_without_clearing,
        pairs(&surveyed["plan"]["redacted_fields"]),
        "the survey names what the act redacts, minus the act's own clearing event"
    );
    assert!(
        pairs(&out["redacted_fields"]).contains(&pair(clearing, &["property_key"])),
        "the clearing unset's key text is redacted with the family"
    );

    assert_replay_byte_identical(&pool, "after a clear-mode scrub of a live key").await;
}

/// Spec witness 1, clear mode, `doc_type` (S2): the placeholder type `scrubbed` is set, which folds
/// the live type row, so `resource_created.doc_type` and the retype become prior and take their
/// event's doc-type sentinel; the key stays the structural literal. The family's handle is
/// `resource_created`, the row the listing gives it (erasure D4).
///
/// FAILS IF: the listing does not hand out `resource_created` as the doc_type family's handle, the
/// act refuses it, the key is renamed, a prior type survives, or replay diverges.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn clear_mode_sets_the_placeholder_type_and_keeps_the_doc_type_key(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-clear-type", &[]).await;
    set(&pool, emitter, r, "doc_type", "a-leaked-type").await;
    let created = resource_events(&pool, "resource_created", r).await[0];
    let retype = property_events(&pool, "property_set", r, "doc_type").await[0];
    assert!(
        listing(&pool, r)
            .await
            .iter()
            .any(|row| row["field"] == "property" && row["family"] == created.to_string()),
        "the doc_type family's handle is resource_created"
    );

    let out = act(&pool, r, "property", Some(created), true).await;

    let live: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT property_value FROM kb_properties \
          WHERE owner_id = $1 AND property_key = 'doc_type' AND NOT is_folded",
    )
    .bind(r.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(live, vec![serde_json::json!("scrubbed")]);
    assert_eq!(
        pairs(&out["redacted_fields"]),
        vec![pair(created, &["doc_type"]), pair(retype, &["value"])]
    );
    assert_eq!(
        payload_of(&pool, created).await["doc_type"],
        value_sentinel(created)
    );
    let retyped = payload_of(&pool, retype).await;
    assert_eq!(retyped["value"], value_sentinel(retype));
    assert_eq!(retyped["property_key"], "doc_type");
    assert_eq!(
        rows_of(&pool, created).await,
        vec![("doc_type".to_string(), value_sentinel(created), true)]
    );
    assert_eq!(
        rows_of(&pool, retype).await,
        vec![("doc_type".to_string(), value_sentinel(retype), true)]
    );

    assert_replay_byte_identical(&pool, "after a clear-mode doc_type scrub").await;
}

/// Spec witness 1, key text across an unset boundary, with a later lifecycle of the same key (S3):
/// A and U take the renamed key; B, after the unset and folded by C, has its value redacted and its
/// key kept; C, today's value, is untouched; the `size` family is untouched.
///
/// FAILS IF: a key is renamed past the latest unset, a prior value is missed, the later lifecycle
/// is touched, another family is reached, or replay diverges.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn keep_mode_renames_key_text_up_to_the_unset_and_keeps_the_later_lifecycle(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let f = families(&pool, owner, emitter, home).await;
    let c = property_events(&pool, "property_set", f.r, "colour").await[2];

    let out = act(&pool, f.r, "property", Some(f.a), false).await;

    assert_eq!(
        pairs(&out["redacted_fields"]),
        vec![
            pair(f.a, &["property_key", "value"]),
            pair(f.u, &["property_key"]),
            pair(f.b, &["value"]),
        ]
    );
    let renamed = key_sentinel(f.a);
    assert_eq!(
        payload_of(&pool, f.a).await["property_key"],
        renamed.as_str()
    );
    assert_eq!(
        payload_of(&pool, f.u).await["property_key"],
        renamed.as_str()
    );
    assert_eq!(payload_of(&pool, f.b).await["property_key"], "colour");
    assert_eq!(payload_of(&pool, c).await["value"], "green");
    assert_eq!(payload_of(&pool, f.s1).await["value"], "one");
    assert_eq!(
        rows_of(&pool, f.a).await,
        vec![(renamed, value_sentinel(f.a), true)]
    );
    assert_eq!(
        rows_of(&pool, f.b).await,
        vec![("colour".to_string(), value_sentinel(f.b), true)]
    );

    assert_replay_byte_identical(&pool, "after a key-text scrub across an unset").await;
}

/// Spec witness 1, facets (S3, S7). E1 {status, owner} is partly folded by E2 {status} and fully by
/// the whole-facet set S; E3 {a, b} is folded by S. Those three are prior: one mark per original,
/// each inner key renamed by the first event naming it and its position there, so E2's status mark
/// takes E1's status sentinel. S and E4, fully folded but not before the latest whole-facet fold,
/// are unreachable; E6 {c, d}, partly live, and the live E5 and E7 are kept.
///
/// FAILS IF: an event at or after the latest whole-facet fold is rewritten, a partly-live event is
/// rewritten, an inner key is renamed inconsistently across events, a mark row keeps its inner key,
/// or replay diverges.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn facets_are_scrubbed_before_the_latest_whole_fold_and_named_unreachable_after(
    pool: PgPool,
) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-facets", &[]).await;
    facet(
        &pool,
        emitter,
        r,
        serde_json::json!({"status": "open", "owner": "x"}),
    )
    .await;
    facet(&pool, emitter, r, serde_json::json!({"status": "closed"})).await;
    facet(&pool, emitter, r, serde_json::json!({"a": 1, "b": 2})).await;
    set_value(
        &pool,
        emitter,
        r,
        "facet",
        serde_json::json!({"phase": "one"}),
    )
    .await;
    facet(&pool, emitter, r, serde_json::json!({"phase": "two"})).await;
    facet(&pool, emitter, r, serde_json::json!({"phase": "three"})).await;
    facet(&pool, emitter, r, serde_json::json!({"c": 1, "d": 2})).await;
    facet(&pool, emitter, r, serde_json::json!({"c": 3})).await;
    let asserted = property_events(&pool, "property_asserted", r, "facet").await;
    assert_eq!(asserted.len(), 7);
    let (e1, e2, e3, e4, e5, e6, e7) = (
        asserted[0],
        asserted[1],
        asserted[2],
        asserted[3],
        asserted[4],
        asserted[5],
        asserted[6],
    );
    let s = property_events(&pool, "property_set", r, "facet").await[0];
    let handle = handle_of(&pool, r, "facet").await;
    assert_eq!(handle, e1);

    let p = plan(&pool, r, "property", Some(handle), false).await;
    assert_eq!(
        p["unreachable"],
        serde_json::json!([
            held(s, &["value"], "after_whole_facet_fold"),
            held(e4, &["value"], "after_whole_facet_fold"),
        ])
    );
    assert_eq!(
        p["kept"],
        serde_json::json!([
            held(e5, &["value"], "current"),
            held(e6, &["value"], "current"),
            held(e7, &["value"], "current"),
        ])
    );

    let out = act(&pool, r, "property", Some(handle), false).await;

    assert_eq!(
        pairs(&out["redacted_fields"]),
        vec![
            pair(e1, &["value"]),
            pair(e2, &["value"]),
            pair(e3, &["value"])
        ]
    );
    // jsonb stores object keys by length, then bytes: E1 holds "owner" at 1 and "status" at 2.
    let mark = |event: Uuid, pos: u32| format!("scrubbed-facet-{event}-{pos}");
    assert_eq!(
        payload_of(&pool, e1).await["value"],
        serde_json::json!({mark(e1, 1): "erased", mark(e1, 2): "erased"})
    );
    assert_eq!(
        payload_of(&pool, e2).await["value"],
        serde_json::json!({mark(e1, 2): "erased"}),
        "E2's status mark takes the sentinel of status's first appearance"
    );
    assert_eq!(
        payload_of(&pool, e3).await["value"],
        serde_json::json!({mark(e3, 1): "erased", mark(e3, 2): "erased"})
    );
    for (event, value) in [
        (s, serde_json::json!({"phase": "one"})),
        (e4, serde_json::json!({"phase": "two"})),
        (e6, serde_json::json!({"c": 1, "d": 2})),
    ] {
        assert_eq!(payload_of(&pool, event).await["value"], value);
    }
    assert_eq!(
        rows_of(&pool, e2).await,
        vec![(
            "facet".to_string(),
            serde_json::json!({mark(e1, 2): "erased"}),
            true
        )]
    );
    assert_eq!(
        rows_of(&pool, e1).await.len(),
        2,
        "one mark row per original mark"
    );

    assert_replay_byte_identical(&pool, "after a facet scrub").await;
}

/// Review focus 1: a family scrubbed twice across lifecycles. The first scrub renames A and U to
/// `scrubbed-key-<A>`. The owner sets the key again and unsets it; the family's handle is now B,
/// the first event still carrying the text, so the second scrub's sentinel differs from the
/// first's, and the first scrub's events are untouched.
///
/// FAILS IF: the handle is not recomputed from the events still carrying the text, the second scrub
/// rewrites the first one's events, the two lifecycles share a sentinel, or replay diverges.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_family_scrubbed_twice_across_lifecycles_takes_two_sentinels(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-twice", &[]).await;
    set(&pool, emitter, r, "colour", "blue").await;
    unset(&pool, emitter, r, "colour").await;
    let a = property_events(&pool, "property_set", r, "colour").await[0];
    let u1 = property_events(&pool, "property_unset", r, "colour").await[0];
    let first = handle_of(&pool, r, "colour").await;
    assert_eq!(first, a);
    act(&pool, r, "property", Some(first), false).await;
    let (a_after, u1_after) = (payload_of(&pool, a).await, payload_of(&pool, u1).await);

    set(&pool, emitter, r, "colour", "red").await;
    unset(&pool, emitter, r, "colour").await;
    let b = property_events(&pool, "property_set", r, "colour").await[0];
    let u2 = property_events(&pool, "property_unset", r, "colour").await[0];
    let second = handle_of(&pool, r, "colour").await;
    assert_eq!(
        second, b,
        "the handle is the first event still carrying the text"
    );
    act(&pool, r, "property", Some(second), false).await;

    assert_ne!(key_sentinel(first), key_sentinel(second));
    assert_eq!(
        payload_of(&pool, a).await,
        a_after,
        "the first scrub's events are untouched"
    );
    assert_eq!(payload_of(&pool, u1).await, u1_after);
    for e in [b, u2] {
        assert_eq!(
            payload_of(&pool, e).await["property_key"],
            key_sentinel(second).as_str()
        );
    }

    assert_replay_byte_identical(&pool, "after scrubbing a family across two lifecycles").await;
}

/// Review focus 3: a key never set on R, nulled in `open_meta`, fires a lone `property_unset`. It
/// is a family in the listing, unset and never live, with no value type, and the listing carries
/// no key text. Its key text is scrubbable, and the scrub leaves no row.
///
/// FAILS IF: the lone unset is not a family, the listing carries key text, the scrub refuses it,
/// or replay diverges.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_family_whose_only_event_is_an_unset_is_listed_and_scrubbable(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-lone-unset", &[]).await;
    unset(&pool, emitter, r, "ghostkey").await;
    let u = property_events(&pool, "property_unset", r, "ghostkey").await[0];

    let rows = listing(&pool, r).await;
    let row = rows
        .iter()
        .find(|row| row["family"] == u.to_string())
        .expect("the lone unset is a family");
    assert_eq!(row["events"], 1);
    assert_eq!(row["live"], false);
    assert_eq!(row["unset"], true);
    assert!(row["value_type"].is_null());
    assert!(
        !serde_json::to_string(&rows).unwrap().contains("ghostkey"),
        "the listing carries no key text"
    );

    let out = act(&pool, r, "property", Some(u), false).await;

    assert_eq!(
        pairs(&out["redacted_fields"]),
        vec![pair(u, &["property_key"])]
    );
    assert_eq!(
        payload_of(&pool, u).await["property_key"],
        key_sentinel(u).as_str()
    );
    assert!(rows_of(&pool, u).await.is_empty());

    assert_replay_byte_identical(&pool, "after scrubbing a lone unset").await;
}

/// Review focus 4: an edge-owned property event with the family's key text never joins R's family.
/// It is refused as a handle, and a scrub of R's family leaves its payload and row byte-identical.
///
/// FAILS IF: the family reaches past `kb_resources`-owned events, or an edge-owned event is
/// accepted as a handle.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_owned_event_with_the_same_key_text_is_not_in_the_family(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-edge-r", &[]).await;
    let twin = create(&pool, owner, emitter, home, "scrub-edge-twin", &[]).await;
    set(&pool, emitter, r, "colour", "blue").await;
    unset(&pool, emitter, r, "colour").await;
    let edge = writes::assert_anchored_edge_with(
        &pool,
        writes::AssertAnchoredEdgeParams {
            source: AnchorRef::resource(r),
            target: AnchorRef::resource(twin),
            kind: temper_substrate::affinity::EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("derived_from"),
            weight: 1.0,
            home: EdgeHome::Context(home),
            emitter,
        },
        EventContext::default(),
    )
    .await
    .unwrap();
    writes::assert_keyed_property_with(
        &pool,
        PropertyOwner::Edge { id: edge },
        "colour",
        &serde_json::json!("edge-blue"),
        1.0,
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();
    let edge_event: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_events WHERE payload#>>'{owner,id}' = $1")
            .bind(edge.uuid().to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
    let (payload_before, rows_before) = (
        payload_of(&pool, edge_event).await,
        rows_of(&pool, edge_event).await,
    );

    let err = try_act(&pool, r, "property", Some(edge_event), false)
        .await
        .expect_err("an edge-owned event is not a handle");
    assert!(
        err.contains(&format!(
            "resource_field_scrub_execute: family {edge_event} is not a property family handle of resource {}",
            r.uuid()
        )),
        "got {err}"
    );
    let handle = handle_of(&pool, r, "colour").await;
    act(&pool, r, "property", Some(handle), false).await;

    assert_eq!(payload_of(&pool, edge_event).await, payload_before);
    assert_eq!(rows_of(&pool, edge_event).await, rows_before);

    assert_replay_byte_identical(&pool, "after a scrub beside an edge-owned key").await;
}

/// Spec witness 7, the title guard (S3): a title-carrying event inserted, in one transaction, with
/// an explicit id lower than the last one's and projected, so the walk's last title is not the
/// projection's. Keep mode refuses with `projection disagrees`; the same request just before the
/// inversion is admitted by the plan.
///
/// FAILS IF: the act redacts under an inverted order instead of refusing.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn keep_mode_refuses_when_the_last_title_disagrees_with_the_projection(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-guard", &[]).await;
    retitle(&pool, emitter, r, "a later title").await;
    let last = resource_events(&pool, "resource_updated", r).await[0];
    let inverted = Uuid::from_u128(last.as_u128() - 1);
    let payload = serde_json::json!({"resource_id": r.uuid(), "title": "an inverted title"});
    let (operator, _) = system_actor(&pool).await;

    let mut tx = pool.begin().await.unwrap();
    let before: serde_json::Value =
        sqlx::query_scalar("SELECT resource_field_scrub_plan($1, 'title', NULL, false)")
            .bind(r.uuid())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert!(before["refusal"].is_null(), "{before}");
    sqlx::query(
        "INSERT INTO kb_events (id, event_type_id, emitter_entity_id, producing_anchor_table, \
                                producing_anchor_id, payload, category) \
         SELECT $1, et.id, $2, h.anchor_table, h.anchor_id, $3, 'domain' \
           FROM kb_event_types et, kb_resource_homes h \
          WHERE et.name = 'resource_updated' AND h.resource_id = $4",
    )
    .bind(inverted)
    .bind(emitter)
    .bind(&payload)
    .bind(r.uuid())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("SELECT _project_resource_updated($1, $2)")
        .bind(inverted)
        .bind(&payload)
        .execute(&mut *tx)
        .await
        .unwrap();
    let err =
        sqlx::query("SELECT resource_field_scrub_execute($1, 'title', NULL, false, $2, $3, $4)")
            .bind(r.uuid())
            .bind(operator.uuid())
            .bind(emitter)
            .bind(Uuid::now_v7())
            .execute(&mut *tx)
            .await
            .map(|_| ())
            .map_err(db_message)
            .expect_err("an inverted title order refuses");
    tx.rollback().await.unwrap();

    assert_eq!(err, "resource_field_scrub_execute: projection disagrees");
}

/// Assert that the act on `resource` raises `expected` and that the ledger gained no event.
async fn refused_with(
    pool: &PgPool,
    resource: ResourceId,
    field: &str,
    family: Option<Uuid>,
    clear: bool,
    expected: &str,
) {
    let before = event_count(pool).await;
    let err = try_act(pool, resource, field, family, clear)
        .await
        .expect_err(expected);
    assert!(err.contains(expected), "expected {expected:?}, got {err}");
    assert_eq!(
        event_count(pool).await,
        before,
        "{expected}: nothing recorded"
    );
}

/// The act's refusals RAISE, each with its own stable text, and change nothing (S4). The recorded
/// ones (a charter, an erased resource) and the 400s (a foreign or malformed handle, a malformed
/// request, keep mode with nothing prior) are told apart by the service from these texts.
///
/// FAILS IF: any refusal completes, records an event, or raises a different text.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_act_refuses_by_raising_and_changes_nothing(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let f = families(&pool, owner, emitter, home).await;
    let other = create(&pool, owner, emitter, home, "scrub-refuse-other", &[]).await;
    let other_created = resource_events(&pool, "resource_created", other).await[0];
    let untouched = create(&pool, owner, emitter, home, "scrub-refuse-fresh", &[]).await;
    retitle(&pool, emitter, f.r, "a later title").await;
    let r_update = resource_events(&pool, "resource_updated", f.r).await[0];
    let foreign = |id: Uuid| {
        format!(
            "resource_field_scrub_execute: family {id} is not a property family handle of resource {}",
            f.r.uuid()
        )
    };

    refused_with(
        &pool,
        f.r,
        "property",
        Some(other_created),
        false,
        &foreign(other_created),
    )
    .await;
    refused_with(
        &pool,
        f.r,
        "property",
        Some(r_update),
        false,
        &foreign(r_update),
    )
    .await;
    refused_with(&pool, f.r, "property", Some(f.u), false, &foreign(f.u)).await;
    refused_with(
        &pool,
        f.r,
        "title",
        Some(f.a),
        false,
        "resource_field_scrub_execute: field title takes no family handle",
    )
    .await;
    refused_with(
        &pool,
        f.r,
        "property",
        None,
        false,
        "resource_field_scrub_execute: field property needs a family handle",
    )
    .await;
    refused_with(
        &pool,
        f.r,
        "properties",
        None,
        true,
        "resource_field_scrub_execute: properties cannot be cleared",
    )
    .await;
    refused_with(
        &pool,
        f.r,
        "body",
        None,
        false,
        "resource_field_scrub_execute: unknown field kind",
    )
    .await;
    refused_with(
        &pool,
        untouched,
        "title",
        None,
        false,
        "resource_field_scrub_execute: nothing prior to scrub",
    )
    .await;

    let telos = {
        let mut conn = pool.acquire().await.unwrap();
        fire(
            &mut conn,
            SeedAction::CogmapGenesis {
                name: "field-scrub-refusal-map",
                telos_title: "telos-for-field-scrub-refusal",
                charter: &[],
                cogmap_id: None,
                telos_resource_id: None,
                owner,
                emitter,
            },
        )
        .await
        .unwrap()
        .cogmap_genesis()
        .unwrap()
        .1
    };
    refused_with(
        &pool,
        telos,
        "title",
        None,
        false,
        "resource_field_scrub_execute: charter resource (map-grain erasure is filed task",
    )
    .await;

    erase(&pool, other).await;
    refused_with(
        &pool,
        other,
        "title",
        None,
        false,
        "resource_field_scrub_execute: already erased",
    )
    .await;
    let surveyed = survey(&pool, other, "title", None, false).await;
    assert_eq!(surveyed["plan"]["refusal"], "already_erased");
    assert!(
        surveyed["families"].is_null(),
        "no listing for an erased resource"
    );
}

/// A sentinel the act would write that R's ledger already names as key text refuses (S7,
/// `sentinel_collision`), and the refusal records through `resource_erasure_refuse` as a field
/// scrub's, typed `ResourceErasureRefused { act: FieldScrub, reason: SentinelCollision }`. The
/// reason is refused beside any other act.
///
/// FAILS IF: the act renames a family onto key text R already uses, the refusal record does not
/// admit the field scrub's act and reason, or admits the reason for another act.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_sentinel_already_on_the_owners_ledger_refuses_and_records_as_a_field_scrub(
    pool: PgPool,
) {
    let (owner, emitter, home) = setup(&pool).await;
    let f = families(&pool, owner, emitter, home).await;
    set(
        &pool,
        emitter,
        f.r,
        &key_sentinel(f.a),
        "typed in the sentinel's shape",
    )
    .await;

    assert_eq!(
        plan(&pool, f.r, "property", Some(f.a), false).await["refusal"],
        "sentinel_collision"
    );
    refused_with(
        &pool,
        f.r,
        "property",
        Some(f.a),
        false,
        "resource_field_scrub_execute: sentinel collision",
    )
    .await;

    let refusal: Uuid = sqlx::query_scalar(
        "SELECT resource_erasure_refuse($1, $2, $3, $4, 'sentinel_collision', NULL, 'field_scrub')",
    )
    .bind(f.r.uuid())
    .bind(owner.uuid())
    .bind(emitter)
    .bind(Uuid::now_v7())
    .fetch_one(&pool)
    .await
    .expect("the field scrub's refusal records");
    let recorded: ResourceErasureRefused =
        serde_json::from_value(payload_of(&pool, refusal).await).expect("a ResourceErasureRefused");
    assert_eq!(recorded.act, Some(ErasureAct::FieldScrub));
    assert_eq!(recorded.reason, RecordedRefusalReason::SentinelCollision);
    assert_eq!(recorded.subject_id, f.r.uuid());
    assert!(recorded.blocks.is_empty());

    let err = sqlx::query_scalar::<_, Uuid>(
        "SELECT resource_erasure_refuse($1, $2, $3, $4, 'sentinel_collision', NULL, 'erasure')",
    )
    .bind(f.r.uuid())
    .bind(owner.uuid())
    .bind(emitter)
    .bind(Uuid::now_v7())
    .fetch_one(&pool)
    .await
    .map_err(db_message)
    .expect_err("the reason is the field scrub's alone");
    assert_eq!(
        err,
        "resource_erasure_refuse: sentinel_collision is recorded only for a field_scrub refusal"
    );
}

/// The bite of spec witness 1 (S3, "a `property_set` cannot be the boundary"): rename one event's
/// key past a `property_set` boundary, as a derivation without S3's unset rule would, and rewrite
/// its folded row to match, as the projection rewrite would. Replay of that ledger leaves the
/// renamed row live, because the later set folds by its own key text, while the live projection
/// holds it folded: the dumps differ. The verifier refuses this rename outright
/// (`a_scrub_renames_key_text_only_up_to_the_latest_unset`), so the forgery bypasses the ledger's
/// triggers, which the test role may do on its own schema.
///
/// FAILS IF: replay of a key renamed past a set boundary lands where live is, which would mean
/// S3's unset rule guards nothing and the witnesses above could not catch its loss.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_key_renamed_past_a_set_boundary_makes_replay_diverge(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-bite", &[]).await;
    set(&pool, emitter, r, "colour", "blue").await;
    set(&pool, emitter, r, "colour", "red").await;
    let a = property_events(&pool, "property_set", r, "colour").await[0];
    let renamed = key_sentinel(a);

    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(
        "ALTER TABLE kb_events DISABLE TRIGGER kb_events_append_only, \
                               DISABLE TRIGGER kb_events_redaction_in_trail",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("UPDATE kb_events SET payload = jsonb_set(payload, '{property_key}', to_jsonb($2::text)) WHERE id = $1")
        .bind(a)
        .bind(&renamed)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE kb_properties SET property_key = $2 WHERE asserted_by_event_id = $1")
        .bind(a)
        .bind(&renamed)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(
        "ALTER TABLE kb_events ENABLE TRIGGER kb_events_append_only, \
                               ENABLE TRIGGER kb_events_redaction_in_trail",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        rows_of(&pool, a).await,
        vec![(renamed.clone(), serde_json::json!("blue"), true)],
        "live: the renamed row is folded"
    );

    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();

    let table = |dumps: &[(String, serde_json::Value)]| {
        dumps
            .iter()
            .find(|(t, _)| t == "kb_properties")
            .map(|(_, d)| d.clone())
            .unwrap()
    };
    assert_ne!(
        table(&before),
        table(&after),
        "replay of a key renamed past a set boundary must diverge"
    );
    assert_eq!(
        rows_of(&pool, a).await,
        vec![(renamed, serde_json::json!("blue"), false)],
        "replay: the later set folds by its own key text and leaves the renamed row live"
    );
}

// ── Replay (Task 5): the resource_scrubbed arm ─────────────────────────────────────────────────

/// Spec witness 2: scrub (keep, a family's key text), then erase, then replay byte-identical. The
/// erasure completes unchanged over the scrubbed ledger: its rows for the paths the scrub rewrote
/// land beside the scrub's (S6.1), and the walk projects both records at their positions.
///
/// FAILS IF: replay has no arm for `resource_scrubbed` (it refused the walk before Task 5), the
/// erasure collides with the scrub's rows, or replay diverges.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_erasure_after_a_scrub_completes_and_replays_identically(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let f = families(&pool, owner, emitter, home).await;
    act(&pool, f.r, "property", Some(f.a), false).await;

    erase(&pool, f.r).await;

    let authorities: Vec<String> = sqlx::query_scalar(
        "SELECT authority FROM kb_event_field_redactions \
          WHERE event_id = $1 AND path = 'property_key' ORDER BY authority",
    )
    .bind(f.a)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(authorities, ["erasure", "scrub"]);

    assert_replay_byte_identical(&pool, "after a scrub, then an erasure").await;
}

/// The replay arm refuses a `resource_scrubbed` whose payload names another table, planted by a
/// raw ledger append (the act never writes it).
///
/// FAILS IF: replay succeeds, or the refusal does not name the offending table.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_refuses_a_resource_scrubbed_naming_another_table(pool: PgPool) {
    let (_, emitter, _) = setup(&pool).await;
    sqlx::query(
        "SELECT _event_append('resource_scrubbed', $1, NULL, NULL, \
                jsonb_build_object('subject_table', 'kb_content_blocks', \
                                   'subject_id', $2::uuid, \
                                   'field', jsonb_build_object('kind', 'title')), \
                p_correlation => $3)",
    )
    .bind(emitter)
    .bind(Uuid::now_v7())
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect("the raw append lands");

    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    let err = replay::replay(&pool, &snap)
        .await
        .expect_err("replay must refuse a resource_scrubbed naming another table");
    let chain = format!("{err:#}");
    assert!(
        chain.contains("names subject_table \"kb_content_blocks\", not kb_resources"),
        "{chain}"
    );
}

// ── Review fixes (migration 20261018100040) ─────────────────────────────────────────────────────
//
// Each witness below fails against 20261018100030's functions, as its FAILS IF says. The bite for
// the verifier witnesses: drop the exact-sentinel check from `kb_events_redaction_in_trail` (its
// second EXISTS in the scrub loop) and the split, merge and facet forgeries are admitted.

/// A `property_asserted` of `key` on `resource` with the explicit id `id`, projected now: an event
/// whose id was minted before events that committed ahead of it (an inversion across
/// transactions, erasure D14). Appended as the write path appends it, with the id chosen.
async fn assert_at(
    pool: &PgPool,
    emitter: EntityId,
    resource: ResourceId,
    id: Uuid,
    key: &str,
    value: serde_json::Value,
) {
    let payload = serde_json::json!({
        "property_id": Uuid::now_v7(),
        "owner": {"table": "kb_resources", "id": resource.uuid()},
        "property_key": key,
        "value": value,
        "weight": 1.0,
    });
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO kb_events (id, event_type_id, emitter_entity_id, producing_anchor_table, \
                                producing_anchor_id, payload, category) \
         SELECT $1, et.id, $2, a.anchor_table, a.anchor_id, $3, 'domain' \
           FROM kb_event_types et, _property_owner_anchor('kb_resources', $4) a \
          WHERE et.name = 'property_asserted'",
    )
    .bind(id)
    .bind(emitter)
    .bind(&payload)
    .bind(resource.uuid())
    .execute(&mut *tx)
    .await
    .expect("the inverted event appends");
    sqlx::query("SELECT _project_property_asserted($1, $2)")
        .bind(id)
        .bind(&payload)
        .execute(&mut *tx)
        .await
        .expect("the inverted event projects");
    tx.commit().await.unwrap();
}

/// A scrub may not split a family across two key sentinels (S7: the sentinel is
/// `scrubbed-key-<the family's handle>`, exactly). A and U are the colour family's at-or-before-
/// the-unset events. Renaming U to any other sentinel-shaped key is refused; so is renaming A
/// alone, which would let a second statement under a second record rename U to its own handle's
/// sentinel. Renaming both to the family's own sentinel in one statement is admitted.
///
/// FAILS IF: the verifier admits a renamed key by its shape alone (20261018100010's
/// `_erasure_sentinel_admits` pattern), so U lands on `scrubbed-key-<another uuid>` and replay of
/// the unset folds nothing of A's; or it admits a statement that renames part of a family's prior
/// key text.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_scrub_refuses_a_family_split_across_two_key_sentinels(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let f = families(&pool, owner, emitter, home).await;
    let rf = redacted(&[(f.a, "property_key"), (f.u, "property_key")]);
    let split = format!(
        "UPDATE kb_events SET payload = jsonb_set(payload, '{{property_key}}', \
                 to_jsonb(CASE WHEN id = '{u}' THEN '{other}' ELSE '{own}' END)) \
          WHERE id IN ({list})",
        u = f.u,
        other = key_sentinel(Uuid::now_v7()),
        own = key_sentinel(f.a),
        list = in_list(&[f.a, f.u]),
    );

    // Across two statements: renaming A alone leaves U the family's only carrier of the text, so
    // U's handle becomes U and a second record could rename it to `scrubbed-key-<U>`.
    append_only(
        "part of the family's prior key text renamed",
        scrub(
            &pool,
            emitter,
            f.r,
            property_family(f.a),
            redacted(&[(f.a, "property_key")]),
            &key_rewrite(&[f.a], &key_sentinel(f.a)),
        )
        .await,
    );
    append_only(
        "the family's unset renamed to another sentinel",
        scrub(
            &pool,
            emitter,
            f.r,
            property_family(f.a),
            rf.clone(),
            &split,
        )
        .await,
    );
    admitted(
        "both renamed to the family's own sentinel",
        scrub(
            &pool,
            emitter,
            f.r,
            property_family(f.a),
            rf,
            &key_rewrite(&[f.a, f.u], &key_sentinel(f.a)),
        )
        .await,
    );
}

/// A scrub may not rename a family onto another family's sentinel, nor onto key text the owner
/// typed in a sentinel's shape (S7). Both families are unset, so every rename is otherwise prior;
/// only the sentinel differs.
///
/// FAILS IF: the verifier admits a renamed key by shape, so `colour` merges into `shade` (whose own
/// scrub would write `scrubbed-key-<shade>`) or into the planted key, and replay of either unset
/// folds the other family's rows.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_scrub_refuses_a_family_renamed_onto_another_familys_sentinel(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-merge", &[]).await;
    set(&pool, emitter, r, "colour", "blue").await;
    unset(&pool, emitter, r, "colour").await;
    set(&pool, emitter, r, "shade", "dark").await;
    unset(&pool, emitter, r, "shade").await;
    let planted = key_sentinel(Uuid::now_v7());
    set(&pool, emitter, r, &planted, "typed in the sentinel's shape").await;
    let a = property_events(&pool, "property_set", r, "colour").await[0];
    let u = property_events(&pool, "property_unset", r, "colour").await[0];
    let shade = property_events(&pool, "property_set", r, "shade").await[0];
    let rf = redacted(&[(a, "property_key"), (u, "property_key")]);

    for (what, onto) in [
        ("colour renamed onto shade's sentinel", key_sentinel(shade)),
        ("colour renamed onto a planted sentinel-shaped key", planted),
    ] {
        append_only(
            what,
            scrub(
                &pool,
                emitter,
                r,
                property_family(a),
                rf.clone(),
                &key_rewrite(&[a, u], &onto),
            )
            .await,
        );
    }
    admitted(
        "colour renamed onto its own sentinel",
        scrub(
            &pool,
            emitter,
            r,
            property_family(a),
            rf,
            &key_rewrite(&[a, u], &key_sentinel(a)),
        )
        .await,
    );
}

/// A scrub may not map two facet inner keys onto one sentinel (S7: each inner key takes
/// `scrubbed-facet-<id of the first event naming it>-<its position there>`). E1 {a} and E2 {b} are
/// folded by the whole-facet set, so both are prior; renaming E2's `b` onto E1's `a` sentinel is
/// refused, and each onto its own is admitted.
///
/// FAILS IF: the verifier admits a facet value by shape and mark count alone, so `b` merges into
/// `a` and a later inner-key fold of either reaches both.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_scrub_refuses_two_facet_inner_keys_mapped_onto_one_sentinel(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-facet-forge", &[]).await;
    facet(&pool, emitter, r, serde_json::json!({"a": "1"})).await;
    facet(&pool, emitter, r, serde_json::json!({"b": "2"})).await;
    set_value(&pool, emitter, r, "facet", serde_json::json!({"z": "9"})).await;
    let asserted = property_events(&pool, "property_asserted", r, "facet").await;
    let (e1, e2) = (asserted[0], asserted[1]);
    let handle = handle_of(&pool, r, "facet").await;
    assert_eq!(handle, e1);
    let rf = redacted(&[(e1, "value"), (e2, "value")]);
    let merged = format!(
        "UPDATE kb_events SET payload = jsonb_set(payload, '{{value}}', \
                 jsonb_build_object('scrubbed-facet-{e1}-1', 'erased')) \
          WHERE id IN ({})",
        in_list(&[e1, e2])
    );
    let own = format!(
        "UPDATE kb_events SET payload = jsonb_set(payload, '{{value}}', \
                 jsonb_build_object('scrubbed-facet-' || id::text || '-1', 'erased')) \
          WHERE id IN ({})",
        in_list(&[e1, e2])
    );

    append_only(
        "two inner keys onto one sentinel",
        scrub(
            &pool,
            emitter,
            r,
            property_family(handle),
            rf.clone(),
            &merged,
        )
        .await,
    );
    admitted(
        "each inner key onto its own sentinel",
        scrub(&pool, emitter, r, property_family(handle), rf, &own).await,
    );
}

/// Security review F1: the owner types a key in the shape of family F's sentinel,
/// `scrubbed-key-<F>`, as family G. F's scrub refuses (`sentinel collision`), as S7 rules. G's own
/// clear-mode scrub then renames G's key text to G's own sentinel, though it already had a
/// sentinel's shape, so F's scrub completes afterwards and no event of R carries F's key text.
///
/// FAILS IF: the derivation treats any sentinel-shaped key as already done, so G keeps
/// `scrubbed-key-<F>` after its own scrub and F stays refused for good; or replay diverges.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_key_typed_in_anothers_sentinel_shape_is_renamed_by_its_own_scrub(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-planted", &[]).await;
    set(&pool, emitter, r, "secretkey", "one").await;
    set(&pool, emitter, r, "secretkey", "two").await;
    let f = handle_of(&pool, r, "secretkey").await;
    set(&pool, emitter, r, &key_sentinel(f), "planted").await;
    let g = handle_of(&pool, r, &key_sentinel(f)).await;

    refused_with(
        &pool,
        r,
        "property",
        Some(f),
        true,
        "resource_field_scrub_execute: sentinel collision",
    )
    .await;

    act(&pool, r, "property", Some(g), true).await;
    let g_events: Vec<Uuid> = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e WHERE e.payload#>>'{owner,id}' = $1 \
            AND e.payload->>'property_key' = $2 ORDER BY e.id",
    )
    .bind(r.uuid().to_string())
    .bind(key_sentinel(g))
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        g_events.len(),
        2,
        "G's set and its clearing unset take G's own sentinel"
    );
    assert_eq!(
        property_events(&pool, "property_set", r, &key_sentinel(f))
            .await
            .len(),
        0,
        "no event still carries F's sentinel as key text"
    );

    act(&pool, r, "property", Some(f), true).await;
    let carrying: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events \
          WHERE payload#>>'{owner,id}' = $1 AND payload::text LIKE '%secretkey%'",
    )
    .bind(r.uuid().to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(carrying, 0, "no event of R still carries F's key text");

    assert_replay_byte_identical(&pool, "after scrubbing a planted family, then F").await;
}

/// Verifier review F2 (S6.3): a `property_asserted` whose id sorts between the set and the unset
/// but whose projection landed after the unset holds a live row under the real key. Its key text is
/// not prior: the act holds it as `current`, the verifier refuses a rename of it, and its live row
/// keeps a key text the family listing still names. Renaming the rest of the family keeps replay
/// identical: the renamed unset folds only the renamed rows, as live did. (Without the scrub, this
/// inversion diverges on replay by itself, erasure D14: the real-keyed unset would fold the row.)
///
/// FAILS IF: key text at or before the unset is prior though its event asserts a live row, so the
/// act renames it (the live row keeps the real key under an event that no longer names it, and
/// replay folds it at the renamed unset), or the verifier admits that rename.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_key_whose_event_asserts_a_live_row_is_kept_though_it_sorts_before_the_unset(
    pool: PgPool,
) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-inverted-key", &[]).await;
    set(&pool, emitter, r, "secretkey", "one").await;
    let e1 = property_events(&pool, "property_set", r, "secretkey").await[0];
    unset(&pool, emitter, r, "secretkey").await;
    let u = property_events(&pool, "property_unset", r, "secretkey").await[0];
    // Sorts just before the unset, as an id minted before it in a transaction committed after it.
    let inverted = Uuid::from_u128(u.as_u128() - 1);
    assert_at(
        &pool,
        emitter,
        r,
        inverted,
        "secretkey",
        serde_json::json!("live-value"),
    )
    .await;
    assert!(
        e1 < inverted && inverted < u,
        "the asserted's id sorts between the set and the unset"
    );

    append_only(
        "the live event's key renamed with the family",
        scrub(
            &pool,
            emitter,
            r,
            property_family(e1),
            redacted(&[
                (e1, "property_key"),
                (inverted, "property_key"),
                (u, "property_key"),
            ]),
            &key_rewrite(&[e1, inverted, u], &key_sentinel(e1)),
        )
        .await,
    );

    let p = plan(&pool, r, "property", Some(e1), false).await;
    assert_eq!(
        p["kept"],
        serde_json::json!([held(inverted, &["property_key", "value"], "current")])
    );

    let out = act(&pool, r, "property", Some(e1), false).await;

    assert_eq!(
        pairs(&out["redacted_fields"]),
        vec![
            pair(e1, &["property_key", "value"]),
            pair(u, &["property_key"])
        ]
    );
    assert_eq!(
        payload_of(&pool, inverted).await["property_key"],
        "secretkey"
    );
    assert_eq!(
        rows_of(&pool, inverted).await,
        vec![(
            "secretkey".to_string(),
            serde_json::json!("live-value"),
            false
        )]
    );
    assert!(
        listing(&pool, r)
            .await
            .iter()
            .any(|row| row["family"] == inverted.to_string() && row["live"] == true),
        "the live row's family is listed by the inverted event, the first still carrying its key"
    );

    assert_replay_byte_identical(&pool, "after scrubbing around an inverted live key").await;
}

/// Replay review F1 (S8 amended): a clear-mode scrub of a `tags` family renames every event that
/// rebuilt R's search vector live. `keywords`, asserted after the tag, rebuilds nothing on its own,
/// so live the vector reached `kwterm` only at the act's clearing unset. The act rebuilds the vector
/// last and replay's arm rebuilds it at the record, so replay ends on the live vector, and the
/// secret tag is in neither.
///
/// FAILS IF: the replay arm does not rebuild the subject's vector (replay ends on the vector the
/// create left, without `kwterm`), or the live vector still matches the cleared tag.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_cleared_tags_family_leaves_the_search_vector_replay_reaches(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-vector", &[]).await;
    set(&pool, emitter, r, "tags", "secretword").await;
    let keywords = serde_json::json!("kwterm");
    write(
        &pool,
        SeedAction::PropertyAssert {
            resource: r,
            key: "keywords",
            value: &keywords,
            weight: 1.0,
            emitter,
        },
    )
    .await;
    let handle = handle_of(&pool, r, "tags").await;

    act(&pool, r, "property", Some(handle), true).await;

    let live = search_vector(&pool, r)
        .await
        .expect("the resource has a vector");
    assert!(!vector_matches(&pool, r, "secretword").await, "{live}");
    assert!(vector_matches(&pool, r, "kwterm").await, "{live}");

    assert_replay_byte_identical(&pool, "after clearing a tags family").await;
    assert_eq!(
        search_vector(&pool, r).await.as_deref(),
        Some(live.as_str()),
        "replay ends on the live search vector"
    );
}

/// Locking review F1 (S2, S4): clear mode is not repeatable. Once a clear has landed, the same clear
/// of the title, a property family (by the handle the listing now gives it) or `doc_type` raises
/// `already cleared` and records nothing; the plan, which the survey renders, answers
/// `already_cleared`, and a keep-mode plan of the cleared title has nothing prior. Those two answers
/// are what the field scrub's reconcile hint reads as "the act landed".
///
/// FAILS IF: a repeated clear appends a second placeholder, unset or type and a second
/// `resource_scrubbed`, or the plan does not say the field is already cleared.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_clear_that_landed_answers_already_cleared_and_records_nothing(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(&pool, owner, emitter, home, "scrub-clear-twice", &[]).await;
    retitle(&pool, emitter, r, "a leaked title").await;
    set(&pool, emitter, r, "secretkey", "one").await;
    set(&pool, emitter, r, "secretkey", "two").await;
    let key = handle_of(&pool, r, "secretkey").await;
    let doc_type = resource_events(&pool, "resource_created", r).await[0];
    const ALREADY: &str = "resource_field_scrub_execute: already cleared";

    act(&pool, r, "title", None, true).await;
    assert_eq!(
        plan(&pool, r, "title", None, true).await["refusal"],
        "already_cleared"
    );
    assert_eq!(
        plan(&pool, r, "title", None, false).await["refusal"],
        "nothing_prior"
    );
    refused_with(&pool, r, "title", None, true, ALREADY).await;

    act(&pool, r, "property", Some(key), true).await;
    let renamed: Uuid = sqlx::query_scalar(
        "SELECT handle FROM _field_scrub_property_events($1) WHERE event_id = $2",
    )
    .bind(r.uuid())
    .bind(key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        plan(&pool, r, "property", Some(renamed), true).await["refusal"],
        "already_cleared"
    );
    refused_with(&pool, r, "property", Some(renamed), true, ALREADY).await;

    act(&pool, r, "property", Some(doc_type), true).await;
    refused_with(&pool, r, "property", Some(doc_type), true, ALREADY).await;

    let records: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_scrubbed' AND e.payload->>'subject_id' = $1",
    )
    .bind(r.uuid().to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(records, 3, "one record per clear that landed");
}
