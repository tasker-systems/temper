#![cfg(feature = "artifact-tests")]
//! Data artifacts — the Beat A write path (`migrations/20260820000020_data_artifacts.sql`).
//!
//! Agents already persist structured data in temper, as fenced JSON/YAML inside resource bodies,
//! read back by a LATER, unrelated session. Three failures of that practice were measured on
//! 2026-08-20: `kb_properties` rejects anything over 2704 bytes (btree v4 index maximum), the
//! chunker shreds a fence at its own YAML comment lines (`heading_re()` has no fence-state
//! tracking), and every fragment is then embedded into a corpus built for prose.
//!
//! Beat A is SQL-only — there is no `fire()` action or typed payload yet (Beat B) — so these tests
//! drive `data_artifact_commit` directly, which is the write path as it currently exists.
//!
//! What is actually unknown here, and therefore what these pin:
//!
//! 1. **That committing an artifact changes nothing about the searchable corpus.** This is the
//!    goal's standing negative clause `structured-data-is-never-found-by-resemblance`. The test
//!    carries a POSITIVE CONTROL, because "nothing changed" is also what a broken detector reports.
//! 2. **That the store folds only what the writer named.** No uniqueness index exists on purpose;
//!    a second artifact of the same kind must sit alongside the first unless the writer said
//!    otherwise (`no-supersession-is-asserted-that-a-writer-did-not-declare`).
//! 3. **Where the event anchors**, including the cogmap tiebreak for a doubly-homed resource.
//!
//! Harness + seeding helpers follow the per-file convention of this suite.

mod common;

use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::payloads::{ArtifactIntent, KindOwner};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

// ── fixtures ──────────────────────────────────────────────────────────────────────────────────

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

async fn make_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    title: &str,
) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: title,
            body: "seed body",
            doc_type: "research",
            home: AnchorRef::context(home),
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

/// A world with one resource homed in one context.
async fn world(pool: &sqlx::PgPool, slug: &str) -> (EntityId, ContextId, ResourceId) {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let home = ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    );
    let resource = make_resource(pool, owner, emitter, home, "measurement subject").await;
    (emitter, home, resource)
}

/// The payload `data_artifact_commit` takes. It carries the content HASH and never the content —
/// the bytes ride the wrapper's own `p_content` argument (see [`commit`]).
///
/// The content is returned alongside so callers can pass both without recomputing the hash.
fn payload(
    artifact: Uuid,
    resource: ResourceId,
    kind: &str,
    intent: &str,
    content: serde_json::Value,
    supersedes: &[Uuid],
) -> serde_json::Value {
    let raw = serde_json::to_string(&content).unwrap();
    serde_json::json!({
        "artifact_id":   artifact,
        "resource_id":   resource.uuid(),
        "artifact_kind": kind,
        "intent":        intent,
        "content_hash":  format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(raw.as_bytes())),
        "content_bytes": raw.len() as i64,
        "supersedes":    supersedes,
        // Carried here ONLY so `commit` can lift it back out into the separate argument. The
        // wrapper refuses a payload that still holds it — see `a_payload_carrying_content_is_refused`.
        "__test_content": content,
    })
}

/// Drive the SQL wrapper directly, splitting `__test_content` out of the payload into `p_content`
/// the way `fire()` does.
async fn commit(
    pool: &sqlx::PgPool,
    emitter: EntityId,
    mut p: serde_json::Value,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let content = p
        .as_object_mut()
        .and_then(|o| o.remove("__test_content"))
        .unwrap_or(serde_json::Value::Null);
    sqlx::query_scalar::<_, Vec<Uuid>>("SELECT data_artifact_commit($1, $2, $3)")
        .bind(p)
        .bind(content)
        .bind(emitter.uuid())
        .fetch_one(pool)
        .await
}

