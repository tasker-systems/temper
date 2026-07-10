//! Per-act agent-authorship + invocation correlation — the shared wire carrier for the
//! act-level half of the invocation accountability grain.
//!
//! These types are the single canonical home (CLAUDE.md: "the wire type lives in temper-core").
//! `temper-substrate` re-exports `AgentAuthorship`/`ConfidenceBand` from here (the same chain as
//! `crate::ids`) and serializes the authorship into `kb_events.metadata`; the command layer
//! (`temper-workflow`), the MCP/HTTP/CLI surfaces, and `temper-client` all carry [`ActContext`].
//!
//! **Invariant (06-18 plan §arch):** authorship rides `kb_events.metadata`, NOT the event payload —
//! so it is invisible to projections (and thus affinity math) by construction, and survives replay
//! verbatim. The `invocation` correlator rides `kb_events.invocation_id`.
//!
//! **Invariant (correlation ≠ authz):** `invocation` is a correlation aid, never a substitute for
//! authn/authz. An act with no `invocation` is fully valid (a one-off attributed act, a human at the
//! same CLI/API/MCP tools). The presence of an invocation triggers an *additive* correlation-integrity
//! check at the backend; it never authorizes the write on its own.

use serde::{Deserialize, Serialize};

use crate::error::TemperError;
use crate::types::ids::{CorrelationId, InvocationId};

/// The agent's SUBJECTIVE self-assessment of an authored act — a graded band, not a false-precision
/// scalar. Ordinal: `Tentative < Probable < Confident`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
// Inline the enum in MCP tool schemas. A `$ref` into `$defs` reaches the Anthropic tool-use
// layer with no type signal and comes back as `null` (the same bug fixed for EdgeKind/Polarity
// in `types::graph`); inlining emits `{"type":"string","enum":[…]}` so the band is visible.
#[cfg_attr(feature = "mcp", schemars(inline))]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceBand {
    Tentative,
    Probable,
    Confident,
}

/// Per-act agent-authorship metadata — rides in `kb_events.metadata`, NOT the payload, so it is
/// invisible to projections (and thus affinity math) by construction.
///
/// `confidence` is **required whenever authorship is supplied** (it is non-`Option`): a caller either
/// attaches authorship — and must grade its confidence — or attaches none at all (the whole
/// [`ActContext::authorship`] is `None`). The other fields are optional context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct AgentAuthorship {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    pub confidence: ConfidenceBand,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// The shared act-level carrier threaded onto every authored write command and surface DTO.
/// Maps 1:1 to `temper_substrate::events::EventContext`. `Default` is the empty context
/// (`None`/`None`) — a keyboard-holder/system act with no run correlation and no authorship.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct ActContext {
    /// The invocation this act is correlated under (`kb_events.invocation_id`). Optional — see the
    /// correlation-≠-authz invariant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation: Option<InvocationId>,
    /// The act-grain thread this write belongs to (`kb_events.correlation_id`). Optional; when
    /// absent the event self-roots. Caller-minted, so two writes from different processes under
    /// different credentials can share one act — see [`CorrelationId`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation: Option<CorrelationId>,
    /// The agent's authorship of this act (`kb_events.metadata`). Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorship: Option<AgentAuthorship>,
}

impl ActContext {
    /// True when nothing is attached — equivalent to `ActContext::default()`. Lets surfaces skip
    /// building a command field when neither correlation nor authorship was supplied.
    pub fn is_empty(&self) -> bool {
        self.invocation.is_none() && self.correlation.is_none() && self.authorship.is_none()
    }
}

