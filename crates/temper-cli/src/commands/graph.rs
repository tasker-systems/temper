//! `temper graph` command dispatch.
//!
//! Thin wrapper over `actions::graph_build`. Unpacks clap flags, loads
//! the vault config, and delegates to the action.

use crate::cli::GraphAction;
use crate::config::Config;
use crate::error::Result;

pub fn run(config: &Config, action: GraphAction) -> Result<()> {
    match action {
        GraphAction::Build {
            context,
            dry_run,
            verbose,
        } => {
            let _ = (config, context, dry_run, verbose);
            Err(crate::error::TemperError::Project(
                "temper graph build: not yet implemented".into(),
            ))
        }
    }
}
