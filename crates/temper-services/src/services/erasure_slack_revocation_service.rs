//! A person erasure revokes the subject's Slack grants at Slack, and retries the ones that fail.
//!
//! Ruled 2026-10-10 on task 01a125a1-7b5b-75ee-aafb-3046b16d532b. The SQL act destroys temper's
//! copies of the grants; it cannot reach Slack. So the service revokes each grant first, while the
//! ciphertext still exists — Slack's revoke endpoint takes the refresh token itself — through the
//! same [`revoke_at_idp`] a disconnect runs.
//!
//! **A failed revoke is retried, and that is a deliberate exception.** A disconnect destroys the
//! token whatever Slack says and never keeps it to retry, because keeping it preserves the secret
//! the user asked to destroy (`slack_disconnect_service`). For an erasure the ruling is the other
//! way: a person who is being erased should not leave a grant live at Slack because Slack was down
//! for a minute. The failed token is re-sealed under the queue row's own id, beside no Slack id and
//! no profile, in the SAME transaction as the act — a failed act queues nothing — and the erasure
//! drain tick retries it with backoff until Slack confirms or [`GIVE_UP_AFTER`] passes. Either way
//! the secret columns are emptied and only the outcome stays.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::erasure::SlackRevocationTally;
use temper_core::types::slack::IdpRevocation;

use super::grant_crypto::VaultKey;
use super::slack_disconnect_service::{revoke_at_idp, IdpRevokeTarget};
use super::slack_grant_vault_service;
use crate::error::ApiResult;

/// How long a failed revoke is retried before it is abandoned: seven days. A Slack outage lasts
/// hours, not days; past a week the retries only keep a secret alive.
pub const GIVE_UP_AFTER_SECONDS: f64 = 604_800.0;

/// The longest wait between two attempts: six hours. The wait doubles from one minute.
const MAX_BACKOFF_SECONDS: f64 = 21_600.0;

/// Rows one drain tick attempts. Each is an HTTP call made while its row is locked.
const BATCH_ROWS: i64 = 25;

/// The key version a queued token is sealed under. The vault seals at version 1 (`store_grant`);
/// a key rotation would move both together.
const KEY_VERSION: i16 = 1;

/// The vault key and the revoke endpoint: what revoking a Slack grant needs. `None` at a call site
/// means Slack linking is not configured, so no grant can be opened.
#[derive(Debug, Clone, Copy)]
pub struct SlackRevokeConfig<'a> {
    pub key: &'a VaultKey,
    pub target: IdpRevokeTarget<'a>,
}

/// Grants whose revoke failed before the act, still in hand, to be queued in the act's
/// transaction. Never `Debug`: it holds tokens.
pub struct PendingRevocations {
    key: Option<VaultKey>,
    tokens: Vec<String>,
}

/// Revoke every grant the act will destroy, before it runs. Returns the tally so far (`queued`
/// counts the failures this hands back) and the failures, which [`queue_pending`] seals into the
/// act's transaction.
pub async fn revoke_before_erasure(
    pool: &PgPool,
    subject: Uuid,
    cfg: Option<&SlackRevokeConfig<'_>>,
) -> ApiResult<(SlackRevocationTally, PendingRevocations)> {
    let mut tally = SlackRevocationTally::default();
    let Some(cfg) = cfg else {
        // No vault key: nothing can be opened, so nothing can be revoked. The act still destroys
        // the rows; the operator is told how many.
        let n: i64 = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM kb_slack_grant_vault
                WHERE slack_principal_id IN (SELECT _erasure_subject_slack_principals($1))"#,
            subject
        )
        .fetch_one(pool)
        .await?;
        tally.not_attempted = u32::try_from(n).unwrap_or(u32::MAX);
        return Ok((
            tally,
            PendingRevocations {
                key: None,
                tokens: Vec::new(),
            },
        ));
    };

    let mut conn = pool.acquire().await?;
    let grants = slack_grant_vault_service::grants_for_erasure(&mut conn, cfg.key, subject).await?;
    let mut tokens = Vec::new();
    for grant in grants {
        let Some(rt) = grant.refresh_token else {
            tally.not_attempted += 1;
            continue;
        };
        match revoke_at_idp(&mut conn, &rt, &cfg.target, &grant.slack_principal_id).await? {
            IdpRevocation::Revoked => tally.revoked += 1,
            // `revoke_at_idp` never answers NotAttempted; count it as a failure if it ever does.
            IdpRevocation::Failed | IdpRevocation::NotAttempted => {
                tally.queued += 1;
                tokens.push(rt);
            }
        }
    }
    Ok((
        tally,
        PendingRevocations {
            key: Some(cfg.key.clone()),
            tokens,
        },
    ))
}

