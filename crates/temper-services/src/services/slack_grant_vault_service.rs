//! The Slack grant vault's persistence: seal a per-user grant, mint access tokens from it,
//! and read it back for disconnect to destroy. All SQL for T3 lives here; the callback handler
//! dispatches and never touches the database or the cipher.
//!
//! The vault holds each linked Slack user's OWN refresh token — the independent grant family
//! T2's `offline_access` consent minted, never an export of that user's local CLI grant. The RT
//! (and a cached access token) are stored as XChaCha20-Poly1305 ciphertext via
//! [`grant_crypto`](super::grant_crypto); the database never sees plaintext or the key.
//!
//! Each row also records the fingerprint of the key that sealed it (`key_fingerprint`). It is what
//! lets [`mint_access_token`] tell a misconfigured key (restore it, and every row heals) apart from
//! a deliberately retired key or a bad row (only a re-link heals those). See `Unopenable`.

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::slack::LinkRefusal;

use super::grant_crypto::{CryptoError, KeyFingerprint, VaultKey};
use super::slack_link_state::{resolve, LinkEvidence};
use crate::error::{ApiError, ApiResult};
use crate::oauth_client;

/// Refresh a cached access token once it is within this window of expiry. Matches temper-client's
/// `needs_refresh` (`auth.rs`): a token that outlives the mention it is minted for is the point.
const AT_REFRESH_SKEW: Duration = Duration::minutes(5);

/// Fallback access-token lifetime when the IdP omits `expires_in`. Mirrors temper-client's
/// refresh default; Auth0 always sends one, so this only guards a spec-legal omission.
const DEFAULT_AT_TTL_SECS: u64 = 3600;

/// Ceiling on a provider-supplied `expires_in`, in seconds (24h). A CLAMP, not a validation.
///
/// `expires_in` is an unbounded `u64` off the wire, and it used to be cast straight to `i64` for
/// `Duration::seconds`. A hostile or broken IdP answering with a huge value wraps that cast
/// NEGATIVE, so `access_expires_at` lands in the PAST, the cached-token branch of
/// [`mint_access_token`] never takes, and every single mention spends a refresh — which maximizes
/// exposure to the documented RT-rotation dual-write window (see that function's docs) on every
/// mention rather than once a day.
///
/// Clamping rather than erroring is deliberate: a too-long TTL is not a reason to refuse a
/// mint, and treating the cache as shorter-lived than the provider claims is always safe (the
/// worst case is one extra refresh). 24h matches the link exchange's own `expires_in`, so no
/// real Auth0/AS response is affected.
const MAX_AT_TTL_SECS: u64 = 86_400;

/// Turn a provider-supplied `expires_in` into a cache lifetime that cannot be negative.
///
/// After [`MAX_AT_TTL_SECS`] the `as i64` is provably lossless — the value is at most 86_400.
fn access_ttl(expires_in: Option<u64>) -> Duration {
    Duration::seconds(
        expires_in
            .unwrap_or(DEFAULT_AT_TTL_SECS)
            .min(MAX_AT_TTL_SECS) as i64,
    )
}

/// What a mint produced. A typed outcome, not `Option<String>`: "no grant on file" and "the
/// grant was revoked" are different facts the caller (T4) must be able to tell apart and say
/// something specific about, and neither is an error.
///
/// `Debug` is hand-written to REDACT the token: `MintOutcome::Token` wraps a live, presentable
/// access token, and this is the exact value the mention path handles — a stray `?outcome` in a
/// log would otherwise write an act-as-the-human credential to disk.
#[derive(Clone, PartialEq, Eq)]
pub enum MintOutcome {
    /// A valid access token the caller may present as the linked human, with the absolute instant
    /// it expires.
    ///
    /// `expires_at` is carried because the consumer is a token *cache*: eve's `TokenResult` takes
    /// an optional `expiresAt` and refreshes proactively when it has one. Without it the runtime
    /// caches until something 401s, which turns every expiry into a user-visible failed turn
    /// before the retry. It is the same instant stored on the vault row, not a re-derivation.
    Token {
        access_token: String,
        expires_at: DateTime<Utc>,
    },
    /// No token was minted, and the typed reason why — carried verbatim from
    /// [`super::slack_link_state::resolve`], or [`LinkRefusal::NotVaulted`] when the IdP refused
    /// the vaulted grant as `invalid_grant` (see [`mint_access_token`]). This replaces the former `Revoked` / `NotVaulted`
    /// pair, which collapsed seven distinct facts (six standing reasons plus a dead soft-revoke
    /// flag) into two arms and shipped a re-link remedy that could not work for a standing refusal.
    /// (A live token handed out earlier still survives to its own `exp` — honest revocation
    /// semantics are unchanged.)
    Refused(LinkRefusal),
}

impl std::fmt::Debug for MintOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Token { expires_at, .. } => write!(f, "Token(redacted, expires_at={expires_at})"),
            // `LinkRefusal` carries no secret, so its derived `Debug` is safe to surface.
            Self::Refused(reason) => write!(f, "Refused({reason:?})"),
        }
    }
}

/// A freshly obtained grant to seal, straight off the callback exchange. A params struct rather
/// than a long argument list: five same-domain values whose order (two opaque token strings side
/// by side) is easy to transpose at the call site — the named fields make that impossible.
///
/// `Debug` is hand-written to REDACT the two token fields: this struct carries the plaintext
/// refresh and access tokens, so a derived `Debug` + a stray `?grant` would dump the durable
/// grant to a log.
pub struct NewGrant<'a> {
    /// The person the grant is vaulted for: a machine never holds a Slack grant.
    pub owner: &'a crate::auth::HumanPrincipal,
    /// The WHOLE opaque principal (`slack:<team>:<user>`), never split.
    pub slack_principal_id: &'a str,
    /// The refresh token — the durable grant.
    pub refresh_token: &'a str,
    /// The access token from the same exchange, cached so the first mention spends no refresh.
    pub access_token: &'a str,
    /// The exchange's `expires_in`. `None` falls back to `DEFAULT_AT_TTL_SECS`.
    pub access_ttl_secs: Option<u64>,
}

impl std::fmt::Debug for NewGrant<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewGrant")
            .field("profile_id", &self.owner.profile().id)
            .field("slack_principal_id", &self.slack_principal_id)
            .field("refresh_token", &"redacted")
            .field("access_token", &"redacted")
            .field("access_ttl_secs", &self.access_ttl_secs)
            .finish()
    }
}

/// Associated data binding a sealed secret to its principal and field. It is covered by the AEAD
/// tag (authenticated, not encrypted), so a valid ciphertext transplanted into another row or the
/// other field fails to open — see [`grant_crypto`](super::grant_crypto). The NUL separator keeps
/// `principal || field` unambiguous; Slack principals never contain NUL.
fn aad(slack_principal_id: &str, field: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(slack_principal_id.len() + 1 + field.len());
    v.extend_from_slice(slack_principal_id.as_bytes());
    v.push(0);
    v.extend_from_slice(field);
    v
}

/// Field tags for [`aad`] — distinguish the refresh-token column from the access-token column so
/// one cannot be opened in place of the other.
const FIELD_RT: &[u8] = b"rt";
const FIELD_AT: &[u8] = b"at";

/// The `SLACK_VAULT_RETIRED_KEY_FINGERPRINTS` entry that retires legacy rows: those written before
/// `key_fingerprint` existed and never opened since, so the vault cannot say which key sealed them.
const UNSTAMPED: &str = "unstamped";

/// Keys an operator has deliberately rotated out of the vault, named by [`KeyFingerprint`]. This is
/// how a deliberate rotation is told apart from a misconfigured key: a row sealed under a retired key
/// that will not open is a `NotVaulted` refusal (the user gets a re-link URL), while a row sealed
/// under any other key the deployment does not hold stays a 500.
///
/// Retiring a key is an explicit act because the two cases fail identically, and guessing wrong in
/// the generous direction is what PR #1120 reverted: with a wrong key deployed, each re-link would
/// overwrite a row that restoring the key would have healed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetiredKeys {
    fingerprints: Vec<KeyFingerprint>,
    /// Whether legacy rows (`key_fingerprint IS NULL`) that do not open count as retired.
    unstamped: bool,
}

impl RetiredKeys {
    /// Parse the comma-separated env value: 16-hex-character fingerprints, and optionally the word
    /// `unstamped`. Blank entries are skipped. An entry naming `current` (the configured key) is
    /// dropped with a warning: a row that opens is used before any fingerprint is consulted, so it
    /// would never take effect, and keeping it would only mislead the next reader.
    ///
    /// The error never echoes an entry. An operator who pasted the key itself here by mistake must
    /// not have it written to the log.
    pub fn parse(raw: &str, current: KeyFingerprint) -> Result<Self, String> {
        let mut out = Self::default();
        for (i, entry) in raw.split(',').map(str::trim).enumerate() {
            if entry.is_empty() {
                continue;
            }
            if entry.eq_ignore_ascii_case(UNSTAMPED) {
                out.unstamped = true;
                continue;
            }
            let fp = KeyFingerprint::from_hex(entry).ok_or_else(|| {
                format!(
                    "entry {} is neither a 16-hex-character fingerprint nor `{UNSTAMPED}`",
                    i + 1
                )
            })?;
            if fp == current {
                tracing::warn!(
                    key_fingerprint = %fp,
                    "SLACK_VAULT_RETIRED_KEY_FINGERPRINTS names the configured SLACK_VAULT_ENC_KEY; \
                     that entry is ignored"
                );
                continue;
            }
            if !out.fingerprints.contains(&fp) {
                out.fingerprints.push(fp);
            }
        }
        Ok(out)
    }

    /// Whether a row stamped `stored` (or unstamped, `None`) was sealed under a retired key.
    fn retires(&self, stored: Option<KeyFingerprint>) -> bool {
        match stored {
            Some(fp) => self.fingerprints.contains(&fp),
            None => self.unstamped,
        }
    }
}

/// Why a stored secret could not be turned into a token.
#[derive(Debug)]
enum OpenFailure {
    /// The AEAD open failed: a different key, a tampered byte, a mismatched `aad` (a transplanted
    /// ciphertext), or a malformed nonce. AEAD cannot say which.
    Sealed(CryptoError),
    /// The open SUCCEEDED, so the tag verified, but the plaintext is not UTF-8 (tokens are ASCII).
    NotUtf8,
}

