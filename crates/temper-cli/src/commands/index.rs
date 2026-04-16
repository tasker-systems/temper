//! `temper index` command — build HNSW vector index over the vault.

use crate::config::Config;
use crate::error::Result;

/// Run the index build pipeline.
pub fn run(config: &Config, context: Option<&str>, full: bool) -> Result<()> {
    let params = crate::actions::index::IndexParams {
        context_filter: context.map(String::from),
        full,
    };
    let report = crate::actions::index::run(config, params)?;
    render_report(&report);
    Ok(())
}

fn render_report(report: &crate::actions::index::IndexReport) {
    use crate::output;

    output::header(format!(
        "temper index — {} files indexed",
        report.files_indexed
    ));
    output::plain(format!("  Skipped (unchanged): {}", report.files_skipped));
    output::plain(format!("  Errors: {}", report.errors));
    if !report.skipped_files.is_empty() && !report.skipped_files.len() > 20 {
        output::blank();
        output::plain("Skipped files:");
        for f in &report.skipped_files {
            output::plain(format!("  - {f}"));
        }
    }
}
