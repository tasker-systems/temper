//! The erasure byte-delete fence (spec 2026-08-31 D3/D5; the delete-act design's substrate
//! contract, Beat 4 of task 01a0577c).
//!
//! `blob_delete` releases bytes POST-commit ("a provider call cannot join the transaction",
//! 20260906000010) and names the release in the `principal_erased` (or `resource_erased`)
//! payload — so between the act's commit and the provider delete there is a window no
//! transaction can close. The substrate contract rules what watches it: *"A byte-deleting build
//! MUST run that fence or its equivalent — retry plus age alerting."* This module is that fence,
//! in three moves:
//!
//! * **Derivation** — pending deletes are DERIVED from the `principal_erased` and
//!   `resource_erased` payloads' per-target strike verdicts (specific pathnames, never provider
//!   enumeration — BlobStore has no `list`), seeded into `kb_erasure_blob_deletes`
//!   (20260909000040) with first-due at the EVENT's `occurred_at`. Every tick re-derives from the
//!   ledger; the seed's `(erasure_event_id, pathname)` key makes re-derivation free.
//! * **Drain** — one tick reaps expired leases, claims due deletes in one bounded batch,
//!   then — inside ONE critical section over the hashes' advisory locks — RE-DERIVES
//!   released-ness at drain time, issues ONE idempotent `BlobStore::delete(&[&str])` for
//!   everything still released, and records the completions. A hash a live row re-holds is
//!   NOT deleted: the commit path restores byte presence under the same lock before a row
//!   can go live (task 01a09360-e00a-7d90-858d-f4998dd70b6c), so the skip records a real
//!   re-occupation. Failure hands the batch back through the 20260828000030 backoff curve;
//!   the max-attempts arm goes `dead`.
//! * **Age alerting** — [`fence_channel_report`] renders the fence's durable state into the
//!   `internal_call_health_service` vocabulary: the same `ChannelState` enum whose `Sustained`
//!   is the alertable state an alert rule matches, the same span fields. A pending delete older
//!   than [`ALERTABLE_AFTER_SECONDS`] — or a `dead` one — is `Sustained`: an alarm, not a
//!   silent retry forever. No alert-delivery is built here; the consumers exist.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use temper_substrate::blob_store::BlobStore;
use temper_substrate::payloads::ErasureTargetOutcome;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::internal_call_health_service::{ChannelReport, ChannelState};

/// The fence's channel name in the `internal_call_health` vocabulary. Not a
/// `kb_internal_call_health` writer — this channel's facts are DERIVED from the fence's own
/// durable table, so the drain's silence cannot hide debt: the rows age whether or not the cron
/// ever runs again. Reported through the same check, the same states, the same span.
pub const ERASURE_FENCE_CHANNEL: &str = "erasure_blob_deletes";

/// How old a pending delete may get before the fence is alarming rather than merely behind.
///
/// One day. The retry ladder exhausts in about two and a half hours (five attempts through the
/// capped curve), so a delete this old has survived a full day of drains — a provider outage, a
/// dark cron, or bytes that will not go. Shorter is noise against a one-minute cron that is
/// merely slow; longer lets an erasure obligation linger unwatched. The tunable half is the
/// alert rule, which reads the raw fields this report emits and needs no deploy to change (the
/// `internal_call_health_service` posture).
pub const ALERTABLE_AFTER_SECONDS: i64 = 86_400;

/// Deletes claimed per tick. The provider call is one batched `BlobStore::delete`, so this
/// bounds the batch, not a loop.
pub const DRAIN_BATCH: i32 = 100;

/// How long a claimed delete holds its lease before the reaper hands it back. One pass is a
/// single provider call; the lease exists for the crash the drain does not return from.
const LEASE_SECONDS: i32 = 600;

/// The completion resolution for a hash a live row re-holds at drain time. Under the
/// drain's advisory locks the re-derivation is EXACT: a live row's bytes are present —
/// the commit path restores byte presence under the same lock before the row can go live
/// (task 01a09360-e00a-7d90-858d-f4998dd70b6c) — so the skip records a real
/// re-occupation. When the re-holding hash is in `kb_erased_content`, that re-occupation
/// is the ruled custody-never-bytes posture, not a laundering: 20260911000000 retired the
/// re-admission refusal deliberately (erasure is offboarding; a third party lawfully
/// holding the same bytes re-commits them like any content), and this resolution records
/// the erasure byte obligation's lawful end rather than hiding it.
const SKIPPED_REOCCUPIED: &str = "skipped-reoccupied";

