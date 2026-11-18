use super::*;
use sqlx::PgPool;
use uuid::Uuid;

// Survivor-style assertions throughout, per the AS reaper suite's own note: they name
// the rows they mean and cannot be moved by a row the test did not create.

/// A staged session `updated_age_seconds` old by `updated` and
/// `created_age_seconds` old by `created` (the two are independent — the
/// fresh-but-old witness passes them unequal on purpose), with `segment_count`
/// segments of `segment_bytes` bytes each. Stamped the way the real path stamps:
/// `created` at begin, `updated` advancing on every landed append — here expressed
/// directly, since the witnesses are about what the sweep reads, not about the append
/// path's own UPDATE (witnessed in substrate).
async fn seed_session(
    pool: &PgPool,
    id: &Uuid,
    updated_age_seconds: i64,
    created_age_seconds: i64,
    segment_count: i32,
    segment_bytes: i64,
) {
    let owner = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, 'Probe')")
        .bind(owner)
        .bind(format!("probe-{owner}"))
        .execute(pool)
        .await
        .expect("seed owner profile");
    sqlx::query(
        "INSERT INTO kb_blob_uploads (id, owner_profile_id, home_table, home_id, \
                                    content_type, created, updated) \
         VALUES ($1, $2, 'kb_contexts', $2, 'application/octet-stream', \
                 now() - make_interval(secs => $3::double precision), \
                 now() - make_interval(secs => $4::double precision))",
    )
    .bind(id)
    .bind(owner)
    .bind(created_age_seconds as f64)
    .bind(updated_age_seconds as f64)
    .execute(pool)
    .await
    .expect("seed staged session");

    for seq in 0..segment_count {
        sqlx::query(
            "INSERT INTO kb_blob_upload_segments (upload_id, seq, bytes, segment_hash) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(seq)
        .bind(vec![b'x'; segment_bytes as usize])
        .bind(format!("hash-{id}-{seq}"))
        .execute(pool)
        .await
        .expect("seed segment");
    }
}

async fn survivor_ids(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM kb_blob_uploads ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("read back sessions")
}

async fn survivor_segments(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_blob_upload_segments")
        .fetch_one(pool)
        .await
        .expect("read back segments")
}

/// **The headline ruling, asserted in the direction that matters.**
///
/// FAILS IF: the sweep reaps on `created`. `fresh-but-old` was begun three days ago and
/// appended to an hour ago — old by `created`, alive by `updated` — and deleting it
/// discards a resumable upload its owner is still working on. Its stale sibling, whose
/// last touch is past the window, is the row this sweep exists for.
#[sqlx::test(migrations = "../../migrations")]
async fn a_session_past_the_ttl_is_reaped_and_a_touched_one_survives(pool: PgPool) {
    let stale = Uuid::now_v7();
    let fresh_but_old = Uuid::now_v7();
    seed_session(&pool, &stale, 2 * 86_400, 2 * 86_400, 2, 512).await;
    seed_session(&pool, &fresh_but_old, 3_600, 3 * 86_400, 1, 256).await;

    let summary = reap_abandoned_blob_uploads(&pool).await.expect("sweep");

    assert_eq!(
        survivor_ids(&pool).await,
        vec![fresh_but_old],
        "a session past the TTL on its last touch goes; one touched inside the window \
         stays, however old its begin"
    );
    assert_eq!(
        survivor_segments(&pool).await,
        1,
        "the surviving session keeps its segments"
    );
    assert_eq!(
        summary,
        BlobReapSummary {
            uploads_reaped: 1,
            bytes_freed: 2 * 512,
            more_pending: false,
        },
        "the summary reports the reaped rows and the bytes they freed"
    );
}

/// FAILS IF: the cascade does not carry, or the sweep stopped at the session row and
/// left the bytes. The freed storage is the segments; a reaper that reports bytes but
/// leaves `bytea` rows behind is a lie with a summary.
#[sqlx::test(migrations = "../../migrations")]
async fn a_reaped_session_takes_its_segments_with_it(pool: PgPool) {
    let doomed = Uuid::now_v7();
    seed_session(&pool, &doomed, 2 * 86_400, 2 * 86_400, 3, 1024).await;

    reap_abandoned_blob_uploads(&pool).await.expect("sweep");

    assert!(
        survivor_ids(&pool).await.is_empty(),
        "the past-TTL session is gone"
    );
    assert_eq!(
        survivor_segments(&pool).await,
        0,
        "the session's staged bytes are gone with it"
    );
}

/// FAILS IF: the sweep is not idempotent. The cron runs it on a schedule forever; a
/// second pass over the same state must delete nothing and must not error.
#[sqlx::test(migrations = "../../migrations")]
async fn a_second_sweep_over_the_same_state_deletes_nothing(pool: PgPool) {
    let ancient = Uuid::now_v7();
    seed_session(&pool, &ancient, 100 * 86_400, 100 * 86_400, 1, 64).await;

    let first = reap_abandoned_blob_uploads(&pool)
        .await
        .expect("first sweep");
    assert!(
        survivor_ids(&pool).await.is_empty(),
        "the first sweep clears what is past the TTL"
    );
    assert_eq!(first.uploads_reaped, 1);

    let second = reap_abandoned_blob_uploads(&pool)
        .await
        .expect("second sweep");
    assert_eq!(
        second,
        BlobReapSummary::default(),
        "a re-run is a no-op, and says so in its summary"
    );
}

/// The read-back the summary's `bytes_freed` rests on: `octet_length` over the victims'
/// segments, summed in the delete's own statement. A live session (untouched by the
/// sweep) contributes nothing to either count — checked from the summary rather than a
/// hand-tallied constant, so a seeding bug cannot flatter the sweep.
#[sqlx::test(migrations = "../../migrations")]
async fn the_summary_counts_only_what_the_sweep_reaped(pool: PgPool) {
    let doomed = Uuid::now_v7();
    let survivor = Uuid::now_v7();
    seed_session(&pool, &doomed, 2 * 86_400, 2 * 86_400, 2, 100).await;
    seed_session(&pool, &survivor, 60, 60, 4, 1_000).await;

    let summary = reap_abandoned_blob_uploads(&pool).await.expect("sweep");

    assert_eq!(
        summary,
        BlobReapSummary {
            uploads_reaped: 1,
            bytes_freed: 200,
            more_pending: false,
        },
        "the survivor's four live segments count toward neither row nor byte"
    );
    assert_eq!(survivor_ids(&pool).await, vec![survivor]);
}

// The per-run cap (`more_pending`) is the incumbent's measured posture carried over
// wholesale; driving 5_001 sessions through seeding to bite it would be ceremony.
// Unwitnessed HERE, and — checked before writing this note — unwitnessed in the
// incumbent's suite too: no test anywhere drives the cap exit. The loop is carried by
// inspection, line for line. Declared, not silently absent.
