use anyhow::Result;
use clap::Parser;
use temper_migrate::{connect, ledger, MIGRATOR};

/// Apply pending migrations, recording each apply's outcome in `kb_migration_ledger`.
///
/// This is what `cargo make db-migrate` and the deploy's build command both run instead of
/// `sqlx migrate run`. The difference is not the applying — that is still sqlx's `Migrator`,
/// unchanged — it is that a migration which FAILS leaves a `failed` entry behind, and one that
/// crashes mid-apply leaves a `pending` one that blocks the next run. Neither is recordable from
/// inside a migration, because sqlx wraps the body and its bookkeeping in a single transaction that
/// a failure rolls back.
#[derive(Parser)]
#[command(name = "temper-migrate")]
struct Cli {
    /// Apply only the leading run of migrations declared `additive`, and STOP at the first one that
    /// is shape-breaking, undeclared, or declared with a token outside the vocabulary.
    ///
    /// This is what the deploy runs (`scripts/vercel-build.sh`). Without it, every pending
    /// migration is applied — which is right for a developer and for an operator running a cutover,
    /// because in both of those taking a shape-breaking migration is a deliberate act rather than a
    /// side effect of some later, unrelated, additive deploy.
    ///
    /// Exits 3 when it halts, so a caller can tell "an operator must take this" apart from "a
    /// migration failed".
    #[arg(long)]
    additive_only: bool,
}

/// The halt is a refusal, not an error, and a caller has to be able to tell them apart. Distinct
/// from 1 (anyhow's failure exit) for that reason.
const HALTED: i32 = 3;

/// The carried migration set and the database disagree about history. Distinct from 1 because the
/// operator's next move is different and the ledger has nothing to say about it — see
/// [`ledger::divergent_version`].
const DIVERGED: i32 = 4;

/// Exit on the divergence class if that is what this is; otherwise hand the error back unchanged.
///
/// Written as a `Result` pass-through rather than a branch at each call site so the two run modes
/// cannot drift — the full run and the additive run reach `run_direct`'s validation identically.
fn classify<T>(outcome: Result<T, sqlx::migrate::MigrateError>) -> Result<T> {
    let err = match outcome {
        Ok(value) => return Ok(value),
        Err(err) => err,
    };

    let Some(version) = ledger::divergent_version(&err) else {
        return Err(err.into());
    };

    eprintln!("{err}");
    eprintln!();
    eprintln!("DIVERGED at migration {version}");
    eprintln!();
    eprintln!(
        "The database has applied a migration that does not match the set this binary carries.\n\
         Nothing was applied and nothing failed — sqlx refuses before touching the schema, so\n\
         `kb_migration_ledger` has no entry for this and `migration_current` will show nothing\n\
         unusual. Do not go looking there.\n\
         \n\
         The usual causes, in the order they actually occur:\n\
         \n\
         * two branches claimed the same migration version, and the other one merged first;\n\
         * a shipped migration was edited or renumbered — sqlx checksums applied migrations, so\n\
           any change to one already applied is permanent and must be superseded, never amended;\n\
         * this environment's database was branched from one carrying a version this build lacks.\n\
         \n\
         On a PREVIEW the cheap fix is usually to renumber this branch's migration above the one\n\
         that merged, and let the preview's database be re-created. On PRODUCTION, stop: a\n\
         mismatch there means a shipped migration changed under a database that already ran it."
    );
    std::process::exit(DIVERGED);
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let pool = connect().await?;
    let mut conn = pool.acquire().await?;

    if !cli.additive_only {
        classify(ledger::run_with_ledger(&MIGRATOR, &mut conn).await)?;
        println!("migrations applied; outcomes recorded in kb_migration_ledger");
        return Ok(());
    }

    let run = classify(ledger::run_additive_only(&MIGRATOR, &mut conn).await)?;

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

    // The halt is reported AFTER what was applied, because both are true and the operator needs to
    // know the schema moved before being told where it stopped.
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

    Ok(())
}
