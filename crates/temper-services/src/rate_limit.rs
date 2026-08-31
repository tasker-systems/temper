//! The rate-limit seam: per-route windowed limits, Postgres-backed, **default off**.
//!
//! Design authority: `temper-artifacts/specs/2026-08-31-tmpr22-rate-limit-seam-design.md`
//! (decided; every axis answered or declared there). This module implements its chosen
//! approach; the load-bearing invariants are quoted in the doc comments beside the code
//! that enforces them.
//!
//! # The two invariants this seam exists to hold
//!
//! **The edge is a non-authority.** Every control the instance's security posture relies
//! on is present in what the repository deploys. Something fronting the instance may add
//! defence; it may never be the thing being relied upon. That is why every counter here
//! lives in Postgres — the substrate every horizontally-scaled instance already shares —
//! and why keying below refuses every edge-supplied input: an IP or a forwarded-for
//! header would make the edge load-bearing, and the register's negative face refuses
//! that before design begins.
//!
//! **A limit's number is chosen, never inherited.** Nothing in this module carries a
//! default limit, a default window, or any fallback number: with the environment unset
//! the seam is *absent*, not silent. The operator chooses values in the deployment
//! environment and records them in the PR that sets them, beside the reasoning this
//! module's comments carry.
//!
//! # Where the counters live (spec A2)
//!
//! *Count the canonical artifact where one exists; mint counter state only where no
//! artifact exists.* The two opt-in doors therefore do not share a mechanism — their
//! keys differ by spec A1:
//!
//! | Door | Keyed on | Counted from | Mechanism |
//! |---|---|---|---|
//! | reconcile pair (`internal_routes`) | the route itself | `kb_rate_counters` (no per-call artifact exists — the handlers trace only) | [`require_route_rate_limit`] middleware |
//! | `POST /api/access/requests` | the verified JWT principal | the `kb_join_requests` rows themselves (the audit trail) | the `create_join_request` guard |
//!
//! One note on placement: the spec's chosen approach sketched the layer "in
//! `temper-services::transport`"; it lives in this module instead (its own home,
//! beside the counters and the guard it shares state with). That is the latitude the
//! spec's step-4 parenthetical grants — the load-bearing half it requires, one place
//! where a limit's number and window are stated, is what this module is.
//!
//! An in-process counter is rejected as *the* control: on a horizontally-scaled
//! serverless deployment it bounds one instance and reads as coverage it cannot have —
//! the `acquire_timeout` mis-framing (`api/axum.rs`, the deployment-grounding comment).
//!
//! # The MCP door is excluded, deliberately
//!
//! `/mcp` is `nest_service` over a raw tower service, so no axum per-route layer reaches
//! it, and its AS entry points are public-by-protocol where keying is hardest. Declared
//! open in spec A8 — a decision recorded, not an omission; revisit after the two opt-ins
//! have exercised.

use axum::extract::Request;
use axum::extract::State;
use axum::middleware::Next;
use axum::response::Response;
use sqlx::PgPool;

use crate::auth_config::ConfigError;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// One door's limit: at most [`Self::max`] events per [`Self::window_secs`], as chosen
/// by the operator in the environment. No defaults — a `None` on the door means the
/// seam is absent there, which is the shipped posture (default off).
///
/// `max = 0` is coherent and allowed: it declines *every* request on that door, which is
/// a legitimate operator statement ("this door stays shut until I raise it"). A negative
/// max or a non-positive window is refused at boot — see [`parse_rate_limit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowLimit {
    pub max: i64,
    pub window_secs: i32,
}

/// The seam's configuration: one optional [`WindowLimit`] per opt-in door.
///
/// `None` on a door ⇒ unlimited, and that is the *default* — the numbers below exist
/// only when an operator sets them. This is the default-off posture from the task's own
/// criterion: shipped code, zero shipped enforcement, the operator's knob.
///
/// # The numbers, and where they are recorded
///
/// Per spec A9 this module fixes the *form* — named environment variables, one window
/// each, recorded here and in the PR that chooses values — and no value. When the
/// operator chooses numbers, the PR description carries them beside the reasoning in
/// these comments: what each door is, what keys it, and the per-instance consequence of
/// that keying (see the door table in the module docs).
///
/// **The operator's choice for the first deployment, recorded at build (2026-08-31):**
/// the reconcile pair at **30 calls per route per 3600 s window**
/// (`RATE_LIMIT_RECONCILE_MAX=30`), `POST /api/access/requests` at **10 requests per
/// principal per 3600 s window** (`RATE_LIMIT_CREATE_REQUEST_MAX=10`). Both sit far
/// above any legitimate traffic on doors that are low-volume by design, while bounding
/// the tight Request/Withdraw cycling this seam exists to answer. The values live in the
/// deployment environment, never here — nothing in this module falls back to them.
///
/// | Variable pair | Door |
/// |---|---|
/// | `RATE_LIMIT_RECONCILE_MAX` / `RATE_LIMIT_RECONCILE_WINDOW_SECS` | the reconcile pair (`/internal/saml/reconcile`, `/internal/principal/resolve`) |
/// | `RATE_LIMIT_CREATE_REQUEST_MAX` / `RATE_LIMIT_CREATE_REQUEST_WINDOW_SECS` | `POST /api/access/requests` |
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateLimitConfig {
    /// The reconcile channel's limit, applied per route (each route counts separately —
    /// the key is the route, so the two endpoints never spend one budget) to every route
    /// on `internal_routes`, which is exactly the reconcile pair today.
    pub reconcile: Option<WindowLimit>,
    /// The self-service join-request door's limit, keyed on the verified principal.
    pub create_request: Option<WindowLimit>,
}

