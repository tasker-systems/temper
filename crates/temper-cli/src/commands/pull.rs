//! `temper pull <context>` — materialize a context into the local
//! read-only projection. See `crate::projection`.

use crate::actions::runtime;
use crate::output;

pub fn run(context: &str) -> crate::error::Result<()> {
    let context = context.to_string();
    let summary = runtime::with_client(|client| {
        let context = context.clone();
        Box::pin(async move {
            let config = crate::config::load(None)?;
            crate::projection::pull_context(client, &config, &context).await
        })
    })?;

    output::success(format!(
        "Pulled context '{}': {} written, {} pruned",
        summary.context, summary.written, summary.pruned
    ));
    Ok(())
}
