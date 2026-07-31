use anyhow::Result;
use clap::Parser;
use temper_core::types::home::HomeAnchor;
use temper_substrate::{embed::embed_chunks, substrate, write::materialize};

/// `temper-substrate` harness binary over the shared substrate.
///
/// The `migrate` command used to live here. It moved to the `temper-migrate` binary, whose crate
/// depends on sqlx and nothing heavy — this one pulls ort in through `temper-ingest(embed)`, and
/// the deploy compiles the migrate binary before it applies any schema.
#[derive(Parser)]
#[command(name = "temper-substrate")]
enum Cmd {
    /// Embed content blocks then materialize a cogmap's emergent telos-lens regions (spec §1). The
    /// lens name selects the region-set over the same substrate (S6f plurality).
    Materialize {
        /// Cogmap name to materialize.
        #[arg(default_value = "onboarding-cogmap")]
        cogmap: String,
        /// Lens name (e.g. `telos-default`, `telos-default-propheavy`).
        #[arg(default_value = "telos-default")]
        lens: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cmd = Cmd::parse();
    let pool = substrate::connect().await?;
    match cmd {
        Cmd::Materialize { cogmap: name, lens } => {
            embed_chunks(&pool).await?;
            let cogmap = substrate::cogmap_by_name(&pool, &name).await?;
            // Materialization is attributed to the entity that seeded this cogmap (its bound steward) —
            // a real referent, not "latest event".
            let emitter = substrate::cogmap_genesis_emitter(&pool, cogmap).await?;
            let outcome = materialize(&pool, HomeAnchor::Cogmap(cogmap), &lens, emitter).await?;
            println!(
                "materialized {} region(s) for '{}' (lens '{}')\nmembership: {}",
                outcome.regions, name, lens, outcome.membership_fingerprint
            );
        }
    }
    Ok(())
}
