#![cfg(feature = "test-db")]
//! The invocation-envelope vertical (`DbBackend::{open,close}_invocation` + the
//! `substrate_read` show/list wrappers), exercised directly — the same approach as
//! `cogmap_shape_handler_test`. Full HTTP routing is covered by a later e2e task.
//!
//! Happy path runs against the L0 kernel map (root-joined → readable by any approved profile;
//! it has a reserved telos resource, so `open` succeeds). Deny path runs against a random
//! cogmap the acting profile cannot read (open → Forbidden) and a random invocation id
//! (show → Ok(None), the leak-safe deny/absent contract).
//!
//! **The deny path has two dialects, and this suite carries both poles.** A caller who READS the
//! map but cannot author it is told which capability it lacks (`ForbiddenDetail`); a caller who
//! cannot read it at all keeps the argument-free `Forbidden`, because for that caller the map's
//! existence is itself the secret. L0 makes both reachable in one fixture: any approved profile is
//! root-joined to it (so it reads), and no profile holds write on it until granted.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::invocation::Disposition;
use temper_services::backend::{substrate_read, DbBackend};
use temper_workflow::operations::{Backend, CloseInvocation, OpenInvocation, Surface};

mod common;

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// The reader's dialect: refused, and told **which capability** was missing and **on what**.
///
/// Three assertions, and each fails a different wrong implementation. The variant alone would pass
/// if the sentence were empty or generic; the `write grant` clause alone would pass if the refusal
/// named no subject, leaving a caller that authors into several maps unable to tell which one it was
/// refused on; and the subject alone would pass a message that named the map but not the capability
/// — which is the state this whole change exists to leave, since naming the subject without naming
/// the missing grant is exactly as unactionable as saying nothing.
fn assert_names_the_missing_capability<T: std::fmt::Debug>(
    result: &Result<T, TemperError>,
    cogmap: Uuid,
) {
    let Err(TemperError::ForbiddenDetail(msg)) = result else {
        panic!(
            "a principal who READS this map must be refused in the disclosing dialect \
             (ForbiddenDetail), not the argument-free one: {result:?}"
        );
    };
    assert!(
        msg.contains("write grant"),
        "the refusal must name the capability that was missing: {msg:?}"
    );
    assert!(
        msg.contains(&cogmap.to_string()),
        "the refusal must name the subject it refused: {msg:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn open_show_close_roundtrip_on_l0(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "opener@example.com").await;
    // Grant `approved` standing (D11 front door). L0 is the public kernel map, so it is readable
    // regardless of membership; the principal is still read-only on L0 until granted write below.
    common::fixtures::approve_standing(&pool, profile).await;
    // Self-attributed open now requires WRITE on the originating map (F2) — read alone is not enough.
    common::fixtures::grant_cogmap_write(&pool, L0_COGMAP, profile).await;
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
    assert!(
        view.disposition.is_none(),
        "an open invocation has no disposition"
    );

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
    assert_eq!(
        closed.disposition,
        Some(Disposition::Completed),
        "the disposition derived from `status` must survive a real close+show round-trip"
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
    //
    // **The NEGATIVE POLE for the disclosure split**, and the reason it asserts the variant rather
    // than `is_err()`: this caller cannot read the map, so the refusal must stay argument-free.
    // Naming the missing capability here would confirm to a stranger that they had named a real
    // subject — an existence oracle over maps they have no standing to see. Without this assertion,
    // a gate that disclosed to *everyone* would pass the two tests below and nothing would notice.
    let result = backend
        .open_invocation(OpenInvocation {
            trigger_kind: "manual".to_string(),
            originating_cogmap: CogmapId::from(Uuid::now_v7()),
            parent_cogmap: None,
            origin: Surface::ApiHttp,
        })
        .await;
    assert!(
        matches!(result, Err(temper_core::error::TemperError::Forbidden)),
        "a caller who cannot READ the map must get the argument-free refusal, never one naming \
         the capability: {result:?}"
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

/// F2 — a SELF-ATTRIBUTED open (`parent_cogmap: None`) requires WRITE on the originating map, not just
/// read. An approved profile is root-joined to L0 (read) but holds no write grant: its self-attributed
/// open is denied. Granting explicit cogmap-write then lets the same open succeed. Closes the
/// reader-posts-inert-envelopes ledger-noise vector.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn self_attributed_open_requires_write(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "reader@example.com").await;
    common::fixtures::approve_standing(&pool, profile).await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    // Read-only (root-joined) profile → self-attributed open denied.
    let denied = backend
        .open_invocation(OpenInvocation {
            trigger_kind: "manual".to_string(),
            originating_cogmap: CogmapId::from(L0_COGMAP),
            parent_cogmap: None,
            origin: Surface::ApiHttp,
        })
        .await;
    // Still refused — and now the refusal SAYS SO. This principal is root-joined to L0, so it reads
    // the map; the gate owes it the reason, because withholding one bought no confidentiality and
    // cost the production steward the fact it needed in order to stop probing.
    assert_names_the_missing_capability(&denied, L0_COGMAP);

    // Grant explicit write → the same open now succeeds.
    common::fixtures::grant_cogmap_write(&pool, L0_COGMAP, profile).await;
    backend
        .open_invocation(OpenInvocation {
            trigger_kind: "manual".to_string(),
            originating_cogmap: CogmapId::from(L0_COGMAP),
            parent_cogmap: None,
            origin: Surface::ApiHttp,
        })
        .await
        .expect("self-attributed open with write must succeed");
}

/// Supplying `parent_cogmap` must NOT downgrade the gate — the regression test for the production
/// bypass, and the inversion of the retired `delegated_open_needs_only_read`.
///
/// That test asserted a read-only principal may open a DELEGATED envelope, on F2's reasoning that "the
/// substrate's parent→originating lineage is the control". It is not a control over the caller:
/// `cogmaps_share_a_team(parent, originating)` takes two cogmap ids and **no principal**, and is
/// reflexive for any team-joined map. So the retired test passed `parent == originating` — a value every
/// caller already holds — and thereby demonstrated the bypass it was certifying as correct.
///
/// The steward found the same manoeuvre in production independently: 47 of 47 opens on L0 were
/// self-parented by a principal with no write grant on it, after being refused self-attributed.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn parent_cogmap_does_not_downgrade_the_open_gate(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "delegate@example.com").await;
    common::fixtures::approve_standing(&pool, profile).await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    // The exact production manoeuvre: read-only principal, parent == originating. Must be refused.
    let denied = backend
        .open_invocation(OpenInvocation {
            trigger_kind: "manual".to_string(),
            originating_cogmap: CogmapId::from(L0_COGMAP),
            parent_cogmap: Some(CogmapId::from(L0_COGMAP)),
            origin: Surface::ApiHttp,
        })
        .await;
    // Refused, in the reader's dialect — and the message is the point of this pairing. The bypass
    // this test guards was FOUND by an agent probing an opaque refusal; a refusal that names the
    // missing grant is what makes the probe pointless rather than merely blocked.
    assert_names_the_missing_capability(&denied, L0_COGMAP);

    // Self-parenting stays LEGAL, it is merely inert: with write, the identical call succeeds. The fix
    // removes the authorization discount, not the ability to record a (meaningless) self-binding.
    common::fixtures::grant_cogmap_write(&pool, L0_COGMAP, profile).await;
    backend
        .open_invocation(OpenInvocation {
            trigger_kind: "manual".to_string(),
            originating_cogmap: CogmapId::from(L0_COGMAP),
            parent_cogmap: Some(CogmapId::from(L0_COGMAP)),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("a self-parented open by an authoring principal must still succeed");
}
