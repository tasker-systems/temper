//! Authenticated AND system-access-gated — default-deny for all data routes.
//!
//! The routes whose ONLY gate is the `&SystemAdmin` proof are not here: they are the admin
//! group (`admin.rs`), same tier, isolated so the operator surface reads as one set. What stays
//! here is gated on the caller's own reach — including the operator-adjacent families below
//! (the admin ledger, machine clients, connections, subscriptions) whose gate is
//! `is_system_admin OR <a scoped role>`, which a non-admin team owner or actor also passes.

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::handlers;
use temper_services::state::AppState;

pub(super) fn gated_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            handlers::resources::list,
            handlers::resources::create
        ))
        .routes(routes!(
            handlers::resources::get,
            handlers::resources::update,
            handlers::resources::delete
        ))
        .routes(routes!(handlers::resources::get_content))
        .routes(routes!(handlers::data_artifacts::list))
        .routes(routes!(handlers::data_artifacts::get))
        .routes(routes!(handlers::data_artifacts::commit))
        // The flat artifact read — the MCP `get_data_artifact` tool's wire twin (the tool
        // takes only the artifact id; the nested route's REST parent is a field its
        // declaration cannot grow). Route-first for the beat G4 door crossing.
        .routes(routes!(handlers::data_artifacts::get_by_id))
        // `blobs::commit` and `blobs::append_segment` are NOT mounted here: they are the
        // two doors whose legal body sizes exceed axum's inherited default, so they merge
        // through `blob_commit_routes` / `blob_segment_routes` with limits sized from
        // the config and the plan numbers (see those functions, merged in [`create_app`]).
        .routes(routes!(handlers::blobs::get))
        .routes(routes!(handlers::blobs::delete))
        // One `.routes()` per handler: the multi-handler form is for same-path method
        // grouping (the ingest blocks GET+POST shape); distinct paths in one call mangle
        // the mounted patterns into overlaps.
        .routes(routes!(handlers::blobs::begin_upload))
        .routes(routes!(handlers::blobs::upload_progress))
        .routes(routes!(handlers::blobs::finalize_upload))
        .routes(routes!(handlers::blobs::list))
        .routes(routes!(handlers::blobs::relate))
        .routes(routes!(handlers::blobs::relations))
        .routes(routes!(handlers::data_artifact_shapes::list_shapes))
        .routes(routes!(handlers::data_artifact_shapes::get_shape))
        .routes(routes!(handlers::data_artifact_shapes::declare_shape))
        // The cogmap-home shapes pair — the MCP `list_data_artifact_shapes` /
        // `declare_data_artifact_shape` tools admit a cogmap `home_type` and the
        // substrate read/write are home-generic, but only the context arm had a wire
        // door. Route-first for the beat G4 door crossing.
        .routes(routes!(handlers::data_artifact_shapes::list_cogmap_shapes))
        .routes(routes!(
            handlers::data_artifact_shapes::declare_cogmap_shape
        ))
        .routes(routes!(
            handlers::resources::provenance,
            handlers::resources::annotate
        ))
        .routes(routes!(handlers::resources::read_block))
        // Corpus adoption — one bounded, resumable, per-row-gated re-block step per call,
        // dispatched to the Backend's `reblock_resources` command. Here rather than in the admin
        // group (unlike the admin-enclosed `/api/embed/admin/reembed` trigger): the gate is the
        // backend seam — the deployment-wide arm is SystemAdmin-checked there, the
        // resource/context arms ride the caller's own visibility — and the contract is the
        // receipt.
        .routes(routes!(handlers::reblock::reblock))
        .routes(routes!(handlers::reassign::reassign_resource))
        .routes(routes!(handlers::edges::list))
        .routes(routes!(handlers::edges::list_connections))
        .routes(routes!(handlers::evidence::evidence))
        // Both methods on `/api/resources/{id}/citation-audits` — one `routes!` group, as with the
        // resource CRUD trio above, so the path is declared once.
        .routes(routes!(
            handlers::citation_audits::record,
            handlers::citation_audits::list
        ))
        // The block-addressed audit write — same gate, same command, no finding in the address.
        .routes(routes!(handlers::citation_audits::record_for_block))
        .routes(routes!(handlers::edges::lineage))
        .routes(routes!(handlers::edges::assert))
        .routes(routes!(handlers::edges::retype))
        .routes(routes!(handlers::edges::reweight))
        .routes(routes!(handlers::edges::fold))
        .routes(routes!(handlers::facets::set_facet))
        .routes(routes!(handlers::facets::list_resource_facets))
        .routes(routes!(
            handlers::facets::set_edge_facet,
            handlers::facets::list_edge_facets,
            handlers::facets::retract_edge_facet
        ))
        .routes(routes!(handlers::graph::cogmap_neighborhood_slice))
        .routes(routes!(handlers::graph::region_composition))
        .routes(routes!(handlers::graph::context_panorama))
        .routes(routes!(handlers::graph::context_composition))
        .routes(routes!(handlers::graph::entry))
        .routes(routes!(handlers::graph::traverse))
        .routes(routes!(handlers::graph::atlas_home))
        .routes(routes!(handlers::graph::cogmap_panorama))
        .routes(routes!(
            handlers::meta::get_meta,
            handlers::meta::update_meta
        ))
        .routes(routes!(
            handlers::resources::grant,
            handlers::resources::revoke
        ))
        .routes(routes!(
            handlers::contexts::list,
            handlers::contexts::create
        ))
        .routes(routes!(handlers::contexts::resolve))
        .routes(routes!(handlers::contexts::get, handlers::contexts::delete))
        .routes(routes!(handlers::contexts::restore))
        .routes(routes!(handlers::contexts::share_team))
        .routes(routes!(handlers::contexts::unshare_team))
        .routes(routes!(handlers::contexts::reassign))
        .routes(routes!(handlers::contexts::rename))
        // Context orientation reads (T8) — the peers of the five cognitive-map orientation reads
        // below (shape, materialize-delta, materialize, region-metrics, analytics).
        .routes(routes!(handlers::contexts::shape))
        .routes(routes!(handlers::contexts::region_metrics))
        .routes(routes!(handlers::contexts::materialize_delta))
        .routes(routes!(handlers::contexts::materialize))
        .routes(routes!(handlers::contexts::analytics))
        .routes(routes!(handlers::teams::list, handlers::teams::create))
        .routes(routes!(handlers::teams::add_member))
        .routes(routes!(handlers::invitations::create))
        .routes(routes!(handlers::invitations::list))
        .routes(routes!(handlers::invitations::revoke))
        .routes(routes!(handlers::reassign::reassign_team))
        .routes(routes!(
            handlers::teams::detail,
            handlers::teams::update,
            handlers::teams::delete
        ))
        .routes(routes!(
            handlers::teams::remove_member,
            handlers::teams::change_role
        ))
        .routes(routes!(handlers::ingest::create))
        .routes(routes!(handlers::ingest::update))
        .routes(routes!(
            handlers::segments::list_blocks_handler,
            handlers::segments::append_block_handler
        ))
        .routes(routes!(handlers::segments::finalize_handler))
        .routes(routes!(
            handlers::cognitive_maps::genesis,
            handlers::cognitive_maps::list
        ))
        .routes(routes!(
            handlers::cognitive_maps::reconcile,
            handlers::cognitive_maps::show
        ))
        .routes(routes!(handlers::cognitive_maps::shape))
        .routes(routes!(handlers::cognitive_maps::materialize_delta))
        .routes(routes!(handlers::cognitive_maps::materialize))
        .routes(routes!(handlers::cognitive_maps::region_metrics))
        .routes(routes!(handlers::cognitive_maps::analytics))
        .routes(routes!(handlers::cognitive_maps::bind_team))
        .routes(routes!(handlers::cognitive_maps::unbind_team))
        .routes(routes!(
            handlers::cognitive_maps::grant,
            handlers::cognitive_maps::revoke
        ))
        .routes(routes!(
            handlers::invocations::open,
            handlers::invocations::list
        ))
        .routes(routes!(handlers::invocations::show))
        .routes(routes!(handlers::invocations::close))
        .routes(routes!(handlers::steward::delta))
        .routes(routes!(handlers::steward::advance))
        .routes(routes!(handlers::steward::sweep))
        .routes(routes!(handlers::steward::candidates))
        .routes(routes!(handlers::steward::dispatch))
        .routes(routes!(handlers::auditor::sweep))
        .routes(routes!(handlers::auditor::dispatch))
        .routes(routes!(handlers::auditor::complete))
        .routes(routes!(handlers::events::cursor))
        .routes(routes!(handlers::events::element_trail))
        // The vocabularies each kind of work carries. Caller-independent answers over the
        // embedded schemas — still on the gated surface, because caller-independence is a
        // property of the answer and not a reason to publish it.
        .routes(routes!(handlers::schema::list_doc_types))
        .routes(routes!(handlers::schema::describe_doc_type))
        .routes(routes!(handlers::schema::describe_open_meta))
        .routes(routes!(handlers::search::search))
        .merge(super::query::query_routes())
        // The admin ledger's read surface. Not in the admin group despite its path:
        // `list_by_actor` is self-gating (an actor reads their own acts) and `list_by_subject`
        // dispatches per act family, so a non-admin can be answered. Authorization is in
        // `admin_ledger_service`, which gates per act family rather than with a prelude, and
        // denies with 404 so a refusal discloses nothing about the subject.
        .routes(routes!(handlers::admin_ledger::list))
        // Machine-principal registration (G3 Phase A). NOT admin-only: the gate is
        // `is_system_admin OR owner of the machine's owning team` (`machine_authz::authorize` for
        // provision/issue, `MachineClientControlAuthority` per row, which refuses as a missing id), so
        // any authenticated profile that owns any team can reach `provision`, `issue`, and
        // `apply_reach`. Only `rebind` is admin-only (`machine_registration_service::rebind`), and
        // it is mounted by the admin group (`admin.rs`), not here.
        //
        // The gate lives in the SERVICES, not in these handlers — the handlers are gate-free by
        // design, as `handlers::machine_clients`' module doc explains. Treat it as load-bearing,
        // not defense-in-depth: how much the router's `require_system_access` layer actually
        // excludes is an operational setting an instance can change at any time, so the service
        // check is the only guarantee that does not move. Do not relax it on the strength of a
        // configuration value read at some past moment.
        .routes(routes!(
            handlers::machine_clients::list,
            handlers::machine_clients::provision
        ))
        .routes(routes!(
            handlers::machine_clients::get,
            handlers::machine_clients::revoke
        ))
        .routes(routes!(handlers::machine_clients::issue))
        .routes(routes!(handlers::machine_clients::rotate_secret))
        // Connection provisioning (external systems as subscribed emitters, S1). Same shape as
        // machine-clients above and for the same reasons: gated inside the service
        // (`MachineAuthority`'s policy, verbatim — a connection is a machine principal wearing an
        // integration's clothes; per row through `ConnectionControlAuthority`, which refuses as a
        // missing id).
        .routes(routes!(
            handlers::connections::list,
            handlers::connections::provision
        ))
        .routes(routes!(
            handlers::connections::get,
            handlers::connections::revoke
        ))
        // The credential and the two capability tiers, each its own endpoint. They are separately
        // provisioned and both explicit — folding them into one PATCH would let a caller grant
        // reach while believing they were only registering a webhook.
        .routes(routes!(handlers::connections::attach_credential))
        .routes(routes!(handlers::connections::set_webhook_events))
        .routes(routes!(handlers::connections::set_tool_manifest))
        // A team's read-reach on the connection, its own endpoint (a `kb_access_grants` write, not
        // a connection-row mutation). Owning ≠ reaching, so this is separate from provisioning.
        // Grant and revoke share the path — POST adds, DELETE removes — both carrying the team.
        .routes(routes!(
            handlers::connections::grant_reach,
            handlers::connections::revoke_reach
        ))
        // Subscription management (external systems as subscribed emitters, S2). Same shape as
        // connections above, gated inside the service (`SubscriptionAuthority` on the authoring
        // team, plus the `kb_access_grants` reach-grant read on create).
        .routes(routes!(
            handlers::subscriptions::list,
            handlers::subscriptions::create
        ))
        .routes(routes!(
            handlers::subscriptions::get,
            handlers::subscriptions::revoke
        ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::gated_routes;

    /// The scoped operator families' documented surface, pinned by operation id and tag. A dropped
    /// method (one handler out of a shared-path `routes!` pair), a retag (which renames a generated
    /// SDK class), or a move back to an undocumented plain mount changes this set and fails.
    const SCOPED_OPERATOR_OPERATIONS: [(&str, &str); 20] = [
        ("list_admin_ledger", "Admin Ledger"),
        ("list_machine_clients", "Machine Clients"),
        ("provision_machine_client", "Machine Clients"),
        ("get_machine_client", "Machine Clients"),
        ("revoke_machine_client", "Machine Clients"),
        ("issue_machine_credential", "Machine Clients"),
        ("rotate_machine_client_secret", "Machine Clients"),
        ("list_connections", "Connections"),
        ("provision_connection", "Connections"),
        ("get_connection", "Connections"),
        ("revoke_connection", "Connections"),
        ("attach_connection_credential", "Connections"),
        ("set_connection_webhook_events", "Connections"),
        ("set_connection_tool_manifest", "Connections"),
        ("grant_connection_reach", "Connections"),
        ("revoke_connection_reach", "Connections"),
        ("list_subscriptions", "Subscriptions"),
        ("create_subscription", "Subscriptions"),
        ("get_subscription", "Subscriptions"),
        ("revoke_subscription", "Subscriptions"),
    ];

    const SCOPED_OPERATOR_TAGS: [&str; 4] = [
        "Admin Ledger",
        "Machine Clients",
        "Connections",
        "Subscriptions",
    ];

    #[test]
    fn the_scoped_operator_families_document_exactly_the_pinned_operations() {
        let spec = gated_routes().split_for_parts().1;
        let mut actual = BTreeSet::new();
        for (path, item) in spec.paths.paths {
            for op in [item.get, item.post, item.put, item.patch, item.delete]
                .into_iter()
                .flatten()
            {
                let tag = op
                    .tags
                    .as_ref()
                    .and_then(|t| t.first())
                    .cloned()
                    .unwrap_or_default();
                if SCOPED_OPERATOR_TAGS.contains(&tag.as_str()) {
                    let id = op
                        .operation_id
                        .unwrap_or_else(|| panic!("{path} has no operation_id"));
                    actual.insert((id, tag));
                }
            }
        }
        let expected: BTreeSet<(String, String)> = SCOPED_OPERATOR_OPERATIONS
            .iter()
            .map(|(id, tag)| (id.to_string(), tag.to_string()))
            .collect();
        assert_eq!(actual, expected);
    }
}
