//! Reconcile-or-replay guidance for the authenticated write path.
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
//! only sees the dropped connection. What makes a retry safe differs by act:
//!
//! - **A create carries an idempotency key.** The server claims the `(owner, key)`
//!   slot atomically with the create, so a create re-sent with the same key returns
//!   the first write's resource instead of minting a second. A retry is a new CLI
//!   invocation, so the key must outlive the process: the caller supplies it with
//!   `--idempotency-key`, or [`pin_create_idempotency_key`] mints it before dispatch
//!   and the hint printed on a network error names it, so the retry can send it back.
//! - **An update has no key**, and a create re-sent without its first key mints a
//!   duplicate. For those the recovery is **reconcile first**: confirm server state
//!   before re-issuing.
//!
//! The hint fires only for those two commands and only on a genuine transport
//! error — not on a 4xx/5xx the server actually returned, where the failure is
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

use std::borrow::Cow;
use std::sync::Mutex;

use temper_core::error::TemperError;
use uuid::Uuid;

use crate::cli::{AdminAction, AdminErasureAction, Commands, ResourceAction};

/// The opening every lost-ack hint shares: what a network error on a write can mean.
const LOST_ACK_PREAMBLE: &str = "\
note: a write can fail with a network error *after* the server has already committed it.
This is a lost acknowledgment, not a lost write — it clusters during deploy / cold-start windows.";

/// The multi-line guidance printed after a lost-ack write failure that has no idempotency key to
/// replay: an update, or a create whose key is not known. Written to **stderr** by the caller (via
/// [`crate::output::hint`]) so it never corrupts the JSON-by-default stdout payload; in JSON mode
/// it rides the error payload's `hint`.
pub const RECONCILE_HINT: &str = "\
note: a write can fail with a network error *after* the server has already committed it.
This is a lost acknowledgment, not a lost write — it clusters during deploy / cold-start windows.
A `create` is safe to re-issue with the idempotency key it was first sent with (`--idempotency-key <uuid>`):
the server returns the first write if it landed. Without that key a retried `create` mints a duplicate,
and an `update` has no key, so reconcile first and re-issue only if the write genuinely did not land:
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

/// The guidance printed after a create sent with `key` fails with a network error: re-issuing it
/// with the same key is the safe retry, so the hint names the key and needs no reconcile step.
pub fn keyed_create_hint(key: Uuid) -> String {
    format!(
        "{LOST_ACK_PREAMBLE}\n\
This create was sent with idempotency key {key}. Re-issue the same command with `--idempotency-key {key}`:
if the first attempt landed, the server returns that resource instead of creating a second; if it did not,
the create runs once."
    )
}

/// Give a `resource create` the idempotency key it will send and return it: the caller's
/// `--idempotency-key`, else a key minted into the command's `minted_idempotency_key`; `None` for
/// any other command. Run **before** dispatch, so the key exists in `main` beside the error the
/// create may return, and [`reconcile_hint`] can name it.
///
/// The create sends this key, with one exception: a segmented create that resumes an interrupted
/// attempt from its local manifest continues under, and the hint names, the key that attempt
/// saved ([`record_resumed_create_key`]). Every in-process retry of a request re-sends the same body
/// and `Idempotency-Key` header, so one invocation is one key.
pub fn pin_create_idempotency_key(command: &mut Commands) -> Option<Uuid> {
    match command {
        Commands::Resource {
            action:
                ResourceAction::Create {
                    idempotency_key,
                    minted_idempotency_key,
                    ..
                },
        } => Some(
            idempotency_key
                .unwrap_or_else(|| *minted_idempotency_key.get_or_insert_with(Uuid::now_v7)),
        ),
        _ => None,
    }
}

/// The key a segmented create resumed under, when it resumed: `Some(None)` for a legacy manifest
/// that saved no key. Read only by `main`, through [`LostAckProne::as_dispatched`], and only for
/// a top-level `resource create`, which runs one create per process; an in-process caller that
/// creates many (`memory migrate`) may write it, and nothing reads it there.
static RESUMED_CREATE_KEY: Mutex<Option<Option<Uuid>>> = Mutex::new(None);

/// Record that this process's create resumed an interrupted attempt under `key`, the key that
/// attempt saved (`None`: it saved none), so the network-error hint names the key the server saw
/// rather than the one pinned before dispatch.
pub fn record_resumed_create_key(key: Option<Uuid>) {
    if let Ok(mut slot) = RESUMED_CREATE_KEY.lock() {
        *slot = Some(key);
    }
}

