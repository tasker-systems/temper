use axum::extract::{FromRequest, Request, State};
use axum::{Extension, Json};

use crate::middleware::auth::AuthUser;
use temper_core::types::ids::ProfileId;
use temper_core::types::query::composition::{Composition, CompositionShape};
use temper_core::types::query::envelope::QueryResponse;
use temper_services::backend::query_read;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::state::AppState;
use temper_workflow::operations::{RelayedSurface, Surface};

#[utoipa::path(
    post,
    path = "/api/query",
    tag = "Query",
    request_body = Composition,
    security(("bearer_auth" = [])),
    responses(
        (
            status = 200,
            description = "One entry in `returned` per `outcome.returns` — no more, no fewer — \
                keyed by stage name rather than merged into one ordered list, so combining two \
                acts' rows takes a deliberate act by the caller. `trace` covers EVERY stage, \
                including those whose rows were not returned, because the pipe carries ids rather \
                than rows and an untraced composition is a black box with an answer at the end.",
            body = QueryResponse,
        ),
        (
            status = 400,
            description = "Two codes. `PLAN_REFUSED`: the composition will not run, with \
                **every** static reason at once in `error.details.refusals` — never just the first, \
                because repairing a plan one refusal per round trip is the experience this contract \
                exists to avoid. A caller meets this response before they meet a 200, so it is the \
                door's most-read documentation. `UNREADABLE_PLAN`, with no `details`: the body is \
                not JSON at all, so there is no plan to refuse (see the `422`).",
            body = ErrorBody,
        ),
        (
            status = 422,
            description = "The body is not a composition this door can read — a value of the wrong \
                type, or a name outside a closed vocabulary — under the code `UNREADABLE_PLAN`. \
                There is no plan yet, so there are no refusals: the message names what failed and \
                repeats at most 1024 bytes of it. Malformed JSON answers `400`, a non-JSON content \
                type `415`, and a body past this door's own limit `413`, each under the same code. \
                A deployment's platform may refuse a large body before this door reads it, without \
                the code.",
            body = ErrorBody,
        ),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "System access required", body = ErrorBody),
    )
)]
// The door onto the composition contract: a caller sends a plan, the server answers it or
// refuses it. Everything before this route built a door that nothing could knock on.
//
// The pipeline is not assembled here. `query_read::prepare` owns the order — shape-gate, then
// embed, then validate — and `validate` is the only constructor of a `ValidatedComposition`
// (cross-crate privacy seals it: no other crate can build one). `prepare` is the only way THIS
// crate reaches one, so the handler cannot run an unvalidated plan even by mistake. Spelling the
// order out here would make this the second place that knows it, and the day the MCP tool and the
// CLI arrive, the third and fourth.
//
// The refusal branch is the only thing that differs from `super::search::search`, whose shape
// this otherwise copies: `search_select` takes params and answers, while `prepare` may refuse
// first.
/// Run a composition of situated acts
///
/// Send a declared composition — a plan of situated acts, piped — and receive its result or a refusal.
///
/// The plan is shape-gated, embedded and validated before any act runs, so an invalid plan is refused whole rather than partially executed.
pub async fn query(
    State(state): State<AppState>,
    auth: AuthUser,
    relayed: Option<Extension<RelayedSurface>>,
    CompositionBody(composition): CompositionBody,
) -> ApiResult<Json<QueryResponse>> {
    // **Measured before anything decides whether to answer it**, which is the entire design. A
    // shape emitted after validation would show only the traffic that already passes — never the
    // traffic a ceiling refuses — and the question this exists to answer is whether the ceilings
    // sit above what callers actually send. See `CompositionShape`.
    //
    // **Measured here for every door, labelled by the door it arrived on.** The MCP edge relays a
    // plan unread (it never deserializes one), so this is the only place a relayed composition
    // exists as one. `RelayedSurface` names the edge; it is planted by `relay_trust` ONLY beside a
    // valid service credential AND the honored carrier, so a forged carrier degrades to `http`, not
    // to a relabel — a caller cannot move their own measurement with headers.
    //
    // Two accepted residuals, named (RG-2 pass, ruled 2026-09-25, restated when the measurement
    // moved here from the edge):
    // - A caller HOLDING the service credential can POST here directly, bypassing the MCP edge,
    // and is measured as `mcp`. Telemetry-only; the credential's documented residual already
    // covers stolen-secret `@mcp` attribution on the thief's own acts.
    // - While the API's `mcp_service_secret` is unset or mid-rotation (and the MCP edge is
    // configured), the extension is never planted and relayed acts measure as `http`. The
    // degrade shows as the root span's `relay_trust` value, but nothing re-labels the
    // measurement; the skew self-heals only at rotation end.
    // Labels are this measurement's own vocabulary, not the emitter markers: `relay_trust` plants
    // `Surface::Mcp` alone, and any other relayed surface would need its label decided here.
    let door = match relayed {
        None => "http",
        Some(Extension(RelayedSurface(Surface::Mcp))) => "mcp",
        Some(Extension(RelayedSurface(_))) => "relayed",
    };
    CompositionShape::of(&composition).record(door);

    let validated = query_read::prepare(composition)
        .await
        .map_err(|refusals| ApiError::PlanRefused { refusals })?;

    let response = query_read::run_composition(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        &validated,
    )
    .await?;
    Ok(Json(response))
}

