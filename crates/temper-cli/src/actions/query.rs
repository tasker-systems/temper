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

/// What `--check` can answer, stated so a clean result is not read as a promise.
///
/// Spec §C: *"Its disclosure is that it reports expressibility and says so — it cannot speak to what
/// the server has implemented and does not try."* Carried as a field rather than printed as a
/// footer, so it survives `--format json` — the agent reading this programmatically is exactly the
/// caller most likely to treat `expressible: true` as "this will run".
pub const SHAPE_DISCLOSURE: &str =
    "Expressibility only: the plan was checked against the published contract with no server \
     consulted. A clean result does not promise the server will run it — the server may be older \
     or newer than this client, and only it knows what it has built.";

/// The verdict of a local `--check`.
#[derive(Debug, serde::Serialize)]
pub struct ShapeReport {
    /// True when the plan raises no shape refusal. **Not** a prediction that it will run.
    pub expressible: bool,
    /// Every shape refusal at once, never just the first.
    pub refusals: Vec<PlanRefusal>,
    /// Always [`SHAPE_DISCLOSURE`]. Present in every report, including clean ones — a disclosure
    /// that appears only on failure is absent exactly when it is most likely to mislead.
    pub disclosure: &'static str,
}

/// Check a plan's shape locally: no network, no declarations, every refusal at once.
///
/// Calls [`temper_core::types::query::validate::validate_shape`], which is the **same** pass the
/// server runs before it embeds — so this cannot drift into a second, kinder validator. What it
/// deliberately does not run is the capability pass, which is why a clean result is a statement
/// about expressibility and not about this deployment.
pub fn check_plan(composition: &Composition) -> ShapeReport {
    let refusals = temper_core::types::query::validate::validate_shape(composition);
    ShapeReport {
        expressible: refusals.is_empty(),
        refusals,
        disclosure: SHAPE_DISCLOSURE,
    }
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

    /// `--check` reports every shape refusal at once, exactly as the server's 400 does. A check
    /// that stopped at the first would send a plan author round the same loop the door avoids.
    #[test]
    fn check_reports_every_shape_refusal_not_just_the_first() {
        // Two find acts, neither carrying an intention — independently unrunnable.
        let plan: Composition = serde_json::from_str(
            r#"{"stages":[{"name":"one","act":"find-about-anywhere"},
                          {"name":"two","act":"find-about-anywhere"}],
                "outcome":{"returns":[{"stage":"one","with":[]}]}}"#,
        )
        .expect("fixture parses");

        let report = check_plan(&plan);
        assert!(!report.expressible);
        assert!(
            report.refusals.len() >= 2,
            "a shape check must report all of them; got {:?}",
            report.refusals
        );
        let stages: Vec<&str> = report
            .refusals
            .iter()
            .filter_map(|r| r.stage.as_ref().map(|s| s.as_str()))
            .collect();
        assert!(stages.contains(&"one") && stages.contains(&"two"));
    }

    /// **The disclosure rides on the clean result too.** A caveat that appears only when something
    /// is wrong is missing exactly when it is most likely to mislead — `expressible: true` is the
    /// value an agent will read as "this will run".
    #[test]
    fn a_clean_check_still_carries_its_disclosure() {
        let plan: Composition = serde_json::from_str(
            r#"{"stages":[{"name":"about","act":"find-about-anywhere",
                           "intention":{"query":"anything"}}],
                "outcome":{"returns":[{"stage":"about","with":[]}]}}"#,
        )
        .expect("fixture parses");

        let report = check_plan(&plan);
        assert!(report.expressible, "refusals: {:?}", report.refusals);
        assert!(report.refusals.is_empty());
        assert_eq!(report.disclosure, SHAPE_DISCLOSURE);
        assert!(
            report.disclosure.contains("does not promise"),
            "the disclosure must decline to promise, not merely describe itself"
        );
        // It survives serialization — the JSON an agent parses is the point.
        let json = serde_json::to_value(&report).expect("serializes");
        assert!(json["disclosure"].is_string());
        assert_eq!(json["expressible"], true);
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
