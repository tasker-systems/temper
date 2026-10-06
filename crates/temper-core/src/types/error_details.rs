//! The typed payload of `ErrorBody.error.details` — the `oneOf` that widening it produced.
//!
//! `details` carried exactly one shape from the day it was added until `POST /api/query` needed a
//! second. `temper-services`' `ErrorDetail` holds it as a `serde_json::Value` at runtime, because
//! `IntoResponse` erases the error variant before serializing; [`ErrorDetails`] is what that field
//! is **declared** to the generators as, so the SDKs get the union rather than an untyped blob.
//!
//! It lives in `temper-core` rather than beside `ErrorDetail` for the reason every wire type does:
//! both arms are already `temper-core` types, and the CLI, the generated TypeScript and the Ruby
//! gem all need to name the union without depending on the server crate.

use serde::{Deserialize, Serialize};

use crate::types::access_gate::SystemAccessDetails;
use crate::types::query::validate::PlanRefusal;

/// Every static reason a composition is not executable, in one payload.
///
/// **A list rather than a single refusal, and that is the whole point.** `validate` returns every
/// refusal rather than the first *"because a caller repairing a plan should see all of it in one
/// round trip"* — a property that exists on the wire only if the transport carries the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "error_details.ts"))]
pub struct PlanRefusalDetails {
    pub refusals: Vec<PlanRefusal>,
}

/// The `oneOf` an error's `details` may be.
///
/// Untagged, so the payload stays exactly what each arm already sent — widening the contract must
/// not move the shipped `SYSTEM_ACCESS_REQUIRED` body, which every current client parses.
///
/// **The variants are unambiguous by required field, not by order.** `SystemAccessDetails` requires
/// `refusal` (its four other fields are `Option`); `PlanRefusalDetails` requires `refusals`. Neither
/// payload satisfies the other's required field, so untagged deserialization cannot pick wrong —
/// asserted by `each_arm_round_trips_to_its_own_variant`. Adding an all-optional arm here would
/// break that, silently, at the first payload that omits everything.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "error_details.ts"))]
#[serde(untagged)]
pub enum ErrorDetails {
    SystemAccess(Box<SystemAccessDetails>),
    PlanRefusals(PlanRefusalDetails),
}

// The error envelope every door answers a failure with: `{"error": {...}}`. A `//` comment, not a
// doc comment: utoipa would publish a doc comment as the schema's description, and moving the type
// here must not change openapi.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

impl ErrorBody {
    /// Build a typed error body with no `details` payload — the shape used by both
    /// `ApiError::into_response` and temper-services' `transport::fallback_handler`, so an
    /// unmatched route is not the one error a client has to special-case.
    pub fn new(code: &str, message: String) -> Self {
        Self {
            error: ErrorDetail {
                code: code.to_string(),
                message,
                details: None,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    /// Present on `SYSTEM_ACCESS_REQUIRED`, where it carries the typed access refusal, and on
    /// `PLAN_REFUSED`, where it carries every static refusal of a composition; absent on every
    /// other error.
    // Held as a `Value` because `IntoResponse` erases the variant before serializing, but declared
    // to the generators as what it actually is: an untyped `details` described nothing while
    // costing the SDKs their typed refusal.
    //
    // `[widened — 2026-08-13]` This was declared as the bare `SystemAccessDetails` under a note
    // saying "should a second variant ever carry details, this becomes a `oneOf` — widen it then,
    // deliberately." B1 is that second variant, and this is that widening. Which ARM a body carries
    // is told by `error.code`, never by sniffing the payload's shape — the two arms are
    // distinguishable by required field (see `ErrorDetails`), but a client that leans on that is
    // one all-optional arm away from silently misparsing.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "web-api", schema(value_type = Option<ErrorDetails>))]
    pub details: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::disposition::RefusalReason;

    fn a_refusal(detail: &str) -> PlanRefusal {
        PlanRefusal {
            stage: None,
            reason: RefusalReason::UnknownAct,
            detail: detail.to_string(),
        }
    }

    /// The disambiguation the doc block claims, executed rather than asserted in prose. If a future
    /// arm makes the union ambiguous, this is what reds.
    #[test]
    fn each_arm_round_trips_to_its_own_variant() {
        let refusals = ErrorDetails::PlanRefusals(PlanRefusalDetails {
            refusals: vec![a_refusal("first"), a_refusal("second")],
        });
        let json = serde_json::to_value(&refusals).expect("serializes");
        assert!(
            json.get("refusals").is_some(),
            "the refusal payload must be an object keyed `refusals` — the wire path \
             `ErrorBody.error.details.refusals` is what the spec and `PlanRefusal`'s own doc name"
        );
        match serde_json::from_value::<ErrorDetails>(json).expect("deserializes") {
            ErrorDetails::PlanRefusals(d) => assert_eq!(d.refusals.len(), 2),
            ErrorDetails::SystemAccess(_) => {
                panic!("a refusal payload deserialized as the access arm")
            }
        }

        let access = ErrorDetails::SystemAccess(Box::new(SystemAccessDetails {
            email: None,
            display_name: None,
            refusal: temper_principal::Refusal::NoStanding,
            request_url: None,
            cli_command: None,
        }));
        let json = serde_json::to_value(&access).expect("serializes");
        match serde_json::from_value::<ErrorDetails>(json).expect("deserializes") {
            ErrorDetails::SystemAccess(d) => {
                assert_eq!(d.refusal, temper_principal::Refusal::NoStanding);
            }
            ErrorDetails::PlanRefusals(_) => {
                panic!("an access payload deserialized as the refusal arm")
            }
        }
    }

    /// The regression boundary named in the plan's *Declared risk*: `ErrorDetail` is on every route
    /// in the project, so the widening must not move the body a shipped client already parses.
    #[test]
    fn the_access_arm_serializes_exactly_as_the_bare_type_did() {
        let bare = SystemAccessDetails {
            email: Some("a@b.c".into()),
            display_name: Some("A".into()),
            refusal: temper_principal::Refusal::NoStanding,
            request_url: Some("https://example.test".into()),
            cli_command: Some("temper auth request-access".into()),
        };
        let through_union =
            serde_json::to_value(ErrorDetails::SystemAccess(Box::new(bare.clone())))
                .expect("union serializes");
        let direct = serde_json::to_value(&bare).expect("bare serializes");
        assert_eq!(
            through_union, direct,
            "wrapping the access details in the `oneOf` changed the bytes on the wire"
        );
    }
}