/// How a door parses out of the environment: all-or-nothing per door, values validated.
///
/// A *partial* pair is a boot error, not a silently-unconfigured door. The precedent
/// (`parse_vercel_connect`, `parse_slack_link`) treats partial as unconfigured because a
/// half-configured *flow* fails loudly at request time. A half-configured **limit** is
/// the opposite failure: it passes every request while the operator believes a bound is
/// live — the inherited-value trap this seam exists to close. So: set one of the pair
/// without the other and boot refuses, naming the missing variable.
///
/// A malformed or degenerate value (`max < 0`, `window_secs <= 0`) is likewise a boot
/// error: a limit that silently does nothing is worse than no limit, because no limit
/// announces itself.
pub fn parse_rate_limit(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Option<RateLimitConfig>, ConfigError> {
    let reconcile = parse_window(
        &lookup,
        "RATE_LIMIT_RECONCILE_MAX",
        "RATE_LIMIT_RECONCILE_WINDOW_SECS",
    )?;
    let create_request = parse_window(
        &lookup,
        "RATE_LIMIT_CREATE_REQUEST_MAX",
        "RATE_LIMIT_CREATE_REQUEST_WINDOW_SECS",
    )?;

    if reconcile.is_none() && create_request.is_none() {
        return Ok(None);
    }
    Ok(Some(RateLimitConfig {
        reconcile,
        create_request,
    }))
}

/// Parse one door's pair with the all-or-nothing + validated rules documented on
/// [`parse_rate_limit`].
fn parse_window(
    lookup: &impl Fn(&str) -> Option<String>,
    max_var: &'static str,
    window_var: &'static str,
) -> Result<Option<WindowLimit>, ConfigError> {
    let raw_max = lookup(max_var).filter(|s| !s.is_empty());
    let raw_window = lookup(window_var).filter(|s| !s.is_empty());

    match (raw_max, raw_window) {
        (None, None) => Ok(None),
        (Some(_), None) => Err(ConfigError::Missing(window_var)),
        (None, Some(_)) => Err(ConfigError::Missing(max_var)),
        (Some(max), Some(window)) => {
            let max = max
                .trim()
                .parse::<i64>()
                .map_err(|_| ConfigError::NotAnInteger(max_var, "a whole number of requests"))?;
            let window_secs = window
                .trim()
                .parse::<i32>()
                .map_err(|_| ConfigError::NotAnInteger(window_var, "a whole number of seconds"))?;
            if max < 0 {
                return Err(ConfigError::RateLimitOutOfRange(
                    max_var,
                    "a non-negative request count (0 declines every request on the door)",
                ));
            }
            if window_secs <= 0 {
                return Err(ConfigError::RateLimitOutOfRange(
                    window_var,
                    "a positive window in seconds (a non-positive window never counts anything)",
                ));
            }
            Ok(Some(WindowLimit { max, window_secs }))
        }
    }
}

/// The reconcile channel's counter: one row per route, bumped by a **single statement**
/// so the increment-and-rollover is atomic per caller — safe under PgBouncer transaction
/// mode, where session-level state is not reliable (`api/axum.rs`, the deployment
/// grounding). The window rollover is decided inside the same statement that counts, so
/// two concurrent calls can never resurrect a window that just expired.
///
/// Returns `(allowed, retry_after_secs)`. `allowed` is `call_count <= max`; a window
/// that expired rolls to `1` for the caller that tripped it. The retry value is
/// computed **inside the statement** — `window_started_at` and `now()` are both the
/// database's clock, so a `Retry-After: 0` cannot be manufactured by app/DB clock skew;
/// the value is the honest number of seconds until a retry re-enters as the window's
/// first call.
///
/// The approximation to *state* in the door's reasoning, not to fix: the counter bounds
/// pressure, it is not a ledger. The row-level atomicity makes the count exact per
/// statement; what stays approximate is the operator's intent — a window boundary can
/// admit `max` more calls one instant after refusing the `max + 1`-th, which is what a
/// window is.
pub async fn bump_route(pool: &PgPool, route: &str, limit: WindowLimit) -> ApiResult<(bool, i64)> {
    let row = sqlx::query!(
        r#"
        INSERT INTO kb_rate_counters AS c (route, window_started_at, call_count)
        VALUES ($1, now(), 1)
        ON CONFLICT (route) DO UPDATE SET
            call_count = CASE
                WHEN c.window_started_at < now() - make_interval(secs => $2)
                    THEN 1
                    ELSE c.call_count + 1
            END,
            window_started_at = CASE
                WHEN c.window_started_at < now() - make_interval(secs => $2)
                    THEN now()
                    ELSE c.window_started_at
            END
        RETURNING call_count,
                  GREATEST(0, extract(epoch FROM
                      (window_started_at + make_interval(secs => $2)) - now()))::bigint
                      AS "retry_after_secs!"
        "#,
        route,
        // `make_interval`'s `secs` parameter is double precision, so the bind is an f64;
        // whole seconds are exact in it.
        f64::from(limit.window_secs),
    )
    .fetch_one(pool)
    .await?;

    let allowed = row.call_count <= limit.max;
    Ok((allowed, row.retry_after_secs))
}

/// The self-service guard: **count the canonical artifact** (spec A2). "This principal
/// requested recently" is already stated by the `kb_join_requests` rows — the audit
/// trail — so the guard reads a windowed COUNT off those rows and mints no second
/// bookkeeping beside them.
///
/// `None` is the default-off path and passes immediately — not "checks with an infinite
/// limit", *passes*: an absent limit must not pay for a round trip it will never use.
///
/// Refusal carries a `Retry-After` computed **inside the statement** from the oldest
/// in-window request — both operands on the database's clock — which is the moment the
/// count next drops: the sliding-window honest answer. The COUNT is approximate under
/// races (two concurrent callers near the edge may both pass); that is
/// accepted by spec and must stay *stated* in the operator-facing reasoning: a limit is
/// pressure control, not a ledger.
pub async fn guard_join_request(
    pool: &PgPool,
    profile_id: uuid::Uuid,
    limit: Option<WindowLimit>,
) -> ApiResult<()> {
    let Some(limit) = limit else {
        return Ok(());
    };

    let row = sqlx::query!(
        r#"
        SELECT count(*) AS "count!",
               GREATEST(0, extract(epoch FROM
                   (COALESCE(min(created), now()) + make_interval(secs => $2)) - now()))::bigint
                   AS "retry_after_secs!"
          FROM kb_join_requests
         WHERE requesting_profile_id = $1
           AND created > now() - make_interval(secs => $2)
        "#,
        profile_id,
        // Same double-precision `secs` bind as `bump_route` above.
        f64::from(limit.window_secs),
    )
    .fetch_one(pool)
    .await?;

    if row.count >= limit.max {
        tracing::info!(
            count = row.count,
            max = limit.max,
            "join-request rate limit reached"
        );
        return Err(ApiError::TooManyRequests {
            message: "too many access requests in the current window".to_string(),
            retry_after_secs: row.retry_after_secs,
        });
    }

    Ok(())
}

/// The reconcile channel's per-route limit middleware, mounted **at the merge sites** of
/// `internal_routes` (the discipline that keeps a route from ever being mounted ungated
/// — here the concern is the opposite face of the same coin: mounted *limited* or
/// mounted nowhere).
///
/// Keyed on the route path itself (spec A1: the reconcile caller is
/// anonymous-but-secret-bearing, so the route is the only controlled input). Config
/// `None` ⇒ pass-through with no database touch: default off means *absent*, not
/// "checks something and always says yes".
///
/// Mounted *inside* the signature gate at every site (the signature layer is applied
/// after this one, so it runs first on the request): an unsigned caller must never be
/// able to spend the signed caller's budget — garbage gets the 401, and the counter
/// never hears about it.
pub async fn require_route_rate_limit(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let Some(limit) = state.config.rate_limit.and_then(|r| r.reconcile) else {
        return Ok(next.run(request).await);
    };

    let route = request.uri().path().to_string();
    let (allowed, retry_after_secs) = bump_route(&state.pool, &route, limit).await?;

    if !allowed {
        tracing::info!(%route, max = limit.max, "reconcile rate limit reached");
        return Err(ApiError::TooManyRequests {
            message: format!("too many requests to {route} in the current window"),
            retry_after_secs,
        });
    }

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    // --- default off: the posture the task ships ---

    // FAILS IF: the seam ever grows a default limit or a fallback number. Unset is
    // ABSENT — no parse error, and no Some carrying an inherited value.
    #[test]
    fn unset_environment_parses_to_no_seam() {
        assert_eq!(parse_rate_limit(env(&[])), Ok(None));
    }

    #[test]
    fn empty_strings_are_unset_not_zero() {
        assert_eq!(
            parse_rate_limit(env(&[
                ("RATE_LIMIT_RECONCILE_MAX", ""),
                ("RATE_LIMIT_RECONCILE_WINDOW_SECS", ""),
            ])),
            Ok(None)
        );
    }

    // --- a limit exists only when both of its numbers were chosen ---

    #[test]
    fn one_chosen_pair_parses() {
        assert_eq!(
            parse_rate_limit(env(&[
                ("RATE_LIMIT_RECONCILE_MAX", "30"),
                ("RATE_LIMIT_RECONCILE_WINDOW_SECS", "3600"),
            ])),
            Ok(Some(RateLimitConfig {
                reconcile: Some(WindowLimit {
                    max: 30,
                    window_secs: 3600
                }),
                create_request: None,
            }))
        );
    }

    // FAILS IF: a half-set door silently disables its limit. An operator who set the max
    // and mistyped the window variable name must not get an unlimited door that looks
    // configured — boot refuses, naming the missing variable.
    #[test]
    fn a_max_without_a_window_refuses_to_boot() {
        assert_eq!(
            parse_rate_limit(env(&[("RATE_LIMIT_CREATE_REQUEST_MAX", "5"),])),
            Err(ConfigError::Missing(
                "RATE_LIMIT_CREATE_REQUEST_WINDOW_SECS"
            ))
        );
    }

    #[test]
    fn a_window_without_a_max_refuses_to_boot() {
        assert_eq!(
            parse_rate_limit(env(&[("RATE_LIMIT_RECONCILE_WINDOW_SECS", "60"),])),
            Err(ConfigError::Missing("RATE_LIMIT_RECONCILE_MAX"))
        );
    }

    // --- degenerate values refuse rather than silently doing nothing ---

    #[test]
    fn a_non_numeric_max_refuses_to_boot() {
        assert!(matches!(
            parse_rate_limit(env(&[
                ("RATE_LIMIT_RECONCILE_MAX", "thirty"),
                ("RATE_LIMIT_RECONCILE_WINDOW_SECS", "60"),
            ])),
            Err(ConfigError::NotAnInteger("RATE_LIMIT_RECONCILE_MAX", _))
        ));
    }

    #[test]
    fn a_negative_max_refuses_to_boot() {
        assert_eq!(
            parse_rate_limit(env(&[
                ("RATE_LIMIT_RECONCILE_MAX", "-1"),
                ("RATE_LIMIT_RECONCILE_WINDOW_SECS", "60"),
            ])),
            Err(ConfigError::RateLimitOutOfRange(
                "RATE_LIMIT_RECONCILE_MAX",
                "a non-negative request count (0 declines every request on the door)"
            ))
        );
    }

    // FAILS IF: a zero or negative window parses. Such a window never counts anything —
    // every call re-rolls it — so it is an absent limit wearing a configured one's
    // clothes, exactly the shape boot must refuse.
    #[test]
    fn a_zero_window_refuses_to_boot() {
        assert_eq!(
            parse_rate_limit(env(&[
                ("RATE_LIMIT_CREATE_REQUEST_MAX", "5"),
                ("RATE_LIMIT_CREATE_REQUEST_WINDOW_SECS", "0"),
            ])),
            Err(ConfigError::RateLimitOutOfRange(
                "RATE_LIMIT_CREATE_REQUEST_WINDOW_SECS",
                "a positive window in seconds (a non-positive window never counts anything)"
            ))
        );
    }

    // --- max = 0 is a statement, not an error: "this door stays shut" ---

    #[test]
    fn a_zero_max_is_a_chosen_block_all() {
        assert_eq!(
            parse_rate_limit(env(&[
                ("RATE_LIMIT_RECONCILE_MAX", "0"),
                ("RATE_LIMIT_RECONCILE_WINDOW_SECS", "60"),
            ])),
            Ok(Some(RateLimitConfig {
                reconcile: Some(WindowLimit {
                    max: 0,
                    window_secs: 60
                }),
                create_request: None,
            }))
        );
    }

    // --- doors are independent: choosing one leaves the other absent ---

    #[test]
    fn choosing_one_door_leaves_the_other_unlimited() {
        let parsed = parse_rate_limit(env(&[
            ("RATE_LIMIT_CREATE_REQUEST_MAX", "10"),
            ("RATE_LIMIT_CREATE_REQUEST_WINDOW_SECS", "86400"),
            ("RATE_LIMIT_RECONCILE_WINDOW_SECS", "60"), // partial: no MAX
        ]));
        assert_eq!(
            parsed,
            Err(ConfigError::Missing("RATE_LIMIT_RECONCILE_MAX")),
            "the doors must be all-or-nothing independently — a partial pair on EITHER door boots \
             no instance"
        );
    }
}
