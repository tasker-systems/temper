pub use temper_core::error::{Result, TemperError};
use temper_core::types::error_details::{ErrorDetails, PlanRefusalDetails};

/// CLI-only error surface for commands whose failures don't belong in the
/// shared [`TemperError`] vocabulary.
///
/// `temper update`'s self-install failures (subprocess, checksum, atomic swap,
/// cargo-build refusal) are the motivating case: they never originate on a
/// server surface, so folding an `Install` variant into `temper-core`'s
/// `TemperError` would push install semantics onto every crate that consumes it
/// (`temper-api`, `temper-services`, `temper-mcp`) — surfaces that never
/// install anything. Keeping the variant here contains that blast radius.
///
/// The [`From<TemperError>`] pass-through means ordinary `?`-propagation of core
/// errors still works inside a CLI command that returns [`CliResult`]; only the
/// genuinely CLI-local failures use [`CliError::Install`].
#[derive(Debug)]
pub enum CliError {
    /// A self-update / installer failure (installer subprocess, checksum,
    /// atomic swap) or a precondition refusal (`cargo install` build).
    Install(String),
    /// A core error propagated unchanged (network/API version resolution,
    /// render, etc.).
    Temper(TemperError),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Install(msg) => write!(f, "Install error: {msg}"),
            CliError::Temper(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CliError::Install(_) => None,
            CliError::Temper(e) => Some(e),
        }
    }
}

impl From<TemperError> for CliError {
    fn from(e: TemperError) -> Self {
        CliError::Temper(e)
    }
}

// There is deliberately **no** `impl From<ClientError> for TemperError`, here or
// anywhere in the workspace. Do not add one.
//
// It would be a genuine ergonomic win — every `client.foo().await?` would just
// work — and that is exactly the problem. `ClientError` carries structure that
// `TemperError` can only preserve if the conversion is deliberate:
// `SystemAccessRequired(details)` must survive as its own variant for the
// enriched 403 renderer to fire, and `is_network()` must survive to distinguish
// a down server from a rejecting one. A blanket `From` makes every `?` a silent
// flattening site, and the loss is invisible at the call site.
//
// This is not hypothetical. Two rival hand-written lifters once disagreed on
// exactly those two properties, so which guidance a gated user saw depended on
// which helper a call site happened to import — the enriched access-gate block
// was unreachable on every path a normal user hits, for months, while its unit
// tests stayed green. See PR #486.
//
// The single lifter is `actions::runtime::client_err_to_temper`. Because no
// `From` impl exists, every conversion must be written by hand and is therefore
// greppable — which is the only reason auditing this surface is tractable at
// all. Adding the impl would not just risk a regression; it would remove the
// property that lets anyone find the next one.

/// Result alias for CLI commands that can raise a [`CliError`].
pub type CliResult<T> = std::result::Result<T, CliError>;

/// The structured `details` a [`TemperError`] carries onto the payload, in the shape the HTTP door
/// puts in `error.details`. Only a refused plan has any; every other error is `None` and its payload
/// carries no `details` key.
fn error_details(e: &TemperError) -> Option<ErrorDetails> {
    match e {
        TemperError::PlanRefused { refusals, .. } => {
            Some(ErrorDetails::PlanRefusals(PlanRefusalDetails {
                refusals: refusals.clone(),
            }))
        }
        _ => None,
    }
}

/// Render a [`TemperError`] as a JSON `ErrorPayload` string for stdout.
///
/// In JSON mode, `main.rs` calls this instead of `output::error` (stderr prose)
/// so an agent that merges streams and parses JSON gets a parseable payload
/// even on failure. The exit code stays non-zero; the structured payload is the
/// explanation an agent can branch on.
pub fn render_error_payload(e: &TemperError, hint: Option<&str>) -> String {
    let payload = temper_core::types::ErrorPayload {
        code: e.code(),
        message: format!("{e}"),
        hint: hint.map(|h| h.to_string()),
        details: error_details(e),
    };
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| {
        // If serialization fails (it should not — ErrorPayload is plain strings),
        // fall back to a minimal object so the payload is still valid JSON.
        format!(
            "{{\n  \"code\": \"{}\",\n  \"message\": \"{}\"\n}}",
            e.code(),
            e
        )
    })
}

