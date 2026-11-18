use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use bytes::Bytes;
use sqlx::PgPool;
use temper_substrate::blob_store::{blob_pathname, InMemoryBlobStore};
use temper_substrate::ids::{BlobId, ContextId, EntityId, ProfileId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::writes::{self, CommitBlobParams};
use temper_workflow::operations::Surface;
use uuid::Uuid;

use super::*;
use crate::services::erasure_service;
use crate::test_support;

/// A profile + its `<handle>@web` emitter entity (the erasure_service fixture shape).
async fn insert_profile(pool: &PgPool) -> Uuid {
    let id = Uuid::now_v7();
    let handle = format!("user-{id}");
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
                 VALUES ($1, $2, $2, $3, '{}'::jsonb)",
    )
    .bind(id)
    .bind(&handle)
    .bind(format!("{handle}@x.test"))
    .execute(pool)
    .await
    .expect("seed profile");
    sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("{handle}@web"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    id
}

/// A governed (personal) context owned by `subject` — the act's governed home.
async fn insert_personal_context(pool: &PgPool, subject: Uuid) -> Uuid {
    crate::test_support::context::insert_personal(pool, subject, "notes", "Notes").await
}

async fn emitter_of(pool: &PgPool, profile: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id = $1 AND name LIKE '%@web'")
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("emitter entity")
}

/// A live blob row through the REAL commit path, with the bytes pre-put in `store`.
async fn seed_blob(
    pool: &PgPool,
    store: &InMemoryBlobStore,
    home: Uuid,
    subject: Uuid,
    bytes: &[u8],
) -> (Uuid, String, String) {
    use sha2::Digest;
    let hash = hex::encode(sha2::Sha256::digest(bytes));
    let pathname = blob_pathname(&hash);
    store.insert(&pathname);
    let blob = writes::commit_blob(
        pool,
        store,
        CommitBlobParams {
            id: BlobId::from(Uuid::now_v7()),
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(subject),
            originator: None,
            content_hash: hash.clone(),
            content_type: "image/png".to_string(),
            content_bytes: bytes.len() as i64,
            max_bytes: 10 * 1024 * 1024,
            allowlist: &["image/png".to_string()],
            emitter: EntityId::from(emitter_of(pool, subject).await),
        },
    )
    .await
    .expect("commit blob");
    (blob.uuid(), hash, pathname)
}

/// Erase `subject` as `operator` — the real act, real payload, real strikes. The operator's
/// proof is minted through the real gate, so `operator` must already hold governance.
async fn erase(pool: &PgPool, operator: Uuid, subject: Uuid) -> erasure_service::ErasureCompletion {
    let admin = test_support::system_admin_proof_for(pool, operator).await;
    erasure_service::execute_erasure(
        pool,
        &admin,
        ProfileId::from(subject),
        Uuid::now_v7(),
        Surface::ApiHttp,
        &Default::default(),
    )
    .await
    .expect("the act completes")
}

/// A store that can fail on demand and RECORDS every delete batch — the witness
/// instrument for both the retry ladder and the batching assertion.
#[derive(Default)]
struct FenceStore {
    inner: InMemoryBlobStore,
    failing: AtomicBool,
    delete_calls: Mutex<Vec<Vec<String>>>,
}

impl FenceStore {
    fn holds(&self, pathname: &str) -> bool {
        self.inner.contains(pathname)
    }

    fn delete_batches(&self) -> Vec<Vec<String>> {
        self.delete_calls.lock().unwrap().clone()
    }
}

impl std::fmt::Debug for FenceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FenceStore").finish()
    }
}

#[async_trait::async_trait]
impl BlobStore for FenceStore {
    async fn exists(&self, pathname: &str) -> anyhow::Result<bool> {
        self.inner.exists(pathname).await
    }
    async fn put(
        &self,
        pathname: &str,
        content_type: &str,
        body: Bytes,
        cache_control_max_age: u32,
    ) -> anyhow::Result<temper_substrate::blob_store::PutReceipt> {
        self.inner
            .put(pathname, content_type, body, cache_control_max_age)
            .await
    }
    async fn get(
        &self,
        pathname: &str,
        consistent: bool,
    ) -> anyhow::Result<temper_substrate::blob_store::ByteStream> {
        self.inner.get(pathname, consistent).await
    }
    async fn head(
        &self,
        pathname: &str,
    ) -> anyhow::Result<Option<temper_substrate::blob_store::BlobHead>> {
        self.inner.head(pathname).await
    }
    async fn delete(&self, pathnames: &[&str]) -> anyhow::Result<()> {
        if self.failing.load(Ordering::SeqCst) {
            anyhow::bail!("provider unavailable (the fence witness's outage)");
        }
        self.delete_calls
            .lock()
            .unwrap()
            .push(pathnames.iter().map(|s| s.to_string()).collect());
        self.inner.delete(pathnames).await
    }
}