/// Live (non-folded) artifacts of a resource, as `(id, kind, intent)` in creation order.
async fn live(pool: &sqlx::PgPool, r: ResourceId) -> Vec<(Uuid, String, String)> {
    sqlx::query_as(
        "SELECT id, artifact_kind, intent FROM kb_data_artifacts
          WHERE resource_id=$1 AND NOT is_folded ORDER BY created, id",
    )
    .bind(r.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Is `token` reachable by resemblance on this resource? Checks every surface that makes text
/// findable: the FTS vector, and the chunk corpus (whose text is what gets embedded).
///
/// This asks the direct question rather than a proxy. An earlier version of this test compared a
/// before/after "fingerprint" of chunk counts and the FTS vector, and it could NOT detect the
/// failure it claimed to guard — see `committing_an_artifact_changes_nothing_findable`.
async fn findable_by_token(pool: &sqlx::PgPool, r: ResourceId, token: &str) -> (bool, bool) {
    let in_fts: bool = sqlx::query_scalar(
        "SELECT COALESCE(bool_or(search_vector @@ plainto_tsquery('english', $2)), false)
           FROM kb_resource_search_index WHERE resource_id=$1",
    )
    .bind(r.uuid())
    .bind(token)
    .fetch_one(pool)
    .await
    .unwrap();

    let in_chunks: bool = sqlx::query_scalar(
        "SELECT COALESCE(bool_or(cc.content LIKE '%' || $2 || '%'), false)
           FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id
          WHERE c.resource_id = $1",
    )
    .bind(r.uuid())
    .bind(token)
    .fetch_one(pool)
    .await
    .unwrap();

    (in_fts, in_chunks)
}

// ── the clauses ───────────────────────────────────────────────────────────────────────────────

/// `structured-data-survives-the-round-trip` — bytes out equal bytes in, with no reassembly.
///
/// The content deliberately contains a `# comment` line: as prose in a resource body that is
/// exactly what the chunker misreads as a markdown heading and splits on.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn committed_content_round_trips_byte_identical(pool: sqlx::PgPool) {
    let (emitter, _home, resource) = world(&pool, "round-trip").await;
    let content = serde_json::json!({
        "# candidate weights": {"w_express": 0.4, "w_contains": 0.2},
        "notes": "a\n\nb\n# not a heading\n",
    });
    let id = Uuid::now_v7();

    commit(
        &pool,
        emitter,
        payload(id, resource, "measurement", "member", content.clone(), &[]),
    )
    .await
    .unwrap();

    let (stored, hash): (serde_json::Value, String) = sqlx::query_as(
        "SELECT content, content_hash FROM kb_data_artifact_content WHERE artifact_id=$1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(stored, content, "content must survive verbatim");

    let row_hash: String =
        sqlx::query_scalar("SELECT content_hash FROM kb_data_artifacts WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        row_hash, hash,
        "metadata row and content table agree on the hash"
    );
}

/// `structured-data-is-never-found-by-resemblance` — the standing negative clause.
///
/// **This test was rewritten after being shown not to bite.** Its first form compared a
/// before/after fingerprint (chunk count, embedded count, FTS vector) and asserted "unchanged".
/// Injecting the forbidden `_rebuild_resource_search_vector` call into the projector did NOT make
/// it fail — because that call recomputes the vector from the resource's OWN title/body/properties,
/// to which an artifact contributes nothing. It is a no-op, not a leak, so the test was passing on
/// a proxy that could never move.
///
/// This form asks the direct question instead: is a distinctive token that exists ONLY inside the
/// artifact reachable by text search on its owning resource? The positive control writes the same
/// token where it genuinely does become findable, proving the probe can observe a leak at all.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn committing_an_artifact_changes_nothing_findable(pool: sqlx::PgPool) {
    let (emitter, _home, resource) = world(&pool, "no-resemblance").await;
    const TOKEN: &str = "zzqxunfindabletoken";

    commit(
        &pool,
        emitter,
        payload(
            Uuid::now_v7(),
            resource,
            "measurement",
            "member",
            serde_json::json!({ "note": format!("contains {TOKEN} and nothing else does") }),
            &[],
        ),
    )
    .await
    .unwrap();

    let (in_fts, in_chunks) = findable_by_token(&pool, resource, TOKEN).await;
    assert!(
        !in_fts,
        "the artifact's content reached the resource's FTS vector — it must be reachable only \
         through what points at it"
    );
    assert!(
        !in_chunks,
        "the artifact's content reached the chunk corpus"
    );

    // ── positive control ──
    // The same token, written where it IS meant to be findable. If this does not become reachable,
    // the probe above cannot observe a leak and its two passing assertions mean nothing.
    sqlx::query("SELECT property_set($1, $2)")
        .bind(serde_json::json!({
            "property_id": Uuid::now_v7(),
            "owner": {"table": "kb_resources", "id": resource.uuid()},
            "property_key": "tags",
            "value": [TOKEN],
            "weight": 1.0,
        }))
        .bind(emitter.uuid())
        .execute(&pool)
        .await
        .unwrap();

    let (control_fts, _) = findable_by_token(&pool, resource, TOKEN).await;
    assert!(
        control_fts,
        "positive control failed: the probe cannot observe a token becoming findable, so this \
         test could never detect an artifact leaking into the corpus"
    );
}

/// `no-supersession-is-asserted-that-a-writer-did-not-declare` — the absent unique index.
///
/// Two artifacts of the SAME kind on the SAME resource, neither declaring supersession, must both
/// stay live. A `uq_..._active`-style index (which every sibling assert/fold table has) would make
/// the second write either fail or silently fold the first.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_second_artifact_of_a_kind_does_not_displace_the_first(pool: sqlx::PgPool) {
    let (emitter, _home, resource) = world(&pool, "has-many").await;

    for run in 0..3 {
        commit(
            &pool,
            emitter,
            payload(
                Uuid::now_v7(),
                resource,
                "measurement",
                "member",
                serde_json::json!({ "run": run }),
                &[],
            ),
        )
        .await
        .unwrap();
    }

    assert_eq!(
        live(&pool, resource).await.len(),
        3,
        "three runs were committed and none claimed to replace another; all three must be live"
    );
}

