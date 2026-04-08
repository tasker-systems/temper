//! Team membership commands: join, status, leave.

use crate::output;
use temper_core::types::access_gate::JoinRequestStatus;

/// Submit a join request for a team (defaults to system gating team).
pub fn join(message: Option<&str>) -> crate::error::Result<()> {
    let message = message.map(|s| s.to_string());
    crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            match client
                .access()
                .create_request(message.as_deref(), "cli", None)
                .await
            {
                Ok(result) => {
                    output::success("Access request submitted.");
                    output::plain("  You'll gain access once an admin approves your request.");
                    output::hint("  Run `temper team status` to check.");
                    output::blank();
                    output::dim(format!("  Request ID: {}", result.id));
                }
                Err(temper_client::error::ClientError::Conflict { .. }) => {
                    output::warning("You already have a pending request.");
                    output::hint("  Run `temper team status` to check its status.");
                }
                Err(e) => return Err(crate::commands::client_err(e)),
            }

            Ok(())
        })
    })
}

/// Check the status of the caller's join request.
pub fn status() -> crate::error::Result<()> {
    crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            let request = client
                .access()
                .get_own_request()
                .await
                .map_err(crate::commands::client_err)?;

            match request {
                None => {
                    output::plain("You haven't requested access yet.");
                    output::hint("Run `temper team join` to get started.");
                }
                Some(req) => match req.status {
                    JoinRequestStatus::Pending => {
                        output::plain(format!(
                            "Your request is pending review (submitted {}).",
                            req.created.format("%Y-%m-%d")
                        ));
                    }
                    JoinRequestStatus::Approved => {
                        let reviewed = req
                            .reviewed_at
                            .map(|d| d.format("%Y-%m-%d").to_string())
                            .unwrap_or_else(|| "unknown date".to_string());
                        output::success(format!("You have access. Approved on {reviewed}."));
                    }
                    JoinRequestStatus::Rejected => {
                        output::warning("Your previous request was not approved.");
                        output::hint(
                            "You can submit a new one with `temper team join --message \"...\"`.",
                        );
                    }
                    JoinRequestStatus::Withdrawn => {
                        output::plain("You withdrew your request.");
                        output::hint("Submit a new one with `temper team join --message \"...\"`.");
                    }
                },
            }

            Ok(())
        })
    })
}

/// Withdraw a pending request or leave a team.
pub fn leave() -> crate::error::Result<()> {
    crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            let request = client
                .access()
                .get_own_request()
                .await
                .map_err(crate::commands::client_err)?;

            match request {
                None => {
                    output::plain("Nothing to leave — you don't have a pending request.");
                }
                Some(req) => match req.status {
                    JoinRequestStatus::Pending => {
                        client
                            .access()
                            .withdraw_request()
                            .await
                            .map_err(crate::commands::client_err)?;
                        output::success("Request withdrawn.");
                    }
                    JoinRequestStatus::Approved => {
                        output::plain("To leave a team after approval, contact an admin.");
                    }
                    _ => {
                        output::plain("Nothing to leave — no active request or membership.");
                    }
                },
            }

            Ok(())
        })
    })
}
