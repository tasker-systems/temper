//! Witness that `ServerHandler::call_tool` (the service's own, which crosses the host's seam
//! and then calls the router `#[tool_router]` builds) actually dispatches into the router.
//!
//! The existing tests in `service.rs` assert what the router *contains* — they call
//! `TemperMcpService::tool_router()` (a pure associated function) and inspect its advertised
//! tools. None of them drive `ServerHandler::call_tool`, the generated entry point a real MCP
//! client reaches. So a bump of `rmcp` that silently changed the `#[tool_handler]` default —
//! rebuilding a fresh router per call instead of reading the stored field, orphaning it — was
//! only noticed by a `-D dead-code` lint. Had the bump broken routing outright rather than
//! orphaning a field, every router-contents test would still have passed. That is a gate that
//! cannot fail for the thing it appears to cover.
//!
//! This test drives `call_tool` through a real `RequestContext` built off a
//! served `RunningService`'s `Peer` (the only public way to obtain one — `Peer::new` is
//! `pub(crate)` in rmcp). It uses `serve_directly`, which skips the client-handshake
//! initialization that `serve().await` waits for and returns a `RunningService` synchronously.
//! The witness reaches no network: the service's relay is unavailable, so a tool body that runs
//! answers the host's own sentence.
//!
//! The two halves together prove the full dispatch path is wired:
//!   - An **unknown tool** is refused by the router with `INVALID_PARAMS` "tool not found" —
//!     proving `call_tool` reached the router at all. Without a `call_tool` (the service's own,
//!     or `#[tool_handler]`'s generated one), the trait default returns `METHOD_NOT_FOUND`.
//!   - A **known tool** is dispatched to its wrapper and its body runs, answering the dark
//!     door's own sentence — proving the router found the tool and handed off to its wrapper.
//!     Without the dispatch, this too returns `METHOD_NOT_FOUND`.
//!
//! Both halves fail (become `METHOD_NOT_FOUND`) if `call_tool` stops dispatching into the
//! router — that is the regression boundary this witness guards.

use rmcp::{
    model::{CallToolRequestParams, ErrorCode, RequestId},
    service::{serve_directly, RequestContext},
    ServerHandler,
};
use temper_mcp::service::TemperMcpService;
use temper_mcp::BlobDoor;

/// The sentence the witness service's dark door answers — a host's own words, here a test's.
const UNAVAILABLE: &str = "dispatch witness: this service relays nothing";

/// Build a `TemperMcpService` whose relay is unavailable and whose blob door is closed.
///
/// The dark door is the honest shape for a dispatch witness: a tool body that runs answers the
/// host's own sentence instead of reaching for a network, and the seam is never consulted.
fn service_for_dispatch_witness() -> TemperMcpService {
    TemperMcpService::unavailable(
        BlobDoor::Closed {
            refusal: "unused".to_string(),
        },
        UNAVAILABLE,
    )
}

/// Dispatch a tool call through the generated `ServerHandler::call_tool` against a bare
/// `RequestContext` (no HTTP parts, no auth, no DB) and return the resulting error.
///
/// Both witness assertions expect an *error* — dispatch reaches the router but fails before any
/// tool body runs — so this helper wraps the "expect an error" plumbing. The service is served
/// directly (skipping the init handshake) on a throwaway duplex transport; the served `Peer` is
/// `Arc`-backed so it outlives the `RunningService`. The same service instance is then driven
/// through `call_tool` (it is `Clone`).
async fn dispatch_fails(service: TemperMcpService, name: &'static str) -> rmcp::ErrorData {
    let (server_io, _client_io) = tokio::io::duplex(4096);
    let running = serve_directly(service.clone(), server_io, None);
    let ctx = RequestContext::new(RequestId::Number(1), running.peer().clone());
    ServerHandler::call_tool(&service, CallToolRequestParams::new(name), ctx)
        .await
        .expect_err("dispatch must reach the router and fail there, not succeed")
}

/// The error from dispatching an **unknown** tool through `ServerHandler::call_tool` is
/// `INVALID_PARAMS` "tool not found" — the router was reached and refused the name.
///
/// Without the `call_tool` dispatch this becomes `METHOD_NOT_FOUND` (the trait default), which is the
/// regression this witness exists to catch.
#[tokio::test]
async fn an_unknown_tool_is_refused_by_the_router_not_by_the_default_handler() {
    let service = service_for_dispatch_witness();
    let err = dispatch_fails(service, "definitely_not_a_real_tool").await;

    assert_eq!(
        err.code,
        ErrorCode::INVALID_PARAMS,
        "unknown-tool dispatch returned {:?} ({:?}); expected INVALID_PARAMS (\"tool not \
         found\"). METHOD_NOT_FOUND would mean call_tool is not dispatching into the \
         router — the regression this witness guards.",
        err.code,
        err.message,
    );
    assert!(
        err.message.contains("tool not found"),
        "unknown-tool refusal message changed: {:?}. The witness keys on this string; if rmcp \
         reworded it, update the assertion to the new message — do not weaken it to a code-only \
         check, because the message is what distinguishes \"router reached\" from other \
         INVALID_PARAMS failures.",
        err.message,
    );
}

/// The error from dispatching a **known** tool through `ServerHandler::call_tool` is the dark
/// door's own sentence — the router found the tool, handed off to its wrapper, and the tool body
/// ran far enough to ask for a relay client. (The bare context carries no HTTP parts; `call_tool`
/// supplies empty ones, so the wrapper's `Extension<Parts>` extracts on every transport.)
///
/// Without the `call_tool` dispatch this becomes `METHOD_NOT_FOUND` (the trait default).
///
/// This is the half that proves the router routes *to a real tool*, not merely that it was
/// reached. The unknown-tool test alone could pass against a router that refused everything;
/// this one confirms a known name is dispatched.
#[tokio::test]
async fn a_known_tool_is_dispatched_to_its_wrapper_not_the_default_handler() {
    let service = service_for_dispatch_witness();
    let err = dispatch_fails(service, "search").await;

    assert_eq!(
        err.code,
        ErrorCode::INTERNAL_ERROR,
        "known-tool dispatch returned {:?} ({:?}); expected the dark door's INTERNAL_ERROR. \
         METHOD_NOT_FOUND would mean `call_tool` is not wired into the router — the regression \
         this witness guards.",
        err.code,
        err.message,
    );
    assert_eq!(
        err.message, UNAVAILABLE,
        "the tool body must have run and answered the host's sentence — any other message means \
         dispatch stopped before the wrapper (do not weaken this to a code-only check)",
    );
}
