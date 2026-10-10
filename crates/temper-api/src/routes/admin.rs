//! The system-admin surface: every route whose ONLY authorization is the sealed `&SystemAdmin`
//! proof. The gated stack plus the machine refusal (`Tier::HumanGated`), and documented
//! under the `Admin` tag like any other route.
//!
//! **Membership rule.** A route belongs here when its handler mints `&SystemAdmin` via
//! `require_system_admin` (or `require_erasure_operator`, its 404-rendering wrapper)
//! unconditionally, before dispatch, and the service it calls requires that proof in its
//! signature. A route whose gate is `is_system_admin OR <a scoped role>` — the admin ledger, the
//! machine-client / connection / subscription families, `reblock`'s deployment-wide arm — is NOT
//! admin-only: a team owner or the actor themself reaches it too, so it stays in `gated_routes`.
//! The path is not the test: `/api/machine-clients/{id}/rebind` lives here, apart from its
//! owner-gated siblings, because it alone requires the proof.
//!
//! **Why documented.** These doors are not secret — the repository is public, and the CLI and
//! any OpenAPI client with an admin's bearer use them. What protects them is the proof the
//! service signature demands, which runs before any lookup, and the tests that pin it; an
//! omission from the contract protected nothing and left them the least-described routes in the
//! API. The routes that belong out of the contract are the ones no bearer can reach: the
//! shared-secret crons and HMAC-signed internal calls (see `embed_internal.rs` / `internal.rs`).
//! The scoped operator families in `gated.rs` (the admin ledger, machine clients, connections,
//! subscriptions) are documented too, under their own tags, since a non-admin reaches them.
//!
//! Group isolation is also what keeps this set auditable: `audit-route-auth.sh` pins the row,
//! and a reviewer can read the whole operator surface in one file.

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::handlers;
use temper_services::state::AppState;

