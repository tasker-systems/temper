#![cfg(feature = "artifact-tests")]
//! WS5 drift-signal proof: the graded, lens-relative, component-scoped signal.
//!
//! Right after a materialize, `lens_drift` reports `Fresh` — nothing has touched the cogmap. Home a
//! new isolated concept (no edges, no shared facets ⇒ its own nonzero-affinity component) and the
//! signal becomes `Structural`, naming EXACTLY the one new component as changed while every prior
//! component stays provably unchanged. That component-scoping is the point: a refresh re-clusters only
//! the touched component, not the whole map (the over-trigger the binary event-recency staleness
//! suffered). The two-tier decision table itself (incl. the Readout tier) is unit-proven in `drift`.
mod common;

use temper_substrate::drift::{self, DriftTier};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{CogmapId, EntityId, ProfileId};
use temper_substrate::scenario::{bootseed, loader, model::Seed};
use temper_substrate::{content, embed, write};

const SEED: &str = r#"
name: drift-signal-test
cogmap:
  telos: { title: "Min", statement: "A tiny telos.", questions: [{ question: "why?" }] }
  owner: alice
  emitter: "agent#1"
world:
  profiles: [{ handle: alice, display_name: Alice, system_access: approved }]
  entities: [{ name: "agent#1", profile: alice }]
resources:
  - { key: a, origin_uri: "temper://drift/a", home: cogmap, body: "alpha body" }
  - { key: b, origin_uri: "temper://drift/b", home: cogmap, body: "beta body" }
edges:
  - { from: a, to: b, kind: leads_to, label: then, weight: 1.0 }
  - { from: telos, to: a, kind: express, label: operationalized_by, weight: 1.0 }
uses_lenses: [telos-default]
"#;

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn lens_drift_is_fresh_after_materialize_then_component_scoped_structural(
    pool: sqlx::PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let seed: Seed = serde_yaml::from_str(SEED).unwrap();
    let loaded = loader::load_seed(&pool, &seed).await.unwrap();

    embed::embed_chunks(&pool).await.unwrap();
    write::materialize_cogmap(
        &pool,
        loaded.cogmap.into(),
        "telos-default",
        loaded.emitter.into(),
    )
    .await
    .unwrap();

    // Fresh — just materialized, nothing has touched the cogmap since.
    let (tier, diff) = drift::lens_drift(&pool, loaded.cogmap, "telos-default")
        .await
        .unwrap();
    assert_eq!(tier, DriftTier::Fresh);
    assert!(!diff.has_structural_change());
    let prior_components = diff.unchanged.len();
    assert!(prior_components >= 1, "the seed materialized ≥1 component");

    // home a new isolated concept — no edges, no shared facets ⇒ a fresh singleton component.
    let mut tx = pool.begin().await.unwrap();
    let blocks = content::prepare_blocks(&[(
        None,
        "An isolated concept with no edges and no shared facets.",
    )])
    .unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceCreate {
            title: "isolated",
            origin_uri: "temper://drift/isolated",
            resource_id: None,
            home: temper_substrate::payloads::AnchorRef::cogmap(CogmapId::from(loaded.cogmap)),
            owner: ProfileId::from(loaded.owner),
            originator: None,
            blocks: &blocks,
            doc_type: None,
            emitter: EntityId::from(loaded.emitter),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Structural — and component-scoped: exactly the one new component is changed; every prior
    // component is provably unchanged; nothing is stale.
    let (tier2, diff2) = drift::lens_drift(&pool, loaded.cogmap, "telos-default")
        .await
        .unwrap();
    assert_eq!(tier2, DriftTier::Structural);
    assert_eq!(
        diff2.changed.len(),
        1,
        "only the new isolated component should need re-cluster"
    );
    assert_eq!(
        diff2.unchanged.len(),
        prior_components,
        "every prior component stays provably current"
    );
    assert!(diff2.stale.is_empty());
}
