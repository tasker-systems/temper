#![cfg(feature = "test-db")]
//! Agent-invocation envelope `open → (act) → close → read` end-to-end: drives the
//! REAL Axum server (in-process), real Postgres, real JWT auth, through the production
//! `temper-client` invocations sub-client (`app.client.invocations()`), NOT raw reqwest.
//!
//! Invocations carry NO body/chunks/embeddings, so this needs NO embed feature — it runs
//! on plain `cargo make test-e2e`.
//!
//! The open/close auth gate is `anchor_readable_by_profile(profile,'kb_cogmaps',L0)`:
//! root-team membership satisfies it. `common::enable_invite_only` both configures the
//! gating slug AND makes the principal an owner of the root `temper-system` team, which
//! is sufficient READ access for the L0 system-default kernel map (born by migration
//! `20260625000001`). The reconcile e2e proves opening against L0 works (reconcile itself
//! opens+closes an envelope against L0); here we surface that lifecycle directly.

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

use temper_core::types::invocation::Disposition;
use temper_core::types::invocation_requests::{CloseInvocationRequest, OpenInvocationRequest};

/// The L0 kernel cognitive map reserved id (birth migration `20260625000001`).
const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// Pre-flight a token by hitting GET /api/profile (auto-provisions the profile), returning its UUID.
async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight request failed");
    assert_eq!(resp.status(), StatusCode::OK, "preflight should succeed");
    let body: serde_json::Value = resp.json().await.expect("preflight json parse");
    body["id"]
        .as_str()
        .expect("profile id missing")
        .parse()
        .expect("profile id parse")
}

/// The full envelope lifecycle through the production client: open mints an envelope
/// (whose `delegated_launch` event IS the first act), show reflects the open state and
/// lists, close terminates it with a disposition + outcome, and a re-read reflects the
/// closed state with a grown act trail (the `invocation_closed` event now also stamped).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn open_act_close_read_lifecycle_through_real_server(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Provision the e2e principal, then make it a root-team owner so it can READ L0 (the
    // open/close auth gate). `enable_invite_only` configures the gating slug AND the
    // membership in one step.
    let principal = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, principal).await;

    // 1. open ────────────────────────────────────────────────────────────────────────
    let ack = app
        .client
        .invocations()
        .open(&OpenInvocationRequest {
            trigger_kind: "e2e".into(),
            originating_cogmap: L0_COGMAP,
            parent_cogmap: None,
        })
        .await
        .expect("open should succeed against L0 with root-team read access");
    let invocation_id = ack.invocation_id;

    // 2. read (open state) ─────────────────────────────────────────────────────────────
    let opened = app
        .client
        .invocations()
        .show(invocation_id)
        .await
        .expect("show open invocation");
    assert_eq!(opened.id, invocation_id);
    assert_eq!(opened.status, "open", "freshly opened envelope is open");
    assert_eq!(opened.originating_cogmap_id, L0_COGMAP);
    assert!(opened.closed_at.is_none(), "open envelope has no closed_at");
    assert!(opened.outcome.is_none(), "open envelope has no outcome");
    // The `delegated_launch` event the open itself stamps with this invocation_id IS the
    // first act — open→act is one beat (per-act domain threading is a separate task).
    assert!(
        !opened.acts.is_empty(),
        "the open stamps a launch act under the envelope"
    );
    let acts_at_open = opened.acts.len();

    // 3. list ──────────────────────────────────────────────────────────────────────────
    let by_cogmap = app
        .client
        .invocations()
        .list(Some(L0_COGMAP), None)
        .await
        .expect("list by cogmap");
    assert!(
        by_cogmap.iter().any(|s| s.id == invocation_id),
        "list(cogmap=L0) contains the new invocation"
    );
    let by_status = app
        .client
        .invocations()
        .list(None, Some("open".into()))
        .await
        .expect("list by status");
    assert!(
        by_status.iter().any(|s| s.id == invocation_id),
        "list(status=open) contains the new invocation"
    );

    // 4. close ──────────────────────────────────────────────────────────────────────────
    app.client
        .invocations()
        .close(
            invocation_id,
            &CloseInvocationRequest {
                disposition: Disposition::Completed,
                outcome: serde_json::json!({ "note": "e2e done" }),
            },
        )
        .await
        .expect("close should succeed");

    // 5. read (closed state) ─────────────────────────────────────────────────────────────
    let closed = app
        .client
        .invocations()
        .show(invocation_id)
        .await
        .expect("show closed invocation");
    assert_eq!(closed.status, "completed", "closed envelope is completed");
    assert!(
        closed.closed_at.is_some(),
        "closed envelope has a closed_at"
    );
    let outcome = closed.outcome.expect("closed envelope carries an outcome");
    assert_eq!(
        outcome["note"], "e2e done",
        "the outcome payload round-trips"
    );
    assert!(
        closed.acts.len() > acts_at_open,
        "the close stamps an additional act (invocation_closed) under the envelope"
    );
}

// Deny case (authed principal that CANNOT read L0 → Forbidden from open) is NOT added
// here: reaching the handler requires system access, but any root-team member can READ
// L0 (so they'd be allowed, not denied), and contriving "system-access + NOT L0-readable"
// membership state would be faked rather than real. Deny is covered at the handler-test +
// substrate-artifact layers. (SG-5 / escalate-don't-fabricate.)
