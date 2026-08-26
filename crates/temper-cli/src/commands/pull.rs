//! `temper pull <context>` — materialize a context into the local
//! read-only projection. See `crate::projection`.

use crate::actions::runtime;
use crate::output;

/// Materialize `context` into the local projection under the resolved vault root.
///
/// **`vault` is not optional to the caller by accident.** It is `--vault`, and
/// `run` used to call `config::load(None)` — discarding it, so
/// `temper --vault /somewhere pull <ctx>` silently wrote to the *configured*
/// vault instead. `pull` is the one command whose entire job is writing to a
/// vault root, and it was the only one of the ten `config::load` call sites not
/// threading the flag; every other passes `cli.vault.as_deref()`.
///
/// `TEMPER_VAULT` masked it — `config::load` reads that env var itself, so the
/// override appeared to work whenever it was spelled as an env var rather than
/// a flag. Taking the value as a parameter is what makes dropping it again a
/// compile error rather than a silent redirect.
///
/// A compile error is not the whole guard, though: a parameter can be threaded
/// and then ignored, and for a while nothing asserted the flag was *honoured*.
/// `tests/e2e/tests/projection_pull_test.rs::pull_writes_to_the_vault_the_flag_names`
/// spawns the real binary with `--vault` pointed away from both the config file
/// and `TEMPER_VAULT`, and asserts at both ends — the tree appears where the flag
/// said, and does not appear where it would have gone otherwise.
pub fn run(context: &str, vault: Option<&str>) -> crate::error::Result<()> {
    let context = context.to_string();
    let vault = vault.map(str::to_owned);
    let summary = runtime::with_client(|client| {
        let context = context.clone();
        let vault = vault.clone();
        Box::pin(async move {
            let config = crate::config::load(vault.as_deref())?;
            crate::projection::pull_context(client, &config, &context).await
        })
    })?;

    output::success(format!(
        "Pulled context '{}': {} written, {} pruned",
        summary.context, summary.written, summary.pruned
    ));
    Ok(())
}