/// The completion resolution for bytes the provider actually struck.
const RESOLUTION_DELETED: &str = "deleted";

/// The delete-target prefix of Beat 2's pinned v1 outcome vocabulary
/// (`20260909000025`: `'erased; released=' || v_rel::text || '; pathname=' || v_path`): a
/// strike is a delete target exactly when it reads `erased; released=true` — `released=false`
/// means the strike-time refcount found another live row holding the hash, so the bytes were
/// never this act's to remove. `already-erased` and the `independent_obligation` remainder are
/// not byte fates at all.
const RELEASED_STRIKE_PREFIX: &str = "erased; released=true; pathname=";

/// The strike-time refcount held: another live row re-holds the hash, so the bytes were never
/// this act's to remove. A KNOWN shape, and never a delete target.
const HELD_STRIKE_PREFIX: &str = "erased; released=false; pathname=";

/// A governed-home row that was ALREADY struck when this act reached it. A KNOWN shape, and
/// never a delete target.
const ALREADY_ERASED_OUTCOME: &str = "already-erased";

/// The named remainder (disposition iii): the row's home is governed by a team or map, so the
/// act never struck it. A KNOWN shape, and never a delete target.
const OBLIGATION_OUTCOME_PREFIX: &str = "independent_obligation: ";

/// The only payload target whose outcome this fence parses — every other target kind's
/// outcome is other vocabulary entirely.
pub(crate) const BLOB_TARGET: &str = "kb_blobs";

/// The total classification of a `kb_blobs` outcome string against the pinned v1 vocabulary.
///
/// The fence's prose interface MUST fail loud on drift (the substrate contract: retry plus age
/// alerting — a verdict the fence cannot parse would otherwise silently no-op the whole
/// derivation while the obligation stands, [`ChannelState::Healthy`] over unpaid debt). An
/// outcome matching NONE of the known shapes is therefore its own arm, counted in the seed
/// summary and raised as an alertable cause in [`fence_channel_report`] — never quietly
/// treated as known.
pub(crate) fn classify_blob_outcome(outcome: &str) -> BlobOutcomeClass {
    if let Some(pathname) = outcome.strip_prefix(RELEASED_STRIKE_PREFIX) {
        // The template demands a path; a released verdict with an empty pathname is drift,
        // not a strike.
        if !pathname.is_empty() {
            return BlobOutcomeClass::Released(pathname.to_owned());
        }
        return BlobOutcomeClass::Unrecognized;
    }
    if outcome.starts_with(HELD_STRIKE_PREFIX)
        || outcome == ALREADY_ERASED_OUTCOME
        || outcome.starts_with(OBLIGATION_OUTCOME_PREFIX)
    {
        return BlobOutcomeClass::Known;
    }
    BlobOutcomeClass::Unrecognized
}

/// The content hash a released strike's pathname names. The pathname IS the derivation
/// (`blob_pathname()`, blob_store.rs: `{hash[0:2]}/{hash}`), so the hash — the key the
/// drain-time refcount check and the post-commit release lock on — is its last segment. One
/// derivation for every reader of a strike verdict, so the hash locked and the pathname deleted
/// always come from the same string.
pub(crate) fn content_hash_of_pathname(pathname: &str) -> &str {
    pathname.rsplit('/').next().unwrap_or(pathname)
}

pub(crate) enum BlobOutcomeClass {
    /// `erased; released=true; pathname=…` — the bytes were this act's to remove.
    Released(String),
    /// A known, non-seeding shape (held strike, already-erased, independent_obligation).
    Known,
    /// No known shape — prose drift; counted and alertable, never silently skipped.
    Unrecognized,
}

/// What one whole-catalogue derivation pass found.
struct SeedScan {
    /// Rows newly created.
    seeded: u64,
    /// `kb_blobs` targets whose outcome matched NO known shape — prose drift; never seeded,
    /// counted so the fence fails loud (see [`classify_blob_outcome`]).
    unparseable: usize,
}

