use anyhow::Result;
use clap::Parser;
use temper_next::synthesis::{self, RunOpts};
use temper_next::{embed::embed_chunks, substrate, write::materialize_cogmap};

/// `temper-next` harness binary. Two subcommands over the shared `temper_next` substrate.
#[derive(Parser)]
#[command(name = "temper-next")]
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
    /// Synthesize the `temper_next` substrate from current `public.*` state (WS6 §0). Explicitly
    /// invoked — never a migrate-time side effect (§D).
    Synthesize {
        /// Stop after N resources (rehearsal); 0 = all.
        #[arg(long, default_value_t = 0)]
        limit: usize,
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
            let outcome = materialize_cogmap(&pool, cogmap, &lens, emitter).await?;
            println!(
                "materialized {} region(s) for '{}' (lens '{}')\nmembership: {}",
                outcome.regions, name, lens, outcome.membership_fingerprint
            );
        }
        Cmd::Synthesize { limit } => {
            let report = synthesis::run(&pool, RunOpts { limit }).await?;
            println!(
                "synthesized: {} resource(s), {} property(ies), {} edge(s)",
                report.resources, report.properties, report.edges
            );
        }
    }
    Ok(())
}