/// The most of a body rejection's text this door returns.
///
/// Room for any message a mistyped plan produces — serde's path, the offending token when it is
/// typo-sized, and the expected names, which are this contract's own — and no room for an
/// arbitrary caller string.
const MAX_REJECTION_TEXT_BYTES: usize = 1024;

/// The composition, read as `Json` reads it, but rejected without repeating the caller's payload.
///
/// Every rejection answers as [`ApiError::UnreadablePlan`]: the reader's own status under the code
/// `UNREADABLE_PLAN`, so a client — the MCP edge, which relays plans unread, above all — can tell a
/// plan it could not read from a server fault without reading the message.
///
/// axum's `JsonRejection` text carries serde's message, and serde quotes the caller's text whole in
/// two of them: `invalid type` (a string sent where `stages`, `outcome` or `returns` belongs) and
/// `unknown variant` (a `with` section this contract does not name). On a door whose body limit is
/// 25 MiB, that is a response as large as the request. Not every malformed plan echoes: a stage is
/// an untagged enum, so an unknown field or a wrong type inside one is reported as matching no
/// variant, without the caller's text, and an unknown act name parses into the open `ActName`
/// vocabulary for `validate` to refuse under its own 64-byte echo rule. The status is kept, and the
/// text is cut to 1024 bytes (`MAX_REJECTION_TEXT_BYTES`) on a character boundary, so a
/// typo-sized message passes unchanged.
#[derive(Debug)]
pub struct CompositionBody(pub Composition);

impl<S> FromRequest<S> for CompositionBody
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<Composition>::from_request(req, state).await {
            Ok(Json(composition)) => Ok(Self(composition)),
            Err(rejection) => Err(ApiError::UnreadablePlan {
                status: rejection.status(),
                message: bounded_rejection_text(rejection.body_text()),
            }),
        }
    }
}

/// `text` whole when it fits [`MAX_REJECTION_TEXT_BYTES`], else its longest prefix that does,
/// ending on a character boundary, followed by how many bytes were left out.
fn bounded_rejection_text(text: String) -> String {
    if text.len() <= MAX_REJECTION_TEXT_BYTES {
        return text;
    }
    let mut cut = MAX_REJECTION_TEXT_BYTES;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    // A fresh string, not `truncate`: a truncated `String` keeps its capacity, and the response
    // body would hold the whole serde message alive until a slow reader drained it.
    format!(
        "{} … ({} more bytes not repeated)",
        &text[..cut],
        text.len() - cut
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::header::CONTENT_TYPE;

    /// The rejection's status and message, or `None` when the body was read.
    async fn read(body: String) -> Option<(axum::http::StatusCode, String)> {
        let req = Request::builder()
            .method("POST")
            .uri("/api/query")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .expect("request builds");
        match CompositionBody::from_request(req, &()).await {
            Ok(_) => None,
            Err(ApiError::UnreadablePlan { status, message }) => Some((status, message)),
            Err(other) => panic!("every rejection is UnreadablePlan, got {other:?}"),
        }
    }

    /// Bound on what a rejection may add past the cap: the elision note.
    const NOTE_BYTES: usize = 64;

    #[tokio::test]
    async fn a_rejected_body_never_repeats_more_than_the_cap_of_what_the_caller_sent() {
        let huge = "z".repeat(100_000);
        // Every position found where serde quotes the caller's string, by probing the default
        // rejection: 5,000 bytes in came back as 5,132 to 5,192.
        for (what, body) in [
            (
                "stages",
                format!(r#"{{"outcome":{{"returns":[{{"stage":"a"}}]}},"stages":"{huge}"}}"#),
            ),
            ("outcome", format!(r#"{{"outcome":"{huge}","stages":[]}}"#)),
            (
                "returns",
                format!(r#"{{"outcome":{{"returns":"{huge}"}},"stages":[]}}"#),
            ),
            (
                "with",
                format!(
                    r#"{{"outcome":{{"returns":[{{"stage":"a","with":["{huge}"]}}]}},"stages":[]}}"#
                ),
            ),
        ] {
            let (status, text) = read(body)
                .await
                .unwrap_or_else(|| panic!("{what}: the fixture must be a body serde refuses"));
            assert!(
                status.is_client_error(),
                "{what}: the rejection keeps axum's status, got {status}"
            );
            assert!(
                text.len() <= MAX_REJECTION_TEXT_BYTES + NOTE_BYTES,
                "{what}: a rejection repeated {} bytes of a 100 KB string",
                text.len()
            );
        }
    }

    #[tokio::test]
    async fn a_typo_sized_rejection_is_returned_whole() {
        let (_, text) = read(r#"{"outcome":{"returns":[{"stage":"a"}]},"stages":"oops"}"#.into())
            .await
            .expect("a string where the stage list belongs is refused");
        assert!(
            text.contains("oops") && !text.contains("not repeated"),
            "a short message keeps the caller's token so they can see the typo: {text}"
        );
    }

    #[test]
    fn the_cut_lands_on_a_character_boundary() {
        // A three-byte character straddling the cap.
        let text = format!("{}€€", "x".repeat(MAX_REJECTION_TEXT_BYTES - 1));
        let out = bounded_rejection_text(text);
        assert!(out.starts_with(&"x".repeat(MAX_REJECTION_TEXT_BYTES - 1)));
        assert!(out.contains("(6 more bytes not repeated)"), "{out}");
    }
}