/// Render a [`CliError`] as a JSON `ErrorPayload` string for stdout.
///
/// `CliError::Install` maps to code `"install"`; `CliError::Temper` delegates to
/// the inner [`TemperError`]'s code.
pub fn render_cli_error_payload(e: &CliError) -> String {
    let (code, message, details) = match e {
        CliError::Install(msg) => ("install", format!("Install error: {msg}"), None),
        CliError::Temper(e) => (e.code(), format!("{e}"), error_details(e)),
    };
    let payload = temper_core::types::ErrorPayload {
        code,
        message,
        hint: None,
        details,
    };
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| {
        format!(
            "{{\n  \"code\": \"{}\",\n  \"message\": \"{}\"\n}}",
            code, e
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::artifact_ask::RefusalTeaching;
    use temper_core::types::query::disposition::RefusalReason;
    use temper_core::types::query::validate::PlanRefusal;
    use temper_core::types::query::StageName;

    /// A refusal that teaches, built by hand: no door raises one on this route yet, so the payload
    /// is witnessed with the refusal a door will raise (`path_outside_shape`).
    fn a_refusal_that_teaches() -> PlanRefusal {
        PlanRefusal {
            stage: Some(StageName::parse("artifacts").expect("valid stage name")),
            reason: RefusalReason::PathOutsideShape,
            detail: "the shape does not declare that path".to_string(),
            teaches: Some(Box::new(RefusalTeaching {
                terms: vec!["current".to_string(), "member".to_string()],
                non_closure: Some("stored content is not closed by its shape".to_string()),
                ..RefusalTeaching::default()
            })),
        }
    }

    fn a_plain_refusal() -> PlanRefusal {
        PlanRefusal {
            stage: None,
            reason: RefusalReason::NoReturns,
            detail: "outcome.returns is empty".to_string(),
            teaches: None,
        }
    }

    /// **A refused plan's payload carries `details.refusals` with `teaches` intact.** Read back
    /// through the type the HTTP door sends, the list equals the one the error held. FAILS IF
    /// `render_error_payload` sets `details: None`, or the details drop `teaches`.
    #[test]
    fn a_refused_plans_payload_carries_its_refusals_with_what_they_teach() {
        let refusals = vec![a_refusal_that_teaches(), a_plain_refusal()];
        let err = crate::actions::query::refused(refusals.clone());

        let rendered = render_error_payload(&err, None);
        let raw: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");

        assert_eq!(raw["code"], "project", "a refused plan's code is `project`");
        assert_eq!(
            raw["message"].as_str(),
            Some(err.to_string().as_str()),
            "the message is the displayed text"
        );
        assert_eq!(
            raw["details"]["refusals"][0]["teaches"]["terms"],
            serde_json::json!(["current", "member"]),
            "teaches is a field of the first refusal: {rendered}"
        );
        assert!(
            raw["details"]["refusals"][1].get("teaches").is_none(),
            "a refusal with nothing to teach carries no teaches: {rendered}"
        );
        let back: PlanRefusalDetails =
            serde_json::from_value(raw["details"].clone()).expect("the HTTP door's details type");
        assert_eq!(back.refusals, refusals);
    }

    /// The `CliError` payload builder carries a wrapped refused plan's `details` too, not only the
    /// `TemperError` one. FAILS IF its `Temper` arm sets `details: None`.
    #[test]
    fn a_wrapped_refused_plan_keeps_its_details_in_the_cli_error_payload() {
        let refusals = vec![a_refusal_that_teaches()];
        let cli = CliError::Temper(crate::actions::query::refused(refusals.clone()));
        let raw: serde_json::Value =
            serde_json::from_str(&render_cli_error_payload(&cli)).expect("valid JSON");
        assert_eq!(raw["code"], "project");
        let back: PlanRefusalDetails =
            serde_json::from_value(raw["details"].clone()).expect("the HTTP door's details type");
        assert_eq!(back.refusals, refusals);
    }

    /// Every other error's payload carries no `details` key at all.
    #[test]
    fn an_error_that_is_not_a_refused_plan_has_no_details_key() {
        for err in [
            TemperError::NotFound("gone".to_string()),
            TemperError::Project("no plan supplied".to_string()),
            TemperError::Forbidden,
        ] {
            let raw: serde_json::Value =
                serde_json::from_str(&render_error_payload(&err, Some("a hint")))
                    .expect("valid JSON");
            assert!(raw.get("details").is_none(), "{err}: {raw}");
        }
        let cli = CliError::Install("swap failed".to_string());
        let raw: serde_json::Value =
            serde_json::from_str(&render_cli_error_payload(&cli)).expect("valid JSON");
        assert!(raw.get("details").is_none(), "{raw}");
    }
}
