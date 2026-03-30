use crate::actions::types::NormalizeSummary;
use crate::config::Config;
use crate::error::Result;
use crate::output;

pub fn run(
    config: &Config,
    project: Option<&str>,
    dry_run: bool,
    fix_slugs: bool,
) -> Result<NormalizeSummary> {
    let summary = crate::actions::normalize::run(config, project, dry_run, fix_slugs)?;

    // Print summary
    if dry_run {
        output::header("Normalize dry-run (no changes made):");
    } else {
        output::header("Normalize complete:");
    }
    output::plain(format!("  {} IDs backfilled", summary.ids_backfilled));
    output::plain(format!("  {} files moved", summary.files_moved));
    output::plain(format!("  {} stages migrated", summary.stages_migrated));
    output::plain(format!("  {} slug mismatches", summary.slugs_fixed));
    output::plain(format!(
        "  {} frontmatter fields fixed",
        summary.frontmatter_fixed
    ));
    if summary.tasks_without_effort > 0 {
        output::plain(format!(
            "  {} tasks without effort",
            summary.tasks_without_effort
        ));
    }

    Ok(summary)
}
