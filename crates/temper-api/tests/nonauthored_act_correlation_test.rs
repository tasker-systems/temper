#![cfg(feature = "test-db")]
//! Chunk 3 — invocation correlation + authorship through the **non-authored** `DbBackend` write
//! methods (update / delete / retype / reweight), the follow-on to the authored-act spine
//! (`act_authorship_test.rs`). Mirrors that harness: an act under an open invocation stamps
//! `kb_events.invocation_id` + the authorship `metadata`; the additive correlation-integrity gate
//! rejects an act claiming an unknown/unreadable invocation (404) or a non-open run (409); and an act
//! with no invocation leaves the correlator NULL (projection-invisible baseline). The four non-authored
//! sub-events of an `update` fan-out share one `ctx.clone()` threading in `update_resource_in_tx`, so
//! the `property_set` assertion here proves that path; the body-only (`block_mutated`) + rehome
//! correlations are exercised end-to-end through the real surfaces in the e2e suite.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::authorship::{ActContext, AgentAuthorship, ConfidenceBand};
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, EdgeId, InvocationId, ProfileId, ResourceId};
use temper_core::types::invocation::Disposition;
use temper_services::backend::DbBackend;
use temper_workflow::operations::{
    AssertRelationship, Backend, CloseInvocation, CreateResource, DeleteResource, OpenInvocation,
    RetypeRelationship, ReweightRelationship, Surface, UpdateResource,
};
use temper_workflow::types::managed_meta::ManagedMeta;

mod common;

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// Approve the profile so it joins the `temper-system` root team that owns L0 — making the kernel map
/// readable (so it can open + correlate invocations on L0). Same shape as `act_authorship_test`.
async fn approved_backend(pool: &PgPool, email: &str) -> (DbBackend, ContextId) {
    let (profile, context) = common::fixtures::create_test_profile_with_context(pool, email).await;
    sqlx::query("UPDATE kb_profiles SET system_access = 'approved' WHERE id = $1")
        .bind(profile)
        .execute(pool)
        .await
        .expect("approve test profile");
    // Self-attributed invocation-open on L0 now requires WRITE (F2) — grant it so `open_inv` succeeds.
    common::fixtures::grant_cogmap_write(pool, L0_COGMAP, profile).await;
    (
        DbBackend::new(pool.clone(), ProfileId::from(profile)),
        ContextId::from(context),
    )
}

fn sample_authorship() -> AgentAuthorship {
    AgentAuthorship {
        reasoning: Some("ACT_SENTINEL".into()),
        confidence: ConfidenceBand::Probable,
        rationale: None,
        persona: Some("steward".into()),
        model: None,
    }
}

fn stamped(inv: InvocationId) -> ActContext {
    ActContext {
        invocation: Some(inv),
        authorship: Some(sample_authorship()),
    }
}