/// Open a stored `(nonce, ciphertext)` pair to a UTF-8 token string, keeping the two failures apart.
fn open_secret(
    key: &VaultKey,
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<String, OpenFailure> {
    let bytes = key
        .decrypt(nonce, ciphertext, aad)
        .map_err(OpenFailure::Sealed)?;
    String::from_utf8(bytes).map_err(|_| OpenFailure::NotUtf8)
}

/// What the mint makes of a vault row that would not open. Decided by [`classify`] from the failure
/// and the row's `key_fingerprint`; [`unopenable_outcome`] turns it into the answer.
///
/// | Failure | Stored fingerprint | Outcome |
/// |---|---|---|
/// | opened, not UTF-8 | any | [`Unopenable::NotUtf8`]: 500 |
/// | tag or nonce | the configured key's | [`Unopenable::RowFault`]: `NotVaulted` |
/// | tag or nonce | retired | [`Unopenable::Retired`]: `NotVaulted` |
/// | tag or nonce | anything else, or NULL not retired | [`Unopenable::UnknownKey`]: 500 |
#[derive(Debug, PartialEq, Eq)]
enum Unopenable {
    /// The key opened it, so something holding the key sealed bytes that are not a token: a sealing
    /// bug or a compromised key. Never a refusal: a re-link would hide it.
    NotUtf8,
    /// Sealed under the configured key, yet it will not open: the row was tampered with or
    /// transplanted, or a bug sealed it wrong. No key change heals it, so a re-link destroys nothing
    /// recoverable and is the user's only way back. A pinned seal-format test guards the one bug
    /// that would put every row here (a change to how rows are opened).
    RowFault,
    /// Sealed under a key the operator retired in `SLACK_VAULT_RETIRED_KEY_FINGERPRINTS`.
    Retired,
    /// Sealed under a key this deployment neither holds nor has retired, or a legacy row whose key
    /// is unknown. Most likely `SLACK_VAULT_ENC_KEY` is wrong, and restoring it heals every such
    /// row, so the row must not be overwritten: a 500, and nothing is written.
    UnknownKey,
}

fn classify(
    failure: &OpenFailure,
    stored: Option<KeyFingerprint>,
    current: KeyFingerprint,
    retired: &RetiredKeys,
) -> Unopenable {
    match failure {
        OpenFailure::NotUtf8 => Unopenable::NotUtf8,
        OpenFailure::Sealed(_) if stored == Some(current) => Unopenable::RowFault,
        OpenFailure::Sealed(_) if retired.retires(stored) => Unopenable::Retired,
        OpenFailure::Sealed(_) => Unopenable::UnknownKey,
    }
}

/// A stored fingerprint for a log line: its hex, or `unstamped`.
fn sealed_by(stored: Option<KeyFingerprint>) -> String {
    stored.map_or_else(|| UNSTAMPED.to_string(), |fp| fp.to_string())
}

/// The mint's answer for an [`Unopenable`] row, logged. The caller returns it without writing, so
/// the transaction rolls back and the row's bytes are untouched. Fingerprints are not secret; the
/// ciphertext, nonce, key and error text never reach a log line. `ApiError::Internal`'s text is not
/// shown to the client either.
fn unopenable_outcome(
    kind: Unopenable,
    field: &str,
    slack_principal_id: &str,
    stored: Option<KeyFingerprint>,
    current: KeyFingerprint,
) -> ApiResult<MintOutcome> {
    let sealed_by = sealed_by(stored);
    match kind {
        Unopenable::NotUtf8 => {
            tracing::error!(
                principal = %slack_principal_id,
                field,
                sealed_by = %sealed_by,
                configured = %current,
                "slack mint: a vault grant opened under the configured key but is not a token (not \
                 UTF-8). Something holding SLACK_VAULT_ENC_KEY sealed it: a sealing bug, or a \
                 compromised key."
            );
            Err(ApiError::Internal(
                "vault: decrypted grant was not valid UTF-8".to_string(),
            ))
        }
        Unopenable::RowFault => {
            tracing::error!(
                principal = %slack_principal_id,
                field,
                sealed_by = %sealed_by,
                configured = %current,
                "slack mint: a vault grant sealed under the configured key will not open. The row \
                 was tampered with or transplanted, or a bug sealed it wrong. Answering not_vaulted \
                 so the user can re-link; investigate the row."
            );
            Ok(MintOutcome::Refused(LinkRefusal::NotVaulted))
        }
        Unopenable::Retired => {
            tracing::info!(
                principal = %slack_principal_id,
                field,
                sealed_by = %sealed_by,
                configured = %current,
                "slack mint: the vault grant was sealed under a retired key; answering not_vaulted \
                 so the user can re-link"
            );
            Ok(MintOutcome::Refused(LinkRefusal::NotVaulted))
        }
        Unopenable::UnknownKey => {
            tracing::error!(
                principal = %slack_principal_id,
                field,
                sealed_by = %sealed_by,
                configured = %current,
                "slack mint: the vault grant was sealed under key {sealed_by}, but \
                 SLACK_VAULT_ENC_KEY is {current}. Is the deployed key the one that sealed the \
                 vault? Restoring it heals every such grant. If the key was rotated on purpose, \
                 list the old fingerprint (or `unstamped`) in \
                 SLACK_VAULT_RETIRED_KEY_FINGERPRINTS. Answering 500 and leaving the row untouched."
            );
            Err(ApiError::Internal(
                "vault: the grant was sealed under a key this deployment does not hold".to_string(),
            ))
        }
    }
}

/// Record that the row opened under `key` — its first open since fingerprints existed, or a row
/// whose stored fingerprint was wrong. Does not touch `updated_at`: nothing was resealed, and the
/// startup check reads the most recently SEALED rows.
async fn stamp_fingerprint(
    conn: &mut sqlx::PgConnection,
    slack_principal_id: &str,
    fingerprint: KeyFingerprint,
) -> ApiResult<()> {
    sqlx::query!(
        "UPDATE kb_slack_grant_vault SET key_fingerprint = $2 WHERE slack_principal_id = $1",
        slack_principal_id,
        fingerprint.as_bytes(),
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Seal a freshly obtained grant for its principal. Called at T2's callback seam once the
/// directory row is written.
///
/// Upsert on the principal: re-linking (a fresh consent) replaces the stored grant with the new
/// one and **clears any prior revocation** — a new consent is a new, working grant, so a user who
/// disconnected and reconnected is not left mysteriously revoked. The principal binds to one
/// profile (T2's guarantee), so `profile_id` is re-stamped defensively but does not move.
///
/// The overwritten refresh token's plaintext is gone after this write, so nothing can revoke it
/// later. The link callback calls `slack_disconnect_service::revoke_superseded_grant` first, which
/// revokes it in AS mode and deliberately leaves it in ExternalIdp mode (see that function).
///
/// **Takes `&mut PgConnection`, not a pool, so the callback can run this in the SAME transaction
/// as [`slack_link_service::link_slack_principal`].** These two writes used to be independent
/// autocommits, and the gap between them was not benign: process death after the directory row
/// committed and before the vault row did left the user LINKED with a refresh token that existed
/// only in a dropped stack frame — unrecoverable, because the state nonce was already burned, so
/// no retry of that callback could re-derive it. The handler's own comment argued the failure was
/// benign because "the user is shown a retry page", which holds for an observable `Err` return and
/// does NOT hold for a kill between two commits, where no page renders at all. The executor type
/// is what makes that shape unrepresentable. Mirrors
/// [`take_refresh_token_locked`], which takes `&mut PgConnection` for the same reason.
///
/// [`slack_link_service::link_slack_principal`]: super::slack_link_service::link_slack_principal
pub async fn store_grant(
    conn: &mut sqlx::PgConnection,
    key: &VaultKey,
    grant: NewGrant<'_>,
) -> ApiResult<()> {
    let (rt_nonce, rt_ciphertext) = key.encrypt(
        grant.refresh_token.as_bytes(),
        &aad(grant.slack_principal_id, FIELD_RT),
    );
    let (at_nonce, at_ciphertext) = key.encrypt(
        grant.access_token.as_bytes(),
        &aad(grant.slack_principal_id, FIELD_AT),
    );
    let access_expires_at = Utc::now() + access_ttl(grant.access_ttl_secs);
    let fingerprint = key.fingerprint();

    sqlx::query!(
        r#"
        INSERT INTO kb_slack_grant_vault
            (id, profile_id, slack_principal_id, key_version, key_fingerprint,
             rt_nonce, rt_ciphertext, at_nonce, at_ciphertext, access_expires_at, revoked_at)
        VALUES ($1, $2, $3, 1, $9, $4, $5, $6, $7, $8, NULL)
        ON CONFLICT (slack_principal_id) DO UPDATE
            SET profile_id        = EXCLUDED.profile_id,
                key_version       = EXCLUDED.key_version,
                key_fingerprint   = EXCLUDED.key_fingerprint,
                rt_nonce          = EXCLUDED.rt_nonce,
                rt_ciphertext     = EXCLUDED.rt_ciphertext,
                at_nonce          = EXCLUDED.at_nonce,
                at_ciphertext     = EXCLUDED.at_ciphertext,
                access_expires_at = EXCLUDED.access_expires_at,
                revoked_at        = NULL,
                updated_at        = now()
        "#,
        Uuid::now_v7(),
        grant.owner.profile().id,
        grant.slack_principal_id,
        &rt_nonce[..],
        rt_ciphertext,
        &at_nonce[..],
        at_ciphertext,
        access_expires_at,
        fingerprint.as_bytes(),
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Mint a fresh access token for `slack_principal_id`, refreshing the grant only when the cached
/// token is near expiry.
///
/// The whole thing runs in one transaction under a `SELECT ... FOR UPDATE OF v` row lock (locking
/// the vault row only, not the joined profile). The lock serializes CONCURRENT mints of the same
/// principal: Auth0 rotates the refresh token on every refresh, so two mentions arriving at once
/// must not both spend the same stored RT — the second blocks, then reads (under READ COMMITTED's
/// EvalPlanQual re-read) the RT the first already rotated in, and finds the cache fresh so it does
/// not refresh at all.
///
/// **What the lock does NOT close** (documented, not solved here): the *dual-write* window. The RT
/// rotation is an IdP side effect that is not part of this transaction, so if the process dies or
/// the COMMIT fails AFTER Auth0 returns a rotated RT but BEFORE the `UPDATE` commits, the row keeps
/// the now-dead RT and the next mint trips reuse-detection, bricking the grant until the user
/// re-links. No row lock can make an external HTTP effect atomic with a local commit. The
/// mitigation belongs at the deployment layer: enable Auth0 refresh-token rotation **leeway** (a
/// grace window that tolerates a brief RT reuse). The recovery is re-link, and that part IS here: a
/// grant the IdP refuses as `invalid_grant` — bricked this way, or simply aged past the IdP's
/// refresh-token lifetime — reports [`LinkRefusal::NotVaulted`], whose reply tells the user to
/// reconnect, rather than an error the agent can only call transient. The row is kept, not
/// deleted; see that arm for why. See the T3 spec's "grant vault" section.
///
/// A principal whose standing does not admit them mints nothing — the decision is delegated to
/// [`super::slack_link_state::resolve`] (which calls `temper_principal::admit`), so a `deactivated`
/// / `revoked` / `denied` / absent / unrecognized standing each reports as
/// [`MintOutcome::Refused`] carrying the matching [`LinkRefusal::Standing`] reason, and the mention
/// agent names the remedy that fits (an admin approval), never the false "re-link".
///
/// `token_url` / `client_id` come from `link_provider::derive` at the call site — the same public
/// client that minted the grant. They are NOT stored on the row: the instance's config is
/// authoritative, and a config change that invalidates old grants invalidates them at the IdP too.
///
/// **A row that will not open** is classified by its `key_fingerprint` (see `Unopenable` for the
/// table). A misconfigured key is a 500 that writes nothing, so restoring the key heals every user.
/// A retired key, or a row that fails under the very key that sealed it, is `NotVaulted`, so the
/// user gets a re-link URL. A row that opens is stamped with the configured key's fingerprint if it
/// is not already: that is how a legacy (NULL) row gets one. Both ciphertexts in a row are always
/// sealed under one key in one statement, so opening either proves the key for the row.
///
/// AUTHORIZATION: this function enforces none — it mints for whatever principal it is handed. Its
/// caller (T4's mention path) MUST derive `slack_principal_id` from an HMAC-verified server-to-
/// server request whose principal comes from Slack's own verified event, never a client-supplied
/// field. Naming a principal must not be sufficient to mint its token.
pub async fn mint_access_token(
    pool: &PgPool,
    key: &VaultKey,
    retired: &RetiredKeys,
    token_url: &str,
    client_id: &str,
    slack_principal_id: &str,
) -> ApiResult<MintOutcome> {
    let mut tx = pool.begin().await?;

    // Rooted at the IDENTITY, not the vault, so a link with no vaulted grant is `NotLinked` vs
    // `NotVaulted` distinctly, and standing is reachable even when no vault row exists (spec §2.3).
    // The lock lives in the `locked` CTE because `FOR UPDATE` cannot be applied to the nullable
    // side of an outer join — verified against the live database, and its row lock proven by a
    // two-session test. `ON v.profile_id = l.profile_id` is the fail-closed correlation: if the
    // vault and link rows disagree on the owner (an account-merge skew), the vault reads as absent
    // rather than minting one profile's grant under another's standing. `vaulted` is event-carried
    // as `v.id IS NOT NULL`; every `v.*` column is nullable on the outer-join side and annotated `?`.
    let row = sqlx::query!(
        r#"
        WITH locked AS (
            SELECT * FROM kb_slack_grant_vault WHERE slack_principal_id = $1 FOR UPDATE
        )
        SELECT v.rt_nonce          AS "rt_nonce?",
               v.rt_ciphertext     AS "rt_ciphertext?",
               v.at_nonce          AS "at_nonce?",
               v.at_ciphertext     AS "at_ciphertext?",
               v.access_expires_at AS "access_expires_at?",
               v.key_fingerprint   AS "key_fingerprint?",
               (v.id IS NOT NULL)  AS "vaulted!",
               s.state             AS "standing?"
          FROM kb_profile_auth_links l
          LEFT JOIN locked v                ON v.profile_id = l.profile_id
          LEFT JOIN kb_principal_standing s ON s.profile_id = l.profile_id
         WHERE l.auth_provider = 'slack' AND l.auth_provider_user_id = $1
        "#,
        slack_principal_id,
    )
    .fetch_optional(&mut *tx)
    .await?;

    // No link row at all → not linked. The vault is unreachable without a link, so this is the one
    // refusal the query answers purely by absence.
    let Some(row) = row else {
        return Ok(MintOutcome::Refused(LinkRefusal::NotLinked));
    };

    // Decide before any decrypt or RT spend (auth before writes). `resolve` delegates standing to
    // `admit` and orders NotLinked → Standing → NotVaulted, so a refused human is told the remedy
    // that works: a standing refusal names "ask an admin", never the false "re-link". Dropping the
    // former `revoked_at.is_some()` disjunct is deliberate — soft-revoke was superseded by
    // disconnect's DELETE (commit `3a45b1ab`); the column is retained but inert.
    if let Err(refusal) = resolve(LinkEvidence {
        linked: true,
        vaulted: row.vaulted,
        standing: row.standing.as_deref(),
    }) {
        return Ok(MintOutcome::Refused(refusal));
    }

    // `resolve` returned `Ok`, so the row is vaulted and its NOT-NULL refresh columns are present;
    // a missing one is an internal inconsistency (a torn row), not a user-facing refusal.
    let (Some(rt_nonce), Some(rt_ciphertext)) = (&row.rt_nonce, &row.rt_ciphertext) else {
        return Err(ApiError::Internal(
            "a vaulted grant is missing its refresh ciphertext".to_string(),
        ));
    };

    let current = key.fingerprint();
    // A malformed stored value is unreachable (the column's CHECK) and reads as unstamped.
    let stored = row
        .key_fingerprint
        .as_deref()
        .and_then(KeyFingerprint::from_bytes);
    let needs_stamp = stored != Some(current);

    // Cached access token still comfortably valid? Hand it back — no refresh, no RT rotation.
    if let (Some(at_nonce), Some(at_ciphertext), Some(expires_at)) =
        (&row.at_nonce, &row.at_ciphertext, row.access_expires_at)
    {
        if expires_at > Utc::now() + AT_REFRESH_SKEW {
            let token = match open_secret(
                key,
                at_nonce,
                at_ciphertext,
                &aad(slack_principal_id, FIELD_AT),
            ) {
                Ok(token) => token,
                Err(failure) => {
                    let kind = classify(&failure, stored, current, retired);
                    return unopenable_outcome(kind, "at", slack_principal_id, stored, current);
                }
            };
            if needs_stamp {
                stamp_fingerprint(&mut tx, slack_principal_id, current).await?;
            }
            tx.commit().await?;
            return Ok(MintOutcome::Token {
                access_token: token,
                expires_at,
            });
        }
    }

    // The cached token is gone or stale: spend the RT, then rotate everything the response gave.
    let refresh_token = match open_secret(
        key,
        rt_nonce,
        rt_ciphertext,
        &aad(slack_principal_id, FIELD_RT),
    ) {
        Ok(token) => token,
        Err(failure) => {
            let kind = classify(&failure, stored, current, retired);
            return unopenable_outcome(kind, "rt", slack_principal_id, stored, current);
        }
    };
    // Stamped now, not only by the re-seal below, so a grant the IdP refuses (the arm below
    // commits) is still recorded as sealed under this key.
    if needs_stamp {
        stamp_fingerprint(&mut tx, slack_principal_id, current).await?;
    }
    let tokens = match oauth_client::refresh_grant(token_url, client_id, &refresh_token).await? {
        oauth_client::RefreshOutcome::Refreshed(tokens) => tokens,
        oauth_client::RefreshOutcome::GrantRejected => {
            // The IdP says the grant is dead: the RT aged out (the common case — a user who has
            // not mentioned @temper for longer than the IdP's refresh-token lifetime), was rotated
            // out by the dual-write window above, or was revoked. Retrying cannot produce a token
            // and the remedy is a fresh consent — exactly what `NotVaulted` means to the agent
            // ("retrying won't fix it; reconnect"). Not an `Err`: that is a 401 to the agent,
            // which can only say "try again", forever.
            //
            // The row is deliberately LEFT IN PLACE, not deleted. The transaction is committed only
            // for the fingerprint stamp above, the one write it can hold. `invalid_grant` is the
            // IdP's word, and it is not always about the grant: Auth0 answers it when the RT is
            // presented to the wrong client or tenant, so a misconfigured `client_id` / issuer
            // would refuse EVERY grant. (The temper AS answers a store failure during rotation with
            // `temporarily_unavailable`, a 503 that `refresh_grant` reports as an `Err`, never as
            // this arm.) Deleting would turn a config slip into permanent loss — and, worse,
            // destroy the only plaintext source of a possibly-live RT, so disconnect and erasure
            // could never revoke it. Kept, a config fix heals every user at once, and disconnect
            // still revokes before it deletes. The cost is that a truly dead RT is re-presented to
            // the IdP on each mention, which is harmless.
            // A re-link overwrites the row (`store_grant` upserts); in AS mode the callback first
            // revokes the RT it overwrites (`slack_disconnect_service::revoke_superseded_grant`).
            tx.commit().await?;
            return Ok(MintOutcome::Refused(LinkRefusal::NotVaulted));
        }
    };

    // Auth0 rotates, so the response's RT supersedes the spent one. A provider that does not
    // rotate omits it (RFC 6749 §6: keep the existing RT); keep the one we hold in that case
    // rather than blanking the grant. NOTE: against a rotating IdP a 200-without-RT is a smell —
    // the leeway mitigation above is what keeps re-storing the old RT from bricking the family.
    let new_refresh = tokens.refresh_token.as_deref().unwrap_or(&refresh_token);
    let (rt_nonce, rt_ciphertext) =
        key.encrypt(new_refresh.as_bytes(), &aad(slack_principal_id, FIELD_RT));
    let (at_nonce, at_ciphertext) = key.encrypt(
        tokens.access_token.as_bytes(),
        &aad(slack_principal_id, FIELD_AT),
    );
    let access_expires_at = Utc::now() + access_ttl(tokens.expires_in);

    sqlx::query!(
        r#"
        UPDATE kb_slack_grant_vault
           SET rt_nonce = $2, rt_ciphertext = $3,
               at_nonce = $4, at_ciphertext = $5,
               access_expires_at = $6, key_fingerprint = $7, updated_at = now()
         WHERE slack_principal_id = $1
        "#,
        slack_principal_id,
        &rt_nonce[..],
        rt_ciphertext,
        &at_nonce[..],
        at_ciphertext,
        access_expires_at,
        current.as_bytes(),
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(MintOutcome::Token {
        access_token: tokens.access_token,
        expires_at: access_expires_at,
    })
}

/// How many of the most recently sealed rows [`check_configured_key`] reads.
const KEY_CHECK_SAMPLE: i64 = 20;

/// What [`check_configured_key`] found among the most recently sealed rows.
#[derive(Debug, PartialEq, Eq)]
pub struct KeyCheck {
    /// Rows read (stamped rows only; at most `KEY_CHECK_SAMPLE`).
    pub sampled: usize,
    /// Rows sealed under the configured key.
    pub matching: usize,
    /// Fingerprints that are neither the configured key nor retired, with how many rows carry each,
    /// most common first.
    pub unknown: Vec<(KeyFingerprint, usize)>,
}

impl KeyCheck {
    fn assess(
        sample: impl IntoIterator<Item = KeyFingerprint>,
        current: KeyFingerprint,
        retired: &RetiredKeys,
    ) -> Self {
        let mut check = Self {
            sampled: 0,
            matching: 0,
            unknown: Vec::new(),
        };
        for fp in sample {
            check.sampled += 1;
            if fp == current {
                check.matching += 1;
            } else if !retired.retires(Some(fp)) {
                match check.unknown.iter_mut().find(|(seen, _)| *seen == fp) {
                    Some((_, n)) => *n += 1,
                    None => check.unknown.push((fp, 1)),
                }
            }
        }
        check.unknown.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        check
    }

    /// The configured key sealed none of the sampled rows, and something it does not know did: every
    /// mint of those grants will 500. Almost certainly the wrong `SLACK_VAULT_ENC_KEY`.
    pub fn is_mismatch(&self) -> bool {
        self.matching == 0 && !self.unknown.is_empty()
    }
}

/// Compare the configured vault key against the fingerprints of the most recently sealed rows, and
/// log loudly if it does not look like the key that sealed them. Call once per process at startup,
/// after the pool connects. It never fails boot: a query error is a warning, and the result is also
/// returned for tests.
///
/// The mint already logs an error for every grant sealed under an unknown key; this says the same
/// thing once, at boot, before any user mentions @temper, and names the fingerprints. Legacy
/// (unstamped) rows carry no evidence and are not read.
pub async fn check_configured_key(
    pool: &PgPool,
    cfg: &crate::config::SlackLinkConfig,
) -> Option<KeyCheck> {
    let current = cfg.vault_key.fingerprint();
    let sample = match sqlx::query_scalar!(
        r#"
        SELECT key_fingerprint AS "key_fingerprint!"
          FROM kb_slack_grant_vault
         WHERE key_fingerprint IS NOT NULL
         ORDER BY updated_at DESC
         LIMIT $1
        "#,
        KEY_CHECK_SAMPLE,
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "slack vault key check: could not read the vault; skipped");
            return None;
        }
    };
    let check = KeyCheck::assess(
        sample.iter().filter_map(|b| KeyFingerprint::from_bytes(b)),
        current,
        &cfg.retired_vault_keys,
    );
    let unknown = check
        .unknown
        .iter()
        .map(|(fp, n)| format!("{fp} ({n})"))
        .collect::<Vec<_>>()
        .join(", ");
    if check.is_mismatch() {
        tracing::error!(
            configured = %current,
            sampled = check.sampled,
            unknown = %unknown,
            "slack vault key check: SLACK_VAULT_ENC_KEY ({current}) sealed none of the {} most \
             recently sealed vault rows, which carry {unknown}. Every mint of those grants will \
             answer 500 until the key that sealed them is restored, or, if it was rotated on \
             purpose, its fingerprint is listed in SLACK_VAULT_RETIRED_KEY_FINGERPRINTS.",
            check.sampled,
        );
    } else if !check.unknown.is_empty() {
        tracing::warn!(
            configured = %current,
            sampled = check.sampled,
            matching = check.matching,
            unknown = %unknown,
            "slack vault key check: some recently sealed vault rows carry a key fingerprint that is \
             neither SLACK_VAULT_ENC_KEY's nor retired ({unknown}); their mints will answer 500"
        );
    }
    Some(check)
}

/// Read and decrypt the stored refresh token under a row lock, for a caller about to destroy or
/// overwrite it: disconnect (which deletes the row) and a re-link
/// (`slack_disconnect_service::revoke_superseded_grant`, before `store_grant` overwrites it).
///
/// Takes a `&mut PgConnection` so the caller can run this inside the same
/// transaction as the deletes. Locks the row (`FOR UPDATE`) so a concurrent
/// `mint_access_token` cannot rotate the RT out from under the revoke.
///
/// Returns `Ok(None)` when the principal has no vault row — a pre-T3 link, or
/// an already-disconnected principal. That is not an error: disconnect is
/// idempotent, and a missing grant simply means there is nothing to revoke.
///
/// Deliberately ignores `revoked_at`: a soft-revoked row still holds live
/// ciphertext, and disconnect's job is to destroy it.
///
/// **A decrypt failure also yields `Ok(None)`, and that is deliberate.** Every
/// other reader of the vault treats an unopenable ciphertext as an error,
/// because it is about to *use* the token. These callers are about to DESTROY or
/// overwrite it, and an unopenable ciphertext has no value to revoke —
/// so failing here would abort the transaction and delete nothing. That is
/// exactly backwards for the scenario that produces the failure: rotating
/// `SLACK_VAULT_ENC_KEY` is a documented flag-day that makes every pre-rotation
/// ciphertext unopenable, and the reason to rotate it is key compromise, i.e.
/// precisely when the unbind lever must still work. Bricking disconnect would
/// leave an unremovable stale identity row (re-link cannot rebind to another
/// profile). A genuine sqlx error still propagates — only the AEAD open is
/// downgraded.
pub async fn take_refresh_token_locked(
    tx: &mut sqlx::PgConnection,
    key: &VaultKey,
    slack_principal_id: &str,
) -> ApiResult<Option<String>> {
    let row = sqlx::query!(
        r#"
        SELECT rt_nonce, rt_ciphertext, key_fingerprint
          FROM kb_slack_grant_vault
         WHERE slack_principal_id = $1
         FOR UPDATE
        "#,
        slack_principal_id
    )
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    match decrypt_to_string(
        key,
        &row.rt_nonce,
        &row.rt_ciphertext,
        &aad(slack_principal_id, FIELD_RT),
    ) {
        Ok(rt) => Ok(Some(rt)),
        Err(_) => {
            // Principal only. NEVER the ciphertext, the nonce or the key — and
            // never the underlying error text either: `ApiError::Internal` is
            // echoed verbatim to clients, and this branch's inputs are secret
            // material.
            // The fingerprints say which case it was: `sealed_by` equal to `configured` is a bad
            // row; anything else is a rotated or misconfigured key.
            tracing::warn!(
                principal = %slack_principal_id,
                sealed_by = %sealed_by(row.key_fingerprint.as_deref().and_then(KeyFingerprint::from_bytes)),
                configured = %key.fingerprint(),
                "slack grant: the stored grant could not be opened (key rotated, or the row was \
                 tampered with), so it cannot be revoked. The caller (a disconnect or a re-link) \
                 destroys or overwrites it anyway. Revoke at the IdP out-of-band if that matters."
            );
            Ok(None)
        }
    }
}

/// One of an erasure subject's vault grants, opened for revocation. `refresh_token` is `None` when
/// the row could not be opened (key rotated, row tampered): there is nothing usable to revoke.
pub struct SubjectGrant {
    pub slack_principal_id: String,
    pub refresh_token: Option<String>,
}

impl std::fmt::Debug for SubjectGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubjectGrant")
            .field("slack_principal_id", &self.slack_principal_id)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "redacted"),
            )
            .finish()
    }
}

