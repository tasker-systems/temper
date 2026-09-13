#![cfg(feature = "test-db")]
//! Witnesses for the read-only erasure survey (task 01a09628 item 2, ruled 2026-09-12).
//! The survey shares the act's ONE computation (`principal_erasure_survey_plan`,
//! migration 20260913000010 — the act consumes the same plan), so three behaviors:
//! the survey's prediction IS the subsequent act's record, per target, for every blob
//! classification (the differential — including the verdict bite: a hash with a second
//! live row in another home must predict released=false); a survey writes NOTHING (the
//! whole ledger and the touched projections stay byte-identical); and a non-operator's
//! survey is a silent 404 — a survey attempt is not an erasure request, so no
//! `principal_erasure_refused` event is ever recorded.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_services::services::erasure_service::{
    execute_erasure, survey_erasure, ErasureOutcome, ErasureSurvey,
};
use temper_services::test_support;
use temper_workflow::operations::Surface;

// ── fixture helpers (duplicated per file, per this suite's convention) ──────────────────────

/// Minimal profile + its `<handle>@web` emitter entity. The handle is the FULL id:
/// two UUIDv7s minted in the same millisecond share leading bytes, so a truncated
/// handle collides on `kb_profiles_handle_key` (the slack fixture's rule).
async fn insert_profile(pool: &PgPool) -> (Uuid, String) {
    let id = Uuid::now_v7();
    let handle = format!("user-{id}");
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
                 VALUES ($1, $2, $2, $3, $4)",
    )
    .bind(id)
    .bind(&handle)
    .bind(format!("{handle}@x.test"))
    .bind(serde_json::json!({ "theme": "dark" }))
    .execute(pool)
    .await
    .expect("seed profile");
    sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("{handle}@web"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    (id, handle)
}

/// A DISTINCT, faithful 64-hex stand-in for a sha256 — the fixtures must look exactly
/// like what `content_hash` carries, because the payload's shape IS bare hex.
fn fake_sha(tag: char) -> String {
    let hex = format!("{}{}", Uuid::now_v7().simple(), Uuid::now_v7().simple());
    format!("{tag}{}", &hex[..63])
}

fn embedding_literal() -> String {
    format!("[{}]", vec!["0.1"; 768].join(","))
}

async fn emitter_of(pool: &PgPool, profile: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_entities e WHERE e.profile_id = $1 \
                        AND e.name LIKE '%@web'",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .expect("emitter entity")
}

/// A governed (personal) context with a watermark to lose, one resource homed there with
/// the full content stack (block revision + verbatim bytes + chunk + prose + FTS row).
struct ContentWorld {
    context: Uuid,
    resource: Uuid,
    chunk: Uuid,
    chunk_hash: String,
    block_hash: String,
}

