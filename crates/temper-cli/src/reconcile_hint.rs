//! Reconcile-don't-retry guidance for the authenticated write path.
//!
//! A resource-mutating call (`resource create`, `resource update`) can fail with
//! a transport-level network error **after the server has already committed the
//! write** — a lost acknowledgment, not a lost write. The request reaches the
//! backend, the mutation is persisted, and the connection then drops before the
//! response returns (the endpoint does trailing post-commit work on the same
//! request, so a cold-start or rolling-deploy window can kill the instance with
//! the row already durable). See [issue #581].
//!
//! The client cannot tell "never committed" from "committed but un-acked" — it
//! only sees the dropped connection. The danger is the *reaction*: because
//! `create` mints a fresh identifier per invocation and the write path carries
//! no idempotency key, a naive retry **creates a duplicate** rather than
//! converging on the already-committed resource. The correct recovery is
//! **reconcile, not retry**: confirm server state before re-issuing.
//!
//! Until a server-side idempotency key or an off-request post-write tail lands,
//! this hint makes the hazard visible at the exact moment a write errors, so the
//! manual mitigation is not tribal knowledge. It fires only for the two commands
//! the issue names (`create`/`update`) and only on a genuine transport error —
//! not on a 4xx/5xx the server actually returned, where the failure is
//! unambiguous and no reconcile is needed.
//!
//! [issue #581]: https://github.com/tasker-systems/temper/issues/581

use temper_core::error::TemperError;

use crate::cli::{Commands, ResourceAction};

/// The multi-line guidance printed after a lost-ack write failure. Written to
/// **stderr** by the caller (via [`crate::output::hint`]) so it never corrupts
/// the JSON-by-default stdout payload.
pub const RECONCILE_HINT: &str = "\
note: a write can fail with a network error *after* the server has already committed it.
This is a lost acknowledgment, not a lost write — it clusters during deploy / cold-start windows.
Do NOT blindly retry: a retried `create` mints a duplicate, since the write path has no idempotency key.
First reconcile, then re-issue only if the write genuinely did not land:
  • create — `temper resource list --title-contains <title>` to see whether the resource is present
  • update — `temper resource show <ref>` (add `--edges` for a `--goal`/link change) to see whether the mutation applied";

/// True when `command` is a write whose retry-after-lost-ack is unsafe: `resource
/// create` (mints a duplicate) and `resource update` (re-applies, and a
/// `--goal`/link change re-asserts an edge). Computed from `&cli.command`
/// **before** dispatch consumes it, then paired with the resulting error by
/// [`reconcile_hint`].
///
/// Idempotent mutations — `resource delete`, `edge assert`, grant / revoke — are
/// deliberately excluded: re-issuing them converges, so they need no reconcile
/// warning.
pub fn is_lost_ack_prone_write(command: &Commands) -> bool {
    matches!(
        command,
        Commands::Resource {
            action: ResourceAction::Create { .. } | ResourceAction::Update { .. }
        }
    )
}

/// The guidance to print, or `None`. Fires only when the invoked command was a
/// lost-ack-prone write (`was_lost_ack_write`, from [`is_lost_ack_prone_write`])
/// **and** the failure is a transport-level network error — the one failure
/// shape where the write may nonetheless have committed. A `4xx`/`5xx` the
/// server actually returned is unambiguous (nothing committed, or the server
/// said exactly what went wrong), so it yields `None`.
pub fn reconcile_hint(was_lost_ack_write: bool, err: &TemperError) -> Option<&'static str> {
    if was_lost_ack_write && matches!(err, TemperError::Network(_)) {
        Some(RECONCILE_HINT)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_action() -> ResourceAction {
        ResourceAction::Create {
            r#type: "research".into(),
            title: Some("t".into()),
            context: Some("@me/x".into()),
            cogmap: None,
            mode: None,
            effort: None,
            open_meta: None,
            goal: None,
            task: None,
            show_template: false,
            stdin: false,
            body: None,
            from: None,
            sources: vec![],
            sources_as_edges: false,
            no_source: false,
            act: crate::cli::ActArgs::default(),
        }
    }

    #[test]
    fn create_is_lost_ack_prone() {
        assert!(is_lost_ack_prone_write(&Commands::Resource {
            action: create_action()
        }));
    }

    #[test]
    fn read_command_is_not_lost_ack_prone() {
        // `DescribeOpenMeta` is a unit read variant — no lost-ack hazard.
        assert!(!is_lost_ack_prone_write(&Commands::Resource {
            action: ResourceAction::DescribeOpenMeta
        }));
    }

    #[test]
    fn non_resource_command_is_not_lost_ack_prone() {
        assert!(!is_lost_ack_prone_write(&Commands::Invitations));
    }

    #[test]
    fn fires_on_write_network_error() {
        let err = TemperError::Network("error sending request".into());
        assert_eq!(reconcile_hint(true, &err), Some(RECONCILE_HINT));
    }

    #[test]
    fn silent_on_write_non_network_error() {
        // A server-returned 409/422/5xx is unambiguous — nothing to reconcile.
        let err = TemperError::Conflict("already exists".into());
        assert_eq!(reconcile_hint(true, &err), None);
    }

    #[test]
    fn silent_on_read_network_error() {
        let err = TemperError::Network("down".into());
        assert_eq!(reconcile_hint(false, &err), None);
    }
}
