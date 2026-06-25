//! Resource addressing primitives — the one decorated-ref resolver.
//!
//! Identity contract (Adjudication 5): a resource is addressed by a bare
//! UUID or the decorated form `sluggify(title)-<uuid>`. Resolution is
//! trailing-UUID-only — the decoration is parsed off and ignored, so a
//! stale or wrong slug half is harmless. Decorations are never stored,
//! never authoritative. This module migrates to `temper-workflow` at
//! post-cutover crate extraction.

use temper_core::error::TemperError;
use temper_core::types::ids::ResourceId;
use uuid::Uuid;

/// Slugify a title for the decoration half of a ref / a filename.
/// Lowercase, non-alphanumeric (ascii) runs → `-`, trimmed.
pub fn sluggify(title: &str) -> String {
    title
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
        .trim_matches('-')
        .to_owned()
}

/// The decorated, self-resolving form printed for every resource:
/// `sluggify(title)-<uuid>`.
pub fn decorated_ref(title: &str, id: ResourceId) -> String {
    format!("{}-{}", sluggify(title), id.0)
}

/// Resolve a ref string to a `ResourceId`. Accepts a bare UUID or a
/// decorated `…-<uuid>` form; resolution is trailing-UUID-only (the
/// decoration is ignored). No fuzzy/fragment matching — unparseable input
/// is an error, never a guess.
pub fn parse_ref(s: &str) -> Result<ResourceId, TemperError> {
    let s = s.trim();
    // Bare UUID.
    if let Ok(id) = Uuid::parse_str(s) {
        return Ok(ResourceId(id));
    }
    // Decorated: the trailing UUID is the last 5 hyphen-delimited groups
    // (UUIDs contain 4 internal hyphens). Walk from the right.
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() >= 5 {
        let tail = parts[parts.len() - 5..].join("-");
        if let Ok(id) = Uuid::parse_str(&tail) {
            return Ok(ResourceId(id));
        }
    }
    Err(TemperError::Project(format!(
        "not a resource ref (expected a UUID or `slug-<uuid>`): {s:?}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sluggify_lowercases_and_dashes() {
        // Each non-`-` non-alphanumeric char maps to one `-` (separator runs
        // are NOT collapsed); unicode letters are kept (is_alphanumeric).
        assert_eq!(sluggify("Hello, World!"), "hello--world");
        assert_eq!(sluggify("  Trim --Me-- "), "trim---me");
        assert_eq!(sluggify("Café déjà"), "café-déjà"); // unicode letters kept
    }

    #[test]
    fn decorated_ref_is_slug_dash_uuid() {
        let id = ResourceId(Uuid::parse_str("019e84ab-26ba-7560-9d34-c60d74a9fbe2").unwrap());
        assert_eq!(
            decorated_ref("My Task", id),
            "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2"
        );
    }

    #[test]
    fn parse_ref_accepts_bare_uuid() {
        let s = "019e84ab-26ba-7560-9d34-c60d74a9fbe2";
        assert_eq!(
            parse_ref(s).unwrap(),
            ResourceId(Uuid::parse_str(s).unwrap())
        );
    }

    #[test]
    fn parse_ref_accepts_decorated_and_ignores_slug_half() {
        let uuid = "019e84ab-26ba-7560-9d34-c60d74a9fbe2";
        let want = ResourceId(Uuid::parse_str(uuid).unwrap());
        // correct decoration
        assert_eq!(parse_ref(&format!("my-task-{uuid}")).unwrap(), want);
        // STALE/WRONG decoration resolves identically — harmless by construction
        assert_eq!(
            parse_ref(&format!("totally-wrong-slug-{uuid}")).unwrap(),
            want
        );
    }

    #[test]
    fn parse_ref_round_trips_decorated_ref() {
        let id = ResourceId(Uuid::now_v7());
        for title in ["A B C", "", "punct!@#", "already-slug"] {
            assert_eq!(parse_ref(&decorated_ref(title, id)).unwrap(), id);
        }
    }

    #[test]
    fn parse_ref_rejects_fragments_and_garbage() {
        // no trailing uuid → error, NO fuzzy fallback
        assert!(parse_ref("just-a-slug").is_err());
        assert!(parse_ref("").is_err());
        assert!(parse_ref("not-a-uuid-1234").is_err());
    }
}
