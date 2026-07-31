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
    Migrate {
        /// Apply only the leading run of migrations declared `additive`, and STOP at the first one
        /// that is shape-breaking, undeclared, or declared with a token outside the vocabulary.
        ///
        /// This is what the deploy runs (`scripts/vercel-build.sh`). Without it, every pending
        /// migration is applied — which is right for a developer and for an operator running a
        /// cutover, because in both of those taking a shape-breaking migration is a deliberate act
        /// rather than a side effect of some later, unrelated, additive deploy.
        ///
        /// Exits 3 when it halts, so a caller can tell "an operator must take this" apart from "a
        /// migration failed".
        #[arg(long)]
        additive_only: bool,
    },
}

/// The halt is a refusal, not an error, and a caller has to be able to tell them apart. Distinct
/// from 1 (anyhow's failure exit) for that reason.
const HALTED: i32 = 3;

#[tokio::main]
async fn main() -> Result<()> {
    let cmd = Cmd::parse();
    let pool = substrate::connect().await?;
    match cmd {
        Cmd::Migrate {
            additive_only: false,
        } => {
            let mut conn = pool.acquire().await?;
            migrate_ledger::run_with_ledger(&temper_substrate::MIGRATOR, &mut conn).await?;
            println!("migrations applied; outcomes recorded in kb_migration_ledger");
        }
        Cmd::Migrate {
            additive_only: true,
        } => {
            let mut conn = pool.acquire().await?;
            let run =
                migrate_ledger::run_additive_only(&temper_substrate::MIGRATOR, &mut conn).await?;

            if run.applied.is_empty() {
                println!("no additive migration was pending");
            } else {
                println!(
                    "applied {} additive migration(s): {}; outcomes recorded in kb_migration_ledger",
                    run.applied.len(),
                    run.applied
                        .iter()
                        .map(i64::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }

            // The halt is reported AFTER what was applied, because both are true and the operator
            // needs to know the schema moved before being told where it stopped.
            if let Some(halt) = run.halt {
                eprintln!();
                eprintln!("HALTED at migration {halt}");
                eprintln!();
                eprintln!(
                    "The deploy applies additive migrations only. This one is operator-gated: take it\n\
                     as a cutover (DEPLOYING.md § 'Shape-breaking migration'), then deploy again — the\n\
                     next run finds it already applied and continues past it.\n\
                     \n\
                     Nothing after it was applied. Halting is stopping, not skipping: a later migration\n\
                     applied over a schema its predecessor never made is the failure this prevents."
                );
                std::process::exit(HALTED);
            }
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
