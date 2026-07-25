//! Keeping credentials out of span attributes.
//!
//! ## The leak this was built for is now closed at the source
//!
//! This module shipped in PR #540 as a **stopgap**: `POST /api/invitations/{token}/accept` (and
//! `/decline`) carried a **bearer capability token as a path segment**, and two recording sites put
//! that path into an exported span attribute. `invitation_service` mints the token as 128 CSPRNG bits
//! and its own docs say *"the token IS the authority"* — a credential, valid seven days.
//!
//! Those routes are gone. The token now rides in the request body
//! (`temper_core::types::invitation::InvitationTokenRequest`), so no temper route carries a credential
//! in its path and this function has no live leak to stop.
//!
//! ## Why it stays anyway
//!
//! Three reasons, none of them sentiment:
//!
//! 1. **It guards reintroduction.** The behaviour below is deny-by-default: any *unrecognised* segment
//!    under `/api/invitations/` is treated as a possible credential and redacted. A future route that
//!    reintroduces the mistake is covered the day it lands, not the day someone notices.
//! 2. **It is the attachment seam** for the general mechanism — goal
//!    `019f99dd-dc9c-79f1-947c-e61bde2148a9`, a hard-redact set plus an allow/deny registry across Rust
//!    and TypeScript. Deleting the seam would mean rebuilding it.
//! 3. **Its tests are the regression proof** that this class of leak is closed. They still assert that
//!    a token-shaped path never survives into a span attribute — which is now a statement about a
//!    shape no route produces, and that is exactly what a regression test is for.
//!
//! ## Keyed on the route, not on the shape of the value
//!
//! A pattern like "redact anything that looks like 32 hex characters" would be shape-keyed, and this
//! repo has already paid for that mistake elsewhere: guards key on the **bug**, not on the shape. It
//! would also be wrong in both directions — it would miss a token that happened to look like a slug,
//! and it would corrupt the many paths that legitimately carry a UUID.
//!
//! So the discrimination is by **route segment**, against an explicit list of the segments this API
//! actually serves. That list is the load-bearing part: when the token moved to the body, the two new
//! segments (`accept`, `decline`) had to join it, or deny-by-default would have redacted the very
//! routes that fixed the leak — recording both as `/api/invitations/{token}`, collapsing two distinct
//! routes into one attribute that also falsely reads as a redacted credential.

use std::borrow::Cow;

/// Prefix of the route family that once carried a credential in its path, and that deny-by-default
/// still guards.
const INVITATION_PREFIX: &str = "/api/invitations/";

/// The segments under `INVITATION_PREFIX` that this API actually serves, none of which is a token.
///
/// **Adding an `/api/invitations/*` route means adding its segment here.** Omitting it is not a silent
/// no-op: the segment gets redacted to `{token}` and that route becomes unobservable in traces. The
/// direction of the failure is deliberate — an unknown segment is treated as a credential, so the
/// mistake is a lost trace attribute rather than a leaked token.
const INVITATION_NON_TOKEN_SEGMENTS: [&str; 3] = ["mine", "accept", "decline"];

/// What replaces a redacted segment: a placeholder that reads like a route parameter rather than like
/// a corrupted path, so a trace showing `/api/invitations/{token}` is legible as "a credential was
/// removed here" — which, now that no route puts one there, means "something reintroduced one".
const TOKEN_PLACEHOLDER: &str = "{token}";

/// Replace a credential embedded in a request path with a placeholder.
///
/// Returns `Cow::Borrowed` for the overwhelming majority of paths, so the common case allocates
/// nothing — this runs on the root span of every request on both surfaces.
pub fn redact_path(path: &str) -> Cow<'_, str> {
    let Some(rest) = path.strip_prefix(INVITATION_PREFIX) else {
        return Cow::Borrowed(path);
    };

    // Compare the whole segment, not a prefix: a token that happened to start with "mine" must still
    // be redacted.
    let (segment, tail) = match rest.split_once('/') {
        Some((segment, tail)) => (segment, Some(tail)),
        None => (rest, None),
    };
    if INVITATION_NON_TOKEN_SEGMENTS.contains(&segment) || segment.is_empty() {
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

    /// The shape the retired routes had. No route produces it any more — which is the point: this is
    /// the regression proof that reintroducing a token-in-path is covered the day it lands.
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

    /// The live invitation routes carry no credential and must survive untouched.
    ///
    /// This is the case deny-by-default gets wrong if the segment list is not maintained: `accept` and
    /// `decline` are not tokens, but they are under the guarded prefix, so an unmaintained list
    /// redacts them to `/api/invitations/{token}` — collapsing two routes into one attribute that also
    /// falsely reads as a caught credential. Verified to fail against the pre-move segment list.
    #[test]
    fn the_live_invitation_routes_are_not_mistaken_for_tokens() {
        for path in [
            "/api/invitations/accept",
            "/api/invitations/decline",
            "/api/invitations/mine",
        ] {
            let redacted = redact_path(path);
            assert_eq!(
                redacted, path,
                "`{path}` is a real route segment, not a token; redacting it makes the route \
                 unobservable and looks like a credential was caught"
            );
        }
    }

    /// Redaction must not corrupt the paths that carry no credential — which is nearly all of them.
    #[test]
    fn every_other_path_is_returned_untouched_and_unallocated() {
        for path in [
            "/api/invitations/mine",
            "/api/invitations/accept",
            "/api/invitations/decline",
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
