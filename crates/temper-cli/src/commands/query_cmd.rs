//! `temper query` — thin CLI wrapper over actions::query (cloud-only).

use crate::actions::{query as query_actions, runtime};
use crate::error::{Result, TemperError};
use crate::format::OutputFormat;

/// Run a composition.
///
/// `plan_flag` is `--plan`'s raw value; `stdin_is_tty` is resolved by the caller so the body-source
/// precedence stays testable without a terminal — see `main.rs`'s `Commands::Query` arm.
///
/// **A refused plan exits non-zero with every refusal on stderr**, like every other caller error in
/// this CLI. It is a caller fault — that is exactly what `ClientError::PlanRefused` establishes by
/// not being a `Server` error — and inventing a fourth output shape for it would make a refusal the
/// one error an agent has to special-case. The answer, when there is one, goes to stdout in the
/// resolved format, which with a non-TTY stdout is JSON.
pub fn run(plan_flag: Option<&str>, stdin_is_tty: bool, fmt: OutputFormat) -> Result<()> {
    // Source and parse before entering `with_client`, so a malformed plan costs no round trip and
    // the error names the plan rather than the server.
    let raw = query_actions::resolve_plan(plan_flag, stdin_is_tty)?;
    let composition = query_actions::parse_composition(&raw)?;

    let outcome = runtime::with_client(|client| {
        Box::pin(async move { query_actions::run_query(client, &composition).await })
    })?;

    match outcome {
        Ok(response) => {
            let rendered = crate::format::render(&response, fmt)?;
            crate::output::plain(rendered);
            Ok(())
        }
        Err(refusals) => Err(TemperError::Project(query_actions::render_refusals(
            &refusals,
        ))),
    }
}
