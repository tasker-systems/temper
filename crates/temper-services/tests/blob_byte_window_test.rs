//! Integration test — the post-commit byte window (task
//! `01a09360-e00a-7d90-858d-f4998dd70b6c`, goal `01a07684-8baf-7a72-a6aa-8549a7635f04`).
//!
//! The invariant under witness: **no live `kb_blobs` row exists whose provider object is
//! absent** — the "ghost" state. It was reachable through two interleavings, both traced to
//! file:line in [the axes sitting](vault research `01a09360-742b-7540-bde7-6a1bce4c0e9c`):
//!
//! * **P1** — a commit whose home-scoped dedup pre-check saw the row a concurrent strike
//!   empties; the pre-check suppresses the put, the strike releases and deletes the bytes,
//!   and the commit's insert mints a fresh row over the absent object.
//! * **P2** — a commit into a second home that DID put its bytes, eaten by a sibling
//!   strike's unconditional post-commit delete.
//!
//! The choreography is honest precisely because only the PROVIDER's timing is staged —
//! the one component whose interleaving production genuinely permits. Both DB sides run
//! real entry points (`commit_blob`, `blob_service::delete_blob`) end to end; the gated
//! store turns the provider's unordered timing into a deterministic handoff (the commit's
//! presence check is signaled; the release's delete waits for that signal before striking).
//!
//! Under the unfixed tree these tests were shown RED (the ghost formed; the runs are the
//! bite evidence in the PR). The fix makes the ghost unconstructible: byte presence is
//! re-derived and restored under the hash advisory lock, and byte releases serialize on
//! the same lock.
#![cfg(feature = "test-db")]

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::authorship::ActContext;
use temper_core::types::ids::ProfileId;
use temper_services::config::{BlobConfig, BlobCredentialMode};
use temper_services::services::blob_service;
use temper_substrate::blob_store::{BlobStore, InMemoryBlobStore, PutReceipt};
use temper_workflow::operations::Surface;

// ── fixtures ────────────────────────────────────────────────────────────────────────

/// Seed a substrate profile + emitter entities + a profile-owned context (the
/// `blob_delete_door_test` shape, minus what these witnesses do not need).
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (Uuid, Uuid, String) {
    let profile_id = Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    let context_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id, handle)
}

fn blob_cfg() -> BlobConfig {
    BlobConfig {
        store_id: "store_test".to_string(),
        read_write_token: Some("vercel_rw_test_store_test".to_string()),
        credential_mode: BlobCredentialMode::Token,
        oidc_token_source: Arc::new(|| None),
        max_bytes: 1 << 20,
        allowlist: vec!["image/png".to_string()],
        single_request_max_bytes: 64 * 1024,
    }
}

async fn commit_blob(
    pool: &PgPool,
    store: &dyn BlobStore,
    home: Uuid,
    caller: Uuid,
    bytes: &[u8],
) -> temper_services::services::blob_service::BlobCommitOutcome {
    blob_service::commit_blob(
        pool,
        store,
        &blob_cfg(),
        blob_service::BlobCommitCommand {
            caller: ProfileId::from(caller),
            home_table: Some("kb_contexts".to_string()),
            home_id: Some(home.to_string()),
            content_type: "image/png".to_string(),
            bytes: Bytes::copy_from_slice(bytes),
            surface: Surface::ApiHttp,
        },
    )
    .await
    .expect("commit through the service")
}

async fn delete_blob(
    pool: &PgPool,
    store: &dyn BlobStore,
    caller: Uuid,
    blob: Uuid,
) -> Result<temper_core::types::blob::BlobDeleteAck, temper_services::error::ApiError> {
    blob_service::delete_blob(
        pool,
        ProfileId::from(caller),
        temper_core::types::ids::BlobId::from(blob),
        store,
        ActContext::default(),
        Surface::ApiHttp,
    )
    .await
}

/// Live rows carrying the hash — the ghost's first half.
async fn live_rows_for_hash(pool: &PgPool, hash: &str) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!: i64" FROM kb_blobs
           WHERE content_hash = $1 AND content_type IS NOT NULL"#,
        hash
    )
    .fetch_one(pool)
    .await
    .expect("live-row count")
}

// ── the gated provider ──────────────────────────────────────────────────────────────