/// The world: a subject with a governed context and two blobs (so one act yields TWO
/// strikes), erased by an operator. Returns `(subject, home, subject_emitter, strikes)`
/// with the strikes as `(blob, hash, pathname)`. The emitter is captured BEFORE the act:
/// erasure sentinels the subject's entity names (20260909000025), so a post-erasure
/// re-commit must attribute by the captured id — looking the entity up by its
/// personal-name pattern afterwards is the lookup the act exists to break.
async fn erased_world(pool: &PgPool) -> (Uuid, Uuid, Uuid, Vec<(Uuid, String, String)>) {
    let subject = insert_profile(pool).await;
    let operator = insert_profile(pool).await;
    test_support::grant_governance(pool, operator).await;
    let home = insert_personal_context(pool, subject).await;
    let subject_emitter = emitter_of(pool, subject).await;
    let fixture_store = InMemoryBlobStore::default();
    let first = seed_blob(pool, &fixture_store, home, subject, b"\x89PNG-first").await;
    let second = seed_blob(pool, &fixture_store, home, subject, b"\x89PNG-second").await;
    erase(pool, operator, subject).await;
    (subject, home, subject_emitter, vec![first, second])
}

/// A FenceStore pre-put with the world's pathnames — the provider as the act left it.
fn fence_store_for(strikes: &[(Uuid, String, String)]) -> FenceStore {
    let store = FenceStore::default();
    for (_, _, pathname) in strikes {
        store.inner.insert(pathname.clone());
    }
    store
}

/// Push every fence row's retry schedule back to "due now" — the timing elapse the
/// workflow_job tests use, applied to the fence's own column.
async fn elapse_backoff(pool: &PgPool) {
    sqlx::query("UPDATE kb_erasure_blob_deletes SET next_attempt_at = now()")
        .execute(pool)
        .await
        .expect("elapse backoff");
}

async fn row_of(pool: &PgPool, pathname: &str) -> (String, i32, Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT status, attempts, resolution, last_error \
           FROM kb_erasure_blob_deletes WHERE pathname = $1",
    )
    .bind(pathname)
    .fetch_one(pool)
    .await
    .expect("the fence row")
}

// ── WITNESS: the retry ladder ────────────────────────────────────────────────────────
/// FAILS IF a failing provider tick is not retried with recorded state: the first tick must
/// leave the rows `waiting_for_retry` with attempts incremented and the error recorded, and
/// the tick after the store recovers must delete and record success.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_failing_store_is_retried_and_the_success_is_recorded(pool: sqlx::PgPool) {
    let (_, _, _, strikes) = erased_world(&pool).await;
    let store = fence_store_for(&strikes);
    store.failing.store(true, Ordering::SeqCst);

    let summary = drain(&pool, &store).await.expect("tick runs");
    assert_eq!(summary.seeded, 2, "both strikes derived from the payload");
    assert_eq!(summary.claimed, 2);
    assert_eq!(summary.failed, 2, "the failed batch is handed back");

    for (_, _, pathname) in &strikes {
        let (status, attempts, resolution, last_error) = row_of(&pool, pathname).await;
        assert_eq!(status, "waiting_for_retry", "{pathname}");
        assert_eq!(attempts, 1, "{pathname}: attempts incremented at claim");
        assert_eq!(resolution, None);
        assert!(
            last_error
                .as_deref()
                .is_some_and(|e| e.contains("provider unavailable")),
            "{pathname}: the failure is recorded, got {last_error:?}"
        );
        let deferred: bool = sqlx::query_scalar(
            "SELECT next_attempt_at > now() FROM kb_erasure_blob_deletes WHERE pathname = $1",
        )
        .bind(pathname)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            deferred,
            "{pathname}: the retry is backed off, not immediate"
        );
        assert!(
            store.holds(pathname),
            "{pathname}: the bytes survive a failed tick"
        );
    }

    // The store recovers; the backoff elapses; the same fence pays the debt.
    store.failing.store(false, Ordering::SeqCst);
    elapse_backoff(&pool).await;
    let summary = drain(&pool, &store).await.expect("second tick runs");
    assert_eq!(summary.deleted, 2);
    assert_eq!(
        summary.seeded, 0,
        "the re-derivation re-seeds nothing — the (event, pathname) keys exist"
    );

    for (_, _, pathname) in &strikes {
        let (status, attempts, resolution, _) = row_of(&pool, pathname).await;
        assert_eq!(status, "done", "{pathname}");
        assert_eq!(attempts, 2, "{pathname}: the retry's attempt is counted");
        assert_eq!(
            resolution.as_deref(),
            Some(RESOLUTION_DELETED),
            "{pathname}"
        );
        assert!(
            !store.holds(pathname),
            "{pathname}: the bytes are gone once the store recovers"
        );
    }
    assert_eq!(
        store.delete_batches().len(),
        1,
        "the recovering tick issues exactly one delete call"
    );
}