/// Fold-and-reassert: revision is the folded chain, and there is no mutable `revised` column.
/// The prior row survives as history rather than being overwritten or deleted.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_declared_supersession_folds_only_what_was_named(pool: sqlx::PgPool) {
    let (emitter, _home, resource) = world(&pool, "fold-chain").await;

    let first = Uuid::now_v7();
    let bystander = Uuid::now_v7();
    commit(
        &pool,
        emitter,
        payload(
            first,
            resource,
            "extraction",
            "current",
            serde_json::json!({"v": 1}),
            &[],
        ),
    )
    .await
    .unwrap();
    commit(
        &pool,
        emitter,
        payload(
            bystander,
            resource,
            "measurement",
            "member",
            serde_json::json!({"v": 0}),
            &[],
        ),
    )
    .await
    .unwrap();

    let second = Uuid::now_v7();
    commit(
        &pool,
        emitter,
        payload(
            second,
            resource,
            "extraction",
            "current",
            serde_json::json!({"v": 2}),
            &[first],
        ),
    )
    .await
    .unwrap();

    let live_ids: Vec<Uuid> = live(&pool, resource)
        .await
        .into_iter()
        .map(|r| r.0)
        .collect();
    assert!(live_ids.contains(&second), "the new revision is live");
    assert!(
        !live_ids.contains(&first),
        "the named prior revision folded"
    );
    assert!(
        live_ids.contains(&bystander),
        "an artifact the writer did not name must be untouched, even of a different kind"
    );

    // History survives: the folded row is still there, carrying the event that folded it.
    let (folded, last_event): (bool, Uuid) =
        sqlx::query_as("SELECT is_folded, last_event_id FROM kb_data_artifacts WHERE id=$1")
            .bind(first)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        folded,
        "the prior revision is retained, folded — never deleted"
    );

    let asserting: Uuid =
        sqlx::query_scalar("SELECT asserted_by_event_id FROM kb_data_artifacts WHERE id=$1")
            .bind(second)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        last_event, asserting,
        "the fold is attributed to the event that asserted the replacement"
    );
}

/// `a-declined-act-teaches-its-vocabulary` — a refusal carries the answer, not just a complaint.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_unrecognized_intent_is_refused_with_the_vocabulary(pool: sqlx::PgPool) {
    let (emitter, _home, resource) = world(&pool, "vocabulary").await;

    let err = commit(
        &pool,
        emitter,
        payload(
            Uuid::now_v7(),
            resource,
            "measurement",
            "latest",
            serde_json::json!({}),
            &[],
        ),
    )
    .await
    .expect_err("an intent outside the closed vocabulary must be refused");

    let msg = err.to_string();
    for term in ["current", "member", "pinned"] {
        assert!(
            msg.contains(term),
            "the refusal must name the vocabulary so the caller learns it; missing {term:?} in: {msg}"
        );
    }
}

