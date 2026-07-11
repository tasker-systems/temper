use anyhow::Result;
use clap::Parser;
use temper_core::types::home::HomeAnchor;
use temper_substrate::{embed::embed_chunks, substrate, write::materialize};

/// `temper-substrate` harness binary over the shared substrate.
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
            // a real referent, not "latest event". The genesis (`cogmap_seeded`) event is the earliest
            // map-anchored one.
            let emitter: uuid::Uuid = sqlx::query_scalar(
                "SELECT emitter_entity_id FROM kb_events \
                 WHERE producing_anchor_table='kb_cogmaps' AND producing_anchor_id=$1 \
                 ORDER BY occurred_at ASC LIMIT 1",
            )
            .bind(cogmap)
            .fetch_one(&pool)
            .await?;
            let outcome =
                materialize(&pool, HomeAnchor::Cogmap(cogmap), &lens, emitter.into()).await?;
            println!(
                "materialized {} region(s) for '{}' (lens '{}')\nmembership: {}",
                outcome.regions, name, lens, outcome.membership_fingerprint
            );
        }
    }
    Ok(())
}