fn resumed_create_key() -> Option<Option<Uuid>> {
    RESUMED_CREATE_KEY.lock().ok().and_then(|slot| *slot)
}

/// A write whose outcome a lost acknowledgment can hide, and so which errors call for reconciling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostAckProne {
    /// `resource create`: only a transport error is ambiguous. With its key known, the safe retry
    /// is a replay of that key; without it, a reconcile.
    ResourceCreate { idempotency_key: Option<Uuid> },
    /// `resource update`: only a transport error is ambiguous, and there is no key to replay.
    ResourceUpdate,
    /// A resource, principal or block-history erasure act run with `--execute`: any error but a
    /// refusal or a local parse failure is.
    ErasureAct,
    /// A field scrub run with `--execute`: the erasure acts' trigger, its own reconcile.
    FieldScrubAct,
}

impl LostAckProne {
    /// This classification as dispatch left it: a create that resumed an interrupted attempt names
    /// the key it resumed under ([`record_resumed_create_key`]) instead of the one pinned before
    /// dispatch. Every other classification is returned unchanged.
    pub fn as_dispatched(self) -> Self {
        match (self, resumed_create_key()) {
            (LostAckProne::ResourceCreate { .. }, Some(idempotency_key)) => {
                LostAckProne::ResourceCreate { idempotency_key }
            }
            (other, _) => other,
        }
    }
}

/// Which kind of lost-ack-prone write `command` is, else `None`: `resource create` (a retry without
/// its key mints a duplicate; the key is read from the command, so pin it first with
/// [`pin_create_idempotency_key`]), `resource update`
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
            action:
                ResourceAction::Create {
                    idempotency_key,
                    minted_idempotency_key,
                    ..
                },
        } => Some(LostAckProne::ResourceCreate {
            idempotency_key: idempotency_key.or(*minted_idempotency_key),
        }),
        Commands::Resource {
            action: ResourceAction::Update { .. },
        } => Some(LostAckProne::ResourceUpdate),
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
            // Setting or removing a policy is idempotent: re-running it after a lost answer is
            // harmless, so there is nothing to warn about.
            AdminErasureAction::Retention { .. } => None,
        },
        _ => None,
    }
}

