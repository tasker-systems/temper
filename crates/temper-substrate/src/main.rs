use anyhow::Result;
use clap::Parser;
use temper_core::types::home::HomeAnchor;
use temper_substrate::{embed::embed_chunks, migrate_ledger, substrate, write::materialize};

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
    /// Apply pending migrations, recording each apply's outcome in `kb_migration_ledger`.
    ///
    /// This is what `cargo make db-migrate` runs instead of `sqlx migrate run`. The difference is
    /// not the applying — that is still sqlx's `Migrator`, unchanged — it is that a migration which
    /// FAILS leaves a `failed` entry behind, and one that crashes mid-apply leaves a `pending` one
    /// that blocks the next run. Neither is recordable from inside a migration, because sqlx wraps
    /// the body and its bookkeeping in a single transaction that a failure rolls back.
    Migrate,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cmd = Cmd::parse();
    let pool = substrate::connect().await?;
    match cmd {
        Cmd::Migrate => {
            let mut conn = pool.acquire().await?;
            migrate_ledger::run_with_ledger(&temper_substrate::MIGRATOR, &mut conn).await?;
            println!("migrations applied; outcomes recorded in kb_migration_ledger");
        }
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
