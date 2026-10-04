//! The endpoint registry — every request this client sends names its operation here.
//!
//! One constant per operation: the HTTP verb, the path template in `openapi.json`'s `{param}`
//! spelling, and the `operationId` it answers to. Every client method builds its request through
//! one of these ([`HttpClient::request`](crate::http::HttpClient::request) takes an [`Op`]), so
//! the verb and path a method sends come from the same place the parity check reads.
//!
//! # The invariant this module carries
//!
//! `temperkb-client` is always **equal to or a superset of** `openapi.json`: every published
//! operation has a constant here and a method that uses it. Three mechanisms hold that, and none
//! of them alone would:
//!
//! - **The parity test** (`tests` below) reads the repo-root `openapi.json` and fails when an
//!   operation has no constant, when a constant's verb or template disagrees with the spec, or when
//!   a published constant names an operation the spec no longer has.
//! - **Dead code.** The constants are `pub(crate)` and [`ALL`] exists only under `cfg(test)`, so in
//!   the library build a constant no method uses is an unused item, and CI's clippy runs with
//!   `-D warnings`. A constant therefore cannot claim coverage that no method provides.
//! - **The raw-literal guard** (`.github/scripts/check-client-op-registry.sh`) refuses a `"/api/`
//!   string literal anywhere in this crate's `src` outside this file, so a method cannot route
//!   around the registry by spelling a path itself.
//!
//! An [`Visibility::Unpublished`] entry is a route this client reaches that `openapi.json` does not
//! document. It carries its reason, and the parity test refuses it if the spec ever does publish
//! the route (it would then be a published entry under the wrong name).

use std::fmt::Display;

use reqwest::Method;

/// The HTTP verb of an [`Op`]. Its own `Copy` type rather than [`Method`] so the registry can be
/// plain `const` items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verb {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Verb {
    pub(crate) fn method(self) -> Method {
        match self {
            Verb::Get => Method::GET,
            Verb::Post => Method::POST,
            Verb::Put => Method::PUT,
            Verb::Patch => Method::PATCH,
            Verb::Delete => Method::DELETE,
        }
    }
}

/// Whether an [`Op`] is documented in `openapi.json`. Read only by the parity test.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum Visibility {
    /// Documented; carries the spec's `operationId`.
    Published(&'static str),
    /// Reached by this client but not documented in the spec; carries the reason. None today —
    /// the last one, the retired `/api/upload`, was removed rather than kept — so the variant is
    /// never constructed; it stays as the documented way to carry such a route.
    #[allow(dead_code)]
    Unpublished(&'static str),
}

/// One operation this client can send.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Op {
    pub(crate) verb: Verb,
    /// The path in `openapi.json`'s spelling — `{param}` placeholders, no query string.
    pub(crate) template: &'static str,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) visibility: Visibility,
}

impl Op {
    pub(crate) fn method(&self) -> Method {
        self.verb.method()
    }

    /// Render the template, substituting each `{param}` placeholder in order with the next of
    /// `args`. Values are inserted verbatim: a caller whose segment can carry reserved characters
    /// encodes it first, exactly as it did when it spelled the path with `format!`.
    ///
    /// A query string, when a method needs one, is appended to the rendered path by the caller;
    /// templates never carry one.
    ///
    /// # Panics
    ///
    /// When `args` does not supply exactly one value per placeholder — a bug in the calling
    /// method, which its unit test exercises.
    pub(crate) fn path(&self, args: &[&dyn Display]) -> String {
        let mut out = String::with_capacity(self.template.len() + 16 * args.len());
        let mut args = args.iter();
        let mut rest = self.template;
        while let Some(open) = rest.find('{') {
            let close = rest[open..]
                .find('}')
                .map(|i| open + i)
                .unwrap_or_else(|| panic!("unterminated placeholder in {}", self.template));
            out.push_str(&rest[..open]);
            let arg = args
                .next()
                .unwrap_or_else(|| panic!("too few path arguments for {}", self.template));
            out.push_str(&arg.to_string());
            rest = &rest[close + 1..];
        }
        out.push_str(rest);
        assert!(
            args.next().is_none(),
            "too many path arguments for {}",
            self.template
        );
        out
    }
}

