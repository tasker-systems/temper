//! Keeping credentials out of span attributes.
//!
//! ## Why this exists as one line of defence and not as a system
//!
//! It is a **stopgap with a named successor**, and saying so is the point: goal
//! `019f99dd-dc9c-79f1-947c-e61bde2148a9` owns the real mechanism — a hard-redact set plus an
//! allow/deny registry, applied across Rust and TypeScript. This module covers the one leak that could
//! not wait for it, because the leak is live and egressing.
//!
//! ## The leak
//!
//! `POST /api/invitations/{token}/accept` (and `/decline`) put a **bearer capability token in the URL
//! path**. `invitation_service` mints it as 128 CSPRNG bits and its own docs say *"the token IS the
//! authority"* — it is a credential, not an identifier, and it is valid for seven days.
//!
//! Two recording sites put that path into a span attribute:
//!
//! - the server's root span, `path = %request.uri().path()` (the `root_span!` macro), and
//! - temper-client's outbound span, `request = "{method} {path}"` (`ApiRequest`'s `Display`, whose
//!   comment claims it *"Never contains sensitive data (tokens, bodies)"*).
//!
//! While nothing exported, that was a credential in our own logs — bad, and bounded. With OTLP export
//! it is a live credential sent **to a third-party vendor**, sitting in a trace store for as long as
//! their retention says, readable by anyone with vendor access.
//!
//! ## Keyed on the route, not on the shape of the value
//!
//! A pattern like "redact anything that looks like 32 hex characters" would be shape-keyed, and this
//! repo has already paid for that mistake elsewhere: guards key on the **bug**, not on the shape. It
//! would also be wrong in both directions — it would miss a token that happened to look like a slug,
//! and it would corrupt the many paths that legitimately carry a UUID.
//!
//! So: an explicit list of route shapes known to carry a credential in the path. Adding a route that
//! does the same is a deliberate act, and the honest fix is to stop putting credentials in paths at
//! all — tracked as its own task, because moving the token to the request body is a wire-contract
//! change that does not belong inside a telemetry PR.

use std::borrow::Cow;

/// Prefix of the one route family that carries a credential in its path.
const INVITATION_PREFIX: &str = "/api/invitations/";

/// The sub-path that is *not* a token, so it must survive untouched.
const INVITATION_NOT_A_TOKEN: &str = "mine";

/// What replaces a redacted segment. Deliberately the OpenAPI parameter name, so a trace reads like
/// the route it is — `/api/invitations/{token}/accept` — rather than like a corrupted path.
const TOKEN_PLACEHOLDER: &str = "{token}";

/// Replace a credential embedded in a request path with a placeholder.
///
/// Returns `Cow::Borrowed` for the overwhelming majority of paths, so the common case allocates
/// nothing — this runs on the root span of every request on both surfaces.
pub fn redact_path(path: &str) -> Cow<'_, str> {
    let Some(rest) = path.strip_prefix(INVITATION_PREFIX) else {
        return Cow::Borrowed(path);
    };

    // `/api/invitations/mine` carries no token. Compare the whole segment, not a prefix: a token that
    // happened to start with "mine" must still be redacted.
    let (segment, tail) = match rest.split_once('/') {
        Some((segment, tail)) => (segment, Some(tail)),
        None => (rest, None),
    };
    if segment == INVITATION_NOT_A_TOKEN || segment.is_empty() {
        return Cow::Borrowed(path);
    }

    Cow::Owned(match tail {
        Some(tail) => format!("{INVITATION_PREFIX}{TOKEN_PLACEHOLDER}/{tail}"),
        None => format!("{INVITATION_PREFIX}{TOKEN_PLACEHOLDER}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two routes that actually carry the credential.
    #[test]
    fn an_invitation_token_never_survives_into_a_span_attribute() {
        // A real-shaped token: 128 CSPRNG bits, hex-encoded, per `invitation_service`.
        let token = "9f8e7d6c5b4a39281706f5e4d3c2b1a0";

        for (path, expected) in [
            (
                format!("/api/invitations/{token}/accept"),
                "/api/invitations/{token}/accept",
            ),
            (
                format!("/api/invitations/{token}/decline"),
                "/api/invitations/{token}/decline",
            ),
            // Defensive: a future sub-route, and the bare form, must not leak either.
            (
                format!("/api/invitations/{token}"),
                "/api/invitations/{token}",
            ),
            (
                format!("/api/invitations/{token}/something-new"),
                "/api/invitations/{token}/something-new",
            ),
        ] {
            let redacted = redact_path(&path);
            assert_eq!(redacted, expected);
            assert!(
                !redacted.contains(token),
                "the token survived redaction of `{path}`: {redacted}"
            );
        }
    }

    /// Redaction must not corrupt the paths that carry no credential — which is nearly all of them.
    #[test]
    fn every_other_path_is_returned_untouched_and_unallocated() {
        for path in [
            "/api/invitations/mine",
            "/api/resources",
            "/api/resources/019f97a7-ad61-7e40-b325-73028060ac06",
            "/api/profile",
            "/api/teams/some-team/members",
            "/",
            "",
        ] {
            let redacted = redact_path(path);
            assert_eq!(redacted, path);
            assert!(
                matches!(redacted, Cow::Borrowed(_)),
                "`{path}` allocated; this runs on every request's root span, so the untouched case \
                 must borrow"
            );
        }
    }

    /// `mine` is matched as a whole segment. A token beginning with those four letters is still a
    /// token, and a prefix comparison would have leaked it.
    #[test]
    fn a_token_that_starts_with_mine_is_still_redacted() {
        let redacted = redact_path("/api/invitations/mine9f8e7d6c5b4a3928/accept");
        assert_eq!(redacted, "/api/invitations/{token}/accept");
    }
}