/// Derive pending deletes from every `principal_erased` and `resource_erased` payload and seed
/// the not-yet-seeded ones. Store-independent by construction: the work is DERIVED from the
/// ledger, and nothing here touches a provider (a derivation that needed the store could never
/// run for a deployment whose provider configuration is gone — exactly the deployment whose
/// stranded deletes most need the fence to see them).
///
/// The scan is whole-catalogue on purpose: erasures are rare admin acts, the seed is
/// `ON CONFLICT DO NOTHING` against the (event, pathname) key, and derive-don't-remember means
/// the ledger alone drives the fence — there is no enqueue step whose failure could strand a
/// release.
async fn seed_from_ledger(pool: &PgPool) -> ApiResult<SeedScan> {
    let rows = sqlx::query!(
        r#"
        SELECT e.id           AS "erasure_event_id!: Uuid",
               e.occurred_at  AS "occurred_at!: DateTime<Utc>",
               e.payload->'targets' AS "targets!: serde_json::Value"
          FROM kb_events e
          JOIN kb_event_types t ON t.id = e.event_type_id
                                 AND t.name IN ('principal_erased', 'resource_erased')
         ORDER BY e.occurred_at, e.id
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut scan = SeedScan {
        seeded: 0,
        unparseable: 0,
    };
    for row in rows {
        let targets: Vec<ErasureTargetOutcome> =
            serde_json::from_value(row.targets).map_err(|e| {
                ApiError::Internal(format!(
                    "erasure event {} carries an unreadable targets array: {e}",
                    row.erasure_event_id
                ))
            })?;
        for t in &targets {
            if t.target != BLOB_TARGET {
                continue;
            }
            let pathname = match classify_blob_outcome(&t.outcome) {
                BlobOutcomeClass::Released(pathname) => pathname,
                BlobOutcomeClass::Known => continue,
                BlobOutcomeClass::Unrecognized => {
                    scan.unparseable += 1;
                    continue;
                }
            };
            // Derived once, at seed, rather than re-parsed on every claim.
            let content_hash = content_hash_of_pathname(&pathname).to_owned();
            let seeded_id = sqlx::query_scalar!(
                r#"SELECT erasure_delete_seed($1, $2, $3, $4) AS "id: Uuid""#,
                row.erasure_event_id,
                content_hash,
                pathname,
                row.occurred_at,
            )
            .fetch_one(pool)
            .await?;
            if seeded_id.is_some() {
                scan.seeded += 1;
            }
        }
    }
    Ok(scan)
}

/// What one drain tick did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DrainSummary {
    /// Deletes newly derived this tick from `principal_erased` and `resource_erased` payloads
    /// (the blob delete door seeds the same queue inside its own transaction).
    pub seeded: u64,
    /// `kb_blobs` targets whose outcome matched NO known strike-verdict shape (prose drift).
    /// Never seeded; the report raises `unparseable_verdicts` as an alertable cause — the
    /// prose interface fails loud rather than silently no-oping the fence.
    pub unparseable_verdicts: usize,
    /// Leases the reaper reclaimed (a previous tick died mid-pass).
    pub reaped: i32,
    /// Deletes claimed this tick.
    pub claimed: usize,
    /// Pathnames the provider struck (one batched call).
    pub deleted: usize,
    /// Strikes skipped because a live row re-holds the hash — re-put bytes are not erased.
    pub skipped_reoccupied: usize,
    /// Deletes handed back to the retry ladder (or `dead` at max attempts).
    pub failed: usize,
}

/// One claimed fence row, exactly what `erasure_delete_claim` returns.
#[derive(Debug, sqlx::FromRow)]
struct ClaimedDelete {
    id: Uuid,
    content_hash: String,
    pathname: String,
}

