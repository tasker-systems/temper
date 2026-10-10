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
//!
//! The erasure acts (`admin erasure … --execute`) share the hazard with a different recovery and a
//! wider trigger. An operator who sees an error does not know whether the act ran — and for an
//! erasure a server-returned error is ambiguous too: a gateway can answer `504` while the act
//! completes behind it. So the erasure hints fire on any error except the two whose outcome is
//! known — a refusal (its recorded answer was printed) and a ref refused before anything was sent —
//! and point at the survey, which answers the question.
//!
//! A blind retry is not equally safe across the family. A second resource or principal erasure is
//! answered `already_erased` (or as a no-op). A second field scrub is answered "nothing prior"
//! (keep mode) or "already cleared" (`--clear`) only while nothing has been written to the field
//! since the first landed: if the owner set a new title or value in between, a retried `--clear`
//! clears that new value, and a retried keep-mode scrub redacts what became prior. So the field
//! scrub has its own hint, which names the two answers that mean the act landed.

use temper_core::error::TemperError;

use crate::cli::{AdminAction, AdminErasureAction, Commands, ResourceAction};

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

/// The guidance printed after an erasure act fails with a network error.
pub const ERASURE_RECONCILE_HINT: &str = "\
note: the erasure may have run even though no answer came back — a network error cannot say.
Re-run the same command without --execute: the survey shows the current state (a resource already
erased reads `already_erased`). Re-issue with --execute only if the survey shows the act did not land.";

/// The guidance printed after a field scrub fails with a network error: the survey of the same
/// request reads the two answers a landed scrub leaves.
pub const FIELD_SCRUB_RECONCILE_HINT: &str = "\
note: the field scrub may have run even though no answer came back — a network error cannot say.
Re-run the same command without --execute. The act landed if the survey answers that nothing is
prior (keep mode: a plan with no redacted_fields) or that the field is already cleared (--clear).
Re-issue with --execute only if the survey shows the act did not land: a write to the field since
then would make a retry act on the new value.";

/// A write whose outcome a lost acknowledgment can hide, and so which errors call for reconciling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostAckProne {
    /// `resource create` / `update`: only a transport error is ambiguous.
    ResourceWrite,
    /// A resource, principal or block-history erasure act run with `--execute`: any error but a
    /// refusal or a local parse failure is.
    ErasureAct,
    /// A field scrub run with `--execute`: the erasure acts' trigger, its own reconcile.
    FieldScrubAct,
}

/// Which kind of lost-ack-prone write `command` is, else `None`: `resource create` (a retry mints a duplicate), `resource update`
/// (re-applies, and a `--goal`/link change re-asserts an edge), and an erasure act run with
/// `--execute`. Computed from `&cli.command` **before** dispatch consumes it, then paired with
/// the resulting error by [`reconcile_hint`].
///
/// Idempotent mutations — `resource delete`, `edge assert`, grant / revoke — are
/// deliberately excluded: re-issuing them converges, so they need no reconcile
/// warning. An erasure survey is a read.
pub fn lost_ack_prone(command: &Commands) -> Option<LostAckProne> {
    match command {
        Commands::Resource {
            action: ResourceAction::Create { .. } | ResourceAction::Update { .. },
        } => Some(LostAckProne::ResourceWrite),
        Commands::Admin {
            action: AdminAction::Erasure { action },
        } => match action {
            AdminErasureAction::Resource { execute, .. }
            | AdminErasureAction::Principal { execute, .. }
            | AdminErasureAction::BlockHistory { execute, .. } => {
                execute.then_some(LostAckProne::ErasureAct)
            }
            AdminErasureAction::Field { execute, .. } => {
                execute.then_some(LostAckProne::FieldScrubAct)
            }
        },
        _ => None,
    }
}