async fn seed_content(pool: &PgPool, subject: Uuid) -> ContentWorld {
    let context = Uuid::now_v7();
    let resource = Uuid::now_v7();
    let block = Uuid::now_v7();
    let chunk = Uuid::now_v7();
    let revision = Uuid::now_v7();
    let chunk_hash = fake_sha('c');
    let block_hash = fake_sha('b');

    let event: Uuid = sqlx::query_scalar(
        "SELECT _event_append('resource_created', $1, 'kb_contexts', $2, \
                jsonb_build_object('resource_id', $3))",
    )
    .bind(emitter_of(pool, subject).await)
    .bind(context)
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("seed genesis event");

    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name, \
                 shape_materialized_event_id) \
                 VALUES ($1, 'kb_profiles', $2, 'notes', 'Notes', $3)",
    )
    .bind(context)
    .bind(subject)
    .bind(event)
    .execute(pool)
    .await
    .expect("seed personal context");
    sqlx::query("INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, 'Secret plans', 'test://secret')")
        .bind(resource)
        .execute(pool)
        .await
        .expect("seed resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, \
                 originator_profile_id, owner_profile_id) \
                 VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(resource)
    .bind(context)
    .bind(subject)
    .execute(pool)
    .await
    .expect("seed home");
    sqlx::query(
        "INSERT INTO kb_content_blocks (id, resource_id, seq, genesis_event_id, \
                 last_event_id) VALUES ($1, $2, 0, $3, $3)",
    )
    .bind(block)
    .bind(resource)
    .bind(event)
    .execute(pool)
    .await
    .expect("seed block");
    sqlx::query(
        "INSERT INTO kb_block_revisions (id, block_id, block_body_hash, chunk_count) \
                 VALUES ($1, $2, $3, 1)",
    )
    .bind(revision)
    .bind(block)
    .bind(&block_hash)
    .execute(pool)
    .await
    .expect("seed revision");
    sqlx::query(
        "INSERT INTO kb_block_content (block_revision_id, content, content_hash) \
                 VALUES ($1, $2, $3)",
    )
    .bind(revision)
    .bind("the secret plan bytes")
    .bind(&block_hash)
    .execute(pool)
    .await
    .expect("seed verbatim bytes");
    sqlx::query(
        "INSERT INTO kb_chunks (id, block_id, resource_id, chunk_index, version, \
                 content_hash, embedding, embedded_with) \
                 VALUES ($1, $2, $3, 0, 1, $4, $5::vector, 'model-sha')",
    )
    .bind(chunk)
    .bind(block)
    .bind(resource)
    .bind(&chunk_hash)
    .bind(embedding_literal())
    .execute(pool)
    .await
    .expect("seed chunk");
    sqlx::query(
        "INSERT INTO kb_chunk_content (chunk_id, content) VALUES ($1, 'the secret plan prose')",
    )
    .bind(chunk)
    .execute(pool)
    .await
    .expect("seed chunk prose");
    sqlx::query(
        "INSERT INTO kb_resource_search_index (resource_id, search_vector) \
                 VALUES ($1, to_tsvector('english', 'secret plans prose'))",
    )
    .bind(resource)
    .execute(pool)
    .await
    .expect("seed search index");

    ContentWorld {
        context,
        resource,
        chunk,
        chunk_hash,
        block_hash,
    }
}

/// A live blob row, seeded through the same event machinery the substrate writes with.
async fn seed_blob_with_hash(
    pool: &PgPool,
    owner: Uuid,
    home_table: &str,
    home_id: Uuid,
    tag: &str,
    hash: &str,
) -> (Uuid, String, String) {
    let blob = Uuid::now_v7();
    let pathname = format!("{tag}/{}", &hash[..16.min(hash.len())]);
    let event: Uuid = sqlx::query_scalar(
        "SELECT _event_append('blob_committed', $1, $2, $3, \
                jsonb_build_object('blob_id', $4))",
    )
    .bind(emitter_of(pool, owner).await)
    .bind(home_table)
    .bind(home_id)
    .bind(blob)
    .fetch_one(pool)
    .await
    .expect("seed blob event");
    sqlx::query(
        "INSERT INTO kb_blobs (id, content_hash, blob_pathname, content_type, \
                 content_bytes, home_table, home_id, owner_profile_id, originator_profile_id, \
                 asserted_by_event_id, last_event_id) \
                 VALUES ($1, $2, $3, 'image/png', 10, $4, $5, $6, $6, $7, $7)",
    )
    .bind(blob)
    .bind(hash)
    .bind(&pathname)
    .bind(home_table)
    .bind(home_id)
    .bind(owner)
    .bind(event)
    .execute(pool)
    .await
    .expect("seed blob row");
    (blob, hash.to_string(), pathname)
}

async fn seed_blob(
    pool: &PgPool,
    owner: Uuid,
    home_table: &str,
    home_id: Uuid,
    tag: &str,
) -> (Uuid, String, String) {
    let hash = fake_sha(tag.chars().next().expect("tag"));
    seed_blob_with_hash(pool, owner, home_table, home_id, tag, &hash).await
}

