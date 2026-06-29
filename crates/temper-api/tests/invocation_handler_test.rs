#![cfg(feature = "test-db")]
//! The invocation-envelope vertical (`DbBackend::{open,close}_invocation` + the
//! `substrate_read` show/list wrappers), exercised directly — the same approach as
//! `cogmap_shape_handler_test`. Full HTTP routing is covered by a later e2e task.
//!
//! Happy path runs against the L0 kernel map (root-joined → readable by any approved profile;
//! it has a reserved telos resource, so `open` succeeds). Deny path runs against a random
//! cogmap the acting profile cannot read (open → Forbidden) and a random invocation id
//! (show → Ok(None), the leak-safe deny/absent contract).

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::invocation::Disposition;
use temper_services::backend::{substrate_read, DbBackend};
use temper_workflow::operations::{Backend, CloseInvocation, OpenInvocation, Surface};

mod common;

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn open_show_close_roundtrip_on_l0(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "opener@example.com").await;
    // Approve the profile: the `sync_system_membership` trigger then joins it to the `temper-system`
    // root team, which owns L0 (`kb_team_cogmaps`) — so the kernel map becomes readable. This is the
    // production "approval auto-joins the root" path, not a special-case grant.
    sqlx::query("UPDATE kb_profiles SET system_access = 'approved' WHERE id = $1")
        .bind(profile)
        .execute(&pool)
        .await
        .expect("approve test profile");
    let profile_id = ProfileId::from(profile);
    let backend = DbBackend::new(pool.clone(), profile_id);

    // open — against L0 (readable + has a telos resource), returns the minted id.
    let out = backend
        .open_invocation(OpenInvocation {
            trigger_kind: "manual".to_string(),
            originating_cogmap: CogmapId::from(L0_COGMAP),
            parent_cogmap: None,
            origin: Surface::ApiHttp,
        })
        .await
        .expect("open against readable L0 must succeed");
    let invocation_id = out.value;

    // show — the freshly opened envelope is visible and status == "open".
    let view = substrate_read::invocation_show_select(&pool, profile_id, invocation_id)
        .await
        .expect("show must be Ok")
        .expect("opened invocation must be present");
    assert_eq!(view.status, "open", "freshly opened: {view:?}");
    assert!(view.outcome.is_none(), "no outcome while open: {view:?}");

    // close — Completed with an outcome payload.
    backend
        .close_invocation(CloseInvocation {
            invocation: invocation_id,
            disposition: Disposition::Completed,
            outcome: serde_json::json!({ "result": "ok" }),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("close must succeed");

    // show again — now completed, with the outcome present.
    let closed = substrate_read::invocation_show_select(&pool, profile_id, invocation_id)
        .await
        .expect("show must be Ok")
        .expect("closed invocation must still be present");
    assert_eq!(closed.status, "completed", "after close: {closed:?}");
    assert!(
        closed.outcome.is_some(),
        "outcome present after close: {closed:?}"
    );
    assert!(
        closed.closed_at.is_some(),
        "closed_at set after close: {closed:?}"
    );

    // append-only: close is a one-shot terminal transition. Re-closing a completed envelope is a
    // Conflict, not a second silent overwrite of the terminal record.
    let reclose = backend
        .close_invocation(CloseInvocation {
            invocation: invocation_id,
            disposition: Disposition::Failed,
            outcome: serde_json::json!({ "result": "should-not-apply" }),
            origin: Surface::ApiHttp,
        })
        .await;
    assert!(
        matches!(reclose, Err(temper_core::error::TemperError::Conflict(_))),
        "re-closing a completed invocation must be a Conflict: {reclose:?}"
    );

    // the rejected re-close left the terminal record untouched.
    let still = substrate_read::invocation_show_select(&pool, profile_id, invocation_id)
        .await
        .expect("show must be Ok")
        .expect("invocation still present");
    assert_eq!(
        still.status, "completed",
        "terminal record preserved after rejected re-close: {still:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn open_on_unreadable_cogmap_is_forbidden(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "nobody@example.com").await;
    let profile_id = ProfileId::from(profile);
    let backend = DbBackend::new(pool.clone(), profile_id);

    // A random cogmap the profile cannot read → the backend's auth-before-write denies.
    let result = backend
        .open_invocation(OpenInvocation {
            trigger_kind: "manual".to_string(),
            originating_cogmap: CogmapId::from(Uuid::now_v7()),
            parent_cogmap: None,
            origin: Surface::ApiHttp,
        })
        .await;
    assert!(
        result.is_err(),
        "open against an unreadable cogmap must be denied: {result:?}"
    );

    // A random invocation id the profile cannot read → leak-safe Ok(None), never an error.
    let absent = substrate_read::invocation_show_select(&pool, profile_id, Uuid::now_v7())
        .await
        .expect("non-readable invocation is None, not an error");
    assert!(
        absent.is_none(),
        "unknown invocation must be None: {absent:?}"
    );
}
