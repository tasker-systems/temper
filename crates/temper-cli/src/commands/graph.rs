//! `temper graph` command dispatch.

use crate::actions::graph_build::{self, GraphBuildParams};
use crate::cli::GraphAction;
use crate::config::Config;
use crate::error::Result;
use crate::output;

pub fn run(config: &Config, action: GraphAction) -> Result<()> {
    match action {
        GraphAction::Build {
            context,
            dry_run,
            verbose,
        } => {
            let params = GraphBuildParams {
                context_filter: context,
                dry_run,
                verbose,
            };
            let report = graph_build::run(config, params)?;
            render_report(&report, dry_run, verbose);
            Ok(())
        }
    }
}

fn render_report(report: &graph_build::GraphBuildReport, dry_run: bool, verbose: bool) {
    let verb = if dry_run { "Would modify" } else { "Modified" };

    output::header(format!(
        "temper graph build — {} files walked",
        report.files_walked
    ));
    output::plain(format!(
        "  Pass 2 (scanning):    {} references found",
        report.references_found
    ));
    output::plain("  Pass 3 (merge):");
    output::plain(format!("    Files modified:     {}", report.files_modified));
    output::plain(format!(
        "    References added:   {}",
        report.references_added
    ));
    output::plain(format!(
        "    Already present:    {}",
        report.already_present
    ));

    if !report.modified_files.is_empty() {
        output::blank();
        output::plain(format!("{verb} files:"));
        for mf in &report.modified_files {
            output::plain(format!("  {}  (+{} references)", mf.rel_path, mf.added));
            if verbose {
                for r in &mf.added_refs {
                    output::plain(format!("    - {r}"));
                }
            }
        }
    }
}