/// A live UNFOLDED edge from a blob to a resource — the guest naming pass's ATTACHED shape
/// (a live relation to an estate resource outlives the act).
async fn attach_blob_to_resource(
    pool: &PgPool,
    emitter: Uuid,
    blob: Uuid,
    resource: Uuid,
    context: Uuid,
) {
    let event: Uuid = sqlx::query_scalar(
        "SELECT _event_append('blob_committed', $1, 'kb_contexts', $2, \
                jsonb_build_object('blob_id', $3))",
    )
    .bind(emitter)
    .bind(context)
    .bind(blob)
    .fetch_one(pool)
    .await
    .expect("seed edge assertion event");
    sqlx::query(
        "INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, \
                 home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
                 VALUES ('kb_blobs', $1, 'kb_resources', $2, 'contains', 'kb_contexts', $3, $4, $4)",
    )
    .bind(blob)
    .bind(resource)
    .bind(context)
    .bind(event)
    .execute(pool)
    .await
    .expect("seed blob-resource edge");
}

/// A context OWNED by the subject's personal team (the trigger made the team at profile
/// insert) — team governance, disposition iii's not-governed home.
async fn seed_team_context(pool: &PgPool, handle: &str) -> Uuid {
    let context = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
                 SELECT $1, 'kb_teams', t.id, 'shared', 'Shared' \
                   FROM kb_teams t WHERE t.slug = 'personal-' || $2",
    )
    .bind(context)
    .bind(handle)
    .execute(pool)
    .await
    .expect("seed team context");
    context
}

/// A governed (personal) context owned by `owner`, bare — the second live home for a
/// shared hash. The slug must be distinct per owner (kb_contexts' owner+slug key).
async fn seed_bare_personal_context(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
    let context = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
                 VALUES ($1, 'kb_profiles', $2, $3, 'Theirs')",
    )
    .bind(context)
    .bind(owner)
    .bind(slug)
    .execute(pool)
    .await
    .expect("seed bare personal context");
    context
}

async fn event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .expect("event count")
}

/// (id, content_hash, pathname, type, bytes, home, owner, originator, asserted_by,
/// last_event, created), id order — the full blob row, for the leaves-no-trace snapshot.
type BlobRows = Vec<(
    Uuid,
    String,
    Option<String>,
    Option<String>,
    i64,
    String,
    Uuid,
    Uuid,
    Option<Uuid>,
    Uuid,
    chrono::DateTime<chrono::Utc>,
)>;

