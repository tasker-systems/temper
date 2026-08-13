//! `temper query` — transport only.
//!
//! **No judgment lives here.** The CLI does not validate a plan before sending it, does not rank
//! anything in the response, and does not decide between acts. It sources the plan, posts it, and
//! renders what came back — including a refusal, which is an answer rather than a failure to
//! produce one.

use temper_client::error::ClientError;
use temper_client::TemperClient;
use temper_core::types::query::validate::PlanRefusal;
use temper_core::types::query::{Composition, QueryResponse};

use crate::error::{Result, TemperError};

/// Read the plan from `--plan`/stdin, using the SAME precedence `temper resource update` uses for a
/// body.
///
/// Calls [`crate::actions::body_source::resolve_body_source`] rather than restating the rule:
/// `--plan @<path>` wins, `--plan -` always blocks-reads stdin, and implicit non-TTY stdin is
/// auto-detected (polling briefly, so an open-but-idle pipe resolves to "no plan" instead of
/// hanging). An inlined copy would be a true statement about real behaviour that nothing links to
/// the original, which is exactly how two copies drift.
///
/// **Where this diverges from `update`, and why it is deliberate:** a missing plan is an ERROR.
/// `update` treats absent input as "no body update requested", because it has a frontmatter-only
/// case to fall back on. `query` has nothing to send — spec §B: *"A missing plan is an error —
/// unlike `update`, there is no frontmatter-only case."*
pub fn resolve_plan(plan_flag: Option<&str>, stdin_is_tty: bool) -> Result<String> {
    let sourced = crate::actions::body_source::resolve_body_source(
        plan_flag,
        stdin_is_tty,
        std::io::stdin(),
        crate::actions::body_source::stdin_has_input_within,
    )?;
    sourced.ok_or_else(|| {
        TemperError::Project(
            "no plan supplied — pass `--plan @<path>`, pipe one on stdin, or use `--plan -`"
                .to_string(),
        )
    })
}

/// Parse a composition from JSON, naming the parse failure as the caller's rather than the API's.
///
/// Done before any network call so a malformed plan never costs a round trip, and so the error
/// says *this JSON is not a composition* instead of surfacing a 400 from the server about the same
/// bytes. This is parsing, not validation — whether the plan will RUN is the server's to say, and
/// deciding it here would be a second validator to keep in step with `validate`.
pub fn parse_composition(raw: &str) -> Result<Composition> {
    serde_json::from_str(raw)
        .map_err(|e| TemperError::Project(format!("plan is not a valid composition: {e}")))
}

/// An answered plan, or the refusals that stopped it.
///
/// **A refusal is an outcome, not a transport failure**, and the type says so — mirroring the
/// server's own `prepare(…) -> Result<ValidatedComposition, Vec<PlanRefusal>>`. Collapsing it into
/// the error channel would put "the server was unreachable" and "your plan has two fixable
/// mistakes" in one arm, which is the distinction a plan author most needs kept.
pub type QueryOutcome = std::result::Result<QueryResponse, Vec<PlanRefusal>>;

/// POST the composition and return its answer, or its refusals.
///
/// Every OTHER client error still travels the error channel — this widens exactly one variant, so
/// a 401 or a network drop is not quietly reported as a bad plan.
pub async fn run_query(client: &TemperClient, composition: &Composition) -> Result<QueryOutcome> {
    match client.query().run(composition).await {
        Ok(response) => Ok(Ok(response)),
        Err(ClientError::PlanRefused { refusals }) => Ok(Err(refusals)),
        Err(other) => Err(crate::actions::runtime::client_err_to_temper(other)),
    }
}

/// Every refusal as one message, for the error a refused plan exits with.
///
/// All of them, on separate lines. Showing a plan author one refusal per run is precisely the
/// experience `validate`'s *"every refusal, not the first"* rule exists to prevent, and the rule
/// only reaches them if the last hop renders the whole list.
pub fn render_refusals(refusals: &[PlanRefusal]) -> String {
    let lines: Vec<String> = refusals.iter().map(render_refusal_line).collect();
    format!(
        "the plan will not run ({} refusal(s)):\n  {}",
        refusals.len(),
        lines.join("\n  ")
    )
}

/// Render one refusal as a single human line: the stage it attaches to, then the reason.
///
/// A refusal with no stage is about the composition as a whole — a cycle, a dangling reference, a
/// duplicate name — and prefixing it with a stage name it does not have would point the caller at
/// the wrong thing to fix.
pub fn render_refusal_line(refusal: &PlanRefusal) -> String {
    match &refusal.stage {
        Some(stage) => format!("{}: {}", stage.as_str(), refusal.detail),
        None => refusal.detail.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::query::disposition::RefusalReason;
    use temper_core::types::query::StageName;

    fn refusal(stage: Option<&str>, detail: &str) -> PlanRefusal {
        PlanRefusal {
            stage: stage.map(|s| StageName::parse(s).expect("valid stage name")),
            reason: RefusalReason::UnknownAct,
            detail: detail.to_string(),
        }
    }

    #[test]
    fn a_missing_plan_is_an_error_rather_than_an_empty_request() {
        // TTY stdin with no flag: `update`'s "no body requested" case, which `query` cannot have.
        let err = resolve_plan(None, true).expect_err("a missing plan must not be sent");
        assert!(
            err.to_string().contains("no plan supplied"),
            "the error must say what to pass; got {err}"
        );
    }

    #[test]
    fn a_malformed_plan_is_named_as_the_callers_before_any_round_trip() {
        let err = parse_composition("{not json").expect_err("malformed plan must not be sent");
        assert!(
            err.to_string().contains("not a valid composition"),
            "got {err}"
        );
    }

    #[test]
    fn a_composition_level_refusal_is_not_attributed_to_a_stage() {
        assert_eq!(
            render_refusal_line(&refusal(None, "the composition is cyclic")),
            "the composition is cyclic",
            "a whole-composition refusal must not be prefixed with a stage it does not name"
        );
        assert_eq!(
            render_refusal_line(&refusal(Some("about"), "needs a question")),
            "about: needs a question"
        );
    }

    /// The headline property, at the last hop that can drop it: **every** refusal is rendered.
    #[test]
    fn every_refusal_is_rendered_not_just_the_first() {
        let rendered = render_refusals(&[
            refusal(Some("one"), "needs a question"),
            refusal(Some("two"), "no such act"),
            refusal(None, "the composition is cyclic"),
        ]);
        for expected in [
            "needs a question",
            "no such act",
            "the composition is cyclic",
        ] {
            assert!(
                rendered.contains(expected),
                "refusal {expected:?} was dropped from:\n{rendered}"
            );
        }
        assert!(
            rendered.contains("3 refusal(s)"),
            "the count tells a caller whether they are seeing all of them:\n{rendered}"
        );
    }
}
