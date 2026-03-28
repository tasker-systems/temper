// temper index — incremental embed pipeline

use crate::config::Config;
use crate::error::Result;
use crate::output;

pub fn run(
    config: &Config,
    force: bool,
    paths_filter: Option<&str>,
    sources_override: Option<&str>,
) -> Result<()> {
    let stats = crate::actions::index::run(config, force, paths_filter, sources_override, |msg| {
        output::dim(msg)
    })?;

    output::blank();
    output::success(format!(
        "Indexed {} documents ({} chunks) in {:.1}s",
        stats.documents, stats.chunks, stats.duration_secs
    ));

    Ok(())
}