/// The event anchors through the owning resource's home, and the artifact is reachable from it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_event_anchors_on_the_owning_resources_home(pool: sqlx::PgPool) {
    let (emitter, home, resource) = world(&pool, "anchoring").await;
    let id = Uuid::now_v7();

    commit(
        &pool,
        emitter,
        payload(
            id,
            resource,
            "query-plan",
            "pinned",
            serde_json::json!({"stages": []}),
            &[],
        ),
    )
    .await
    .unwrap();

    let (table, anchor): (String, Uuid) = sqlx::query_as(
        "SELECT e.producing_anchor_table, e.producing_anchor_id
           FROM kb_events e JOIN kb_data_artifacts a ON a.asserted_by_event_id = e.id
          WHERE a.id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(table, "kb_contexts");
    assert_eq!(
        anchor,
        home.uuid(),
        "anchored on the owning resource's home"
    );
}

/// A resource with no home cannot anchor an artifact event, and says so rather than writing a
/// NULL-anchored one. Mirrors `_property_owner_anchor`'s refusal.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_unhomed_resource_is_refused_not_silently_null_anchored(pool: sqlx::PgPool) {
    let (emitter, _home, resource) = world(&pool, "unhomed").await;
    sqlx::query("DELETE FROM kb_resource_homes WHERE resource_id=$1")
        .bind(resource.uuid())
        .execute(&pool)
        .await
        .unwrap();

    let err = commit(
        &pool,
        emitter,
        payload(
            Uuid::now_v7(),
            resource,
            "measurement",
            "member",
            serde_json::json!({}),
            &[],
        ),
    )
    .await
    .expect_err("an unhomed resource must be refused");
    assert!(
        err.to_string().contains("no home to anchor"),
        "the refusal must name the reason: {err}"
    );
}

/// The 2704-byte wall that motivated the whole design: an artifact comfortably exceeds what
/// `kb_properties` can physically hold, and stores fine here.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_artifact_exceeds_what_a_property_can_physically_hold(pool: sqlx::PgPool) {
    let (emitter, _home, resource) = world(&pool, "over-the-wall").await;
    // HIGH ENTROPY, deliberately. An earlier fixture used `format!("{i:0>32}")` — near-all-zero
    // strings, which TOAST compresses to well under the index limit, so the write SUCCEEDED and the
    // test proved nothing. The index measures the datum as stored; a fixture for this wall must be
    // incompressible or it never reaches it.
    let content = serde_json::json!({
        "rows": (0..300).map(|_| Uuid::now_v7().to_string()).collect::<Vec<_>>()
    });
    assert!(
        serde_json::to_string(&content).unwrap().len() > 2704,
        "the fixture must actually exceed the btree v4 index-row maximum to be a test of anything"
    );

    // The incumbent path refuses this outright.
    let as_property = sqlx::query("SELECT property_set($1, $2)")
        .bind(serde_json::json!({
            "property_id": Uuid::now_v7(),
            "owner": {"table": "kb_resources", "id": resource.uuid()},
            "property_key": "measurement",
            "value": content.clone(),
            "weight": 1.0,
        }))
        .bind(emitter.uuid())
        .execute(&pool)
        .await;
    assert!(
        as_property.is_err(),
        "if kb_properties has stopped rejecting oversized values, the premise of this design has \
         changed and the spec needs revisiting"
    );

    // The artifact path takes it.
    let id = Uuid::now_v7();
    commit(
        &pool,
        emitter,
        payload(id, resource, "measurement", "member", content.clone(), &[]),
    )
    .await
    .unwrap();
    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT content FROM kb_data_artifact_content WHERE artifact_id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, content);
}

/// The kind namespace defaults to the owning resource's home owner, and is resolved INTO THE
/// PAYLOAD at commit rather than at projection.
///
/// The payload-carrying matters for replay: a context's owner can change (`context_reassigned`), so
/// a projector that re-resolved the namespace would qualify an old artifact with today's owner and
/// the byte-exact diff would fail. Identity-as-input applies to the namespace too.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_kind_namespace_defaults_from_the_home_and_rides_the_payload(pool: sqlx::PgPool) {
    let (emitter, home, resource) = world(&pool, "namespace-default").await;
    let id = Uuid::now_v7();

    // The caller says only "query-plan" — no namespace. That is the whole anti-friction promise.
    commit(
        &pool,
        emitter,
        payload(
            id,
            resource,
            "query-plan",
            "current",
            serde_json::json!({"stages": []}),
            &[],
        ),
    )
    .await
    .unwrap();

    let (owner_table, owner_id): (String, Uuid) =
        sqlx::query_as("SELECT kind_owner_table, kind_owner_id FROM kb_data_artifacts WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let (ctx_table, ctx_owner): (String, Uuid) =
        sqlx::query_as("SELECT owner_table, owner_id FROM kb_contexts WHERE id=$1")
            .bind(home.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(
        (owner_table.as_str(), owner_id),
        (ctx_table.as_str(), ctx_owner),
        "a bare family name lands in the owning resource's namespace"
    );

    // The resolved namespace is in the EVENT payload, not merely in the projected row.
    let stored: serde_json::Value = sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e
           JOIN kb_data_artifacts a ON a.asserted_by_event_id = e.id WHERE a.id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        stored["kind_owner_id"]
            .as_str()
            .and_then(|s| s.parse::<Uuid>().ok()),
        Some(owner_id),
        "the resolved namespace must be payload-carried so replay reproduces it verbatim"
    );
}