/// The flat, discrete authorship + correlation fields a caller surface collects — MCP tool input,
/// HTTP request DTO, or CLI flags. Flattened onto each authored-write surface struct so the fields
/// appear as discrete top-level keys/flags (the resolved design: discrete fields, not a JSON blob),
/// then assembled into an [`ActContext`] by [`ActInput::into_act_context`].
///
/// The assembler owns the **one** cross-surface validation — "confidence is required iff any other
/// authorship field is supplied" — in a single place, so MCP/API/CLI can never drift on it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema, utoipa::IntoParams))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct ActInput {
    /// The invocation this act is correlated under (`kb_events.invocation_id`). Optional — a
    /// correlation aid, never a substitute for authn/authz.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<InvocationId>,
    /// The act-grain thread this write belongs to (`kb_events.correlation_id`). Optional, caller-
    /// minted, provenance-only. Rides independently of `invocation_id` and of authorship.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<CorrelationId>,
    /// Free-text reasoning for the act. Authorship field — requires `confidence`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Graded self-assessed confidence band. Required whenever any other authorship field is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ConfidenceBand>,
    /// Structured rationale for the act. Authorship field — requires `confidence`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// The persona/role the author acted as. Authorship field — requires `confidence`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    /// The model that authored the act. Authorship field — requires `confidence`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl ActInput {
    /// Assemble the discrete fields into an [`ActContext`].
    ///
    /// Authorship is `Some` iff `confidence` is supplied. Supplying any other authorship field
    /// (`reasoning`/`rationale`/`persona`/`model`) without a `confidence` band is a hard
    /// [`TemperError::BadRequest`] — `AgentAuthorship::confidence` is non-`Option`, so a graded
    /// band is mandatory once authorship is claimed. The `invocation_id` and `correlation_id`
    /// correlators ride independently and may be present with no authorship at all.
    pub fn into_act_context(self) -> Result<ActContext, TemperError> {
        let ActInput {
            invocation_id,
            correlation_id,
            reasoning,
            confidence,
            rationale,
            persona,
            model,
        } = self;

        let authorship = match confidence {
            Some(confidence) => Some(AgentAuthorship {
                reasoning,
                confidence,
                rationale,
                persona,
                model,
            }),
            None => {
                if reasoning.is_some()
                    || rationale.is_some()
                    || persona.is_some()
                    || model.is_some()
                {
                    return Err(TemperError::BadRequest(
                        "agent authorship requires a confidence band \
                         (tentative|probable|confident) when any of \
                         reasoning/rationale/persona/model is supplied"
                            .to_string(),
                    ));
                }
                None
            }
        };

        Ok(ActContext {
            invocation: invocation_id,
            correlation: correlation_id,
            authorship,
        })
    }
}

