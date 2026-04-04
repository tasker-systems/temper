use crate::config::Config;
use crate::error::Result;

/// Run doctor (validate only).
pub fn run(config: &Config, context: Option<&str>, format: &str) -> Result<()> {
    let _ = (config, context, format);
    crate::output::plain("temper doctor: not yet implemented");
    Ok(())
}

/// Run doctor fix (validate + auto-fix).
pub fn run_fix(config: &Config, context: Option<&str>, dry_run: bool) -> Result<()> {
    let _ = (config, context, dry_run);
    crate::output::plain("temper doctor fix: not yet implemented");
    Ok(())
}