pub(super) fn admin_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        // The access gate's join-request queue. The counting sibling is static `/count`, which
        // beats the `{id}` pattern in the router; it is GET while `{id}` is PATCH, so neither
        // shadows the other.
        .routes(routes!(handlers::access::list_pending))
        .routes(routes!(handlers::access::count_pending))
        .routes(routes!(handlers::access::review_request))
        // The D15 reconsideration inbox — read and closed by an operator, never by a library
        // caller administering their own access.
        .routes(routes!(handlers::access::list_reviews))
        .routes(routes!(handlers::access::count_reviews))
        .routes(routes!(handlers::access::close_review))
        // Full system settings (the gating slug the public `GET /api/access/settings` withholds).
        .routes(routes!(
            handlers::access::get_admin_settings,
            handlers::access::update_settings
        ))
        // Governance: mint and revoke the system-admin grant.
        .routes(routes!(handlers::access::promote_admin))
        .routes(routes!(handlers::access::demote_admin))
        // The admin standing acts (Task 13).
        .routes(routes!(handlers::access::approve_principal))
        .routes(routes!(handlers::access::revoke_principal))
        .routes(routes!(handlers::access::deactivate_principal))
        .routes(routes!(handlers::access::reactivate_principal))
        // The auto-join roster repair: converge every `auto_join_role` team to the
        // standing-approved population and report what it added.
        .routes(routes!(handlers::access::reconcile_auto_join))
        // The operator directory (admin-operator-directory spec §5/§6). The proof is minted
        // before any existence lookup, so absence never leaks to a non-admin. `?email=` on the
        // list route is an identity-resolution act (exact, ambiguity-refusing) that answers a
        // state card, not a page.
        .routes(routes!(handlers::admin_directory::list_profiles))
        .routes(routes!(handlers::admin_directory::show_profile))
        // The admin arm of Slack-link disconnect — the repair path for an orphaned grant. The
        // self-serve arm (`disconnect_me`) is on the auth-only router.
        .routes(routes!(handlers::slack_disconnect::admin_disconnect))
        // Re-embed trigger: enqueue embed jobs for chunks whose vector came from a model we no
        // longer embed with. The per-minute drain does the work; this is only the trigger, gated
        // on the operator's own login rather than the drain's deploy secret.
        .routes(routes!(handlers::embed::reembed))
        // The erasure doors (principal and resource), the block history scrub and the field
        // scrub, each with its read-only survey (and the field scrub's family listing). A caller
        // who is not a system admin is answered 404, never 403, before any lookup, with one
        // telemetry line and no ledger event (ruled 2026-09-30) — the 404 protects the SUBJECT;
        // the doors themselves are documented.
        .routes(routes!(handlers::erasure::execute))
        .routes(routes!(handlers::erasure::survey))
        .routes(routes!(handlers::resource_erasure::execute))
        .routes(routes!(handlers::resource_erasure::survey))
        .routes(routes!(handlers::block_history_scrub::execute))
        .routes(routes!(handlers::block_history_scrub::survey))
        .routes(routes!(handlers::field_scrub::execute))
        .routes(routes!(handlers::field_scrub::survey))
        .routes(routes!(handlers::field_scrub::families))
        // Machine-client rebind (G3 B2). Its siblings under `/api/machine-clients` are gated on
        // `is_system_admin OR owner of the owning team` and stay in `gated_routes`; rebind alone
        // requires the proof, because team ownership cannot bound the reach a rebind inherits.
        .routes(routes!(handlers::machine_clients::rebind))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use utoipa::openapi::path::Operation;

    use super::admin_routes;

    /// The admin group's documented surface, pinned by operation id. A route added here, dropped
    /// from here, or moved back to an undocumented plain mount changes this set and fails —
    /// the review the move deserves happens at the diff of this list.
    const ADMIN_OPERATIONS: [&str; 29] = [
        "admin_list_join_requests",
        "admin_count_join_requests",
        "admin_review_join_request",
        "admin_list_reviews",
        "admin_count_reviews",
        "admin_close_review",
        "admin_get_settings",
        "admin_update_settings",
        "admin_promote",
        "admin_demote",
        "admin_approve_principal",
        "admin_revoke_principal",
        "admin_deactivate_principal",
        "admin_reactivate_principal",
        "admin_reconcile_auto_join",
        "admin_list_profiles",
        "admin_show_profile",
        "admin_disconnect",
        "admin_reembed",
        "admin_erase_principal",
        "admin_survey_principal_erasure",
        "admin_erase_resource",
        "admin_survey_resource_erasure",
        "admin_scrub_block_history",
        "admin_survey_block_history_scrub",
        "admin_scrub_resource_field",
        "admin_survey_resource_field_scrub",
        "admin_list_resource_field_families",
        "admin_rebind_machine_client",
    ];

    fn operations() -> Vec<(String, Operation)> {
        let spec = admin_routes().split_for_parts().1;
        let mut ops = Vec::new();
        for (path, item) in spec.paths.paths {
            for (method, op) in [
                ("GET", item.get),
                ("POST", item.post),
                ("PUT", item.put),
                ("PATCH", item.patch),
                ("DELETE", item.delete),
            ] {
                if let Some(op) = op {
                    ops.push((format!("{method} {path}"), op));
                }
            }
        }
        ops
    }

    #[test]
    fn the_admin_group_documents_exactly_the_pinned_operations() {
        let actual: BTreeSet<String> = operations()
            .into_iter()
            .map(|(route, op)| {
                op.operation_id
                    .unwrap_or_else(|| panic!("{route} has no operation_id"))
            })
            .collect();
        let expected: BTreeSet<String> = ADMIN_OPERATIONS.iter().map(|s| s.to_string()).collect();
        assert_eq!(actual, expected);
    }

    /// Every admin operation is published under the `Admin` tag, so the tag's description ("every
    /// route here requires a system admin") is a statement about exactly this group. The one
    /// exception is the Slack-link admin disconnect, which was documented under `Slack Link` before
    /// the group existed; retagging it would rename its generated SDK class, a breaking change.
    #[test]
    fn every_admin_operation_is_tagged_admin() {
        for (route, op) in operations() {
            let id = op.operation_id.clone().unwrap_or_default();
            let tag = op.tags.as_ref().and_then(|t| t.first()).cloned();
            let want = if id == "admin_disconnect" {
                "Slack Link"
            } else {
                "Admin"
            };
            assert_eq!(
                tag.as_deref(),
                Some(want),
                "{route} ({id}) is tagged {tag:?}"
            );
        }
    }
}
