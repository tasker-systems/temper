//! The exhaustive `AuthzError` witness — moved here from the tool layer when the seam split
//! the crates (the goal's seam ruling 5). The tool layer maps the API's 401/403 BODY
//! (`temper_mcp::service::map_post_edge_refusal`, public crate surface for exactly this), which no
//! compiler checks; this test restores the forcing: a new `AuthzError` variant fails to compile
//! here until someone states what the API sends for it and what the MCP surface answers.

use temper_client::error::ClientError;

/// The wire body temper-api renders for each `AuthzError`, as temper-client types it on the
/// relay — `None` where the API answers a fault (5xx), which no post-edge arm maps. Mirrors
/// `temper-api/src/middleware/auth.rs` (Level 1) and `middleware/system_access.rs` (Level 2);
/// the tool layer depends on neither temper-api nor temper-services, so the mirror is written
/// here, in the deployed host, where the services' error types and the tool layer's public
/// refusal mapping are both legitimately visible, and the `match`
/// below is EXHAUSTIVE on purpose: a new `AuthzError` variant fails to compile until someone
/// states what the API sends for it and what this surface answers.
fn api_wire_refusal(e: temper_services::auth::AuthzError) -> Option<ClientError> {
    use temper_services::auth::AuthzError;
    use temper_services::error::ApiError;
    let unauthorized = |cause: String| ClientError::UnauthorizedDetails {
        message: format!("Unauthorized: {cause}"),
    };
    match e {
        AuthzError::Refused(why) => {
            Some(unauthorized(format!("machine credential refused: {why}")))
        }
        AuthzError::Deactivated { .. } => Some(unauthorized("account is deactivated".to_string())),
        AuthzError::EmailResolution(err) | AuthzError::ProfileResolution(err) => match err {
            ApiError::Unauthorized(cause) => Some(unauthorized(cause)),
            _ => None,
        },
        AuthzError::AccessCheck(_) => None,
        AuthzError::SystemAccessDenied { refusal, .. } => Some(ClientError::SystemAccessRequired(
            Box::new(temper_core::error::CliAccessDetails {
                email: Some("someone@example.com".to_string()),
                display_name: Some("Someone".to_string()),
                refusal: Some(refusal),
                request_url: None,
                cli_command: None,
            }),
        )),
    }
}

/// **Every `AuthzError` the API can refuse with has an MCP face — compiler-forced.**
///
/// Until teardown the direct binding's `map_authz_error` matched `AuthzError` exhaustively,
/// so a new variant could not compile without an MCP rendering. The relay maps the API's
/// 401/403 BODY instead (`map_post_edge_refusal`), which no compiler checks; this test
/// restores the forcing through [`api_wire_refusal`]'s exhaustive match, and pins each
/// variant's face: the terminal sentences for the machine gate, deactivation, the email
/// ladder and the registration gate, the system-access arm with its typed refusal, and no
/// post-edge answer at all for a fault (the tool's own mapping owns those).
#[test]
fn every_authz_refusal_has_an_mcp_face() {
    use temper_services::auth::AuthzError;
    use temper_services::error::ApiError;
    let terminal = -32600;
    let cases: Vec<(AuthzError, Option<(i32, &str)>)> = vec![
        (
            AuthzError::Refused("no grant type"),
            Some((
                terminal,
                temper_mcp::service::TERMINAL_MACHINE_GATE_SENTENCE,
            )),
        ),
        (
            AuthzError::Deactivated {
                profile_id: uuid::Uuid::nil(),
            },
            Some((
                terminal,
                temper_mcp::service::TERMINAL_DEACTIVATION_SENTENCE,
            )),
        ),
        (
            AuthzError::EmailResolution(ApiError::Unauthorized(
                "Token missing email claim and userinfo lookup failed".to_string(),
            )),
            Some((
                terminal,
                temper_mcp::service::TERMINAL_EMAIL_RESOLUTION_SENTENCE,
            )),
        ),
        (
            AuthzError::ProfileResolution(ApiError::Unauthorized(
                "machine client 'x' is not registered with this instance.".to_string(),
            )),
            Some((
                terminal,
                "machine client 'x' is not registered with this instance. This error is \
                 terminal and should not be retried.",
            )),
        ),
        (
            AuthzError::ProfileResolution(ApiError::Internal("db".to_string())),
            None,
        ),
        (
            AuthzError::AccessCheck(ApiError::Internal("db".to_string())),
            None,
        ),
        (
            AuthzError::SystemAccessDenied {
                profile_id: uuid::Uuid::nil(),
                refusal: temper_principal::Refusal::Denied,
            },
            Some((
                terminal,
                "Access to this temper instance requires approval for someone@example.com",
            )),
        ),
    ];
    for (variant, expected) in cases {
        let label = format!("{variant:?}");
        let mapped = api_wire_refusal(variant)
            .and_then(|wire| temper_mcp::service::map_post_edge_refusal(&wire));
        match (mapped, expected) {
            (None, None) => {}
            (Some(err), Some((code, prefix))) => {
                assert_eq!(err.code.0, code, "{label}: {err}");
                assert!(err.message.starts_with(prefix), "{label}: {}", err.message);
            }
            (got, want) => panic!("{label}: got {got:?}, want {want:?}"),
        }
    }
}
