//! W-trail-correlation — the receipt-to-ledger pairing instrument. The reblock receipt
//! echoes a batch correlation id; the trail read over the reblocked resource must return
//! the act's event CARRYING that same id, or the playbook's validate step pairs nothing.
//! Bites against the unprojected trail: `kb_events.correlation_id` stored but absent from
//! the `element_trail_*` SELECT renders this witness compile-unbindable (mechanism absent),
//! and a projector that stamps NULL fails the assertion at runtime.
//!
//! The create event on the same trail carries the old-acts arm: `fire` self-roots when no
//! EventContext is threaded, so its correlation is its OWN event id — the two arms together
//! prove the read carries real threading, not any uuid.
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::element_trail::ElementKind;
use temper_core::types::ids::ProfileId;
use temper_core::types::reblock::ReblockScope;
use temper_services::backend::DbBackend;
use temper_services::services::event_service;
use temper_substrate::content::{prepare_block_from_chunks, IncomingChunk};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{EntityId, ProfileId as SubstrateProfileId};
use temper_substrate::payloads::AnchorRef;
use temper_workflow::operations::{Backend, ReblockResources, Surface};
use uuid::Uuid;

// ── fixtures (duplicated per file, per this suite's convention) ─────────────────────────────

async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (Uuid, Uuid, Uuid) {
    let profile_id = Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
        .execute(pool)
        .await
        .expect("seed profile");
    let mut web_entity = Uuid::now_v7();
    for surface in ["web", "cli", "mcp"] {
        let id = Uuid::now_v7();
        if surface == "web" {
            web_entity = id;
        }
        sqlx::query(
            "INSERT INTO kb_entities (id, profile_id, name, metadata) VALUES ($1,$2,$3,'{}'::jsonb)",
        )
        .bind(id)
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    let context_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id, web_entity)
}

fn incoming_of(text: &str) -> Vec<IncomingChunk> {
    temper_ingest::chunk::chunk_markdown(text)
        .into_iter()
        .map(|c| IncomingChunk {
            chunk_index: c.chunk_index as i32,
            content_hash: c.content_hash,
            content: c.content,
            embedding: vec![0.0; 768],
            embedded_with: None,
            header_path: c.header_path.clone(),
            heading_depth: c.heading_depth as i16,
        })
        .collect()
}

async fn fire_block_resource(
    pool: &PgPool,
    owner: Uuid,
    emitter: Uuid,
    context: Uuid,
    title: &str,
    body: &str,
) -> Uuid {
    let mut block = prepare_block_from_chunks(0, None, incoming_of(body));
    block.raw_text = Some(body.to_string());
    let blocks = [block];
    let mut conn = pool.acquire().await.unwrap();
    fire(
        &mut conn,
        SeedAction::ResourceCreate {
            title,
            origin_uri: &format!("temper://trail-corr/{title}"),
            resource_id: None,
            home: AnchorRef::context(temper_substrate::ids::ContextId::from(context)),
            owner: SubstrateProfileId::from(owner),
            originator: Some(SubstrateProfileId::from(owner)),
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter: EntityId::from(emitter),
            segmented: false,
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap()
    .uuid()
}

// ── the witness ──────────────────────────────────────────────────────────────────────────────

/// The trail read over a reblocked resource returns the act's `resource_reblocked` event
/// carrying the batch correlation id the receipt echoed; the create event on the same trail
/// (an act that self-rooted) carries none.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_trail_carries_the_batch_correlation_id_the_receipt_echoed(pool: PgPool) {
    let (owner, context, entity) = seed_profile_with_context(&pool, "owner@example.com").await;
    const BODY_A_B: &str = "# Alpha\n\nAlpha body paragraph.\n## Beta\n\nBeta body paragraph.\n";
    let resource =
        fire_block_resource(&pool, owner, entity, context, "corr-witness", BODY_A_B).await;

    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));
    let receipt = backend
        .reblock_resources(ReblockResources {
            scope: ReblockScope::Context(context),
            dry_run: false,
            limit: 10,
            after_id: None,
            origin: Surface::ApiHttp,
        })
        .await
        .unwrap()
        .value;
    assert_eq!(receipt.summary.reblocked, 1, "the act fired");

    let trail =
        event_service::element_trail(&pool, ProfileId::from(owner), ElementKind::Node, resource)
            .await
            .unwrap();

    let reblocked = trail
        .events
        .iter()
        .find(|e| e.kind == "resource_reblocked")
        .expect("the reblock act is on the trail");
    assert_eq!(
        reblocked.correlation_id,
        Some(receipt.correlation_id),
        "the trail event carries the batch correlation id the receipt echoed"
    );

    let created = trail
        .events
        .iter()
        .find(|e| e.kind == "resource_created")
        .expect("the create act is on the trail");
    // `fire` with no EventContext SELF-ROOTS (COALESCE(p_correlation, v_ev)): the act carries
    // its own event id, never a batch's. The two arms of this witness are the pairing proof —
    // the reblocked event rides the batch id (≠ its own event id), the self-rooted rides only
    // itself, so a projector stamping any uuid at all cannot pass.
    assert_eq!(
        created.correlation_id,
        Some(created.event_id),
        "a self-rooted act carries its own id, never a batch correlation"
    );
    assert_ne!(
        reblocked.correlation_id,
        Some(reblocked.event_id),
        "the batch-correlated act is not self-rooted — the id it carries came from the batch"
    );
}