macro_rules! registry {
    (
        published { $( $name:ident = $verb:ident $template:literal => $operation_id:literal; )* }
        unpublished { $( $uname:ident = $uverb:ident $utemplate:literal, because $reason:literal; )* }
    ) => {
        $(
            pub(crate) const $name: Op = Op {
                verb: Verb::$verb,
                template: $template,
                visibility: Visibility::Published($operation_id),
            };
        )*
        $(
            pub(crate) const $uname: Op = Op {
                verb: Verb::$uverb,
                template: $utemplate,
                visibility: Visibility::Unpublished($reason),
            };
        )*

        /// Every entry, for the parity test. `cfg(test)` only — see the module docs for why.
        #[cfg(test)]
        pub(crate) const ALL: &[Op] = &[ $( $name, )* $( $uname, )* ];
    };
}

registry! {
    published {
        ADMIN_RECONCILE_AUTO_JOIN = Post "/api/access/admin/auto-join/reconcile" => "admin_reconcile_auto_join";
        ADMIN_DEMOTE = Post "/api/access/admin/demote" => "admin_demote";
        ADMIN_APPROVE_PRINCIPAL = Post "/api/access/admin/principals/{id}/approve" => "admin_approve_principal";
        ADMIN_DEACTIVATE_PRINCIPAL = Post "/api/access/admin/principals/{id}/deactivate" => "admin_deactivate_principal";
        ADMIN_REACTIVATE_PRINCIPAL = Post "/api/access/admin/principals/{id}/reactivate" => "admin_reactivate_principal";
        ADMIN_REVOKE_PRINCIPAL = Post "/api/access/admin/principals/{id}/revoke" => "admin_revoke_principal";
        ADMIN_LIST_PROFILES = Get "/api/access/admin/profiles" => "admin_list_profiles";
        ADMIN_SHOW_PROFILE = Get "/api/access/admin/profiles/{profile_id}" => "admin_show_profile";
        ADMIN_PROMOTE = Post "/api/access/admin/promote" => "admin_promote";
        ADMIN_LIST_JOIN_REQUESTS = Get "/api/access/admin/requests" => "admin_list_join_requests";
        ADMIN_REVIEW_JOIN_REQUEST = Patch "/api/access/admin/requests/{id}" => "admin_review_join_request";
        ADMIN_COUNT_JOIN_REQUESTS = Get "/api/access/admin/requests/count" => "admin_count_join_requests";
        ADMIN_LIST_REVIEWS = Get "/api/access/admin/reviews" => "admin_list_reviews";
        ADMIN_CLOSE_REVIEW = Patch "/api/access/admin/reviews/{id}" => "admin_close_review";
        ADMIN_COUNT_REVIEWS = Get "/api/access/admin/reviews/count" => "admin_count_reviews";
        ADMIN_GET_SETTINGS = Get "/api/access/admin/settings" => "admin_get_settings";
        ADMIN_UPDATE_SETTINGS = Patch "/api/access/admin/settings" => "admin_update_settings";
        CREATE_REQUEST = Post "/api/access/requests" => "create_request";
        GET_OWN_REQUEST = Get "/api/access/requests/me" => "get_own_request";
        WITHDRAW_REQUEST = Delete "/api/access/requests/me" => "withdraw_request";
        CREATE_REVIEW_REQUEST = Post "/api/access/reviews" => "create_review_request";
        GET_SETTINGS = Get "/api/access/settings" => "get_settings";
        LIST_ADMIN_LEDGER = Get "/api/admin/ledger" => "list_admin_ledger";
        ADMIN_DISCONNECT = Post "/api/admin/slack/links/disconnect" => "admin_disconnect";
        DISCONNECT_ME = Delete "/api/auth/slack/link/me" => "disconnect_me";
        COMMIT_BLOB = Post "/api/blobs" => "commit_blob";
        LIST_BLOBS = Get "/api/blobs" => "list_blobs";
        GET_BLOB = Get "/api/blobs/{id}" => "get_blob";
        BLOB_RELATIONS = Get "/api/blobs/{id}/relations" => "blob_relations";
        RELATE_BLOB = Post "/api/blobs/{id}/relations" => "relate_blob";
        BEGIN_BLOB_UPLOAD = Post "/api/blobs/uploads" => "begin_blob_upload";
        BLOB_UPLOAD_PROGRESS = Get "/api/blobs/uploads/{id}" => "blob_upload_progress";
        FINALIZE_BLOB_UPLOAD = Post "/api/blobs/uploads/{id}/finalize" => "finalize_blob_upload";
        APPEND_BLOB_SEGMENT = Post "/api/blobs/uploads/{id}/segments" => "append_blob_segment";
        RECORD_CITATION_AUDIT_FOR_BLOCK = Post "/api/citation-audits" => "record_citation_audit_for_block";
        GENESIS = Post "/api/cognitive-maps" => "genesis";
        LIST_COGNITIVE_MAPS = Get "/api/cognitive-maps" => "list_cognitive_maps";
        GET_COGNITIVE_MAP = Get "/api/cognitive-maps/{id}" => "get_cognitive_map";
        RECONCILE = Put "/api/cognitive-maps/{id}" => "reconcile";
        ANALYTICS = Get "/api/cognitive-maps/{id}/analytics" => "analytics";
        GRANT_COGMAP_ACCESS = Post "/api/cognitive-maps/{id}/grants" => "grant_cogmap_access";
        REVOKE_COGMAP_ACCESS = Delete "/api/cognitive-maps/{id}/grants" => "revoke_cogmap_access";
        MATERIALIZE = Post "/api/cognitive-maps/{id}/materialize" => "materialize";
        MATERIALIZE_DELTA = Get "/api/cognitive-maps/{id}/materialize-delta" => "materialize_delta";
        REGION_METRICS = Get "/api/cognitive-maps/{id}/region-metrics" => "region_metrics";
        SHAPE = Get "/api/cognitive-maps/{id}/shape" => "shape";
        DECLARE_COGMAP_SHAPE = Post "/api/cognitive-maps/{id}/shapes" => "declare_cogmap_shape";
        LIST_COGMAP_SHAPES = Get "/api/cognitive-maps/{id}/shapes" => "list_cogmap_shapes";
        BIND_TEAM = Post "/api/cognitive-maps/{id}/teams" => "bind_team";
        UNBIND_TEAM = Delete "/api/cognitive-maps/{id}/teams/{team_id}" => "unbind_team";
        LIST_CONNECTIONS = Get "/api/connections" => "list_connections";
        PROVISION_CONNECTION = Post "/api/connections" => "provision_connection";
        GET_CONNECTION = Get "/api/connections/{id}" => "get_connection";
        REVOKE_CONNECTION = Delete "/api/connections/{id}" => "revoke_connection";
        ATTACH_CONNECTION_CREDENTIAL = Post "/api/connections/{id}/credential" => "attach_connection_credential";
        GRANT_CONNECTION_REACH = Post "/api/connections/{id}/reach" => "grant_connection_reach";
        REVOKE_CONNECTION_REACH = Delete "/api/connections/{id}/reach" => "revoke_connection_reach";
        SET_CONNECTION_TOOL_MANIFEST = Post "/api/connections/{id}/tool-manifest" => "set_connection_tool_manifest";
        SET_CONNECTION_WEBHOOK_EVENTS = Post "/api/connections/{id}/webhook-events" => "set_connection_webhook_events";
        CREATE_CONTEXT = Post "/api/contexts" => "create_context";
        LIST_CONTEXTS = Get "/api/contexts" => "list_contexts";
        DELETE_CONTEXT = Delete "/api/contexts/{id}" => "delete_context";
        GET_CONTEXT = Get "/api/contexts/{id}" => "get_context";
        CONTEXT_ANALYTICS = Get "/api/contexts/{id}/analytics" => "context_analytics";
        CONTEXT_MATERIALIZE = Post "/api/contexts/{id}/materialize" => "context_materialize";
        CONTEXT_MATERIALIZE_DELTA = Get "/api/contexts/{id}/materialize-delta" => "context_materialize_delta";
        REASSIGN = Post "/api/contexts/{id}/reassign" => "reassign";
        CONTEXT_REGION_METRICS = Get "/api/contexts/{id}/region-metrics" => "context_region_metrics";
        RENAME = Post "/api/contexts/{id}/rename" => "rename";
        RESTORE_CONTEXT = Post "/api/contexts/{id}/restore" => "restore_context";
        CONTEXT_SHAPE = Get "/api/contexts/{id}/shape" => "context_shape";
        DECLARE_SHAPE = Post "/api/contexts/{id}/shapes" => "declare_shape";
        LIST_SHAPES = Get "/api/contexts/{id}/shapes" => "list_shapes";
        SHARE_TEAM = Post "/api/contexts/{id}/teams" => "share_team";
        UNSHARE_TEAM = Delete "/api/contexts/{id}/teams/{team_id}" => "unshare_team";
        RESOLVE_CONTEXT = Get "/api/contexts/resolve" => "resolve_context";
        GET_ARTIFACT_BY_ID = Get "/api/data-artifacts/{artifact_id}" => "get_artifact_by_id";
        ADMIN_REEMBED = Post "/api/embed/admin/reembed" => "admin_reembed";
        CURSOR = Get "/api/events/{kb_context_id}/cursor" => "cursor";
        SET_FACET = Post "/api/facets" => "set_facet";
        ELEMENT_TRAIL = Get "/api/graph/elements/{kind}/{id}/trail" => "element_trail";
        ENTRY = Get "/api/graph/entry" => "entry";
        TRAVERSE = Get "/api/graph/traverse" => "traverse";
        CREATE_INGEST = Post "/api/ingest" => "create_ingest";
        UPDATE_INGEST = Put "/api/ingest/{id}" => "update_ingest";
        ACCEPT = Post "/api/invitations/accept" => "accept";
        DECLINE = Post "/api/invitations/decline" => "decline";
        LIST_MINE = Get "/api/invitations/mine" => "list_mine";
        COUNT_MINE = Get "/api/invitations/mine/count" => "count_mine";
        LIST_INVOCATIONS = Get "/api/invocations" => "list_invocations";
        OPEN = Post "/api/invocations" => "open";
        SHOW = Get "/api/invocations/{id}" => "show";
        CLOSE = Post "/api/invocations/{id}/close" => "close";
        LIST_MACHINE_CLIENTS = Get "/api/machine-clients" => "list_machine_clients";
        PROVISION_MACHINE_CLIENT = Post "/api/machine-clients" => "provision_machine_client";
        GET_MACHINE_CLIENT = Get "/api/machine-clients/{id}" => "get_machine_client";
        REVOKE_MACHINE_CLIENT = Delete "/api/machine-clients/{id}" => "revoke_machine_client";
        ADMIN_REBIND_MACHINE_CLIENT = Post "/api/machine-clients/{id}/rebind" => "admin_rebind_machine_client";
        ROTATE_MACHINE_CLIENT_SECRET = Post "/api/machine-clients/{id}/rotate-secret" => "rotate_machine_client_secret";
        ISSUE_MACHINE_CREDENTIAL = Post "/api/machine-clients/issue" => "issue_machine_credential";
        GET_PROFILE = Get "/api/profile" => "get_profile";
        UPDATE_PROFILE = Patch "/api/profile" => "update_profile";
        LIST_AUTH_LINKS = Get "/api/profile/auth-links" => "list_auth_links";
        QUERY = Post "/api/query" => "query";
        ASSERT = Post "/api/relationships" => "assert";
        LIST_EDGE_FACETS = Get "/api/relationships/{edge_handle}/facets" => "list_edge_facets";
        SET_EDGE_FACET = Post "/api/relationships/{edge_handle}/facets" => "set_edge_facet";
        RETRACT_EDGE_FACET = Delete "/api/relationships/{edge_handle}/facets/{property_id}" => "retract_edge_facet";
        FOLD = Post "/api/relationships/{edge_handle}/fold" => "fold";
        RETYPE = Post "/api/relationships/{edge_handle}/retype" => "retype";
        REWEIGHT = Post "/api/relationships/{edge_handle}/reweight" => "reweight";
        CREATE_RESOURCE = Post "/api/resources" => "create_resource";
        LIST_RESOURCES = Get "/api/resources" => "list_resources";
        DELETE_RESOURCE = Delete "/api/resources/{id}" => "delete_resource";
        GET_RESOURCE = Get "/api/resources/{id}" => "get_resource";
        UPDATE_RESOURCE = Patch "/api/resources/{id}" => "update_resource";
        COMMIT_ARTIFACT = Post "/api/resources/{id}/artifacts" => "commit_artifact";
        LIST_ARTIFACTS = Get "/api/resources/{id}/artifacts" => "list_artifacts";
        GET_ARTIFACT = Get "/api/resources/{id}/artifacts/{artifact_id}" => "get_artifact";
        APPEND_BLOCK = Post "/api/resources/{id}/blocks" => "append_block";
        LIST_BLOCKS = Get "/api/resources/{id}/blocks" => "list_blocks";
        READ_BLOCK = Get "/api/resources/{id}/blocks/{block_id}" => "read_block";
        RECORD_CITATION_AUDIT = Post "/api/resources/{id}/citation-audits" => "record_citation_audit";
        GET_CONTENT = Get "/api/resources/{id}/content" => "get_content";
        LIST_RESOURCE_EDGES = Get "/api/resources/{id}/edges" => "list_resource_edges";
        RESOURCE_EVIDENCE = Get "/api/resources/{id}/evidence" => "resource_evidence";
        LIST_RESOURCE_FACETS = Get "/api/resources/{id}/facets" => "list_resource_facets";
        FINALIZE_RESOURCE = Post "/api/resources/{id}/finalize" => "finalize_resource";
        GRANT_RESOURCE_ACCESS = Post "/api/resources/{id}/grants" => "grant_resource_access";
        REVOKE_RESOURCE_ACCESS = Delete "/api/resources/{id}/grants" => "revoke_resource_access";
        RESOURCE_LINEAGE = Get "/api/resources/{id}/lineage" => "resource_lineage";
        GET_META = Get "/api/resources/{id}/meta" => "get_meta";
        UPDATE_META = Put "/api/resources/{id}/meta" => "update_meta";
        ANNOTATE_RESOURCE = Post "/api/resources/{id}/provenance" => "annotate_resource";
        PROVENANCE = Get "/api/resources/{id}/provenance" => "provenance";
        REASSIGN_RESOURCE = Post "/api/resources/{id}/reassign" => "reassign_resource";
        REBLOCK_RESOURCES = Post "/api/resources/reblock" => "reblock_resources";
        SEARCH = Post "/api/search" => "search";
        GET_SHAPE = Get "/api/shapes/{shape_id}" => "get_shape";
        DELTA = Get "/api/steward/{cogmap}/delta" => "delta";
        ADVANCE = Post "/api/steward/{cogmap}/watermark" => "advance";
        CREATE_SUBSCRIPTION = Post "/api/subscriptions" => "create_subscription";
        LIST_SUBSCRIPTIONS = Get "/api/subscriptions" => "list_subscriptions";
        GET_SUBSCRIPTION = Get "/api/subscriptions/{id}" => "get_subscription";
        REVOKE_SUBSCRIPTION = Delete "/api/subscriptions/{id}" => "revoke_subscription";
        CREATE_TEAM = Post "/api/teams" => "create_team";
        LIST_TEAMS = Get "/api/teams" => "list_teams";
        DELETE_TEAM = Delete "/api/teams/{id}" => "delete_team";
        DETAIL = Get "/api/teams/{id}" => "detail";
        UPDATE_TEAM = Patch "/api/teams/{id}" => "update_team";
        LIST_TEAM_INVITATIONS = Get "/api/teams/{id}/invitations" => "list_team_invitations";
        REVOKE_TEAM_INVITATION = Delete "/api/teams/{id}/invitations/{invitation_id}" => "revoke_team_invitation";
        CREATE_TEAM_INVITATION = Post "/api/teams/{id}/invite" => "create_team_invitation";
        ADD_MEMBER = Post "/api/teams/{id}/members" => "add_member";
        CHANGE_ROLE = Patch "/api/teams/{id}/members/{profile_id}" => "change_role";
        REMOVE_MEMBER = Delete "/api/teams/{id}/members/{profile_id}" => "remove_member";
        REASSIGN_TEAM = Post "/api/teams/{id}/reassign" => "reassign_team";
    }
    unpublished {
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    #[test]
    fn path_substitutes_placeholders_in_order() {
        let op = Op {
            verb: Verb::Get,
            template: "/api/x/{a}/y/{b}",
            visibility: Visibility::Published("x"),
        };
        assert_eq!(op.path(&[&1, &"two"]), "/api/x/1/y/two");
    }

    #[test]
    fn path_without_placeholders_is_the_template() {
        let op = Op {
            verb: Verb::Get,
            template: "/api/x",
            visibility: Visibility::Published("x"),
        };
        assert_eq!(op.path(&[]), "/api/x");
    }

    #[test]
    #[should_panic(expected = "too few path arguments")]
    fn path_refuses_too_few_arguments() {
        let op = Op {
            verb: Verb::Get,
            template: "/api/x/{a}",
            visibility: Visibility::Published("x"),
        };
        op.path(&[]);
    }

    #[test]
    #[should_panic(expected = "too many path arguments")]
    fn path_refuses_too_many_arguments() {
        let op = Op {
            verb: Verb::Get,
            template: "/api/x",
            visibility: Visibility::Published("x"),
        };
        op.path(&[&1]);
    }

    /// `(verb, template)` for every operation in the repo-root `openapi.json`, keyed by
    /// `operationId`.
    fn spec_operations() -> BTreeMap<String, (String, String)> {
        // CARGO_MANIFEST_DIR is …/crates/temper-client; the spec is two levels up. Read at test
        // time rather than `include_str!`, so the published crate never references a file outside
        // its package.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openapi.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let spec: serde_json::Value = serde_json::from_str(&raw).expect("openapi.json parses");
        let mut ops = BTreeMap::new();
        for (template, item) in spec["paths"].as_object().expect("paths object") {
            for verb in ["get", "post", "put", "patch", "delete"] {
                if let Some(id) = item[verb]["operationId"].as_str() {
                    let prior = ops.insert(id.to_string(), (verb.to_uppercase(), template.clone()));
                    assert!(prior.is_none(), "openapi.json repeats operationId {id}");
                }
            }
        }
        ops
    }

    /// The gate: `temperkb-client` is equal to or a superset of `openapi.json`.
    #[test]
    fn registry_covers_every_openapi_operation() {
        let spec = spec_operations();
        let mut published: BTreeMap<&str, &Op> = BTreeMap::new();
        let mut failures = Vec::new();

        for op in ALL {
            match op.visibility {
                Visibility::Published(id) => {
                    if published.insert(id, op).is_some() {
                        failures.push(format!("{id}: two registry entries name this operation"));
                    }
                }
                Visibility::Unpublished(reason) => {
                    let verb = op.method().to_string();
                    if let Some((id, _)) = spec
                        .iter()
                        .find(|(_, (v, t))| *v == verb && *t == op.template)
                    {
                        failures.push(format!(
                            "{verb} {}: marked unpublished ({reason}) but openapi.json publishes \
                             it as {id} — make it a published entry",
                            op.template
                        ));
                    }
                }
            }
        }

        for (id, (verb, template)) in &spec {
            match published.get(id.as_str()) {
                None => failures.push(format!(
                    "{id} ({verb} {template}): in openapi.json but temper-client has no entry — \
                     add a constant to crates/temper-client/src/ops.rs and a client method that \
                     sends it"
                )),
                Some(op) => {
                    let ours = (op.method().to_string(), op.template.to_string());
                    if ours != (verb.clone(), template.clone()) {
                        failures.push(format!(
                            "{id}: registry says {} {}, openapi.json says {verb} {template} — \
                             correct the constant in crates/temper-client/src/ops.rs",
                            ours.0, ours.1
                        ));
                    }
                }
            }
        }

        let spec_ids: BTreeSet<&str> = spec.keys().map(String::as_str).collect();
        for id in published.keys() {
            if !spec_ids.contains(id) {
                failures.push(format!(
                    "{id}: a published registry entry names an operation openapi.json does not \
                     have — remove it, or mark it Visibility::Unpublished with the reason"
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "temper-client has fallen behind openapi.json ({} problem(s)):\n  {}",
            failures.len(),
            failures.join("\n  ")
        );
    }
}