/// The guidance to print, or `None`. For a resource write it fires only on a
/// transport-level network error — the one failure shape where the write may
/// nonetheless have committed; a `4xx`/`5xx` the server returned is unambiguous.
/// For an erasure act it fires on every error but a refusal (`Conflict`, whose
/// recorded answer was already printed) and a ref refused locally (`BadRequest`),
/// since a gateway's `5xx` can front an act that completed.
pub fn reconcile_hint(prone: Option<LostAckProne>, err: &TemperError) -> Option<&'static str> {
    match prone? {
        LostAckProne::ResourceWrite => {
            matches!(err, TemperError::Network(_)).then_some(RECONCILE_HINT)
        }
        LostAckProne::ErasureAct => {
            (!matches!(err, TemperError::Conflict(_) | TemperError::BadRequest(_)))
                .then_some(ERASURE_RECONCILE_HINT)
        }
        LostAckProne::FieldScrubAct => {
            (!matches!(err, TemperError::Conflict(_) | TemperError::BadRequest(_)))
                .then_some(FIELD_SCRUB_RECONCILE_HINT)
        }
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
            preserve_source: false,
            act: crate::cli::ActArgs::default(),
        }
    }

    #[test]
    fn create_is_lost_ack_prone() {
        assert_eq!(
            lost_ack_prone(&Commands::Resource {
                action: create_action()
            }),
            Some(LostAckProne::ResourceWrite)
        );
    }

    fn erasure(execute: bool) -> Commands {
        Commands::Admin {
            action: AdminAction::Erasure {
                action: AdminErasureAction::Resource {
                    resource: "r".into(),
                    also_strike_blobs: vec![],
                    execute,
                },
            },
        }
    }

    #[test]
    fn an_erasure_survey_is_a_read() {
        assert_eq!(lost_ack_prone(&erasure(false)), None);
    }

    /// FAILS IF a server-returned error on an executed erasure goes unhinted: a gateway `5xx`
    /// can front an act that completed, so it is as ambiguous as a dropped connection.
    #[test]
    fn an_executed_erasure_points_at_the_survey_on_network_and_server_errors() {
        let prone = lost_ack_prone(&erasure(true));
        for err in [
            TemperError::Network("connection reset".into()),
            TemperError::Api("504 Gateway Timeout".into()),
        ] {
            assert_eq!(
                reconcile_hint(prone, &err),
                Some(ERASURE_RECONCILE_HINT),
                "{err}"
            );
        }
    }

    /// FAILS IF an executed field scrub goes unhinted, or its listing or survey is treated as a
    /// write.
    #[test]
    fn only_an_executed_field_scrub_is_lost_ack_prone() {
        let field = |field: Option<crate::cli::CliScrubField>, execute: bool| Commands::Admin {
            action: AdminAction::Erasure {
                action: AdminErasureAction::Field {
                    resource: "r".into(),
                    field,
                    family: None,
                    clear: false,
                    execute,
                },
            },
        };
        let title = Some(crate::cli::CliScrubField::Title);
        assert_eq!(lost_ack_prone(&field(None, false)), None, "the listing");
        assert_eq!(lost_ack_prone(&field(title, false)), None, "the survey");
        assert_eq!(
            lost_ack_prone(&field(title, true)),
            Some(LostAckProne::FieldScrubAct)
        );
    }

    /// FAILS IF an executed field scrub that lost its answer is pointed at the erasure hint (whose
    /// `already_erased` a field scrub never answers) instead of its own, which names the survey's
    /// two answers for a landed scrub; or if a refusal or a request refused before sending is hinted.
    #[test]
    fn an_executed_field_scrub_points_at_its_own_survey_answers() {
        let prone = lost_ack_prone(&Commands::Admin {
            action: AdminAction::Erasure {
                action: AdminErasureAction::Field {
                    resource: "r".into(),
                    field: Some(crate::cli::CliScrubField::Title),
                    family: None,
                    clear: true,
                    execute: true,
                },
            },
        });
        for err in [
            TemperError::Network("connection reset".into()),
            TemperError::Api("504 Gateway Timeout".into()),
        ] {
            let hint = reconcile_hint(prone, &err);
            assert_eq!(hint, Some(FIELD_SCRUB_RECONCILE_HINT), "{err}");
            let hint = hint.unwrap_or_default();
            assert!(hint.contains("without --execute"), "{hint}");
            assert!(hint.contains("nothing is\nprior"), "{hint}");
            assert!(hint.contains("already cleared"), "{hint}");
            assert!(!hint.contains("already_erased"), "{hint}");
        }
        for err in [
            TemperError::Conflict("field scrub refused (sentinel_collision)".into()),
            TemperError::BadRequest("field title is already cleared".into()),
        ] {
            assert_eq!(reconcile_hint(prone, &err), None, "{err}");
        }
    }

    #[test]
    fn a_refused_or_unsent_erasure_needs_no_reconcile() {
        let prone = lost_ack_prone(&erasure(true));
        for err in [
            TemperError::Conflict("resource erasure refused (charter_resource)".into()),
            TemperError::BadRequest("invalid resource ref".into()),
        ] {
            assert_eq!(reconcile_hint(prone, &err), None, "{err}");
        }
    }

    #[test]
    fn read_command_is_not_lost_ack_prone() {
        // `DescribeOpenMeta` is a read — no lost-ack hazard.
        assert_eq!(
            lost_ack_prone(&Commands::Resource {
                action: ResourceAction::DescribeOpenMeta { local: false }
            }),
            None
        );
    }

    #[test]
    fn non_resource_command_is_not_lost_ack_prone() {
        assert_eq!(lost_ack_prone(&Commands::Invitations), None);
    }

    #[test]
    fn fires_on_write_network_error() {
        let err = TemperError::Network("error sending request".into());
        assert_eq!(
            reconcile_hint(Some(LostAckProne::ResourceWrite), &err),
            Some(RECONCILE_HINT)
        );
    }

    #[test]
    fn silent_on_write_non_network_error() {
        // A server-returned 409/422/5xx is unambiguous — nothing to reconcile.
        let err = TemperError::Conflict("already exists".into());
        assert_eq!(
            reconcile_hint(Some(LostAckProne::ResourceWrite), &err),
            None
        );
    }

    #[test]
    fn silent_on_read_network_error() {
        let err = TemperError::Network("down".into());
        assert_eq!(reconcile_hint(None, &err), None);
    }
}
