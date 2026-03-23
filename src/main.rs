mod cli;

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

fn run(cli: Cli) -> temper_cli::error::Result<()> {
    match cli.command {
        Commands::Init { path, no_interactive } => {
            let vault_path = path
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
            temper_cli::commands::init::run(&vault_path, no_interactive)
        }
        Commands::Check { quiet } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::check::run(&config, quiet)
        }
        Commands::Status { verbose } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::status::run(&config, verbose)
        }
        Commands::Index { force, paths, sources } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::index::run(
                &config,
                force,
                paths.as_deref(),
                sources.as_deref(),
            )
        }
        Commands::Search { query, format, note_type, project, limit } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::search::run(
                &config,
                &query,
                &format,
                note_type.as_deref(),
                project.as_deref(),
                limit,
            )
        }
        Commands::Context { topic, format, depth, limit } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::context::run(
                &config,
                &topic,
                &format,
                depth,
                limit,
            )
        }
    }
}
