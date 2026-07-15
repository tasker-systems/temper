#![cfg(feature = "artifact-tests")]
//! WS5 slice 3b: the Readout drift tier, reached end-to-end. A content-only revision (block_mutated)
//! moves a member's chunk embedding — a readout input — WITHOUT changing any component's membership
//! inputs (affinity is declared-only; content is not in the component fingerprint). So `lens_drift`
//! reports `Readout`: something touched the map, but no component must re-cluster. This closes the gap
//! `drift_signal.rs` names ("the Readout tier is unit-proven in `drift`" — never reached end-to-end).
mod common;

use temper_core::types::home::HomeAnchor;
use temper_substrate::drift::{self, DriftTier};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{BlockId, EntityId};
use temper_substrate::scenario::{bootseed, loader, model::Seed};
use temper_substrate::{content, embed, write};
use uuid::Uuid;

const SEED: &str = r#"
name: readout-tier-test
cogmap:
  telos: { title: "Min", statement: "A tiny telos about onboarding.", questions: [{ question: "why?" }] }
  owner: alice
  emitter: "agent#1"
world:
  profiles: [{ handle: alice, display_name: Alice, system_access: approved }]
  entities: [{ name: "agent#1", profile: alice }]
resources:
  - { key: a, origin_uri: "temper://readout/a", home: cogmap, body: "alpha concept about deployment confidence" }
  - { key: b, origin_uri: "temper://readout/b", home: cogmap, body: "beta concept about deployment confidence" }
edges:
  - { from: a, to: b, kind: leads_to, label: then, weight: 1.0 }
  - { from: telos, to: a, kind: express, label: operationalized_by, weight: 1.0 }
uses_lenses: [telos-default]
"#;

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn revise_reaches_readout_tier_no_component_changes(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let seed: Seed = serde_yaml::from_str(SEED).unwrap();
    let loaded = loader::load_seed(&pool, &seed).await.unwrap();

    embed::embed_chunks(&pool).await.unwrap();
    write::materialize(
        &pool,
        HomeAnchor::Cogmap(loaded.cogmap.into()),
        "telos-default",
        loaded.emitter.into(),
    )
    .await
    .unwrap();

    // Fresh — nothing has touched the cogmap since the materialize.
    let (tier, diff) = drift::lens_drift(
        &pool,
        HomeAnchor::Cogmap(loaded.cogmap.into()),
        "telos-default",
    )
    .await
    .unwrap();
    assert_eq!(tier, DriftTier::Fresh, "fresh right after materialize");
    let prior = diff.unchanged.len();
    assert!(prior >= 1, "the seed materialized ≥1 component");

    // revise concept `a`'s body — a content-only change to a member's prose (no edge, no facet).
    let block_id: Uuid = sqlx::query_scalar(
        "SELECT b.id FROM kb_content_blocks b JOIN kb_resources r ON r.id=b.resource_id \
         WHERE r.origin_uri='temper://readout/a' AND NOT b.is_folded",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let prepared = content::prepare_block(
        0,
        None,
        "alpha concept — now entirely about quantum chromodynamics and lattice gauge theory",
    )
    .unwrap();
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::BlockMutate {
            incorporated: &[],
            block: BlockId::from(block_id),
            chunks: &prepared.chunks,
            raw: None,
            emitter: EntityId::from(loaded.emitter),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Readout — touched, but no component's membership inputs changed.
    let (tier2, diff2) = drift::lens_drift(
        &pool,
        HomeAnchor::Cogmap(loaded.cogmap.into()),
        "telos-default",
    )
    .await
    .unwrap();
    assert_eq!(
        tier2,
        DriftTier::Readout,
        "a body revision is a readout-tier change"
    );
    assert!(
        !diff2.has_structural_change(),
        "no component must re-cluster"
    );
    assert_eq!(
        diff2.unchanged.len(),
        prior,
        "every component stays provably current"
    );
    assert!(diff2.changed.is_empty() && diff2.stale.is_empty());
}