/// Seal each failed grant into `kb_erasure_slack_revocations`, in the act's transaction.
///
/// A plain `sqlx` error, so the act's retry loop judges a failure here as it judges the act's own.
pub async fn queue_pending(
    conn: &mut sqlx::PgConnection,
    pending: &PendingRevocations,
) -> Result<(), sqlx::Error> {
    let Some(key) = &pending.key else {
        return Ok(());
    };
    for rt in &pending.tokens {
        let id = Uuid::now_v7();
        let (nonce, ciphertext) = slack_grant_vault_service::seal_for_revocation(key, id, rt);
        sqlx::query!(
            r#"INSERT INTO kb_erasure_slack_revocations
                   (id, key_version, rt_nonce, rt_ciphertext, give_up_at)
               VALUES ($1, $2, $3, $4, now() + make_interval(secs => $5))"#,
            id,
            KEY_VERSION,
            nonce,
            ciphertext,
            GIVE_UP_AFTER_SECONDS,
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// What one drain tick did with the queued revocations.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SlackRevocationDrain {
    /// Revokes attempted this tick.
    pub attempted: u32,
    /// Confirmed revoked, and emptied.
    pub revoked: u32,
    /// Failed again; retried at a later tick.
    pub retrying: u32,
    /// Emptied without a confirmed revoke: past the retry window, unopenable, or found with Slack
    /// not configured past the window.
    pub abandoned: u32,
}

/// One drain tick over the queue. Rows past their window are abandoned first, whether or not
/// Slack is configured — the secret must not outlive the window because a deployment lost its
/// configuration. Then, with Slack configured, the due rows are attempted.
pub async fn drain(
    pool: &PgPool,
    cfg: Option<&SlackRevokeConfig<'_>>,
) -> ApiResult<SlackRevocationDrain> {
    let mut out = SlackRevocationDrain::default();
    let expired = sqlx::query!(
        r#"UPDATE kb_erasure_slack_revocations
              SET key_version = NULL, rt_nonce = NULL, rt_ciphertext = NULL,
                  resolved_at = now(), outcome = 'abandoned'
            WHERE resolved_at IS NULL AND give_up_at <= now()"#
    )
    .execute(pool)
    .await?
    .rows_affected();
    if expired > 0 {
        tracing::warn!(
            abandoned = expired,
            "erasure drain: Slack revocations abandoned after their retry window. The grants may \
             stay live at Slack until they expire."
        );
    }
    out.abandoned = u32::try_from(expired).unwrap_or(u32::MAX);

    let Some(cfg) = cfg else {
        return Ok(out);
    };

    let mut tx = pool.begin().await?;
    let due = sqlx::query!(
        r#"SELECT id, attempts, rt_nonce AS "rt_nonce!", rt_ciphertext AS "rt_ciphertext!"
             FROM kb_erasure_slack_revocations
            WHERE resolved_at IS NULL AND next_attempt_at <= now()
            ORDER BY next_attempt_at
            LIMIT $1
              FOR UPDATE SKIP LOCKED"#,
        BATCH_ROWS
    )
    .fetch_all(&mut *tx)
    .await?;
    for row in due {
        let label = row.id.to_string();
        let verdict = match slack_grant_vault_service::open_for_revocation(
            cfg.key,
            row.id,
            &row.rt_nonce,
            &row.rt_ciphertext,
        ) {
            Ok(rt) => {
                out.attempted += 1;
                Some(revoke_at_idp(&mut tx, &rt, &cfg.target, &label).await?)
            }
            Err(_) => {
                tracing::warn!(
                    row = %label,
                    "erasure drain: a queued Slack revocation could not be opened; abandoned"
                );
                None
            }
        };
        match verdict {
            Some(IdpRevocation::Revoked) => {
                resolve(&mut tx, row.id, "revoked").await?;
                out.revoked += 1;
            }
            Some(IdpRevocation::Failed | IdpRevocation::NotAttempted) => {
                // One minute, doubling, capped.
                let wait =
                    (60.0 * 2f64.powi(row.attempts.clamp(0, 20))).min(MAX_BACKOFF_SECONDS);
                sqlx::query!(
                    r#"UPDATE kb_erasure_slack_revocations
                          SET attempts = attempts + 1,
                              next_attempt_at = now() + make_interval(secs => $2)
                        WHERE id = $1"#,
                    row.id,
                    wait
                )
                .execute(&mut *tx)
                .await?;
                out.retrying += 1;
            }
            None => {
                resolve(&mut tx, row.id, "abandoned").await?;
                out.abandoned += 1;
            }
        }
    }
    tx.commit().await?;
    Ok(out)
}

/// Empty a queued row's secret and record how it ended.
async fn resolve(conn: &mut sqlx::PgConnection, id: Uuid, outcome: &str) -> ApiResult<()> {
    sqlx::query!(
        r#"UPDATE kb_erasure_slack_revocations
              SET key_version = NULL, rt_nonce = NULL, rt_ciphertext = NULL,
                  resolved_at = now(), outcome = $2
            WHERE id = $1"#,
        id,
        outcome
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}