/// A provider whose internal TIMING is choreographed — the one component whose
/// interleaving production genuinely permits. `exists` reads the truth FIRST, then
/// signals that the commit's presence check has passed, then waits on the gate before
/// ANSWERING (the captured answer may be stale by then — exactly the production race).
/// `delete` waits for that same signal, so a release can strike bytes a commit has
/// already decided are present.
#[derive(Debug)]
struct GatedStore {
    inner: InMemoryBlobStore,
    /// Gate arming: the seeded commit runs ungated; only the RACING commit's presence
    /// check is held.
    armed: std::sync::atomic::AtomicBool,
    exists_checked_tx: tokio::sync::watch::Sender<bool>,
    gate_tx: tokio::sync::watch::Sender<bool>,
    gate_rx: tokio::sync::watch::Receiver<bool>,
}

impl GatedStore {
    fn new() -> Arc<Self> {
        let (exists_checked_tx, _) = tokio::sync::watch::channel(false);
        let (gate_tx, gate_rx) = tokio::sync::watch::channel(false);
        Arc::new(Self {
            inner: InMemoryBlobStore::default(),
            armed: std::sync::atomic::AtomicBool::new(false),
            exists_checked_tx,
            gate_tx,
            gate_rx,
        })
    }

    /// Hold the NEXT `exists` answer behind the gate — call before spawning the racing
    /// commit.
    fn arm(&self) {
        self.armed.store(true, std::sync::atomic::Ordering::Release);
    }

    /// The ungated truth — assertions must read the provider without joining the
    /// choreography.
    async fn probe_exists(&self, pathname: &str) -> bool {
        self.inner.exists(pathname).await.expect("probe exists")
    }
}

#[async_trait]
impl BlobStore for GatedStore {
    async fn exists(&self, pathname: &str) -> anyhow::Result<bool> {
        // Capture the answer FIRST — the signal tells the striker the check has passed,
        // and the gate holds the ANSWER back until the release has had its turn. A stale
        // `true` surviving a concurrent delete is exactly the production race.
        let captured = self.inner.exists(pathname).await?;
        if self.armed.load(std::sync::atomic::Ordering::Acquire) {
            self.exists_checked_tx.send(true).ok();
            let mut gate = self.gate_rx.clone();
            while !*gate.borrow_and_update() {
                gate.changed().await.ok();
            }
        }
        Ok(captured)
    }

    async fn put(
        &self,
        pathname: &str,
        content_type: &str,
        body: Bytes,
        cache_control_max_age: u32,
    ) -> anyhow::Result<PutReceipt> {
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
        // The release waits for the commit's presence check — the interleaving both
        // ghosts ride.
        let mut exists_checked = self.exists_checked_tx.subscribe();
        while !*exists_checked.borrow_and_update() {
            exists_checked.changed().await.ok();
        }
        self.inner.delete(pathnames).await
    }
}

/// The invariant, stated so it can only fail one way: a live row's bytes are at the
/// provider. `live > 0 && !present` is the ghost.
async fn assert_no_ghost(pool: &PgPool, store: &GatedStore, hash: &str, pathname: &str) {
    let live = live_rows_for_hash(pool, hash).await;
    let present = store.probe_exists(pathname).await;
    assert!(
        !(live > 0 && !present),
        "GHOST: {live} live row(s) carry hash {hash} but the provider holds no object at \
         {pathname} — a commit minted a row over bytes a strike released"
    );
}

const THE_BYTES: &[u8] = b"byte-window-witness-bytes";

async fn pathname_for(hash: &str) -> String {
    temper_substrate::blob_store::blob_pathname(hash)
}

// ── the witnesses ───────────────────────────────────────────────────────────────────