impl From<ActContext> for ActInput {
    /// Flatten an assembled [`ActContext`] back into the discrete wire shape. The inverse of
    /// [`ActInput::into_act_context`] — used by the CLI translator, which carries an `ActContext`
    /// on its command but serializes the discrete [`ActInput`] onto the wire DTO. Always valid
    /// (no validation): a present `authorship` already has its mandatory `confidence`.
    fn from(ctx: ActContext) -> Self {
        let ActContext {
            invocation,
            correlation,
            authorship,
        } = ctx;
        match authorship {
            Some(AgentAuthorship {
                reasoning,
                confidence,
                rationale,
                persona,
                model,
            }) => ActInput {
                invocation_id: invocation,
                correlation_id: correlation,
                reasoning,
                confidence: Some(confidence),
                rationale,
                persona,
                model,
            },
            None => ActInput {
                invocation_id: invocation,
                correlation_id: correlation,
                ..Default::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorship_serializes_confidence_band() {
        let a = AgentAuthorship {
            reasoning: Some("because X".into()),
            confidence: ConfidenceBand::Probable,
            rationale: None,
            persona: None,
            model: None,
        };
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["confidence"], "probable");
        // Optional Nones skip-serialize, so the wire stays minimal.
        assert!(v.get("rationale").is_none());
        let back: AgentAuthorship = serde_json::from_value(v).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn confidence_is_required_when_authorship_supplied() {
        // Supplying authorship without a confidence band is a hard error — confidence is non-Option.
        let err = serde_json::from_value::<AgentAuthorship>(serde_json::json!({
            "reasoning": "no band given",
        }));
        assert!(
            err.is_err(),
            "authorship without confidence must fail to deserialize"
        );
    }

    #[test]
    fn act_context_default_is_empty() {
        let ctx = ActContext::default();
        assert!(ctx.is_empty());
        assert!(ctx.invocation.is_none() && ctx.correlation.is_none() && ctx.authorship.is_none());
        // Empty context serializes to `{}` (all fields skip).
        assert_eq!(serde_json::to_value(&ctx).unwrap(), serde_json::json!({}));
    }

    #[test]
    fn act_context_with_only_correlation_is_not_empty() {
        // Correlation alone is a meaningful context: the surface must still build a command field
        // for it. Were `is_empty` to ignore correlation, the caller's thread would be dropped.
        let ctx = ActContext {
            correlation: Some(CorrelationId::new()),
            ..Default::default()
        };
        assert!(!ctx.is_empty());
    }

    #[test]
    fn act_context_round_trips_invocation_correlation_and_authorship() {
        let ctx = ActContext {
            invocation: Some(InvocationId::new()),
            correlation: Some(CorrelationId::new()),
            authorship: Some(AgentAuthorship {
                reasoning: Some("r".into()),
                confidence: ConfidenceBand::Confident,
                rationale: None,
                persona: Some("steward".into()),
                model: None,
            }),
        };
        let back: ActContext = serde_json::from_value(serde_json::to_value(&ctx).unwrap()).unwrap();
        assert_eq!(back, ctx);
    }

    #[test]
    fn correlation_serializes_as_a_bare_uuid_string() {
        // The newtype is `#[serde(transparent)]`, so the wire shape is exactly the bare UUID a
        // non-Rust caller (temper-rb, a Sidekiq job argument) mints and sends. If this ever
        // becomes `{"correlation":{"0":"…"}}` every generated client breaks.
        let id = CorrelationId::new();
        let ctx = ActContext {
            correlation: Some(id),
            ..Default::default()
        };
        let v = serde_json::to_value(&ctx).unwrap();
        assert_eq!(v["correlation"], serde_json::json!(id.uuid().to_string()));
    }

    // ── ActInput discrete-fields assembler ──────────────────────────────────

    #[test]
    fn act_input_default_assembles_empty_context() {
        let ctx = ActInput::default()
            .into_act_context()
            .expect("empty input is valid");
        assert!(ctx.is_empty(), "no fields supplied → empty ActContext");
    }

    #[test]
    fn act_input_invocation_only_assembles_correlation_without_authorship() {
        let inv = InvocationId::new();
        let ctx = ActInput {
            invocation_id: Some(inv),
            ..Default::default()
        }
        .into_act_context()
        .expect("invocation-only input is valid");
        assert_eq!(ctx.invocation, Some(inv));
        assert!(
            ctx.authorship.is_none(),
            "no authorship fields → authorship None (correlation rides alone)"
        );
    }

    #[test]
    fn act_input_confidence_assembles_authorship() {
        let ctx = ActInput {
            confidence: Some(ConfidenceBand::Probable),
            reasoning: Some("because X".into()),
            persona: Some("steward".into()),
            ..Default::default()
        }
        .into_act_context()
        .expect("authorship with confidence is valid");
        let authorship = ctx.authorship.expect("authorship present");
        assert_eq!(authorship.confidence, ConfidenceBand::Probable);
        assert_eq!(authorship.reasoning.as_deref(), Some("because X"));
        assert_eq!(authorship.persona.as_deref(), Some("steward"));
        assert!(ctx.invocation.is_none());
    }

    #[test]
    fn act_input_authorship_field_without_confidence_is_rejected() {
        // reasoning supplied but no confidence band → hard error (confidence required iff
        // authorship supplied). This is the one validation the shared assembler owns.
        let err = ActInput {
            reasoning: Some("no band given".into()),
            ..Default::default()
        }
        .into_act_context()
        .expect_err("authorship without confidence must be rejected");
        assert!(
            matches!(err, crate::error::TemperError::BadRequest(_)),
            "expected BadRequest, got {err:?}"
        );
    }

    #[test]
    fn act_input_correlation_only_assembles_without_authorship_or_invocation() {
        // The temper-rb background-worker shape: a bare correlation, no run envelope, no persona.
        let corr = CorrelationId::new();
        let ctx = ActInput {
            correlation_id: Some(corr),
            ..Default::default()
        }
        .into_act_context()
        .expect("correlation-only input is valid");
        assert_eq!(ctx.correlation, Some(corr));
        assert!(ctx.invocation.is_none(), "correlation rides without a run");
        assert!(ctx.authorship.is_none(), "correlation is not authorship");
        assert!(!ctx.is_empty());
        // …and survives the authorship-less arm of the inverse translator.
        let back = ActInput::from(ctx.clone())
            .into_act_context()
            .expect("reassembles");
        assert_eq!(back, ctx);
    }

    #[test]
    fn act_context_to_act_input_roundtrips() {
        // The translator path turns a command's ActContext back into the discrete ActInput wire
        // shape; the handler reassembles it. The roundtrip must be identity.
        let ctx = ActContext {
            invocation: Some(InvocationId::new()),
            correlation: Some(CorrelationId::new()),
            authorship: Some(AgentAuthorship {
                reasoning: Some("r".into()),
                confidence: ConfidenceBand::Confident,
                rationale: Some("because".into()),
                persona: None,
                model: Some("opus".into()),
            }),
        };
        let back = ActInput::from(ctx.clone())
            .into_act_context()
            .expect("reassembles");
        assert_eq!(back, ctx);
    }

    #[test]
    fn empty_act_context_to_act_input_is_empty() {
        let back = ActInput::from(ActContext::default())
            .into_act_context()
            .expect("reassembles");
        assert!(back.is_empty());
    }

    #[test]
    fn act_input_confidence_alone_is_valid_authorship() {
        // Confidence with no other field is a valid, minimal authorship.
        let ctx = ActInput {
            confidence: Some(ConfidenceBand::Confident),
            ..Default::default()
        }
        .into_act_context()
        .expect("confidence-only authorship is valid");
        let authorship = ctx.authorship.expect("authorship present");
        assert_eq!(authorship.confidence, ConfidenceBand::Confident);
        assert!(authorship.reasoning.is_none());
    }
}
