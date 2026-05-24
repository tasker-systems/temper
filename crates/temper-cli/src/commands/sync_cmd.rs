//! `temper sync` — cloud-only mode keeps only the `run` subcommand as an
//! explanatory error. Use `temper resource create` / `temper resource update`
//! to write, and `temper pull <context>` to refresh the local projection.

use crate::error::Result;

/// `temper sync run` — removed. temper is cloud-only: there is no local
/// vault to reconcile.
pub fn run(_contexts: &[String], _format: &str) -> Result<()> {
    Err(crate::error::TemperError::Project(
        "temper is cloud-only — there is no local vault to sync. Use \
         `temper resource create` / `temper resource update` to write, \
         and `temper pull <context>` to refresh the local projection."
            .to_string(),
    ))
}
