//! `temper query` — thin CLI wrapper over actions::query (cloud-only).

use crate::actions::{query as query_actions, runtime};
use crate::error::Result;
use crate::format::OutputFormat;

/// Run a composition.
///
/// `plan_flag` is `--plan`'s raw value; `stdin_is_tty` is resolved by the caller so the body-source
/// precedence stays testable without a terminal — see `main.rs`'s `Commands::Query` arm.
///
/// **A refused plan exits non-zero**, like every other caller error in this CLI. It is a caller
/// fault — that is exactly what `ClientError::PlanRefused` establishes by not being a `Server`
/// error — and inventing a fourth output shape for it would make a refusal the one error an agent
/// has to special-case. The answer, when there is one, goes to stdout in the resolved format, which
/// with a non-TTY stdout is JSON.
///
/// The refusal travels as [`crate::error::TemperError::PlanRefused`]: every refusal in the message,
/// and the list itself beside it. `main` prints the message on stderr at a terminal; under the JSON
/// format it prints the one `ErrorPayload` on stdout, whose `details.refusals` carries each
/// refusal's `reason` and `teaches` — the HTTP door's `error.details`. The prose has each refusal's
/// `detail` and none of its `teaches`, which is why the list is carried and not only the text.
pub fn run(
    plan_flag: Option<&str>,
    check: bool,
    strict: bool,
    stdin_is_tty: bool,
    fmt: OutputFormat,
) -> Result<()> {
    // Source and parse before entering `with_client`, so a malformed plan costs no round trip and
    // the error names the plan rather than the server.
    let raw = query_actions::resolve_plan(plan_flag, stdin_is_tty)?;
    let composition = query_actions::parse_composition(&raw)?;

    if check {
        return run_check(&composition, strict, fmt);
    }

    let outcome = runtime::with_client(|client| {
        Box::pin(async move { query_actions::run_query(client, &composition).await })
    })?;

    match outcome {
        Ok(response) => {
            let rendered = crate::format::render(&response, fmt)?;
            crate::output::plain(rendered);
            Ok(())
        }
        Err(refusals) => Err(query_actions::refused(refusals)),
    }
}

/// `--check`: the shape verdict on **stdout** as data, and the exit code as the verdict.
///
/// Unlike the run path, refusals here are the command's **output**, not its failure — `--check` was
/// asked to find them, and an agent gating on this needs to parse them. So they render to stdout in
/// the resolved format (JSON with a non-TTY stdout) while the exit code stays non-zero, which is how
/// a linter behaves and what makes `temper query --check < plan.json && temper query < plan.json`
/// mean the right thing.
///
/// **No network is touched on this path at all** — `run` returns before `with_client`, so `--check`
/// works with no token, offline, against a server that does not exist yet. That is the point:
/// spec §C, *"No network, no declarations."*
///
/// **Two tiers, and only one fails by default** `[ruled — 2026-11-27, Pete, ruling 33]`. A shape
/// refusal exits non-zero: no server could run the plan. A capability finding is this client's
/// build speaking, so it is reported and exits zero, unless `--strict` says the caller knows this
/// client matches the server it will send to.
fn run_check(
    composition: &temper_core::types::query::Composition,
    strict: bool,
    fmt: OutputFormat,
) -> Result<()> {
    let report = query_actions::check_plan(composition);
    let fails = check_fails(&report, strict);
    let rendered = crate::format::render(&report, fmt)?;
    crate::output::plain(rendered);

    if fails {
        // `process::exit` runs no destructors, so any spans this process opened would be lost.
        // See `main`'s drain comment — this is a counted exit, not an uncounted one.
        temper_telemetry::shutdown_telemetry();
        std::process::exit(1);
    }
    Ok(())
}

/// Whether a `--check` report fails the command: any shape refusal, and a capability finding only
/// under `--strict`.
fn check_fails(report: &query_actions::ShapeReport, strict: bool) -> bool {
    !report.expressible || (strict && !report.capability_findings.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(plan: serde_json::Value) -> query_actions::ShapeReport {
        let c = query_actions::parse_composition(&plan.to_string()).expect("the plan parses");
        query_actions::check_plan(&c)
    }

    /// A plan only a capability finding stands against: a resource filter on a ranked find act.
    fn capability_only() -> serde_json::Value {
        serde_json::json!({
            "stages": [{"name": "hits", "act": "find-exact", "intention": {"query": "q"},
                        "resource_filter": {"doc_type": ["task"]}}],
            "outcome": {"returns": [{"stage": "hits"}]}
        })
    }

    /// **A capability finding is reported with the server's typed reason and fails only under
    /// `--strict`; a shape refusal always fails.**
    ///
    /// FAILS IF: `--check` stops running the capability pass (no finding), reports it as a refusal
    /// (`expressible` false), or fails on it without `--strict`.
    #[test]
    fn a_capability_finding_is_reported_and_fails_only_under_strict() {
        let r = report(capability_only());
        assert!(
            r.expressible,
            "a capability finding is not a shape refusal: {:?}",
            r.refusals
        );
        assert!(r.refusals.is_empty());
        assert_eq!(
            r.capability_findings.len(),
            1,
            "{:?}",
            r.capability_findings
        );
        assert_eq!(
            r.capability_findings[0].reason,
            temper_core::types::query::RefusalReason::FilterNotApplicable
        );
        assert!(!check_fails(&r, false), "exit 0 without --strict");
        assert!(check_fails(&r, true), "exit 1 under --strict");

        // A shape refusal fails either way: two stages under one name.
        let shape = report(serde_json::json!({
            "stages": [{"name": "a", "act": "find-exact", "intention": {"query": "q"}},
                       {"name": "a", "act": "find-exact", "intention": {"query": "q"}}],
            "outcome": {"returns": [{"stage": "a"}]}
        }));
        assert!(!shape.expressible);
        assert!(check_fails(&shape, false) && check_fails(&shape, true));

        // A clean plan passes either way, with nothing in either tier.
        let clean = report(serde_json::json!({
            "stages": [{"name": "hits", "act": "find-exact", "intention": {"query": "q"}}],
            "outcome": {"returns": [{"stage": "hits"}]}
        }));
        assert!(clean.refusals.is_empty() && clean.capability_findings.is_empty());
        assert!(!check_fails(&clean, false) && !check_fails(&clean, true));
    }

    /// Both tiers and both disclosures reach `--format json`, the form an agent reads.
    #[test]
    fn the_capability_tier_and_its_disclosure_survive_json() {
        let json = serde_json::to_value(report(capability_only())).expect("serializes");
        assert_eq!(
            json["capability_findings"][0]["reason"],
            "filter_not_applicable"
        );
        assert_eq!(
            json["capability_disclosure"],
            query_actions::CAPABILITY_DISCLOSURE
        );
        assert!(json["capability_disclosure"]
            .as_str()
            .is_some_and(|d| d.contains("only as current as")));
        assert_eq!(json["disclosure"], query_actions::SHAPE_DISCLOSURE);
    }
}