// ── WITNESS: the age alert ───────────────────────────────────────────────────────────
/// FAILS IF the age bound does not separate behind from alertable: a fresh pending delete is
/// `transient`; the same delete past [`ALERTABLE_AFTER_SECONDS`] is `sustained` with the age
/// as its cause; a never-exercised fence is `no_attempt_recorded`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_delete_pending_past_the_age_bound_is_the_alertable_state(pool: sqlx::PgPool) {
    let now = Utc::now();

    // A fence with no history is reported, never omitted — and never alertable.
    let quiet = fence_channel_report(&pool, now).await.expect("report runs");
    assert_eq!(quiet.channel, ERASURE_FENCE_CHANNEL);
    assert_eq!(quiet.state, ChannelState::NoAttemptRecorded);
    assert_eq!(quiet.seconds_since_success, None, "absent, not zero");
    assert_eq!(quiet.failure_cause, None);

    let (_, _, _, strikes) = erased_world(&pool).await;
    let store = fence_store_for(&strikes);
    store.failing.store(true, Ordering::SeqCst);
    drain(&pool, &store).await.expect("tick runs");

    // Fresh debt: outstanding, not yet alertable.
    let fresh = fence_channel_report(&pool, now).await.expect("report runs");
    assert_eq!(fresh.state, ChannelState::Transient);
    assert!(fresh.failure_cause.is_none(), "not alertable, no cause");
    assert!(
        fresh
            .failing_for_seconds
            .is_some_and(|s| s < ALERTABLE_AFTER_SECONDS),
        "the age of the oldest pending delete is the field the rule reads, got {:?}",
        fresh.failing_for_seconds
    );

    // The SAME delete, aged past the bound: sustained, with the cause that says why.
    sqlx::query(
        "UPDATE kb_erasure_blob_deletes \
            SET first_due_at = first_due_at - make_interval(secs => $1)",
    )
    .bind(ALERTABLE_AFTER_SECONDS + 60)
    .execute(&pool)
    .await
    .expect("age the deletes");
    let aged = fence_channel_report(&pool, now).await.expect("report runs");
    assert_eq!(aged.state, ChannelState::Sustained);
    assert_eq!(
        aged.failure_cause.as_deref(),
        Some("pending_past_age"),
        "the cause names the age, not a generic failure"
    );
    assert!(
        aged.failure_detail
            .as_deref()
            .is_some_and(|d| d.contains("outstanding") && d.contains("86400")),
        "the detail names the delete, got {:?}",
        aged.failure_detail
    );
}

// ── WITNESS: retries exhausted is alertable too ──────────────────────────────────────
/// FAILS IF a delete that burned its attempts goes quiet: a `dead` row must read
/// `sustained` with `retries_exhausted`, not healthy silence — the fence's contract is
/// alert, never silent-forever.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_dead_delete_stays_alertable(pool: sqlx::PgPool) {
    let (_, _, _, strikes) = erased_world(&pool).await;
    let store = fence_store_for(&strikes);
    store.failing.store(true, Ordering::SeqCst);

    // Burn the ladder: five claims, each handed back through the fail path.
    for _ in 0..5 {
        elapse_backoff(&pool).await;
        drain(&pool, &store).await.expect("tick runs");
    }
    for (_, _, pathname) in &strikes {
        let (status, attempts, _, _) = row_of(&pool, pathname).await;
        assert_eq!(status, "dead", "{pathname}");
        assert_eq!(attempts, 5, "{pathname}: max_attempts burned");
        assert!(
            store.holds(pathname),
            "{pathname}: the bytes were never struck"
        );
    }

    let report = fence_channel_report(&pool, Utc::now())
        .await
        .expect("report runs");
    assert_eq!(report.state, ChannelState::Sustained);
    assert_eq!(
        report.failure_cause.as_deref(),
        Some("retries_exhausted"),
        "the dead arm names its own cause"
    );
}

