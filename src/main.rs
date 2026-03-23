mod cli;
mod error;

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        eprintln!("temper: {e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> error::Result<()> {
    match cli.command {
        Commands::Init { .. } => {
            eprintln!("temper init: not yet implemented");
            Ok(())
        }
        Commands::Check { .. } => {
            eprintln!("temper check: not yet implemented");
            Ok(())
        }
    }
}