/// An explicit namespace is honoured — naming another owner's family is possible, just never
/// implicit.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_explicit_kind_namespace_overrides_the_default(pool: sqlx::PgPool) {
    let (emitter, _home, resource) = world(&pool, "namespace-explicit").await;
    let team = common::create_team(&pool, "someone-elses-team").await;
    let id = Uuid::now_v7();

    let mut p = payload(
        id,
        resource,
        "query-plan",
        "current",
        serde_json::json!({}),
        &[],
    );
    p["kind_owner_table"] = serde_json::json!("kb_teams");
    p["kind_owner_id"] = serde_json::json!(team);
    commit(&pool, emitter, p).await.unwrap();

    let (owner_table, owner_id): (String, Uuid) =
        sqlx::query_as("SELECT kind_owner_table, kind_owner_id FROM kb_data_artifacts WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((owner_table.as_str(), owner_id), ("kb_teams", team));
}

/// Two owners may hold a family of the SAME bare name, and their artifacts never conflate.
/// This is the collision the qualification exists to make impossible.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_same_bare_name_under_two_owners_stays_distinct(pool: sqlx::PgPool) {
    let (emitter, _home, resource) = world(&pool, "namespace-collision").await;
    let team = common::create_team(&pool, "other-team").await;

    let mine = Uuid::now_v7();
    commit(
        &pool,
        emitter,
        payload(
            mine,
            resource,
            "query-plan",
            "member",
            serde_json::json!({"whose": "mine"}),
            &[],
        ),
    )
    .await
    .unwrap();

    let theirs = Uuid::now_v7();
    let mut p = payload(
        theirs,
        resource,
        "query-plan",
        "member",
        serde_json::json!({"whose": "theirs"}),
        &[],
    );
    p["kind_owner_table"] = serde_json::json!("kb_teams");
    p["kind_owner_id"] = serde_json::json!(team);
    commit(&pool, emitter, p).await.unwrap();

    let distinct: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT (kind_owner_table, kind_owner_id)) FROM kb_data_artifacts
          WHERE resource_id=$1 AND artifact_kind='query-plan' AND NOT is_folded",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        distinct, 2,
        "one bare name, two namespaces — a flat namespace would have conflated these, and a shape \
         registered by one owner would then have stamped verdicts on the other's data"
    );
}

// ── Beat B: the typed Rust path, and replay ───────────────────────────────────────────────────

/// Commit through `fire()` — the typed path — rather than the raw SQL wrapper.
async fn fire_commit(
    pool: &sqlx::PgPool,
    resource: ResourceId,
    kind: &str,
    intent: ArtifactIntent,
    content: &serde_json::Value,
    emitter: EntityId,
) -> temper_substrate::ids::DataArtifactId {
    let mut tx = pool.begin().await.unwrap();
    let id = fire(
        &mut tx,
        SeedAction::DataArtifactCommit {
            resource,
            kind,
            kind_owner: None,
            intent,
            precedence: 0.0,
            content,
            supersedes: &[],
            emitter,
        },
    )
    .await
    .unwrap()
    .data_artifact()
    .unwrap();
    tx.commit().await.unwrap();
    id
}