// ── WITNESS: the re-occupied hash ────────────────────────────────────────────────────
/// FAILS IF the drain strikes bytes a live row re-holds: after a re-commit of identical
/// bytes (the declared-open window healing itself), the drain must SKIP that pathname —
/// recorded as `skipped-reoccupied`, bytes still at the provider — while an unrestored
/// sibling is deleted.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_re_occupied_hash_is_not_deleted_and_an_unrestored_one_is(pool: sqlx::PgPool) {
    let (subject, home, subject_emitter, strikes) = erased_world(&pool).await;
    let (restored_blob, restored_hash, restored_path) = &strikes[0];
    let (_, _, untouched_path) = &strikes[1];

    // The healing re-commit: identical bytes re-put, a FRESH live row minted
    // (20260906000010's re-commit rule — never deduplicated against the struck row).
    let fixture_store = InMemoryBlobStore::default();
    fixture_store.insert(restored_path);
    let recommitted: Uuid = writes::commit_blob(
        &pool,
        &fixture_store,
        CommitBlobParams {
            id: BlobId::from(Uuid::now_v7()),
            home: AnchorRef::context(ContextId::from(home)),
            owner: ProfileId::from(subject),
            originator: None,
            content_hash: restored_hash.clone(),
            content_type: "image/png".to_string(),
            content_bytes: 16,
            max_bytes: 10 * 1024 * 1024,
            allowlist: &["image/png".to_string()],
            emitter: EntityId::from(subject_emitter),
        },
    )
    .await
    .expect("the re-commit is lawful")
    .uuid();
    let _ = (restored_blob, recommitted);

    let (live_rows,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM kb_blobs WHERE content_hash = $1 AND content_type IS NOT NULL",
    )
    .bind(restored_hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(live_rows, 1, "the re-commit mints a live row for the hash");

    let store = fence_store_for(&strikes);
    let summary = drain(&pool, &store).await.expect("tick runs");
    assert_eq!(
        summary.skipped_reoccupied, 1,
        "the re-occupied strike skips"
    );
    assert_eq!(summary.deleted, 1, "the unrestored strike deletes");

    let (status, _, resolution, _) = row_of(&pool, restored_path).await;
    assert_eq!(status, "done");
    assert_eq!(
        resolution.as_deref(),
        Some(SKIPPED_REOCCUPIED),
        "the skip is recorded, not silent"
    );
    assert!(
        store.holds(restored_path),
        "the re-put bytes are NOT struck — a live row owns them now"
    );
    assert!(
        !store.holds(untouched_path),
        "the unrestored bytes ARE struck"
    );
}

// ── WITNESS: batching ────────────────────────────────────────────────────────────────
/// FAILS IF the drain loops per pathname: one act striking two blobs must produce exactly
/// ONE `BlobStore::delete` call carrying BOTH pathnames — the verb is array-shaped and the
/// per-pathname loop is the shape the ruled verb exists to prevent.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_drain_deletes_through_one_batched_call(pool: sqlx::PgPool) {
    let (_, _, _, strikes) = erased_world(&pool).await;
    let store = fence_store_for(&strikes);

    let summary = drain(&pool, &store).await.expect("tick runs");
    assert_eq!(summary.deleted, 2);

    let batches = store.delete_batches();
    assert_eq!(batches.len(), 1, "one delete call, not a per-pathname loop");
    let mut batch = batches[0].clone();
    batch.sort();
    let mut expected: Vec<String> = strikes.iter().map(|(_, _, p)| p.clone()).collect();
    expected.sort();
    assert_eq!(batch, expected, "the batch carries BOTH pathnames");
}

// ── WITNESS: paid work stays paid ────────────────────────────────────────────────────
/// FAILS IF re-deriving from the ledger re-arms completed work: a second tick over a fully
/// drained fence seeds nothing, claims nothing, and calls the provider not at all.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn re_deriving_the_ledger_never_re_arms_paid_work(pool: sqlx::PgPool) {
    let (_, _, _, strikes) = erased_world(&pool).await;
    let store = fence_store_for(&strikes);
    drain(&pool, &store).await.expect("first tick");
    let batches_after_first = store.delete_batches().len();
    drain(&pool, &store).await.expect("second tick");

    let summary = drain(&pool, &store).await.expect("third tick");
    assert_eq!(summary.seeded, 0, "the (event, pathname) keys exist");
    assert_eq!(summary.claimed, 0, "nothing is pending");
    assert_eq!(
        store.delete_batches().len(),
        batches_after_first,
        "a quiet fence never touches the provider again"
    );
}

