//! The erasure byte-delete fence (spec 2026-08-31 D3/D5; the delete-act design's substrate
//! contract, Beat 4 of task 01a0577c).
//!
//! `blob_delete` releases bytes POST-commit ("a provider call cannot join the transaction",
//! 20260906000010) and names the release in the `principal_erased` payload — so between the
//! act's commit and the provider delete there is a window no transaction can close. The
//! substrate contract rules what watches it: *"A byte-deleting build MUST run that fence or its
//! equivalent — retry plus age alerting."* This module is that fence, in three moves:
//!
//! * **Derivation** — pending deletes are DERIVED from the `principal_erased` payload's
//!   per-target strike verdicts (specific pathnames, never provider enumeration — BlobStore has
//!   no `list`), seeded into `kb_erasure_blob_deletes` (20260909000040) with first-due at the
//!   EVENT's `occurred_at`. Every tick re-derives from the ledger; the seed's
//!   `(erasure_event_id, pathname)` key makes re-derivation free.
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
const BLOB_TARGET: &str = "kb_blobs";

/// The total classification of a `kb_blobs` outcome string against the pinned v1 vocabulary.
///
/// The fence's prose interface MUST fail loud on drift (the substrate contract: retry plus age
/// alerting — a verdict the fence cannot parse would otherwise silently no-op the whole
/// derivation while the obligation stands, [`ChannelState::Healthy`] over unpaid debt). An
/// outcome matching NONE of the known shapes is therefore its own arm, counted in the seed
/// summary and raised as an alertable cause in [`fence_channel_report`] — never quietly
/// treated as known.
fn classify_blob_outcome(outcome: &str) -> BlobOutcomeClass {
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

enum BlobOutcomeClass {
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

/// Derive pending deletes from every `principal_erased` payload and seed the not-yet-seeded
/// ones. Store-independent by construction: the work is DERIVED from the ledger, and nothing
/// here touches a provider (a derivation that needed the store could never run for a
/// deployment whose provider configuration is gone — exactly the deployment whose stranded
/// deletes most need the fence to see them).
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
          JOIN kb_event_types t ON t.id = e.event_type_id AND t.name = 'principal_erased'
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
            // The pathname IS the derivation (`blob_pathname()`, blob_store.rs:
            // `{hash[0:2]}/{hash}`), so the hash — the key the drain-time refcount check
            // needs — is its last segment. Derived once, at seed, rather than re-parsed on
            // every claim.
            let content_hash = pathname.rsplit('/').next().unwrap_or(&pathname).to_owned();
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
    /// Deletes newly derived from `principal_erased` payloads this tick.
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
          JOIN kb_event_types t ON t.id = e.event_type_id AND t.name = 'principal_erased',
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
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    use bytes::Bytes;
    use sqlx::PgPool;
    use temper_substrate::blob_store::{blob_pathname, InMemoryBlobStore};
    use temper_substrate::ids::{BlobId, ContextId, EntityId, ProfileId};
    use temper_substrate::payloads::AnchorRef;
    use temper_substrate::writes::{self, CommitBlobParams};
    use uuid::Uuid;

    use super::*;
    use crate::services::erasure_service::{self, ErasureOutcome};
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
        sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
                 VALUES ('kb_profiles', $1, 'notes', 'Notes') RETURNING id",
        )
        .bind(subject)
        .fetch_one(pool)
        .await
        .expect("seed personal context")
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
        let hash = format!("{:x}", sha2::Sha256::digest(bytes));
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

    /// Erase `subject` as `operator` — the real act, real payload, real strikes.
    async fn erase(
        pool: &PgPool,
        operator: Uuid,
        subject: Uuid,
    ) -> erasure_service::ErasureCompletion {
        let outcome = erasure_service::execute_erasure(
            pool,
            ProfileId::from(operator),
            ProfileId::from(subject),
            Uuid::now_v7(),
        )
        .await
        .expect("the act completes");
        match outcome {
            ErasureOutcome::Completed(c) => c,
            ErasureOutcome::Refused(r) => panic!("the operator's act must complete, got {r:?}"),
        }
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

    async fn row_of(
        pool: &PgPool,
        pathname: &str,
    ) -> (String, i32, Option<String>, Option<String>) {
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
    /// BOTH the minting migration (20260909000025) and the LIVE carrier of the template and
    /// the guest-naming outcomes (20260911000010): every `kb_blobs` outcome the act can
    /// emit must start with a prefix this file pins. (Mirrors
    /// `payload_schema::the_migration_literal_matches_the_committed_fixture`.)
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

        // The LIVE carrier (20260911000000's home-pure rewrite moved the walk; 20260911000010
        // carries it today and minted the guest-naming outcomes). A future rewrite that
        // moves the template or rewords the guest prefix away from a pinned shape fails
        // HERE, not in a fence drain.
        let carrier_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../migrations/20260911000010_blob_arm_names_guest_rows.sql"
        );
        let carrier =
            std::fs::read_to_string(carrier_path).expect("the live carrier migration exists");
        assert!(
            carrier.contains(template),
            "the strike-outcome template drifted in the live carrier — the fence parses by \
             exact prefix, so the Rust constants and this literal MUST move together"
        );
        assert!(
            carrier.contains(&format!(
                "'outcome', '{OBLIGATION_OUTCOME_PREFIX}committed by a guest of the erased principal"
            )),
            "the guest-naming outcomes must carry the independent_obligation prefix the \
             fence classifies as a known non-delete-target"
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
}
