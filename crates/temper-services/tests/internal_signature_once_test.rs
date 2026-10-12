#![cfg(feature = "test-db")]

//! An internal v2 signature is spent once, judged on the API's clock.
//!
//! The HTTP witnesses (`temper-api/tests/internal_saml_test.rs`, the e2e mint suite) run with the
//! API and the database on one machine, so their clocks agree. This one makes them disagree: the
//! API's `now` is passed in, and the database's clock is far ahead of it. Freshness is judged on
//! the API's clock, so the nonce's expiry and its prune must be too, or the database would prune a
//! row while its signature was still fresh and the replay would be claimed again.

use sqlx::PgPool;
use temper_core::internal_sig::{sign_v2, DOOR_SLACK_MINT};
use temper_services::services::internal_sig_service::{accept_once, SignedRequest};

const SECRET: &[u8] = b"mint-secret";
const NONCE: &str = "0123456789abcdef0123456789abcdef";
const BODY: &[u8] = b"{}";

#[sqlx::test(migrations = "../../migrations")]
async fn a_replay_is_refused_while_fresh_by_the_apis_clock(pool: PgPool) {
    // An API clock an hour behind the database's: to the database, this signature expired long
    // ago; to the API, it is fresh.
    let api_now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM now())::bigint - 3600")
        .fetch_one(&pool)
        .await
        .expect("read clock");
    let signature = sign_v2(SECRET, "POST", DOOR_SLACK_MINT, api_now, NONCE, BODY);
    let request = SignedRequest {
        method: "POST",
        door: DOOR_SLACK_MINT,
        timestamp: api_now,
        nonce: NONCE,
        signature: &signature,
        body: BODY,
    };

    accept_once(&pool, SECRET, &request, api_now, "test")
        .await
        .expect("the first presentation is accepted");

    // Another request's claim prunes expired rows. Judged on the database's clock, that would
    // delete the first nonce's row while the API still holds its signature fresh. (A replay's own
    // claim cannot do it: its prune and its insert share one snapshot.)
    let other_nonce = "fedcba9876543210fedcba9876543210";
    let other_signature = sign_v2(SECRET, "POST", DOOR_SLACK_MINT, api_now, other_nonce, BODY);
    let other = SignedRequest {
        nonce: other_nonce,
        signature: &other_signature,
        ..request
    };
    accept_once(&pool, SECRET, &other, api_now + 5, "test")
        .await
        .expect("an unrelated request is accepted");

    accept_once(&pool, SECRET, &request, api_now + 10, "test")
        .await
        .expect_err("the same nonce, still fresh to the API, must be refused");
}

/// A nonce is spent at its own door. A holder of a cheaper key (link-state) who saw an in-flight
/// mint nonce cannot spend it first at their own door and have the real mint refused as a replay.
#[sqlx::test(migrations = "../../migrations")]
async fn a_nonce_spent_at_one_door_is_still_good_at_another(pool: PgPool) {
    use temper_core::internal_sig::DOOR_SLACK_LINK_STATE;

    let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM now())::bigint")
        .fetch_one(&pool)
        .await
        .expect("read clock");
    let link_key: &[u8] = b"link-secret";
    let link_sig = sign_v2(link_key, "POST", DOOR_SLACK_LINK_STATE, now, NONCE, BODY);
    let at_link = SignedRequest {
        method: "POST",
        door: DOOR_SLACK_LINK_STATE,
        timestamp: now,
        nonce: NONCE,
        signature: &link_sig,
        body: BODY,
    };
    accept_once(&pool, link_key, &at_link, now, "test")
        .await
        .expect("the link-state request is accepted");

    let mint_sig = sign_v2(SECRET, "POST", DOOR_SLACK_MINT, now, NONCE, BODY);
    let at_mint = SignedRequest {
        door: DOOR_SLACK_MINT,
        signature: &mint_sig,
        ..at_link
    };
    accept_once(&pool, SECRET, &at_mint, now, "test")
        .await
        .expect("the same nonce at the mint door is its own");
}
