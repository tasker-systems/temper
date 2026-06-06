// tests/substrate_read.rs — requires the temper_next artifact loaded (Plan 1 + seed).
// RE-GROUND: confirm the seeded cogmap name and member count after Plan 3's enriched seed;
// until then this asserts against the CURRENT (sparse) seed.
#[tokio::test]
async fn loads_homed_concepts_and_edges_for_a_cogmap() {
    let pool = temper_next::substrate::connect().await.unwrap();
    let cogmap = temper_next::substrate::cogmap_by_name(&pool, "onboarding-cogmap")
        .await
        .unwrap();
    let s = temper_next::substrate::load(&pool, cogmap).await.unwrap();
    assert!(!s.nodes.is_empty(), "expected ≥1 homed concept-resource");
    // edges/facets may be empty in the sparse seed; structure must load without error.
}