async fn blob_rows(pool: &PgPool) -> BlobRows {
    sqlx::query_as(
        "SELECT id, content_hash, blob_pathname, content_type, content_bytes, home_table, \
                owner_profile_id, originator_profile_id, asserted_by_event_id, last_event_id, \
                created \
           FROM kb_blobs ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("blob rows")
}

// ── the witnesses ────────────────────────────────────────────────────────────────────────────

/// THE DIFFERENTIAL: the survey's prediction IS the act's record, for every classification.
/// FAILS IF survey and act can diverge — same targets (exact prose, exact order), same
/// redacted set (ordered), same per-blob verdicts, same already_erased — including THE
/// VERDICT BITE: a hash with a second live row in another home must predict released=false,
/// and the act's strike-time refcount must agree; and THE SEQUENTIAL-REFCOUNT BITE: the SAME
/// subject holding the SAME hash in TWO governed homes they own must predict released=false
/// then released=true, in plan order, because the act strikes sequentially and each
/// blob_delete counts live rows at ITS moment.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_survey_matches_the_subsequent_act_over_every_classification(pool: PgPool) {
    let (subject, handle) = insert_profile(&pool).await;
    let (operator, _) = insert_profile(&pool).await;
    let (guest, _) = insert_profile(&pool).await;
    let (other, _) = insert_profile(&pool).await;
    test_support::grant_governance(&pool, operator).await;

    // Governed text content (the email/preferences arms come from insert_profile).
    let world = seed_content(&pool, subject).await;

    // A personal-homed blob whose hash is SOLE — the released=true prediction.
    let (sole_blob, sole_hash, _) =
        seed_blob(&pool, subject, "kb_contexts", world.context, "aa").await;

    // THE VERDICT BITE: a personal-homed blob whose hash has a SECOND LIVE ROW in another
    // home — the second row homed in ANOTHER principal's governed context (nobody's
    // target in this subject's act), so the strike must find the bytes held.
    let shared_hash = fake_sha('s');
    let (held_blob, _, _) = seed_blob_with_hash(
        &pool,
        subject,
        "kb_contexts",
        world.context,
        "ss",
        &shared_hash,
    )
    .await;
    let other_context = seed_bare_personal_context(&pool, other, "theirs").await;
    let (_, _, _) = seed_blob_with_hash(
        &pool,
        other,
        "kb_contexts",
        other_context,
        "ss",
        &shared_hash,
    )
    .await;

    // THE SEQUENTIAL-REFCOUNT BITE: the SAME subject holds the SAME content_hash in TWO
    // DIFFERENT governed contexts THEY own (the home-scope uniqueness key permits one row
    // per home-scope, so two homes). The act strikes the plan's would-strike rows IN PLAN
    // ORDER and each blob_delete counts live rows at ITS moment: strike 1 finds the sibling
    // still live (released=false), strike 2 finds it already emptied (released=true). A
    // survey that pre-counts with all rows live predicts false for BOTH and disagrees with
    // the act — the exact-equality differential below is what bites on a revert.
    let twin_hash = fake_sha('t');
    let twin_context_a = seed_bare_personal_context(&pool, subject, "twin-a").await;
    let twin_context_b = seed_bare_personal_context(&pool, subject, "twin-b").await;
    let (twin_a, _, _) = seed_blob_with_hash(
        &pool,
        subject,
        "kb_contexts",
        twin_context_a,
        "tt",
        &twin_hash,
    )
    .await;
    let (twin_b, _, _) = seed_blob_with_hash(
        &pool,
        subject,
        "kb_contexts",
        twin_context_b,
        "tt",
        &twin_hash,
    )
    .await;

    // A team-homed blob (the subject's own row, ungoverned home) — the named remainder.
    let team_context = seed_team_context(&pool, &handle).await;
    let (_, team_hash, _) = seed_blob(&pool, subject, "kb_contexts", team_context, "bb").await;

    // A guest-committed governed-home blob, ATTACHED — a live relation to an estate resource.
    let (guest_attached, guest_attached_hash, _) =
        seed_blob(&pool, guest, "kb_contexts", world.context, "dd").await;
    attach_blob_to_resource(
        &pool,
        emitter_of(&pool, guest).await,
        guest_attached,
        world.resource,
        world.context,
    )
    .await;

    // …and UNATTACHED — no relation, the home's owner is the erased subject.
    let (guest_unattached, guest_unattached_hash, _) =
        seed_blob(&pool, guest, "kb_contexts", world.context, "ee").await;

    // ── The survey, then the act: the prediction is the record. ───────────────────────────
    let survey = survey_erasure(&pool, ProfileId::from(operator), ProfileId::from(subject))
        .await
        .expect("the operator surveys");
    let outcome = execute_erasure(
        &pool,
        ProfileId::from(operator),
        ProfileId::from(subject),
        Uuid::now_v7(),
        Surface::ApiHttp,
    )
    .await
    .expect("the act completes");
    let ErasureOutcome::Completed(completion) = outcome else {
        panic!("the operator's act must complete");
    };

    // Exact equality: prose, order, hashes, verdicts, tombstone state.
    assert_eq!(
        serde_json::to_value(&survey.targets).unwrap(),
        serde_json::to_value(&completion.targets).unwrap(),
        "the survey's targets must equal the act's recorded targets, exactly"
    );
    assert_eq!(
        survey.redacted_hashes, completion.redacted_hashes,
        "the redacted set must agree, in order"
    );
    assert_eq!(survey.already_erased, completion.already_erased);
    assert!(!survey.already_erased);
    assert_eq!(
        survey.blob_strikes.len(),
        4,
        "four would-strike predictions: the sole hash, the held hash, and the subject's two \
         same-hash homes"
    );
    for predicted in &survey.blob_strikes {
        let actual = completion
            .blob_strikes
            .iter()
            .find(|s| s.blob_id == predicted.blob_id)
            .unwrap_or_else(|| panic!("no act verdict for {}", predicted.blob_id));
        assert_eq!(
            predicted.released, actual.released,
            "the verdict for {} must agree",
            predicted.blob_id
        );
    }
    // blob_strikes is in plan order, and the act consumes the plan in that order — so the
    // twins must come back false THEN true: strike 1 finds its sibling live, strike 2 finds
    // it emptied. This ordering is the bite: a pre-count predicts false for both.
    let twin_verdicts: Vec<bool> = survey
        .blob_strikes
        .iter()
        .filter(|s| s.blob_id == twin_a || s.blob_id == twin_b)
        .map(|s| s.released)
        .collect();
    assert_eq!(
        twin_verdicts,
        vec![false, true],
        "THE SEQUENTIAL-REFCOUNT BITE: the act's own order strikes the first twin while its \
         sibling is still live (released=false), then the second alone (released=true) — a \
         pre-count with all rows live would predict false for both and the differential \
         above would fail"
    );

    // The world covered every classification — the prediction names them all, so the
    // equality above is not vacuous.
    let outcome_of = |survey: &ErasureSurvey, blob: Uuid| {
        survey
            .targets
            .iter()
            .find(|t| t.outcome.contains(&blob.to_string()))
            .map(|t| t.outcome.clone())
            .unwrap_or_else(|| panic!("no target names {blob}"))
    };
    assert!(
        survey.targets.iter().any(|t| t.target == "kb_blobs"
            && t.outcome.contains("released=true")
            && t.outcome.contains("aa/")),
        "the sole-hash blob predicts a released strike: {:?}",
        survey.targets
    );
    let held_outcome = survey
        .targets
        .iter()
        .find(|t| t.target == "kb_blobs" && t.outcome.contains("ss/"))
        .expect("the held blob's prediction");
    assert!(
        held_outcome.outcome.contains("released=false"),
        "THE VERDICT BITE: the shared-hash blob must predict released=false, got {held_outcome:?}"
    );
    assert!(outcome_of(&survey, guest_attached).contains("guest of the erased principal"));
    let team_outcome = survey
        .targets
        .iter()
        .find(|t| t.outcome.contains(&team_hash))
        .expect("the remainder names the hash");
    assert!(
        team_outcome.outcome.starts_with("independent_obligation"),
        "the team-homed blob is the named remainder: {team_outcome:?}"
    );
    let attached_outcome = outcome_of(&survey, guest_attached);
    assert!(
        attached_outcome.contains("guest of the erased principal")
            && attached_outcome.contains("a live relation to an estate resource outlives the act"),
        "the attached guest blob names its provenance: {attached_outcome}"
    );
    let unattached_outcome = outcome_of(&survey, guest_unattached);
    assert!(
        unattached_outcome.contains("unattached, and the home's owner is the erased subject"),
        "the unattached guest blob names its provenance: {unattached_outcome}"
    );
    assert!(survey
        .targets
        .iter()
        .any(|t| t.target == "kb_profiles.email" && t.outcome == "erased"));
    assert!(survey
        .targets
        .iter()
        .any(|t| t.target == "kb_profiles.preferences" && t.outcome == "erased"));
    assert!(survey
        .targets
        .iter()
        .any(|t| t.target == "kb_contexts.is_active" && t.outcome == "retired"));
    assert!(survey.redacted_hashes.contains(&world.chunk_hash));
    assert!(survey.redacted_hashes.contains(&world.block_hash));
    assert!(survey.redacted_hashes.contains(&sole_hash));
    assert!(
        survey.redacted_hashes.contains(&shared_hash),
        "the held hash IS struck (its subject-owned row is a target); it is just not released"
    );
    assert!(
        survey.redacted_hashes.contains(&twin_hash),
        "the twin hash IS struck — both subject-owned rows are targets"
    );
    assert!(
        !survey.redacted_hashes.contains(&team_hash)
            && !survey.redacted_hashes.contains(&guest_attached_hash)
            && !survey.redacted_hashes.contains(&guest_unattached_hash),
        "named remainders never enter the redacted set"
    );

    // ── The second round: survey the erased subject, execute again — the survey predicts
    // the no-op completion's already-erased shape the same way. ────────────────────────────
    let survey2 = survey_erasure(&pool, ProfileId::from(operator), ProfileId::from(subject))
        .await
        .expect("the second survey");
    let outcome2 = execute_erasure(
        &pool,
        ProfileId::from(operator),
        ProfileId::from(subject),
        Uuid::now_v7(),
        Surface::ApiHttp,
    )
    .await
    .expect("the re-erase completes");
    let ErasureOutcome::Completed(completion2) = outcome2 else {
        panic!("a re-erase is a completion, never a refusal");
    };
    assert!(
        survey2.already_erased,
        "the second survey predicts the tombstone"
    );
    assert_eq!(
        survey2.already_erased, completion2.already_erased,
        "the second prediction must agree with the second act"
    );
    assert_eq!(
        serde_json::to_value(&survey2.targets).unwrap(),
        serde_json::to_value(&completion2.targets).unwrap(),
        "the second survey's targets must equal the second act's record, exactly"
    );
    assert!(
        survey2.blob_strikes.is_empty() && completion2.blob_strikes.is_empty(),
        "struck rows are never re-struck: nothing to predict, nothing to fire"
    );
    let _ = (&sole_blob, &held_blob, &world.chunk);
}