// ── WITNESS: seeding is store-independent ────────────────────────────────────────────
/// FAILS IF a deployment without a provider config never seeds: the store-less tick must
/// still derive the released strikes into `kb_erasure_blob_deletes` (first-due at the
/// EVENT's occurred_at, nothing claimed, no provider call — there is no store to call),
/// and the stranded debt must still age into the alertable `sustained` state. This is the
/// substrate contract's stranding posture: retry plus age alerting has no
/// store-configured precondition.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_store_less_deployment_still_seeds_and_ages_alertable(pool: sqlx::PgPool) {
    let (_, _, _, strikes) = erased_world(&pool).await;

    let summary = drain_without_store(&pool)
        .await
        .expect("store-less tick runs");
    assert_eq!(
        summary.seeded, 2,
        "the released strikes seed WITHOUT any provider config"
    );
    for (_, _, pathname) in &strikes {
        let (status, attempts, resolution, first_due, occurred): (
            String,
            i32,
            Option<String>,
            DateTime<Utc>,
            DateTime<Utc>,
        ) = sqlx::query_as(
            "SELECT d.status, d.attempts, d.resolution, d.first_due_at, e.occurred_at \
               FROM kb_erasure_blob_deletes d \
               JOIN kb_events e ON e.id = d.erasure_event_id \
              WHERE d.pathname = $1",
        )
        .bind(pathname)
        .fetch_one(&pool)
        .await
        .expect("the seeded fence row");
        assert_eq!(status, "pending", "{pathname}");
        assert_eq!(attempts, 0, "{pathname}: nothing was claimed");
        assert_eq!(resolution, None, "{pathname}: no provider call was made");
        assert_eq!(
            first_due, occurred,
            "{pathname}: first-due is the EVENT's occurred_at, so the age is real debt"
        );
    }

    // The stranded deletes age durably — and the existing Sustained alert fires, which is
    // the correct answer for bytes gone from the server but still held at a provider this
    // deployment can no longer name.
    sqlx::query(
        "UPDATE kb_erasure_blob_deletes \
            SET first_due_at = first_due_at - make_interval(secs => $1)",
    )
    .bind(ALERTABLE_AFTER_SECONDS + 60)
    .execute(&pool)
    .await
    .expect("age the deletes");
    let report = fence_channel_report(&pool, Utc::now())
        .await
        .expect("report runs");
    assert_eq!(
        report.state,
        ChannelState::Sustained,
        "config-removal strands the deletes into the alertable state, never silence"
    );
    assert_eq!(report.failure_cause.as_deref(), Some("pending_past_age"));
}

