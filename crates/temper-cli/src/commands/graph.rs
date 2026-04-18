//! `temper graph` command dispatch.

use crate::actions::graph_build::{self, GraphBuildParams};
use crate::actions::graph_index::{self, GraphIndexParams};
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
        GraphAction::Index {
            context,
            dry_run,
            verbose,
        } => {
            let params = GraphIndexParams {
                context_filter: context,
                dry_run,
                verbose,
            };
            let report = graph_index::run(config, params)?;
            render_graph_index_report(&report, dry_run, verbose);
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
    if report.skipped_files > 0 {
        output::plain(format!(
            "  Skipped (unparseable frontmatter): {}",
            report.skipped_files
        ));
    }
    output::plain(format!(
        "  Pass 2 (scanning):    {} references resolved",
        report.references_resolved
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

fn render_graph_index_report(report: &graph_index::GraphIndexReport, dry_run: bool, verbose: bool) {
    let verb = if dry_run { "Would create" } else { "Created" };

    output::header(format!(
        "temper graph index — {} seeds, {} clusters",
        report.seeds_extracted, report.clusters_formed
    ));
    output::plain(format!(
        "  Proposals returned: {}",
        report.proposals_returned
    ));
    output::plain(format!("  {verb} concepts:    {}", report.concepts_created));
    output::plain(format!(
        "  Skipped (non-concept): {}",
        report.concepts_skipped
    ));
    output::plain(format!("  Members updated:    {}", report.members_updated));
    if report.errors > 0 {
        output::plain(format!("  Errors:             {}", report.errors));
    }

    if verbose {
        if !report.seeds_preview.is_empty() {
            output::blank();
            output::plain(format!("Seeds ({}):", report.seeds_preview.len()));
            for s in &report.seeds_preview {
                output::plain(format!("  - {s}"));
            }
        }
        if !report.clusters_preview.is_empty() {
            output::blank();
            output::plain(format!("Clusters ({}):", report.clusters_preview.len()));
            for c in &report.clusters_preview {
                output::plain(format!("  \"{}\"  ({} members)", c.seed, c.member_count));
                for m in &c.top_members {
                    output::plain(format!("    - {m}"));
                }
            }
        }
        if !report.failed.is_empty() {
            output::blank();
            output::plain("Failures:");
            for f in &report.failed {
                output::plain(format!("  - {f}"));
            }
        }
    }
}