/// LEAVES-NO-TRACE: FAILS IF the survey writes ANYTHING — not a ledger row of any kind, not
/// a blob row, not the erased-content set, not the profile, not the chunk prose, vector,
/// search vector, the verbatim block bytes or context liveness. It reads, it never writes.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_survey_pass_leaves_no_trace(pool: PgPool) {
    let (subject, handle) = insert_profile(&pool).await;
    let (operator, _) = insert_profile(&pool).await;
    test_support::grant_governance(&pool, operator).await;
    let world = seed_content(&pool, subject).await;
    let (blob, hash, _) = seed_blob(&pool, subject, "kb_contexts", world.context, "aa").await;
    let team_context = seed_team_context(&pool, &handle).await;

    // The snapshot: everything a survey could conceivably touch.
    let events_before = event_count(&pool).await;
    let blobs_before = blob_rows(&pool).await;
    let erased_before: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_erased_content")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (prof_handle, prof_email, tomb_before): (String, Option<String>, Option<i32>) =
        sqlx::query_as(
            "SELECT handle, email, (tombstoned_at IS NOT NULL)::int FROM kb_profiles WHERE id = $1",
        )
        .bind(subject)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (prose_before, emb_before, prov_before): (String, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT cc.content, c.embedding::text, c.embedded_with FROM kb_chunk_content cc \
               JOIN kb_chunks c ON c.id = cc.chunk_id WHERE cc.chunk_id = $1",
        )
        .bind(world.chunk)
        .fetch_one(&pool)
        .await
        .unwrap();
    let fts_before: String = sqlx::query_scalar(
        "SELECT search_vector::text FROM kb_resource_search_index WHERE resource_id = $1",
    )
    .bind(world.resource)
    .fetch_one(&pool)
    .await
    .unwrap();
    let block_bytes_before: String =
        sqlx::query_scalar("SELECT content FROM kb_block_content WHERE content_hash = $1")
            .bind(&world.block_hash)
            .fetch_one(&pool)
            .await
            .unwrap();
    let (ctx_before, team_ctx_before): (bool, bool) = sqlx::query_as(
        "SELECT is_active, \
                (SELECT is_active FROM kb_contexts WHERE id = $2) \
           FROM kb_contexts WHERE id = $1",
    )
    .bind(world.context)
    .bind(team_context)
    .fetch_one(&pool)
    .await
    .unwrap();

    survey_erasure(&pool, ProfileId::from(operator), ProfileId::from(subject))
        .await
        .expect("the survey answers");

    assert_eq!(
        event_count(&pool).await,
        events_before,
        "a survey fires no event of ANY kind — it reads, it never writes"
    );
    assert_eq!(
        blob_rows(&pool).await,
        blobs_before,
        "no kb_blobs row moves — the survey must not strike"
    );
    let erased_after: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_erased_content")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        erased_after, erased_before,
        "the erased-content set admits nothing"
    );
    let (prof_handle_after, prof_email_after, tomb_after): (String, Option<String>, Option<i32>) =
        sqlx::query_as(
            "SELECT handle, email, (tombstoned_at IS NOT NULL)::int FROM kb_profiles WHERE id = $1",
        )
        .bind(subject)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        (prof_handle_after, prof_email_after, tomb_after),
        (prof_handle, prof_email, tomb_before),
        "the profile is untouched — no pseudonym break on a survey"
    );
    let (prose_after, emb_after, prov_after): (String, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT cc.content, c.embedding::text, c.embedded_with FROM kb_chunk_content cc \
               JOIN kb_chunks c ON c.id = cc.chunk_id WHERE cc.chunk_id = $1",
        )
        .bind(world.chunk)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        (prose_after, emb_after, prov_after),
        (prose_before, emb_before, prov_before),
        "the chunk prose, vector and provenance are untouched"
    );
    let fts_after: String = sqlx::query_scalar(
        "SELECT search_vector::text FROM kb_resource_search_index WHERE resource_id = $1",
    )
    .bind(world.resource)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(fts_after, fts_before, "the search vector is untouched");
    let block_bytes_after: String =
        sqlx::query_scalar("SELECT content FROM kb_block_content WHERE content_hash = $1")
            .bind(&world.block_hash)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        block_bytes_after, block_bytes_before,
        "the verbatim block bytes are untouched — kb_block_content.content is a projection \
         the redaction empties, so a survey must leave it alone"
    );
    let (ctx_after, team_ctx_after): (bool, bool) = sqlx::query_as(
        "SELECT is_active, \
                (SELECT is_active FROM kb_contexts WHERE id = $2) \
           FROM kb_contexts WHERE id = $1",
    )
    .bind(world.context)
    .bind(team_context)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (ctx_after, team_ctx_after),
        (ctx_before, team_ctx_before),
        "no context is retired by a survey"
    );
    let _ = (blob, hash);
}