/// One fence tick WITHOUT a configured provider: seed-and-report only. Seeding is
/// store-independent — the deletes are DERIVED from the ledger, never enumerated from a
/// provider (BlobStore has no `list`) — so a deployment whose provider configuration is
/// absent must still seed its erasures' released strikes and still age them into the
/// alertable state. That is the substrate contract's exact stranding posture ("a byte-deleting
/// build MUST run that fence or its equivalent — retry plus age alerting"): config-removal
/// darks the provider CALLS, never the watching. No provider call, no claims.
pub async fn drain_without_store(pool: &PgPool) -> ApiResult<DrainSummary> {
    let scan = seed_from_ledger(pool).await?;
    Ok(DrainSummary {
        seeded: scan.seeded,
        unparseable_verdicts: scan.unparseable,
        ..DrainSummary::default()
    })
}

/// One fence tick: derive → reap → claim → re-derive released-ness UNDER the hash
/// advisory locks → one batched delete → completions, all inside that one critical
/// section.
pub async fn drain(pool: &PgPool, store: &dyn BlobStore) -> ApiResult<DrainSummary> {
    let scan = seed_from_ledger(pool).await?;
    let mut summary = DrainSummary {
        seeded: scan.seeded,
        unparseable_verdicts: scan.unparseable,
        ..DrainSummary::default()
    };

    summary.reaped = sqlx::query_scalar!(
        r#"SELECT erasure_delete_reap($1) AS "n!: i32""#,
        "lease expired"
    )
    .fetch_one(pool)
    .await?;

    let claimed: Vec<ClaimedDelete> = sqlx::query_as!(
        ClaimedDelete,
        r#"
        SELECT id AS "id!: Uuid", content_hash AS "content_hash!", pathname AS "pathname!"
          FROM erasure_delete_claim($1, $2)
        "#,
        DRAIN_BATCH,
        LEASE_SECONDS,
    )
    .fetch_all(pool)
    .await?;
    summary.claimed = claimed.len();
    if claimed.is_empty() {
        return Ok(summary);
    }

    // ONE critical section (task 01a09360-e00a-7d90-858d-f4998dd70b6c), run through a
    // bounded deadlock retry: the advisory locks for every claimed hash — acquired through
    // the substrate's ONE Rust definition of the key, in sorted order — the released-ness
    // re-derivation, the batched provider delete, and the completions share ONE
    // transaction, so a commit can neither interleave a live row inside the re-derivation
    // nor restore-and-live behind a delete that targeted it. The provider call inside a
    // transaction is deliberate and bounded: this transaction touches only the
    // already-claimed queue rows and the locks — no blob rows, no events (the reasoning
    // `writes::release_blob_bytes` carries for the door's release). What the re-derivation
    // sees under the lock is the truth the delete acts on.
    let hashes: Vec<String> = claimed.iter().map(|c| c.content_hash.clone()).collect();
    let mut deadlock_retries = 0;
    let outcome = loop {
        match run_critical_section(pool, store, &claimed, &hashes).await? {
            CriticalSection::Resolved(outcome) => break outcome,
            CriticalSection::Deadlocked if deadlock_retries < 2 => {
                // Postgres resolved a multi-taker cycle by aborting this transaction (the
                // erasure act takes its hashes in unordered cursor order and is the other
                // multi-lock taker in this lock space). The abort releases everything and
                // recorded nothing; an immediate retry re-derives against the post-cycle
                // world. Bounded: after three, the batch goes to the ladder below.
                deadlock_retries += 1;
            }
            CriticalSection::Deadlocked => {
                // Out of retries: nothing was recorded, the claims keep their lease, and
                // the reaper hands them back through the normal ladder. The next tick
                // re-derives against whatever the concurrent multi-key taker did.
                return Ok(summary);
            }
        }
    };

    match outcome {
        CriticalOutcome::SkippedOnly { skipped } => {
            summary.skipped_reoccupied = skipped;
        }
        CriticalOutcome::Deleted { skipped, deleted } => {
            summary.skipped_reoccupied = skipped;
            summary.deleted = deleted;
        }
        CriticalOutcome::ProviderFailed {
            skipped,
            ids,
            error,
        } => {
            // The provider said no — but the critical section COMMITTED: the skips are
            // resolved same-tick (a re-occupied hash's bytes were never at risk, and
            // stranding them on the lease ladder burned real retries toward a false dead
            // and raised false age alerts). Only the unreleasable batch hands back to the
            // retry ladder (attempts were already incremented at claim; the fail arms the
            // bounded backoff).
            summary.skipped_reoccupied = skipped;
            summary.failed = ids.len();
            sqlx::query_scalar!(
                r#"SELECT erasure_delete_fail($1, $2) AS "n!: i32""#,
                &ids[..],
                error,
            )
            .fetch_one(pool)
            .await?;
        }
    }

    Ok(summary)
}