/// **P1**: the commit whose pre-check deduped is paused behind its presence check while a
/// strike of the very row it deduped against empties it, releases the bytes, and deletes
/// them. The commit then answers its (now stale) presence check and mints its row. Under
/// the unfixed tree this minted the ghost; the fix holds the advisory lock from before the
/// presence check through the commit, so the strike cannot interleave and the restored
/// bytes cannot be eaten.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn p1_a_commit_paused_behind_its_presence_check_cannot_mint_a_row_over_struck_bytes(
    pool: PgPool,
) {
    let (owner, home, _handle) = seed_profile_with_context(&pool, "p1@example.com").await;
    let store = GatedStore::new();

    let committed = commit_blob(&pool, store.as_ref(), home, owner, THE_BYTES).await;
    let hash = committed.content_hash.clone();
    let pathname = pathname_for(&hash).await;

    // The racing commit: same bytes, same home. Its pre-check dedups against the live
    // row, so it never puts; its presence check is where the choreography holds it.
    store.arm();
    let pool_for_c = pool.clone();
    let store_for_c = store.clone();
    let c = tokio::spawn(async move {
        commit_blob(&pool_for_c, store_for_c.as_ref(), home, owner, THE_BYTES).await
    });

    // Hold until C's presence check has passed, then let the strike complete: it empties
    // the deduped row (the last live row of the hash → released) and its post-commit
    // release deletes the bytes. Under the fix the strike CANNOT complete here — C holds
    // the advisory lock — so the timeout is the fixed tree's path, never a flake.
    let mut exists_checked = store.exists_checked_tx.subscribe();
    while !*exists_checked.borrow_and_update() {
        exists_checked.changed().await.ok();
    }
    let pool_for_s = pool.clone();
    let store_for_s = store.clone();
    let strike = tokio::spawn(async move {
        delete_blob(
            &pool_for_s,
            store_for_s.as_ref(),
            owner,
            committed.blob_id.uuid(),
        )
        .await
    });
    let struck_within_window =
        tokio::time::timeout(std::time::Duration::from_secs(2), strike).await;

    // Release C. Today it answers the stale `true` and mints the ghost; under the fix its
    // restore has already re-put (or the strike never released at all).
    store.gate_tx.send(true).ok();
    let c_outcome = c.await.expect("racing commit task must not panic");
    assert!(
        c_outcome.deduped,
        "P1's spine: the pre-check deduped against the live row, which is why no put ran"
    );
    let _ = struck_within_window;

    assert_no_ghost(&pool, &store, &hash, &pathname).await;
}

/// **P2**: a commit into a SECOND home that paid its put, with a sibling strike of the
/// first home's row releasing and deleting those same bytes. The put is not protection —
/// an unconditional post-commit delete eats it. Under the fix the release re-derives
/// released-ness under the advisory lock, sees the second row live, and skips.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn p2_a_release_cannot_eat_a_concurrent_commits_put(pool: PgPool) {
    let (owner_a, home_a, _ha) = seed_profile_with_context(&pool, "p2a@example.com").await;
    let (owner_b, home_b, _hb) = seed_profile_with_context(&pool, "p2b@example.com").await;
    let store = GatedStore::new();

    // Home A holds the hash's only live row.
    let first = commit_blob(&pool, store.as_ref(), home_a, owner_a, THE_BYTES).await;
    let hash = first.content_hash.clone();
    let pathname = pathname_for(&hash).await;

    // Home B's commit: clean pre-check, a real put, then paused behind its presence check.
    store.arm();
    let pool_for_c = pool.clone();
    let store_for_c = store.clone();
    let c = tokio::spawn(async move {
        commit_blob(
            &pool_for_c,
            store_for_c.as_ref(),
            home_b,
            owner_b,
            THE_BYTES,
        )
        .await
    });
    let mut exists_checked = store.exists_checked_tx.subscribe();
    while !*exists_checked.borrow_and_update() {
        exists_checked.changed().await.ok();
    }

    // Home A's custodian strikes: B's row is not yet live (C is paused), so the refcount
    // is exact at ONE and the act releases — the release's delete fires because the
    // presence check it waits on already has.
    let pool_for_s = pool.clone();
    let store_for_s = store.clone();
    let strike = tokio::spawn(async move {
        delete_blob(
            &pool_for_s,
            store_for_s.as_ref(),
            owner_a,
            first.blob_id.uuid(),
        )
        .await
    });
    let struck_within_window =
        tokio::time::timeout(std::time::Duration::from_secs(2), strike).await;

    store.gate_tx.send(true).ok();
    let c_outcome = c.await.expect("racing commit task must not panic");
    assert!(
        !c_outcome.deduped,
        "P2's spine: home B's pre-check was clean, so the commit paid its own put"
    );
    let _ = struck_within_window;

    assert_no_ghost(&pool, &store, &hash, &pathname).await;
}
