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