// ── WITNESS: the prose interface fails loud ──────────────────────────────────────────
/// FAILS IF the strike-outcome template can drift from what the fence parses: the pin
/// asserts the SQL literal composes to exactly the Rust constants — the released prefix
/// the seed parses through, the held prefix it skips, and the non-strike shapes — in
/// the minting migration (20260909000025, retired), the retired guest-naming carrier
/// (20260911000010 — its outcome prefixes stay pinned for records already in the
/// ledger), and the LIVE carrier: `blob_strike_outcome_text` (20260913000010), which
/// the home-pure plan (20260913000020) calls rather than inlining the prose. Every
/// `kb_blobs` outcome the act can emit must start with a prefix this file pins.
/// (Mirrors `payload_schema::the_migration_literal_matches_the_committed_fixture`.)
#[test]
fn the_migration_strike_template_matches_the_pinned_constants() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/20260909000025_erasure_act_execution.sql"
    );
    let migration = std::fs::read_to_string(path).expect("the execution migration exists");
    // The template, spelled in the migration as literal || v_rel::text || literal || v_path.
    let template = "'erased; released=' || v_rel::text || '; pathname=' || v_path";
    assert!(
        migration.contains(template),
        "the strike-outcome template drifted in the SQL — the fence parses by exact \
         prefix, so the Rust constants and this literal MUST move together"
    );
    // The migration's template asserts the two SQL literal halves; the constants are those
    // halves with the verdict spliced in — spelled out here so a drift on EITHER side
    // fails this pin rather than silently changing what the fence parses.
    assert_eq!(
        RELEASED_STRIKE_PREFIX, "erased; released=true; pathname=",
        "the released prefix is the template at v_rel = true"
    );
    assert_eq!(
        HELD_STRIKE_PREFIX, "erased; released=false; pathname=",
        "the held prefix is the template at v_rel = false"
    );
    // The other two known kb_blobs shapes the migration spells.
    assert!(
        migration.contains(&format!("'outcome', '{ALREADY_ERASED_OUTCOME}'")),
        "the already-erased kb_blobs shape is pinned"
    );
    assert!(
        migration.contains(&format!("'outcome', '{OBLIGATION_OUTCOME_PREFIX}")),
        "the independent_obligation remainder shape is pinned"
    );

    // The retired guest-naming carrier (20260911000010) minted the independent_obligation
    // guest prefix; the ledger carries such records, so the prefix stays pinned HERE even
    // though the body is retired. A future rewrite that rewords it away from a pinned
    // shape fails HERE, not in a fence drain.
    let carrier_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/20260911000010_blob_arm_names_guest_rows.sql"
    );
    let carrier = std::fs::read_to_string(carrier_path).expect("the live carrier migration exists");
    assert!(
        carrier.contains(template),
        "the strike-outcome template drifted in the retired carrier — the fence parses \
         by exact prefix, so the Rust constants and this literal MUST move together"
    );
    assert!(
        carrier.contains(&format!(
            "'outcome', '{OBLIGATION_OUTCOME_PREFIX}committed by a guest of the erased principal"
        )),
        "the guest-naming outcomes must carry the independent_obligation prefix the \
         fence classifies as a known non-delete-target"
    );

    // The LIVE carrier of the template: `blob_strike_outcome_text` (minted 20260913000010)
    // — the home-pure plan (20260913000020) calls it rather than inlining the prose, so
    // this body's spelling is what every fresh payload the fence parses arrives in.
    let live = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/20260913000010_erasure_survey_door.sql"
    ))
    .expect("the survey-door migration exists");
    assert!(
        live.contains("'erased; released=' || p_released::text || '; pathname=' || p_pathname"),
        "blob_strike_outcome_text's template drifted — the fence parses by exact prefix, \
         so the Rust constants and this literal MUST move together"
    );
}

/// FAILS IF an unparseable `kb_blobs` verdict is silently skipped: a seeded target whose
/// outcome matches no known shape is COUNTED in the seed summary and makes the report
/// alertable (`sustained`, cause `unparseable_verdicts`) — prose drift must never no-op
/// the fence into healthy silence — while a lawful sibling verdict still seeds.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_unparseable_blob_verdict_is_counted_and_alertable(pool: sqlx::PgPool) {
    let (_, _, _, strikes) = erased_world(&pool).await;
    assert_eq!(
        strikes.len(),
        2,
        "precondition: two lawful released verdicts"
    );

    // A SECOND erasure event whose kb_blobs verdict drifted — a future template the Rust
    // pins never learned. The ledger is append-only, so the drifted event is appended via
    // the same `_event_append` the act uses (NULL-anchored admin shape), never an UPDATE.
    let drifted: Uuid = sqlx::query_scalar(
        "SELECT _event_append('principal_erased', \
            (SELECT e.id FROM kb_entities e LIMIT 1), NULL, NULL, \
            jsonb_build_object('targets', jsonb_build_array(jsonb_build_object( \
                'target', 'kb_blobs', \
                'outcome', 'erased; released=YES; pathname=aa/bb'))))",
    )
    .fetch_one(&pool)
    .await
    .expect("append the drifted erasure event");
    let _: Uuid = drifted;

    let summary = drain_without_store(&pool).await.expect("tick runs");
    assert_eq!(
        summary.unparseable_verdicts, 1,
        "the drifted verdict is counted, not silently skipped"
    );
    assert_eq!(
        summary.seeded, 2,
        "the lawful siblings still seed — drift refuses only itself"
    );

    let report = fence_channel_report(&pool, Utc::now())
        .await
        .expect("report runs");
    assert_eq!(
        report.state,
        ChannelState::Sustained,
        "prose drift is alertable, never a healthy no-op"
    );
    assert_eq!(
        report.failure_cause.as_deref(),
        Some("unparseable_verdicts"),
        "the cause names the drift, not a row state"
    );
    assert!(
        report
            .failure_detail
            .as_deref()
            .is_some_and(|d| d.contains("match no known strike-outcome shape")),
        "the detail says what drifted, got {:?}",
        report.failure_detail
    );
}