/// `the-governance-record-outlives-the-data` — the ledger records the act and proves the bytes,
/// without holding them.
///
/// This is the property the whole metadata/bytes split exists for. It is easy to *say* and easy to
/// lose: `_event_append` writes whatever is in `p_payload` verbatim, so a single well-meaning
/// "just put the content in the payload, it's simpler" would move every artifact body into
/// `kb_events` and nothing downstream would complain.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_ledger_carries_the_hash_and_never_the_body(pool: sqlx::PgPool) {
    let (emitter, _home, resource) = world(&pool, "hash-not-body").await;
    const TOKEN: &str = "zzqxbodytoken";
    let content = serde_json::json!({ "secret": TOKEN, "rows": [1, 2, 3] });

    let id = fire_commit(
        &pool,
        resource,
        "measurement",
        ArtifactIntent::Member,
        &content,
        emitter,
    )
    .await;

    let event: serde_json::Value = sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e
           JOIN kb_data_artifacts a ON a.asserted_by_event_id = e.id WHERE a.id = $1",
    )
    .bind(id.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();

    let rendered = serde_json::to_string(&event).unwrap();
    assert!(
        !rendered.contains(TOKEN),
        "the artifact body reached the event ledger: {rendered}"
    );
    for key in ["__content", "content"] {
        assert!(
            event.get(key).is_none(),
            "the event payload carries a {key:?} key — the split has been defeated"
        );
    }
    assert!(
        event.get("content_hash").is_some(),
        "the payload must carry the hash it uses in place of the body"
    );

    // And the hash actually proves the stored bytes.
    let (stored, hash): (serde_json::Value, String) = sqlx::query_as(
        "SELECT content, content_hash FROM kb_data_artifact_content WHERE artifact_id=$1",
    )
    .bind(id.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, content);
    assert_eq!(
        event["content_hash"].as_str().unwrap(),
        hash,
        "the ledger's hash and the stored content's hash must agree, or the ledger proves nothing"
    );
}

/// The wrapper refuses a payload that smuggles content, so the split cannot be bypassed by a caller
/// that does not go through `fire()`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_payload_carrying_content_is_refused(pool: sqlx::PgPool) {
    let (emitter, _home, resource) = world(&pool, "no-smuggling").await;
    let mut p = payload(
        Uuid::now_v7(),
        resource,
        "measurement",
        "member",
        serde_json::json!({}),
        &[],
    );
    p["__content"] = serde_json::json!({"smuggled": true});

    let err = commit(&pool, emitter, p)
        .await
        .expect_err("a payload carrying content must be refused");
    assert!(
        err.to_string().contains("never the body"),
        "the refusal must name the reason: {err}"
    );
}

/// Artifacts replay byte-identically: the metadata rows rebuild from the ledger, and the bytes come
/// back from the sidecar.
///
/// The fold is included deliberately. A folded artifact keeps its content row (fold affects
/// visibility, never existence), so a sidecar pass that filtered on `NOT is_folded` would drop
/// those bytes and replay would reproduce a partial content table — which only a round-trip like
/// this can catch.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn artifacts_replay_byte_identically(pool: sqlx::PgPool) {
    use temper_substrate::replay;
    let (emitter, _home, resource) = world(&pool, "replay").await;

    let first = fire_commit(
        &pool,
        resource,
        "extraction",
        ArtifactIntent::Current,
        &serde_json::json!({"v": 1, "note": "superseded below"}),
        emitter,
    )
    .await;
    fire_commit(
        &pool,
        resource,
        "measurement",
        ArtifactIntent::Member,
        &serde_json::json!({"run": 1, "rows": [1, 2, 3]}),
        emitter,
    )
    .await;

    // A supersession, so the replayed set includes a folded row AND its retained bytes.
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::DataArtifactCommit {
            resource,
            kind: "extraction",
            kind_owner: Some(KindOwner::Profile(Uuid::nil())),
            intent: ArtifactIntent::Current,
            precedence: 1.0,
            content: &serde_json::json!({"v": 2}),
            supersedes: &[first],
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();

    let mut checked = 0;
    for ((ta, va), (tb, vb)) in before.iter().zip(after.iter()) {
        assert_eq!(ta, tb);
        assert_eq!(va, vb, "projection table {ta} diverged under replay");
        if ta.starts_with("kb_data_artifact") {
            checked += 1;
            assert!(
                va.as_array().is_some_and(|a| !a.is_empty()),
                "{ta} is empty, so comparing it proves nothing — the fixture must actually write rows"
            );
        }
    }
    assert_eq!(
        checked, 2,
        "both artifact tables must be in PROJECTION_DUMPS"
    );
}