/// Every vault grant a person erasure reaches (`_erasure_subject_slack_principals`: through a Slack
/// auth link or the row's own `profile_id`), opened while the ciphertext still exists — the act
/// deletes the rows. Read-only: the act, not this, destroys them.
pub async fn grants_for_erasure(
    conn: &mut sqlx::PgConnection,
    key: &VaultKey,
    subject: Uuid,
) -> ApiResult<Vec<SubjectGrant>> {
    let rows = sqlx::query!(
        r#"
        SELECT slack_principal_id, rt_nonce, rt_ciphertext, key_fingerprint
          FROM kb_slack_grant_vault
         WHERE slack_principal_id IN (SELECT _erasure_subject_slack_principals($1))
         ORDER BY slack_principal_id
        "#,
        subject
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let refresh_token = decrypt_to_string(
                key,
                &r.rt_nonce,
                &r.rt_ciphertext,
                &aad(&r.slack_principal_id, FIELD_RT),
            )
            .ok();
            if refresh_token.is_none() {
                // No principal in the line: the erasure is removing it. The fingerprints are not
                // identifying, and say whether the key or the row was at fault.
                tracing::warn!(
                    sealed_by = %sealed_by(r.key_fingerprint.as_deref().and_then(KeyFingerprint::from_bytes)),
                    configured = %key.fingerprint(),
                    "erasure: a Slack grant could not be opened (key rotated, or the row was \
                     tampered with). It is destroyed with the subject's other stores and cannot \
                     be revoked at Slack."
                );
            }
            SubjectGrant {
                slack_principal_id: r.slack_principal_id,
                refresh_token,
            }
        })
        .collect())
}

