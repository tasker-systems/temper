#![cfg(feature = "test-db")]
//! Field-grain scrub witnesses (spec 2026-10-09 field-grain-scrub).
//!
//! `_project_property_unset` (migration 20261018100000) is the one body of the `property_unset`
//! projection: the write guard, the fold of `(kb_resources, owner, key)`'s live rows, and the
//! search-vector rebuild for `keywords` / `descriptor` / `tags`. The fire path and replay both
//! reach it through `events::project_property_unset`; a SQL act clearing a property family
//! (S4 step 3) can call it directly. These witnesses call it directly, against a world built
//! through the real write paths.

mod common;

use sha2::Digest;
use sqlx::PgPool;
use temper_core::types::ids::EntityId;
use temper_core::types::property_owner::PropertyOwner;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, ProfileId, ResourceId};
use temper_substrate::payloads::{self, AnchorRef, EdgePolarity};
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