/// What one pass of the drain's critical section resolved.
enum CriticalOutcome {
    /// Every claim re-derived to a live row re-holding its hash.
    SkippedOnly { skipped: usize },
    /// The skips plus a provider delete that landed.
    Deleted { skipped: usize, deleted: usize },
    /// The provider refused the delete; the skips are committed, the failed batch's ids
    /// ride back to the retry ladder.
    ProviderFailed {
        skipped: usize,
        ids: Vec<Uuid>,
        error: String,
    },
}

/// The resolved-or-deadlocked verdict of one critical-section pass.
enum CriticalSection {
    Resolved(CriticalOutcome),
    Deadlocked,
}

/// One pass: sorted advisory takes (the substrate's one Rust key), the occupied
/// re-derivation UNDER them, the batched delete, the completions — one transaction. A
/// `40P01` (deadlock resolved by abort) leaves nothing recorded and reports
/// [`CriticalSection::Deadlocked`]; the skips resolve in the same transaction as the
/// delete when it lands, and survive a provider failure.
async fn run_critical_section(
    pool: &PgPool,
    store: &dyn BlobStore,
    claimed: &[ClaimedDelete],
    hashes: &[String],
) -> ApiResult<CriticalSection> {
    let mut locked_hashes = hashes.to_vec();
    locked_hashes.sort();
    locked_hashes.dedup();
    let deadlock = |e: &sqlx::Error| {
        e.as_database_error()
            .is_some_and(|d| d.code().as_deref() == Some("40P01"))
    };

    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) if deadlock(&e) => return Ok(CriticalSection::Deadlocked),
        Err(e) => return Err(e.into()),
    };
    for hash in &locked_hashes {
        if let Err(e) = temper_substrate::writes::take_hash_lock(&mut tx, hash).await {
            return if deadlock(&e) {
                Ok(CriticalSection::Deadlocked)
            } else {
                Err(e.into())
            };
        }
    }
    let occupied: Vec<String> = sqlx::query!(
        r#"
        SELECT DISTINCT content_hash AS "content_hash!"
          FROM kb_blobs
         WHERE content_hash = ANY($1) AND content_type IS NOT NULL
        "#,
        hashes,
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| r.content_hash)
    .collect();

    let (skipped, releasable): (Vec<&ClaimedDelete>, Vec<&ClaimedDelete>) = claimed
        .iter()
        .partition(|c| occupied.iter().any(|h| h == &c.content_hash));

    let skipped_ids: Vec<Uuid> = skipped.iter().map(|c| c.id).collect();
    let ids: Vec<Uuid> = releasable.iter().map(|c| c.id).collect();

    if releasable.is_empty() {
        // Nothing to strike; record the skips and release the locks.
        if !skipped_ids.is_empty() {
            sqlx::query_scalar!(
                r#"SELECT erasure_delete_complete($1, $2) AS "n!: i32""#,
                &skipped_ids[..],
                SKIPPED_REOCCUPIED,
            )
            .fetch_one(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        return Ok(CriticalSection::Resolved(CriticalOutcome::SkippedOnly {
            skipped: skipped.len(),
        }));
    }

    // ONE batched call for the whole due set — the verb is array-shaped and idempotent (a
    // pathname the provider no longer holds deletes as a no-op), so the at-least-once fence
    // carries no existence guard. The locks stay held across it: a commit concurrent with
    // this delete waits, then finds the bytes absent and restores them under the lock it
    // finally takes.
    match store.delete(&pathnames_of(&releasable)).await {
        Ok(()) => {
            if !skipped_ids.is_empty() {
                sqlx::query_scalar!(
                    r#"SELECT erasure_delete_complete($1, $2) AS "n!: i32""#,
                    &skipped_ids[..],
                    SKIPPED_REOCCUPIED,
                )
                .fetch_one(&mut *tx)
                .await?;
            }
            sqlx::query_scalar!(
                r#"SELECT erasure_delete_complete($1, $2) AS "n!: i32""#,
                &ids[..],
                RESOLUTION_DELETED,
            )
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(CriticalSection::Resolved(CriticalOutcome::Deleted {
                skipped: skipped.len(),
                deleted: releasable.len(),
            }))
        }
        Err(e) => {
            // The provider said no: the releasable rows read nothing-done, but the skips
            // still commit — they never depended on the delete's outcome. The failed batch
            // is failed OUTSIDE the (now committed) critical section.
            if !skipped_ids.is_empty() {
                sqlx::query_scalar!(
                    r#"SELECT erasure_delete_complete($1, $2) AS "n!: i32""#,
                    &skipped_ids[..],
                    SKIPPED_REOCCUPIED,
                )
                .fetch_one(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            Ok(CriticalSection::Resolved(CriticalOutcome::ProviderFailed {
                skipped: skipped.len(),
                ids,
                error: e.to_string(),
            }))
        }
    }
}

fn pathnames_of<'a>(releasable: &'a [&'a ClaimedDelete]) -> Vec<&'a str> {
    releasable.iter().map(|c| c.pathname.as_str()).collect()
}

/// What the fence's durable state amounts to, in the `internal_call_health` vocabulary.
///
/// The state mapping, one arm per row-shape the table can hold:
///
/// * any `kb_blobs` verdict in the erasure ledger matching NO known shape —
///   [`ChannelState::Sustained`] with cause `unparseable_verdicts`: the fence's prose
///   interface has drifted, its derivation would silently no-op, and the obligation stands.
///   The scan is the same derive-don't-remember pass the seed runs, so the report cannot
///   disagree with it. Checked FIRST: drift outranks every row-state, because a drifted
///   template is permanent until code ships while row states can heal.
/// * no rows at all — [`ChannelState::NoAttemptRecorded`]: a deployment that has never erased
///   anything has no fence history. Never alertable, exactly the reconcile channel's refusal to
///   read silence as failure.
/// * rows, none active — [`ChannelState::Healthy`]: every derived delete has been paid or
///   skipped; there is no outstanding debt.
/// * active, none past [`ALERTABLE_AFTER_SECONDS`], none dead —
///   [`ChannelState::Transient`]: work outstanding, retries may still heal it.
/// * any active delete past the age bound, or any `dead` row —
///   [`ChannelState::Sustained`]: **the alertable state.** A delete this old has outlived its
///   whole retry ladder's worth of time; a `dead` one has exhausted retries outright. Neither
///   is a silent retry forever.
///
/// No `Stale` arm, deliberately: the rows are durable Postgres facts, so this verdict is always
/// a claim about now — the decay problem the reconcile channel solves with `Stale` (a row only
/// a login refreshes) does not exist here.
pub async fn fence_channel_report(pool: &PgPool, now: DateTime<Utc>) -> ApiResult<ChannelReport> {
    #[derive(Debug, sqlx::FromRow)]
    struct FenceCounts {
        rows_ever: i64,
        active: i64,
        active_failed: i64,
        failures_total: i64,
        oldest_active_first_due: Option<DateTime<Utc>>,
        last_completed_at: Option<DateTime<Utc>>,
        alertable_status: Option<String>,
        alertable_pathname: Option<String>,
        alertable_first_due: Option<DateTime<Utc>>,
    }

    // The same scan the seed runs, parse-only: every `kb_blobs` outcome ever put on the
    // ledger, for the unparseable-verdict arm above.
    let outcomes: Vec<String> = sqlx::query!(
        r#"
        SELECT tg->>'outcome' AS "outcome!"
          FROM kb_events e
          JOIN kb_event_types t ON t.id = e.event_type_id
                                 AND t.name IN ('principal_erased', 'resource_erased'),
               jsonb_array_elements(e.payload->'targets') AS tg
         WHERE tg->>'target' = $1
        "#,
        BLOB_TARGET,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| r.outcome)
    .collect();
    let unparseable_verdicts = outcomes
        .iter()
        .filter(|o| matches!(classify_blob_outcome(o), BlobOutcomeClass::Unrecognized))
        .count();

    let counts = sqlx::query_as!(
        FenceCounts,
        r#"
        SELECT base.rows_ever                  AS "rows_ever!",
               base.active                     AS "active!",
               base.active_failed              AS "active_failed!",
               base.failures_total             AS "failures_total!",
               base.oldest_active_first_due    AS "oldest_active_first_due",
               base.last_completed_at          AS "last_completed_at",
               a.status                        AS "alertable_status",
               a.pathname                      AS "alertable_pathname",
               a.first_due_at                  AS "alertable_first_due"
          FROM (
              SELECT count(*) AS rows_ever,
                     count(*) FILTER (WHERE status IN ('pending','in_progress','waiting_for_retry')) AS active,
                     count(*) FILTER (WHERE status IN ('pending','in_progress','waiting_for_retry')
                                        AND attempts > 0) AS active_failed,
                     -- Failures, not claims: a done row's last claim succeeded; every other
                     -- row's claims all ended in the failure path that put it there.
                     coalesce(sum(CASE WHEN status = 'done' THEN attempts - 1 ELSE attempts END), 0) AS failures_total,
                     min(first_due_at) FILTER (WHERE status IN ('pending','in_progress','waiting_for_retry')) AS oldest_active_first_due,
                     max(completed_at) FILTER (WHERE status = 'done') AS last_completed_at
                FROM kb_erasure_blob_deletes
          ) AS base
          LEFT JOIN LATERAL (
              SELECT status, pathname, first_due_at
                FROM kb_erasure_blob_deletes
               WHERE status = 'dead'
                  OR (status IN ('pending','in_progress','waiting_for_retry')
                      AND first_due_at < $1)
               ORDER BY first_due_at, id
               LIMIT 1
          ) a ON true
        "#,
        now - chrono::Duration::seconds(ALERTABLE_AFTER_SECONDS),
    )
    .fetch_one(pool)
    .await?;

    let state = if unparseable_verdicts > 0 {
        ChannelState::Sustained
    } else if counts.rows_ever == 0 {
        ChannelState::NoAttemptRecorded
    } else if counts.alertable_status.is_some() {
        ChannelState::Sustained
    } else if counts.active > 0 {
        ChannelState::Transient
    } else {
        ChannelState::Healthy
    };

    // The cause names WHY the fence is alertable, so the operator's action is readable off the
    // span: the prose interface drifted, retries exhausted outright, or a pending delete past
    // the age bound.
    let (failure_cause, failure_detail) = if unparseable_verdicts > 0 {
        (
            Some("unparseable_verdicts".to_string()),
            Some(format!(
                "{unparseable_verdicts} kb_blobs verdict(s) in the erasure ledger match no \
                 known strike-outcome shape — the fence cannot derive work from them"
            )),
        )
    } else {
        match counts.alertable_status.as_deref() {
            Some("dead") => (
                Some("retries_exhausted".to_string()),
                Some(format!(
                    "delete at {} exhausted its retry ladder",
                    counts.alertable_pathname.as_deref().unwrap_or("?"),
                )),
            ),
            Some(_) => {
                let age = counts
                    .alertable_first_due
                    .map(|d| (now - d).num_seconds())
                    .unwrap_or(0);
                (
                    Some("pending_past_age".to_string()),
                    Some(format!(
                        "delete at {} outstanding for {age}s (bound {ALERTABLE_AFTER_SECONDS}s)",
                        counts.alertable_pathname.as_deref().unwrap_or("?"),
                    )),
                )
            }
            None => (None, None),
        }
    };

    Ok(ChannelReport {
        channel: ERASURE_FENCE_CHANNEL.to_owned(),
        state,
        // `consecutive_failures` is BORROWED vocabulary on this channel: the fence has no call
        // streak to count, so the field carries the number of ACTIVE deletes that have failed
        // at least once (attempts > 0) — the closest per-row analogue, reported so the alert
        // rule reads one field shape across every channel.
        consecutive_failures: counts.active_failed as i32,
        failures_total: counts.failures_total,
        failure_cause,
        failure_detail,
        seconds_since_success: counts.last_completed_at.map(|at| (now - at).num_seconds()),
        failing_for_seconds: counts
            .oldest_active_first_due
            .map(|at| (now - at).num_seconds()),
    })
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
