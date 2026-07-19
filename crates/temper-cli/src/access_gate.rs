//! Rendering for the `SystemAccessRequired` 403.
//!
//! Lives in the lib rather than in `main.rs` so the commands it advertises are
//! reachable by a test. Every string here tells a user to run something, and
//! the only honest check of such a string is the parser itself — see the tests
//! at the bottom, which feed each one to clap exactly as a shell would.

use crate::output;

/// The command a caller with a pending request runs to check on it.
///
/// CLI-authored, unlike
/// [`REQUEST_ACCESS_COMMAND`](temper_core::types::access_gate::REQUEST_ACCESS_COMMAND),
/// which arrives over the wire in the 403 payload.
pub const CHECK_STATUS_COMMAND: &str = "temper auth status";

/// Renders the "you're signed in, but this instance requires approved access"
/// error, tailored to whether the caller has a request outstanding.
///
/// `cli_command` is the server's advertised remedy (see
/// [`REQUEST_ACCESS_COMMAND`](temper_core::types::access_gate::REQUEST_ACCESS_COMMAND));
/// it is `Option` because it crosses the wire and an older server may omit it.
pub fn render_system_access_required(
    email: Option<&str>,
    join_request_status: Option<&str>,
    request_url: Option<&str>,
    cli_command: Option<&str>,
) {
    let identity = email.unwrap_or("your account");
    output::error(format!(
        "You're signed in as {identity}, but this temper instance\n  requires approved access."
    ));
    output::blank_err();

    match join_request_status {
        Some("pending") => {
            output::plain_err("  Your access request is pending review.");
            output::hint(format!(
                "  Run `{CHECK_STATUS_COMMAND}` to check for updates."
            ));
        }
        Some("rejected") => {
            output::plain_err(
                "  Your previous request was not approved. You can submit a new one:",
            );
            if let Some(cmd) = cli_command {
                output::hint(format!("    {cmd}"));
            }
        }
        Some("withdrawn") => {
            output::plain_err("  You withdrew your previous request. Submit a new one:");
            if let Some(cmd) = cli_command {
                output::hint(format!("    {cmd}"));
            }
        }
        _ => {
            output::plain_err("  To request access, run:");
            if let Some(cmd) = cli_command {
                output::hint(format!("    {cmd}"));
            }
            if let Some(url) = request_url {
                output::blank_err();
                output::plain_err(format!("  Or visit: {url}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use temper_core::types::access_gate::REQUEST_ACCESS_COMMAND;

    /// Parses an advertised command the way a user's shell would, then hands it
    /// to clap.
    ///
    /// The split must be shell-aware, not `split_whitespace`: `--message "..."`
    /// is two argv entries, and a naive split would feed clap a token the user
    /// never typed.
    fn parses_as_a_real_command(advertised: &str) -> Result<(), String> {
        let argv = shlex::split(advertised)
            .ok_or_else(|| format!("`{advertised}` is not even shell-splittable"))?;
        crate::cli::Cli::try_parse_from(&argv).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Both advertised commands must parse against the real clap tree.
    ///
    /// This is a regression pin for a bug where the 403 sent users to
    /// `temper team join --message "..."` — a command that *exists* but takes a
    /// positional invitation token and has no `--message`, so it accepts an
    /// invite instead of requesting access. The existing `DOCUMENTED_VERBS` test
    /// could not catch it: that gate resolves verb *paths*, and `["team","join"]`
    /// resolves. Only feeding the whole string, flags included, to the parser
    /// falsifies it.
    #[test]
    fn advertised_commands_parse_against_the_clap_tree() {
        for advertised in [REQUEST_ACCESS_COMMAND, CHECK_STATUS_COMMAND] {
            if let Err(err) = parses_as_a_real_command(advertised) {
                panic!(
                    "the access-gate 403 tells users to run `{advertised}`, \
                     but that does not parse: {err}"
                );
            }
        }
    }

    /// A command that parses is necessary but not sufficient — `temper team join
    /// --message "..."` failed on the flag, but a wrong verb that happened to
    /// take the same flags would sail through the test above. Pin the verbs too.
    #[test]
    fn advertised_commands_name_the_intended_verbs() {
        assert!(
            REQUEST_ACCESS_COMMAND.starts_with("temper auth request-access"),
            "the 403's remedy must request system access, not accept a team \
             invitation: got `{REQUEST_ACCESS_COMMAND}`"
        );
        assert_eq!(CHECK_STATUS_COMMAND, "temper auth status");
    }

    /// Proves the test above can actually fail — a gate that cannot go red is
    /// not a gate. This is the exact string that shipped.
    #[test]
    fn the_shipped_bug_would_now_be_caught() {
        assert!(
            parses_as_a_real_command("temper team join --message \"...\"").is_err(),
            "`team join` has no --message; if this parses, the gate is inert"
        );
        assert!(
            parses_as_a_real_command("temper team status").is_err(),
            "`team status` does not exist; if this parses, the gate is inert"
        );
    }
}
