#![cfg(feature = "test-db")]
//! Chunk A spine — per-act authorship + invocation correlation through `DbBackend` writes.
//!
//! Exercises the backend write methods directly (the same approach as `invocation_handler_test`):
//! an authored act under an open invocation stamps `kb_events.invocation_id` + the authorship
//! `metadata`; the additive correlation-integrity gate rejects an act claiming an unknown/unreadable
//! invocation (404) or a non-open run (409); and an act with no invocation leaves the event
//! correlator NULL (the projection-invisible baseline). No caller surface supplies the `ActContext`
//! yet — that is Chunks B/C — so the command is built directly here.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::authorship::{ActContext, AgentAuthorship, ConfidenceBand};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, InvocationId, ProfileId};
use temper_core::types::invocation::Disposition;
use temper_services::backend::DbBackend;
use temper_workflow::operations::{
    AssertRelationship, Backend, CloseInvocation, CreateResource, OpenInvocation, Surface,
};
use temper_workflow::types::managed_meta::ManagedMeta;

mod common;

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// Approve the profile so the `sync_system_membership` trigger joins it to the `temper-system` root
/// team that owns L0 — making the kernel map readable (so it can open + correlate invocations on L0).
async fn approved_backend(pool: &PgPool, email: &str) -> (DbBackend, ProfileId, ContextId) {
    let (profile, context) = common::fixtures::create_test_profile_with_context(pool, email).await;
    sqlx::query("UPDATE kb_profiles SET system_access = 'approved' WHERE id = $1")
        .bind(profile)
        .execute(pool)
        .await
        .expect("approve test profile");
    // Self-attributed invocation-open on L0 now requires WRITE (F2), not just the read root-join
    // confers — grant it so `open_inv` succeeds.
    common::fixtures::grant_cogmap_write(pool, L0_COGMAP, profile).await;
    let profile_id = ProfileId::from(profile);
    (
        DbBackend::new(pool.clone(), profile_id),
        profile_id,
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

fn create_cmd(context: ContextId, slug: &str, act: ActContext) -> CreateResource {
    CreateResource {
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home: HomeAnchor::Context(context),
        title: format!("Act test {slug}"),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: Some(format!("test://act-{slug}")),
        chunks_packed: None,
        content_hash: None,
        goal: None,
        act,
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

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn authored_create_under_invocation_stamps_metadata_and_invocation(pool: PgPool) {
    let (backend, _profile, context) = approved_backend(&pool, "author@example.com").await;
    let inv = open_inv(&backend).await;

    let act = ActContext {
        invocation: Some(inv),
        authorship: Some(sample_authorship()),
    };
    backend
        .create_resource(create_cmd(context, "stamped", act))
        .await
        .expect("create under an open invocation must succeed");

    // The authored `resource_created` event carries the invocation correlator + authorship metadata.
    let (got_inv, meta): (Option<Uuid>, serde_json::Value) = sqlx::query_as(
        "SELECT e.invocation_id, e.metadata \
           FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_created' AND e.invocation_id = $1",
    )
    .bind(inv.uuid())
    .fetch_one(&pool)
    .await
    .expect("the stamped resource_created event must exist");

    assert_eq!(
        got_inv,
        Some(inv.uuid()),
        "invocation_id stamped on the act"
    );
    assert_eq!(
        meta["reasoning"], "ACT_SENTINEL",
        "authorship reasoning in metadata: {meta}"
    );
    assert_eq!(
        meta["confidence"], "probable",
        "graded confidence in metadata: {meta}"
    );
    assert_eq!(meta["persona"], "steward", "persona in metadata: {meta}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn authored_assert_under_invocation_stamps_the_edge_act(pool: PgPool) {
    let (backend, _profile, context) = approved_backend(&pool, "edge@example.com").await;
    let inv = open_inv(&backend).await;

    // Two resources to connect (un-stamped — only the edge act is under test).
    let src = backend
        .create_resource(create_cmd(context, "src", ActContext::default()))
        .await
        .expect("src create")
        .value
        .id;
    let tgt = backend
        .create_resource(create_cmd(context, "tgt", ActContext::default()))
        .await
        .expect("tgt create")
        .value
        .id;

    backend
        .assert_relationship(AssertRelationship {
            source: src,
            target: tgt,
            edge_kind: temper_core::types::graph::EdgeKind::Near,
            polarity: temper_core::types::graph::Polarity::Forward,
            label: "relates".to_string(),
            weight: 1.0,
            act: ActContext {
                invocation: Some(inv),
                authorship: Some(sample_authorship()),
            },
            origin: Surface::ApiHttp,
        })
        .await
        .expect("assert under an open invocation must succeed");

    let (got_inv, meta): (Option<Uuid>, serde_json::Value) = sqlx::query_as(
        "SELECT e.invocation_id, e.metadata \
           FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'relationship_asserted' AND e.invocation_id = $1",
    )
    .bind(inv.uuid())
    .fetch_one(&pool)
    .await
    .expect("the stamped relationship_asserted event must exist");
    assert_eq!(got_inv, Some(inv.uuid()));
    assert_eq!(
        meta["confidence"], "probable",
        "edge act authorship in metadata: {meta}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn act_claiming_unknown_invocation_is_not_found(pool: PgPool) {
    let (backend, _profile, context) = approved_backend(&pool, "ghost@example.com").await;
    // A random invocation id the caller cannot read (it does not exist) → uniform 404, no oracle.
    let act = ActContext {
        invocation: Some(InvocationId::from(Uuid::now_v7())),
        authorship: None,
    };
    let result = backend
        .create_resource(create_cmd(context, "ghost", act))
        .await;
    assert!(
        matches!(result, Err(TemperError::NotFound(_))),
        "claiming an unknown/unreadable invocation must be NotFound (404): {result:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn act_on_closed_invocation_is_conflict(pool: PgPool) {
    let (backend, _profile, context) = approved_backend(&pool, "closed@example.com").await;
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

    let act = ActContext {
        invocation: Some(inv),
        authorship: None,
    };
    let result = backend
        .create_resource(create_cmd(context, "after-close", act))
        .await;
    assert!(
        matches!(result, Err(TemperError::Conflict(_))),
        "stamping an act onto a closed run must be a Conflict (409): {result:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_without_act_leaves_invocation_null(pool: PgPool) {
    let (backend, _profile, context) = approved_backend(&pool, "plain@example.com").await;
    // The default (empty) ActContext — a keyboard-holder act: no correlation, no authorship.
    backend
        .create_resource(create_cmd(context, "plain", ActContext::default()))
        .await
        .expect("an un-attributed create must still succeed");

    // The event correlator is NULL and the metadata carries no authorship — the projection-invisible
    // baseline (a no-act write is byte-identical to pre-feature behaviour).
    let (got_inv, meta): (Option<Uuid>, serde_json::Value) = sqlx::query_as(
        "SELECT e.invocation_id, e.metadata \
           FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_created' AND e.producing_anchor_id IS NOT NULL \
          ORDER BY e.occurred_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("the resource_created event must exist");
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
