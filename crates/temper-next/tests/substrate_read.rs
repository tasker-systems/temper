// tests/substrate_read.rs — requires the temper_next artifact loaded (Plan 1 + seed).
// RE-GROUND: confirm the seeded cogmap name and member count after Plan 3's enriched seed;
// until then this asserts against the CURRENT (sparse) seed.
#[tokio::test]
async fn loads_homed_concepts_and_edges_for_a_cogmap() {
    let pool = temper_next::substrate::connect().await.unwrap();
    let cogmap = temper_next::substrate::cogmap_by_name(&pool, "onboarding-cogmap")
        .await
        .unwrap();
    let s = temper_next::substrate::load(&pool, cogmap, "telos-default")
        .await
        .unwrap();
    assert!(!s.nodes.is_empty(), "expected ≥1 homed concept-resource");
    // edges/facets may be empty in the sparse seed; structure must load without error.
}

// Plan 3 Step 0: `load` takes a lens name (default "telos-default" at the callers). The name must
// actually bind the lens query — a bogus name resolves to no lens row and errors, proving the
// parameter is honored rather than ignored. S6f plurality depends on materializing
// `telos-default-propheavy` over the same substrate, which requires this binding.
#[tokio::test]
async fn lens_name_parameter_binds_the_lens_query() {
    let pool = temper_next::substrate::connect().await.unwrap();
    let cogmap = temper_next::substrate::cogmap_by_name(&pool, "onboarding-cogmap")
        .await
        .unwrap();
    // the seeded default lens loads by name
    temper_next::substrate::load(&pool, cogmap, "telos-default")
        .await
        .expect("telos-default lens loads by name");
    // a name with no matching lens row errors (fetch_one finds nothing) — the param is bound, not ignored
    let bogus = temper_next::substrate::load(&pool, cogmap, "no-such-lens").await;
    assert!(
        bogus.is_err(),
        "loading an unknown lens name must error, proving the name binds the query"
    );
}