/// Associated data for a token queued for revocation after an erasure: bound to the queue row's
/// own id, because the row holds no Slack principal to bind it to.
fn revocation_aad(row: Uuid) -> Vec<u8> {
    let mut v = b"erasure-revocation".to_vec();
    v.push(0);
    v.extend_from_slice(row.as_bytes());
    v
}

/// Seal a refresh token for `kb_erasure_slack_revocations` row `row`.
pub fn seal_for_revocation(key: &VaultKey, row: Uuid, refresh_token: &str) -> (Vec<u8>, Vec<u8>) {
    let (nonce, ciphertext) = key.encrypt(refresh_token.as_bytes(), &revocation_aad(row));
    (nonce.to_vec(), ciphertext)
}

/// Open a token sealed by [`seal_for_revocation`] for the same row.
pub fn open_for_revocation(
    key: &VaultKey,
    row: Uuid,
    nonce: &[u8],
    ciphertext: &[u8],
) -> ApiResult<String> {
    decrypt_to_string(key, nonce, ciphertext, &revocation_aad(row))
}

/// [`open_secret`] for a caller that does not classify the failure: either way the secret is
/// unusable, so it is an internal error rather than a token we can't trust. The mint classifies
/// instead (see [`Unopenable`]); disconnect and erasure downgrade the error to "nothing to revoke".
fn decrypt_to_string(
    key: &VaultKey,
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
) -> ApiResult<String> {
    open_secret(key, nonce, ciphertext, aad).map_err(|failure| match failure {
        OpenFailure::Sealed(e) => ApiError::Internal(format!("vault: {e}")),
        OpenFailure::NotUtf8 => {
            ApiError::Internal("vault: decrypted grant was not valid UTF-8".to_string())
        }
    })
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::super::grant_crypto::VaultKey;
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use temper_principal::Refusal;

    const PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";

    fn key() -> VaultKey {
        VaultKey::from_base64(&STANDARD.encode([3u8; 32])).unwrap()
    }

    /// Minimal profile insert — this suite tests the vault, not provisioning. The handle is the
    /// FULL id: two UUIDv7s minted in the same millisecond share their first bytes, so a truncated
    /// handle collides on `kb_profiles_handle_key` when a test inserts two profiles.
    async fn insert_profile(pool: &PgPool) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)",
            id,
            format!("user-{id}"),
        )
        .execute(pool)
        .await
        .unwrap();
        // Mint is gated on `approved` standing (D-A), so a mintable fixture is an approved one.
        set_standing(pool, id, "approved").await;
        id
    }

    /// Upsert a principal's standing directly — the test-side analogue of the state machine, used
    /// to drive the mint gate across `approved`/`revoked`/`deactivated` without an admin actor.
    async fn set_standing(pool: &PgPool, profile: Uuid, state: &str) {
        sqlx::query(
            "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,$2) \
             ON CONFLICT (profile_id) DO UPDATE SET state = EXCLUDED.state",
        )
        .bind(profile)
        .bind(state)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Seal a grant under the test key AND write the identity row that production always writes
    /// alongside it. Wraps the [`NewGrant`] construction the tests would otherwise repeat.
    ///
    /// The link row is load-bearing since the re-root (spec §2.3): `mint_access_token` now roots at
    /// `kb_profile_auth_links`, so a vault row with no link reads as `NotLinked`. A real grant is
    /// never linkless — the callback writes both in one transaction — so a faithful fixture links
    /// and vaults the same profile+principal (which also keeps them correlated, never skewed).
    ///
    /// `store_grant` takes `&mut PgConnection` (so the link callback can run it in the same
    /// transaction as the directory write); these tests need no such atomicity, so they hand it a
    /// one-off pooled connection.
    async fn store(
        pool: &PgPool,
        profile: Uuid,
        principal: &str,
        rt: &str,
        at: &str,
        ttl: Option<u64>,
    ) {
        store_sealed_by(pool, &key(), profile, principal, rt, at, ttl).await;
    }

    /// [`store`], sealed under a given key: a deployment that held a different key at link time.
    async fn store_sealed_by(
        pool: &PgPool,
        sealing_key: &VaultKey,
        profile: Uuid,
        principal: &str,
        rt: &str,
        at: &str,
        ttl: Option<u64>,
    ) {
        link(pool, profile, principal).await;
        let owner = crate::test_support::human_principal_for(pool, profile).await;
        let mut conn = pool.acquire().await.expect("acquire");
        store_grant(
            &mut conn,
            sealing_key,
            NewGrant {
                owner: &owner,
                slack_principal_id: principal,
                refresh_token: rt,
                access_token: at,
                access_ttl_secs: ttl,
            },
        )
        .await
        .unwrap();
    }

    /// Write the `kb_profile_auth_links` directory row binding a Slack principal to a profile —
    /// the identity fact the re-rooted mint query starts from. Idempotent, so a test may link
    /// without vaulting (to exercise the `NotVaulted` arm) and `store` may call it unconditionally.
    async fn link(pool: &PgPool, profile: Uuid, principal: &str) {
        sqlx::query(
            "INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id) \
             VALUES ($1, $2, 'slack', $3) \
             ON CONFLICT (auth_provider, auth_provider_user_id) DO NOTHING",
        )
        .bind(Uuid::now_v7())
        .bind(profile)
        .bind(principal)
        .execute(pool)
        .await
        .unwrap();
    }

    /// The stored refresh token is ENCRYPTED, not plaintext: the raw bytea column must not equal
    /// the token. This is the whole "encrypted at rest" claim, asserted against the real column.
    #[sqlx::test(migrations = "../../migrations")]
    async fn store_seals_the_refresh_token_at_rest(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(
            &pool,
            profile,
            PRINCIPAL,
            "the-refresh-token",
            "the-at",
            Some(3600),
        )
        .await;

        let stored: Vec<u8> = sqlx::query_scalar!(
            "SELECT rt_ciphertext FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
            PRINCIPAL,
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_ne!(
            stored, b"the-refresh-token",
            "the refresh token must be stored encrypted, never as plaintext bytes",
        );
    }

    /// A mint while the cached access token is still valid returns that token WITHOUT a refresh —
    /// no network. This exercises the FOR UPDATE + decrypt-cached + return path end to end.
    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_returns_the_cached_token_without_refreshing(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(
            &pool,
            profile,
            PRINCIPAL,
            "rt",
            "cached-access-token",
            Some(3600),
        )
        .await;

        // token_url is deliberately unreachable: a passing test proves no refresh was attempted.
        let out = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1/unused",
            "client",
            PRINCIPAL,
        )
        .await
        .unwrap();
        // Assert the token AND that the expiry rides along pointing at the future. The expiry is
        // what the caller hands eve as `TokenResult.expiresAt`; a zero or past value would still
        // be "a token" and would still pass a token-only assertion, while making the runtime
        // refresh on every single call.
        match out {
            MintOutcome::Token {
                access_token,
                expires_at,
            } => {
                assert_eq!(access_token, "cached-access-token");
                assert!(
                    expires_at > Utc::now(),
                    "the cached mint must report a future expiry, got {expires_at}",
                );
            }
            other => panic!("expected the cached token, got {other:?}"),
        }
    }

    /// **The `expires_in` cast must not wrap.** A hostile or broken IdP answering with a huge
    /// `expires_in` used to be cast `as i64` unchecked, wrapping NEGATIVE — so `access_expires_at`
    /// landed in the PAST, the cached-token branch never took, and every mint spent a refresh,
    /// maximizing exposure to the documented RT-rotation dual-write window.
    ///
    /// **Why this bites:** `u64::MAX as i64` is `-1`, so without the clamp the stored expiry is one
    /// second ago and the cached branch is skipped — mint then tries to refresh against a
    /// deliberately unreachable `token_url` and the call ERRORS. Only the clamped version can hand
    /// back the cached token. The stored expiry is asserted directly too, because a future change
    /// that made mint tolerate a refresh failure would otherwise mask the wrap.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_huge_expires_in_is_clamped_rather_than_wrapping_negative(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(
            &pool,
            profile,
            PRINCIPAL,
            "rt",
            "cached-access-token",
            Some(u64::MAX),
        )
        .await;

        let stored: DateTime<Utc> = sqlx::query_scalar!(
            "SELECT access_expires_at FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
            PRINCIPAL,
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .expect("access_expires_at is written by store_grant");
        assert!(
            stored > Utc::now(),
            "a huge expires_in must clamp to a FUTURE expiry, not wrap to the past: {stored}",
        );

        // Unreachable token_url: a passing test proves the cached branch took, i.e. no refresh.
        let out = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1/unused",
            "client",
            PRINCIPAL,
        )
        .await
        .expect("a clamped expiry must serve the cached token without refreshing");
        match out {
            MintOutcome::Token { access_token, .. } => {
                assert_eq!(access_token, "cached-access-token");
            }
            other => panic!("expected the cached token, got {other:?}"),
        }
    }

    /// **FINDING C, the service half.** `store_grant` writes on the CALLER's connection, so it
    /// shares that transaction's fate.
    ///
    /// **Why this bites:** it rolls the transaction back and asserts the row is GONE. Against the
    /// old `&PgPool` signature the INSERT was its own autocommit and survived the rollback — which
    /// is exactly the property that let the callback commit a directory row and then lose the
    /// grant to a process death.
    #[sqlx::test(migrations = "../../migrations")]
    async fn store_grant_is_rolled_back_with_its_caller_transaction(pool: PgPool) {
        let profile = insert_profile(&pool).await;

        let owner = crate::test_support::human_principal_for(&pool, profile).await;
        let mut tx = pool.begin().await.expect("begin");
        store_grant(
            &mut tx,
            &key(),
            NewGrant {
                owner: &owner,
                slack_principal_id: PRINCIPAL,
                refresh_token: "rt",
                access_token: "at",
                access_ttl_secs: Some(3600),
            },
        )
        .await
        .expect("store");
        tx.rollback().await.expect("rollback");

        let rows: i64 = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
            PRINCIPAL,
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or_default();
        assert_eq!(
            rows, 0,
            "the grant must share the caller's transaction — surviving a rollback is the \
             autocommit shape that made the callback's dual write unrecoverable",
        );
    }

    /// An unknown principal has no identity row, so the re-rooted query returns nothing → the mint
    /// is `NotLinked`, distinct from `NotVaulted`. Before the re-root this arm could not exist: the
    /// vault-rooted query conflated "never linked" with "linked, not vaulted".
    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_reports_not_linked_for_an_unknown_principal(pool: PgPool) {
        let out = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1",
            "c",
            PRINCIPAL,
        )
        .await
        .unwrap();
        assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotLinked));
    }

    /// A link with no vaulted grant (a pre-T3 row) is `NotVaulted` — genuinely distinct from
    /// `NotLinked`. This is the cell the whole re-root exists to separate.
    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_reports_not_vaulted_for_a_link_without_a_grant(pool: PgPool) {
        let profile = insert_profile(&pool).await; // approved
        link(&pool, profile, PRINCIPAL).await; // linked, but no `store` → no vault row

        let out = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1",
            "c",
            PRINCIPAL,
        )
        .await
        .unwrap();
        assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotVaulted));
    }

    /// The ordering fix: a linked human whose standing refuses is told the STANDING reason, even
    /// when there is also no vaulted grant. Before the re-root, "no vault" won and the human was
    /// told to re-link — the false remedy, since only an admin approval helps.
    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_prefers_the_standing_refusal_over_a_missing_vault(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        set_standing(&pool, profile, "denied").await;
        link(&pool, profile, PRINCIPAL).await; // linked, denied, NOT vaulted

        let out = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1",
            "c",
            PRINCIPAL,
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            MintOutcome::Refused(LinkRefusal::Standing {
                refusal: Refusal::Denied
            }),
            "standing must win over vault state so the remedy named is the one that works",
        );
    }

    /// Fail-closed correlation (spec §2.3): when the link row and the vault row are owned by
    /// DIFFERENT profiles (an account-merge skew), the join finds no vault for the link's owner, so
    /// the mint reads as `NotVaulted` rather than decrypting one profile's grant under another's
    /// standing. There is no production path that produces this today; the seed is manual.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_vault_owned_by_a_different_profile_than_the_link_reads_as_not_vaulted(pool: PgPool) {
        let link_owner = insert_profile(&pool).await; // approved
        let grant_owner = insert_profile(&pool).await; // approved
        link(&pool, link_owner, PRINCIPAL).await;
        // Vault the SAME principal, but owned by the OTHER profile — the skew.
        store(&pool, grant_owner, PRINCIPAL, "rt", "at", Some(3600)).await;

        let out = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1",
            "c",
            PRINCIPAL,
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            MintOutcome::Refused(LinkRefusal::NotVaulted),
            "a link/vault owner skew must fail closed, never mint across profiles",
        );
    }

    /// The former `revoked_at` disjunct is dropped (spec §2.4): a flagged `revoked_at` no longer
    /// blocks minting, because soft-revoke was superseded by disconnect's DELETE (commit
    /// `3a45b1ab`). The column is retained but inert; this pins that so a regression re-adding the
    /// disjunct fails. A comfortably-valid cached AT keeps this off the (irrelevant) refresh path.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_flagged_revoked_at_no_longer_blocks_minting(pool: PgPool) {
        let profile = insert_profile(&pool).await; // approved
        store(
            &pool,
            profile,
            PRINCIPAL,
            "rt",
            "still-valid-at",
            Some(3600),
        )
        .await;
        sqlx::query!(
            "UPDATE kb_slack_grant_vault SET revoked_at = now() WHERE slack_principal_id = $1",
            PRINCIPAL,
        )
        .execute(&pool)
        .await
        .unwrap();

        let out = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1/unused",
            "c",
            PRINCIPAL,
        )
        .await
        .unwrap();
        match out {
            MintOutcome::Token { access_token, .. } => assert_eq!(access_token, "still-valid-at"),
            other => panic!("a flagged revoked_at must no longer block minting, got {other:?}"),
        }
    }

    /// The deactivation kill-switch reaches the vault: a grant whose principal standing is
    /// `deactivated` mints nothing, even with a still-valid cached token and without ever being
    /// explicitly revoked.
    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_refuses_a_deactivated_profile(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        // A comfortably-valid cached AT — absent the standing gate, mint would hand it straight back.
        store(
            &pool,
            profile,
            PRINCIPAL,
            "rt",
            "still-valid-at",
            Some(3600),
        )
        .await;
        set_standing(&pool, profile, "deactivated").await;

        let out = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1/unused",
            "c",
            PRINCIPAL,
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            MintOutcome::Refused(LinkRefusal::Standing {
                refusal: Refusal::Deactivated
            }),
            "a deactivated profile must not mint — the kill-switch must reach the vault",
        );
    }

    /// D-A: a `Revoke` stops minting immediately. This is the Phase-2 policy call — the mint gate is
    /// `standing = 'approved'`, not the behaviour-preserving `<> 'deactivated'` — so a revoked
    /// principal (whose cached token is otherwise still valid) mints nothing.
    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_refuses_a_revoked_profile(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(
            &pool,
            profile,
            PRINCIPAL,
            "rt",
            "still-valid-at",
            Some(3600),
        )
        .await;
        set_standing(&pool, profile, "revoked").await;

        let out = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1/unused",
            "c",
            PRINCIPAL,
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            MintOutcome::Refused(LinkRefusal::Standing {
                refusal: Refusal::Revoked
            }),
            "a revoked principal must not mint (D-A: mint requires an approved standing)",
        );
    }

    /// D-A / §8: a Slack-linked human born `Denied` (or with no standing row at all) cannot mint —
    /// being born denied *is* the access control. Here the profile is left without an approved
    /// standing, and even a fresh cached token is withheld.
    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_refuses_a_profile_without_approved_standing(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(
            &pool,
            profile,
            PRINCIPAL,
            "rt",
            "still-valid-at",
            Some(3600),
        )
        .await;
        set_standing(&pool, profile, "denied").await;

        let out = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1/unused",
            "c",
            PRINCIPAL,
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            MintOutcome::Refused(LinkRefusal::Standing {
                refusal: Refusal::Denied
            }),
            "a born-Denied principal must not mint until approved (§8)",
        );
    }

    /// A ciphertext sealed for one principal cannot be opened under another, even by the same key:
    /// the AAD binding defeats a row-transplant. We seal principal A's grant, copy A's rt
    /// ciphertext+nonce into B's row (the exact move a DB-write attacker would make), expire B's
    /// cached AT so mint must decrypt that transplanted RT, and require mint to FAIL CLOSED rather
    /// than hand back A's refresh token under B.
    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_rejects_a_transplanted_ciphertext(pool: PgPool) {
        const PRINCIPAL_B: &str = "slack:T0BHAHEN79C:UATTACKER";
        let profile_a = insert_profile(&pool).await;
        let profile_b = insert_profile(&pool).await;
        store(
            &pool,
            profile_a,
            PRINCIPAL,
            "A-secret-rt",
            "A-at",
            Some(3600),
        )
        .await;
        // B's own grant, with an already-expired cached AT so mint is forced onto the RT path.
        store(&pool, profile_b, PRINCIPAL_B, "B-rt", "B-at", Some(0)).await;

        // Transplant A's sealed RT (ciphertext + nonce) into B's row.
        sqlx::query!(
            r#"
            UPDATE kb_slack_grant_vault dst
               SET rt_nonce = src.rt_nonce, rt_ciphertext = src.rt_ciphertext
              FROM kb_slack_grant_vault src
             WHERE dst.slack_principal_id = $1 AND src.slack_principal_id = $2
            "#,
            PRINCIPAL_B,
            PRINCIPAL,
        )
        .execute(&pool)
        .await
        .unwrap();

        // B's row still carries the configured key's fingerprint, so the failure is a row-level
        // fault: B is told to re-link (and an error is logged). What must never happen is a token.
        let result = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1/unused",
            "c",
            PRINCIPAL_B,
        )
        .await;
        assert!(
            matches!(result, Ok(MintOutcome::Refused(LinkRefusal::NotVaulted))),
            "a ciphertext sealed for principal A must not decrypt under principal B — the AAD \
             binding must fail the tag rather than yield A's refresh token to B; got {result:?}",
        );
    }

    /// Re-storing the same principal upserts to ONE row and clears a prior revocation — a fresh
    /// consent is a fresh, working grant.
    #[sqlx::test(migrations = "../../migrations")]
    async fn re_store_upserts_one_row_and_clears_revocation(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(&pool, profile, PRINCIPAL, "rt1", "at1", Some(3600)).await;
        // Flip the flag directly — the write-side `revoke` fn is gone (disconnect now deletes
        // the row instead), but a re-store must still clear a row flagged before that change.
        sqlx::query!(
            "UPDATE kb_slack_grant_vault SET revoked_at = now() WHERE slack_principal_id = $1",
            PRINCIPAL,
        )
        .execute(&pool)
        .await
        .unwrap();

        // Re-link: new grant, longer-lived cached AT.
        store(&pool, profile, PRINCIPAL, "rt2", "at2-fresh", Some(3600)).await;

        let rows: i64 = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
            PRINCIPAL,
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or_default();
        assert_eq!(rows, 1, "re-store must not duplicate the row");

        // The revocation is cleared and the new cached AT is served.
        let out = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1/unused",
            "c",
            PRINCIPAL,
        )
        .await
        .unwrap();
        match out {
            MintOutcome::Token {
                access_token,
                expires_at,
            } => {
                assert_eq!(access_token, "at2-fresh");
                assert!(
                    expires_at > Utc::now(),
                    "a re-stored grant must serve a future expiry, got {expires_at}",
                );
            }
            other => panic!("expected the re-stored token, got {other:?}"),
        }
    }

    /// The principal is the key, WHOLE. A different principal has its own (here absent) identity
    /// row, so it reads as `NotLinked` — it never reaches, let alone reads, PRINCIPAL's grant.
    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_does_not_match_a_different_principal(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(&pool, profile, PRINCIPAL, "rt", "at", Some(3600)).await;

        let out = mint_access_token(
            &pool,
            &key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1",
            "c",
            "slack:T0BHAHEN79C:UOTHER",
        )
        .await
        .unwrap();
        assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotLinked));
    }

    // --- key fingerprints -------------------------------------------------------------------

    /// A key this deployment does not hold: the one an operator mis-pasted, or the one rotated to.
    fn other_key() -> VaultKey {
        VaultKey::from_base64(&STANDARD.encode([4u8; 32])).unwrap()
    }

    /// Every byte of a vault row the mint could write, for "nothing changed" assertions.
    type RowBytes = (
        Vec<u8>,
        Vec<u8>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<DateTime<Utc>>,
        DateTime<Utc>,
    );

    async fn row_bytes(pool: &PgPool, principal: &str) -> RowBytes {
        let r = sqlx::query!(
            "SELECT rt_nonce, rt_ciphertext, at_nonce, at_ciphertext, key_fingerprint, \
                    access_expires_at, updated_at \
               FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
            principal,
        )
        .fetch_one(pool)
        .await
        .unwrap();
        (
            r.rt_nonce,
            r.rt_ciphertext,
            r.at_nonce,
            r.at_ciphertext,
            r.key_fingerprint,
            r.access_expires_at,
            r.updated_at,
        )
    }

    async fn stored_fingerprint(pool: &PgPool, principal: &str) -> Option<Vec<u8>> {
        sqlx::query_scalar!(
            "SELECT key_fingerprint FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
            principal,
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Make the row legacy: written before fingerprints existed.
    async fn unstamp(pool: &PgPool, principal: &str) {
        sqlx::query!(
            "UPDATE kb_slack_grant_vault SET key_fingerprint = NULL WHERE slack_principal_id = $1",
            principal,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// Flip one byte of the cached access-token ciphertext: tamper under the right key.
    async fn tamper_at(pool: &PgPool, principal: &str) {
        sqlx::query!(
            "UPDATE kb_slack_grant_vault \
                SET at_ciphertext = set_byte(at_ciphertext, 0, get_byte(at_ciphertext, 0) # 255) \
              WHERE slack_principal_id = $1",
            principal,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    fn retiring(raw: &str) -> RetiredKeys {
        RetiredKeys::parse(raw, key().fingerprint()).expect("valid retirement list")
    }

    async fn mint_with(
        pool: &PgPool,
        key: &VaultKey,
        retired: &RetiredKeys,
        token_url: &str,
    ) -> ApiResult<MintOutcome> {
        mint_access_token(pool, key, retired, token_url, "c", PRINCIPAL).await
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn store_grant_stamps_the_sealing_keys_fingerprint(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(&pool, profile, PRINCIPAL, "rt", "at", Some(3600)).await;
        assert_eq!(
            stored_fingerprint(&pool, PRINCIPAL).await.as_deref(),
            Some(key().fingerprint().as_bytes()),
        );
    }

    /// **The hazard PR #1120 reverted.** A wrong-but-well-formed key is a 500 that writes nothing,
    /// so restoring the right key heals the grant.
    ///
    /// **Why this bites:** mapping [`Unopenable::UnknownKey`] to `NotVaulted` (the reverted
    /// behaviour) fails the first assertion. COMMITTING a write on the failure path (a stamp, a
    /// reseal) fails the byte comparison; an uncommitted one rolls back with the transaction, which
    /// is correct and does not fail it. And if the failure path overwrote the ciphertext, the right
    /// key could no longer mint.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_wrong_key_is_a_500_that_leaves_the_row_for_the_right_key(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(&pool, profile, PRINCIPAL, "rt", "cached-at", Some(3600)).await;
        let before = row_bytes(&pool, PRINCIPAL).await;

        let wrong = mint_with(
            &pool,
            &other_key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1",
        )
        .await;
        assert!(
            matches!(wrong, Err(ApiError::Internal(_))),
            "a key that did not seal the row and is not retired must be a 500, got {wrong:?}",
        );
        assert_eq!(
            row_bytes(&pool, PRINCIPAL).await,
            before,
            "a wrong-key mint must not write the row",
        );

        let healed = mint_with(&pool, &key(), &RetiredKeys::default(), "http://127.0.0.1:1").await;
        match healed {
            Ok(MintOutcome::Token { access_token, .. }) => assert_eq!(access_token, "cached-at"),
            other => panic!("restoring the key must heal the grant, got {other:?}"),
        }
    }

    /// The same on the refresh path (no usable cached token): the refresh token is not spent, and
    /// the row is not written.
    ///
    /// **Why this bites:** the token URL is unreachable, so any attempt to refresh would also be an
    /// `Err` — but an `Internal` one only by accident. The byte comparison is what bites: a stamp or
    /// reseal before the open is classified changes the row.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_wrong_key_on_the_refresh_path_is_a_500_that_leaves_the_row(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(&pool, profile, PRINCIPAL, "rt", "at", Some(0)).await;
        let before = row_bytes(&pool, PRINCIPAL).await;

        let out = mint_with(
            &pool,
            &other_key(),
            &RetiredKeys::default(),
            "http://127.0.0.1:1",
        )
        .await;
        assert!(matches!(out, Err(ApiError::Internal(_))), "got {out:?}");
        assert_eq!(row_bytes(&pool, PRINCIPAL).await, before);
    }

    /// A deliberate rotation, acknowledged by naming the old key's fingerprint, turns a row sealed
    /// under that key into `NotVaulted`, so the user gets a re-link URL.
    ///
    /// **Why this bites:** without the retirement the same row is a 500 (asserted first), so only
    /// consulting the retired set can produce the refusal.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_declared_rotation_mints_not_vaulted(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        // Sealed under the OLD key; the deployment now holds key().
        store_sealed_by(
            &pool,
            &other_key(),
            profile,
            PRINCIPAL,
            "rt",
            "at",
            Some(3600),
        )
        .await;

        let unacknowledged =
            mint_with(&pool, &key(), &RetiredKeys::default(), "http://127.0.0.1:1").await;
        assert!(
            matches!(unacknowledged, Err(ApiError::Internal(_))),
            "an unacknowledged rotation must stay a 500, got {unacknowledged:?}",
        );

        let retired = retiring(&other_key().fingerprint().to_string());
        let out = mint_with(&pool, &key(), &retired, "http://127.0.0.1:1")
            .await
            .unwrap();
        assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotVaulted));
    }

    /// A row that fails under the very key that sealed it is a row-level fault (tamper, transplant,
    /// a sealing bug). No key change heals it, so the user is offered a re-link.
    ///
    /// **Why this bites:** dropping the [`Unopenable::RowFault`] arm sends this row to `UnknownKey`
    /// (no retirement is configured), which is a 500.
    #[sqlx::test(migrations = "../../migrations")]
    async fn tamper_under_the_right_key_mints_not_vaulted(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(&pool, profile, PRINCIPAL, "rt", "at", Some(3600)).await;
        tamper_at(&pool, PRINCIPAL).await;

        let out = mint_with(&pool, &key(), &RetiredKeys::default(), "http://127.0.0.1:1")
            .await
            .unwrap();
        assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotVaulted));
    }

    /// Bytes that open under the right key but are not UTF-8 were sealed by something holding the
    /// key. That is a bug or a compromise, never a refusal.
    ///
    /// **Why this bites:** classifying a decoded-but-invalid payload as a tag failure would land it
    /// in `RowFault` (the row carries the configured key's fingerprint), which is `NotVaulted`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn invalid_utf8_under_the_right_key_is_a_500(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(&pool, profile, PRINCIPAL, "rt", "at", Some(3600)).await;
        let (nonce, ciphertext) = key().encrypt(&[0xff, 0xfe, 0xfd], &aad(PRINCIPAL, FIELD_AT));
        sqlx::query!(
            "UPDATE kb_slack_grant_vault SET at_nonce = $2, at_ciphertext = $3 \
              WHERE slack_principal_id = $1",
            PRINCIPAL,
            &nonce[..],
            ciphertext,
        )
        .execute(&pool)
        .await
        .unwrap();

        let out = mint_with(&pool, &key(), &RetiredKeys::default(), "http://127.0.0.1:1").await;
        assert!(matches!(out, Err(ApiError::Internal(_))), "got {out:?}");
    }

    /// A legacy row (NULL fingerprint) gets one on its first successful open.
    ///
    /// **Why this bites:** without the stamp on the cached path the column stays NULL.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_legacy_row_is_stamped_on_its_first_cached_open(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(&pool, profile, PRINCIPAL, "rt", "cached-at", Some(3600)).await;
        unstamp(&pool, PRINCIPAL).await;

        let out = mint_with(&pool, &key(), &RetiredKeys::default(), "http://127.0.0.1:1")
            .await
            .unwrap();
        assert!(matches!(out, MintOutcome::Token { .. }), "got {out:?}");
        assert_eq!(
            stored_fingerprint(&pool, PRINCIPAL).await.as_deref(),
            Some(key().fingerprint().as_bytes()),
        );
    }

    /// The refresh path stamps a legacy row even when the IdP refuses the grant: the refusal arm
    /// commits the stamp, so a dead grant is still classified when the key later rotates.
    ///
    /// **Why this bites:** stamping only in the reseal `UPDATE` (which the refusal never reaches),
    /// or not committing on the refusal arm, leaves the column NULL.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_legacy_row_is_stamped_on_the_refresh_path_even_when_the_idp_refuses(pool: PgPool) {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let idp = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({ "error": "invalid_grant" })),
            )
            .mount(&idp)
            .await;

        let profile = insert_profile(&pool).await;
        store(&pool, profile, PRINCIPAL, "rt", "at", Some(0)).await;
        unstamp(&pool, PRINCIPAL).await;

        let out = mint_with(&pool, &key(), &RetiredKeys::default(), &idp.uri())
            .await
            .unwrap();
        assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotVaulted));
        assert_eq!(
            stored_fingerprint(&pool, PRINCIPAL).await.as_deref(),
            Some(key().fingerprint().as_bytes()),
        );
    }

    /// The reseal after a successful refresh stamps the configured key's fingerprint. The row here
    /// is stamped with the WRONG fingerprint but opens, which is also the restamp case.
    ///
    /// **Why this bites:** only when BOTH writers are removed. The early stamp (which runs for any
    /// row not already carrying the configured key's fingerprint) and the reseal `UPDATE` each set
    /// the column, so dropping either one alone still passes. Dropping both leaves the old key's
    /// fingerprint on a row now sealed under the new one.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_refresh_reseals_and_stamps_the_configured_key(pool: PgPool) {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let idp = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "fresh-at",
                "refresh_token": "rotated-rt",
                "token_type": "Bearer",
                "expires_in": 3600,
            })))
            .mount(&idp)
            .await;

        let profile = insert_profile(&pool).await;
        store(&pool, profile, PRINCIPAL, "rt", "at", Some(0)).await;
        let stale = other_key().fingerprint();
        sqlx::query!(
            "UPDATE kb_slack_grant_vault SET key_fingerprint = $2 WHERE slack_principal_id = $1",
            PRINCIPAL,
            stale.as_bytes(),
        )
        .execute(&pool)
        .await
        .unwrap();

        let out = mint_with(&pool, &key(), &RetiredKeys::default(), &idp.uri())
            .await
            .unwrap();
        match out {
            MintOutcome::Token { access_token, .. } => assert_eq!(access_token, "fresh-at"),
            other => panic!("expected a refreshed token, got {other:?}"),
        }
        assert_eq!(
            stored_fingerprint(&pool, PRINCIPAL).await.as_deref(),
            Some(key().fingerprint().as_bytes()),
        );
    }

    /// A legacy row that does not open has no evidence of its key: a 500, unless the operator
    /// retired `unstamped`.
    ///
    /// **Why this bites:** treating NULL as the configured key would make the first call a
    /// `RowFault` refusal; ignoring `unstamped` would make the second a 500.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unopenable_legacy_row_is_a_500_unless_unstamped_is_retired(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store_sealed_by(
            &pool,
            &other_key(),
            profile,
            PRINCIPAL,
            "rt",
            "at",
            Some(3600),
        )
        .await;
        unstamp(&pool, PRINCIPAL).await;

        let out = mint_with(&pool, &key(), &RetiredKeys::default(), "http://127.0.0.1:1").await;
        assert!(matches!(out, Err(ApiError::Internal(_))), "got {out:?}");

        let out = mint_with(&pool, &key(), &retiring("unstamped"), "http://127.0.0.1:1")
            .await
            .unwrap();
        assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotVaulted));
    }

    /// The startup check reports a mismatch when the configured key sealed none of the recent rows.
    ///
    /// **Why this bites:** each assertion pins one arm: a mismatch under the wrong key, a match
    /// under the right one, and a retired key that no longer counts as unknown.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_startup_check_flags_a_key_that_sealed_none_of_the_recent_rows(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(&pool, profile, PRINCIPAL, "rt", "at", Some(3600)).await;
        let cfg =
            |vault_key: VaultKey, retired_vault_keys: RetiredKeys| crate::config::SlackLinkConfig {
                client_id: "c".to_string(),
                hmac_secret: "s".to_string(),
                public_base_url: "https://temper.test".to_string(),
                vault_key,
                retired_vault_keys,
            };

        let wrong = check_configured_key(&pool, &cfg(other_key(), RetiredKeys::default()))
            .await
            .expect("the query runs");
        assert!(wrong.is_mismatch(), "{wrong:?}");
        assert_eq!(wrong.unknown, vec![(key().fingerprint(), 1)]);

        let right = check_configured_key(&pool, &cfg(key(), RetiredKeys::default()))
            .await
            .expect("the query runs");
        assert_eq!(
            right,
            KeyCheck {
                sampled: 1,
                matching: 1,
                unknown: vec![],
            }
        );

        let retired =
            RetiredKeys::parse(&key().fingerprint().to_string(), other_key().fingerprint())
                .unwrap();
        let rotated = check_configured_key(&pool, &cfg(other_key(), retired))
            .await
            .expect("the query runs");
        assert!(!rotated.is_mismatch(), "{rotated:?}");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn take_refresh_token_returns_the_sealed_token_in_plaintext(pool: PgPool) {
        let key = key();
        let profile_id = insert_profile(&pool).await;
        store(
            &pool,
            profile_id,
            "slack:T1:U1",
            "rt-plaintext",
            "at-plaintext",
            Some(3600),
        )
        .await;

        let mut conn = pool.acquire().await.expect("acquire");
        let got = take_refresh_token_locked(&mut conn, &key, "slack:T1:U1")
            .await
            .expect("take");
        assert_eq!(got.as_deref(), Some("rt-plaintext"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn take_refresh_token_returns_none_for_an_unknown_principal(pool: PgPool) {
        let key = key();
        let mut conn = pool.acquire().await.expect("acquire");
        let got = take_refresh_token_locked(&mut conn, &key, "slack:T9:U9")
            .await
            .expect("take");
        assert!(
            got.is_none(),
            "an unvaulted principal must yield None, not an error"
        );
    }
}

#[cfg(test)]
mod key_fingerprint_tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;

    fn key_of(byte: u8) -> VaultKey {
        VaultKey::from_base64(&STANDARD.encode([byte; 32])).unwrap()
    }

    const PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";

    /// **The seal format is pinned.** A grant sealed by an earlier binary must open in this one.
    /// [`Unopenable::RowFault`] answers `NotVaulted` on the premise that a row failing under the key
    /// that sealed it is the row's fault. A change to how rows are opened (the AAD layout, the field
    /// tags, the cipher) would fail EVERY row that way and offer every user a re-link, so it must
    /// fail the build instead.
    ///
    /// **Why this bites:** the nonce and ciphertext are fixed bytes, computed outside this code
    /// (Python: HChaCha20 checked against the XChaCha draft's test vector, then `cryptography`'s
    /// ChaCha20-Poly1305) for key `[3; 32]` and AAD `slack:T0BHAHEN79C:U0BH6A3L6JF\0rt`. Any
    /// change to `aad`, `FIELD_RT` or the cipher fails the open.
    #[test]
    fn a_grant_sealed_by_an_earlier_binary_still_opens() {
        const NONCE: [u8; 24] = [0x11; 24];
        const SEALED: &str = "80Tufn1JNLvHIxkeNVa8kphjNkwKUtfjCD7f6QwFxFYT/GoH";
        let ciphertext = STANDARD.decode(SEALED).unwrap();
        let opened = open_secret(&key_of(3), &NONCE, &ciphertext, &aad(PRINCIPAL, FIELD_RT))
            .expect("the pinned grant must open");
        assert_eq!(opened, "pinned-refresh-token");
    }

    #[test]
    fn retired_keys_parse_fingerprints_and_unstamped() {
        let current = key_of(1).fingerprint();
        let old = key_of(2).fingerprint();
        let retired = RetiredKeys::parse(&format!(" {old} , ,UNSTAMPED,{old}"), current).unwrap();
        assert!(retired.retires(Some(old)));
        assert!(retired.retires(None), "`unstamped` retires legacy rows");
        assert!(!retired.retires(Some(current)));
        assert_eq!(retired.fingerprints, vec![old], "duplicates collapse");

        assert_eq!(
            RetiredKeys::parse("", current).unwrap(),
            RetiredKeys::default()
        );
        assert!(!RetiredKeys::default().retires(None));
    }

    #[test]
    fn retiring_the_configured_key_is_dropped() {
        let current = key_of(1).fingerprint();
        let retired = RetiredKeys::parse(&current.to_string(), current).unwrap();
        assert_eq!(retired, RetiredKeys::default());
    }

    /// A malformed entry is an error naming its position, never its text: an operator who pasted
    /// the key itself here must not have it logged.
    #[test]
    fn a_malformed_entry_is_an_error_that_does_not_echo_it() {
        let current = key_of(1).fingerprint();
        let pasted_key = STANDARD.encode([5u8; 32]);
        let err = RetiredKeys::parse(&format!("unstamped,{pasted_key}"), current).unwrap_err();
        assert!(err.contains("entry 2"), "{err}");
        assert!(
            !err.contains(&pasted_key),
            "the entry must not be echoed: {err}"
        );
    }

    #[test]
    fn classify_follows_the_table() {
        let current = key_of(1).fingerprint();
        let old = key_of(2).fingerprint();
        let stranger = key_of(3).fingerprint();
        let retired = RetiredKeys::parse(&format!("{old},unstamped"), current).unwrap();
        let none = RetiredKeys::default();
        let sealed = || OpenFailure::Sealed(CryptoError::OpenFailed);

        assert_eq!(
            classify(&OpenFailure::NotUtf8, Some(current), current, &retired),
            Unopenable::NotUtf8
        );
        assert_eq!(
            classify(&OpenFailure::NotUtf8, Some(old), current, &retired),
            Unopenable::NotUtf8
        );
        assert_eq!(
            classify(&sealed(), Some(current), current, &none),
            Unopenable::RowFault
        );
        assert_eq!(
            classify(
                &OpenFailure::Sealed(CryptoError::BadNonce),
                Some(current),
                current,
                &none
            ),
            Unopenable::RowFault
        );
        assert_eq!(
            classify(&sealed(), Some(old), current, &retired),
            Unopenable::Retired
        );
        assert_eq!(
            classify(&sealed(), None, current, &retired),
            Unopenable::Retired
        );
        assert_eq!(
            classify(&sealed(), Some(old), current, &none),
            Unopenable::UnknownKey
        );
        assert_eq!(
            classify(&sealed(), None, current, &none),
            Unopenable::UnknownKey
        );
        assert_eq!(
            classify(&sealed(), Some(stranger), current, &retired),
            Unopenable::UnknownKey
        );
    }

    #[test]
    fn the_key_check_counts_matches_and_unknowns() {
        let current = key_of(1).fingerprint();
        let old = key_of(2).fingerprint();
        let stranger = key_of(3).fingerprint();
        let retired = RetiredKeys::parse(&old.to_string(), current).unwrap();

        let empty = KeyCheck::assess([], current, &retired);
        assert!(!empty.is_mismatch(), "an empty vault is no evidence");

        let mixed = KeyCheck::assess([current, stranger, old, stranger], current, &retired);
        assert_eq!(mixed.sampled, 4);
        assert_eq!(mixed.matching, 1);
        assert_eq!(mixed.unknown, vec![(stranger, 2)]);
        assert!(!mixed.is_mismatch());

        let wrong = KeyCheck::assess([stranger, old], current, &retired);
        assert!(wrong.is_mismatch());

        let rotated = KeyCheck::assess([old, old], current, &retired);
        assert!(
            !rotated.is_mismatch(),
            "rows under a retired key are not a misconfiguration"
        );
    }
}