fn create_cmd(context: ContextId, slug: &str) -> CreateResource {
    CreateResource {
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home: HomeAnchor::Context(context),
        title: format!("Nonauthored test {slug}"),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: Some(format!("test://nonauth-{slug}")),
        chunks_packed: None,
        content_hash: None,
        goal: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

async fn open_inv(backend: &DbBackend) -> InvocationId {
    let out = backend
        .open_invocation(OpenInvocation {
            trigger_kind: "manual".to_string(),
            originating_cogmap: temper_core::types::ids::CogmapId::from(L0_COGMAP),
            parent_cogmap: None,
            origin: Surface::ApiHttp,
        })
        .await
        .expect("open against readable L0 must succeed");
    InvocationId::from(out.value)
}

async fn make_resource(backend: &DbBackend, context: ContextId, slug: &str) -> ResourceId {
    backend
        .create_resource(create_cmd(context, slug))
        .await
        .unwrap_or_else(|e| panic!("create {slug}: {e:?}"))
        .value
        .id
}

/// `(invocation_id, metadata)` of the event of `type_name` correlated to `inv` (the stamped one — a
/// create fires its own un-stamped `property_set`, so filtering on `invocation_id` isolates ours).
async fn stamped_event(
    pool: &PgPool,
    type_name: &str,
    inv: InvocationId,
) -> (Option<Uuid>, serde_json::Value) {
    sqlx::query_as(
        "SELECT e.invocation_id, e.metadata \
           FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = $1 AND e.invocation_id = $2",
    )
    .bind(type_name)
    .bind(inv.uuid())
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("the stamped `{type_name}` event must exist: {e}"))
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_under_invocation_stamps_the_property_act(pool: PgPool) {
    let (backend, context) = approved_backend(&pool, "update@example.com").await;
    let resource = make_resource(&backend, context, "to-update").await;
    let inv = open_inv(&backend).await;

    // An open_meta-only update fires a `property_set` sub-event of the update fan-out.
    backend
        .update_resource(UpdateResource {
            resource,
            title: None,
            slug: None,
            body: None,
            managed_meta: None,
            open_meta: Some(serde_json::json!({ "reviewed_by": "qa" })),
            move_to: None,
            context_ref: None,
            goal: None,
            act: stamped(inv),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("update under an open invocation must succeed");

    let (got_inv, meta) = stamped_event(&pool, "property_set", inv).await;
    assert_eq!(
        got_inv,
        Some(inv.uuid()),
        "property_set carries the invocation"
    );
    assert_eq!(
        meta["reasoning"], "ACT_SENTINEL",
        "authorship in metadata: {meta}"
    );
    assert_eq!(meta["confidence"], "probable", "graded confidence: {meta}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn delete_under_invocation_stamps_the_delete_act(pool: PgPool) {
    let (backend, context) = approved_backend(&pool, "delete@example.com").await;
    let resource = make_resource(&backend, context, "to-delete").await;
    let inv = open_inv(&backend).await;

    backend
        .delete_resource(DeleteResource {
            resource,
            force: false,
            act: stamped(inv),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("delete under an open invocation must succeed");

    let (got_inv, meta) = stamped_event(&pool, "resource_deleted", inv).await;
    assert_eq!(
        got_inv,
        Some(inv.uuid()),
        "resource_deleted carries the invocation"
    );
    assert_eq!(
        meta["confidence"], "probable",
        "authorship in metadata: {meta}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retype_and_reweight_under_invocation_stamp_the_edge_acts(pool: PgPool) {
    let (backend, context) = approved_backend(&pool, "edge@example.com").await;
    let src = make_resource(&backend, context, "src").await;
    let tgt = make_resource(&backend, context, "tgt").await;
    let edge = backend
        .assert_relationship(AssertRelationship {
            source: src,
            target: tgt,
            edge_kind: EdgeKind::Near,
            polarity: Polarity::Forward,
            label: "relates".to_string(),
            weight: 1.0,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("assert edge")
        .value;

    let inv = open_inv(&backend).await;

    backend
        .retype_relationship(RetypeRelationship {
            edge_handle: EdgeId::from(Uuid::from(edge)),
            edge_kind: EdgeKind::LeadsTo,
            polarity: Polarity::Forward,
            act: stamped(inv),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("retype under an open invocation must succeed");
    backend
        .reweight_relationship(ReweightRelationship {
            edge_handle: EdgeId::from(Uuid::from(edge)),
            weight: 0.5,
            act: stamped(inv),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("reweight under an open invocation must succeed");

    let (retype_inv, _) = stamped_event(&pool, "relationship_retyped", inv).await;
    assert_eq!(
        retype_inv,
        Some(inv.uuid()),
        "relationship_retyped carries the invocation"
    );
    let (reweight_inv, meta) = stamped_event(&pool, "relationship_reweighted", inv).await;
    assert_eq!(
        reweight_inv,
        Some(inv.uuid()),
        "relationship_reweighted carries the invocation"
    );
    assert_eq!(
        meta["confidence"], "probable",
        "edge act authorship in metadata: {meta}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn nonauthored_act_claiming_unknown_invocation_is_not_found(pool: PgPool) {
    let (backend, context) = approved_backend(&pool, "ghost@example.com").await;
    let resource = make_resource(&backend, context, "ghost").await;
    let act = ActContext {
        invocation: Some(InvocationId::from(Uuid::now_v7())),
        authorship: None,
    };
    let result = backend
        .update_resource(UpdateResource {
            resource,
            title: None,
            slug: None,
            body: None,
            managed_meta: None,
            open_meta: Some(serde_json::json!({ "k": "v" })),
            move_to: None,
            context_ref: None,
            goal: None,
            act,
            origin: Surface::ApiHttp,
        })
        .await;
    assert!(
        matches!(result, Err(TemperError::NotFound(_))),
        "claiming an unknown/unreadable invocation must be NotFound (404): {result:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn nonauthored_act_on_closed_invocation_is_conflict(pool: PgPool) {
    let (backend, context) = approved_backend(&pool, "closed@example.com").await;
    let resource = make_resource(&backend, context, "closed").await;
    let inv = open_inv(&backend).await;
    backend
        .close_invocation(CloseInvocation {
            invocation: inv.uuid(),
            disposition: Disposition::Completed,
            outcome: serde_json::json!({}),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("close must succeed");

    let result = backend
        .delete_resource(DeleteResource {
            resource,
            force: false,
            act: ActContext {
                invocation: Some(inv),
                authorship: None,
            },
            origin: Surface::ApiHttp,
        })
        .await;
    assert!(
        matches!(result, Err(TemperError::Conflict(_))),
        "stamping an act onto a closed run must be a Conflict (409): {result:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_without_act_leaves_invocation_null(pool: PgPool) {
    let (backend, context) = approved_backend(&pool, "plain@example.com").await;
    let resource = make_resource(&backend, context, "plain").await;

    backend
        .update_resource(UpdateResource {
            resource,
            title: None,
            slug: None,
            body: None,
            managed_meta: None,
            open_meta: Some(serde_json::json!({ "reviewed_by": "qa" })),
            move_to: None,
            context_ref: None,
            goal: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("an un-attributed update must still succeed");

    // The most-recent property_set (ours — the create's own property_set came earlier) has NULL
    // correlator and empty metadata: byte-identical to pre-feature behaviour.
    let (got_inv, meta): (Option<Uuid>, serde_json::Value) = sqlx::query_as(
        "SELECT e.invocation_id, e.metadata \
           FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'property_set' ORDER BY e.occurred_at DESC, e.id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("the property_set event must exist");
    assert_eq!(
        got_inv, None,
        "no invocation correlator on an un-attributed act"
    );
    assert_eq!(
        meta,
        serde_json::json!({}),
        "no authorship metadata: {meta}"
    );
}
