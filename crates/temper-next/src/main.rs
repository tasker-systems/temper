use anyhow::Result;
use temper_next::{embed::embed_all_blocks, substrate, write::materialize_cogmap};

/// Harness entry point (spec §1): connect to the `temper_next` artifact, run Job A (embed content
/// blocks) then Job B (materialize the cogmap's emergent telos-lens regions). Cogmap name is the
/// first CLI arg (default `onboarding-cogmap`).
#[tokio::main]
async fn main() -> Result<()> {
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "onboarding-cogmap".to_string());

    let pool = substrate::connect().await?;
    embed_all_blocks(&pool).await?;
    let cogmap = substrate::cogmap_by_name(&pool, &name).await?;
    let outcome = materialize_cogmap(&pool, cogmap).await?;

    println!(
        "materialized {} region(s) for '{}'\nmembership: {}",
        outcome.regions, name, outcome.membership_fingerprint
    );
    Ok(())
}