/// THE GATE: FAILS IF a non-operator's survey mutates or discloses — the answer is the same
/// NotFound the execute door renders, and ZERO new events of any kind (the ruled
/// no-event face: this is the witness that bites if someone "fixes" the survey to record a
/// refusal — a survey attempt is not an erasure request, ruled 2026-09-12). The bite probe:
/// the SAME caller with the gate granted gets the survey.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_non_operator_survey_records_nothing_until_the_gate_stands_down(pool: PgPool) {
    let (subject, _) = insert_profile(&pool).await;
    let (caller, _) = insert_profile(&pool).await;
    let world = seed_content(&pool, subject).await;
    let _ = seed_blob(&pool, subject, "kb_contexts", world.context, "aa").await;

    let events_before = event_count(&pool).await;

    let answer = survey_erasure(&pool, ProfileId::from(caller), ProfileId::from(subject)).await;
    assert!(
        matches!(answer, Err(temper_services::error::ApiError::NotFound(_))),
        "a non-operator gets the silent 404 face, got {answer:?}"
    );

    // THE GATE-BEFORE-EXISTENCE ORDER, witnessed by the MESSAGE: a non-operator surveying a
    // subject that does NOT exist still gets the gate's face — EXACTLY "not found", never
    // "profile not found". `matches!` above cannot see the message; this can, and it bites
    // if anyone moves the existence check above the gate, which would leak that semantics to
    // a caller the gate has already declined.
    let ghost = Uuid::now_v7();
    let ghost_answer = survey_erasure(&pool, ProfileId::from(caller), ProfileId::from(ghost)).await;
    match ghost_answer {
        Err(temper_services::error::ApiError::NotFound(message)) => assert_eq!(
            message, "not found",
            "the gate's silent face is EXACTLY \"not found\" — \"profile not found\" would \
             betray an existence check running above the gate"
        ),
        other => panic!("a non-operator gets the silent 404 face, got {other:?}"),
    }

    assert_eq!(
        event_count(&pool).await,
        events_before,
        "a refused survey records NOTHING — no principal_erasure_refused, no event of any kind"
    );

    // THE BITE: the same caller, the gate granted, the same call — the survey answers.
    test_support::grant_governance(&pool, caller).await;
    let survey = survey_erasure(&pool, ProfileId::from(caller), ProfileId::from(subject))
        .await
        .expect("with the gate stood down the same call surveys");
    assert_eq!(survey.subject, ProfileId::from(subject));
}
