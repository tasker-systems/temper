#![cfg(feature = "artifact-tests")]
//! Cogmap-homed kernel writes: `create_kernel_resource` homes a resource to the cogmap (not a
//! context), `set_property` stamps the per-key `provenance: kernel`, `set_facet` writes the
//! clustering `layer` facet, and `assert_kernel_edge` homes an edge to the cogmap. Read back through
//! `kernel_slice` (which filters `property_key='provenance' = 'kernel'`) to confirm both kernel
//! resources land, keyed by `origin_uri`. Runs against an isolated `#[sqlx::test]` ephemeral DB
//! (MIGRATOR-applied canonical schema + seed).

use temper_substrate::affinity::EdgeKind;
use temper_substrate::ids::{CogmapId, EntityId, ProfileId};
use temper_substrate::payloads::EdgePolarity;
use temper_substrate::scenario::{loader, model::Seed};
use temper_substrate::writes::{self, KernelCreateParams, KernelEdgeParams};

/// A minimal seed that just genesises a cogmap (with telos) and the system world; no scenario
/// resources — the kernel resources are created directly through the new wrappers under test.
const SEED: &str = r#"
name: kernel-homed-writes-test
cogmap:
  telos: { title: "K", statement: "Kernel telos.", questions: [{ question: "why?" }] }
  owner: alice
  emitter: "agent#1"
world:
  profiles: [{ handle: alice, display_name: Alice, system_access: approved }]
  entities: [{ name: "agent#1", profile: alice }]
resources: []
uses_lenses: [telos-default]
"#;

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn create_kernel_resource_homes_to_cogmap_with_facet_and_edge(pool: sqlx::PgPool) {
    // The MIGRATOR ephemeral DB already carries the canonical seed (the `system` profile/entity); the
    // scenario seed below genesises the cogmap.
    let s: Seed = serde_yaml::from_str(SEED).unwrap();
    let loaded = loader::load_seed(&pool, &s).await.unwrap();
    let cogmap = CogmapId::from(loaded.cogmap);

    // The boot-seeded canonical system actor is the kernel content owner + emitter.
    let profile: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let entity: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(&pool)
            .await
            .unwrap();
    let owner = ProfileId::from(profile);
    let emitter = EntityId::from(entity);

    // Resource A — homed to the cogmap, stamped provenance:kernel + clustering layer facet.
    let a = writes::create_kernel_resource(
        &pool,
        KernelCreateParams {
            cogmap,
            resource_id: uuid::Uuid::now_v7(),
            title: "cogmap",
            origin_uri: "temper://kernel/concept/cogmap",
            doc_type: "kernel_landmark",
            body: "A cognitive map.",
            chunks: None,
            owner,
            emitter,
        },
    )
    .await
    .unwrap();
    writes::set_property(
        &pool,
        a,
        "provenance",
        &serde_json::json!("kernel"),
        emitter,
    )
    .await
    .unwrap();
    writes::set_facet(
        &pool,
        a,
        &serde_json::json!({ "layer": "concept" }),
        1.0,
        emitter,
    )
    .await
    .unwrap();

    // Resource B — a second kernel landmark, also provenance:kernel.
    let b = writes::create_kernel_resource(
        &pool,
        KernelCreateParams {
            cogmap,
            resource_id: uuid::Uuid::now_v7(),
            title: "telos",
            origin_uri: "temper://kernel/concept/telos",
            doc_type: "kernel_landmark",
            body: "The governing telos.",
            chunks: None,
            owner,
            emitter,
        },
    )
    .await
    .unwrap();
    writes::set_property(
        &pool,
        b,
        "provenance",
        &serde_json::json!("kernel"),
        emitter,
    )
    .await
    .unwrap();

    // A cogmap-homed edge A → B.
    let edge = writes::assert_kernel_edge(
        &pool,
        KernelEdgeParams {
            cogmap,
            src: a,
            tgt: b,
            kind: EdgeKind::Express,
            polarity: EdgePolarity::Forward,
            label: Some("governs"),
            weight: 1.0,
            emitter,
        },
    )
    .await
    .unwrap();
    assert!(!edge.uuid().is_nil(), "edge id minted");

    // kernel_slice returns exactly the two provenance:kernel resources, keyed by origin_uri (the
    // genesis telos has no provenance property and is excluded).
    let slice = temper_substrate::readback::kernel_slice(&pool, cogmap)
        .await
        .unwrap();
    assert_eq!(slice.len(), 2, "both kernel resources in the slice");
    let uris: Vec<_> = slice.iter().map(|r| r.origin_uri.as_str()).collect();
    assert!(uris.contains(&"temper://kernel/concept/cogmap"));
    assert!(uris.contains(&"temper://kernel/concept/telos"));
    assert!(
        slice.iter().all(|r| r.body_hash.is_some()),
        "body_hash populated"
    );
    // Decision #6: the two mechanisms land under distinct keys in the merged property object.
    // `set_property` (per-key) → top-level `provenance`; `set_facet` (clustering) → one
    // `property_key='facet'` row holding the whole `{layer: concept}` object.
    let a_row = slice
        .iter()
        .find(|r| r.origin_uri == "temper://kernel/concept/cogmap")
        .unwrap();
    assert_eq!(
        a_row.facets.get("provenance").and_then(|v| v.as_str()),
        Some("kernel"),
        "provenance is a per-key property"
    );
    assert_eq!(
        a_row
            .facets
            .get("facet")
            .and_then(|f| f.get("layer"))
            .and_then(|v| v.as_str()),
        Some("concept"),
        "the clustering facet nests under property_key='facet'"
    );
}
