#![cfg(feature = "artifact-tests-legacy")]
// tests/substrate_read.rs — requires the temper_next artifact loaded (Plan 1 + seed).
// RE-GROUND: confirm the seeded cogmap name and member count after Plan 3's enriched seed;
// until then this asserts against the CURRENT (sparse) seed.
#[tokio::test]
async fn loads_homed_concepts_and_edges_for_a_cogmap() {
    let pool = temper_substrate::substrate::connect().await.unwrap();
    let cogmap = temper_substrate::substrate::cogmap_by_name(&pool, "onboarding-cogmap")
        .await
        .unwrap();
    let s = temper_substrate::substrate::load(&pool, cogmap, "telos-default")
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
    let pool = temper_substrate::substrate::connect().await.unwrap();
    let cogmap = temper_substrate::substrate::cogmap_by_name(&pool, "onboarding-cogmap")
        .await
        .unwrap();
    // the seeded default lens loads by name
    temper_substrate::substrate::load(&pool, cogmap, "telos-default")
        .await
        .expect("telos-default lens loads by name");
    // a name with no matching lens row errors (fetch_one finds nothing) — the param is bound, not ignored
    let bogus = temper_substrate::substrate::load(&pool, cogmap, "no-such-lens").await;
    assert!(
        bogus.is_err(),
        "loading an unknown lens name must error, proving the name binds the query"
    );
}

// Enforce the "MUST mirror the seeded telos-default row" invariant that affinity.rs only asserts in a
// comment. Production reads the lens from the DB (substrate::load); Lens::telos_default() is the
// test-only twin. Without this check, tuning the seed lens silently desyncs the two.
#[tokio::test]
async fn seeded_telos_default_lens_mirrors_the_rust_default() {
    use temper_substrate::affinity::Lens;
    let pool = temper_substrate::substrate::connect().await.unwrap();
    let cogmap = temper_substrate::substrate::cogmap_by_name(&pool, "onboarding-cogmap")
        .await
        .unwrap();
    let s = temper_substrate::substrate::load(&pool, cogmap, "telos-default")
        .await
        .unwrap();
    let d = Lens::telos_default();
    assert_eq!(s.lens.w_express, d.w_express, "w_express");
    assert_eq!(s.lens.w_contains, d.w_contains, "w_contains");
    assert_eq!(s.lens.w_leads_to, d.w_leads_to, "w_leads_to");
    assert_eq!(s.lens.w_near, d.w_near, "w_near");
    assert_eq!(s.lens.w_prop, d.w_prop, "w_prop");
    assert_eq!(s.lens.s_telos, d.s_telos, "s_telos");
    assert_eq!(s.lens.s_ref, d.s_ref, "s_ref");
    assert_eq!(s.lens.s_central, d.s_central, "s_central");
    assert_eq!(s.lens.resolution, d.resolution, "resolution");
}