/// The guidance to print, or `None`. For a resource write it fires only on a
/// transport-level network error — the one failure shape where the write may
/// nonetheless have committed; a `4xx`/`5xx` the server returned is unambiguous.
/// For an erasure act it fires on every error but a refusal (`Conflict`: a recorded
/// answer already printed, or the server's `409`) and a request refused before it
/// ran (`BadRequest`: a ref the CLI refused locally), since a gateway's `5xx` can
/// front an act that completed. A server `400` arrives untyped, as `Api`, so it
/// gets the hint too. A create whose key is known is told to replay it ([`keyed_create_hint`]).
pub fn reconcile_hint(prone: Option<LostAckProne>, err: &TemperError) -> Option<Cow<'static, str>> {
    let network = matches!(err, TemperError::Network(_));
    let unrefused = !matches!(err, TemperError::Conflict(_) | TemperError::BadRequest(_));
    match prone? {
        LostAckProne::ResourceCreate {
            idempotency_key: Some(key),
        } => network.then(|| Cow::Owned(keyed_create_hint(key))),
        LostAckProne::ResourceCreate {
            idempotency_key: None,
        }
        | LostAckProne::ResourceUpdate => network.then_some(Cow::Borrowed(RECONCILE_HINT)),
        LostAckProne::ErasureAct => unrefused.then_some(Cow::Borrowed(ERASURE_RECONCILE_HINT)),
        LostAckProne::FieldScrubAct => {
            unrefused.then_some(Cow::Borrowed(FIELD_SCRUB_RECONCILE_HINT))
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
            idempotency_key: None,
            minted_idempotency_key: None,
            act: crate::cli::ActArgs::default(),
        }
    }

    #[test]
    fn create_is_lost_ack_prone() {
        assert_eq!(
            lost_ack_prone(&Commands::Resource {
                action: create_action()
            }),
            Some(LostAckProne::ResourceCreate {
                idempotency_key: None
            })
        );
    }

    /// FAILS IF pinning overwrites a key the caller gave, mints none for a create, or touches any
    /// other command; or if the classification does not carry the pinned key to the hint.
    #[test]
    fn pinning_keeps_a_given_key_mints_a_missing_one_and_feeds_the_hint() {
        let given = Uuid::now_v7();
        let mut keyed = Commands::Resource {
            action: create_action(),
        };
        if let Commands::Resource {
            action: ResourceAction::Create {
                idempotency_key, ..
            },
        } = &mut keyed
        {
            *idempotency_key = Some(given);
        }
        assert_eq!(pin_create_idempotency_key(&mut keyed), Some(given));
        assert_eq!(
            lost_ack_prone(&keyed),
            Some(LostAckProne::ResourceCreate {
                idempotency_key: Some(given)
            })
        );

        let mut unkeyed = Commands::Resource {
            action: create_action(),
        };
        let minted = pin_create_idempotency_key(&mut unkeyed).expect("a create gets a key");
        let Commands::Resource {
            action:
                ResourceAction::Create {
                    idempotency_key,
                    minted_idempotency_key,
                    ..
                },
        } = &unkeyed
        else {
            unreachable!("built as a create")
        };
        assert_eq!(
            *idempotency_key, None,
            "a minted key never poses as the caller's, which `main` reads as given"
        );
        assert_eq!(*minted_idempotency_key, Some(minted));
        assert_eq!(
            pin_create_idempotency_key(&mut unkeyed),
            Some(minted),
            "a second pin keeps the first key"
        );
        assert_eq!(
            lost_ack_prone(&unkeyed),
            Some(LostAckProne::ResourceCreate {
                idempotency_key: Some(minted)
            })
        );

        assert_eq!(pin_create_idempotency_key(&mut Commands::Invitations), None);
    }

    /// FAILS IF a keyed create's network error does not name its key in the flag the retry takes,
    /// or tells the caller to reconcile instead of replaying.
    #[test]
    fn a_keyed_create_network_error_names_the_key_to_replay() {
        let key = Uuid::now_v7();
        let hint = reconcile_hint(
            Some(LostAckProne::ResourceCreate {
                idempotency_key: Some(key),
            }),
            &TemperError::Network("connection reset".into()),
        )
        .expect("a network error on a create is hinted");
        assert!(hint.starts_with(LOST_ACK_PREAMBLE), "{hint}");
        assert!(hint.contains(&format!("--idempotency-key {key}")), "{hint}");
        assert!(!hint.contains("--title-contains"), "{hint}");
    }

    /// FAILS IF the key-less hint stops stating present truth: the write path has a key, a create
    /// replays safely with it, and only a caller without it must reconcile first.
    #[test]
    fn the_keyless_hint_names_the_flag_and_keeps_reconcile_first() {
        assert!(RECONCILE_HINT.starts_with(LOST_ACK_PREAMBLE));
        assert!(RECONCILE_HINT.contains("--idempotency-key"));
        assert!(!RECONCILE_HINT.contains("has no idempotency key"));
        assert!(RECONCILE_HINT.contains("--title-contains"));
        assert!(RECONCILE_HINT.contains("temper resource show"));
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
                reconcile_hint(prone, &err).as_deref(),
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
            assert_eq!(hint.as_deref(), Some(FIELD_SCRUB_RECONCILE_HINT), "{err}");
            let hint = hint.unwrap_or_default();
            assert!(hint.contains("without --execute"), "{hint}");
            assert!(hint.contains("nothing is\nprior"), "{hint}");
            assert!(hint.contains("already cleared"), "{hint}");
            assert!(!hint.contains("already_erased"), "{hint}");
        }
        for err in [
            TemperError::Conflict("field scrub refused (sentinel_collision)".into()),
            TemperError::BadRequest("invalid resource ref".into()),
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
        for prone in [
            LostAckProne::ResourceUpdate,
            LostAckProne::ResourceCreate {
                idempotency_key: None,
            },
        ] {
            assert_eq!(
                reconcile_hint(Some(prone), &err).as_deref(),
                Some(RECONCILE_HINT),
                "{prone:?}"
            );
        }
    }

    #[test]
    fn silent_on_write_non_network_error() {
        // A server-returned 409/422/5xx is unambiguous — nothing to reconcile.
        let err = TemperError::Conflict("already exists".into());
        for prone in [
            LostAckProne::ResourceUpdate,
            LostAckProne::ResourceCreate {
                idempotency_key: Some(Uuid::now_v7()),
            },
        ] {
            assert_eq!(reconcile_hint(Some(prone), &err), None, "{prone:?}");
        }
    }

    #[test]
    fn silent_on_read_network_error() {
        let err = TemperError::Network("down".into());
        assert_eq!(reconcile_hint(None, &err), None);
    }
}
