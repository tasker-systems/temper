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

// ── Undeclared arguments ────────────────────────────────────────────────────────────────────────

/// Dispatch `name` with `arguments` through `ServerHandler::call_tool` and return the answer.
async fn dispatch(
    service: TemperMcpService,
    name: String,
    arguments: serde_json::Value,
) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
    let (server_io, _client_io) = tokio::io::duplex(4096);
    let running = serve_directly(service.clone(), server_io, None);
    let ctx = RequestContext::new(RequestId::Number(1), running.peer().clone());
    let request = CallToolRequestParams::new(name)
        .with_arguments(arguments.as_object().expect("object arguments").clone());
    ServerHandler::call_tool(&service, request, ctx).await
}

/// Every tool the service declares, by name: the shipped fixture, held equal to the router by
/// `declarations_test`.
fn advertised_tools() -> Vec<String> {
    let list: serde_json::Value =
        serde_json::from_str(temper_mcp::declarations::TOOLS_LIST).expect("the fixture parses");
    list["tools"]
        .as_array()
        .expect("a tool array")
        .iter()
        .map(|t| t["name"].as_str().expect("a name").to_string())
        .collect()
}

/// The text of a tool-execution error result, after asserting it is one.
fn refusal_text(
    name: &str,
    answer: Result<rmcp::model::CallToolResponse, rmcp::ErrorData>,
) -> String {
    let result = match answer {
        Ok(rmcp::model::CallToolResponse::Complete(result)) => result,
        other => panic!("`{name}`: an undeclared argument must answer a tool result: {other:?}"),
    };
    assert_eq!(result.is_error, Some(true), "`{name}`: an error result");
    result.content[0]
        .as_text()
        .expect("a text part")
        .text
        .clone()
}

/// **Every tool refuses an argument it does not declare, before its body runs.** The refusal is a
/// tool-execution error result (never a JSON-RPC error, which rmcp logs at `warn`) naming the key,
/// and the dark door's sentence never appears: the tool body was not reached, so nothing can have
/// been written.
///
/// FAILS IF the shared gate in `call_tool` is removed, or a tool's schema stops being the set the
/// gate reads.
#[tokio::test]
async fn every_tool_refuses_an_undeclared_argument_before_its_body_runs() {
    let tools = advertised_tools();
    assert!(tools.len() >= 40, "found only {} tools", tools.len());
    for name in tools {
        let answer = dispatch(
            service_for_dispatch_witness(),
            name.clone(),
            serde_json::json!({"zz_undeclared_argument": "a-value-never-echoed"}),
        )
        .await;
        let text = refusal_text(&name, answer);
        assert!(
            text.contains("unknown argument `zz_undeclared_argument`"),
            "`{name}`: the refusal names the key: {text}"
        );
        assert!(
            !text.contains("a-value-never-echoed"),
            "`{name}`: no value: {text}"
        );
        assert_ne!(text, UNAVAILABLE, "`{name}`: the body never ran");
    }
}

/// **Every declared argument passes the gate.** Each tool is called with every key its published
/// schema declares (each `null`): the call reaches the tool (its body's sentence, or serde's own
/// refusal of a value), never the undeclared-argument refusal.
#[tokio::test]
async fn every_declared_argument_reaches_its_tool() {
    let list: serde_json::Value =
        serde_json::from_str(temper_mcp::declarations::TOOLS_LIST).expect("the fixture parses");
    for tool in list["tools"].as_array().expect("a tool array") {
        let name = tool["name"].as_str().expect("a name").to_string();
        let declared: serde_json::Map<String, serde_json::Value> = tool["inputSchema"]
            ["properties"]
            .as_object()
            .expect("properties")
            .keys()
            .map(|k| (k.clone(), serde_json::Value::Null))
            .collect();
        assert!(!declared.is_empty(), "`{name}` declares arguments");
        let answer = dispatch(
            service_for_dispatch_witness(),
            name.clone(),
            serde_json::Value::Object(declared),
        )
        .await;
        if let Ok(rmcp::model::CallToolResponse::Complete(result)) = &answer {
            let text = result.content[0]
                .as_text()
                .map(|t| t.text.as_str())
                .unwrap_or("");
            assert!(
                !text.contains("unknown argument"),
                "`{name}`: no declared argument is refused: {text}"
            );
        }
    }
}

/// **The claim case.** `update_resource_meta` with `expects` (for `expect`) is refused and never
/// reaches the write; the same call spelled right reaches it.
#[tokio::test]
async fn a_misspelled_expect_is_refused_and_the_write_never_runs() {
    let call = serde_json::json!({
        "id": "00000000-0000-0000-0000-000000000000",
        "managed_meta": {},
        "open_meta": {"claim_id": "mine"},
    });
    let mut misspelled = call.clone();
    misspelled["expects"] = serde_json::json!({"open": {"claim_id": null}});
    let text = refusal_text(
        "update_resource_meta",
        dispatch(
            service_for_dispatch_witness(),
            "update_resource_meta".to_string(),
            misspelled,
        )
        .await,
    );
    assert!(text.contains("unknown argument `expects`"), "{text}");
    assert!(
        text.contains("`expect`"),
        "the refusal lists the declared spelling: {text}"
    );
    assert!(!text.contains("mine"), "no value: {text}");

    let mut spelled_right = call;
    spelled_right["expect"] = serde_json::json!({"open": {"claim_id": null}});
    let err = dispatch(
        service_for_dispatch_witness(),
        "update_resource_meta".to_string(),
        spelled_right,
    )
    .await
    .expect_err("the dark door answers its sentence");
    assert_eq!(
        err.message, UNAVAILABLE,
        "the declared call reached the write"
    );
}
